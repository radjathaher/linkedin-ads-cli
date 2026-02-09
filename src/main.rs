mod asset_upload;
mod client;
mod command_tree;
mod params;
mod s3;
mod uploads;

use anyhow::{Context, Result, anyhow};
use clap::{Arg, ArgAction, Command};
use command_tree::{CommandTree, Operation};
use params::{build_request, param_key};
use serde_json::Value;
use std::collections::BTreeMap;
use std::env;
use std::io::Write;

use asset_upload::{DEFAULT_IMAGE_RECIPE, DEFAULT_VIDEO_RECIPE, upload_image, upload_video};
use client::{RestliClient, TunnelMode};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let tree = command_tree::load_command_tree();
    let cli = build_cli(&tree);
    let matches = cli.get_matches();

    if let Some(matches) = matches.subcommand_matches("list") {
        return handle_list(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("describe") {
        return handle_describe(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("tree") {
        return handle_tree(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("s3") {
        return handle_s3(matches);
    }
    if let Some(matches) = matches.subcommand_matches("image") {
        return handle_image(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("video") {
        return handle_video(&tree, matches);
    }
    if let Some(matches) = matches.subcommand_matches("raw") {
        return handle_raw(&tree, matches);
    }

    let config = load_config(&tree, &matches)?;
    let client = RestliClient::new(
        config.base_url,
        config.linkedin_version,
        config.access_token,
        config.restli_protocol_version,
        config.timeout,
        config.tunnel_mode,
    )?;

    let pretty = matches.get_flag("pretty");
    let raw_output = matches.get_flag("raw");
    let all = matches.get_flag("all");
    let max_pages = matches.get_one::<u64>("max_pages").copied().unwrap_or(0);
    let max_items = matches.get_one::<u64>("max_items").copied().unwrap_or(0);

    let (res_name, res_matches) = matches
        .subcommand()
        .ok_or_else(|| anyhow!("resource required"))?;
    let (op_name, op_matches) = res_matches
        .subcommand()
        .ok_or_else(|| anyhow!("operation required"))?;

    let op = find_op(&tree, res_name, op_name)
        .ok_or_else(|| anyhow!("unknown command {res_name} {op_name}"))?;

    let id = res_matches
        .get_one::<String>("resource_id")
        .cloned()
        .or_else(|| default_account_id(res_name))
        .or_else(|| default_asset_id(res_name));

    let params_json = op_matches.get_one::<String>("params");
    let fields = op_matches.get_one::<String>("fields");
    let select = op_matches.get_one::<String>("select");

    let built = build_request(op, id.as_deref(), op_matches, params_json, fields, select)?;
    let response = if all {
        paginate_all(
            &client,
            &op.method,
            &built.path,
            &built.query,
            &built.headers,
            built.body.as_ref(),
            max_pages,
            max_items,
        )?
    } else {
        client.call(
            &op.method,
            &built.path,
            &built.query,
            &built.headers,
            built.body.as_ref(),
        )?
    };

    let output = if raw_output {
        serde_json::json!({
            "status": response.status,
            "headers": response.headers,
            "body": response.body
        })
    } else {
        unwrap_body(response.body, &response.headers)
    };

    write_json(&output, pretty)?;
    Ok(())
}

fn load_config(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<Config> {
    let access_token = matches
        .get_one::<String>("access_token")
        .cloned()
        .or_else(|| env::var("LINKEDIN_ACCESS_TOKEN").ok())
        .ok_or_else(|| anyhow!("LINKEDIN_ACCESS_TOKEN missing"))?;

    let linkedin_version = matches
        .get_one::<String>("linkedin_version")
        .cloned()
        .or_else(|| env::var("LINKEDIN_VERSION").ok())
        .unwrap_or_else(|| tree.default_linkedin_version.clone());

    let base_url = matches
        .get_one::<String>("base_url")
        .cloned()
        .or_else(|| env::var("LINKEDIN_BASE_URL").ok())
        .unwrap_or_else(|| tree.default_base_url.clone());

    let restli_protocol_version = matches
        .get_one::<String>("restli_protocol_version")
        .cloned()
        .or_else(|| env::var("LINKEDIN_RESTLI_PROTOCOL_VERSION").ok())
        .unwrap_or_else(|| "2.0.0".to_string());

    let timeout = matches.get_one::<u64>("timeout").copied();

    let tunnel_mode = matches
        .get_one::<String>("tunnel")
        .map(|v| client::TunnelMode::parse(v.as_str()))
        .transpose()?
        .unwrap_or(client::TunnelMode::Auto);

    if matches.get_flag("debug") {
        env_logger::Builder::from_env("RUST_LOG")
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_env("RUST_LOG")
            .filter_level(log::LevelFilter::Warn)
            .init();
    }

    Ok(Config {
        access_token,
        linkedin_version,
        base_url,
        restli_protocol_version,
        timeout,
        tunnel_mode,
    })
}

fn build_cli(tree: &CommandTree) -> Command {
    let mut cmd = Command::new("linkedin-ads")
        .about("LinkedIn Marketing API CLI (Rest.li /rest)")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("access_token")
                .long("access-token")
                .global(true)
                .value_name("TOKEN")
                .help("Access token (env: LINKEDIN_ACCESS_TOKEN)"),
        )
        .arg(
            Arg::new("linkedin_version")
                .long("linkedin-version")
                .global(true)
                .value_name("YYYYMM")
                .help("LinkedIn API version header (env: LINKEDIN_VERSION)"),
        )
        .arg(
            Arg::new("base_url")
                .long("base-url")
                .global(true)
                .value_name("URL")
                .help("API base URL (env: LINKEDIN_BASE_URL)"),
        )
        .arg(
            Arg::new("restli_protocol_version")
                .long("restli-protocol-version")
                .global(true)
                .value_name("VERSION")
                .help("Rest.li protocol version (env: LINKEDIN_RESTLI_PROTOCOL_VERSION)"),
        )
        .arg(
            Arg::new("tunnel")
                .long("tunnel")
                .global(true)
                .value_name("MODE")
                .default_value("auto")
                .value_parser(["auto", "always", "never"])
                .help("Query tunneling mode for long GETs (auto|always|never)"),
        )
        .arg(
            Arg::new("pretty")
                .long("pretty")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Pretty-print JSON output"),
        )
        .arg(
            Arg::new("raw")
                .long("raw")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Include status + headers in output"),
        )
        .arg(
            Arg::new("debug")
                .long("debug")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Enable debug logging"),
        )
        .arg(
            Arg::new("timeout")
                .long("timeout")
                .global(true)
                .value_name("SECONDS")
                .value_parser(clap::value_parser!(u64))
                .help("HTTP timeout in seconds"),
        )
        .arg(
            Arg::new("all")
                .long("all")
                .global(true)
                .action(ArgAction::SetTrue)
                .help("Auto-paginate all pages (follows paging.links rel=next)"),
        )
        .arg(
            Arg::new("max_pages")
                .long("max-pages")
                .global(true)
                .value_name("N")
                .value_parser(clap::value_parser!(u64))
                .help("Max pages to fetch when --all"),
        )
        .arg(
            Arg::new("max_items")
                .long("max-items")
                .global(true)
                .value_name("N")
                .value_parser(clap::value_parser!(u64))
                .help("Max items to fetch when --all"),
        );

    cmd = cmd.subcommand(
        Command::new("list")
            .about("List resources and operations")
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Emit machine-readable JSON"),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("describe")
            .about("Describe a specific operation")
            .arg(Arg::new("resource").required(true))
            .arg(Arg::new("op").required(true))
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Emit machine-readable JSON"),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("tree").about("Show full command tree").arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help("Emit machine-readable JSON"),
        ),
    );

    cmd = cmd.subcommand(
        Command::new("raw")
            .about("Make a raw LinkedIn REST call")
            .arg(Arg::new("method").required(true))
            .arg(Arg::new("path").required(true))
            .arg(
                Arg::new("query")
                    .long("query")
                    .value_name("JSON")
                    .help("JSON object of query params"),
            )
            .arg(
                Arg::new("body")
                    .long("body")
                    .value_name("JSON")
                    .help("JSON object body"),
            )
            .arg(
                Arg::new("headers")
                    .long("headers")
                    .value_name("JSON")
                    .help("JSON object of headers"),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("s3")
            .about("S3 helpers")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("presign")
                    .subcommand_required(true)
                    .arg_required_else_help(true)
                    .subcommand(
                        Command::new("get").arg(Arg::new("url").required(true)).arg(
                            Arg::new("expires")
                                .long("expires")
                                .value_name("SECONDS")
                                .value_parser(clap::value_parser!(u64))
                                .default_value("3600"),
                        ),
                    )
                    .subcommand(
                        Command::new("put")
                            .arg(Arg::new("url").required(true))
                            .arg(
                                Arg::new("expires")
                                    .long("expires")
                                    .value_name("SECONDS")
                                    .value_parser(clap::value_parser!(u64))
                                    .default_value("3600"),
                            )
                            .arg(
                                Arg::new("content_type")
                                    .long("content-type")
                                    .value_name("MIME"),
                            ),
                    ),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("image")
            .about("Image helpers (Assets API)")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("upload")
                    .arg(
                        Arg::new("owner")
                            .long("owner")
                            .value_name("URN")
                            .required(true),
                    )
                    .arg(
                        Arg::new("file")
                            .long("file")
                            .value_name("FILE|URL|S3")
                            .required(true),
                    )
                    .arg(
                        Arg::new("recipe")
                            .long("recipe")
                            .value_name("URN")
                            .default_value(DEFAULT_IMAGE_RECIPE),
                    ),
            ),
    );

    cmd = cmd.subcommand(
        Command::new("video")
            .about("Video helpers (Assets API)")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("upload")
                    .arg(
                        Arg::new("owner")
                            .long("owner")
                            .value_name("URN")
                            .required(true),
                    )
                    .arg(
                        Arg::new("file")
                            .long("file")
                            .value_name("FILE|URL|S3")
                            .required(true),
                    )
                    .arg(
                        Arg::new("recipe")
                            .long("recipe")
                            .value_name("URN")
                            .default_value(DEFAULT_VIDEO_RECIPE),
                    )
                    .arg(
                        Arg::new("wait")
                            .long("wait")
                            .action(ArgAction::SetTrue)
                            .help("Wait for processing (polls /assets/{id})"),
                    ),
            ),
    );

    for resource in &tree.resources {
        let mut res_cmd = Command::new(resource.name.clone())
            .about(resource.name.clone())
            .subcommand_required(true)
            .arg_required_else_help(true)
            .arg(
                Arg::new("resource_id")
                    .long("id")
                    .value_name("ID")
                    .help("Primary resource id (env: LINKEDIN_AD_ACCOUNT_ID for ad-account)"),
            );

        for op in &resource.ops {
            let mut op_cmd = Command::new(op.name.clone()).about(op.path.clone());
            op_cmd = op_cmd.arg(
                Arg::new("params")
                    .long("params")
                    .value_name("JSON")
                    .help("JSON object of parameters"),
            );
            op_cmd = op_cmd.arg(
                Arg::new("fields")
                    .long("fields")
                    .value_name("FIELDS")
                    .help("fields query param"),
            );
            op_cmd = op_cmd.arg(
                Arg::new("select")
                    .long("select")
                    .value_name("FIELDS")
                    .help("Alias for --fields"),
            );

            for param in &op.params {
                op_cmd = op_cmd.arg(build_param_arg(param));
            }
            res_cmd = res_cmd.subcommand(op_cmd);
        }
        cmd = cmd.subcommand(res_cmd);
    }

    cmd
}

fn build_param_arg(param: &command_tree::ParamDef) -> Arg {
    let mut arg = Arg::new(param_key(param))
        .long(param.flag.clone())
        .value_name(param.param_type.clone());
    if param.param_type.starts_with("list<") {
        arg = arg.action(ArgAction::Append);
    }
    arg
}

fn handle_list(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    if matches.get_flag("json") {
        let mut out = Vec::new();
        for res in &tree.resources {
            let ops: Vec<String> = res.ops.iter().map(|op| op.name.clone()).collect();
            out.push(serde_json::json!({"resource": res.name, "ops": ops}));
        }
        write_json(&Value::Array(out), true)?;
        return Ok(());
    }

    for res in &tree.resources {
        write_stdout_line(&res.name)?;
        for op in &res.ops {
            write_stdout_line(&format!("  {}", op.name))?;
        }
    }
    Ok(())
}

fn handle_describe(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    let resource = matches
        .get_one::<String>("resource")
        .ok_or_else(|| anyhow!("resource required"))?;
    let op_name = matches
        .get_one::<String>("op")
        .ok_or_else(|| anyhow!("operation required"))?;

    let op = find_op(tree, resource, op_name)
        .ok_or_else(|| anyhow!("unknown command {resource} {op_name}"))?;

    if matches.get_flag("json") {
        write_json(&serde_json::to_value(op)?, true)?;
        return Ok(());
    }

    write_stdout_line(&format!("{} {}", resource, op.name))?;
    write_stdout_line(&format!("  method: {}", op.method))?;
    write_stdout_line(&format!("  path: {}", op.path))?;
    if let Some(query) = &op.query {
        write_stdout_line("  query defaults:")?;
        for (k, v) in query {
            write_stdout_line(&format!("    {}={}", k, v))?;
        }
    }
    if let Some(headers) = &op.headers {
        write_stdout_line("  headers defaults:")?;
        for (k, v) in headers {
            write_stdout_line(&format!("    {}: {}", k, v))?;
        }
    }
    if !op.params.is_empty() {
        write_stdout_line("  params:")?;
        for param in &op.params {
            write_stdout_line(&format!(
                "    --{}  {}  ({:?})",
                param.flag, param.param_type, param.location
            ))?;
        }
    }
    Ok(())
}

fn handle_tree(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    if matches.get_flag("json") {
        write_json(&serde_json::to_value(tree)?, true)?;
        return Ok(());
    }
    write_stdout_line("Run with --json for machine-readable output.")?;
    Ok(())
}

fn handle_raw(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    let method = matches
        .get_one::<String>("method")
        .ok_or_else(|| anyhow!("method required"))?
        .to_ascii_uppercase();
    let path = matches
        .get_one::<String>("path")
        .ok_or_else(|| anyhow!("path required"))?;

    let query_json = matches.get_one::<String>("query");
    let body_json = matches.get_one::<String>("body");
    let headers_json = matches.get_one::<String>("headers");

    let mut query = BTreeMap::new();
    if let Some(raw) = query_json {
        query = json_object_to_string_map(raw, "--query")?;
    }

    let mut headers = BTreeMap::new();
    if let Some(raw) = headers_json {
        headers = json_object_to_string_map(raw, "--headers")?;
    }

    let body = if let Some(raw) = body_json {
        let value: Value = serde_json::from_str(raw).context("invalid JSON for --body")?;
        Some(value)
    } else {
        None
    };

    let config = load_config(tree, matches)?;
    let client = RestliClient::new(
        config.base_url,
        config.linkedin_version,
        config.access_token,
        config.restli_protocol_version,
        config.timeout,
        config.tunnel_mode,
    )?;

    let resp = client.call(&method, path, &query, &headers, body.as_ref())?;
    let out = serde_json::json!({
        "status": resp.status,
        "headers": resp.headers,
        "body": resp.body
    });
    write_json(&out, matches.get_flag("pretty"))?;
    Ok(())
}

fn handle_s3(matches: &clap::ArgMatches) -> Result<()> {
    let (sub, sub_matches) = matches
        .subcommand()
        .ok_or_else(|| anyhow!("subcommand required"))?;
    if sub != "presign" {
        return Err(anyhow!("unknown s3 command"));
    }
    let (op, op_matches) = sub_matches
        .subcommand()
        .ok_or_else(|| anyhow!("subcommand required"))?;
    let url = op_matches
        .get_one::<String>("url")
        .ok_or_else(|| anyhow!("url required"))?;
    let expires = op_matches
        .get_one::<u64>("expires")
        .copied()
        .unwrap_or(3600);

    let presigned = if op == "get" {
        s3::presign_get_blocking(url, expires)?
    } else if op == "put" {
        let ct = op_matches.get_one::<String>("content_type").cloned();
        s3::presign_put_blocking(url, expires, ct)?
    } else {
        return Err(anyhow!("unknown s3 presign op"));
    };

    write_stdout_line(&presigned)?;
    Ok(())
}

fn handle_image(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    let (op, op_matches) = matches.subcommand().ok_or_else(|| anyhow!("op required"))?;
    if op != "upload" {
        return Err(anyhow!("unknown image op"));
    }

    let owner = op_matches
        .get_one::<String>("owner")
        .ok_or_else(|| anyhow!("owner required"))?;
    let file = op_matches
        .get_one::<String>("file")
        .ok_or_else(|| anyhow!("file required"))?;
    let recipe = op_matches
        .get_one::<String>("recipe")
        .map(|s| s.as_str())
        .unwrap_or(DEFAULT_IMAGE_RECIPE);

    let config = load_config(tree, matches)?;
    let client = RestliClient::new(
        config.base_url,
        config.linkedin_version,
        config.access_token,
        config.restli_protocol_version,
        config.timeout,
        config.tunnel_mode,
    )?;

    let file = uploads::resolve_file_source(file)?;
    let out = upload_image(&client, owner, &file, recipe)?;
    write_json(&out, matches.get_flag("pretty"))?;
    Ok(())
}

fn handle_video(tree: &CommandTree, matches: &clap::ArgMatches) -> Result<()> {
    let (op, op_matches) = matches.subcommand().ok_or_else(|| anyhow!("op required"))?;
    if op != "upload" {
        return Err(anyhow!("unknown video op"));
    }

    let owner = op_matches
        .get_one::<String>("owner")
        .ok_or_else(|| anyhow!("owner required"))?;
    let file = op_matches
        .get_one::<String>("file")
        .ok_or_else(|| anyhow!("file required"))?;
    let recipe = op_matches
        .get_one::<String>("recipe")
        .map(|s| s.as_str())
        .unwrap_or(DEFAULT_VIDEO_RECIPE);
    let wait = op_matches.get_flag("wait");

    let config = load_config(tree, matches)?;
    let client = RestliClient::new(
        config.base_url,
        config.linkedin_version,
        config.access_token,
        config.restli_protocol_version,
        config.timeout,
        config.tunnel_mode,
    )?;

    let file = uploads::resolve_file_source(file)?;
    let out = upload_video(&client, owner, &file, recipe, wait)?;
    write_json(&out, matches.get_flag("pretty"))?;
    Ok(())
}

fn paginate_all(
    client: &RestliClient,
    method: &str,
    path: &str,
    query: &BTreeMap<String, String>,
    headers: &BTreeMap<String, String>,
    body: Option<&Value>,
    max_pages: u64,
    max_items: u64,
) -> Result<client::RestliResponse> {
    let mut resp = client.call(method, path, query, headers, body)?;
    let mut items = Vec::new();
    let mut pages = 1u64;

    loop {
        if let Some(arr) = resp.body.get("elements").and_then(|v| v.as_array()) {
            for item in arr {
                items.push(item.clone());
                if max_items > 0 && items.len() as u64 >= max_items {
                    resp.body = serde_json::json!({ "elements": items });
                    return Ok(resp);
                }
            }
        } else {
            break;
        }

        let next_href = next_link_href(&resp.body);
        let Some(next) = next_href else { break };
        if max_pages > 0 && pages >= max_pages {
            break;
        }
        pages += 1;
        resp = client.call("GET", &next, &BTreeMap::new(), headers, None)?;
    }

    resp.body = serde_json::json!({ "elements": items });
    Ok(resp)
}

fn next_link_href(body: &Value) -> Option<String> {
    let links = body
        .get("paging")
        .and_then(|p| p.get("links"))
        .and_then(|l| l.as_array())?;
    for link in links {
        let rel = link.get("rel").and_then(|v| v.as_str())?;
        if rel == "next" {
            if let Some(href) = link.get("href").and_then(|v| v.as_str()) {
                return Some(href.to_string());
            }
        }
    }
    None
}

fn unwrap_body(body: Value, headers: &BTreeMap<String, String>) -> Value {
    let mut out = if let Some(elements) = body.get("elements").cloned() {
        elements
    } else if let Some(value) = body.get("value").cloned() {
        value
    } else {
        body
    };

    if let Some(id) = find_header_ci(headers, "x-restli-id") {
        match &mut out {
            Value::Null => {
                out = serde_json::json!({ "id": id });
            }
            Value::Object(map) => {
                map.entry("id".to_string()).or_insert(Value::String(id));
            }
            _ => {}
        }
    }

    out
}

fn find_header_ci(headers: &BTreeMap<String, String>, name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn json_object_to_string_map(raw: &str, flag: &str) -> Result<BTreeMap<String, String>> {
    let value: Value =
        serde_json::from_str(raw).with_context(|| format!("invalid JSON for {flag}"))?;
    let Value::Object(map) = value else {
        return Err(anyhow!("{flag} must be a JSON object"));
    };
    let mut out = BTreeMap::new();
    for (k, v) in map {
        out.insert(k, json_value_to_string(&v)?);
    }
    Ok(out)
}

fn json_value_to_string(value: &Value) -> Result<String> {
    match value {
        Value::String(v) => Ok(v.clone()),
        _ => Ok(serde_json::to_string(value)?),
    }
}

fn find_op<'a>(tree: &'a CommandTree, res: &str, op: &str) -> Option<&'a Operation> {
    tree.resources
        .iter()
        .find(|r| r.name == res)
        .and_then(|r| r.ops.iter().find(|o| o.name == op))
}

fn default_account_id(resource: &str) -> Option<String> {
    if resource != "ad-account" {
        return None;
    }
    env::var("LINKEDIN_AD_ACCOUNT_ID").ok()
}

fn default_asset_id(resource: &str) -> Option<String> {
    if resource != "asset" {
        return None;
    }
    env::var("LINKEDIN_ASSET_ID").ok()
}

fn write_json(value: &Value, pretty: bool) -> Result<()> {
    if pretty {
        write_stdout_line(&serde_json::to_string_pretty(value)?)
    } else {
        write_stdout_line(&serde_json::to_string(value)?)
    }
}

fn write_stdout_line(value: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    if let Err(err) = out.write_all(value.as_bytes()) {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        return Err(err.into());
    }
    if let Err(err) = out.write_all(b"\n") {
        if err.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        return Err(err.into());
    }
    Ok(())
}

struct Config {
    access_token: String,
    linkedin_version: String,
    base_url: String,
    restli_protocol_version: String,
    timeout: Option<u64>,
    tunnel_mode: TunnelMode,
}
