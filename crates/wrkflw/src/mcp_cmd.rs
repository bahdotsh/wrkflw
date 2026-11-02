//! `wrkflw mcp serve`: a Model Context Protocol server over stdio.
//!
//! Tools run the wrkflw binary itself with `--format json`. A child process
//! keeps stdout, which here is the protocol, safe from anything the engine
//! prints, and contains the `process::exit` calls in the command paths. It
//! also means a tool result is exactly what the CLI's JSON contract promises.

use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_RUN_TIMEOUT_SECS: u64 = 1800;
/// Longest a read-only tool may take. These only parse files and read history.
const QUICK_TIMEOUT_SECS: u64 = 120;

const INSTRUCTIONS: &str =
    "wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. \
Call validate_workflows after editing YAML, plan_workflow to see what would run and why a trigger \
would or would not fire, and run_workflow to execute. run_workflow returns a failure digest. Use \
list_runs and get_run_failure before rerunning: earlier results are recorded.";

fn string_arg(args: &Value, key: &str) -> Option<String> {
    args[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn push_opt(cli: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        cli.push(flag.to_string());
        cli.push(value);
    }
}

fn changed_files(args: &Value) -> Option<String> {
    let files: Vec<&str> = args["changed_files"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    (!files.is_empty()).then(|| files.join(","))
}

fn event_flags(cli: &mut Vec<String>, args: &Value) {
    push_opt(cli, "--job", string_arg(args, "job"));
    push_opt(cli, "--event", string_arg(args, "event"));
    push_opt(cli, "--base-branch", string_arg(args, "base_branch"));
    push_opt(cli, "--activity-type", string_arg(args, "activity_type"));
    push_opt(cli, "--changed-files", changed_files(args));
    if args["diff"].as_bool() == Some(true) {
        cli.push("--diff".to_string());
    }
}

fn required_path(args: &Value) -> Result<String, String> {
    string_arg(args, "path").ok_or_else(|| "`path` is required".to_string())
}

fn run_ref(args: &Value) -> String {
    string_arg(args, "run_id").unwrap_or_else(|| "latest".to_string())
}

/// The CLI arguments implementing a tool, and how long it may take.
fn tool_command(name: &str, args: &Value) -> Result<(Vec<String>, u64), String> {
    let mut cli: Vec<String> = Vec::new();
    let mut timeout = QUICK_TIMEOUT_SECS;
    match name {
        "validate_workflows" => {
            cli.push("validate".into());
            let paths: Vec<String> = args["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect();
            cli.push("--".into());
            cli.extend(paths);
        }
        "list_workflows" => cli.extend(["list".to_string(), "--jobs".to_string()]),
        "plan_workflow" => {
            cli.push("plan".into());
            event_flags(&mut cli, args);
            cli.push("--".into());
            cli.push(required_path(args)?);
        }
        "run_workflow" => {
            cli.push("run".into());
            event_flags(&mut cli, args);
            push_opt(&mut cli, "--runtime", string_arg(args, "runtime"));
            push_opt(&mut cli, "--intent", string_arg(args, "intent"));
            push_opt(&mut cli, "--from-step", string_arg(args, "from_step"));
            cli.push("--".into());
            cli.push(required_path(args)?);
            timeout = args["timeout_seconds"]
                .as_u64()
                .unwrap_or(DEFAULT_RUN_TIMEOUT_SECS);
        }
        "list_runs" => {
            cli.extend(["runs".to_string(), "list".to_string()]);
            push_opt(&mut cli, "--status", string_arg(args, "status"));
            push_opt(&mut cli, "--agent", string_arg(args, "agent"));
            push_opt(&mut cli, "--workflow", string_arg(args, "workflow"));
            if let Some(limit) = args["limit"].as_u64() {
                cli.push("--limit".into());
                cli.push(limit.to_string());
            }
        }
        "get_run" => cli.extend([
            "runs".to_string(),
            "show".to_string(),
            "--".to_string(),
            run_ref(args),
        ]),
        "get_run_failure" => {
            cli.extend(["runs".to_string(), "failure".to_string()]);
            if let Some(tail) = args["tail"].as_u64() {
                cli.push("--tail".into());
                cli.push(tail.to_string());
            }
            cli.push("--".into());
            cli.push(run_ref(args));
        }
        "get_run_logs" => {
            cli.extend(["runs".to_string(), "logs".to_string()]);
            push_opt(&mut cli, "--job", string_arg(args, "job"));
            push_opt(&mut cli, "--step", string_arg(args, "step"));
            let tail = args["tail"].as_u64().unwrap_or(200);
            cli.push("--tail".into());
            cli.push(tail.to_string());
            cli.push("--".into());
            cli.push(run_ref(args));
        }
        other => return Err(format!("unknown tool: {other}")),
    }
    cli.extend(["--format".to_string(), "json".to_string()]);
    // Global flags after `--` would be read as paths, so move the separator
    // to the end by putting the format first.
    let format = cli.split_off(cli.len() - 2);
    let mut full = format;
    full.append(&mut cli);
    Ok((full, timeout))
}

/// What the tool returns: the last JSON object the command printed, which for
/// `run` is `run_finished` and for everything else the only one.
fn tool_result(stdout: &str, stderr: &str, success: bool) -> Value {
    let last = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok());
    let body = last.unwrap_or_else(|| json!({"type": "error", "message": stderr.trim()}));
    let is_error = body["type"] == "error" || (!success && body["type"] == Value::Null);
    let text = serde_json::to_string_pretty(&body).unwrap_or_default();
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn error_result(message: &str) -> Value {
    json!({"content": [{"type": "text", "text": message}], "isError": true})
}

async fn call_tool(name: &str, args: &Value, agent: Option<&str>) -> Value {
    let (cli, timeout) = match tool_command(name, args) {
        Ok(command) => command,
        Err(e) => return error_result(&e),
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return error_result(&e.to_string()),
    };
    let mut command = tokio::process::Command::new(exe);
    command
        .args(&cli)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("WRKFLW_MCP", "1")
        .kill_on_drop(true);
    if let Some(cwd) = string_arg(args, "cwd") {
        command.current_dir(cwd);
    }
    if let Some(agent) = agent {
        if std::env::var_os("WRKFLW_AGENT").is_none() {
            command.env("WRKFLW_AGENT", agent);
        }
    }
    match tokio::time::timeout(Duration::from_secs(timeout), command.output()).await {
        Ok(Ok(output)) => tool_result(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.success(),
        ),
        Ok(Err(e)) => error_result(&format!("could not start wrkflw: {e}")),
        Err(_) => error_result(&format!(
            "{name} timed out after {timeout}s and was stopped"
        )),
    }
}

fn tool(
    name: &str,
    description: &str,
    read_only: bool,
    properties: Value,
    required: &[&str],
) -> Value {
    let mut schema = json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
    schema["properties"]["cwd"] =
        json!({"type": "string", "description": "Directory to work in. Defaults to the server's."});
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": {"readOnlyHint": read_only},
    })
}

fn event_properties() -> Value {
    json!({
        "path": {"type": "string", "description": "Workflow or pipeline file."},
        "job": {"type": "string", "description": "Only this job and what it needs."},
        "event": {"type": "string", "description": "Event to simulate, e.g. push or pull_request."},
        "base_branch": {"type": "string", "description": "Target branch. Required with pull_request."},
        "activity_type": {"type": "string", "description": "Activity type, e.g. opened, for `types:` filters."},
        "changed_files": {"type": "array", "items": {"type": "string"}, "description": "Repo-relative changed files."},
        "diff": {"type": "boolean", "description": "Take changed files from git diff."},
    })
}

fn tools() -> Value {
    let mut run_properties = event_properties();
    run_properties["runtime"] = json!({
        "type": "string",
        "enum": ["auto", "docker", "podman", "kubernetes", "emulation", "secure-emulation"],
        "description": "`emulation` runs steps directly on this machine.",
    });
    run_properties["intent"] = json!({"type": "string", "description": "Why you are running this. Stored on the run record."});
    run_properties["from_step"] = json!({"type": "string", "description": "Requires `job`. Start at this step, by name or 1-based position. Earlier steps' outputs and files are missing."});
    run_properties["timeout_seconds"] =
        json!({"type": "integer", "description": "Stop the run after this long. Default 1800."});
    let run_id = json!({"type": "string", "description": "Run id, unique prefix, or `latest`. Default `latest`."});
    json!([
        tool("validate_workflows", "Check workflow files. Returns issues with rule, YAML path, line and fix hint.", true,
            json!({"paths": {"type": "array", "items": {"type": "string"}, "description": "Files or directories. Defaults to the workflow directories."}}), &[]),
        tool("list_workflows", "List workflows and pipelines in the repository with their jobs.", true, json!({}), &[]),
        tool("plan_workflow", "Show what a run would do without executing anything: whether the trigger fires and why, job order, matrix legs and steps.", true, event_properties(), &["path"]),
        tool("run_workflow", "Execute a workflow locally. This runs the workflow's code. Returns status, job results and a failure digest. The run is recorded.", false, run_properties, &["path"]),
        tool("list_runs", "List recorded runs of this repository, newest first.", true,
            json!({"limit": {"type": "integer"}, "status": {"type": "string"}, "agent": {"type": "string"}, "workflow": {"type": "string"}}), &[]),
        tool("get_run", "Get one recorded run: jobs, steps, exit codes, timings.", true, json!({"run_id": run_id}), &[]),
        tool("get_run_failure", "Explain why a recorded run failed: failed step, script, env from earlier steps, output tail.", true,
            json!({"run_id": run_id, "tail": {"type": "integer", "description": "Output lines per failed step."}}), &[]),
        tool("get_run_logs", "Read step logs of a recorded run. Returns the last 200 lines of each by default.", true,
            json!({"run_id": run_id, "job": {"type": "string"}, "step": {"type": "string", "description": "Step name or 1-based position."}, "tail": {"type": "integer"}}), &[]),
    ])
}

fn negotiate(requested: Option<&str>) -> &'static str {
    SUPPORTED_PROTOCOLS
        .iter()
        .find(|version| Some(**version) == requested)
        .copied()
        .unwrap_or(SUPPORTED_PROTOCOLS[0])
}

/// Handle one request. `Err` carries a JSON-RPC error code and message.
async fn handle(
    method: &str,
    params: &Value,
    client: &Mutex<Option<String>>,
) -> Result<Value, (i64, String)> {
    match method {
        "initialize" => {
            let name = string_arg(&params["clientInfo"], "name");
            *client.lock().await = name;
            Ok(json!({
                "protocolVersion": negotiate(params["protocolVersion"].as_str()),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "wrkflw", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return Err((-32602, "tools/call needs a name".to_string()));
            };
            let agent = client.lock().await.clone();
            Ok(call_tool(name, &params["arguments"], agent.as_deref()).await)
        }
        other => Err((-32601, format!("method not found: {other}"))),
    }
}

async fn respond(
    request: Value,
    client: Arc<Mutex<Option<String>>>,
    out: Arc<Mutex<tokio::io::Stdout>>,
) {
    let id = request["id"].clone();
    let method = request["method"].as_str().unwrap_or("").to_string();
    // A request without an id is a notification and gets no reply.
    if id.is_null() {
        return;
    }
    let reply = match handle(&method, &request["params"], &client).await {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
        }
    };
    write_message(&out, &reply).await;
}

async fn write_message(out: &Mutex<tokio::io::Stdout>, message: &Value) {
    let mut out = out.lock().await;
    let line = format!("{message}\n");
    let _ = out.write_all(line.as_bytes()).await;
    let _ = out.flush().await;
}

/// Serve until stdin closes. Requests run concurrently, so a long
/// `run_workflow` doesn't stall `ping`.
pub(crate) async fn serve() {
    let client = Arc::new(Mutex::new(None));
    let out = Arc::new(Mutex::new(tokio::io::stdout()));
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut tasks = tokio::task::JoinSet::new();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                tasks.spawn(respond(request, client.clone(), out.clone()));
            }
            Err(e) => {
                let reply = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}});
                write_message(&out, &reply).await;
            }
        }
    }
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_flag_stays_ahead_of_the_path_separator() {
        let (cli, _) =
            tool_command("run_workflow", &json!({"path": "--evil", "job": "a"})).unwrap();
        let separator = cli.iter().position(|a| a == "--").unwrap();
        assert_eq!(cli[separator + 1], "--evil");
        assert!(cli.iter().position(|a| a == "--format").unwrap() < separator);
    }

    #[test]
    fn missing_path_and_unknown_tools_are_errors() {
        assert!(tool_command("run_workflow", &json!({})).is_err());
        assert!(tool_command("nope", &json!({})).is_err());
    }

    #[test]
    fn negotiates_a_known_protocol_or_the_newest() {
        assert_eq!(negotiate(Some("2024-11-05")), "2024-11-05");
        assert_eq!(negotiate(Some("1999-01-01")), SUPPORTED_PROTOCOLS[0]);
    }

    #[test]
    fn tool_result_returns_the_last_object() {
        let stdout =
            "{\"type\":\"run_started\"}\n{\"type\":\"run_finished\",\"status\":\"failure\"}\n";
        let result = tool_result(stdout, "", false);
        assert_eq!(result["isError"], false);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("run_finished"));
    }

    #[test]
    fn tool_result_reports_a_silent_failure() {
        let result = tool_result("", "clap: bad flag", false);
        assert_eq!(result["isError"], true);
        assert!(result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("bad flag"));
    }
}
