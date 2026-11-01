// Emulation runs `run:` steps through a POSIX shell.
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobResult, JobStatus, RuntimeType,
    StepStatus,
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

fn job<'a>(result: &'a ExecutionResult, name: &str) -> &'a JobResult {
    result
        .jobs
        .iter()
        .find(|job| job.name == name)
        .unwrap_or_else(|| panic!("no job named {}", name))
}

fn status(result: &ExecutionResult, name: &str) -> JobStatus {
    job(result, name).status.clone()
}

fn step_status(job: &JobResult, name: &str) -> StepStatus {
    job.steps
        .iter()
        .find(|step| step.name == name)
        .unwrap_or_else(|| panic!("no step named {}", name))
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
async fn dependents_read_an_allowed_failure_as_success() {
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
      - name: saw-success
        if: ${{ needs.flaky.result == 'success' }}
        run: echo ran
      - name: saw-failure
        if: ${{ needs.flaky.result == 'failure' }}
        run: echo ran
"#)
    .await;

    let after = job(&result, "after");
    assert_eq!(step_status(after, "saw-success"), StepStatus::Success);
    assert_eq!(step_status(after, "saw-failure"), StepStatus::Skipped);
}

#[tokio::test]
async fn explicit_false_still_fails_the_run() {
    let result = run(r#"
name: t
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    continue-on-error: false
    steps:
      - run: exit 1
"#)
    .await;

    assert!(!job(&result, "build").failure_allowed);
    assert!(result.failure_details.is_some());
}

#[tokio::test]
async fn unevaluable_expression_warns_and_counts_the_failure() {
    let result = run(r#"
name: t
on: push
jobs:
  unevaluable-job:
    runs-on: ubuntu-latest
    continue-on-error: ${{ 1 == }}
    steps:
      - run: exit 1
"#)
    .await;

    assert!(!job(&result, "unevaluable-job").failure_allowed);
    assert!(result.failure_details.is_some());
    assert!(wrkflw_logging::get_logs()
        .iter()
        .any(|line| line.contains("a failure here will still count")));
}

#[tokio::test]
async fn services_warn_that_they_are_not_started() {
    run(r#"
name: t
on: push
jobs:
  with-services:
    runs-on: ubuntu-latest
    services:
      redis:
        image: redis
    steps:
      - run: echo ran
"#)
    .await;

    assert!(wrkflw_logging::get_logs()
        .iter()
        .any(|line| line.contains("Job 'with-services' declares services")));
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
    assert!(job(&result, "build (experimental: true)").failure_allowed);
    assert!(!job(&result, "build (experimental: false)").failure_allowed);

    let details = result
        .failure_details
        .expect("stable leg must fail the run");
    assert!(details.contains("build (experimental: false)"));
    assert!(!details.contains("build (experimental: true)"));
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
