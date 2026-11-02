//! `wrkflw runs`: read the run history.

use crate::OutputFormat;
use clap::Subcommand;
use serde_json::{json, Value};
use wrkflw_runs::{
    failure_digest, render_digest, RunRecord, RunStatus, RunStore, DEFAULT_TAIL_LINES,
    SCHEMA_VERSION,
};

#[derive(Debug, Clone, Subcommand)]
pub(crate) enum RunsCommand {
    /// List recorded runs of this repository, newest first
    List {
        /// Maximum number of runs
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Runs of every repository
        #[arg(long)]
        all: bool,
        /// Only runs with this status (success, failure, error, skipped, running, interrupted)
        #[arg(long)]
        status: Option<String>,
        /// Only runs by this agent
        #[arg(long)]
        agent: Option<String>,
        /// Only runs of workflows whose path contains this text
        #[arg(long)]
        workflow: Option<String>,
    },
    /// Show one run: jobs, steps, exit codes and timings
    Show {
        /// Run id, unique id prefix, or `latest`
        #[arg(default_value = "latest")]
        run: String,
    },
    /// Print step logs
    Logs {
        /// Run id, unique id prefix, or `latest`
        #[arg(default_value = "latest")]
        run: String,
        /// Only this job
        #[arg(long)]
        job: Option<String>,
        /// Only this step, by name or 1-based position
        #[arg(long)]
        step: Option<String>,
        /// Only the last N lines of each log
        #[arg(long)]
        tail: Option<usize>,
    },
    /// Explain why a run failed: failed step, script, output tail
    Failure {
        /// Run id, unique id prefix, or `latest`
        #[arg(default_value = "latest")]
        run: String,
        /// Lines of output per failed step
        #[arg(long, default_value_t = DEFAULT_TAIL_LINES)]
        tail: usize,
    },
    /// Run a recorded run again with the same arguments, in the same directory
    Replay {
        /// Run id, unique id prefix, or `latest`
        #[arg(default_value = "latest")]
        run: String,
        /// GitLab variable as KEY=value. Values are never recorded, so pass them again
        #[arg(short = 'V', long = "variable")]
        variables: Vec<String>,
    },
}

fn die(format: OutputFormat, message: &str) -> ! {
    if format.is_json() {
        wrkflw_runs::emit_line(&json!({
            "schema_version": SCHEMA_VERSION,
            "type": "error",
            "message": message,
        }));
    }
    eprintln!("Error: {message}");
    std::process::exit(1);
}

fn open_store(format: OutputFormat) -> RunStore {
    RunStore::open_default().unwrap_or_else(|| die(format, "no data directory: set WRKFLW_HOME"))
}

async fn current_repo() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    wrkflw_runs::repo_root_of(&cwd).await
}

async fn resolve(store: &RunStore, reference: &str, format: OutputFormat) -> RunRecord {
    let repo = current_repo().await;
    store
        .resolve(reference, &repo)
        .unwrap_or_else(|e| die(format, &e))
}

fn summary_json(run: &RunRecord) -> Value {
    json!({
        "id": run.id,
        "status": run.status.as_str(),
        "workflow": run.workflow,
        "runtime": run.runtime,
        "source": run.source,
        "agent": run.agent,
        "intent": run.intent,
        "git_sha": run.git_sha,
        "git_branch": run.git_branch,
        "started_at": run.started_at,
        "duration_ms": run.duration_ms,
        "repo_root": run.repo_root,
    })
}

fn tail_text(text: &str, lines: Option<usize>) -> String {
    let Some(lines) = lines else {
        return text.to_string();
    };
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn print_summary(run: &RunRecord) {
    let by = run
        .agent
        .as_deref()
        .map(|agent| format!(" by {agent}"))
        .unwrap_or_default();
    let seconds = run.duration_ms.unwrap_or(0) as f64 / 1000.0;
    println!(
        "{}  {:<11} {:>7.1}s  {}{}",
        run.id,
        run.status.as_str(),
        seconds,
        run.workflow,
        by
    );
    if let Some(intent) = &run.intent {
        println!("    {intent}");
    }
}

fn print_run(run: &RunRecord) {
    print_summary(run);
    if let Some(sha) = &run.git_sha {
        let dirty = if run.git_dirty == Some(true) {
            " (dirty)"
        } else {
            ""
        };
        println!(
            "    {} {}{}",
            run.git_branch.as_deref().unwrap_or("detached"),
            &sha[..sha.len().min(10)],
            dirty
        );
    }
    if let Some(reason) = &run.skip_reason {
        println!("    skipped: {reason}");
    }
    if let Some(error) = &run.error {
        println!("    error: {error}");
    }
    for job in &run.jobs {
        let allowed = if job.failure_allowed {
            " (allowed)"
        } else {
            ""
        };
        println!("  {} {}{}", job.status, job.name, allowed);
        for step in &job.steps {
            let code = step
                .exit_code
                .map(|c| format!(" exit {c}"))
                .unwrap_or_default();
            println!(
                "    {}. {} {}{}",
                step.index + 1,
                step.conclusion,
                step.name,
                code
            );
        }
    }
}

fn logs(
    store: &RunStore,
    run: &RunRecord,
    job: Option<&str>,
    step: Option<&str>,
    tail: Option<usize>,
) -> Vec<Value> {
    let mut out = Vec::new();
    for entry in run
        .jobs
        .iter()
        .filter(|j| job.is_none_or(|wanted| j.name == wanted || j.canonical_name == wanted))
    {
        for record in entry.steps.iter().filter(|s| {
            step.is_none_or(|wanted| {
                s.name == wanted || wanted.parse::<usize>().is_ok_and(|n| n == s.index + 1)
            })
        }) {
            let text = record
                .log
                .as_deref()
                .and_then(|path| store.read_log(&run.id, path).ok())
                .unwrap_or_default();
            out.push(json!({
                "job": entry.name,
                "step": record.name,
                "index": record.index,
                "conclusion": record.conclusion,
                "text": tail_text(&text, tail),
            }));
        }
    }
    out
}

/// The recorded arguments plus this replay's flags. Those go before any `--`,
/// which MCP runs always have, because clap reads what follows it as paths.
fn replay_args(argv: &[String], variables: &[String], format: OutputFormat) -> Vec<String> {
    let separator = argv.iter().position(|arg| arg == "--");
    let (flags, positional) = argv.split_at(separator.unwrap_or(argv.len()));
    let mut args = flags.to_vec();
    for variable in variables {
        args.extend(["--variable".to_string(), variable.clone()]);
    }
    if format.is_json() {
        args.extend(["--format".to_string(), "json".to_string()]);
    }
    args.extend_from_slice(positional);
    args
}

fn replay(run: &RunRecord, variables: &[String], format: OutputFormat) -> ! {
    if run.argv.is_empty() {
        die(
            format,
            &format!("run {} has no recorded command to replay", run.id),
        );
    }
    if variables.is_empty() && !run.variables.is_empty() {
        wrkflw_logging::warning(&format!(
            "the original run set variables that were not recorded: {}. Pass them with -V.",
            run.variables.join(", ")
        ));
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| die(format, &e.to_string()));
    let mut command = std::process::Command::new(exe);
    command
        .args(replay_args(&run.argv, variables, format))
        .current_dir(&run.cwd)
        .env(wrkflw_runs::env::REPLAY_OF, &run.id);
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => die(format, &format!("could not start replay: {e}")),
    }
}

pub(crate) async fn run(command: RunsCommand, format: OutputFormat) {
    let store = open_store(format);
    match command {
        RunsCommand::List {
            limit,
            all,
            status,
            agent,
            workflow,
        } => {
            let repo = current_repo().await;
            let runs: Vec<RunRecord> = store
                .list()
                .into_iter()
                .filter(|run| all || run.repo_root == repo)
                .filter(|run| status.as_deref().is_none_or(|s| run.status.as_str() == s))
                .filter(|run| {
                    agent
                        .as_deref()
                        .is_none_or(|a| run.agent.as_deref() == Some(a))
                })
                .filter(|run| workflow.as_deref().is_none_or(|w| run.workflow.contains(w)))
                .take(limit)
                .collect();
            if format.is_json() {
                wrkflw_runs::emit_line(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "runs",
                    "runs": runs.iter().map(summary_json).collect::<Vec<_>>(),
                }));
            } else if runs.is_empty() {
                println!("No recorded runs for {repo}. Use --all for every repository.");
            } else {
                runs.iter().for_each(print_summary);
            }
        }
        RunsCommand::Show { run } => {
            let record = resolve(&store, &run, format).await;
            if format.is_json() {
                let mut value = serde_json::to_value(&record).unwrap_or(Value::Null);
                value["type"] = json!("run");
                wrkflw_runs::emit_line(&value);
            } else {
                print_run(&record);
            }
        }
        RunsCommand::Logs {
            run,
            job,
            step,
            tail,
        } => {
            let record = resolve(&store, &run, format).await;
            let entries = logs(&store, &record, job.as_deref(), step.as_deref(), tail);
            if entries.is_empty() && (job.is_some() || step.is_some()) {
                die(
                    format,
                    &format!("no matching step logs in run {}", record.id),
                );
            }
            if format.is_json() {
                wrkflw_runs::emit_line(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "logs",
                    "run_id": record.id,
                    "logs": entries,
                }));
            } else {
                for entry in &entries {
                    println!(
                        "=== {} / {} ({}) ===",
                        entry["job"].as_str().unwrap_or(""),
                        entry["step"].as_str().unwrap_or(""),
                        entry["conclusion"].as_str().unwrap_or("")
                    );
                    println!("{}", entry["text"].as_str().unwrap_or(""));
                }
            }
        }
        RunsCommand::Failure { run, tail } => {
            let record = resolve(&store, &run, format).await;
            let digest = failure_digest(&store, &record, tail);
            match digest {
                Some(digest) if format.is_json() => {
                    let mut value = serde_json::to_value(&digest).unwrap_or(Value::Null);
                    value["type"] = json!("failure");
                    wrkflw_runs::emit_line(&value);
                }
                Some(digest) => print!("{}", render_digest(&digest)),
                None if format.is_json() => wrkflw_runs::emit_line(&json!({
                    "schema_version": SCHEMA_VERSION,
                    "type": "failure",
                    "run_id": record.id,
                    "status": record.status,
                    "failures": [],
                })),
                None => println!("Run {} did not fail: {}", record.id, record.status.as_str()),
            }
        }
        RunsCommand::Replay { run, variables } => {
            let record = resolve(&store, &run, format).await;
            if record.status == RunStatus::Running {
                die(format, &format!("run {} is still running", record.id));
            }
            replay(&record, &variables, format);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn replay_flags_stay_ahead_of_the_separator() {
        let args = replay_args(
            &strings(&["run", "--", "ci.yml"]),
            &strings(&["A=b"]),
            OutputFormat::Json,
        );
        assert_eq!(
            args,
            strings(&[
                "run",
                "--variable",
                "A=b",
                "--format",
                "json",
                "--",
                "ci.yml"
            ])
        );
    }

    #[test]
    fn replay_without_a_separator_appends_its_flags() {
        let args = replay_args(&strings(&["run", "ci.yml"]), &[], OutputFormat::Json);
        assert_eq!(args, strings(&["run", "ci.yml", "--format", "json"]));
        let args = replay_args(&strings(&["run", "ci.yml"]), &[], OutputFormat::Text);
        assert_eq!(args, strings(&["run", "ci.yml"]));
    }
}
