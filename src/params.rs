use anyhow::{Context, Result, anyhow};
use clap::ArgMatches;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::command_tree::{Operation, ParamDef, ParamLocation};

#[derive(Debug)]
pub struct BuiltRequest {
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Value>,
}

pub fn param_key(param: &ParamDef) -> String {
    format!("param__{}", param.name)
}

pub fn build_request(
    op: &Operation,
    resource_id: Option<&str>,
    matches: &ArgMatches,
    params_json: Option<&String>,
    fields: Option<&String>,
    select: Option<&String>,
) -> Result<BuiltRequest> {
    let method = op.method.to_ascii_uppercase();

    let mut query = op.query.clone().unwrap_or_default();
    let mut headers = op.headers.clone().unwrap_or_default();

    if let Some(fields) = fields {
        query.insert("fields".to_string(), fields.clone());
    }
    if let Some(select) = select {
        query.insert("fields".to_string(), select.clone());
    }

    let mut body: Option<Value> = None;
    if let Some(raw) = params_json {
        let value: Value = serde_json::from_str(raw).context("invalid JSON for --params")?;
        let Value::Object(_) = value else {
            return Err(anyhow!("--params must be a JSON object"));
        };

        if matches!(method.as_str(), "GET" | "DELETE") {
            if let Value::Object(map) = value {
                for (k, v) in map {
                    query.insert(k, json_value_to_string(&v)?);
                }
            }
        } else {
            body = Some(value);
        }
    }

    // Merge explicit flags into query/body/headers and collect path params.
    let mut path_params = BTreeMap::new();
    for param in &op.params {
        let key = param_key(param);
        if let Some(value) = matches.get_one::<String>(&key) {
            match param.location {
                ParamLocation::Path => {
                    path_params.insert(param.name.clone(), value.clone());
                }
                ParamLocation::Query => {
                    query.insert(param.name.clone(), value.clone());
                }
                ParamLocation::Header => {
                    headers.insert(param.name.clone(), value.clone());
                }
                ParamLocation::Body => {
                    let obj = body.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
                    let Value::Object(map) = obj else {
                        return Err(anyhow!("--params must be a JSON object to set body fields"));
                    };
                    map.insert(param.name.clone(), Value::String(value.clone()));
                }
            }
        }
    }

    let path = render_path(&op.path, resource_id, &path_params)?;
    Ok(BuiltRequest {
        path,
        query,
        headers,
        body,
    })
}

pub fn render_path(
    template: &str,
    resource_id: Option<&str>,
    path_params: &BTreeMap<String, String>,
) -> Result<String> {
    let mut out = template.to_string();

    if out.contains("{id}") {
        let id = resource_id.ok_or_else(|| anyhow!("--id required"))?;
        out = out.replace("{id}", urlencoding::encode(id).as_ref());
    }

    for (k, v) in path_params {
        let needle = format!("{{{k}}}");
        if out.contains(&needle) {
            out = out.replace(&needle, urlencoding::encode(v).as_ref());
        }
    }

    if out.contains('{') || out.contains('}') {
        return Err(anyhow!("missing path parameter for template: {template}"));
    }

    Ok(out)
}

fn json_value_to_string(value: &Value) -> Result<String> {
    match value {
        Value::String(v) => Ok(v.clone()),
        _ => Ok(serde_json::to_string(value)?),
    }
}
