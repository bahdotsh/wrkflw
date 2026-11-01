// Emulation runs scripts through a POSIX shell.
#![cfg(not(target_os = "windows"))]

use std::collections::HashMap;
use std::fs;
use tempfile::tempdir;
use wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
};

async fn run(yaml: &str, target_job: Option<&str>, variables: &[(&str, &str)]) -> ExecutionResult {
    run_in(RuntimeType::Emulation, yaml, target_job, variables).await
}

async fn run_in(
    runtime_type: RuntimeType,
    yaml: &str,
    target_job: Option<&str>,
    variables: &[(&str, &str)],
) -> ExecutionResult {
    let dir = tempdir().unwrap();
    let path = dir.path().join(".gitlab-ci.yml");
    fs::write(&path, yaml).unwrap();
    let cfg = ExecutionConfig {
        runtime_type,
        verbose: false,
        preserve_containers_on_failure: false,
        secrets_config: None,
        show_action_messages: false,
        target_job: target_job.map(str::to_string),
        platform_overrides: Default::default(),
        kubernetes: Default::default(),
        variables: variables
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>(),
    };
    execute_workflow(&path, cfg).await.unwrap()
}

fn status(result: &ExecutionResult, name: &str) -> JobStatus {
    let mut matching = result.jobs.iter().filter(|job| job.name == name);
    let job = matching
        .next()
        .unwrap_or_else(|| panic!("no job named {}", name));
    assert!(matching.next().is_none(), "{} ran twice", name);
    job.status.clone()
}

const FAILING_PIPELINE: &str = r#"
stages: [first, second, third]
ok:
  stage: first
  script: ["true"]
breaks:
  stage: first
  script: ["exit 1"]
next:
  stage: second
  script: ["true"]
tolerated:
  stage: second
  when: on_failure
  script: ["exit 1"]
  allow_failure: true
cleanup:
  stage: third
  when: always
  script: ["true"]
report:
  stage: third
  when: on_failure
  script: ["true"]
"#;

#[tokio::test]
async fn a_failed_stage_selects_the_later_jobs() {
    let result = run(FAILING_PIPELINE, None, &[]).await;
    assert!(result.failure_details.is_some());
    assert_eq!(status(&result, "ok"), JobStatus::Success);
    assert_eq!(status(&result, "breaks"), JobStatus::Failure);
    assert_eq!(status(&result, "next"), JobStatus::Skipped);
    assert_eq!(status(&result, "tolerated"), JobStatus::Failure);
    assert_eq!(status(&result, "cleanup"), JobStatus::Success);
    assert_eq!(status(&result, "report"), JobStatus::Success);
}

#[tokio::test]
async fn an_allowed_failure_does_not_trigger_on_failure_jobs() {
    let result = run(
        r#"
stages: [first, second]
flaky:
  stage: first
  script: ["exit 1"]
  allow_failure: true
after:
  stage: second
  script: ["true"]
report:
  stage: second
  when: on_failure
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert!(result.failure_details.is_none());
    assert_eq!(status(&result, "after"), JobStatus::Success);
    assert_eq!(status(&result, "report"), JobStatus::Skipped);
}

const GATED_PIPELINE: &str = r#"
stages: [deploy]
only_main:
  stage: deploy
  only: [main]
  script: ["true"]
tag_release:
  stage: deploy
  rules:
    - if: $CI_COMMIT_TAG
  script: ["true"]
manual_deploy:
  stage: deploy
  when: manual
  script: ["true"]
"#;

#[tokio::test]
async fn variables_decide_which_gated_jobs_run() {
    let on_dev = run(GATED_PIPELINE, None, &[("CI_COMMIT_BRANCH", "dev")]).await;
    assert_eq!(status(&on_dev, "only_main"), JobStatus::Skipped);
    assert_eq!(status(&on_dev, "tag_release"), JobStatus::Skipped);
    assert_eq!(status(&on_dev, "manual_deploy"), JobStatus::Skipped);

    let on_main = run(
        GATED_PIPELINE,
        None,
        &[("CI_COMMIT_BRANCH", "main"), ("CI_COMMIT_TAG", "")],
    )
    .await;
    assert_eq!(status(&on_main, "only_main"), JobStatus::Success);

    let tagged = run(GATED_PIPELINE, None, &[("CI_COMMIT_TAG", "v1")]).await;
    assert_eq!(status(&tagged, "tag_release"), JobStatus::Success);
}

#[tokio::test]
async fn naming_a_job_runs_it_despite_its_gate() {
    for job in ["only_main", "tag_release", "manual_deploy"] {
        let result = run(GATED_PIPELINE, Some(job), &[("CI_COMMIT_BRANCH", "dev")]).await;
        assert_eq!(status(&result, job), JobStatus::Success, "{}", job);
    }
}

const WORKFLOW_PIPELINE: &str = r#"
workflow:
  rules:
    - if: $CI_PIPELINE_SOURCE == "web"
job:
  script: ["true"]
"#;

#[tokio::test]
async fn workflow_rules_gate_the_whole_pipeline() {
    let excluded = run(WORKFLOW_PIPELINE, None, &[]).await;
    assert_eq!(status(&excluded, "job"), JobStatus::Skipped);
    assert!(excluded.failure_details.is_none());

    let created = run(WORKFLOW_PIPELINE, None, &[("CI_PIPELINE_SOURCE", "web")]).await;
    assert_eq!(status(&created, "job"), JobStatus::Success);

    let named = run(WORKFLOW_PIPELINE, Some("job"), &[]).await;
    assert_eq!(status(&named, "job"), JobStatus::Success);
}

#[tokio::test]
async fn variables_reach_scripts() {
    let result = run(
        "variables:\n  V: yaml\njob:\n  script: ['test \"$V\" = cli']\n",
        None,
        &[("V", "cli")],
    )
    .await;
    assert_eq!(status(&result, "job"), JobStatus::Success);
}

const APPROVAL_PIPELINE: &str = r#"
stages: [gate, deploy]
approve:
  stage: gate
  rules:
    - when: manual
  script: ["true"]
deploy:
  stage: deploy
  script: ["true"]
cleanup:
  stage: deploy
  when: always
  script: ["true"]
"#;

#[tokio::test]
async fn a_blocking_manual_job_stops_later_stages() {
    let result = run(APPROVAL_PIPELINE, None, &[]).await;
    assert_eq!(status(&result, "approve"), JobStatus::Skipped);
    assert_eq!(status(&result, "deploy"), JobStatus::Skipped);
    assert_eq!(status(&result, "cleanup"), JobStatus::Skipped);
    assert!(result.failure_details.is_none());

    let named = run(APPROVAL_PIPELINE, Some("deploy"), &[]).await;
    assert_eq!(status(&named, "deploy"), JobStatus::Success);
}

#[tokio::test]
async fn an_optional_manual_job_does_not_stop_later_stages() {
    let result = run(
        r#"
stages: [gate, deploy]
optional:
  stage: gate
  when: manual
  script: ["true"]
deploy:
  stage: deploy
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert_eq!(status(&result, "optional"), JobStatus::Skipped);
    assert_eq!(status(&result, "deploy"), JobStatus::Success);
}

#[tokio::test]
async fn a_rule_can_allow_a_failure() {
    let result = run(
        r#"
stages: [first, second]
flaky:
  stage: first
  rules:
    - allow_failure: true
  script: ["exit 1"]
after:
  stage: second
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert!(result.failure_details.is_none());
    assert_eq!(status(&result, "after"), JobStatus::Success);
}

#[tokio::test]
async fn a_tag_pipeline_has_no_branch_for_rules() {
    let result = run(
        r#"
on_main:
  rules:
    - if: $CI_COMMIT_BRANCH == "main"
  script: ["true"]
tagged:
  rules:
    - if: $CI_COMMIT_REF_NAME == "v1"
  script: ["true"]
"#,
        None,
        &[("CI_COMMIT_TAG", "v1")],
    )
    .await;
    assert_eq!(status(&result, "on_main"), JobStatus::Skipped);
    assert_eq!(status(&result, "tagged"), JobStatus::Success);
}

#[tokio::test]
async fn a_named_job_still_follows_the_stages_before_it() {
    let pipeline = r#"
stages: [first, second]
breaks:
  stage: first
  script: ["exit 1"]
gated:
  stage: first
  when: manual
  script: ["exit 1"]
named:
  stage: second
  when: manual
  script: ["true"]
"#;
    let result = run(pipeline, Some("named"), &[]).await;
    assert_eq!(status(&result, "breaks"), JobStatus::Failure);
    assert_eq!(status(&result, "gated"), JobStatus::Skipped);
    assert_eq!(status(&result, "named"), JobStatus::Skipped);
}

#[tokio::test]
async fn a_job_with_an_image_runs_through_the_shell_selector() {
    let result = run(
        r#"
variables:
  VERSION: "3.20"
job:
  image: alpine:$VERSION
  before_script: ['test "$VERSION" = 3.20']
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert_eq!(status(&result, "job"), JobStatus::Success);
}

#[tokio::test]
async fn a_job_with_an_image_runs_under_secure_emulation() {
    let result = run_in(
        RuntimeType::SecureEmulation,
        "job:\n  image: alpine:3.20\n  script: [\"true\"]\n",
        None,
        &[],
    )
    .await;
    assert_eq!(status(&result, "job"), JobStatus::Success);
}

#[tokio::test]
async fn a_manual_job_skipped_by_a_failure_does_not_block() {
    let result = run(
        r#"
stages: [test, deploy, cleanup]
unit:
  stage: test
  script: ["exit 1"]
deploy:
  stage: deploy
  rules:
    - when: manual
  script: ["true"]
teardown:
  stage: cleanup
  when: always
  script: ["true"]
notify:
  stage: cleanup
  when: on_failure
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert_eq!(status(&result, "deploy"), JobStatus::Skipped);
    assert_eq!(status(&result, "teardown"), JobStatus::Success);
    assert_eq!(status(&result, "notify"), JobStatus::Success);
}

#[tokio::test]
async fn a_rule_that_forbids_failure_stops_later_stages() {
    let result = run(
        r#"
stages: [first, second]
strict:
  stage: first
  allow_failure: true
  rules:
    - allow_failure: false
  script: ["exit 1"]
after:
  stage: second
  script: ["true"]
"#,
        None,
        &[],
    )
    .await;
    assert!(result.failure_details.is_some());
    assert_eq!(status(&result, "after"), JobStatus::Skipped);
}

#[tokio::test]
async fn naming_a_matrix_job_runs_past_its_matrix_condition() {
    let pipeline = r#"
job:
  parallel:
    matrix:
      - CLOUD: [aws]
  rules:
    - if: $CLOUD == "aws"
  script: ["true"]
"#;
    let excluded = run(pipeline, None, &[]).await;
    assert_eq!(status(&excluded, "job"), JobStatus::Skipped);

    let named = run(pipeline, Some("job"), &[]).await;
    assert!(named.failure_details.is_none());
    assert!(named
        .jobs
        .iter()
        .all(|job| job.status == JobStatus::Success));
}
