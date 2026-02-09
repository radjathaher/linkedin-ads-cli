use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

use crate::s3;

#[derive(Debug)]
#[allow(dead_code)]
pub struct FileParam {
    pub path: PathBuf,
    pub file_name: String,
    _temp: Option<tempfile::TempPath>,
}

pub fn resolve_file_source(value: &str) -> Result<FileParam> {
    if value.starts_with("s3://") {
        return download_s3(value);
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        return download_http(value);
    }
    let local = local_path(value);
    if local.exists() {
        let file_name = local
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("upload")
            .to_string();
        return Ok(FileParam {
            path: local,
            file_name,
            _temp: None,
        });
    }
    Err(anyhow!("file not found: {value}"))
}

pub fn read_all_bytes(file: &FileParam) -> Result<Vec<u8>> {
    let mut f = File::open(&file.path).with_context(|| format!("open {}", file.path.display()))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

fn download_http(url: &str) -> Result<FileParam> {
    let client = Client::new();
    let mut resp = client.get(url).send().context("download url")?;
    let mut file = NamedTempFile::new().context("create temp file")?;
    resp.copy_to(&mut file).context("write temp file")?;
    let temp_path = file.into_temp_path();
    let path = temp_path.to_path_buf();
    let file_name = url
        .split('/')
        .last()
        .filter(|v| !v.is_empty())
        .unwrap_or("download")
        .to_string();
    Ok(FileParam {
        path,
        file_name,
        _temp: Some(temp_path),
    })
}

fn download_s3(url: &str) -> Result<FileParam> {
    let (bucket, key) = s3::parse_s3_url(url)?;
    let mut file = NamedTempFile::new().context("create temp file")?;
    s3::download_object_blocking(&bucket, &key, &mut file)?;
    let temp_path = file.into_temp_path();
    let path = temp_path.to_path_buf();
    let file_name = Path::new(&key)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("s3-object")
        .to_string();
    Ok(FileParam {
        path,
        file_name,
        _temp: Some(temp_path),
    })
}

fn local_path(value: &str) -> PathBuf {
    if let Some(path) = value.strip_prefix('@') {
        return PathBuf::from(path);
    }
    if let Some(path) = value.strip_prefix("file://") {
        return PathBuf::from(path);
    }
    PathBuf::from(value)
}

// TempPath cleans up automatically on drop.
