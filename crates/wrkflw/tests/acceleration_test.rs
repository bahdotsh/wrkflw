//! `--reuse-results`, `--affected` and `gate`, through the
//! real binary, in a throwaway repository. Everything runs in emulation, so
//! these need no container runtime. Fixtures use POSIX shell steps, so this
//! file only runs on Unix.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

/// Stands in for the user's git config, so a global `core.hooksPath` or
/// hook can't change what these tests see or write.
const NO_GIT_CONFIG: &str = "/dev/null";

struct Repo {
    dir: TempDir,
    results: TempDir,
    remote: TempDir,
    log: PathBuf,
}

impl Repo {
    fn new(workflow: &str, sidecar: Option<&str>) -> Self {
        let dir = TempDir::new().unwrap();
        let log = dir.path().join("ran.log");
        let root = dir.path();
        fs::create_dir_all(root.join(".github/workflows")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join(".gitignore"), "ran.log\n").unwrap();
        fs::write(root.join("src/a"), "one").unwrap();
        fs::write(root.join("docs/a"), "one").unwrap();
        fs::write(
            root.join(".github/workflows/ci.yml"),
            workflow.replace("LOG", &log.display().to_string()),
        )
        .unwrap();
        if let Some(sidecar) = sidecar {
            fs::create_dir_all(root.join(".wrkflw")).unwrap();
            fs::write(root.join(".wrkflw/affected.yml"), sidecar).unwrap();
        }
        let repo = Self {
            dir,
            results: TempDir::new().unwrap(),
            remote: TempDir::new().unwrap(),
            log,
        };
        repo.git(&["init", "-q"]);
        repo.git(&["add", "."]);
        repo.git(&["commit", "-qm", "init"]);
        repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(self.root())
            .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn wrkflw(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_wrkflw"))
            .current_dir(self.root())
            .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("WRKFLW_RESULTS_DIR", self.results.path())
            .env("WRKFLW_NO_HISTORY", "1")
            .env("WRKFLW_CACHE_SIGNING_KEY", "test-key")
            .env_remove("WRKFLW_REMOTE_CACHE")
            .args(args)
            .output()
            .unwrap()
    }

    fn run(&self, flags: &[&str]) -> Output {
        let mut args = vec!["run", ".github/workflows/ci.yml", "--runtime", "emulation"];
        args.extend_from_slice(flags);
        self.wrkflw(&args)
    }

    /// Which jobs logged since the last call, sorted.
    fn take_ran(&self) -> Vec<String> {
        let text = fs::read_to_string(&self.log).unwrap_or_default();
        let _ = fs::remove_file(&self.log);
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        lines.sort();
        lines
    }
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

const TWO_JOBS: &str = r#"
name: ci
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo build >> LOG
  docs:
    runs-on: ubuntu-latest
    steps:
      - run: echo docs >> LOG
"#;

const HINTS: &str = r#"
jobs:
  build: {paths: ['src/**']}
  docs: {paths: ['docs/**']}
"#;

#[test]
fn a_second_run_of_the_same_tree_runs_nothing() {
    let repo = Repo::new(TWO_JOBS, None);
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["build", "docs"]);
    assert_ok(&repo.run(&["--reuse-results"]));
    assert!(repo.take_ran().is_empty());
}

#[test]
fn without_the_flag_every_job_runs() {
    let repo = Repo::new(TWO_JOBS, None);
    assert_ok(&repo.run(&["--reuse-results"]));
    repo.take_ran();
    assert_ok(&repo.run(&[]));
    assert_eq!(repo.take_ran(), ["build", "docs"]);
}

#[test]
fn a_change_reruns_only_the_jobs_that_read_it() {
    let repo = Repo::new(TWO_JOBS, Some(HINTS));
    assert_ok(&repo.run(&["--reuse-results"]));
    repo.take_ran();
    fs::write(repo.root().join("src/a"), "two").unwrap();
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["build"]);
}

#[test]
fn a_failed_job_is_never_reused() {
    let repo = Repo::new(
        "name: ci\non: push\njobs:\n  bad:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo bad >> LOG; exit 1\n",
        None,
    );
    assert!(!repo.run(&["--reuse-results"]).status.success());
    repo.take_ran();
    assert!(!repo.run(&["--reuse-results"]).status.success());
    assert_eq!(repo.take_ran(), ["bad"]);
}

#[test]
fn reused_outputs_still_reach_dependent_jobs() {
    let workflow = r#"
name: ci
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      v: ${{ steps.a.outputs.v }}
    steps:
      - id: a
        run: echo "v=42" >> "$GITHUB_OUTPUT"
  test:
    runs-on: ubuntu-latest
    needs: build
    steps:
      - run: test "${{ needs.build.outputs.v }}" = 42 && echo test >> LOG
"#;
    let repo = Repo::new(workflow, Some("jobs:\n  test: {cache: false}\n"));
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["test"]);
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["test"]);
}

#[test]
fn a_partial_run_neither_stores_nor_reuses() {
    let workflow = r#"
name: ci
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo one >> LOG
      - run: echo two >> LOG
"#;
    let repo = Repo::new(workflow, None);
    let partial = ["--reuse-results", "--job", "build", "--from-step", "2"];
    assert_ok(&repo.run(&partial));
    assert_eq!(repo.take_ran(), ["two"]);
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["one", "two"]);
}

#[test]
fn a_job_reading_secrets_always_runs() {
    let workflow = "name: ci\non: push\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo deploy >> LOG\n        env:\n          T: ${{ secrets.T }}\n";
    let repo = Repo::new(workflow, None);
    assert_ok(&repo.run(&["--reuse-results"]));
    repo.take_ran();
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["deploy"]);
}

#[test]
fn affected_skips_jobs_with_no_changed_input() {
    let repo = Repo::new(TWO_JOBS, Some(HINTS));
    fs::write(repo.root().join("docs/a"), "two").unwrap();
    assert_ok(&repo.run(&["--event", "push", "--affected", "--diff-base", "HEAD"]));
    assert_eq!(repo.take_ran(), ["docs"]);
}

#[test]
fn affected_sees_a_new_untracked_file() {
    let repo = Repo::new(TWO_JOBS, Some(HINTS));
    fs::write(repo.root().join("src/new"), "x").unwrap();
    assert_ok(&repo.run(&["--event", "push", "--affected", "--diff-base", "HEAD"]));
    assert_eq!(repo.take_ran(), ["build"]);
}

#[test]
fn a_reused_job_and_an_unaffected_job_report_differently() {
    let repo = Repo::new(TWO_JOBS, Some(HINTS));
    fs::write(repo.root().join("docs/a"), "two").unwrap();
    let output = repo.run(&["--event", "push", "--affected", "--diff-base", "HEAD"]);
    assert_ok(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Not affected"));
}

#[test]
fn affected_runs_a_job_without_hints() {
    let repo = Repo::new(TWO_JOBS, Some("jobs:\n  docs: {paths: ['docs/**']}\n"));
    fs::write(repo.root().join("src/a"), "two").unwrap();
    assert_ok(&repo.run(&["--event", "push", "--affected", "--diff-base", "HEAD"]));
    assert_eq!(repo.take_ran(), ["build"]);
}

#[test]
fn a_broken_sidecar_stops_the_run() {
    let repo = Repo::new(TWO_JOBS, Some("jobs:\n  build: {pathz: []}\n"));
    assert!(!repo.run(&["--reuse-results"]).status.success());
    assert!(repo.take_ran().is_empty());
}

#[test]
fn a_remote_cache_needs_a_signing_key() {
    let repo = Repo::new(TWO_JOBS, None);
    let output = Command::new(env!("CARGO_BIN_EXE_wrkflw"))
        .current_dir(repo.root())
        .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("WRKFLW_NO_HISTORY", "1")
        .env_remove("WRKFLW_CACHE_SIGNING_KEY")
        .args(["run", ".github/workflows/ci.yml", "--runtime", "emulation"])
        .args(["--reuse-results", "--remote-cache", "/tmp/nowhere"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("SIGNING_KEY"));
    assert!(repo.take_ran().is_empty());
}

#[test]
fn the_gate_fails_when_a_workflow_fails_and_passes_when_it_is_fixed() {
    let workflow = "name: ci\non: push\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: echo check >> LOG; test -f src/ok\n";
    let repo = Repo::new(workflow, None);
    let gate = ["gate", "--runtime", "emulation", "--diff-base", "HEAD"];
    assert!(!repo.wrkflw(&gate).status.success());
    fs::write(repo.root().join("src/ok"), "").unwrap();
    assert_ok(&repo.wrkflw(&gate));
    repo.take_ran();
    assert_ok(&repo.wrkflw(&gate));
    assert!(repo.take_ran().is_empty());
}

#[test]
fn the_gate_skips_a_workflow_the_event_does_not_trigger() {
    let workflow = "name: ci\non: workflow_dispatch\njobs:\n  check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo check >> LOG; exit 1\n";
    let repo = Repo::new(workflow, None);
    let output = repo.wrkflw(&["gate", "--runtime", "emulation", "--diff-base", "HEAD"]);
    assert_ok(&output);
    assert!(repo.take_ran().is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 workflow(s) ran, 1 skipped"));
}

#[test]
fn the_gate_counts_a_passing_workflow_as_run() {
    let repo = Repo::new(TWO_JOBS, None);
    let output = repo.wrkflw(&["gate", "--runtime", "emulation", "--diff-base", "HEAD"]);
    assert_ok(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 workflow(s) ran, 0 skipped"));
}

#[test]
fn an_emulated_result_is_not_pushed() {
    let repo = Repo::new(TWO_JOBS, None);
    let remote = repo.remote.path().display().to_string();
    assert_ok(&repo.run(&["--reuse-results", "--remote-cache", &remote, "--cache-push"]));
    assert_eq!(fs::read_dir(repo.remote.path()).unwrap().count(), 0);
}

#[test]
fn the_gate_installs_a_pre_push_hook() {
    let repo = Repo::new(TWO_JOBS, None);
    assert_ok(&repo.wrkflw(&["gate", "--install-hook"]));
    let path = repo.root().join(".git/hooks/pre-push");
    let hook = fs::read_to_string(&path).unwrap();
    assert!(hook.contains("wrkflw gate"));
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o111,
        0o111,
        "the hook must be executable: {:o}",
        mode
    );
}

#[test]
fn every_matrix_leg_is_reused_and_reaches_dependents() {
    let workflow = r#"
name: ci
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        n: [1, 2]
    steps:
      - run: echo "test ${{ matrix.n }}" >> LOG
  after:
    runs-on: ubuntu-latest
    needs: test
    steps:
      - run: test "${{ needs.test.result }}" = success && echo after >> LOG
"#;
    let repo = Repo::new(workflow, Some("jobs:\n  after: {cache: false}\n"));
    assert_ok(&repo.run(&["--reuse-results"]));
    assert_eq!(repo.take_ran(), ["after", "test 1", "test 2"]);
    let output = repo.run(&["--reuse-results"]);
    assert_ok(&output);
    assert_eq!(repo.take_ran(), ["after"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("Reuse result").count(), 2, "{}", stdout);
}

const DEPLOY: &str = r#"
name: ci
on: push
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: echo check >> LOG
  deploy:
    runs-on: ubuntu-latest
    needs: check
    steps:
      - run: echo deploy >> LOG; exit 1
"#;

#[test]
fn the_gate_leaves_a_job_to_ci_when_the_sidecar_says_so() {
    let repo = Repo::new(DEPLOY, Some("jobs:\n  deploy: {gate: false}\n"));
    let gate = ["gate", "--runtime", "emulation", "--diff-base", "HEAD"];
    let output = repo.wrkflw(&gate);
    assert_ok(&output);
    assert_eq!(repo.take_ran(), ["check"]);
    // A plain run still runs it.
    assert!(!repo.run(&[]).status.success());
    assert_eq!(repo.take_ran(), ["check", "deploy"]);
}

#[test]
fn the_gate_warns_about_uncommitted_changes() {
    let repo = Repo::new(TWO_JOBS, None);
    fs::write(repo.root().join("src/a"), "edited").unwrap();
    let output = repo.wrkflw(&["gate", "--runtime", "emulation", "--diff-base", "HEAD"]);
    assert_ok(&output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("uncommitted changes (src/a)"), "{}", stderr);
}

#[test]
fn a_step_never_sees_the_cache_credentials() {
    let workflow = "name: ci\non: push\njobs:\n  peek:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo \"${WRKFLW_CACHE_SIGNING_KEY:-none} ${WRKFLW_REMOTE_CACHE_TOKEN:-none}\" >> LOG\n";
    let repo = Repo::new(workflow, None);
    let output = Command::new(env!("CARGO_BIN_EXE_wrkflw"))
        .current_dir(repo.root())
        .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("WRKFLW_RESULTS_DIR", repo.results.path())
        .env("WRKFLW_NO_HISTORY", "1")
        .env("WRKFLW_CACHE_SIGNING_KEY", "test-key")
        .env("WRKFLW_REMOTE_CACHE_TOKEN", "test-token")
        .env_remove("WRKFLW_REMOTE_CACHE")
        .args(["run", ".github/workflows/ci.yml", "--runtime", "emulation"])
        .output()
        .unwrap();
    assert_ok(&output);
    assert_eq!(repo.take_ran(), ["none none"]);
}

#[test]
fn a_reused_job_with_new_outputs_still_reruns_its_dependents_under_affected() {
    let workflow = r#"
name: ci
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    outputs:
      v: ${{ steps.a.outputs.v }}
    steps:
      - uses: actions/checkout@v4
      - id: a
        run: echo "v=$(cat src/a)" >> "$GITHUB_OUTPUT"
  test:
    runs-on: ubuntu-latest
    needs: build
    steps:
      - run: echo "test ${{ needs.build.outputs.v }}" >> LOG
"#;
    let hints = "jobs:\n  build: {paths: ['src/**']}\n  test: {paths: ['docs/**']}\n";
    let repo = Repo::new(workflow, Some(hints));
    // A change only `build` reads. Running `build` alone stores its pass,
    // whose output the base's `test` never saw.
    fs::write(repo.root().join("src/a"), "two").unwrap();
    assert_ok(&repo.run(&["--reuse-results", "--job", "build"]));
    repo.take_ran();
    let output = repo.run(&[
        "--event",
        "push",
        "--affected",
        "--reuse-results",
        "--diff-base",
        "HEAD",
    ]);
    assert_ok(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Reuse result"),
        "build should come from the store"
    );
    assert_eq!(repo.take_ran(), ["test two"]);
}

#[test]
fn the_gate_skips_the_jobs_that_need_a_job_left_to_ci() {
    let workflow = r#"
name: ci
on: push
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - run: echo check >> LOG
  deploy:
    runs-on: ubuntu-latest
    needs: check
    steps:
      - run: echo deploy >> LOG
  announce:
    runs-on: ubuntu-latest
    needs: deploy
    steps:
      - run: echo announce >> LOG
"#;
    let repo = Repo::new(workflow, Some("jobs:\n  deploy: {gate: false}\n"));
    let output = repo.wrkflw(&["gate", "--runtime", "emulation", "--diff-base", "HEAD"]);
    assert_ok(&output);
    assert_eq!(repo.take_ran(), ["check"]);
}

/// `runner.os` in emulation: the host's OS, as GitHub spells it.
fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        other => panic!("no runner.os for {other}"),
    }
}

fn json_lines(output: &Output) -> Vec<serde_json::Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect()
}

const WINDOWS_JOB: &str = "name: ci\non: push\njobs:\n  win:\n    runs-on: windows-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: echo win >> LOG; test -f src/ok\n";

#[test]
fn a_run_marks_a_job_that_ran_as_another_os() {
    let repo = Repo::new(WINDOWS_JOB, None);
    fs::write(repo.root().join("src/ok"), "").unwrap();
    let args = [
        "--format",
        "json",
        "run",
        ".github/workflows/ci.yml",
        "--runtime",
        "emulation",
    ];
    let output = repo.wrkflw(&args);
    assert_ok(&output);
    let events = json_lines(&output);
    let finished = events
        .iter()
        .find(|e| e["type"] == "job_finished" && e["job"] == "win")
        .expect("no job_finished for win");
    assert_eq!(finished["runner"]["requested"], "windows-latest");
    assert_eq!(finished["runner"]["ran_as"], host_os());
    let run = events.last().unwrap();
    assert_eq!(run["type"], "run_finished");
    assert_eq!(run["status"], "success");
    assert_eq!(run["unverified"][0]["job"], "win");
    assert_eq!(run["jobs"][0]["runner"]["requested"], "windows-latest");
}

#[test]
fn the_gate_prints_one_json_object_naming_the_failed_run() {
    let repo = Repo::new(WINDOWS_JOB, None);
    let gate = [
        "--format",
        "json",
        "gate",
        "--runtime",
        "emulation",
        "--diff-base",
        "HEAD",
    ];

    let output = repo.wrkflw(&gate);
    assert_eq!(output.status.code(), Some(1));
    let lines = json_lines(&output);
    assert_eq!(lines.len(), 1, "gate printed more than one object");
    let report = &lines[0];
    assert_eq!(report["type"], "gate");
    assert_eq!(report["passed"], false);
    assert_eq!(report["failed"].as_array().unwrap().len(), 1);
    let workflow = &report["workflows"][0];
    assert_eq!(workflow["status"], "failure");
    assert!(workflow["run_id"].as_str().is_some_and(|id| !id.is_empty()));
    assert!(!workflow["failure"].is_null(), "no failure digest");

    // Fixed, then gated again with the pass reused: both list the job, since
    // a reused pass ran as another OS too.
    fs::write(repo.root().join("src/ok"), "").unwrap();
    for _ in 0..2 {
        let output = repo.wrkflw(&gate);
        assert_ok(&output);
        let report = &json_lines(&output)[0];
        assert_eq!(report["passed"], true);
        assert_eq!(report["ran"], 1);
        assert_eq!(report["unverified"][0]["job"], "win");
        assert_eq!(report["unverified"][0]["ran_as"], host_os());
        assert_eq!(
            report["unverified"][0]["workflow"],
            report["workflows"][0]["path"]
        );
    }
    assert_eq!(
        repo.take_ran(),
        ["win", "win"],
        "the third gate didn't reuse"
    );
}

const OS_MATRIX: &str = "name: ci\non: push\njobs:\n  build:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, windows-latest]\n    steps:\n      - run: echo ${{ matrix.os }} >> LOG\n";

#[test]
fn a_reused_os_matrix_leg_is_still_unverified() {
    let repo = Repo::new(OS_MATRIX, None);
    let gate = [
        "--format",
        "json",
        "gate",
        "--runtime",
        "emulation",
        "--diff-base",
        "HEAD",
    ];
    // Run, then reused: the Windows leg is listed both times, by the name a
    // run gives it, since a reused pass ran as another OS too.
    for ran in [vec!["ubuntu-latest", "windows-latest"], vec![]] {
        let output = repo.wrkflw(&gate);
        assert_ok(&output);
        let report = &json_lines(&output)[0];
        let unverified: Vec<&str> = report["unverified"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|u| u["job"].as_str())
            .collect();
        assert!(
            unverified.contains(&"build (os: windows-latest)"),
            "{report}"
        );
        assert_eq!(repo.take_ran(), ran);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("matches no leg"), "{stderr}");
    }
}

#[test]
fn the_gate_reports_an_installed_hook_as_json() {
    let repo = Repo::new(TWO_JOBS, None);
    let output = repo.wrkflw(&["--format", "json", "gate", "--install-hook"]);
    assert_ok(&output);
    let lines = json_lines(&output);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["type"], "gate_hook");
    let path = repo.root().join(lines[0]["path"].as_str().unwrap());
    assert!(fs::read_to_string(path).unwrap().contains("wrkflw gate"));
}
