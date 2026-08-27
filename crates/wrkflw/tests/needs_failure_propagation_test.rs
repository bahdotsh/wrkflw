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
