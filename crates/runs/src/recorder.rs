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
    RunHooks,
};

/// Step output carried inline in an NDJSON event. The full text is in the log.
const MAX_EVENT_OUTPUT_BYTES: usize = 64 * 1024;

/// What the caller knows about a run. Everything else is detected.
#[derive(Clone, Debug, Default)]
pub struct RunMeta {
    pub source: String,
    pub agent: Option<String>,
    pub intent: Option<String>,
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
    non_empty_env(env::AGENT)
        .or_else(|| non_empty_env(env::CLAUDE_CODE).map(|_| "claude-code".to_string()))
}

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub meta: RunMeta,
    /// Write NDJSON events to stdout.
    pub ndjson: bool,
    /// Keep a history record. Also subject to `WRKFLW_NO_HISTORY`.
    pub record: bool,
    pub from_step: Option<FromStep>,
}

pub struct RecordedRun {
    pub result: Result<ExecutionResult, ExecutionError>,
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
            let job_slug = slug(&job.name);
            let mut steps = Vec::new();
            for (index, step) in job.steps.iter().enumerate() {
                let relative = format!("logs/{job_slug}/{:02}-{}.log", index, slug(&step.name));
                steps.push(StepRecord {
                    index,
                    name: step.name.clone(),
                    status: step.status.to_string(),
                    outcome: step.outcome.to_string(),
                    conclusion: step.conclusion.to_string(),
                    exit_code: step.exit_code(),
                    duration_ms: None,
                    command: None,
                    uses: None,
                    env_set: Vec::new(),
                    log: self.write_log(&id, &relative, &step.output),
                });
            }
            let relative = format!("logs/{job_slug}/job.log");
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
    fn on_event(&self, event: &ExecutionEvent) {
        self.line(event_json(event));
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
                let relative = format!("logs/{}/{:02}-{}.log", slug(job), index, slug(step));
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
            } => {
                let relative = format!("logs/{}/job.log", slug(job));
                let log = self.write_log(&id, &relative, logs);
                let entry = Self::job_mut(&mut record, job, job);
                entry.status = status.to_string();
                entry.failure_allowed = *failure_allowed;
                entry.duration_ms = Some(*duration_ms);
                entry.log = log;
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

fn event_json(event: &ExecutionEvent) -> Value {
    match event {
        ExecutionEvent::JobStarted {
            job,
            canonical_name,
        } => json!({"type": "job_started", "job": job, "canonical_name": canonical_name}),
        ExecutionEvent::StepStarted { job, step, index } => {
            json!({"type": "step_started", "job": job, "step": step, "index": index})
        }
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
            ..
        } => json!({
            "type": "job_finished",
            "job": job,
            "status": status.to_string(),
            "failure_allowed": failure_allowed,
            "duration_ms": duration_ms,
        }),
    }
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
        replay_of: options.meta.replay_of.clone(),
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
        "exit_code": i32::from(matches!(record.status, RunStatus::Failure | RunStatus::Error)),
        "duration_ms": record.duration_ms,
        "jobs": record.jobs.iter().map(|job| json!({
            "name": job.name,
            "status": job.status,
            "failure_allowed": job.failure_allowed,
        })).collect::<Vec<_>>(),
        "error": record.error,
        "skip_reason": record.skip_reason,
        "failure": digest,
    })
}

/// Execute a workflow with recording and event streaming per `options`.
pub async fn execute_recorded(
    path: &Path,
    config: ExecutionConfig,
    options: RunOptions,
) -> RecordedRun {
    let recorder = begin(path, &config, &options).await;
    let hooks = RunHooks {
        observers: vec![recorder.clone()],
        from_step: options.from_step.clone(),
    };
    let result = with_run_hooks(hooks, wrkflw_executor::execute_workflow(path, config)).await;
    let (status, error, jobs): (_, _, &[JobResult]) = match &result {
        Ok(run) if run.failure_details.is_some() => (RunStatus::Failure, None, &run.jobs),
        Ok(run) => (RunStatus::Success, None, &run.jobs),
        Err(e) => (RunStatus::Error, Some(e.to_string()), &[]),
    };
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
