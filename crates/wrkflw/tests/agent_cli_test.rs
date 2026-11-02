//! End-to-end checks of the agent-facing surface, through the real binary:
//! stdout contracts, run history, failure digests, replay, plan and MCP.
//! Fixtures use POSIX shell steps, so this file only runs on Unix.
#![cfg(unix)]

use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_wrkflw");

const FAILING: &str = r#"
name: failing
on:
  push:
    paths: ['src/**']
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - name: export
        run: echo "STAGE=two" >> "$GITHUB_ENV"
      - name: leak
        env:
          SECRET_VALUE: ${{ secrets.WRKFLW_TEST_TOKEN }}
        run: echo "token is $SECRET_VALUE"
      - name: boom
        run: |
          echo before failing
          exit 7
      - name: never
        run: echo unreachable
"#;

const PASSING: &str = r#"
name: passing
on: [push]
jobs:
  first:
    runs-on: ubuntu-latest
    steps:
      - run: echo one
  second:
    needs: first
    runs-on: ubuntu-latest
    strategy:
      matrix:
        n: [1, 2]
    steps:
      - name: marker
        run: echo "ran-${{ matrix.n }}"
      - name: last
        run: echo "last-step-ran"
"#;

const BROKEN: &str = "name: broken\non: [nonsense]\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: nothing\n";

struct Sandbox {
    home: TempDir,
    repo: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            home: TempDir::new().unwrap(),
            repo: TempDir::new().unwrap(),
        };
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(sandbox.repo.path())
                .output()
                .unwrap();
        };
        git(&["init", "-q"]);
        fs::create_dir_all(sandbox.repo.path().join(".github/workflows")).unwrap();
        sandbox.write("failing.yml", FAILING);
        sandbox.write("passing.yml", PASSING);
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ]);
        sandbox
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.repo.path().join(".github/workflows").join(name);
        fs::write(&path, text).unwrap();
        path
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command
            .args(args)
            .current_dir(self.repo.path())
            .env("WRKFLW_HOME", self.home.path())
            .env("WRKFLW_SECRET_WRKFLW_TEST_TOKEN", "hunter2-very-secret")
            .env("WRKFLW_TEST_TOKEN", "hunter2-very-secret")
            .env_remove("WRKFLW_AGENT")
            .env_remove("CLAUDECODE")
            .env_remove("WRKFLW_INTENT");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Every stored byte, for secret scanning.
    fn stored_text(&self) -> String {
        fn walk(dir: &Path, out: &mut String) {
            for entry in fs::read_dir(dir).unwrap().flatten() {
                if entry.path().is_dir() {
                    walk(&entry.path(), out);
                } else {
                    out.push_str(&fs::read_to_string(entry.path()).unwrap_or_default());
                }
            }
        }
        let mut text = String::new();
        walk(self.home.path(), &mut text);
        text
    }
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Parse stdout strictly: one stray line of text fails the test.
fn json_lines(output: &Output) -> Vec<Value> {
    stdout(output)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}")))
        .collect()
}

fn json_doc(output: &Output) -> Value {
    let mut lines = json_lines(output);
    assert_eq!(lines.len(), 1, "expected one document: {}", stdout(output));
    lines.remove(0)
}

const RUN_FAILING: &[&str] = &[
    "--format",
    "json",
    "run",
    "--runtime",
    "emulation",
    ".github/workflows/failing.yml",
];

#[test]
fn run_streams_only_json_and_reports_the_failure() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(RUN_FAILING);
    assert_eq!(output.status.code(), Some(1));

    let events = json_lines(&output);
    let kinds: Vec<&str> = events.iter().filter_map(|e| e["type"].as_str()).collect();
    assert_eq!(kinds.first(), Some(&"run_started"));
    assert_eq!(kinds.last(), Some(&"run_finished"));
    assert!(kinds.contains(&"job_started") && kinds.contains(&"job_finished"));
    assert!(events.iter().all(|e| e["schema_version"] == 1));
    let run_id = &events[0]["run_id"];
    assert!(events.iter().all(|e| &e["run_id"] == run_id));

    let boom = events
        .iter()
        .find(|e| e["type"] == "step_finished" && e["step"] == "boom")
        .unwrap();
    assert_eq!(boom["exit_code"], 7);
    assert_eq!(boom["status"], "failure");
    assert!(!events.iter().any(|e| e["step"] == "never"));

    let finished = events.last().unwrap();
    assert_eq!(finished["status"], "failure");
    assert_eq!(finished["exit_code"], 1);
    let failure = &finished["failure"]["failures"][0];
    assert_eq!(failure["job"], "build");
    assert_eq!(failure["step"], "boom");
    assert_eq!(failure["exit_code"], 7);
    assert!(failure["command"].as_str().unwrap().contains("exit 7"));
    assert!(failure["output_tail"]
        .to_string()
        .contains("before failing"));
    assert_eq!(
        failure["env_from_earlier_steps"][0],
        json!(["STAGE", "two"])
    );
}

#[test]
fn history_records_runs_and_answers_without_a_rerun() {
    let sandbox = Sandbox::new();
    sandbox
        .command(RUN_FAILING)
        .args(["--intent", "check the exit code", "--agent", "tester"])
        .output()
        .unwrap();

    let listed = json_doc(&sandbox.run(&["--format", "json", "runs", "list"]));
    let run = &listed["runs"][0];
    assert_eq!(run["status"], "failure");
    assert_eq!(run["agent"], "tester");
    assert_eq!(run["intent"], "check the exit code");
    assert_eq!(run["source"], "cli");
    assert_eq!(run["git_sha"].as_str().unwrap().len(), 40);

    let shown = json_doc(&sandbox.run(&["--format", "json", "runs", "show"]));
    assert_eq!(shown["id"], run["id"]);
    assert_eq!(shown["jobs"][0]["steps"][2]["exit_code"], 7);

    let failure = json_doc(&sandbox.run(&["--format", "json", "runs", "failure"]));
    assert_eq!(failure["failures"][0]["step"], "boom");
    let log = failure["failures"][0]["log_path"].as_str().unwrap();
    assert!(fs::read_to_string(log).unwrap().contains("before failing"));

    let logs = json_doc(&sandbox.run(&[
        "--format", "json", "runs", "logs", "--step", "boom", "--tail", "1",
    ]));
    assert_eq!(logs["logs"].as_array().unwrap().len(), 1);

    let filtered = json_doc(&sandbox.run(&[
        "--format",
        "json",
        "runs",
        "list",
        "--agent",
        "someone-else",
    ]));
    assert_eq!(filtered["runs"], json!([]));

    let text = stdout(&sandbox.run(&["runs", "failure"]));
    assert!(text.contains("Failed: build / boom (exit 7)"), "{text}");
}

#[test]
fn listings_are_scoped_to_the_repository() {
    let sandbox = Sandbox::new();
    sandbox.run(RUN_FAILING);
    let other = TempDir::new().unwrap();
    let run = |args: &[&str]| {
        sandbox
            .command(args)
            .current_dir(other.path())
            .output()
            .unwrap()
    };
    assert_eq!(
        json_doc(&run(&["--format", "json", "runs", "list"]))["runs"],
        json!([])
    );
    assert_eq!(
        json_doc(&run(&["--format", "json", "runs", "list", "--all"]))["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(run(&["runs", "show", "latest"]).status.code(), Some(1));
}

#[test]
fn secrets_never_reach_output_or_the_store() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(RUN_FAILING);
    assert!(
        sandbox.stored_text().contains("token is ***"),
        "secret did not resolve, so this proves nothing"
    );
    assert!(!sandbox.stored_text().contains("hunter2-very-secret"));
    assert!(!stdout(&output).contains("hunter2-very-secret"));
}

#[test]
fn variables_are_not_stored_and_replay_keeps_the_arguments() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.repo.path().join(".gitlab-ci.yml"),
        "stages: [test]\nunit:\n  stage: test\n  script:\n    - echo hi\n",
    )
    .unwrap();
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "-V",
        "API_KEY=topsecretvalue",
        ".gitlab-ci.yml",
    ]);
    let first = json_lines(&output)[0]["run_id"].clone();
    assert!(!sandbox.stored_text().contains("topsecretvalue"));
    let shown = json_doc(&sandbox.run(&["--format", "json", "runs", "show"]));
    assert_eq!(shown["variables"], json!(["API_KEY"]));

    let replay = sandbox.run(&["--format", "json", "runs", "replay", "-V", "API_KEY=again"]);
    let events = json_lines(&replay);
    assert_ne!(events[0]["run_id"], first);
    let shown = json_doc(&sandbox.run(&["--format", "json", "runs", "show"]));
    assert_eq!(shown["replay_of"], first);
}

#[test]
fn variable_values_printed_by_a_step_are_masked() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.repo.path().join(".gitlab-ci.yml"),
        "stages: [test]\nunit:\n  stage: test\n  script:\n    - echo \"key is $API_KEY\"\n",
    )
    .unwrap();
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "-V",
        "API_KEY=topsecretvalue",
        ".gitlab-ci.yml",
    ]);
    let events = stdout(&output);
    assert!(events.contains("key is ***"), "{events}");
    assert!(!events.contains("topsecretvalue"));
    assert!(!sandbox.stored_text().contains("topsecretvalue"));
}

#[test]
fn replay_works_after_a_path_separator() {
    let sandbox = Sandbox::new();
    let first = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "--",
        ".github/workflows/passing.yml",
    ]);
    let first_id = json_lines(&first)[0]["run_id"].clone();

    let replay = sandbox.run(&["--format", "json", "runs", "replay"]);
    assert!(replay.status.success(), "{}", stdout(&replay));
    let events = json_lines(&replay);
    assert_ne!(events[0]["run_id"], first_id);
    let shown = json_doc(&sandbox.run(&["--format", "json", "runs", "show"]));
    assert_eq!(shown["replay_of"], first_id);
}

#[test]
fn replay_with_variables_works_after_a_path_separator() {
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.repo.path().join(".gitlab-ci.yml"),
        "stages: [test]\nunit:\n  stage: test\n  script:\n    - echo \"$API_KEY\"\n",
    )
    .unwrap();
    sandbox.run(&[
        "run",
        "--runtime",
        "emulation",
        "-V",
        "API_KEY=first",
        "--",
        ".gitlab-ci.yml",
    ]);
    let replay = sandbox.run(&["--format", "json", "runs", "replay", "-V", "API_KEY=again"]);
    assert!(replay.status.success(), "{}", stdout(&replay));
    json_lines(&replay);
}

#[test]
fn matrix_legs_with_similar_names_keep_their_own_logs() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "legs.yml",
        r#"
name: legs
on: [push]
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        leg: ["a/b", "a-b"]
    steps:
      - name: show
        run: echo "leg ${{ matrix.leg }}"
"#,
    );
    sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        ".github/workflows/legs.yml",
    ]);
    let logs = json_doc(&sandbox.run(&["--format", "json", "runs", "logs", "--step", "show"]));
    let texts: Vec<&str> = logs["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts.len(), 2);
    assert!(texts.iter().any(|t| t.contains("leg a/b")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("leg a-b")), "{texts:?}");
}

#[test]
fn no_record_leaves_no_history_but_keeps_the_digest() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command(RUN_FAILING)
        .arg("--no-record")
        .output()
        .unwrap();
    let finished = json_lines(&output).pop().unwrap();
    assert_eq!(finished["failure"]["failures"][0]["step"], "boom");
    assert!(finished["failure"]["failures"][0]["output_tail"]
        .to_string()
        .contains("before failing"));
    assert_eq!(finished["failure"]["replay"], "");
    let listed = json_doc(&sandbox.run(&["--format", "json", "runs", "list"]));
    assert_eq!(listed["runs"], json!([]));
}

#[test]
fn retention_prunes_the_oldest_runs() {
    let sandbox = Sandbox::new();
    for _ in 0..3 {
        sandbox
            .command(RUN_FAILING)
            .env("WRKFLW_RUNS_KEEP", "2")
            .output()
            .unwrap();
    }
    let count = fs::read_dir(sandbox.home.path().join("runs"))
        .unwrap()
        .count();
    assert_eq!(count, 2);
}

#[test]
fn from_step_starts_at_the_named_step() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "--job",
        "build",
        "--from-step",
        "boom",
        ".github/workflows/failing.yml",
    ]);
    let events = json_lines(&output);
    let status = |name: &str| {
        events
            .iter()
            .find(|e| e["type"] == "step_finished" && e["step"] == name)
            .map(|e| e["status"].clone())
    };
    assert_eq!(status("export"), Some(json!("skipped")));
    assert_eq!(status("boom"), Some(json!("failure")));

    let bad = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "--job",
        "build",
        "--from-step",
        "typo",
        ".github/workflows/failing.yml",
    ]);
    assert_eq!(bad.status.code(), Some(1));
    assert_eq!(json_doc(&bad)["type"], "error");
}

#[test]
fn plan_explains_without_executing() {
    let sandbox = Sandbox::new();
    let plan = json_doc(&sandbox.run(&[
        "--format",
        "json",
        "plan",
        "--event",
        "push",
        "--changed-files",
        "docs/readme.md",
        ".github/workflows/failing.yml",
    ]));
    assert_eq!(plan["would_run"], false);
    assert_eq!(plan["trigger"]["matches"], false);
    assert!(!plan["trigger"]["reason"].as_str().unwrap().is_empty());

    let plan =
        json_doc(&sandbox.run(&["--format", "json", "plan", ".github/workflows/passing.yml"]));
    assert_eq!(plan["would_run"], true);
    assert_eq!(plan["batches"], json!([["first"], ["second"]]));
    assert_eq!(
        plan["jobs"][1]["matrix"]["combinations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let listed = json_doc(&sandbox.run(&["--format", "json", "runs", "list"]));
    assert_eq!(listed["runs"], json!([]), "plan must not execute or record");
}

#[test]
fn validate_reports_structured_issues_and_keeps_exit_codes() {
    let sandbox = Sandbox::new();
    let path = sandbox.write("broken.yml", BROKEN);
    let output = sandbox.run(&["--format", "json", "validate", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(1));
    let report = json_doc(&output);
    assert_eq!(report["valid"], false);
    let issues = report["files"][0]["issues"].as_array().unwrap();
    let rules: Vec<&str> = issues.iter().filter_map(|i| i["rule"].as_str()).collect();
    assert!(rules.contains(&"unknown-trigger"), "{rules:?}");
    let step = issues
        .iter()
        .find(|i| i["rule"] == "step-missing-action")
        .unwrap();
    assert_eq!(step["path"], "jobs.build.steps[0]");
    assert_eq!(step["line"], 7);
    assert!(step["hint"].is_string());

    let output = sandbox.run(&[
        "--format",
        "json",
        "validate",
        "--no-exit-code",
        path.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(0));
    let output = sandbox.run(&[
        "--format",
        "json",
        "--verbose",
        "validate",
        ".github/workflows/passing.yml",
    ]);
    assert_eq!(json_doc(&output)["valid"], true);
}

#[test]
fn list_reports_workflows_as_json() {
    let sandbox = Sandbox::new();
    let listed = json_doc(&sandbox.run(&["--format", "json", "list"]));
    let workflows = listed["workflows"].as_array().unwrap();
    assert_eq!(workflows.len(), 2);
    assert!(workflows.iter().any(|w| w["jobs"] == json!(["build"])));
}

/// Drive `wrkflw mcp serve` the way a client does.
struct Mcp {
    child: std::process::Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(sandbox: &Sandbox) -> Self {
        let mut child = sandbox
            .command(&["mcp", "serve"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            reader,
            next_id: 0,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    fn tool(&mut self, name: &str, arguments: Value) -> (Value, bool) {
        let reply = self.call("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        let body = serde_json::from_str(text).unwrap_or(json!(text));
        (body, result["isError"] == true)
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_server_speaks_the_protocol_and_runs_tools() {
    let sandbox = Sandbox::new();
    let mut mcp = Mcp::start(&sandbox);

    let init = mcp.call(
        "initialize",
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "test-client", "version": "1"},
        }),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "wrkflw");
    mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let tools = mcp.call("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names.len(), 8);
    assert!(names.contains(&"run_workflow"));

    let (report, is_error) = mcp.tool("validate_workflows", json!({}));
    assert!(!is_error);
    assert_eq!(report["type"], "validation");

    let (plan, _) = mcp.tool(
        "plan_workflow",
        json!({"path": ".github/workflows/passing.yml"}),
    );
    assert_eq!(plan["type"], "plan");

    let (finished, is_error) = mcp.tool(
        "run_workflow",
        json!({
            "path": ".github/workflows/failing.yml",
            "runtime": "emulation",
            "intent": "mcp test",
        }),
    );
    assert!(!is_error, "a failed workflow is a result, not a tool error");
    assert_eq!(finished["status"], "failure");
    assert_eq!(finished["failure"]["failures"][0]["exit_code"], 7);

    let (runs, _) = mcp.tool("list_runs", json!({}));
    assert_eq!(runs["runs"][0]["source"], "mcp");
    assert_eq!(runs["runs"][0]["agent"], "test-client");
    assert_eq!(runs["runs"][0]["intent"], "mcp test");

    let (failure, _) = mcp.tool("get_run_failure", json!({}));
    assert_eq!(failure["failures"][0]["step"], "boom");
    let (logs, _) = mcp.tool("get_run_logs", json!({"step": "boom", "tail": 2}));
    assert_eq!(logs["type"], "logs");

    let (_, is_error) = mcp.tool("run_workflow", json!({}));
    assert!(is_error);
    assert_eq!(mcp.call("nope/nope", json!({}))["error"]["code"], -32601);
    assert_eq!(mcp.call("ping", json!({}))["result"], json!({}));
}

#[test]
fn mcp_timeout_stops_the_run_and_replay_of_an_mcp_run_works() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "slow.yml",
        "name: slow\non: [push]\njobs:\n  slow:\n    runs-on: ubuntu-latest\n    steps:\n      - run: sleep 8\n",
    );
    let mut mcp = Mcp::start(&sandbox);
    let started = std::time::Instant::now();
    let (message, is_error) = mcp.tool(
        "run_workflow",
        json!({
            "path": ".github/workflows/slow.yml",
            "runtime": "emulation",
            "timeout_seconds": 2,
        }),
    );
    assert!(is_error);
    assert!(message.as_str().unwrap().contains("timed out"));
    assert!(started.elapsed() < std::time::Duration::from_secs(20));

    let (run, _) = mcp.tool("get_run", json!({}));
    assert_eq!(run["status"], "interrupted");

    let (finished, _) = mcp.tool(
        "run_workflow",
        json!({"path": ".github/workflows/passing.yml", "runtime": "emulation"}),
    );
    assert_eq!(finished["status"], "success");
    let replay = sandbox.run(&["--format", "json", "runs", "replay"]);
    assert!(replay.status.success(), "{}", stdout(&replay));
    json_lines(&replay);
}

#[test]
fn text_mode_prints_the_digest_on_failure() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "run",
        "--runtime",
        "emulation",
        ".github/workflows/failing.yml",
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed: build / boom (exit 7)"), "{stderr}");
    assert!(stderr.contains("wrkflw runs replay"), "{stderr}");
}

#[test]
fn watch_streams_json_cycles_and_records_runs() {
    let sandbox = Sandbox::new();
    let mut child = sandbox
        .command(&[
            "--format",
            "json",
            "watch",
            "--runtime",
            "emulation",
            "--debounce",
            "100",
            ".github/workflows",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let started: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(started["type"], "watch_started");

    std::thread::sleep(std::time::Duration::from_millis(500));
    fs::create_dir_all(sandbox.repo.path().join("src")).unwrap();
    fs::write(sandbox.repo.path().join("src/app.rs"), "// change").unwrap();

    let mut saw_cycle = false;
    let mut saw_finished = false;
    while !(saw_cycle && saw_finished) {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let event: Value =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON ({e}): {line}"));
        saw_cycle |= event["type"] == "watch_cycle";
        saw_finished |= event["type"] == "run_finished";
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(saw_cycle && saw_finished);

    let listed = json_doc(&sandbox.run(&["--format", "json", "runs", "list"]));
    assert_eq!(listed["runs"][0]["source"], "watch");
}

#[test]
fn a_missing_secret_is_empty_and_named_as_on_github() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "secret.yml",
        "name: s\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - name: read\n        run: echo \"value=${{ secrets.WRKFLW_TEST_UNSET }}.\"\n",
    );
    let path = ".github/workflows/secret.yml";

    let plan = json_doc(&sandbox.run(&["--format", "json", "plan", path]));
    assert_eq!(plan["missing_secrets"], json!(["WRKFLW_TEST_UNSET"]));

    let output = sandbox.run(&["--format", "json", "run", "--runtime", "emulation", path]);
    let events = json_lines(&output);
    assert_eq!(events.last().unwrap()["status"], "success");
    let step = events
        .iter()
        .find(|e| e["type"] == "step_finished" && e["step"] == "read")
        .unwrap();
    let text = step["output"].as_str().unwrap();
    assert!(text.contains("value=."), "{text}");
    assert!(text.contains("WRKFLW_TEST_UNSET"), "{text}");
}

#[test]
fn identical_matrix_failures_fold_and_paths_are_relative() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "wide.yml",
        "name: w\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n    strategy:\n      fail-fast: false\n      matrix:\n        n: [1, 2, 3]\n    steps:\n      - name: check\n        run: |\n          echo \"failed at $PWD/src/lib.rs\"\n          exit 1\n",
    );
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        ".github/workflows/wide.yml",
    ]);
    let events = json_lines(&output);
    let failures = &events.last().unwrap()["failure"]["failures"];
    assert_eq!(failures.as_array().unwrap().len(), 1, "{failures}");
    assert_eq!(failures[0]["also_failed_in"].as_array().unwrap().len(), 2);
    let tail = failures[0]["output_tail"].to_string();
    assert!(tail.contains("failed at src/lib.rs"), "{tail}");
}

#[test]
fn unnamed_action_steps_report_the_action() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "checkout.yml",
        "name: c\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n",
    );
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        ".github/workflows/checkout.yml",
    ]);
    assert!(json_lines(&output)
        .iter()
        .any(|e| e["step"] == "actions/checkout@v4"));
}

#[test]
fn from_step_failures_say_earlier_steps_did_not_run() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "--job",
        "build",
        "--from-step",
        "boom",
        ".github/workflows/failing.yml",
    ]);
    let events = json_lines(&output);
    let note = events.last().unwrap()["failure"]["note"].to_string();
    assert!(note.contains("'boom'"), "{note}");
}

#[test]
fn a_missing_workflow_is_an_error_and_not_a_run() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["--format", "json", "run", "nope.yml"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json_doc(&output)["type"], "error");
    assert!(sandbox.stored_text().is_empty());
}

#[test]
fn usage_errors_are_json_in_json_mode() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["--format", "json", "run", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json_doc(&output)["type"], "error");

    let output = sandbox.run(&["run", "--bogus"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
}

#[test]
fn validate_catches_filter_and_expression_mistakes() {
    let sandbox = Sandbox::new();
    let path = sandbox.write(
        "typos.yml",
        "name: t\non:\n  pull_request:\n    branch: [main]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ${{ steps.nope.outputs.x }}\n      - run: echo ${{ github.sha\n",
    );
    let output = sandbox.run(&["--format", "json", "validate", path.to_str().unwrap()]);
    let report = json_doc(&output);
    let rules = report["files"][0]["issues"].to_string();
    for rule in [
        "unknown-trigger-filter",
        "undefined-step-reference",
        "unterminated-expression",
    ] {
        assert!(rules.contains(rule), "{rule} in {rules}");
    }
}

#[test]
fn a_provider_failure_is_not_reported_as_a_missing_secret() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "broken.yml",
        "name: b\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - name: read\n        run: echo ${{ secrets.no_such_provider:TOKEN }}\n",
    );
    let path = ".github/workflows/broken.yml";

    let plan = json_doc(&sandbox.run(&["--format", "json", "plan", path]));
    assert_eq!(plan["missing_secrets"], json!([]));

    let output = sandbox.run(&["--format", "json", "run", "--runtime", "emulation", path]);
    let events = json_lines(&output);
    assert_eq!(events.last().unwrap()["status"], "failure");
    let step = events
        .iter()
        .find(|e| e["type"] == "step_finished" && e["step"] == "read")
        .unwrap();
    let text = step["output"].as_str().unwrap();
    assert!(text.contains("Secret substitution failed"), "{text}");
    assert!(!text.contains("not set"), "{text}");
}

#[test]
fn a_secret_missing_from_a_named_provider_keeps_its_provider() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "named.yml",
        "name: n\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - name: read\n        run: echo ${{ secrets.env:WRKFLW_TEST_UNSET }}\n",
    );
    let path = ".github/workflows/named.yml";

    let plan = json_doc(&sandbox.run(&["--format", "json", "plan", path]));
    assert_eq!(plan["missing_secrets"], json!(["env:WRKFLW_TEST_UNSET"]));

    let output = sandbox.run(&["--format", "json", "run", "--runtime", "emulation", path]);
    let events = json_lines(&output);
    let step = events
        .iter()
        .find(|e| e["type"] == "step_finished" && e["step"] == "read")
        .unwrap();
    let text = step["output"].as_str().unwrap();
    assert!(text.contains("env:WRKFLW_TEST_UNSET"), "{text}");
    assert!(text.contains("to that provider"), "{text}");
    assert!(!text.contains("environment variable"), "{text}");
}

#[test]
fn matrix_failures_fold_when_only_the_script_differs() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "scripted.yml",
        "name: s\non: [push]\njobs:\n  t:\n    runs-on: ubuntu-latest\n    strategy:\n      fail-fast: false\n      matrix:\n        n: [1, 2, 3]\n    steps:\n      - name: check\n        run: |\n          : ${{ matrix.n }}\n          echo boom\n          exit 1\n",
    );
    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        ".github/workflows/scripted.yml",
    ]);
    let events = json_lines(&output);
    let failures = &events.last().unwrap()["failure"]["failures"];
    assert_eq!(failures.as_array().unwrap().len(), 1, "{failures}");
    assert_eq!(failures[0]["also_failed_in"].as_array().unwrap().len(), 2);
}

#[test]
fn a_reported_step_name_can_be_passed_back_to_from_step() {
    let sandbox = Sandbox::new();
    sandbox.write(
        "unnamed.yml",
        "name: u\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: echo one\n      - run: exit 1\n",
    );
    let path = ".github/workflows/unnamed.yml";
    let output = sandbox.run(&["--format", "json", "run", "--runtime", "emulation", path]);
    let events = json_lines(&output);
    let reported = events.last().unwrap()["failure"]["failures"][0]["step"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(reported, "Step 3");

    let output = sandbox.run(&[
        "--format",
        "json",
        "run",
        "--runtime",
        "emulation",
        "--job",
        "a",
        "--from-step",
        &reported,
        path,
    ]);
    let events = json_lines(&output);
    let note = events.last().unwrap()["failure"]["note"].to_string();
    assert!(note.contains("Step 3"), "{note}");
}

#[test]
fn undefined_references_warn_without_failing_validation() {
    let sandbox = Sandbox::new();
    let path = sandbox.write(
        "warned.yml",
        "name: w\non: [push]\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ${{ steps.nope.outputs.x }}\n",
    );
    let output = sandbox.run(&["--format", "json", "validate", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0));
    let report = json_doc(&output);
    assert_eq!(report["files"][0]["valid"], true);
    assert_eq!(report["files"][0]["issues"][0]["severity"], "warning");
    assert_eq!(
        report["files"][0]["issues"][0]["rule"],
        "undefined-step-reference"
    );
}
