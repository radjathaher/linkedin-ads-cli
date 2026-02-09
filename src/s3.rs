use anyhow::{Context, Result, anyhow};
use aws_config::{BehaviorVersion, SdkConfig};
use aws_sdk_s3::Client;
use aws_sdk_s3::presigning::PresigningConfig;
use std::io::Write;
use std::time::Duration;

fn build_runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("create tokio runtime")
}

async fn load_config() -> Result<SdkConfig> {
    Ok(aws_config::load_defaults(BehaviorVersion::latest()).await)
}

pub fn parse_s3_url(url: &str) -> Result<(String, String)> {
    let trimmed = url
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow!("invalid s3 url"))?;
    let mut parts = trimmed.splitn(2, '/');
    let bucket = parts.next().unwrap_or("").to_string();
    let key = parts.next().unwrap_or("").to_string();
    if bucket.is_empty() || key.is_empty() {
        return Err(anyhow!("invalid s3 url: {url}"));
    }
    Ok((bucket, key))
}

pub fn presign_get_blocking(url: &str, expires_secs: u64) -> Result<String> {
    let (bucket, key) = parse_s3_url(url)?;
    let rt = build_runtime()?;
    rt.block_on(async move {
        let config = load_config().await?;
        let client = Client::new(&config);
        let presign_config = PresigningConfig::expires_in(Duration::from_secs(expires_secs))
            .context("invalid presign expiry")?;
        let req = client.get_object().bucket(bucket).key(key);
        let presigned = req.presigned(presign_config).await?;
        Ok(presigned.uri().to_string())
    })
}

pub fn presign_put_blocking(
    url: &str,
    expires_secs: u64,
    content_type: Option<String>,
) -> Result<String> {
    let (bucket, key) = parse_s3_url(url)?;
    let rt = build_runtime()?;
    rt.block_on(async move {
        let config = load_config().await?;
        let client = Client::new(&config);
        let presign_config = PresigningConfig::expires_in(Duration::from_secs(expires_secs))
            .context("invalid presign expiry")?;
        let mut req = client.put_object().bucket(bucket).key(key);
        if let Some(ct) = content_type {
            req = req.content_type(ct);
        }
        let presigned = req.presigned(presign_config).await?;
        Ok(presigned.uri().to_string())
    })
}

pub fn download_object_blocking(bucket: &str, key: &str, out: &mut impl Write) -> Result<()> {
    let bucket = bucket.to_string();
    let key = key.to_string();
    let rt = build_runtime()?;
    rt.block_on(async move {
        let config = load_config().await?;
        let client = Client::new(&config);
        let resp = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .context("get s3 object")?;
        let bytes = resp.body.collect().await?.into_bytes();
        out.write_all(&bytes).context("write s3 object")?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(())
}
