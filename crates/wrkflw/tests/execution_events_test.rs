// Integration test for the executor's event stream. Runs a small two-job,
// three-step workflow with emulation runtime + an attached event sink and
// asserts the shape of the emitted event sequence.

use std::fs;
use tempfile::tempdir;
use tokio::sync::mpsc::unbounded_channel;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};
use wrkflw_lib::executor::events::{EventSink, ExecutionEvent};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

fn drain_events(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<ExecutionEvent>,
) -> Vec<ExecutionEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

#[tokio::test]
async fn event_stream_emits_expected_sequence_for_simple_workflow() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    let workflow = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "step-1"
      - run: echo "step-2"
  verify:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo "step-3"
"#;
    write_file(&workflow_path, workflow);

    let (tx, rx) = unbounded_channel();
    let sink = EventSink::new(tx);

    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        event_sink: Some(sink),
    };

    let _result = execute_workflow(&workflow_path, cfg)
        .await
        .expect("workflow execution failed");

    // Drain after execution — with an UnboundedSender the executor never
    // blocks and we don't need to consume concurrently.
    let events = drain_events(rx);

    assert!(!events.is_empty(), "expected some events");
    assert!(
        matches!(
            events.first(),
            Some(ExecutionEvent::ExecutionStarted { .. })
        ),
        "first event must be ExecutionStarted, got {:?}",
        events.first()
    );
    assert!(
        matches!(
            events.last(),
            Some(ExecutionEvent::ExecutionCompleted { .. })
        ),
        "last event must be ExecutionCompleted, got {:?}",
        events.last()
    );

    // Count job_id values seen via JobStarted — there are 2 jobs in the plan.
    let mut job_started_ids = Vec::new();
    let mut job_completed_ids = Vec::new();
    let mut step_started = Vec::new();
    let mut step_completed = Vec::new();
    for ev in &events {
        match ev {
            ExecutionEvent::JobStarted { job_id, .. } => job_started_ids.push(*job_id),
            ExecutionEvent::JobCompleted { job_id, .. } => job_completed_ids.push(*job_id),
            ExecutionEvent::StepStarted {
                job_id, step_idx, ..
            } => step_started.push((*job_id, *step_idx)),
            ExecutionEvent::StepCompleted {
                job_id, step_idx, ..
            } => step_completed.push((*job_id, *step_idx)),
            _ => {}
        }
    }

    assert_eq!(
        job_started_ids.len(),
        2,
        "expected 2 JobStarted events, got {:?}",
        job_started_ids
    );
    assert_eq!(job_completed_ids.len(), 2, "expected 2 JobCompleted events");
    assert_eq!(step_started.len(), 3, "expected 3 StepStarted events");
    assert_eq!(step_completed.len(), 3, "expected 3 StepCompleted events");

    // Every StepStarted must have a matching StepCompleted under the same
    // (job_id, step_idx).
    for pair in &step_started {
        assert!(
            step_completed.contains(pair),
            "missing StepCompleted for {:?}",
            pair
        );
    }

    // For a single job, all its StepStarted events must precede its
    // JobCompleted. Indexing directly in the event vec lets us enforce it.
    for job_id in &job_completed_ids {
        let completed_idx = events
            .iter()
            .position(
                |ev| matches!(ev, ExecutionEvent::JobCompleted { job_id: j, .. } if j == job_id),
            )
            .unwrap();
        for (i, ev) in events.iter().enumerate() {
            if let ExecutionEvent::StepStarted { job_id: j, .. } = ev {
                if j == job_id {
                    assert!(
                        i < completed_idx,
                        "StepStarted for job {} at idx {} must precede JobCompleted at idx {}",
                        job_id,
                        i,
                        completed_idx
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn no_events_emitted_when_sink_absent() {
    // Smoke check: the CLI path with event_sink: None must run unchanged.
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");
    let workflow = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "hello"
"#;
    write_file(&workflow_path, workflow);

    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        event_sink: None,
    };

    // Just assert we don't panic / still return a valid result.
    let result = execute_workflow(&workflow_path, cfg)
        .await
        .expect("workflow should succeed");
    assert_eq!(result.jobs.len(), 1);
}
