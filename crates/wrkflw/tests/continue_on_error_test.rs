// Emulation runs `run:` steps through a POSIX shell.
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
};

async fn run(yaml: &str) -> ExecutionResult {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    fs::write(&path, yaml).unwrap();
    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        platform_overrides: Default::default(),
        kubernetes: Default::default(),
    };
    execute_workflow(&path, cfg).await.unwrap()
}

fn status(result: &ExecutionResult, name: &str) -> JobStatus {
    result
        .jobs
        .iter()
        .find(|job| job.name == name)
        .unwrap_or_else(|| panic!("no job named {}", name))
        .status
        .clone()
}

#[tokio::test]
async fn allowed_job_failure_keeps_the_run_green_and_dependents_running() {
    let result = run(r#"
name: t
on: push
jobs:
  flaky:
    runs-on: ubuntu-latest
    continue-on-error: true
    steps:
      - run: exit 1
  after:
    needs: flaky
    runs-on: ubuntu-latest
    steps:
      - run: echo ran
"#)
    .await;

    assert_eq!(status(&result, "flaky"), JobStatus::Failure);
    assert_eq!(status(&result, "after"), JobStatus::Success);
    assert!(result.failure_details.is_none());
}

#[tokio::test]
async fn job_expression_applies_per_matrix_combination() {
    let result = run(r#"
name: t
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    continue-on-error: ${{ matrix.experimental }}
    strategy:
      fail-fast: false
      matrix:
        experimental: [true, false]
    steps:
      - run: exit 1
"#)
    .await;

    assert!(result.jobs.iter().all(|j| j.status == JobStatus::Failure));
    assert!(
        result.failure_details.is_some(),
        "stable leg must fail the run"
    );
}

#[tokio::test]
async fn allowed_matrix_failure_does_not_trigger_fail_fast() {
    let result = run(r#"
name: t
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    continue-on-error: true
    strategy:
      max-parallel: 1
      matrix:
        n: [1, 2, 3]
    steps:
      - run: exit 1
"#)
    .await;

    assert_eq!(result.jobs.len(), 3);
    assert!(result.failure_details.is_none());
}

#[tokio::test]
async fn step_expression_is_accepted_and_evaluated() {
    let result = run(r#"
name: t
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - id: a
        continue-on-error: ${{ 1 == 1 }}
        run: exit 1
      - id: b
        continue-on-error: ${{ 1 == 2 }}
        run: exit 1
      - run: echo never
"#)
    .await;

    let job = &result.jobs[0];
    assert_eq!(job.status, JobStatus::Failure);
    assert_eq!(job.steps.len(), 2, "second step must stop the job");
}
