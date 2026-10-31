// Emulation runs `run:` steps through a POSIX shell.
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType};

fn write_file(path: &std::path::Path, content: &str) {
    fs::write(path, content).expect("failed to write file");
}

#[tokio::test]
async fn test_reusable_workflow_inputs_and_declared_outputs() {
    let dir = tempdir().unwrap();
    let called_path = dir.path().join("called.yml");
    let caller_path = dir.path().join("caller.yml");

    let called = r#"
name: Called
on:
  workflow_call:
    inputs:
      name:
        type: string
        required: true
      with-default:
        type: string
        default: dflt
      number:
        type: number
        default: 7
    outputs:
      result:
        description: mapped from a job output
        value: ${{ jobs.inner.outputs.r }}
jobs:
  inner:
    runs-on: ubuntu-latest
    outputs:
      r: ${{ steps.s.outputs.r }}
    steps:
      - id: s
        run: echo "r=${{ inputs.name }}:${{ inputs.with-default }}:${{ inputs.number }}" >> "$GITHUB_OUTPUT"
"#;
    write_file(&called_path, called);

    let caller = r#"
name: Caller
on: workflow_dispatch
jobs:
  pre:
    runs-on: ubuntu-latest
    outputs:
      v: ${{ steps.s.outputs.v }}
    steps:
      - id: s
        run: echo "v=dynamic" >> "$GITHUB_OUTPUT"
  call:
    needs: pre
    uses: ./called.yml
    with:
      name: ${{ needs.pre.outputs.v }}
  after:
    needs: call
    runs-on: ubuntu-latest
    steps:
      - env:
          O: ${{ needs.call.outputs.result }}
        run: test "$O" = "dynamic:dflt:7"
"#;
    write_file(&caller_path, caller);

    // Local reusable workflows must live under the current directory.
    std::env::set_current_dir(dir.path()).unwrap();

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

    let result = execute_workflow(&caller_path, cfg)
        .await
        .expect("workflow execution failed");

    for job in &result.jobs {
        assert_eq!(format!("{:?}", job.status), "Success", "{}", job.logs);
    }
    assert_eq!(result.jobs.len(), 3);
}
