// Integration test: when an event sink is provided, the executor emits
// `StepLogChunk` events for stdout/stderr *before* the matching
// `StepCompleted`. Uses emulation runtime so no Docker/Podman needed.

use std::fs;
use tempfile::tempdir;
use tokio::sync::mpsc::unbounded_channel;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};
use wrkflw_lib::executor::events::{EventSink, ExecutionEvent};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

#[tokio::test]
async fn step_log_chunks_arrive_before_step_completed() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    // Single step that prints three distinct lines. The emulation runtime
    // runs the `run:` block as `sh -c <script>`; the `echo` lines should
    // flow back through the log sink verbatim.
    let workflow = r#"
name: CI
on: push
jobs:
  speak:
    runs-on: ubuntu-latest
    steps:
      - run: |
          echo "alpha"
          echo "beta"
          echo "gamma"
"#;
    write_file(&workflow_path, workflow);

    let (tx, mut rx) = unbounded_channel();
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

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }

    // Collect indices for the relevant events.
    let step_started_idx = events
        .iter()
        .position(|ev| matches!(ev, ExecutionEvent::StepStarted { .. }))
        .expect("expected a StepStarted event");
    let step_completed_idx = events
        .iter()
        .position(|ev| matches!(ev, ExecutionEvent::StepCompleted { .. }))
        .expect("expected a StepCompleted event");

    // Any StepLogChunk for this step must land between Started and Completed.
    let mut chunks_in_order = Vec::new();
    for (i, ev) in events.iter().enumerate() {
        if let ExecutionEvent::StepLogChunk { data, .. } = ev {
            assert!(
                i > step_started_idx,
                "StepLogChunk at idx {} fired before StepStarted at idx {}",
                i,
                step_started_idx
            );
            assert!(
                i < step_completed_idx,
                "StepLogChunk at idx {} fired after StepCompleted at idx {}",
                i,
                step_completed_idx
            );
            chunks_in_order.push(data.clone());
        }
    }

    assert!(
        !chunks_in_order.is_empty(),
        "expected at least one StepLogChunk event, got events: {:?}",
        events
    );

    // Concatenation of all chunks should contain the three echo lines, in
    // order. Line endings vary by platform; assert substring containment.
    let all_chunks: String = chunks_in_order.concat();
    assert!(
        all_chunks.contains("alpha"),
        "missing alpha in {:?}",
        all_chunks
    );
    assert!(
        all_chunks.contains("beta"),
        "missing beta in {:?}",
        all_chunks
    );
    assert!(
        all_chunks.contains("gamma"),
        "missing gamma in {:?}",
        all_chunks
    );
    let i_alpha = all_chunks.find("alpha").unwrap();
    let i_beta = all_chunks.find("beta").unwrap();
    let i_gamma = all_chunks.find("gamma").unwrap();
    assert!(
        i_alpha < i_beta && i_beta < i_gamma,
        "ordering broken: {:?}",
        all_chunks
    );
}
