use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

#[tokio::test]
async fn test_target_job_runs_only_specified_job() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    let workflow = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "building"
  test:
    runs-on: ubuntu-latest
    needs: [build]
    steps:
      - run: echo "testing"
  deploy:
    runs-on: ubuntu-latest
    needs: [test]
    steps:
      - run: echo "deploying"
"#;
    write_file(&workflow_path, workflow);

    // Run only the "test" job — should include "build" (dependency) and "test",
    // but NOT "deploy".
    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: Some("test".to_string()),
    };

    let result = execute_workflow(&workflow_path, cfg)
        .await
        .expect("workflow execution failed");

    let job_names: Vec<&str> = result.jobs.iter().map(|j| j.name.as_str()).collect();
    assert!(
        job_names.contains(&"build"),
        "expected build as a dependency"
    );
    assert!(job_names.contains(&"test"), "expected target job test");
    assert!(
        !job_names.contains(&"deploy"),
        "deploy should not run when targeting test"
    );
}

#[tokio::test]
async fn test_target_job_not_found_returns_error() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    let workflow = r#"
name: CI
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: echo "building"
"#;
    write_file(&workflow_path, workflow);

    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: Some("nonexistent".to_string()),
    };

    let result = execute_workflow(&workflow_path, cfg).await;
    match result {
        Err(err) => {
            let msg = format!("{}", err);
            assert!(
                msg.contains("nonexistent"),
                "error should mention the job name"
            );
        }
        Ok(_) => panic!("expected error for nonexistent job"),
    }
}

#[tokio::test]
async fn test_target_job_with_no_deps_runs_alone() {
    let dir = tempdir().unwrap();
    let workflow_path = dir.path().join("ci.yml");

    let workflow = r#"
name: CI
on: push
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: echo "linting"
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo "testing"
  deploy:
    runs-on: ubuntu-latest
    needs: [lint, test]
    steps:
      - run: echo "deploying"
"#;
    write_file(&workflow_path, workflow);

    // Target "lint" which has no dependencies — only lint should run
    let cfg = ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: Some("lint".to_string()),
    };

    let result = execute_workflow(&workflow_path, cfg)
        .await
        .expect("workflow execution failed");

    assert_eq!(result.jobs.len(), 1, "only the target job should run");
    assert_eq!(result.jobs[0].name, "lint");
}
