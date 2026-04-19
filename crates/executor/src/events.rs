// Live execution-event stream emitted by the executor for consumers that
// want to observe workflow progress in real time (notably the TUI).
//
// The CLI path is unchanged — callers that do not supply an `event_sink`
// in their `ExecutionConfig` see no behavioral difference and no events
// are produced.
//
// Events carry a monotonically-allocated `JobId` so that concurrent
// matrix / parallel jobs can be disambiguated at the receiver.

use chrono::{DateTime, Local};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

use crate::engine::{JobStatus, StepStatus};

/// Per-run identifier for a running job. Matrix combinations and parallel
/// jobs each get their own `JobId` even when they share a canonical name.
pub type JobId = u64;

/// Re-export of the runtime's stream-tag enum so executor consumers (e.g.
/// the TUI) don't need to depend on `wrkflw-runtime` directly. The runtime
/// owns the definition because the type appears in the `ContainerRuntime`
/// trait signature.
pub use wrkflw_runtime::container::LogStream;

/// Events emitted by the executor as a workflow runs. Consumers should treat
/// the stream as append-only and route by `job_id` / `step_idx`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ExecutionEvent {
    ExecutionStarted {
        workflow_name: String,
        started_at: DateTime<Local>,
    },
    JobStarted {
        job_id: JobId,
        /// Display name (matrix-expanded, e.g. "build (os: ubuntu)").
        name: String,
        /// Canonical job key from the workflow definition.
        canonical_name: String,
        started_at: DateTime<Local>,
    },
    StepStarted {
        job_id: JobId,
        step_idx: usize,
        name: String,
        started_at: DateTime<Local>,
    },
    StepLogChunk {
        job_id: JobId,
        step_idx: usize,
        stream: LogStream,
        /// Raw bytes from the container / child process, decoded lossily as
        /// UTF-8. May include partial lines at chunk boundaries; the UI is
        /// responsible for line-splitting.
        data: String,
    },
    StepCompleted {
        job_id: JobId,
        step_idx: usize,
        /// Final status after `continue-on-error` is applied.
        status: StepStatus,
        /// Raw status (may differ from `status` when `continue-on-error` is set).
        outcome: StepStatus,
        /// Effective status (same as `status`; duplicated for symmetry with
        /// `StepResult`).
        conclusion: StepStatus,
        ended_at: DateTime<Local>,
    },
    JobCompleted {
        job_id: JobId,
        status: JobStatus,
        ended_at: DateTime<Local>,
    },
    ExecutionCompleted {
        ended_at: DateTime<Local>,
    },
}

/// Handle used by the executor to emit `ExecutionEvent`s. Bundles the
/// unbounded sender together with the per-run `JobId` allocator so the
/// two always travel together through the call graph.
///
/// Cheap to clone (`Arc`-backed); use the `clone()` in spawned tasks.
///
/// Unbounded because: (1) the UI side caps per-step log buffers after
/// consumption so memory stays bounded regardless, and (2) applying
/// backpressure to the executor from the UI is undesirable — the executor
/// should never block on a slow consumer.
#[derive(Clone)]
pub struct EventSink {
    sender: Arc<UnboundedSender<ExecutionEvent>>,
    allocator: Arc<JobIdAllocator>,
}

impl EventSink {
    pub fn new(sender: UnboundedSender<ExecutionEvent>) -> Self {
        Self {
            sender: Arc::new(sender),
            allocator: Arc::new(JobIdAllocator::new()),
        }
    }

    /// Mint a fresh `JobId`.
    pub fn allocate_job_id(&self) -> JobId {
        self.allocator.allocate()
    }

    /// Fire-and-forget send. Channel closure means the consumer has hung
    /// up — not worth escalating from inside a running executor.
    pub fn emit(&self, ev: ExecutionEvent) {
        let _ = self.sender.send(ev);
    }
}

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventSink").finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct JobIdAllocator {
    next: AtomicU64,
}

impl JobIdAllocator {
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
        }
    }

    pub fn allocate(&self) -> JobId {
        self.next.fetch_add(1, Ordering::Relaxed)
    }
}

/// Helper: emit to an optional sink. No-op when absent (CLI path).
pub(crate) fn emit(sink: Option<&EventSink>, ev: ExecutionEvent) {
    if let Some(s) = sink {
        s.emit(ev);
    }
}
