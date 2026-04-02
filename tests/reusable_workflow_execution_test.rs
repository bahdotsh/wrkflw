use std::fs;
use tempfile::tempdir;
use wrkflw::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

#[tokio::test]
async fn test_local_reusable_workflow_execution_success() {
    // Create temp workspace
    let dir = tempdir().unwrap();
    let called_path = dir.path().join("called.yml");
    let caller_path = dir.path().join("caller.yml");

    // Minimal called workflow with one successful job
    let called = r#"
name: Called
on: workflow_dispatch
jobs:
  inner:
    runs-on: ubuntu-latest
    steps:
      - run: echo "hello from called"
"#;
    write_file(&called_path, called);

    // Caller workflow that uses the called workflow via absolute local path
    let caller = format!(
        r#"
name: Caller
on: workflow_dispatch
jobs:
  call:
    uses: {}
    with:
      foo: bar
    secrets:
      token: testsecret
"#,
        called_path.display()
    );
    write_file(&caller_path, &caller);

    // Execute caller workflow with emulation runtime
    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        target_job: None,
    };

    let result = execute_workflow(&caller_path, cfg)
        .await
        .expect("workflow execution failed");

    // Expect a single caller job summarized
    assert_eq!(result.jobs.len(), 1, "expected one caller job result");
    let job = &result.jobs[0];
    assert_eq!(job.name, "call");
    assert_eq!(format!("{:?}", job.status), "Success");

    // Summary step should include reference to called workflow and inner job status
    assert!(job
        .logs
        .contains("Called workflow:"),
        "expected summary logs to include called workflow path");
    assert!(job.logs.contains("- inner: Success"), "expected inner job success in summary");
}

#[tokio::test]
async fn test_local_reusable_workflow_execution_failure_propagates() {
    // Create temp workspace
    let dir = tempdir().unwrap();
    let called_path = dir.path().join("called.yml");
    let caller_path = dir.path().join("caller.yml");

    // Called workflow with failing job
    let called = r#"
name: Called
on: workflow_dispatch
jobs:
  inner:
    runs-on: ubuntu-latest
    steps:
      - run: false
"#;
    write_file(&called_path, called);

    // Caller workflow
    let caller = format!(
        r#"
name: Caller
on: workflow_dispatch
jobs:
  call:
    uses: {}
"#,
        called_path.display()
    );
    write_file(&caller_path, &caller);

    // Execute caller workflow
    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        target_job: None,
    };

    let result = execute_workflow(&caller_path, cfg)
        .await
        .expect("workflow execution failed");

    assert_eq!(result.jobs.len(), 1);
    let job = &result.jobs[0];
    assert_eq!(job.name, "call");
    assert_eq!(format!("{:?}", job.status), "Failure");
    assert!(job.logs.contains("- inner: Failure"));
}


