// These tests run real `bash` steps through the emulation runtime. On
// windows-latest `bash` resolves to the WSL launcher, which has no distribution
// installed, so every step fails. Engine unit tests use the same gate.
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
};

fn emulation_cfg() -> ExecutionConfig {
    ExecutionConfig {
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        platform_overrides: Default::default(),
        kubernetes: Default::default(),
    }
}

async fn run(yaml: &str) -> Result<ExecutionResult, String> {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    fs::write(&path, yaml).unwrap();
    execute_workflow(&path, emulation_cfg())
        .await
        .map_err(|e| e.to_string())
}

fn job_outcomes(result: &ExecutionResult, job: &str) -> Vec<(String, bool)> {
    let mut outcomes: Vec<_> = result
        .jobs
        .iter()
        .filter(|j| j.canonical_name == job)
        .map(|j| (j.name.clone(), matches!(j.status, JobStatus::Success)))
        .collect();
    outcomes.sort();
    outcomes
}

fn workflow(matrix: &str, output: &str) -> String {
    format!(
        r#"
name: CI
on: push
jobs:
  setup:
    runs-on: ubuntu-latest
    outputs:
      list: ${{{{ steps.gen.outputs.list }}}}
    steps:
      - id: gen
        run: echo 'list={output}' >> "$GITHUB_OUTPUT"
  build:
    needs: setup
    runs-on: ubuntu-latest
    strategy:
      matrix:
{matrix}
    steps:
      - run: echo "os=${{{{ matrix.os }}}} extra=${{{{ matrix.extra }}}}"
"#
    )
}

/// The shape from issue #112: axes plus an `include` computed by another job.
#[tokio::test]
async fn include_from_needs_output_merges_into_axes() {
    let yaml = workflow(
        r#"        os: [ubuntu, windows]
        node: [14, 16]
        include: ${{ fromJSON(needs.setup.outputs.list) }}"#,
        r#"[{"os":"ubuntu","node":14,"extra":"a"},{"os":"macos","node":18,"extra":"b"}]"#,
    );

    let result = run(&yaml).await.unwrap();

    // 4 base combinations, one merged into, plus the standalone macos entry
    let outcomes = job_outcomes(&result, "build");
    assert_eq!(outcomes.len(), 5, "{:?}", outcomes);
    assert!(outcomes.iter().all(|(_, ok)| *ok), "{:?}", outcomes);
    assert_eq!(
        outcomes.iter().filter(|(n, _)| n.contains("extra")).count(),
        2,
        "{:?}",
        outcomes
    );
}

#[tokio::test]
async fn include_only_matrix_comes_from_needs_output() {
    let yaml = workflow(
        "        include: ${{ fromJSON(needs.setup.outputs.list) }}",
        r#"[{"os":"ubuntu"},{"os":"windows"}]"#,
    );

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_outcomes(&result, "build").len(), 2);
}

#[tokio::test]
async fn exclude_from_needs_output_filters_axes() {
    let yaml = workflow(
        r#"        os: [ubuntu, windows]
        exclude: ${{ fromJSON(needs.setup.outputs.list) }}"#,
        r#"[{"os":"windows"}]"#,
    );

    let result = run(&yaml).await.unwrap();

    let outcomes = job_outcomes(&result, "build");
    assert_eq!(outcomes.len(), 1, "{:?}", outcomes);
    assert!(outcomes[0].0.contains("ubuntu"), "{:?}", outcomes);
}

fn job_status(result: &ExecutionResult, job: &str) -> JobStatus {
    result
        .jobs
        .iter()
        .find(|j| j.canonical_name == job)
        .unwrap_or_else(|| panic!("no result for job {}", job))
        .status
        .clone()
}

fn failure_details(result: &ExecutionResult) -> &str {
    result.failure_details.as_deref().expect("run should fail")
}

#[tokio::test]
async fn include_that_is_not_a_list_fails_the_job() {
    let yaml = workflow(
        r#"        os: [ubuntu]
        include: ${{ fromJSON(needs.setup.outputs.list) }}"#,
        "not-a-list",
    );

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_status(&result, "build"), JobStatus::Failure);
    assert!(
        failure_details(&result).contains("array of objects"),
        "{:?}",
        result.failure_details
    );
}

#[tokio::test]
async fn exclude_that_is_not_a_list_fails_the_job() {
    let yaml = workflow(
        r#"        os: [ubuntu]
        exclude: ${{ fromJSON(needs.setup.outputs.list) }}"#,
        "5",
    );

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_status(&result, "build"), JobStatus::Failure);
    assert!(
        failure_details(&result).contains("exclude"),
        "{:?}",
        result.failure_details
    );
}

#[tokio::test]
async fn context_unavailable_in_strategy_fails_the_job() {
    let yaml = workflow(
        r#"        os: [ubuntu]
        include: ${{ fromJSON(env.LIST) }}"#,
        "[]",
    )
    .replace(
        "name: CI\n",
        "name: CI\nenv:\n  LIST: '[{\"os\":\"macos\"}]'\n",
    );

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_status(&result, "build"), JobStatus::Failure);
    assert!(
        failure_details(&result).contains("uses env"),
        "{:?}",
        result.failure_details
    );
}

#[tokio::test]
async fn github_context_is_available_to_strategy() {
    let yaml = workflow(
        r#"        os: [ubuntu]
        exclude: ${{ fromJSON(format('[{0}"os":"{1}"{2}]', '{', github.event_name, '}')) }}"#,
        "[]",
    );

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_outcomes(&result, "build").len(), 1);
    assert_eq!(job_status(&result, "build"), JobStatus::Success);
}

/// A generated matrix with nothing to do must not take the rest of the run down.
#[tokio::test]
async fn empty_include_only_matrix_fails_that_job_and_the_run_carries_on() {
    let yaml = workflow(
        "        include: ${{ fromJSON(needs.setup.outputs.list) }}",
        "[]",
    ) + r#"  sibling:
    needs: setup
    runs-on: ubuntu-latest
    steps:
      - run: echo sibling
  after-build:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - run: echo after
  after-sibling:
    needs: sibling
    runs-on: ubuntu-latest
    steps:
      - run: echo after
"#;

    let result = run(&yaml).await.unwrap();

    assert_eq!(job_status(&result, "build"), JobStatus::Failure);
    assert_eq!(job_status(&result, "sibling"), JobStatus::Success);
    assert_eq!(job_status(&result, "after-sibling"), JobStatus::Success);
    assert_eq!(job_status(&result, "after-build"), JobStatus::Skipped);
    assert!(
        failure_details(&result).contains("no valid parameters"),
        "{:?}",
        result.failure_details
    );
}
