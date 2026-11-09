//! `wrkflw mcp serve`: a Model Context Protocol server over stdio.
//!
//! Tools run the wrkflw binary itself with `--format json`. A child process
//! keeps stdout, which here is the protocol, safe from anything the engine
//! prints, and contains the `process::exit` calls in the command paths. It
//! also means a tool result is exactly what the CLI's JSON contract promises.

use serde_json::{json, Value};
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

const SUPPORTED_PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const DEFAULT_RUN_TIMEOUT_SECS: u64 = 1800;
/// The gate runs every workflow one after another, so its budget covers
/// all of them, not one run.
const DEFAULT_GATE_TIMEOUT_SECS: u64 = 3600;
/// Longest a read-only tool may take. These only parse files and read history.
const QUICK_TIMEOUT_SECS: u64 = 120;
/// How long a timed-out child gets to clean up. Longer than the 10 seconds
/// after which its own signal handler forces an exit.
const STOP_GRACE: Duration = Duration::from_secs(15);

/// Tool calls still running, by pid. Each leads its own process group,
/// outside the terminal's, so a signal to the server reaches them only
/// through [`stop_groups`].
static LIVE: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

fn live_pids() -> Vec<u32> {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Keeps a child's pid in [`LIVE`] until the child has been waited for.
struct Live(Option<u32>);

impl Live {
    fn track(pid: Option<u32>) -> Self {
        if let Some(pid) = pid {
            LIVE.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
        }
        Self(pid)
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            LIVE.lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|live| *live != pid);
        }
    }
}

const INSTRUCTIONS: &str =
    "wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. \
Call validate_workflows after editing YAML, plan_workflow to see what would run and why a trigger \
would or would not fire, and run_workflow to execute. run_workflow returns a failure digest. Pass \
reuse_results while iterating, so jobs that already passed on the same files aren't run again. \
Before pushing, call gate: it runs what CI would run for the push, and on a feature branch \
checks workflows a push doesn't fire as a pull request into the default branch. Read \
`verdict`, not only `passed`: `verified` means everything ran as asked, `partial` means some \
jobs ran as another OS than their runs-on asked for (listed under `unverified`, so their \
OS-specific steps weren't checked), and `nothing_checked` means no job ran at all. Say which \
before claiming CI will pass. A job with `reused` didn't run: an earlier result stands. Use \
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
    if args["no_strict_filter"].as_bool() == Some(true) {
        cli.push("--no-strict-filter".to_string());
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
            push_opt(&mut cli, "--runtime", string_arg(args, "runtime"));
            cli.push("--".into());
            cli.push(required_path(args)?);
        }
        "run_workflow" => {
            cli.push("run".into());
            event_flags(&mut cli, args);
            push_opt(&mut cli, "--runtime", string_arg(args, "runtime"));
            push_opt(&mut cli, "--intent", string_arg(args, "intent"));
            push_opt(&mut cli, "--session", string_arg(args, "session"));
            push_opt(&mut cli, "--from-step", string_arg(args, "from_step"));
            if args["reuse_results"].as_bool() == Some(true) {
                cli.push("--reuse-results".to_string());
            }
            for (key, value) in args["variables"].as_object().into_iter().flatten() {
                let value = value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string);
                cli.push("--variable".into());
                cli.push(format!("{key}={value}"));
            }
            cli.push("--".into());
            cli.push(required_path(args)?);
            timeout = args["timeout_seconds"]
                .as_u64()
                .unwrap_or(DEFAULT_RUN_TIMEOUT_SECS);
        }
        "gate" => {
            cli.push("gate".into());
            push_opt(&mut cli, "--event", string_arg(args, "event"));
            push_opt(&mut cli, "--base-branch", string_arg(args, "base_branch"));
            push_opt(&mut cli, "--diff-base", string_arg(args, "diff_base"));
            push_opt(&mut cli, "--runtime", string_arg(args, "runtime"));
            if let Some(path) = string_arg(args, "path") {
                cli.push("--".into());
                cli.push(path);
            }
            timeout = args["timeout_seconds"]
                .as_u64()
                .unwrap_or(DEFAULT_GATE_TIMEOUT_SECS);
        }
        "list_runs" => {
            cli.extend(["runs".to_string(), "list".to_string()]);
            push_opt(&mut cli, "--status", string_arg(args, "status"));
            push_opt(&mut cli, "--agent", string_arg(args, "agent"));
            push_opt(&mut cli, "--session", string_arg(args, "session"));
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
    let mut command = Command::new(exe);
    command
        .args(&cli)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(wrkflw_runs::env::MCP, "1")
        .kill_on_drop(true);
    // Its own process group, so a timeout reaches the `wrkflw run` children
    // that `gate` starts, not just `gate`.
    #[cfg(unix)]
    command.process_group(0);
    if let Some(cwd) = string_arg(args, "cwd") {
        command.current_dir(cwd);
    }
    if let Some(agent) = agent {
        if std::env::var_os(wrkflw_runs::env::AGENT).is_none() {
            command.env(wrkflw_runs::env::AGENT, agent);
        }
    }
    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return error_result(&format!("could not start wrkflw: {e}")),
    };
    match run_with_deadline(child, Duration::from_secs(timeout), STOP_GRACE).await {
        Some(Ok(output)) => tool_result(
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
            output.status.success(),
        ),
        Some(Err(e)) => error_result(&format!("could not read wrkflw's output: {e}")),
        None => error_result(&format!(
            "{name} timed out after {timeout}s and was stopped"
        )),
    }
}

/// Wait for `child`, or stop it once `limit` has passed and return `None`.
///
/// Its process group is interrupted first, because only the signal handler
/// of each wrkflw in it removes the containers it started. A kill of the
/// group is the fallback, with `kill_on_drop` for the child itself.
async fn run_with_deadline(
    child: tokio::process::Child,
    limit: Duration,
    grace: Duration,
) -> Option<std::io::Result<Output>> {
    let pid = child.id();
    let _live = Live::track(pid);
    let output = child.wait_with_output();
    tokio::pin!(output);
    if let Ok(result) = tokio::time::timeout(limit, &mut output).await {
        return Some(result);
    }
    if let Some(pid) = pid {
        signal_group(pid, Stop::Interrupt);
    }
    if tokio::time::timeout(grace, &mut output).await.is_err() {
        if let Some(pid) = pid {
            signal_group(pid, Stop::Kill);
        }
    }
    None
}

/// Interrupt the process groups `pids` lead, give them `grace` to remove
/// their containers and exit, then kill what is left.
async fn stop_groups(pids: &[u32], grace: Duration) {
    for pid in pids {
        signal_group(*pid, Stop::Interrupt);
    }
    let deadline = Instant::now() + grace;
    let running = || {
        let live = live_pids();
        pids.iter()
            .copied()
            .filter(|pid| live.contains(pid))
            .collect::<Vec<_>>()
    };
    while !running().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for pid in running() {
        signal_group(pid, Stop::Kill);
    }
}

/// The exit code for the first of SIGINT, SIGTERM or SIGHUP the server gets.
#[cfg(unix)]
async fn shutdown_signal() -> i32 {
    use tokio::signal::unix::{signal, SignalKind};
    async fn on(kind: SignalKind) {
        match signal(kind) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    }
    tokio::select! {
        _ = on(SignalKind::interrupt()) => 130,
        _ = on(SignalKind::terminate()) => 143,
        _ = on(SignalKind::hangup()) => 129,
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() -> i32 {
    let _ = tokio::signal::ctrl_c().await;
    130
}

#[derive(Clone, Copy)]
enum Stop {
    Interrupt,
    Kill,
}

/// Signal the process group `pid` leads, as a child started with
/// `process_group(0)` does, or `pid` alone when it leads none.
#[cfg(unix)]
fn signal_group(pid: u32, stop: Stop) {
    use nix::sys::signal::{kill, killpg, Signal};
    use nix::unistd::Pid;
    let signal = match stop {
        Stop::Interrupt => Signal::SIGINT,
        Stop::Kill => Signal::SIGKILL,
    };
    let pid = Pid::from_raw(pid as i32);
    if killpg(pid, signal).is_err() {
        let _ = kill(pid, signal);
    }
}

/// Windows can't send Ctrl+C to one child, so the grace period runs out and
/// the child is killed. Its containers outlive it there.
#[cfg(not(unix))]
fn signal_group(_pid: u32, _stop: Stop) {}

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
        "no_strict_filter": {"type": "boolean", "description": "Proceed without changed files even though the workflow has a `paths:` filter, which then cannot match."},
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
    run_properties["session"] = json!({"type": "string", "description": "Your session id, such as $CLAUDE_CODE_SESSION_ID or $CODEX_THREAD_ID. Stored on the run record."});
    run_properties["from_step"] = json!({"type": "string", "description": "Requires `job`. Start at this step, by name or 1-based position. Earlier steps' outputs and files are missing."});
    run_properties["variables"] = json!({"type": "object", "additionalProperties": {"type": "string"}, "description": "GitLab CI variables. Masked in output and never stored."});
    run_properties["reuse_results"] = json!({"type": "boolean", "description": "Reuse a job's earlier pass when its files, workflow and inputs are unchanged. Use it while iterating."});
    run_properties["timeout_seconds"] =
        json!({"type": "integer", "description": "Stop the run after this long. Default 1800."});
    let gate_properties = json!({
        "path": {"type": "string", "description": "Workflow file or directory. Defaults to the detected workflow directories."},
        "event": {"type": "string", "description": "Event to check as. Default: push, then pull_request into the default branch for workflows a push doesn't fire."},
        "base_branch": {"type": "string", "description": "Target branch, for pull_request. Default: origin/HEAD's branch."},
        "diff_base": {"type": "string", "description": "Ref the remote already has, for working out what changed."},
        "runtime": run_properties["runtime"].clone(),
        "timeout_seconds": {"type": "integer", "description": "Stop the whole gate, every workflow together, after this long. Default 3600."},
    });
    let mut plan_properties = event_properties();
    plan_properties["runtime"] = json!({
        "type": "string",
        "enum": ["auto", "docker", "podman", "kubernetes", "emulation", "secure-emulation"],
        "description": "Runtime a run would use. Decides which jobs `unverified` lists. `auto` may check whether Docker or Podman answers, which can take seconds; pass the runtime you will run with to skip that.",
    });
    let run_id = json!({"type": "string", "description": "Run id, unique prefix, or `latest`. Default `latest`."});
    json!([
        tool("validate_workflows", "Check workflow files. Returns issues with rule, YAML path, line and fix hint.", true,
            json!({"paths": {"type": "array", "items": {"type": "string"}, "description": "Files or directories. Defaults to the workflow directories."}}), &[]),
        tool("list_workflows", "List workflows and pipelines in the repository with their jobs.", true, json!({}), &[]),
        tool("plan_workflow", "Show what a run would do without executing anything: whether the trigger fires and why, job order, matrix legs, steps, and jobs that would run as another OS than runs-on asks for (`unverified`).", true, plan_properties, &["path"]),
        tool("run_workflow", "Execute a workflow locally. This runs the workflow's code. Returns status, job results and a failure digest. The run is recorded.", false, run_properties, &["path"]),
        tool("gate", "Run every workflow a push would trigger, reusing jobs that already passed on the same files. Returns `verdict` (verified, partial, nothing_checked or failed), `passed`, each workflow's status, the event it was checked as, run id and failure digest, and jobs that ran as another OS (`unverified`). Checks the working tree, not commits.", false, gate_properties, &[]),
        tool("list_runs", "List recorded runs of this repository, newest first.", true,
            json!({"limit": {"type": "integer"}, "status": {"type": "string"}, "agent": {"type": "string"}, "session": {"type": "string"}, "workflow": {"type": "string"}}), &[]),
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
///
/// Tool calls run in their own process groups, so the terminal's Ctrl+C and
/// hangup never reach them. On SIGINT, SIGTERM or SIGHUP the server passes
/// the interrupt on and waits for them before it exits, or their containers
/// would outlive it.
pub(crate) async fn serve() {
    tokio::spawn(async {
        let code = shutdown_signal().await;
        stop_groups(&live_pids(), STOP_GRACE).await;
        std::process::exit(code);
    });
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
    fn run_workflow_passes_variables_and_the_strict_filter_opt_out() {
        let args = json!({
            "path": "ci.yml",
            "no_strict_filter": true,
            "variables": {"DEPLOY": "yes"},
        });
        let (cli, _) = tool_command("run_workflow", &args).unwrap();
        let separator = cli.iter().position(|arg| arg == "--").unwrap();
        let flags = &cli[..separator];
        assert!(flags.contains(&"--no-strict-filter".to_string()));
        assert!(flags.windows(2).any(|f| f == ["--variable", "DEPLOY=yes"]));
    }

    #[test]
    fn format_flag_stays_ahead_of_the_path_separator() {
        let (cli, _) =
            tool_command("run_workflow", &json!({"path": "--evil", "job": "a"})).unwrap();
        let separator = cli.iter().position(|a| a == "--").unwrap();
        assert_eq!(cli[separator + 1], "--evil");
        assert!(cli.iter().position(|a| a == "--format").unwrap() < separator);
    }

    #[test]
    fn gate_and_reuse_reach_the_cli() {
        let (cli, timeout) = tool_command(
            "gate",
            &json!({"runtime": "docker", "base_branch": "main", "event": "pull_request", "path": "ci.yml"}),
        )
        .unwrap();
        assert_eq!(&cli[..3], ["--format", "json", "gate"]);
        assert!(cli.windows(2).any(|w| w == ["--runtime", "docker"]));
        assert!(cli.windows(2).any(|w| w == ["--base-branch", "main"]));
        assert_eq!(cli[cli.len() - 2..], ["--", "ci.yml"]);
        assert_eq!(timeout, DEFAULT_GATE_TIMEOUT_SECS);
        let (cli, _) = tool_command(
            "run_workflow",
            &json!({"path": "a.yml", "reuse_results": true}),
        )
        .unwrap();
        assert!(cli.contains(&"--reuse-results".to_string()));
        let (cli, _) = tool_command("run_workflow", &json!({"path": "a.yml"})).unwrap();
        assert!(!cli.contains(&"--reuse-results".to_string()));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timed_out_child_is_interrupted_before_it_is_killed() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("interrupted");
        let script = format!(
            "trap 'echo yes > {}; exit 0' INT; sleep 30 > /dev/null 2>&1 & wait",
            marker.display()
        );
        let child = Command::new("sh")
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let limit = Duration::from_millis(300);
        let outcome = run_with_deadline(child, limit, Duration::from_secs(5)).await;
        assert!(outcome.is_none());
        assert!(marker.exists(), "the child never saw SIGINT");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_timeout_interrupts_the_childs_own_children() {
        // As `gate` starts a `wrkflw run` per workflow: the grandchild must
        // hear the interrupt too, or it outlives the tool call.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild");
        let inner = format!(
            "trap 'echo yes > {}; exit 0' INT; sleep 30 > /dev/null 2>&1 & wait",
            marker.display()
        );
        // A foreground child, since a non-interactive shell's background
        // jobs ignore SIGINT. `; true` stops the shell exec-ing into it.
        let script = format!("sh -c \"{inner}\"; true");
        let mut command = Command::new("sh");
        command
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let child = command.spawn().unwrap();
        let limit = Duration::from_millis(300);
        let outcome = run_with_deadline(child, limit, Duration::from_secs(5)).await;
        assert!(outcome.is_none());
        assert!(marker.exists(), "the grandchild never saw SIGINT");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stopping_the_server_interrupts_a_running_tool_calls_group() {
        // What a SIGTERM to the server does: the tool call is well inside its
        // limit, and the grandchild must still hear the interrupt.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("grandchild");
        let inner = format!(
            "trap 'echo yes > {}; exit 0' INT; sleep 30 > /dev/null 2>&1 & wait",
            marker.display()
        );
        let script = format!("sh -c \"{inner}\"; true");
        let mut command = Command::new("sh");
        command
            .args(["-c", &script])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id().unwrap();
        let call = tokio::spawn(run_with_deadline(
            child,
            Duration::from_secs(60),
            Duration::from_secs(5),
        ));
        while !live_pids().contains(&pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Let the inner shell install its trap.
        tokio::time::sleep(Duration::from_millis(300)).await;
        stop_groups(&[pid], Duration::from_secs(5)).await;
        assert!(
            !live_pids().contains(&pid),
            "the tool call is still running"
        );
        assert!(marker.exists(), "the grandchild never saw SIGINT");
        let outcome = call.await.unwrap();
        assert!(
            outcome.is_some(),
            "the call should end on its own, not time out"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_ignoring_the_interrupt_is_killed_after_the_grace_period() {
        let child = Command::new("sh")
            .args(["-c", "trap '' INT; sleep 30 > /dev/null 2>&1 & wait; wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let short = Duration::from_millis(100);
        assert!(run_with_deadline(child, short, short).await.is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_child_inside_the_limit_returns_its_output() {
        let child = Command::new("sh")
            .args(["-c", "echo done"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let second = Duration::from_secs(5);
        let output = run_with_deadline(child, second, second).await.unwrap();
        assert_eq!(output.unwrap().stdout, b"done\n");
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
