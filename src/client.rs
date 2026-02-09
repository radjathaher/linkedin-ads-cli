use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub enum TunnelMode {
    Auto,
    Always,
    Never,
}

impl TunnelMode {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(anyhow!(
                "invalid --tunnel value {other} (expected: auto|always|never)"
            )),
        }
    }
}

#[derive(Debug)]
pub struct RestliResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

pub struct RestliClient {
    client: Client,
    pub base_url: String,
    pub access_token: String,
    pub linkedin_version: String,
    pub restli_protocol_version: String,
    pub tunnel_mode: TunnelMode,
}

impl RestliClient {
    pub fn new(
        base_url: String,
        linkedin_version: String,
        access_token: String,
        restli_protocol_version: String,
        timeout_secs: Option<u64>,
        tunnel_mode: TunnelMode,
    ) -> Result<Self> {
        let mut builder = Client::builder().user_agent("linkedin-ads-cli/0.1.0");
        if let Some(seconds) = timeout_secs {
            builder = builder.timeout(Duration::from_secs(seconds));
        }
        let client = builder.build().context("build http client")?;
        Ok(Self {
            client,
            base_url,
            access_token,
            linkedin_version,
            restli_protocol_version,
            tunnel_mode,
        })
    }

    pub fn build_url(&self, path: &str) -> Result<String> {
        if path.starts_with("http://") || path.starts_with("https://") {
            return Ok(path.to_string());
        }
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        Ok(format!("{}/{}", base, path))
    }

    pub fn call(
        &self,
        method: &str,
        path: &str,
        query: &BTreeMap<String, String>,
        headers: &BTreeMap<String, String>,
        body: Option<&Value>,
    ) -> Result<RestliResponse> {
        let method = method.to_ascii_uppercase();
        let url = self.build_url(path)?;

        let mut query_pairs: Vec<(String, String)> = Vec::new();
        for (k, v) in query {
            query_pairs.push((k.clone(), v.clone()));
        }

        let should_tunnel = self.should_tunnel(&method, &url, &query_pairs)?;
        let mut req = if should_tunnel {
            // Query tunneling: POST + X-HTTP-Method-Override + x-www-form-urlencoded body.
            self.client
                .post(&url)
                .header("X-HTTP-Method-Override", method.clone())
                .form(&query_pairs)
        } else {
            match method.as_str() {
                "GET" => self.client.get(&url).query(&query_pairs),
                "DELETE" => self.client.delete(&url).query(&query_pairs),
                "POST" => {
                    let mut r = self.client.post(&url).query(&query_pairs);
                    if let Some(body) = body {
                        r = r.json(body);
                    }
                    r
                }
                "PUT" => {
                    let mut r = self.client.put(&url).query(&query_pairs);
                    if let Some(body) = body {
                        r = r.json(body);
                    }
                    r
                }
                other => return Err(anyhow!("unsupported method {other}")),
            }
        };

        req = req
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Linkedin-Version", self.linkedin_version.clone())
            .header("X-LinkedIn-Version", self.linkedin_version.clone())
            .header(
                "X-Restli-Protocol-Version",
                self.restli_protocol_version.clone(),
            )
            .header("Accept", "application/json");

        for (k, v) in headers {
            req = req.header(k, v);
        }

        log::debug!("request {} {}", method, url);
        let resp = req.send().context("send request")?;
        let status = resp.status();

        let mut out_headers = BTreeMap::new();
        for (name, value) in resp.headers().iter() {
            let Ok(value) = value.to_str() else { continue };
            let name = name.as_str().to_string();
            if name.eq_ignore_ascii_case("authorization") {
                continue;
            }
            out_headers.insert(name, value.to_string());
        }

        let text = resp.text().context("read response body")?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text))
        };

        if !status.is_success() {
            return Err(anyhow!("http {}: {}", status, body));
        }

        Ok(RestliResponse {
            status: status.as_u16(),
            headers: out_headers,
            body,
        })
    }

    pub fn put_bytes(
        &self,
        url: &str,
        bytes: Vec<u8>,
        headers: &BTreeMap<String, String>,
        include_auth: bool,
    ) -> Result<RestliResponse> {
        let mut req = self
            .client
            .put(url)
            .header("Content-Type", "application/octet-stream")
            .header("Accept", "application/json")
            .body(bytes);

        if include_auth {
            req = req.header("Authorization", format!("Bearer {}", self.access_token));
        }
        for (k, v) in headers {
            req = req.header(k, v);
        }

        log::debug!("request PUT {}", url);
        let resp = req.send().context("send upload request")?;
        let status = resp.status();

        let mut out_headers = BTreeMap::new();
        for (name, value) in resp.headers().iter() {
            let Ok(value) = value.to_str() else { continue };
            let name = name.as_str().to_string();
            if name.eq_ignore_ascii_case("authorization") {
                continue;
            }
            out_headers.insert(name, value.to_string());
        }

        let text = resp.text().context("read upload response body")?;
        let body = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text))
        };

        if !status.is_success() {
            return Err(anyhow!("http {}: {}", status, body));
        }

        Ok(RestliResponse {
            status: status.as_u16(),
            headers: out_headers,
            body,
        })
    }

    fn should_tunnel(
        &self,
        method: &str,
        base_url: &str,
        query_pairs: &[(String, String)],
    ) -> Result<bool> {
        if !matches!(method, "GET" | "DELETE") {
            return Ok(false);
        }

        match self.tunnel_mode {
            TunnelMode::Never => return Ok(false),
            TunnelMode::Always => return Ok(!query_pairs.is_empty()),
            TunnelMode::Auto => {}
        }

        if query_pairs.is_empty() {
            return Ok(false);
        }

        let mut url = reqwest::Url::parse(base_url).context("parse url")?;
        {
            let mut qp = url.query_pairs_mut();
            for (k, v) in query_pairs {
                qp.append_pair(k, v);
            }
        }
        Ok(url.as_str().len() >= 3800)
    }
}
