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
