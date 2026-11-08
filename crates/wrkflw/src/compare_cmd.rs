//! `wrkflw compare <run>`: run a workflow locally as GitHub ran it, and say
//! where the two disagree.
//!
//! A local pass is worth what its agreement with GitHub is worth. This
//! takes a finished GitHub Actions run, checks that the working tree is the
//! commit it tested, runs the same workflow for the same event in a child
//! `wrkflw run`, and matches the jobs up by the names GitHub gives them.
//! Each disagreement is listed, with what wrkflw knows about why: a job run
//! as another OS than `runs-on` asked for, or one killed for lack of memory.

use crate::{OutputFormat, RuntimeChoice};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};
use wrkflw_github::{RemoteJob, RemoteRun};
use wrkflw_runs::SCHEMA_VERSION;
use wrkflw_ui::cli_style;

pub(crate) struct CompareCtx {
    pub(crate) run: String,
    pub(crate) runtime: RuntimeChoice,
    pub(crate) format: OutputFormat,
}

/// A job as wrkflw names it, and how GitHub names the same job.
#[derive(Debug, Clone, PartialEq)]
struct ExpectedJob {
    /// `build (os: ubuntu-latest)`, as wrkflw reports it.
    local: String,
    /// The name GitHub shows, when it can be worked out exactly.
    github: Option<String>,
    /// For a matrix leg whose name GitHub builds by appending its values:
    /// the name before them, and the values.
    github_base: String,
    matrix_values: Option<BTreeSet<String>>,
}

impl ExpectedJob {
    fn matches(&self, name: &str) -> bool {
        if self.github.as_deref() == Some(name) {
            return true;
        }
        let Some(values) = &self.matrix_values else {
            return false;
        };
        name.strip_prefix(&self.github_base)
            .and_then(|rest| rest.strip_prefix(" ("))
            .and_then(|rest| rest.strip_suffix(')'))
            .is_some_and(|inner| {
                inner
                    .split(", ")
                    .map(str::to_string)
                    .collect::<BTreeSet<_>>()
                    == *values
            })
    }
}

/// A matrix value as GitHub prints it in a job name.
fn value_text(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Whether a job's `name:` has an expression that reads `matrix`. GitHub
/// appends a leg's values to any other name, including one that only says
/// the word, such as `Build the matrix`.
fn reads_matrix(name: &str) -> bool {
    wrkflw_executor::expression::text_reads_context(name, "matrix")
}

/// Every job of the workflow at `path`, with the name GitHub would give it.
///
/// GitHub shows a job's `name:`, or its id, with expressions filled in for
/// the leg. For a matrix leg whose `name:` doesn't use the matrix, it
/// appends the leg's values in brackets.
fn expected_jobs(path: &Path) -> Result<Vec<ExpectedJob>, String> {
    let workflow = wrkflw_parser::workflow::parse_workflow(path)?;
    let raw: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
    )
    .map_err(|e| format!("cannot parse {}: {e}", path.display()))?;
    let mut ids: Vec<&String> = workflow.jobs.keys().collect();
    ids.sort();
    let mut expected = Vec::new();
    for id in ids {
        let job = &workflow.jobs[id];
        let template = raw["jobs"][id.as_str()]["name"]
            .as_str()
            .map(str::to_string);
        let legs = job
            .matrix_config()
            .and_then(|config| wrkflw_matrix::expand_matrix(config).ok());
        let Some(legs) = legs else {
            let name = template
                .as_deref()
                .and_then(|t| wrkflw_executor::expression::evaluate_matrix_text(t, None));
            expected.push(ExpectedJob {
                local: id.clone(),
                github_base: name.clone().unwrap_or_else(|| id.clone()),
                github: Some(name.unwrap_or_else(|| id.clone())),
                matrix_values: None,
            });
            continue;
        };
        for leg in legs {
            let local = wrkflw_matrix::format_combination_name(id, &leg);
            let uses_matrix = template.as_deref().is_some_and(reads_matrix);
            let named = template
                .as_deref()
                .and_then(|t| {
                    wrkflw_executor::expression::evaluate_matrix_text(t, Some(&leg.values))
                })
                .unwrap_or_else(|| id.clone());
            let values: Option<BTreeSet<String>> =
                leg.values.values().map(value_text).collect::<Option<_>>();
            expected.push(if uses_matrix {
                ExpectedJob {
                    local,
                    github: Some(named.clone()),
                    github_base: named,
                    matrix_values: None,
                }
            } else {
                ExpectedJob {
                    local,
                    github: None,
                    github_base: named,
                    matrix_values: values,
                }
            });
        }
    }
    Ok(expected)
}

/// One row of the comparison.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    github_name: Option<String>,
    local_name: Option<String>,
    github: Option<String>,
    local: Option<String>,
    /// `Some(true)` agree, `Some(false)` disagree, `None` when one side
    /// has no result to compare, such as a job GitHub cancelled.
    agree: Option<bool>,
    note: Option<String>,
}

/// Whether GitHub's `conclusion` and wrkflw's `status` say the same.
fn agree(conclusion: Option<&str>, status: Option<&str>) -> Option<bool> {
    let conclusion = conclusion?;
    let status = status?;
    match conclusion {
        "success" | "failure" | "skipped" => Some(conclusion == status),
        // Cancelled by fail-fast or by hand, timed out, or waiting for
        // approval: nothing to compare against.
        _ => None,
    }
}

/// Why a local job may differ from GitHub's, from the run's own report.
fn note_for(local: &Value, failures: &[Value]) -> Option<String> {
    let name = local["name"].as_str().unwrap_or_default();
    let mut notes = Vec::new();
    if let Some(runner) = local.get("runner") {
        notes.push(format!(
            "ran as {} locally, GitHub ran {}",
            runner["ran_as"].as_str().unwrap_or("?"),
            runner["requested"].as_str().unwrap_or("?")
        ));
    }
    if let Some(original) = local["repeated_from"].as_str() {
        notes.push(format!(
            "not run locally: repeats {original} (pass --every-matrix-leg to run it)"
        ));
    }
    if let Some(reason) = local["reused"].as_str() {
        notes.push(format!(
            "not run locally: an earlier result was reused ({reason})"
        ));
    }
    let cause = failures.iter().find_map(|failure| {
        let here = failure["job"] == name
            || failure["also_failed_in"]
                .as_array()
                .is_some_and(|legs| legs.iter().any(|leg| leg == name));
        here.then(|| failure["cause"]["kind"].as_str()).flatten()
    });
    match cause {
        Some("out_of_memory") => notes.push("ran out of memory locally".to_string()),
        Some("killed") => notes.push(
            "a process was killed (SIGKILL) locally, most likely for lack of memory".to_string(),
        ),
        _ => {}
    }
    (!notes.is_empty()).then(|| notes.join("; "))
}

/// Match `remote` jobs to `local` results through `expected` names.
fn rows(
    expected: &[ExpectedJob],
    remote: &[RemoteJob],
    local: &[Value],
    failures: &[Value],
) -> Vec<Row> {
    let mut used = vec![false; remote.len()];
    let mut rows = Vec::new();
    for job in expected {
        let local_job = local.iter().find(|l| l["name"] == job.local.as_str());
        let found = remote
            .iter()
            .enumerate()
            .find(|(i, r)| !used[*i] && job.matches(&r.name));
        if let Some((i, _)) = found {
            used[i] = true;
        }
        let github = found.map(|(_, r)| r);
        let local_status = local_job.and_then(|l| l["status"].as_str());
        let github_conclusion = github.and_then(|r| r.conclusion.as_deref());
        if github.is_none() && local_job.is_none() {
            continue;
        }
        rows.push(Row {
            github_name: github.map(|r| r.name.clone()),
            local_name: Some(job.local.clone()),
            github: github_conclusion.map(str::to_string),
            local: local_status.map(str::to_string),
            agree: agree(github_conclusion, local_status),
            note: local_job.and_then(|l| note_for(l, failures)),
        });
    }
    for (i, remote_job) in remote.iter().enumerate() {
        if !used[i] {
            rows.push(Row {
                github_name: Some(remote_job.name.clone()),
                local_name: None,
                github: remote_job.conclusion.clone(),
                local: None,
                agree: None,
                note: Some(
                    "no local job has this name: a job of a called workflow, or a `name:` \
                     wrkflw couldn't work out, such as one that reads `github`"
                        .to_string(),
                ),
            });
        }
    }
    rows
}

fn git_line(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !line.is_empty()).then_some(line)
}

/// The child `wrkflw run` that checks the workflow as GitHub ran it.
fn run_args(remote: &RemoteRun, runtime: &str) -> Vec<String> {
    let mut args = vec![
        "--format".to_string(),
        "json".to_string(),
        "run".to_string(),
        remote.path.clone(),
        "--event".to_string(),
        remote.event.clone(),
        "--diff".to_string(),
        "--runtime".to_string(),
        runtime.to_string(),
        "--intent".to_string(),
        format!("wrkflw compare with GitHub run {}", remote.id),
    ];
    if let Some(base) = &remote.base_branch {
        args.extend(["--base-branch".to_string(), base.clone()]);
        args.extend(["--diff-base".to_string(), format!("origin/{base}")]);
    }
    args
}

/// What the comparison rests on, and what stops it.
#[derive(Debug, PartialEq)]
struct Outcome {
    /// How many results differ: jobs, or the whole run when no job pairs.
    disagreements: usize,
    /// `jobs` when jobs were paired and compared, `run` when only the run's
    /// result could be, `none` when nothing could.
    basis: &'static str,
    /// Why the comparison says less than a per-job one would.
    problem: Option<String>,
}

/// Judge `rows` and the local run `local` against GitHub's `remote` run.
///
/// Agreement needs something compared. A local run that didn't finish
/// disagrees whatever its jobs say, and a run with no job paired by name
/// falls back to the run's own result, which counts only when a local job
/// ran.
fn outcome(remote: &RemoteRun, local: &Value, rows: &[Row]) -> Outcome {
    let compared = rows.iter().filter(|r| r.agree.is_some()).count();
    let job_disagreements = rows.iter().filter(|r| r.agree == Some(false)).count();
    let status = local["status"].as_str().unwrap_or("unknown");
    let disagree = |problem: String| Outcome {
        disagreements: job_disagreements + 1,
        basis: if compared > 0 { "jobs" } else { "run" },
        problem: Some(problem),
    };
    match status {
        "skipped" => {
            return disagree(format!(
                "GitHub ran this workflow, but wrkflw's trigger filter skipped it: {}",
                local["skip_reason"].as_str().unwrap_or("no reason given")
            ))
        }
        "success" | "failure" => {}
        _ => {
            return disagree(format!(
                "the local run ended with {status}, so its jobs say nothing: {}",
                local["error"].as_str().unwrap_or("no error given")
            ))
        }
    }
    if compared > 0 {
        return Outcome {
            disagreements: job_disagreements,
            basis: "jobs",
            problem: None,
        };
    }
    if local["verdict"] == "nothing_checked" {
        return disagree("no job ran locally, so there is nothing to compare".to_string());
    }
    let unpaired = "no job could be paired with GitHub's by name";
    match agree(remote.conclusion.as_deref(), Some(status)) {
        Some(true) => Outcome {
            disagreements: 0,
            basis: "run",
            problem: Some(format!("{unpaired}, so only the run's result was compared")),
        },
        Some(false) => disagree(format!(
            "{unpaired}, and the run's result differs: GitHub {}, local {status}",
            remote.conclusion.as_deref().unwrap_or("unknown")
        )),
        None => Outcome {
            disagreements: 0,
            basis: "none",
            problem: Some(format!(
                "{unpaired}, and GitHub's run {} has no result to compare",
                remote.conclusion.as_deref().unwrap_or("unknown")
            )),
        },
    }
}

/// The `compare` object. `agree` is true only when something was compared
/// and nothing differed.
fn report(remote: &RemoteRun, local: &Value, rows: &[Row]) -> Value {
    let outcome = outcome(remote, local, rows);
    json!({
        "schema_version": SCHEMA_VERSION,
        "type": "compare",
        "agree": outcome.disagreements == 0 && outcome.basis != "none",
        "disagreements": outcome.disagreements,
        "basis": outcome.basis,
        "problem": outcome.problem,
        "github": {
            "run_id": remote.id,
            "url": remote.html_url,
            "event": remote.event,
            "workflow": remote.path,
            "head_sha": remote.head_sha,
            "conclusion": remote.conclusion,
        },
        "local": {
            "run_id": local["run_id"],
            "status": local["status"],
            "verdict": local["verdict"],
            "skip_reason": local["skip_reason"],
            "error": local["error"],
        },
        "jobs": rows.iter().map(|r| json!({
            "github_name": r.github_name,
            "local_name": r.local_name,
            "github": r.github,
            "local": r.local,
            "agree": r.agree,
            "note": r.note,
        })).collect::<Vec<_>>(),
    })
}

fn print_text(report: &Value) {
    let github = &report["github"];
    println!(
        "{}",
        cli_style::section(&format!(
            "GitHub run {} ({}, {}): {}",
            github["run_id"],
            github["workflow"].as_str().unwrap_or_default(),
            github["event"].as_str().unwrap_or_default(),
            github["conclusion"].as_str().unwrap_or("unknown")
        ))
    );
    if let Some(problem) = report["problem"].as_str() {
        let line = if report["agree"] == true {
            cli_style::warning(problem)
        } else {
            cli_style::error(problem)
        };
        println!("{line}");
    }
    for job in report["jobs"].as_array().into_iter().flatten() {
        let name = job["github_name"]
            .as_str()
            .or(job["local_name"].as_str())
            .unwrap_or_default();
        let line = format!(
            "{name}: GitHub {}, local {}",
            job["github"].as_str().unwrap_or("-"),
            job["local"].as_str().unwrap_or("-")
        );
        let line = match job["agree"].as_bool() {
            Some(true) => cli_style::success(&line),
            Some(false) => cli_style::error(&line),
            None => cli_style::dim(&line),
        };
        println!("  {line}");
        if let Some(note) = job["note"].as_str() {
            println!("    {}", cli_style::dim(note));
        }
    }
    let disagreements = report["disagreements"].as_u64().unwrap_or(0);
    if report["agree"] == true {
        println!("{}", cli_style::success("wrkflw agrees with GitHub"));
    } else if disagreements > 0 {
        println!(
            "{}",
            cli_style::error(&format!(
                "wrkflw disagrees with GitHub: {disagreements} result(s) differ"
            ))
        );
    } else {
        println!(
            "{}",
            cli_style::error("nothing could be compared, so wrkflw can't say it agrees")
        );
    }
}

/// Run the comparison. Exits 0 when wrkflw agrees with GitHub, 1 otherwise.
pub(crate) async fn run(ctx: CompareCtx) -> ! {
    let reference = wrkflw_github::parse_run_reference(&ctx.run).unwrap_or_else(|| {
        ctx.format.fail(&format!(
            "'{}' is not a run id or a https://github.com/<owner>/<repo>/actions/runs/<id> URL",
            ctx.run
        ))
    });
    let remote = wrkflw_github::fetch_run(&reference)
        .await
        .unwrap_or_else(|e| ctx.format.fail(&format!("cannot fetch the run: {e}")));
    if remote.conclusion.is_none() {
        ctx.format
            .fail("the run hasn't finished on GitHub yet, so there's nothing to compare with");
    }
    let head = git_line(&["rev-parse", "HEAD"]).unwrap_or_default();
    if head != remote.head_sha {
        let short = &remote.head_sha[..remote.head_sha.len().min(12)];
        ctx.format.fail(&format!(
            "this checkout is at {}, but the run tested {}. Check that commit out first, \
             for example: git worktree add ../compare-{short} {short}, then run this there",
            if head.is_empty() { "no commit" } else { &head },
            remote.head_sha,
        ));
    }
    if git_line(&["status", "--porcelain"]).is_some() {
        eprintln!(
            "{}",
            cli_style::dim(
                "Warning: the working tree has uncommitted changes, which GitHub's run didn't have"
            )
        );
    }
    if remote.event.starts_with("pull_request") {
        eprintln!(
            "{}",
            cli_style::dim(
                "Note: for a pull request GitHub tested this commit merged into its base branch. \
                 Here it runs as committed, so a change on the base branch can make them differ"
            )
        );
    }
    let path = Path::new(&remote.path);
    let expected = expected_jobs(path).unwrap_or_else(|e| ctx.format.fail(&e));
    let program = std::env::current_exe().unwrap_or_else(|e| {
        ctx.format
            .fail(&format!("cannot find this executable: {e}"))
    });
    let mut child = Command::new(program);
    child
        .args(run_args(&remote, ctx.runtime.as_flag()))
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    crate::cli_secrets::pass_to(&mut child);
    let output = child
        .output()
        .unwrap_or_else(|e| ctx.format.fail(&format!("cannot run wrkflw: {e}")));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let local = lines
        .iter()
        .rev()
        .find(|line| line["type"] == "run_finished")
        .cloned()
        .unwrap_or_else(|| {
            let message = lines
                .iter()
                .rev()
                .find(|line| line["type"] == "error")
                .and_then(|line| line["message"].as_str())
                .unwrap_or("the local run ended without a result");
            ctx.format
                .fail(&format!("the local run failed to start: {message}"))
        });
    let local_jobs = local["jobs"].as_array().cloned().unwrap_or_default();
    let failures = local["failure"]["failures"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let rows = rows(&expected, &remote.jobs, &local_jobs, &failures);
    let report = report(&remote, &local, &rows);
    let agrees = report["agree"] == true;
    if ctx.format.is_json() {
        wrkflw_runs::emit_line(&report);
    } else {
        print_text(&report);
    }
    std::process::exit(i32::from(!agrees));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, yaml: &str) -> std::path::PathBuf {
        let path = dir.join(".github/workflows/ci.yml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, yaml).unwrap();
        path
    }

    fn remote(name: &str, conclusion: &str) -> RemoteJob {
        RemoteJob {
            name: name.to_string(),
            conclusion: Some(conclusion.to_string()),
        }
    }

    const CI: &str = "name: CI\non: push\njobs:\n  fmt:\n    name: Format\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo fmt\n  build:\n    name: Build (${{ matrix.os }})\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, windows-latest]\n    steps:\n      - run: cargo test\n  tests:\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        include:\n          - {python: '3.13'}\n          - {python: '3.12', name: Old}\n    steps:\n      - run: tox\n  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: ruff\n";

    #[test]
    fn jobs_are_named_as_github_names_them() {
        let dir = tempfile::tempdir().unwrap();
        let expected = expected_jobs(&write(dir.path(), CI)).unwrap();
        let find = |local: &str| expected.iter().find(|e| e.local == local).unwrap();
        assert!(find("fmt").matches("Format"));
        assert!(find("lint").matches("lint"));
        assert!(find("build (os: windows-latest)").matches("Build (windows-latest)"));
        assert!(!find("build (os: windows-latest)").matches("Build (ubuntu-latest)"));
        assert!(find("tests (python: 3.13)").matches("tests (3.13)"));
        assert!(find("tests (name: Old, python: 3.12)").matches("tests (3.12, Old)"));
    }

    #[test]
    fn rows_pair_jobs_and_explain_disagreements() {
        let dir = tempfile::tempdir().unwrap();
        let expected = expected_jobs(&write(dir.path(), CI)).unwrap();
        let github = vec![
            remote("Format", "success"),
            remote("Build (ubuntu-latest)", "success"),
            remote("Build (windows-latest)", "failure"),
            remote("lint", "cancelled"),
            remote("deploy / publish", "success"),
        ];
        let local = vec![
            json!({"name": "fmt", "status": "success"}),
            json!({"name": "build (os: ubuntu-latest)", "status": "success"}),
            json!({"name": "build (os: windows-latest)", "status": "success",
                   "runner": {"requested": "windows-latest", "ran_as": "Linux"},
                   "repeated_from": "build (os: ubuntu-latest)"}),
            json!({"name": "lint", "status": "failure"}),
        ];
        let rows = rows(&expected, &github, &local, &[]);
        let row = |name: &str| {
            rows.iter()
                .find(|r| r.github_name.as_deref() == Some(name))
                .unwrap()
        };
        assert_eq!(row("Format").agree, Some(true));
        let windows = row("Build (windows-latest)");
        assert_eq!(windows.agree, Some(false));
        let note = windows.note.as_deref().unwrap();
        assert!(note.contains("ran as Linux locally"), "{note}");
        assert!(note.contains("repeats build (os: ubuntu-latest)"), "{note}");
        assert_eq!(row("lint").agree, None);
        let called = row("deploy / publish");
        assert_eq!(called.local_name, None);
        assert_eq!(called.agree, None);

        let report = report(
            &RemoteRun {
                id: 1,
                owner: "o".into(),
                repo: "r".into(),
                html_url: String::new(),
                head_sha: "abc".into(),
                head_branch: Some("main".into()),
                event: "push".into(),
                path: ".github/workflows/ci.yml".into(),
                conclusion: Some("failure".into()),
                base_branch: None,
                jobs: github,
            },
            &json!({"status": "success", "verdict": "partial"}),
            &rows,
        );
        assert_eq!(report["agree"], false);
        assert_eq!(report["disagreements"], 1);
    }

    fn github_run(conclusion: &str, jobs: Vec<RemoteJob>) -> RemoteRun {
        RemoteRun {
            id: 1,
            owner: "o".into(),
            repo: "r".into(),
            html_url: String::new(),
            head_sha: "abc".into(),
            head_branch: Some("main".into()),
            event: "push".into(),
            path: ".github/workflows/ci.yml".into(),
            conclusion: Some(conclusion.into()),
            base_branch: None,
            jobs,
        }
    }

    #[test]
    fn a_local_run_that_errored_never_agrees() {
        let github = github_run("failure", vec![remote("Format", "failure")]);
        let local =
            json!({"status": "error", "verdict": "error", "error": "no docker", "jobs": []});
        let unpaired = rows(&[], &github.jobs, &[], &[]);
        let report = report(&github, &local, &unpaired);
        assert_eq!(report["agree"], false, "{report}");
        assert_eq!(report["disagreements"], 1);
        assert!(report["problem"].as_str().unwrap().contains("no docker"));
    }

    #[test]
    fn with_no_job_paired_only_the_runs_result_is_compared() {
        let local_pass = json!({"status": "success", "verdict": "verified"});
        let github_jobs = vec![remote("Build on ubuntu", "failure")];
        let none_paired = rows(&[], &github_jobs, &[], &[]);
        assert!(none_paired.iter().all(|r| r.agree.is_none()));

        let failed = report(
            &github_run("failure", github_jobs.clone()),
            &local_pass,
            &none_paired,
        );
        assert_eq!(failed["agree"], false, "{failed}");
        assert_eq!(failed["basis"], "run");
        assert_eq!(failed["disagreements"], 1);

        let passed = report(
            &github_run("success", github_jobs.clone()),
            &local_pass,
            &none_paired,
        );
        assert_eq!(passed["agree"], true, "{passed}");
        assert_eq!(passed["basis"], "run");
        assert!(passed["problem"]
            .as_str()
            .unwrap()
            .contains("only the run's result"));

        let nothing_ran = json!({"status": "success", "verdict": "nothing_checked"});
        let idle = report(
            &github_run("success", github_jobs.clone()),
            &nothing_ran,
            &none_paired,
        );
        assert_eq!(idle["agree"], false, "{idle}");

        let cancelled = report(
            &github_run("cancelled", github_jobs),
            &local_pass,
            &none_paired,
        );
        assert_eq!(cancelled["agree"], false, "{cancelled}");
        assert_eq!(cancelled["basis"], "none");
        assert_eq!(cancelled["disagreements"], 0);
    }

    #[test]
    fn only_a_matrix_expression_keeps_github_from_appending_values() {
        assert!(reads_matrix("Build (${{ matrix.os }})"));
        assert!(reads_matrix("${{ matrix['os'] }}"));
        assert!(!reads_matrix("Build the matrix"));
        assert!(!reads_matrix("${{ github.workflow }} matrix"));
        assert!(!reads_matrix("${{ inputs.matrix_name }}"));

        let dir = tempfile::tempdir().unwrap();
        let yaml = "name: CI\non: push\njobs:\n  build:\n    name: Build the matrix\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        os: [a, b]\n    steps:\n      - run: make\n";
        let expected = expected_jobs(&write(dir.path(), yaml)).unwrap();
        let leg = expected
            .iter()
            .find(|e| e.local == "build (os: b)")
            .unwrap();
        assert!(leg.matches("Build the matrix (b)"));
    }

    #[test]
    fn a_memory_kill_is_named_on_the_job() {
        let failures = vec![json!({
            "job": "build (os: ubuntu-latest)",
            "also_failed_in": ["build (os: macos-latest)"],
            "cause": {"kind": "out_of_memory"},
        })];
        let note = note_for(
            &json!({"name": "build (os: macos-latest)", "status": "failure"}),
            &failures,
        )
        .unwrap();
        assert!(note.contains("ran out of memory"), "{note}");

        let killed = vec![json!({"job": "build", "cause": {"kind": "killed"}})];
        let note = note_for(&json!({"name": "build", "status": "failure"}), &killed).unwrap();
        assert!(note.contains("SIGKILL"), "{note}");
    }

    #[test]
    fn a_pull_request_run_is_checked_against_its_base() {
        let run = RemoteRun {
            id: 9,
            owner: "o".into(),
            repo: "r".into(),
            html_url: String::new(),
            head_sha: "abc".into(),
            head_branch: Some("feat".into()),
            event: "pull_request".into(),
            path: ".github/workflows/ci.yml".into(),
            conclusion: Some("success".into()),
            base_branch: Some("main".into()),
            jobs: vec![],
        };
        let args = run_args(&run, "docker");
        assert!(args.windows(2).any(|w| w == ["--event", "pull_request"]));
        assert!(args.windows(2).any(|w| w == ["--base-branch", "main"]));
        assert!(args.windows(2).any(|w| w == ["--diff-base", "origin/main"]));
    }
}
