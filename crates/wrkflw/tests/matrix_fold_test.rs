// Matrix legs folded by the engine, end to end: which legs run, the order
// results come back in, and what a folded leg reports when the leg it
// repeats fails or is skipped. The steps run real `bash` through the
// emulation runtime, which Windows runners can't start (see
// dynamic_matrix_test.rs).
#![cfg(not(target_os = "windows"))]

use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
};

fn emulation_cfg() -> ExecutionConfig {
    ExecutionConfig {
        acceleration: Default::default(),
        event_name: None,
        git_ref: None,
        base_branch: None,
        runtime_type: RuntimeType::Emulation,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: None,
        platform_overrides: Default::default(),
        kubernetes: Default::default(),
        variables: Default::default(),
    }
}

async fn run(yaml: &str) -> ExecutionResult {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ci.yml");
    fs::write(&path, yaml).unwrap();
    execute_workflow(&path, emulation_cfg())
        .await
        .expect("the workflow runs")
}

/// Each leg's name, status and the leg it repeated, in result order.
fn legs(result: &ExecutionResult) -> Vec<(String, JobStatus, Option<String>)> {
    result
        .jobs
        .iter()
        .filter(|job| job.canonical_name == "test")
        .map(|job| {
            (
                job.name.clone(),
                job.status.clone(),
                job.repeated_from.clone(),
            )
        })
        .collect()
}

fn workflow(strategy: &str, steps: &str) -> String {
    format!(
        "name: CI\non: push\njobs:\n  test:\n    runs-on: ${{{{ matrix.os }}}}\n    strategy:\n{strategy}\n    steps:\n{steps}\n"
    )
}

const OS: &str = "      matrix:\n        os: [ubuntu-latest, macos-latest, windows-latest]";

#[tokio::test]
async fn legs_that_only_change_the_runner_run_once_in_matrix_order() {
    let result = run(&workflow(OS, "      - run: echo hi")).await;
    let legs = legs(&result);
    assert_eq!(legs.len(), 3, "{legs:#?}");
    assert!(legs[0].0.contains("ubuntu-latest"), "{legs:#?}");
    assert!(legs[1].0.contains("macos-latest"), "{legs:#?}");
    assert!(legs[2].0.contains("windows-latest"), "{legs:#?}");
    assert_eq!(legs[0].2, None);
    for leg in &legs[1..] {
        assert_eq!(leg.1, JobStatus::Success, "{legs:#?}");
        assert_eq!(leg.2.as_deref(), Some(legs[0].0.as_str()), "{legs:#?}");
    }
}

#[tokio::test]
async fn a_leg_repeating_a_failed_leg_fails_and_says_whose_failure_it_is() {
    let strategy = format!("      fail-fast: false\n{OS}");
    let result = run(&workflow(&strategy, "      - run: exit 1")).await;
    let legs = legs(&result);
    assert_eq!(legs.len(), 3, "{legs:#?}");
    assert!(
        legs.iter().all(|leg| leg.1 == JobStatus::Failure),
        "{legs:#?}"
    );
    assert_eq!(legs[0].2, None);
    assert_eq!(legs[2].2.as_deref(), Some(legs[0].0.as_str()));
}

#[tokio::test]
async fn a_leg_whose_original_fail_fast_skipped_is_skipped_too() {
    // Two originals, one per `node`, run one at a time; the first fails,
    // so fail-fast skips the second, and the leg repeating it.
    let strategy = "      fail-fast: true\n      max-parallel: 1\n      matrix:\n        node: ['1', '2']\n        os: [ubuntu-latest, windows-latest]";
    let steps = "      - run: test \"${{ matrix.node }}\" != 1";
    let result = run(&workflow(strategy, steps)).await;
    let legs = legs(&result);
    assert_eq!(legs.len(), 4, "{legs:#?}");
    let status = |node: &str, os: &str| {
        legs.iter()
            .find(|leg| leg.0.contains(&format!("node: {node}")) && leg.0.contains(os))
            .unwrap_or_else(|| panic!("no leg {node} {os} in {legs:#?}"))
    };
    assert_eq!(status("1", "ubuntu").1, JobStatus::Failure);
    assert_eq!(status("1", "windows").1, JobStatus::Failure);
    assert_eq!(status("2", "ubuntu").1, JobStatus::Skipped);
    let twin = status("2", "windows");
    assert_eq!(twin.1, JobStatus::Skipped, "{legs:#?}");
    assert_eq!(twin.2.as_deref(), Some(status("2", "ubuntu").0.as_str()));
}

#[tokio::test]
async fn a_condition_reading_the_runner_over_several_lines_runs_every_leg() {
    let steps = "      - if: |\n          github.event_name == 'push' &&\n          matrix.os == 'windows-latest'\n        run: echo windows only";
    let result = run(&workflow(OS, steps)).await;
    let legs = legs(&result);
    assert_eq!(legs.len(), 3, "{legs:#?}");
    assert!(legs.iter().all(|leg| leg.2.is_none()), "{legs:#?}");
}
