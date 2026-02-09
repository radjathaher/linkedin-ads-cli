use anyhow::{Context, Result, anyhow};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::client::RestliClient;
use crate::uploads::{FileParam, read_all_bytes};

pub const DEFAULT_IMAGE_RECIPE: &str = "urn:li:digitalmediaRecipe:companyUpdate-article-image";
pub const DEFAULT_VIDEO_RECIPE: &str = "urn:li:digitalmediaRecipe:ads-video_v2";

pub fn upload_image(
    client: &RestliClient,
    owner: &str,
    file: &FileParam,
    recipe: &str,
) -> Result<Value> {
    let req = serde_json::json!({
        "registerUploadRequest": {
            "owner": owner,
            "recipes": [recipe],
            "serviceRelationships": [{
                "identifier": "urn:li:userGeneratedContent",
                "relationshipType": "OWNER"
            }],
            "supportedUploadMechanism": ["SYNCHRONOUS_UPLOAD"]
        }
    });

    let mut query = BTreeMap::new();
    query.insert("action".to_string(), "registerUpload".to_string());
    let resp = client.call("POST", "/assets", &query, &BTreeMap::new(), Some(&req))?;

    let value = resp
        .body
        .get("value")
        .ok_or_else(|| anyhow!("missing response.value"))?;

    let (upload_url, upload_headers) = extract_http_upload(value)?;
    let bytes = read_all_bytes(file)?;
    client.put_bytes(&upload_url, bytes, &upload_headers, true)?;

    Ok(value.clone())
}

pub fn upload_video(
    client: &RestliClient,
    owner: &str,
    file: &FileParam,
    recipe: &str,
    wait: bool,
) -> Result<Value> {
    let file_size = std::fs::metadata(&file.path)
        .with_context(|| format!("stat {}", file.path.display()))?
        .len();
    let multipart = file_size > 200 * 1024 * 1024;

    let mut register = serde_json::json!({
        "registerUploadRequest": {
            "owner": owner,
            "recipes": [recipe],
            "serviceRelationships": [{
                "identifier": "urn:li:userGeneratedContent",
                "relationshipType": "OWNER"
            }]
        }
    });

    if multipart {
        register["registerUploadRequest"]["supportedUploadMechanism"] =
            Value::Array(vec![Value::String("MULTIPART_UPLOAD".to_string())]);
        register["registerUploadRequest"]["fileSize"] = Value::Number(file_size.into());
    }

    let mut query = BTreeMap::new();
    query.insert("action".to_string(), "registerUpload".to_string());
    let resp = client.call("POST", "/assets", &query, &BTreeMap::new(), Some(&register))?;

    let value = resp
        .body
        .get("value")
        .ok_or_else(|| anyhow!("missing response.value"))?;

    if let Some(http) = value
        .get("uploadMechanism")
        .and_then(|m| m.get("com.linkedin.digitalmedia.uploading.MediaUploadHttpRequest"))
    {
        let upload_url = http
            .get("uploadUrl")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing uploadUrl"))?
            .to_string();
        let upload_headers = json_object_to_headers(http.get("headers"))?;

        let bytes = read_all_bytes(file)?;
        client.put_bytes(&upload_url, bytes, &upload_headers, false)?;

        if wait {
            if let Some(asset) = value.get("asset").and_then(|v| v.as_str()) {
                wait_for_asset_available(client, asset, Duration::from_secs(300))?;
            }
        }

        return Ok(value.clone());
    }

    let multipart = value
        .get("uploadMechanism")
        .and_then(|m| m.get("com.linkedin.digitalmedia.uploading.MultipartUpload"))
        .ok_or_else(|| anyhow!("missing uploadMechanism"))?;

    let metadata = multipart
        .get("metadata")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing multipart metadata"))?
        .to_string();
    let media_artifact = value
        .get("mediaArtifact")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing mediaArtifact"))?
        .to_string();

    let parts = multipart
        .get("partUploadRequests")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("missing partUploadRequests"))?;

    let mut f = File::open(&file.path).with_context(|| format!("open {}", file.path.display()))?;
    let mut part_upload_responses = Vec::new();
    for part in parts {
        let url = part
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing part url"))?
            .to_string();
        let first = part
            .get("byteRange")
            .and_then(|br| br.get("firstByte"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("missing firstByte"))?;
        let last = part
            .get("byteRange")
            .and_then(|br| br.get("lastByte"))
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("missing lastByte"))?;
        let size = (last - first + 1) as usize;

        let mut buf = vec![0u8; size];
        f.seek(SeekFrom::Start(first)).context("seek part")?;
        f.read_exact(&mut buf).context("read part")?;

        let put_headers = json_object_to_headers(part.get("headers"))?;
        let put_resp = client.put_bytes(&url, buf, &put_headers, false)?;
        let etag = find_header_ci(&put_resp.headers, "etag")
            .ok_or_else(|| anyhow!("missing ETag header for multipart part"))?;
        let etag = etag.trim_matches('"').to_string();

        part_upload_responses.push(serde_json::json!({
            "headers": { "ETag": etag },
            "httpStatusCode": 200
        }));
    }

    let complete = serde_json::json!({
        "completeMultipartUploadRequest": {
            "mediaArtifact": media_artifact,
            "metadata": metadata,
            "partUploadResponses": part_upload_responses
        }
    });

    let mut complete_query = BTreeMap::new();
    complete_query.insert("action".to_string(), "completeMultiPartUpload".to_string());
    let complete_resp = client.call(
        "POST",
        "/assets",
        &complete_query,
        &BTreeMap::new(),
        Some(&complete),
    )?;

    if wait {
        if let Some(asset) = value.get("asset").and_then(|v| v.as_str()) {
            wait_for_asset_available(client, asset, Duration::from_secs(300))?;
        }
    }

    Ok(serde_json::json!({
        "register": value,
        "complete": complete_resp.body
    }))
}

fn extract_http_upload(value: &Value) -> Result<(String, BTreeMap<String, String>)> {
    let http = value
        .get("uploadMechanism")
        .and_then(|m| m.get("com.linkedin.digitalmedia.uploading.MediaUploadHttpRequest"))
        .ok_or_else(|| anyhow!("missing uploadMechanism.MediaUploadHttpRequest"))?;
    let upload_url = http
        .get("uploadUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing uploadUrl"))?
        .to_string();
    let headers = json_object_to_headers(http.get("headers"))?;
    Ok((upload_url, headers))
}

fn json_object_to_headers(value: Option<&Value>) -> Result<BTreeMap<String, String>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let Value::Object(map) = value else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (k, v) in map {
        if let Some(s) = v.as_str() {
            out.insert(k.clone(), s.to_string());
        }
    }
    Ok(out)
}

fn find_header_ci(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn wait_for_asset_available(
    client: &RestliClient,
    asset_urn: &str,
    timeout: Duration,
) -> Result<()> {
    let asset_id = asset_id_from_urn(asset_urn).unwrap_or_else(|| asset_urn.to_string());
    let start = Instant::now();
    loop {
        let mut query = BTreeMap::new();
        query.insert("fields".to_string(), "recipes,id,status".to_string());
        let resp = client.call(
            "GET",
            &format!("/assets/{}", asset_id),
            &query,
            &BTreeMap::new(),
            None,
        )?;
        if let Some(recipes) = resp.body.get("recipes").and_then(|v| v.as_array()) {
            let all_ready = recipes.iter().all(|r| {
                r.get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "AVAILABLE")
                    .unwrap_or(false)
            });
            if all_ready {
                return Ok(());
            }
        }
        if start.elapsed() >= timeout {
            return Err(anyhow!("asset processing timeout"));
        }
        sleep(Duration::from_secs(3));
    }
}

fn asset_id_from_urn(value: &str) -> Option<String> {
    // urn:li:digitalmediaAsset:C5405AQEOFHXqeM2vRA -> C5405AQEOFHXqeM2vRA
    value.rsplit_once(':').map(|(_, id)| id.to_string())
}
