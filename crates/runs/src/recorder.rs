//! Runs a workflow while recording it and streaming it as NDJSON.

use crate::digest::{failure_digest, FailureDigest, DEFAULT_TAIL_LINES};
use crate::env;
use crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION};
use crate::store::{history_disabled, RunStore, DEFAULT_KEEP};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wrkflw_executor::engine::{ExecutionError, ExecutionResult};
use wrkflw_executor::{
    with_run_hooks, ExecutionConfig, ExecutionEvent, ExecutionObserver, FromStep, JobResult,
    RunHooks, RunnerSubstitution, UnexecutedAction,
};

/// Step output carried inline in an NDJSON event. The full text is in the log.
const MAX_EVENT_OUTPUT_BYTES: usize = 64 * 1024;

/// What the caller knows about a run. Everything else is detected.
#[derive(Clone, Debug, Default)]
pub struct RunMeta {
    pub source: String,
    pub agent: Option<String>,
    pub intent: Option<String>,
    pub session: Option<String>,
    pub event: Option<String>,
    pub argv: Vec<String>,
    pub variables: Vec<String>,
    pub replay_of: Option<String>,
}

impl RunMeta {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            agent: detect_agent(),
            session: detect_session(),
            replay_of: non_empty_env(env::REPLAY_OF),
            ..Default::default()
        }
    }
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// `WRKFLW_AGENT`, or the coding agent that announces itself in the environment.
pub fn detect_agent() -> Option<String> {
    agent_from(non_empty_env)
}

/// `WRKFLW_SESSION`, or the session id the detected agent exports.
pub fn detect_session() -> Option<String> {
    session_from(non_empty_env)
}

fn agent_from(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(agent) = get(env::AGENT) {
        return Some(agent);
    }
    // Claude Code sets `claude-code_2-1-314_agent`: name, version, kind.
    if let Some(agent) = get(env::AI_AGENT) {
        let name = agent.split('_').next().unwrap_or_default().trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let announced = |names: &[&str]| names.iter().any(|name| get(name).is_some());
    let agent = if announced(&[env::CLAUDE_CODE]) {
        "claude-code"
    } else if announced(&[env::CODEX_THREAD, env::CODEX_SANDBOX, env::CODEX_CI]) {
        "codex"
    } else if announced(&[env::GEMINI_CLI]) {
        "gemini"
    } else if announced(&[env::OPENCODE, env::OPENCODE_CLIENT]) {
        "opencode"
    } else if announced(&[env::CURSOR_AGENT]) {
        "cursor"
    } else if announced(&[env::AUGMENT_AGENT]) {
        "augment"
    } else {
        return None;
    };
    Some(agent.to_string())
}

fn session_from(get: impl Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(session) = get(env::SESSION) {
        return Some(session);
    }
    // An MCP server outlives the agent's session: Claude Code keeps the id it
    // was started with across `/clear`. A stale id is worse than none.
    if get(env::MCP).is_some() {
        return None;
    }
    get(env::CLAUDE_CODE_SESSION).or_else(|| get(env::CODEX_THREAD))
}

#[derive(Clone, Default)]
pub struct RunOptions {
    pub meta: RunMeta,
    /// Write NDJSON events to stdout.
    pub ndjson: bool,
    /// With `ndjson`, also write `step_output` lines while a step runs.
    pub stream_output: bool,
    /// Keep a history record. Also subject to `WRKFLW_NO_HISTORY`.
    pub record: bool,
    pub from_step: Option<FromStep>,
    /// Also receive the run's events, after the recorder. The TUI uses this
    /// to tell the run as it happens.
    pub observers: Vec<Arc<dyn ExecutionObserver>>,
}

impl std::fmt::Debug for RunOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunOptions")
            .field("meta", &self.meta)
            .field("ndjson", &self.ndjson)
            .field("stream_output", &self.stream_output)
            .field("record", &self.record)
            .field("from_step", &self.from_step)
            .field("observers", &self.observers.len())
            .finish()
    }
}

pub struct RecordedRun {
    pub result: Result<ExecutionResult, ExecutionError>,
    /// How much the result proves, as `run_finished.verdict` says it.
    pub verdict: &'static str,
    /// Jobs whose pass doesn't stand for GitHub's, as in `run_finished`.
    pub unverified: Vec<Value>,
    /// `None` when history was off or the store could not be written.
    pub record: Option<RunRecord>,
    pub digest: Option<FailureDigest>,
}

/// Git facts for a working directory. Every field is best effort.
#[derive(Default)]
struct GitInfo {
    root: Option<String>,
    sha: Option<String>,
    branch: Option<String>,
    dirty: Option<bool>,
}

async fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let command = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(5), command)
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn git_info(cwd: &Path) -> GitInfo {
    GitInfo {
        root: git(cwd, &["rev-parse", "--show-toplevel"]).await,
        sha: git(cwd, &["rev-parse", "HEAD"]).await,
        branch: git(cwd, &["branch", "--show-current"])
            .await
            .filter(|branch| !branch.is_empty()),
        dirty: git(cwd, &["status", "--porcelain"])
            .await
            .map(|status| !status.is_empty()),
    }
}

/// Repository a directory belongs to, for scoping listings to it.
pub async fn repo_root_of(cwd: &Path) -> String {
    git(cwd, &["rev-parse", "--show-toplevel"])
        .await
        .unwrap_or_else(|| cwd.to_string_lossy().into_owned())
}

fn new_run_id() -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!(
        "{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S%3f"),
        &random[..6]
    )
}

/// Turn a job or step name into a safe, readable path component.
fn slug(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .take(48)
        .collect();
    let trimmed = mapped.trim_matches('-');
    if trimmed.is_empty() {
        "step".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Directory of a job's logs. Matrix legs differ only in characters `slug`
/// flattens or cuts off, so a name it changed gets a hash of the original:
/// without one, two legs would write over each other's logs.
fn job_dir(name: &str) -> String {
    let readable = slug(name);
    if readable == name {
        return readable;
    }
    let hash = name.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    format!("{readable}-{hash:08x}")
}

/// Keep the end of `text`, on a character boundary: failures print last.
fn tail_bytes(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    (&text[start..], true)
}

struct Recorder {
    store: RunStore,
    /// False when logs go to a scratch directory, because history is off and
    /// the digest still needs them.
    persistent: bool,
    _scratch: Option<tempfile::TempDir>,
    state: Mutex<RunRecord>,
    ndjson: bool,
    stream_output: bool,
    started: Instant,
}

impl Recorder {
    fn persist(&self, record: &RunRecord) {
        if let Err(e) = self.store.save(record) {
            wrkflw_logging::warning(&format!("Could not save run history: {e}"));
        }
    }

    fn write_log(&self, id: &str, relative: &str, text: &str) -> Option<String> {
        match self.store.write_log(id, relative, text) {
            Ok(()) => Some(relative.to_string()),
            Err(e) => {
                wrkflw_logging::warning(&format!("Could not save run log: {e}"));
                None
            }
        }
    }

    fn line(&self, mut value: Value) {
        if !self.ndjson {
            return;
        }
        let id = self.state.lock().map(|r| r.id.clone()).unwrap_or_default();
        value["schema_version"] = json!(SCHEMA_VERSION);
        value["run_id"] = json!(id);
        emit_line(&value);
    }

    fn job_mut<'a>(record: &'a mut RunRecord, job: &str, canonical: &str) -> &'a mut JobRecord {
        let position = record.jobs.iter().position(|j| j.name == job);
        let position = position.unwrap_or_else(|| {
            record.jobs.push(JobRecord {
                name: job.to_string(),
                canonical_name: canonical.to_string(),
                status: "running".to_string(),
                failure_allowed: false,
                duration_ms: None,
                log: None,
                steps: Vec::new(),
                repeated_from: None,
                reused: None,
                runner: None,
                actions_not_run: Vec::new(),
            });
            record.jobs.len() - 1
        });
        &mut record.jobs[position]
    }

    /// Reconcile with the final result: skipped jobs and jobs that failed
    /// before their first step never produced events.
    fn reconcile(&self, record: &mut RunRecord, jobs: &[JobResult]) {
        for job in jobs {
            let id = record.id.clone();
            let existing = Self::job_mut(record, &job.name, &job.canonical_name);
            existing.status = job.status.to_string();
            existing.failure_allowed = job.failure_allowed;
            if !existing.steps.is_empty() {
                continue;
            }
            let job_dir = job_dir(&job.name);
            let mut steps = Vec::new();
            for (index, step) in job.steps.iter().enumerate() {
                let relative = format!("logs/{job_dir}/{:02}-{}.log", index, slug(&step.name));
                steps.push(StepRecord {
                    index,
                    name: step.name.clone(),
                    status: step.status.to_string(),
                    outcome: step.outcome.to_string(),
                    conclusion: step.conclusion.to_string(),
                    exit_code: step.exit_code,
                    duration_ms: None,
                    command: None,
                    uses: None,
                    env_set: Vec::new(),
                    log: self.write_log(&id, &relative, &step.output),
                });
            }
            let relative = format!("logs/{job_dir}/job.log");
            let log = (!job.logs.is_empty())
                .then(|| self.write_log(&id, &relative, &job.logs))
                .flatten();
            let existing = Self::job_mut(record, &job.name, &job.canonical_name);
            existing.steps = steps;
            existing.log = existing.log.take().or(log);
        }
    }

    fn finish(
        &self,
        status: RunStatus,
        error: Option<String>,
        skip_reason: Option<String>,
        jobs: &[JobResult],
    ) -> RunRecord {
        let mut record = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.reconcile(&mut record, jobs);
        record.status = status;
        record.error = error;
        record.skip_reason = skip_reason;
        record.finished_at = Some(chrono::Utc::now().to_rfc3339());
        record.duration_ms = Some(self.started.elapsed().as_millis() as u64);
        self.persist(&record);
        record.clone()
    }
}

impl ExecutionObserver for Recorder {
    fn wants_step_output(&self) -> bool {
        self.stream_output
    }

    fn on_event(&self, event: &ExecutionEvent) {
        // These two are not lines of the event stream in their own right.
        match event {
            ExecutionEvent::StepOutput {
                job,
                step,
                index,
                lines,
            } => {
                if self.stream_output {
                    self.line(json!({
                        "type": "step_output",
                        "job": job,
                        "step": step,
                        "index": index,
                        "lines": lines,
                    }));
                }
                return;
            }
            ExecutionEvent::RunnerSubstituted {
                job,
                requested,
                ran_as,
            } => {
                let mut record = self.state.lock().unwrap_or_else(|e| e.into_inner());
                Self::job_mut(&mut record, job, job).runner = Some(RunnerSubstitution {
                    requested: requested.clone(),
                    ran_as: ran_as.clone(),
                });
                self.persist(&record);
                return;
            }
            ExecutionEvent::ActionNotRun { job, step, uses } => {
                let mut record = self.state.lock().unwrap_or_else(|e| e.into_inner());
                Self::job_mut(&mut record, job, job)
                    .actions_not_run
                    .push(UnexecutedAction {
                        step: step.clone(),
                        uses: uses.clone(),
                    });
                self.persist(&record);
                return;
            }
            _ => {}
        }
        // Live output stays out of the record and the event stream: the
        // step's finish event carries all of it. The recorder doesn't ask
        // for it, but another observer may have.
        let Some(mut line) = event_json(event) else {
            return;
        };
        if let ExecutionEvent::JobFinished { job, .. } = event {
            let record = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(finished) = record.jobs.iter().find(|j| &j.name == job) {
                if let Some(runner) = &finished.runner {
                    line["runner"] = json!(runner);
                }
                if !finished.actions_not_run.is_empty() {
                    line["actions_not_run"] = json!(finished.actions_not_run);
                }
            }
        }
        self.line(line);
        let mut record = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = record.id.clone();
        match event {
            ExecutionEvent::JobStarted {
                job,
                canonical_name,
            } => {
                Self::job_mut(&mut record, job, canonical_name);
                self.persist(&record);
            }
            // Handled above.
            ExecutionEvent::StepOutput { .. }
            | ExecutionEvent::RunnerSubstituted { .. }
            | ExecutionEvent::ActionNotRun { .. } => {}
            ExecutionEvent::StepStarted { .. } => {}
            ExecutionEvent::StepFinished {
                job,
                step,
                index,
                status,
                outcome,
                conclusion,
                exit_code,
                duration_ms,
                output,
                command,
                uses,
                env_set,
            } => {
                let relative = format!("logs/{}/{:02}-{}.log", job_dir(job), index, slug(step));
                let log = self.write_log(&id, &relative, output);
                Self::job_mut(&mut record, job, job).steps.push(StepRecord {
                    index: *index,
                    name: step.clone(),
                    status: status.to_string(),
                    outcome: outcome.to_string(),
                    conclusion: conclusion.to_string(),
                    exit_code: *exit_code,
                    duration_ms: Some(*duration_ms),
                    command: command.clone(),
                    uses: uses.clone(),
                    env_set: env_set.clone(),
                    log,
                });
                self.persist(&record);
            }
            ExecutionEvent::JobFinished {
                job,
                status,
                failure_allowed,
                duration_ms,
                logs,
                repeated_from,
                reused,
            } => {
                let relative = format!("logs/{}/job.log", job_dir(job));
                let log = self.write_log(&id, &relative, logs);
                let entry = Self::job_mut(&mut record, job, job);
                entry.status = status.to_string();
                entry.failure_allowed = *failure_allowed;
                entry.duration_ms = Some(*duration_ms);
                entry.log = log;
                entry.repeated_from = repeated_from.clone();
                entry.reused = reused.clone();
                self.persist(&record);
            }
        }
    }
}

/// Write one JSON line to stdout. A closed pipe is the reader's choice.
pub fn emit_line(value: &Value) {
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = writeln!(lock, "{value}");
    let _ = lock.flush();
}

/// The NDJSON line for `event`, or `None` for events the stream leaves out.
fn event_json(event: &ExecutionEvent) -> Option<Value> {
    let line = match event {
        ExecutionEvent::JobStarted {
            job,
            canonical_name,
        } => json!({"type": "job_started", "job": job, "canonical_name": canonical_name}),
        ExecutionEvent::StepStarted { job, step, index } => {
            json!({"type": "step_started", "job": job, "step": step, "index": index})
        }
        ExecutionEvent::StepOutput { .. }
        | ExecutionEvent::RunnerSubstituted { .. }
        | ExecutionEvent::ActionNotRun { .. } => return None,
        ExecutionEvent::StepFinished {
            job,
            step,
            index,
            status,
            outcome,
            conclusion,
            exit_code,
            duration_ms,
            output,
            ..
        } => {
            let (output, truncated) = tail_bytes(output, MAX_EVENT_OUTPUT_BYTES);
            json!({
                "type": "step_finished",
                "job": job,
                "step": step,
                "index": index,
                "status": status.to_string(),
                "outcome": outcome.to_string(),
                "conclusion": conclusion.to_string(),
                "exit_code": exit_code,
                "duration_ms": duration_ms,
                "output": output,
                "output_truncated": truncated,
            })
        }
        ExecutionEvent::JobFinished {
            job,
            status,
            failure_allowed,
            duration_ms,
            repeated_from,
            reused,
            ..
        } => {
            let mut line = json!({
                "type": "job_finished",
                "job": job,
                "status": status.to_string(),
                "failure_allowed": failure_allowed,
                "duration_ms": duration_ms,
            });
            // Only on legs that didn't run, so other lines keep their shape.
            if let Some(original) = repeated_from {
                line["repeated_from"] = json!(original);
            }
            // Only on jobs that didn't run, as with `repeated_from`.
            if let Some(reason) = reused {
                line["reused"] = json!(reason);
            }
            line
        }
    };
    Some(line)
}

async fn begin(path: &Path, config: &ExecutionConfig, options: &RunOptions) -> Arc<Recorder> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let git = git_info(&cwd).await;
    let cwd_text = cwd.to_string_lossy().into_owned();
    let record = RunRecord {
        schema_version: SCHEMA_VERSION,
        id: new_run_id(),
        status: RunStatus::Running,
        workflow: path.to_string_lossy().into_owned(),
        repo_root: git.root.unwrap_or_else(|| cwd_text.clone()),
        cwd: cwd_text,
        git_sha: git.sha,
        git_branch: git.branch,
        git_dirty: git.dirty,
        runtime: config.runtime_type.mode_name().to_string(),
        event: options.meta.event.clone(),
        source: options.meta.source.clone(),
        agent: options.meta.agent.clone(),
        intent: options.meta.intent.clone(),
        session: options.meta.session.clone(),
        replay_of: options.meta.replay_of.clone(),
        from_step: options.from_step.as_ref().map(|from| from.step.clone()),
        argv: options.meta.argv.clone(),
        variables: options.meta.variables.clone(),
        pid: std::process::id(),
        started_at: chrono::Utc::now().to_rfc3339(),
        finished_at: None,
        duration_ms: None,
        error: None,
        skip_reason: None,
        jobs: Vec::new(),
    };
    let persistent = (options.record && !history_disabled())
        .then(RunStore::open_default)
        .flatten();
    let is_persistent = persistent.is_some();
    let scratch = if is_persistent {
        None
    } else {
        tempfile::tempdir().ok()
    };
    let store = persistent.unwrap_or_else(|| match &scratch {
        Some(dir) => RunStore::at(dir.path()),
        None => RunStore::at(std::env::temp_dir().join("wrkflw-runs")),
    });
    let recorder = Arc::new(Recorder {
        store,
        persistent: is_persistent,
        _scratch: scratch,
        state: Mutex::new(record.clone()),
        ndjson: options.ndjson,
        stream_output: options.ndjson && options.stream_output,
        started: Instant::now(),
    });
    recorder.persist(&record);
    if recorder.persistent {
        recorder.store.prune(keep_limit());
    }
    recorder.line(json!({
        "type": "run_started",
        "workflow": record.workflow,
        "runtime": record.runtime,
        "git_sha": record.git_sha,
        "agent": record.agent,
        "intent": record.intent,
        "session": record.session,
    }));
    recorder
}

fn keep_limit() -> usize {
    non_empty_env(env::RUNS_KEEP)
        .and_then(|keep| keep.parse().ok())
        .unwrap_or(DEFAULT_KEEP)
}

fn run_finished_json(record: &RunRecord, digest: Option<&FailureDigest>) -> Value {
    json!({
        "type": "run_finished",
        "status": record.status.as_str(),
        "exit_code": i32::from(matches!(
            record.status,
            RunStatus::Failure | RunStatus::Error | RunStatus::Interrupted
        )),
        "duration_ms": record.duration_ms,
        "jobs": record.jobs.iter().map(|job| {
            let mut entry = json!({
                "name": job.name,
                "status": job.status,
                "failure_allowed": job.failure_allowed,
            });
            if let Some(original) = &job.repeated_from {
                entry["repeated_from"] = json!(original);
            }
            if let Some(reason) = &job.reused {
                entry["reused"] = json!(reason);
            }
            if let Some(runner) = &job.runner {
                entry["runner"] = json!(runner);
            }
            if !job.actions_not_run.is_empty() {
                entry["actions_not_run"] = json!(job.actions_not_run);
            }
            entry
        }).collect::<Vec<_>>(),
        "unverified": unverified(record),
        "verdict": verdict(record),
        "error": record.error,
        "skip_reason": record.skip_reason,
        "failure": digest,
    })
}

/// How much a run's result proves. A pass splits three ways, so a reader of
/// one word can't take a partial check for a full one:
///
/// - `verified`: something ran, every job that ran did so as the OS its
///   `runs-on` asked for, and every `uses:` step ran its action.
/// - `partial`: it passed, but some jobs ran as another OS or passed a step
///   without running its action (`unverified`).
/// - `nothing_checked`: it passed because no job ran, all skipped by `if:`,
///   `needs` or `gate: false`.
///
/// A job that failed under `continue-on-error` ran, so it counts as checked.
///
/// Anything else is the status: `failed`, `error`, `skipped`, `interrupted`.
pub fn verdict(record: &RunRecord) -> &'static str {
    match record.status {
        RunStatus::Success => {
            if !record.jobs.iter().any(job_ran) {
                "nothing_checked"
            } else if !unverified(record).is_empty() {
                "partial"
            } else {
                "verified"
            }
        }
        RunStatus::Failure => "failed",
        RunStatus::Skipped => "skipped",
        RunStatus::Interrupted => "interrupted",
        RunStatus::Error | RunStatus::Running => "error",
    }
}

/// Whether `job` ran steps of its own: it passed, or failed under
/// `continue-on-error`. A skipped job ran nothing.
fn job_ran(job: &JobRecord) -> bool {
    job.status == "success" || (job.status == "failure" && job.failure_allowed)
}

/// Jobs whose result can't stand for GitHub's, because they ran as another
/// OS than `runs-on` asked for, or passed a `uses:` step without running its
/// action. A skipped job ran nothing, so it isn't one.
fn unverified(record: &RunRecord) -> Vec<Value> {
    record
        .jobs
        .iter()
        .filter(|job| job.status != "skipped")
        .flat_map(|job| {
            let runner = job.runner.as_ref().map(|r| r.unverified_entry(&job.name));
            let actions = job
                .actions_not_run
                .iter()
                .map(|action| action.unverified_entry(&job.name));
            runner.into_iter().chain(actions)
        })
        .collect()
}

/// Execute a workflow with recording and event streaming per `options`.
/// Set once this process has been told to stop.
static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Record this process's runs as interrupted, whatever their steps do next.
/// Call it from the signal handler, before cleaning up.
///
/// Ctrl+C reaches a run's steps as well as wrkflw, since they share the
/// terminal's process group, and so does an MCP timeout. A step killed by
/// it fails, and the run could otherwise be recorded as a plain failure
/// before the handler exits, depending on which gets there first.
pub fn mark_interrupted() {
    INTERRUPTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

fn interrupted() -> bool {
    INTERRUPTED.load(std::sync::atomic::Ordering::SeqCst)
}

/// The status a run that returned `result` is recorded with.
fn final_status(
    result: &Result<ExecutionResult, ExecutionError>,
    interrupted: bool,
) -> (RunStatus, Option<String>, &[JobResult]) {
    let jobs: &[JobResult] = match result {
        Ok(run) => &run.jobs,
        Err(_) => &[],
    };
    if interrupted {
        return (RunStatus::Interrupted, None, jobs);
    }
    match result {
        Ok(run) if run.failure_details.is_some() => (RunStatus::Failure, None, jobs),
        Ok(_) => (RunStatus::Success, None, jobs),
        Err(e) => (RunStatus::Error, Some(e.to_string()), jobs),
    }
}

pub async fn execute_recorded(
    path: &Path,
    config: ExecutionConfig,
    options: RunOptions,
) -> RecordedRun {
    // Held until the run is recorded, so its values stay masked in what
    // is logged about it after `execute_workflow` returns.
    let _log_scope = wrkflw_executor::begin_log_scope();
    let recorder = begin(path, &config, &options).await;
    let mut observers: Vec<Arc<dyn ExecutionObserver>> = vec![recorder.clone()];
    observers.extend(options.observers.iter().cloned());
    let hooks = RunHooks {
        observers,
        from_step: options.from_step.clone(),
    };
    let result = with_run_hooks(hooks, wrkflw_executor::execute_workflow(path, config)).await;
    let (status, error, jobs) = final_status(&result, interrupted());
    let record = recorder.finish(status, error, None, jobs);
    conclude(recorder, record, result)
}

fn conclude(
    recorder: Arc<Recorder>,
    record: RunRecord,
    result: Result<ExecutionResult, ExecutionError>,
) -> RecordedRun {
    let mut digest = failure_digest(&recorder.store, &record, DEFAULT_TAIL_LINES);
    if !recorder.persistent {
        // The scratch directory dies with the recorder, and without a
        // history entry the replay command names nothing.
        if let Some(digest) = &mut digest {
            digest.replay.clear();
            digest.failures.iter_mut().for_each(|f| f.log_path = None);
        }
    }
    recorder.line(run_finished_json(&record, digest.as_ref()));
    RecordedRun {
        result,
        verdict: verdict(&record),
        unverified: unverified(&record),
        record: recorder.persistent.then_some(record),
        digest,
    }
}

/// Record that the trigger filter skipped a workflow, and report it.
pub async fn record_skipped(
    path: &Path,
    config: &ExecutionConfig,
    options: &RunOptions,
    reason: &str,
) {
    let recorder = begin(path, config, options).await;
    let record = recorder.finish(RunStatus::Skipped, None, Some(reason.to_string()), &[]);
    recorder.line(run_finished_json(&record, None));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrkflw_executor::{JobStatus, StepStatus};

    /// Environment variables, and what detection should make of them.
    type Case<'a> = (&'a [(&'a str, &'a str)], Option<&'a str>);

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn agents_are_detected_from_what_they_export() {
        let cases: &[Case] = &[
            (&[], None),
            (&[(env::CLAUDE_CODE, "1")], Some("claude-code")),
            (
                &[(env::AI_AGENT, "claude-code_2-1-314_agent")],
                Some("claude-code"),
            ),
            (
                &[(env::AI_AGENT, "github-copilot-cli")],
                Some("github-copilot-cli"),
            ),
            (
                &[(env::AI_AGENT, "_"), (env::GEMINI_CLI, "1")],
                Some("gemini"),
            ),
            (&[(env::CODEX_THREAD, "t")], Some("codex")),
            (&[(env::CODEX_SANDBOX, "seatbelt")], Some("codex")),
            (&[(env::OPENCODE_CLIENT, "cli")], Some("opencode")),
            (&[(env::CURSOR_AGENT, "1")], Some("cursor")),
            (&[(env::AUGMENT_AGENT, "1")], Some("augment")),
            (&[(env::AGENT, "me"), (env::CLAUDE_CODE, "1")], Some("me")),
        ];
        for (pairs, expected) in cases {
            assert_eq!(agent_from(vars(pairs)).as_deref(), *expected, "{pairs:?}");
        }
    }

    #[test]
    fn sessions_come_from_the_agent_unless_under_mcp() {
        let cases: &[Case] = &[
            (&[], None),
            (&[(env::CLAUDE_CODE_SESSION, "c")], Some("c")),
            (&[(env::CODEX_THREAD, "t")], Some("t")),
            (
                &[(env::SESSION, "s"), (env::CLAUDE_CODE_SESSION, "c")],
                Some("s"),
            ),
            (
                &[(env::MCP, "1"), (env::CLAUDE_CODE_SESSION, "stale")],
                None,
            ),
            (&[(env::MCP, "1"), (env::SESSION, "s")], Some("s")),
        ];
        for (pairs, expected) in cases {
            assert_eq!(session_from(vars(pairs)).as_deref(), *expected, "{pairs:?}");
        }
    }

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn job_dirs_keep_matrix_legs_apart() {
        assert_eq!(job_dir("build"), "build");
        assert_ne!(job_dir("test (a/b)"), job_dir("test (a-b)"));
        assert_ne!(job_dir(&"x".repeat(60)), job_dir(&"x".repeat(61)));
        assert_eq!(job_dir("test (a/b)"), job_dir("test (a/b)"));
    }

    // Field names are a public contract, so changing one must fail a test.
    #[test]
    fn event_fields_are_pinned() {
        let started = ExecutionEvent::JobStarted {
            job: "j".to_string(),
            canonical_name: "j".to_string(),
        };
        assert_eq!(
            keys(&event_json(&started).unwrap()),
            ["canonical_name", "job", "type"]
        );

        let step = ExecutionEvent::StepStarted {
            job: "j".to_string(),
            step: "s".to_string(),
            index: 0,
        };
        assert_eq!(
            keys(&event_json(&step).unwrap()),
            ["index", "job", "step", "type"]
        );

        let finished = ExecutionEvent::StepFinished {
            job: "j".to_string(),
            step: "s".to_string(),
            index: 0,
            status: StepStatus::Success,
            outcome: StepStatus::Success,
            conclusion: StepStatus::Success,
            exit_code: Some(0),
            duration_ms: 1,
            output: String::new(),
            command: None,
            uses: None,
            env_set: Vec::new(),
        };
        assert_eq!(
            keys(&event_json(&finished).unwrap()),
            [
                "conclusion",
                "duration_ms",
                "exit_code",
                "index",
                "job",
                "outcome",
                "output",
                "output_truncated",
                "status",
                "step",
                "type",
            ]
        );

        let job = ExecutionEvent::JobFinished {
            job: "j".to_string(),
            status: JobStatus::Success,
            failure_allowed: false,
            duration_ms: 1,
            logs: String::new(),
            repeated_from: None,
            reused: None,
        };
        assert_eq!(
            keys(&event_json(&job).unwrap()),
            ["duration_ms", "failure_allowed", "job", "status", "type"]
        );

        let repeat = ExecutionEvent::JobFinished {
            job: "j (os: b)".to_string(),
            status: JobStatus::Success,
            failure_allowed: false,
            duration_ms: 0,
            logs: String::new(),
            repeated_from: Some("j (os: a)".to_string()),
            reused: None,
        };
        let line = event_json(&repeat).unwrap();
        assert_eq!(line["repeated_from"], "j (os: a)");
        assert!(line.get("reused").is_none());

        let reused = ExecutionEvent::JobFinished {
            job: "j".to_string(),
            status: JobStatus::Success,
            failure_allowed: false,
            duration_ms: 0,
            logs: String::new(),
            repeated_from: None,
            reused: Some("stored".to_string()),
        };
        assert_eq!(event_json(&reused).unwrap()["reused"], "stored");

        let output = ExecutionEvent::StepOutput {
            job: "j".to_string(),
            step: "s".to_string(),
            index: 0,
            lines: vec!["printed".to_string()],
        };
        assert!(event_json(&output).is_none(), "live output reached NDJSON");
    }

    #[test]
    fn a_job_run_as_another_os_is_unverified_unless_skipped() {
        let job = |name: &str, status: &str| JobRecord {
            name: name.to_string(),
            canonical_name: "b".to_string(),
            status: status.to_string(),
            failure_allowed: false,
            duration_ms: None,
            log: None,
            steps: Vec::new(),
            repeated_from: None,
            reused: None,
            runner: Some(RunnerSubstitution {
                requested: "windows-latest".to_string(),
                ran_as: "Linux".to_string(),
            }),
            actions_not_run: Vec::new(),
        };
        let mut record: RunRecord = serde_json::from_value(json!({
            "schema_version": SCHEMA_VERSION, "id": "r", "status": "success",
            "workflow": "ci.yml", "cwd": "/repo", "repo_root": "/repo", "git_sha": null,
            "git_branch": null, "git_dirty": null, "runtime": "docker", "event": null,
            "source": "cli", "agent": null, "intent": null, "session": null,
            "replay_of": null, "argv": [], "variables": [], "pid": 1, "started_at": "",
            "finished_at": null, "duration_ms": null, "error": null, "skip_reason": null,
            "jobs": [],
        }))
        .unwrap();
        record.jobs = vec![
            job("b (os: windows-latest)", "success"),
            job("c", "skipped"),
        ];
        record.jobs[0].repeated_from = Some("b (os: ubuntu-latest)".to_string());
        record.jobs[0].reused = Some("stored".to_string());
        let line = run_finished_json(&record, None);
        assert_eq!(line["verdict"], "partial");
        assert_eq!(line["jobs"][0]["reused"], "stored");
        assert_eq!(line["unverified"].as_array().unwrap().len(), 1);
        assert_eq!(line["unverified"][0]["job"], "b (os: windows-latest)");
        assert_eq!(line["unverified"][0]["ran_as"], "Linux");
        assert_eq!(line["jobs"][0]["repeated_from"], "b (os: ubuntu-latest)");
        assert_eq!(line["jobs"][0]["runner"]["requested"], "windows-latest");
        assert!(line["jobs"][1].get("repeated_from").is_none());
    }

    #[test]
    fn a_job_that_passed_a_step_without_running_its_action_is_unverified() {
        let mut record: RunRecord = serde_json::from_value(json!({
            "schema_version": SCHEMA_VERSION, "id": "r", "status": "success",
            "workflow": "ci.yml", "cwd": "/repo", "repo_root": "/repo", "git_sha": null,
            "git_branch": null, "git_dirty": null, "runtime": "docker", "event": null,
            "source": "cli", "agent": null, "intent": null, "session": null,
            "replay_of": null, "argv": [], "variables": [], "pid": 1, "started_at": "",
            "finished_at": null, "duration_ms": null, "error": null, "skip_reason": null,
            "jobs": [
                {"name": "tests", "canonical_name": "tests", "status": "success",
                 "failure_allowed": false, "duration_ms": 1, "log": null, "steps": []},
                {"name": "lint", "canonical_name": "lint", "status": "success",
                 "failure_allowed": false, "duration_ms": 1, "log": null, "steps": []},
            ],
        }))
        .unwrap();
        // An old record without the field reads as a job that ran everything.
        assert!(record.jobs[0].actions_not_run.is_empty());
        record.jobs[0].actions_not_run.push(UnexecutedAction {
            step: "Install uv".to_string(),
            uses: "astral-sh/setup-uv@v9".to_string(),
        });
        let line = run_finished_json(&record, None);
        assert_eq!(line["verdict"], "partial");
        assert_eq!(line["unverified"].as_array().unwrap().len(), 1);
        assert_eq!(line["unverified"][0]["job"], "tests");
        assert_eq!(line["unverified"][0]["step"], "Install uv");
        assert_eq!(line["unverified"][0]["uses"], "astral-sh/setup-uv@v9");
        assert_eq!(
            line["jobs"][0]["actions_not_run"][0]["uses"],
            "astral-sh/setup-uv@v9"
        );
        assert!(line["jobs"][1].get("actions_not_run").is_none());
    }

    #[test]
    fn a_stopped_run_is_interrupted_however_its_steps_ended() {
        let failed: Result<ExecutionResult, ExecutionError> = Ok(ExecutionResult {
            jobs: Vec::new(),
            failure_details: Some("step boom failed".to_string()),
        });
        assert_eq!(final_status(&failed, false).0, RunStatus::Failure);
        assert_eq!(final_status(&failed, true).0, RunStatus::Interrupted);
        let broken: Result<ExecutionResult, ExecutionError> =
            Err(ExecutionError::Execution("boom".to_string()));
        let (status, error, _) = final_status(&broken, true);
        assert_eq!(status, RunStatus::Interrupted);
        assert_eq!(error, None);
        assert_eq!(
            final_status(&broken, false).1.as_deref(),
            Some("Execution error: boom")
        );
    }

    #[test]
    fn run_finished_fields_are_pinned() {
        let record: RunRecord = serde_json::from_value(json!({
            "schema_version": SCHEMA_VERSION,
            "id": "r",
            "status": "success",
            "workflow": "ci.yml",
            "cwd": "/repo",
            "repo_root": "/repo",
            "git_sha": null,
            "git_branch": null,
            "git_dirty": null,
            "runtime": "emulation",
            "event": null,
            "source": "cli",
            "agent": null,
            "intent": null,
            "session": null,
            "replay_of": null,
            "argv": [],
            "variables": [],
            "pid": 1,
            "started_at": "",
            "finished_at": null,
            "duration_ms": null,
            "error": null,
            "skip_reason": null,
            "jobs": [],
        }))
        .unwrap();
        assert_eq!(
            keys(&run_finished_json(&record, None)),
            [
                "duration_ms",
                "error",
                "exit_code",
                "failure",
                "jobs",
                "skip_reason",
                "status",
                "type",
                "unverified",
                "verdict",
            ]
        );
    }

    #[test]
    fn a_pass_says_how_much_it_proves() {
        let job = |status: &str, runner: Option<RunnerSubstitution>| JobRecord {
            name: "j".to_string(),
            canonical_name: "j".to_string(),
            status: status.to_string(),
            failure_allowed: false,
            duration_ms: None,
            log: None,
            steps: Vec::new(),
            repeated_from: None,
            reused: None,
            runner,
            actions_not_run: Vec::new(),
        };
        let mut record: RunRecord = serde_json::from_value(json!({
            "schema_version": SCHEMA_VERSION, "id": "r", "status": "success",
            "workflow": "ci.yml", "cwd": "/repo", "repo_root": "/repo", "git_sha": null,
            "git_branch": null, "git_dirty": null, "runtime": "docker", "event": null,
            "source": "cli", "agent": null, "intent": null, "session": null,
            "replay_of": null, "argv": [], "variables": [], "pid": 1, "started_at": "",
            "finished_at": null, "duration_ms": null, "error": null, "skip_reason": null,
            "jobs": [],
        }))
        .unwrap();
        assert_eq!(verdict(&record), "nothing_checked");
        record.jobs = vec![job("skipped", None)];
        assert_eq!(verdict(&record), "nothing_checked");
        record.jobs.push(job("success", None));
        assert_eq!(verdict(&record), "verified");
        record.jobs.push(job(
            "success",
            Some(RunnerSubstitution {
                requested: "windows-latest".to_string(),
                ran_as: "Linux".to_string(),
            }),
        ));
        assert_eq!(verdict(&record), "partial");
        record.status = RunStatus::Failure;
        assert_eq!(verdict(&record), "failed");

        // A job that failed under continue-on-error ran, though the run passed.
        record.status = RunStatus::Success;
        let mut allowed = job("failure", None);
        allowed.failure_allowed = true;
        record.jobs = vec![job("skipped", None), allowed];
        assert_eq!(verdict(&record), "verified");
        record.jobs[1].failure_allowed = false;
        assert_eq!(verdict(&record), "nothing_checked");
    }
}
