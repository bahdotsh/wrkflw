// These tests run real `bash` steps through the emulation runtime. On
// windows-latest `bash` resolves to the WSL launcher, which has no distribution
// installed, so every step fails. Engine unit tests use the same gate.
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

fn emulation_cfg() -> ExecutionConfig {
    ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        platform_overrides: Default::default(),
    }
}

fn status_of<'a>(result: &'a ExecutionResult, job: &str) -> &'a str {
    let jr = result
        .jobs
        .iter()
        .find(|j| j.canonical_name == job)
        .unwrap_or_else(|| {
            let names: Vec<&str> = result
                .jobs
                .iter()
                .map(|j| j.canonical_name.as_str())
                .collect();
            panic!("job '{}' not in results: {:?}", job, names)
        });
    match jr.status {
        JobStatus::Success => "success",
        JobStatus::Failure => "failure",
        JobStatus::Skipped => "skipped",
    }
}

/// A job whose `needs:` dependency failed must be skipped, not run.
#[tokio::test]
async fn dependent_job_is_skipped_when_need_fails() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    write_file(
        &path,
        r#"
name: CI
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  e2e:
    runs-on: ubuntu-latest
    needs: [deploy]
    steps:
      - run: echo "should not run"
"#,
    );

    let result = execute_workflow(&path, emulation_cfg())
        .await
        .expect("workflow execution completed");

    assert_eq!(status_of(&result, "deploy"), "failure");
    assert_eq!(
        status_of(&result, "e2e"),
        "skipped",
        "e2e must be skipped because deploy failed"
    );
}

/// The skip cascades: a job that needs a skipped job is skipped too.
#[tokio::test]
async fn skip_cascades_through_the_needs_chain() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    write_file(
        &path,
        r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  test:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo hi
  publish:
    runs-on: ubuntu-latest
    needs: [test]
    steps:
      - run: echo hi
"#,
    );

    let result = execute_workflow(&path, emulation_cfg())
        .await
        .expect("workflow execution completed");

    assert_eq!(status_of(&result, "build"), "failure");
    assert_eq!(status_of(&result, "test"), "skipped");
    assert_eq!(status_of(&result, "publish"), "skipped");
}

/// An explicit `if:` opts a dependent job back in even when its need failed.
#[tokio::test]
async fn explicit_if_condition_still_runs_after_a_failed_need() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    write_file(
        &path,
        r#"
name: CI
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  notify:
    runs-on: ubuntu-latest
    needs: [deploy]
    if: always()
    steps:
      - run: echo "report the failure"
"#,
    );

    let result = execute_workflow(&path, emulation_cfg())
        .await
        .expect("workflow execution completed");

    assert_eq!(status_of(&result, "deploy"), "failure");
    assert_eq!(
        status_of(&result, "notify"),
        "success",
        "notify has `if: always()` and must still run"
    );
}

/// Independent jobs in a later batch are unaffected by an unrelated failure.
#[tokio::test]
async fn unrelated_jobs_still_run_after_a_failure() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    write_file(
        &path,
        r#"
name: CI
on: push
jobs:
  a:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  b:
    runs-on: ubuntu-latest
    needs: [a]
    steps:
      - run: echo hi
  c:
    runs-on: ubuntu-latest
    steps:
      - run: echo independent
"#,
    );

    let result = execute_workflow(&path, emulation_cfg())
        .await
        .expect("workflow execution completed");

    assert_eq!(status_of(&result, "a"), "failure");
    assert_eq!(status_of(&result, "b"), "skipped");
    assert_eq!(status_of(&result, "c"), "success");
}

/// Run an inline workflow and return the result.
async fn run(yaml: &str) -> ExecutionResult {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    write_file(&path, yaml);
    execute_workflow(&path, emulation_cfg())
        .await
        .expect("workflow execution completed")
}

/// `if: failure()` must run when a need failed, and `if: success()` must not.
#[tokio::test]
async fn status_functions_see_the_failed_need() {
    let result = run(r#"
name: CI
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  on_failure:
    runs-on: ubuntu-latest
    needs: [deploy]
    if: failure()
    steps:
      - run: echo "cleanup"
  on_success:
    runs-on: ubuntu-latest
    needs: [deploy]
    if: success()
    steps:
      - run: echo "should not run"
"#)
    .await;

    assert_eq!(status_of(&result, "deploy"), "failure");
    assert_eq!(status_of(&result, "on_failure"), "success");
    assert_eq!(status_of(&result, "on_success"), "skipped");
}

/// `needs.<id>.result` is populated when a job-level `if:` is evaluated.
#[tokio::test]
async fn needs_result_is_visible_to_job_if() {
    let result = run(r#"
name: CI
on: push
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  report:
    runs-on: ubuntu-latest
    needs: [deploy]
    if: always() && needs.deploy.result == 'failure'
    steps:
      - run: echo "deploy failed"
"#)
    .await;

    assert_eq!(status_of(&result, "report"), "success");
}

/// An `if:` without a status function gets an implicit `success() &&`.
#[tokio::test]
async fn plain_if_is_still_gated_on_needs() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  publish:
    runs-on: ubuntu-latest
    needs: [build]
    if: ${{ true }}
    steps:
      - run: echo "should not run"
"#)
    .await;

    assert_eq!(status_of(&result, "publish"), "skipped");
}

/// A need skipped by its own `if:` skips its dependents.
#[tokio::test]
async fn need_skipped_by_its_own_if_skips_dependents() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: false
    steps:
      - run: echo hi
  test:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo hi
"#)
    .await;

    assert_eq!(status_of(&result, "build"), "skipped");
    assert_eq!(status_of(&result, "test"), "skipped");
}

/// One failed need is enough to skip, even when another need succeeded.
#[tokio::test]
async fn any_failed_need_among_several_skips() {
    let result = run(r#"
name: CI
on: push
jobs:
  ok:
    runs-on: ubuntu-latest
    steps:
      - run: echo ok
  bad:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  merge:
    runs-on: ubuntu-latest
    needs: [ok, bad]
    steps:
      - run: echo hi
"#)
    .await;

    assert_eq!(status_of(&result, "ok"), "success");
    assert_eq!(status_of(&result, "merge"), "skipped");
}

/// A failed grandparent keeps `success()` false even when the parent ran via
/// `if: always()`, matching GitHub's ancestor semantics.
#[tokio::test]
async fn ancestor_failure_propagates_past_an_always_job() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  notify:
    runs-on: ubuntu-latest
    needs: [build]
    if: always()
    steps:
      - run: echo notify
  deploy:
    runs-on: ubuntu-latest
    needs: [notify]
    steps:
      - run: echo "should not run"
  cleanup:
    runs-on: ubuntu-latest
    needs: [notify]
    if: failure()
    steps:
      - run: echo cleanup
"#)
    .await;

    assert_eq!(status_of(&result, "notify"), "success");
    assert_eq!(status_of(&result, "deploy"), "skipped");
    assert_eq!(status_of(&result, "cleanup"), "success");
}

/// Jobs inside a reusable workflow follow the same `needs:` rules.
#[tokio::test]
async fn reusable_workflow_jobs_skip_after_failed_need() {
    // Local reusable-workflow paths must stay inside the current directory
    // (the repository root), so the temp dir is created there.
    let dir = tempfile::Builder::new()
        .prefix(".needs-reusable-")
        .tempdir_in(".")
        .unwrap();
    let dir_name = dir
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let called = dir.path().join("called.yml");
    write_file(
        &called,
        r#"
name: Called
on: workflow_call
jobs:
  inner_build:
    runs-on: ubuntu-latest
    steps:
      - run: exit 1
  inner_deploy:
    runs-on: ubuntu-latest
    needs: [inner_build]
    steps:
      - run: echo "should not run"
"#,
    );
    let caller = dir.path().join("ci.yml");
    write_file(
        &caller,
        &format!(
            r#"
name: CI
on: push
jobs:
  call:
    uses: ./{}/called.yml
"#,
            dir_name
        ),
    );

    let result = execute_workflow(&caller, emulation_cfg())
        .await
        .expect("workflow execution completed");

    assert_eq!(status_of(&result, "call"), "failure");
    let logs = &result
        .jobs
        .iter()
        .find(|j| j.canonical_name == "call")
        .unwrap()
        .logs;
    assert!(
        logs.contains("- inner_deploy: Skipped"),
        "inner_deploy must be skipped because inner_build failed; logs:\n{}",
        logs
    );
}

/// A matrix job's result is the aggregate of its combinations: one failed
/// combination fails the job even when a later combination succeeded.
#[tokio::test]
async fn matrix_need_with_one_failed_combination_counts_as_failed() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        target: [bad, good]
    steps:
      - run: test "${{ matrix.target }}" = good
  deploy:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo "should not run"
  cleanup:
    runs-on: ubuntu-latest
    needs: [build]
    if: failure()
    steps:
      - run: echo cleanup
  report:
    runs-on: ubuntu-latest
    needs: [build]
    if: always() && needs.build.result == 'failure'
    steps:
      - run: echo report
"#)
    .await;

    let build: Vec<&str> = result
        .jobs
        .iter()
        .filter(|j| j.canonical_name == "build")
        .map(|j| match j.status {
            JobStatus::Success => "success",
            JobStatus::Failure => "failure",
            JobStatus::Skipped => "skipped",
        })
        .collect();
    assert!(
        build.contains(&"failure") && build.contains(&"success"),
        "expected one failed and one successful combination, got {:?}",
        build
    );
    assert_eq!(status_of(&result, "deploy"), "skipped");
    assert_eq!(status_of(&result, "cleanup"), "success");
    assert_eq!(status_of(&result, "report"), "success");
}

/// With fail-fast, combinations after the failure are skipped. The job still
/// counts as failed, not skipped.
#[tokio::test]
async fn fail_fast_matrix_need_counts_as_failed() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: true
      max-parallel: 1
      matrix:
        target: [bad, good]
    steps:
      - run: test "${{ matrix.target }}" = good
  cleanup:
    runs-on: ubuntu-latest
    needs: [build]
    if: failure()
    steps:
      - run: echo cleanup
"#)
    .await;

    assert_eq!(status_of(&result, "cleanup"), "success");
}

/// `needs.<id>.outputs` is populated when a job-level `if:` is evaluated.
#[tokio::test]
async fn needs_outputs_are_visible_to_job_if() {
    let result = run(r#"
name: CI
on: push
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      go: ${{ steps.decide.outputs.go }}
    steps:
      - id: decide
        run: echo "go=yes" >> "$GITHUB_OUTPUT"
  deploy:
    runs-on: ubuntu-latest
    needs: [plan]
    if: needs.plan.outputs.go == 'yes'
    steps:
      - run: echo deploy
  hold:
    runs-on: ubuntu-latest
    needs: [plan]
    if: needs.plan.outputs.go == 'no'
    steps:
      - run: echo "should not run"
"#)
    .await;

    assert_eq!(status_of(&result, "plan"), "success");
    assert_eq!(status_of(&result, "deploy"), "success");
    assert_eq!(status_of(&result, "hold"), "skipped");
}

/// A need skipped by the gate (not by its own `if:`) reports `skipped` to its
/// dependents' `needs.<id>.result`.
#[tokio::test]
async fn gate_skipped_need_reports_skipped_result() {
    let result = run(r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    if: false
    steps:
      - run: echo hi
  test:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo hi
  summary:
    runs-on: ubuntu-latest
    needs: [test]
    if: always() && needs.test.result == 'skipped'
    steps:
      - run: echo summary
"#)
    .await;

    assert_eq!(status_of(&result, "test"), "skipped");
    assert_eq!(status_of(&result, "summary"), "success");
}
