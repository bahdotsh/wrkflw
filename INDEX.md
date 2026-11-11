# Codebase Index: wrkflw

> Generated: 2026-11-11 11:07:47 UTC | Files: 260 | Lines: 116241
> Languages: JSON (4), Markdown (26), Rust (167), Shell (6), TOML (19), YAML (38)

## Directory Structure

```
wrkflw/
  AGENTS.md
  BREAKING_CHANGES.md
  CLAUDE.md
  Cargo.toml
  INDEX.md
  README.md
  cliff.toml
  crates/
    README.md
    evaluator/
      Cargo.toml
      README.md
      src/
        lib.rs
    executor/
      Cargo.toml
      README.md
      src/
        acceleration/
          concurrency.rs
          sidecar.rs
          store.rs
          tree.rs
        acceleration.rs
        action_fidelity.rs
        action_resolver.rs
        artifacts.rs
        cache.rs
        checkout_sync.rs
        container_guard.rs
        dependency.rs
        docker.rs
        docker_test.rs
        environment.rs
        events.rs
        expression.rs
        github_env_files.rs
        job_memory.rs
        job_session.rs
        kubernetes/
          manifest.rs
          mod.rs
          sync.rs
        lib.rs
        live_step.rs
        log_redaction.rs
        matrix_fold.rs
        podman.rs
        run_options.rs
        runner_fidelity.rs
        setup_actions.rs
        substitution.rs
        vm_memory.rs
        vm_workspace.rs
        workflow_commands.rs
        workspace.rs
    github/
      Cargo.toml
      README.md
      src/
        lib.rs
    gitlab/
      Cargo.toml
      README.md
      src/
        lib.rs
    logging/
      Cargo.toml
      README.md
      src/
        lib.rs
        symbols.rs
    matrix/
      Cargo.toml
      README.md
      src/
        lib.rs
    models/
      Cargo.toml
      README.md
      src/
        lib.rs
    parser/
      Cargo.toml
      README.md
      src/
        github-workflow.json
        gitlab-ci.json
        gitlab.rs
        gitlab_config.rs
        lib.rs
        schema.rs
        workflow.rs
    runs/
      Cargo.toml
      README.md
      src/
        digest.rs
        env.rs
        lib.rs
        record.rs
        recorder.rs
        store.rs
    runtime/
      Cargo.toml
      README.md
      src/
        container.rs
        emulation.rs
        emulation_test.rs
        lib.rs
        live_output.rs
        sandbox.rs
        secure_emulation.rs
    secrets/
      Cargo.toml
      README.md
      benches/
        masking_bench.rs
      src/
        config.rs
        error.rs
        lib.rs
        manager.rs
        masking.rs
        providers/
          env.rs
          file.rs
          mod.rs
        rate_limit.rs
        storage.rs
        substitution.rs
        validation.rs
      tests/
        integration_tests.rs
    trigger-filter/
      Cargo.toml
      README.md
      src/
        config.rs
        error.rs
        eval.rs
        git.rs
        lib.rs
        model.rs
        parser.rs
        path_matcher.rs
        ref_matcher.rs
    ui/
      Cargo.toml
      README.md
      src/
        app/
          mod.rs
          state.rs
        cli_style.rs
        components/
          button.rs
          checkbox.rs
          dag.rs
          mod.rs
          progress_bar.rs
          progress_dots.rs
        handlers/
          mod.rs
          workflow.rs
        lib.rs
        log_processor.rs
        models/
          mod.rs
        selection.rs
        stress_tests.rs
        text.rs
        theme.rs
        utils/
          mod.rs
        views/
          dag_tab.rs
          execution_tab.rs
          help_overlay.rs
          job_detail.rs
          logs_tab.rs
          mod.rs
          run_header.rs
          runs_tab.rs
          secrets_tab.rs
          status_bar.rs
          title_bar.rs
          trigger_tab.rs
          tweaks_overlay.rs
          workflows_tab.rs
    utils/
      Cargo.toml
      README.md
      src/
        lib.rs
    validators/
      Cargo.toml
      README.md
      src/
        actions.rs
        expressions.rs
        gitlab.rs
        jobs.rs
        lib.rs
        matrix.rs
        steps.rs
        triggers.rs
    watcher/
      Cargo.toml
      README.md
      src/
        debouncer.rs
        error.rs
        event_kind.rs
        git_state.rs
        ignore.rs
        lib.rs
        paths.rs
        reactor.rs
        setup.rs
        shutdown.rs
        trigger_cache.rs
        watcher.rs
    wrkflw/
      Cargo.toml
      README.md
      src/
        cli_secrets.rs
        compare_cmd.rs
        gate_cmd.rs
        git.rs
        json_output.rs
        lib.rs
        main.rs
        mcp_cmd.rs
        mcp_install.rs
        plan_cmd.rs
        prefilter.rs
        run_workflow_cmd.rs
        runs_cmd.rs
        watch_cmd.rs
      tests/
        acceleration_test.rs
        agent_cli_test.rs
        container_fidelity_test.rs
        continue_on_error_test.rs
        dynamic_matrix_test.rs
        gitlab_pipeline_test.rs
        matrix_fold_test.rs
        needs_failure_propagation_test.rs
        reusable_inputs_outputs_test.rs
        startup_test.rs
        target_job_test.rs
  examples/
    secrets-demo/
      README.md
      secrets-workflow.yml
    ui-demo/
      01-dag-diamond.yml
      02-dag-wide-fan.yml
      03-dag-linear.yml
      04-trigger-dispatch.yml
      05-matrix-inspector.yml
      06-secrets-runtime.yml
      07-multi-event.yml
      08-failing.yml
  publish_crates.sh
  schemas/
    github-workflow.json
    gitlab-ci.json
  scripts/
    bump-crate.sh
    conformance/
      plan_corpus.sh
  skills/
    wrkflw/
      SKILL.md
  tests/
    README.md
    cleanup_test.rs
    fixtures/
      gitlab-ci/
        advanced.gitlab-ci.yml
        basic.gitlab-ci.yml
        docker.gitlab-ci.yml
        includes.gitlab-ci.yml
        invalid.gitlab-ci.yml
        minimal.gitlab-ci.yml
        services.gitlab-ci.yml
        workflow.gitlab-ci.yml
    matrix_test.rs
    reusable_workflow_execution_test.rs
    reusable_workflow_test.rs
    safe_workflow.yml
    scripts/
      test-podman-basic.sh
      test-preserve-containers.sh
    security_comparison.yml
    security_demo.yml
    workflows/
      1-basic-workflow.yml
      2-reusable-workflow-caller.yml
      3-reusable-workflow-definition.yml
      4-mixed-jobs.yml
      5-no-name-reusable-caller.yml
      6-invalid-reusable-format.yml
      7-invalid-regular-job.yml
      8-cyclic-dependencies.yml
      cpp-test.yml
      example.yml
      matrix-example.yml
      multi-runtime-test.yml
      node-test.yml
      python-test.yml
      runs-on-array-test.yml
      rust-test.yml
      test.yml
      trigger_gitlab.sh
      working-secrets-test.yml
```

---

## Public API Surface

**AGENTS.md**
- `# wrkflw for coding agents`

**BREAKING_CHANGES.md**
- `# Breaking Changes`

**CLAUDE.md**
- `# wrkflw`

**Cargo.toml**
- `[workspace]`
- `[workspace.package]`
- `[workspace.dependencies]`
- `[profile.release]`

**INDEX.md**
- `# Codebase Index: wrkflw`

**README.md**
- `# WRKFLW`
- `# Launch the TUI (auto-detects .gitea/workflows and .github/workflows)`
- `# Validate workflows`
- `# Run a workflow`
- `# Rerun workflows automatically on file changes`
- `# List detected workflows and pipelines`
- `# Validate all workflows in .gitea/workflows and .github/workflows`
- `# Validate specific files or directories`
- `# Validate multiple paths`
- `# GitLab pipelines`
- `# Verbose output`
- `# Run with auto-detection (default: tries Docker, then Podman, then emulation)`
- `# Run with Docker explicitly`
- `# Run with Podman`
- `# Run each step as a pod in a Kubernetes cluster`
- `# Run in emulation mode (no containers)`
- `# Run in sandboxed secure emulation`
- `# Run a specific job`
- `# Set GitLab pipeline variables, which rules and scripts both see`
- `# List workflows and their jobs`
- `# Preserve failed containers for debugging`
- `# Repeat the flag or separate entries with commas`
- `# Set it once for every command, including the TUI and watch mode`
- `# Auto-detect changed files from git (vs origin/HEAD, main/master, or HEAD~1)`
- `# Pin the diff range`
- `# Supply changed files explicitly (e.g. from a CI wrapper)`
- `# Simulate a pull_request — `--base-branch` is required under strict mode`
- `# Opt out of strict rejection (legacy warn-and-proceed)`
- `# Check everything CI would run for this push, reusing what already passed.`
- `# On a feature branch, a workflow a push doesn't fire is checked as a pull`
- `# request into the default branch. The result says how much the pass proves:`
- `# verified, partial (some jobs ran as another OS, or passed a step whose`
- `# action wrkflw can't run) or nothing_checked.`
- `# Run it on every push (an existing hook is left alone). The hook keeps the`
- `# --runtime and --remote-cache given here, and compares against what the`
- `# remote already has for the pushed branch. It refuses a core.hooksPath`
- `# outside the repository, which other repositories share. It checks the`
- `# working tree, so uncommitted changes count, and it warns when there are any.`
- `# The same pieces, one at a time`
- `# In trusted CI on your main branch: run every job and publish its result`
- `# Anywhere else: read`
- `# Watch .github/workflows for changes and rerun affected workflows`
- `# Watch a specific path, simulate pull_request, and cap concurrency`
- `# Ignore extra directories on top of the built-in list`
- `# Open TUI with default directory`
- `# Open with specific runtime`
- `# Force a color theme`
- `# GitHub (requires GITHUB_TOKEN env var)`
- `# GitLab (requires GITLAB_TOKEN env var)`
- `# Machine-readable output: stdout is JSON only, logs go to stderr`
- `# See what would run, and why a trigger fires, without running anything`
- `# Every run is recorded`
- `# Resume a job at a step`
- `# Run a workflow as a finished GitHub run did, and list where the results differ.`
- `# Check out the commit that run tested first; a detached HEAD is fine, the`
- `# run's branch is used.`
- `# Check a push to another branch than the one checked out, or on a detached HEAD`
- `# Check a tag push`
- `# Serve all of this over the Model Context Protocol`
- `# Pick the cluster and namespace, then run as usual`
- `# Land on GPU nodes and request an accelerator for every step`
- `# Private registry, and a registry wrkflw can push locally built images to`
- `# Every flag has a WRKFLW_K8S_* environment variable, so set them once`
- `# Environment variables (simplest)`
- `# File-based secrets (JSON, YAML, or .env format)`
- `# On Linux, put them in ~/.config/wrkflw/secrets/ (read by the `file` provider)`

**cliff.toml**
- `[changelog]`
- `[#{{ contributor.pr_number }}]`
- `[git]`
- `[git.link]`

**crates/README.md**
- `# Wrkflw Crates`
- `# Build everything`
- `# Build a specific crate`
- `# Run all tests`
- `# Run tests for a specific crate`

**crates/evaluator/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/evaluator/README.md**
- `## wrkflw-evaluator`

**crates/evaluator/src/lib.rs**
- `pub fn evaluate_workflow_file(path: &Path, verbose: bool) -> Result<ValidationResult, String>`

**crates/executor/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/executor/README.md**
- `## wrkflw-executor`

**crates/executor/src/acceleration/sidecar.rs**
- `pub struct JobHints`
- `pub struct Sidecar`

**crates/executor/src/acceleration.rs**
- `pub const SIDECAR_PATH: &str = ".wrkflw/affected.yml"`
- `pub struct Secret(pub String)`
- `pub struct Acceleration`
- `pub fn validate_remote(accel: &Acceleration) -> Result<(), String>`

**crates/executor/src/action_fidelity.rs**
- `pub struct UnexecutedAction`

**crates/executor/src/action_resolver.rs**
- `pub enum ActionType`
- `pub struct ResolvedAction`
- `pub async fn resolve_remote_action( repo: &str, version: &str, sub_path: Option<&str>, ) -> Result<ResolvedAction, String>`

**crates/executor/src/artifacts.rs**
- `pub struct UploadOptions`
- `pub struct ArtifactStore`

**crates/executor/src/cache.rs**
- `pub struct CacheStore`

**crates/executor/src/dependency.rs**
- `pub fn resolve_dependencies(workflow: &WorkflowDefinition) -> Result<Vec<Vec<String>>, String>`
- `pub fn collect_transitive_deps(target_job: &str, jobs: &HashMap<String, Job>) -> HashSet<String>`
- `pub fn filter_plan_to_job( plan: Vec<Vec<String>>, target_job: &str, jobs: &HashMap<String, Job>, kind: &str, ) -> Result<Vec<Vec<String>>, String>`
- `pub fn filter_plan_to_job_by_stage( plan: Vec<Vec<String>>, target_job: &str, jobs: &HashMap<String, Job>, kind: &str, ) -> Result<Vec<Vec<String>>, String>`

**crates/executor/src/docker.rs**
- `pub fn forget_pulled_images()`
- `pub struct DockerRuntime`
- `pub fn is_available() -> bool`
- `pub fn track_container(id: &str)`
- `pub fn untrack_container(id: &str)`
- `pub fn track_network(id: &str)`
- `pub fn untrack_network(id: &str)`
- `pub async fn cleanup_resources(docker: &Docker)`
- `pub async fn cleanup_containers(docker: &Docker) -> Result<(), String>`
- `pub async fn cleanup_networks(docker: &Docker) -> Result<(), String>`
- `pub async fn create_job_network(docker: &Docker) -> Result<String, ContainerError>`
- `pub fn get_tracked_containers() -> Vec<String>`
- `pub fn get_tracked_networks() -> Vec<String>`

**crates/executor/src/environment.rs**
- `pub fn setup_github_environment_files(workspace_dir: &Path) -> io::Result<()>`
- `pub fn isolate_github_environment_files( job_env: &mut HashMap<String, String>, ) -> io::Result<tempfile::TempDir>`
- `pub const CONTAINER_WORKSPACE: &str = "/github/workspace"`
- `pub const CONTAINER_GITHUB_DIR: &str = "/github/workflow"`
- `pub fn apply_github_container_context(env: &mut HashMap<String, String>)`
- `pub fn apply_gitlab_container_context(env: &mut HashMap<String, String>)`
- `pub struct RunRef<'a>`
- `pub fn create_github_context( workflow: &WorkflowDefinition, workspace_dir: &Path, run_ref: &RunRef<'_>, ) -> HashMap<String, String>`
- `pub fn add_job_context(env: &mut HashMap<String, String>, job_name: &str)`
- `pub fn add_matrix_context( env: &mut HashMap<String, String>, matrix_combination: &MatrixCombination, )`

**crates/executor/src/events.rs**
- `pub enum ExecutionEvent`
- `pub trait ExecutionObserver: Send + Sync`
- `pub const EXIT_CODE_LABEL: &str = "Exit code:"`
- `pub const STDOUT_HEADER: &str = "Standard Output:"`
- `pub const STDERR_HEADER: &str = "Standard Error:"`
- `pub struct FromStep`
- `pub struct RunHooks`
- `pub async fn with_run_hooks<F: Future>(hooks: RunHooks, future: F) -> F::Output`

**crates/executor/src/expression.rs**
- `pub enum ExprValue`
- `pub struct ExpressionContext<'a>`
- `pub fn evaluate(expr: &str, ctx: &ExpressionContext) -> Result<ExprValue, String>`
- `pub fn evaluate_as_bool(expr: &str, ctx: &ExpressionContext) -> Result<bool, String>`
- `pub fn strip_expression_wrapper(expr: &str) -> &str`
- `pub fn contexts_read(expr: &str) -> Option<Vec<String>>`
- `pub fn text_reads_context(text: &str, context: &str) -> bool`
- `pub fn evaluate_matrix_text(text: &str, matrix: Option<&HashMap<String, Value>>) -> Option<String>`
- `pub fn calls_status_function(expr: &str) -> bool`

**crates/executor/src/github_env_files.rs**
- `pub struct StepEnvironmentUpdates`
- `pub fn parse_github_kv_file(content: &str) -> HashMap<String, String>`
- `pub fn parse_github_path_file(content: &str) -> Vec<String>`
- `pub fn read_step_environment_updates(job_env: &HashMap<String, String>) -> StepEnvironmentUpdates`
- `pub fn apply_step_environment_updates( job_env: &mut HashMap<String, String>, job_user_env: &mut HashMap<String, String>, step_outputs_map: &mut HashMap<String, HashMap<String, String>>, step_id: Option<&str>, )`
- `pub fn clear_step_files(job_env: &HashMap<String, String>)`

**crates/executor/src/job_memory.rs**
- `pub const JOB_MEMORY_FILE_ENV: &str = "WRKFLW_JOB_MEMORY_FILE"`

**crates/executor/src/kubernetes/manifest.rs**
- `pub(super) struct Mount`
- `pub(super) fn plan_mounts(volumes: &[(&Path, &Path)]) -> Vec<Mount>`
- `pub(super) fn nested_excludes(mount: &Mount, all: &[Mount]) -> Vec<String>`
- `pub(super) fn is_nested(mount: &Mount, all: &[Mount]) -> bool`
- `pub(super) struct PodInput<'a>`
- `pub(super) fn build_pod_manifest(config: &KubernetesConfig, input: &PodInput<'_>) -> Value`
- `pub(super) fn active_deadline_seconds(config: &KubernetesConfig) -> u64`
- `pub(super) fn helper_resources() -> Value`
- `pub(super) fn escape_k8s_expansion(s: &str) -> String`

**crates/executor/src/kubernetes/mod.rs**
- `pub const DEFAULT_HELPER_IMAGE: &str = "busybox:1.36@sha256:73aaf090f3d85aa34ee199857f03fa3a95c8ede2ffd4cc2cdb5b94e566b11662"`
- `pub const DEFAULT_TIMEOUT_SECS: u64 = 6 * 60 * 60`
- `pub const DEFAULT_SCHEDULE_TIMEOUT_SECS: u64 = 10 * 60`
- `pub struct KubernetesConfig`
- `pub fn parse_key_value(s: &str) -> Result<(String, String), String>`
- `pub fn is_available() -> bool`
- `pub fn is_available_with(config: &KubernetesConfig) -> bool`
- `pub fn check_available(config: &KubernetesConfig) -> Result<(), String>`
- `pub async fn cleanup_resources()`
- `pub struct KubernetesRuntime`

**crates/executor/src/kubernetes/sync.rs**
- `pub(super) fn drain_stderr(child: &mut std::process::Child) -> std::thread::JoinHandle<String>`
- `pub(super) fn copy_dir_into_pod( kubectl: &OsStr, kubectl_args: &[String], pod: &str, mount: &Mount, ) -> Result<(), ContainerError>`
- `pub(super) fn stream_dir_into( mut command: std::process::Command, mount: &Mount, ) -> Result<(), ContainerError>`
- `pub(super) fn unpack_archive<R: io::Read>( reader: R, host: &Path, cancelled: &AtomicBool, ) -> io::Result<HashSet<PathBuf>>`
- `pub(super) fn prune_to_archive( host: &Path, seen: &HashSet<PathBuf>, keep: &[String], ) -> io::Result<()>`
- `pub(super) fn prune_unlisted( host: &Path, rel: &Path, seen: &HashSet<PathBuf>, keep: &[PathBuf], ) -> io::Result<()>`
- `pub(super) fn copy_dir_out_of_pod( kubectl: &OsStr, kubectl_args: &[String], pod: &str, mount: &Mount, excludes: &[String], cancelled: &AtomicBool, ) -> Result<(), ContainerError>`
- `pub(super) fn sync_from( mut command: std::process::Command, src: &str, host: &Path, excludes: &[String], prune: bool, cancelled: &AtomicBool, ) -> Result<(), ContainerError>`

**crates/executor/src/lib.rs**
- `pub mod acceleration`
- `pub mod action_fidelity`
- `pub mod action_resolver`
- `pub mod dependency`
- `pub mod docker`
- `pub mod engine`
- `pub mod environment`
- `pub mod events`
- `pub mod expression`
- `pub mod github_env_files`
- `pub mod kubernetes`
- `pub mod podman`
- `pub mod run_options`
- `pub mod runner_fidelity`
- `pub mod substitution`
- `pub mod vm_memory`
- `pub mod workspace`
- `pub mod setup_errors`

**crates/executor/src/log_redaction.rs**
- `pub struct RunScope(())`
- `pub fn begin_run() -> RunScope`

**crates/executor/src/podman.rs**
- `pub struct PodmanRuntime`
- `pub fn is_available() -> bool`
- `pub fn track_container(id: &str)`
- `pub fn untrack_container(id: &str)`
- `pub async fn cleanup_resources()`
- `pub async fn cleanup_containers() -> Result<(), String>`
- `pub fn get_tracked_containers() -> Vec<String>`

**crates/executor/src/run_options.rs**
- `pub const FRESH_WORKSPACE_ENV: &str = "WRKFLW_FRESH_WORKSPACE"`
- `pub const EVERY_MATRIX_LEG_ENV: &str = "WRKFLW_EVERY_MATRIX_LEG"`
- `pub fn set_fresh_workspaces(on: bool)`
- `pub fn fresh_workspaces() -> bool`
- `pub fn set_every_matrix_leg(on: bool)`
- `pub fn every_matrix_leg() -> bool`
- `pub fn switch_is_on(value: &str) -> bool`

**crates/executor/src/runner_fidelity.rs**
- `pub struct RunnerSubstitution`
- `pub fn runner_os_for(runtime: RuntimeType, requested: &[&str]) -> String`
- `pub struct RequestedOs`
- `pub fn requested_os( job: &Job, matrix: Option<&HashMap<String, Value>>, platform_overrides: &HashMap<String, String>, ) -> Option<RequestedOs>`

**crates/executor/src/substitution.rs**
- `pub fn preprocess_command(command: &str, matrix_values: &HashMap<String, Value>) -> String`
- `pub fn process_step_run(run: &str, matrix_combination: &Option<HashMap<String, Value>>) -> String`
- `pub fn apply_matrix_to_steps(steps: &[Step], matrix_values: &HashMap<String, Value>) -> Vec<Step>`
- `pub fn preprocess_hash_files(text: &str, workspace: &Path) -> Result<String, String>`
- `pub fn expand_hash_files(expr: &str, workspace: &Path) -> Result<String, String>`
- `pub fn preprocess_expressions( text: &str, workspace: &Path, ctx: &crate::expression::ExpressionContext<'_>, ) -> Result<String, String>`
- `pub fn preprocess_expressions_strict( text: &str, workspace: &Path, ctx: &crate::expression::ExpressionContext<'_>, ) -> Result<String, String>`

**crates/executor/src/vm_memory.rs**
- `pub const GITHUB_RUNNER_BYTES: u64 = 16 * GIB`
- `pub const MAX_PARALLEL_JOBS_ENV: &str = "WRKFLW_MAX_PARALLEL_JOBS"`
- `pub struct Memory`
- `pub const PROBED_ENV: &str = "WRKFLW_PROBED_MEMORY"`
- `pub fn probe(cli: &str) -> Option<Memory>`

**crates/executor/src/workflow_commands.rs**
- `pub enum WorkflowCommand`
- `pub fn parse_workflow_commands(output: &str) -> Vec<WorkflowCommand>`

**crates/executor/src/workspace.rs**
- `pub fn remove_interrupted_fresh_workspaces()`
- `pub const WORKSPACES_DIR_ENV: &str = "WRKFLW_WORKSPACES_DIR"`
- `pub fn workspaces_root() -> Option<PathBuf>`
- `pub struct CleanReport`
- `pub fn clean() -> std::io::Result<CleanReport>`

**crates/github/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/github/README.md**
- `## wrkflw-github`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**crates/github/src/lib.rs**
- `pub enum GithubError`
- `pub struct RepoInfo`
- `pub fn get_repo_info() -> Result<RepoInfo, GithubError>`
- `pub fn workflow_dispatch_path_segment(name: &str) -> Option<String>`
- `pub async fn list_workflows(_repo_info: &RepoInfo) -> Result<Vec<String>, GithubError>`
- `pub async fn trigger_workflow( workflow_name: &str, branch: Option<&str>, inputs: Option<HashMap<String, String>>, ) -> Result<(), GithubError>`
- `pub struct RemoteJob`
- `pub struct RemoteRun`
- `pub struct RunReference`
- `pub fn parse_run_reference(text: &str) -> Option<RunReference>`
- `pub async fn fetch_run(reference: &RunReference) -> Result<RemoteRun, GithubError>`

**crates/gitlab/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/gitlab/README.md**
- `## wrkflw-gitlab`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**crates/gitlab/src/lib.rs**
- `pub enum GitlabError`
- `pub struct RepoInfo`
- `pub fn get_repo_info() -> Result<RepoInfo, GitlabError>`
- `pub async fn list_pipelines(_repo_info: &RepoInfo) -> Result<Vec<String>, GitlabError>`
- `pub async fn trigger_pipeline( branch: Option<&str>, variables: Option<HashMap<String, String>>, ) -> Result<(), GitlabError>`

**crates/logging/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/logging/README.md**
- `## wrkflw-logging`

**crates/logging/src/lib.rs**
- `pub mod symbols`
- `pub fn set_console_to_stderr(enabled: bool)`
- `pub fn set_redactor(redactor: impl Fn(&str) -> String + Send + Sync + 'static) -> bool`
- `pub enum LogLevel`
- `pub fn set_log_level(level: LogLevel)`
- `pub fn set_quiet_mode(quiet: bool)`
- `pub fn get_log_level() -> LogLevel`
- `pub fn log(level: LogLevel, message: &str)`
- `pub fn get_logs() -> Vec<String>`
- `pub fn clear_logs()`
- `pub fn debug(message: &str)`
- `pub fn info(message: &str)`
- `pub fn warning(message: &str)`
- `pub fn error(message: &str)`

**crates/logging/src/symbols.rs**
- `pub const SUCCESS: &str = "\u`
- `pub const FAILURE: &str = "\u`
- `pub const RUNNING: &str = "\u`
- `pub const SKIPPED: &str = "\u`
- `pub const NOT_STARTED: &str = "\u`
- `pub const WARNING: &str = "\u`
- `pub const INFO: &str = "\u`
- `pub const DEBUG: &str = "\u`
- `pub const GEAR: &str = "\u`
- `pub const LOCK: &str = "\u`
- `pub const BLOCKED: &str = "\u`
- `pub const SEPARATOR: &str = "\u`
- `pub const ARROW: &str = "\u`
- `pub const HRULE: &str = "\u`
- `pub const SELECTED: &str = "\u`
- `pub const CHECKBOX_ON: &str = "[\u`
- `pub const CHECKBOX_OFF: &str = "[ ]"`
- `pub const TAB_DIVIDER: &str = " \u`
- `pub const NESTED: &str = "\u`
- `pub const SPINNER: &[&str] = &[ "\u`

**crates/matrix/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/matrix/README.md**
- `## wrkflw-matrix`

**crates/matrix/src/lib.rs**
- `pub enum MatrixEntries`
- `pub fn is_expression(value: &str) -> bool`
- `pub fn check_strategy_expression(expr: &str) -> Result<(), String>`
- `pub struct MatrixConfig`
- `pub struct MatrixCombination`
- `pub enum MatrixError`
- `pub fn expand_matrix(matrix: &MatrixConfig) -> Result<Vec<MatrixCombination>, MatrixError>`
- `pub fn format_combination_name(job_name: &str, combination: &MatrixCombination) -> String`

**crates/models/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/models/README.md**
- `## wrkflw-models`

**crates/models/src/lib.rs**
- `pub enum Severity`
- `pub struct ValidationIssue`
- `pub const UNCLASSIFIED_RULE: &str = "unclassified"`
- `pub struct ValidationResult`
- `pub fn rule_hint(rule: &str) -> Option<&'static str>`
- `pub fn yaml_line_for_path(source: &str, segments: &[String]) -> Option<usize>`
- `pub mod gitlab`

**crates/parser/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/parser/README.md**
- `## wrkflw-parser`

**crates/parser/src/github-workflow.json**
- `"$schema": "http://json-schema.org/draft-07/schema#"`
- `"$id": "https://json.schemastore.org/github-workflow.json"`
- `"$comment": "https://help.github.com/en/github/automating-your-workflow-with-github-actions/workflow-syntax-for-github-actions"`
- `"additionalProperties": false`
- `"definitions": {`
- `"properties": {`
- `"required": ["on", "jobs"]`
- `"type": "object"`

**crates/parser/src/gitlab-ci.json**
- `"$schema": "http://json-schema.org/draft-07/schema#"`
- `"$id": "https://gitlab.com/.gitlab-ci.yml"`
- `"markdownDescription": "Gitlab has a built-in solution for doing CI called Gitlab CI. It is configured by supplying a file called `.gitlab-ci.yml`, which will list all the jobs that are going to run for the project. A full list of all options can be found [here](https://docs.gitlab.com/ee/ci/yaml/). [Learn More](https://docs.gitlab.com/ee/ci/)."`
- `"type": "object"`
- `"properties": {`
- `"patternProperties": {`
- `"additionalProperties": {`
- `"definitions": {`

**crates/parser/src/gitlab.rs**
- `pub enum GitlabParserError`
- `pub fn parse_pipeline(pipeline_path: &Path) -> Result<Pipeline, GitlabParserError>`
- `pub fn validate_pipeline_structure(pipeline: &Pipeline) -> ValidationResult`
- `pub const SHELL_BASH_OR_SH: &str = "wrkflw-bash-or-sh"`
- `pub enum JobWhen`
- `pub enum SkipReason`
- `pub struct JobDecision`
- `pub fn job_decision(job_name: &str, job: &Job, variables: &HashMap<String, String>) -> JobDecision`
- `pub fn pipeline_exclusion( pipeline: &Pipeline, variables: &HashMap<String, String>, ) -> Option<SkipReason>`
- `pub const CONTINUE_ON_ERROR_ALWAYS: &str = "true"`
- `pub fn convert_to_workflow_format(pipeline: &Pipeline) -> workflow::WorkflowDefinition`

**crates/parser/src/gitlab_config.rs**
- `pub fn load_pipeline(path: &Path) -> Result<Value, String>`

**crates/parser/src/lib.rs**
- `pub mod gitlab`
- `pub mod schema`
- `pub mod workflow`

**crates/parser/src/schema.rs**
- `pub enum SchemaType`
- `pub struct SchemaValidator`

**crates/parser/src/workflow.rs**
- `pub struct ContainerCredentials`
- `pub struct JobContainer`
- `pub struct DefaultsRun`
- `pub struct Defaults`
- `pub struct WorkflowDefinition`
- `pub struct Strategy`
- `pub struct Job`
- `pub struct Service`
- `pub struct Step`
- `pub struct ActionInfo`
- `pub fn parse_workflow(path: &Path) -> Result<WorkflowDefinition, String>`

**crates/runs/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`
- `[target.'cfg(unix)'.dependencies]`
- `[target.'cfg(windows)'.dependencies]`

**crates/runs/README.md**
- `## wrkflw-runs`

**crates/runs/src/digest.rs**
- `pub const DEFAULT_TAIL_LINES: usize = 40`
- `pub fn strip_ansi(line: &str) -> String`
- `pub struct FailureDigest`
- `pub struct FailedStep`
- `pub struct FailureCause`
- `pub fn failure_digest( store: &RunStore, run: &RunRecord, tail_lines: usize, ) -> Option<FailureDigest>`
- `pub fn render_digest(digest: &FailureDigest) -> String`

**crates/runs/src/env.rs**
- `pub const HOME: &str = "WRKFLW_HOME"`
- `pub const NO_HISTORY: &str = "WRKFLW_NO_HISTORY"`
- `pub const RUNS_KEEP: &str = "WRKFLW_RUNS_KEEP"`
- `pub const AGENT: &str = "WRKFLW_AGENT"`
- `pub const INTENT: &str = "WRKFLW_INTENT"`
- `pub const SESSION: &str = "WRKFLW_SESSION"`
- `pub const AI_AGENT: &str = "AI_AGENT"`
- `pub const CLAUDE_CODE: &str = "CLAUDECODE"`
- `pub const CLAUDE_CODE_SESSION: &str = "CLAUDE_CODE_SESSION_ID"`
- `pub const CODEX_THREAD: &str = "CODEX_THREAD_ID"`
- `pub const CODEX_SANDBOX: &str = "CODEX_SANDBOX"`
- `pub const CODEX_CI: &str = "CODEX_CI"`
- `pub const GEMINI_CLI: &str = "GEMINI_CLI"`
- `pub const OPENCODE: &str = "OPENCODE"`
- `pub const OPENCODE_CLIENT: &str = "OPENCODE_CLIENT"`
- `pub const CURSOR_AGENT: &str = "CURSOR_AGENT"`
- `pub const AUGMENT_AGENT: &str = "AUGMENT_AGENT"`
- `pub const MCP: &str = "WRKFLW_MCP"`
- `pub const REPLAY_OF: &str = "WRKFLW_REPLAY_OF"`

**crates/runs/src/lib.rs**
- `pub mod env`

**crates/runs/src/record.rs**
- `pub const SCHEMA_VERSION: u32 = 1`
- `pub enum RunStatus`
- `pub struct RunRecord`
- `pub struct JobRecord`
- `pub struct StepRecord`

**crates/runs/src/recorder.rs**
- `pub struct RunMeta`
- `pub fn detect_agent() -> Option<String>`
- `pub fn detect_session() -> Option<String>`
- `pub struct RunOptions`
- `pub struct RecordedRun`
- `pub async fn repo_root_of(cwd: &Path) -> String`
- `pub fn emit_line(value: &Value)`
- `pub fn verdict(record: &RunRecord) -> &'static str`
- `pub fn mark_interrupted()`
- `pub async fn execute_recorded( path: &Path, config: ExecutionConfig, options: RunOptions, ) -> RecordedRun`
- `pub async fn record_skipped( path: &Path, config: &ExecutionConfig, options: &RunOptions, reason: &str, )`

**crates/runs/src/store.rs**
- `pub const DEFAULT_KEEP: usize = 500`
- `pub fn home_dir() -> Option<PathBuf>`
- `pub fn history_disabled() -> bool`
- `pub struct RunStore`

**crates/runtime/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/runtime/README.md**
- `## wrkflw-runtime`

**crates/runtime/src/container.rs**
- `pub const LOCAL_IMAGE_PREFIX: &str = "wrkflw-"`
- `pub const COMBINED_IMAGE_PREFIX: &str = "wrkflw-combined:"`
- `pub trait ContainerRuntime: Sync`
- `pub const PATH_ADDITIONS_ENV: &str = "WRKFLW_PATH_ADDITIONS"`
- `pub const DEFAULT_CONTAINER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"`
- `pub fn container_path(env_vars: &[(&str, &str)], image_path: Option<&str>) -> Option<String>`
- `pub fn container_env(env_vars: &[(&str, &str)], image_path: Option<&str>) -> Vec<(String, String)>`
- `pub fn path_from_image_env<S: AsRef<str>>(env: &[S]) -> Option<String>`
- `pub struct ContainerOutput`
- `pub enum ContainerError`

**crates/runtime/src/emulation.rs**
- `pub struct EmulationRuntime`
- `pub async fn handle_special_action(action: &str) -> Result<(), ContainerError>`
- `pub async fn cleanup_resources()`
- `pub fn track_process(pid: u32)`
- `pub fn untrack_process(pid: u32)`
- `pub fn track_workspace(path: &Path)`
- `pub fn untrack_workspace(path: &Path)`
- `pub fn get_tracked_workspaces() -> Vec<PathBuf>`
- `pub fn get_tracked_processes() -> Vec<u32>`

**crates/runtime/src/lib.rs**
- `pub mod container`
- `pub mod emulation`
- `pub mod live_output`
- `pub mod sandbox`
- `pub mod secure_emulation`

**crates/runtime/src/live_output.rs**
- `pub struct OutputTap(Arc<TapFn>)`
- `pub enum Stream`
- `pub struct Utf8Chunks`
- `pub async fn with_output_tap<F: Future>(tap: OutputTap, future: F) -> F::Output`
- `pub fn current() -> Option<OutputTap>`
- `pub fn send(stream: Stream, chunk: &str)`
- `pub async fn wait_with_live_output( mut child: tokio::process::Child, ) -> std::io::Result<std::process::Output>`
- `pub fn blocking_wait_with_live_output( mut child: std::process::Child, tap: Option<OutputTap>, ) -> std::io::Result<std::process::Output>`

**crates/runtime/src/sandbox.rs**
- `pub struct SandboxConfig`
- `pub enum SandboxError`
- `pub struct Sandbox`
- `pub fn create_workflow_sandbox_config() -> SandboxConfig`
- `pub fn create_strict_sandbox_config() -> SandboxConfig`

**crates/runtime/src/secure_emulation.rs**
- `pub struct SecureEmulationRuntime`
- `pub async fn handle_special_action_secure(action: &str) -> Result<(), ContainerError>`

**crates/secrets/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[features]`
- `[dev-dependencies]`
- `[[bench]]`

**crates/secrets/README.md**
- `# wrkflw-secrets`

**crates/secrets/src/config.rs**
- `pub struct SecretConfig`
- `pub enum SecretProviderConfig`

**crates/secrets/src/error.rs**
- `pub type SecretResult<T> = Result<T, SecretError>`
- `pub enum SecretError`

**crates/secrets/src/lib.rs**
- `pub mod config`
- `pub mod error`
- `pub mod manager`
- `pub mod masking`
- `pub mod providers`
- `pub mod rate_limit`
- `pub mod storage`
- `pub mod substitution`
- `pub mod validation`
- `pub mod prelude`

**crates/secrets/src/manager.rs**
- `pub struct SecretManager`

**crates/secrets/src/masking.rs**
- `pub struct SecretMasker`

**crates/secrets/src/providers/env.rs**
- `pub struct EnvironmentProvider`

**crates/secrets/src/providers/file.rs**
- `pub struct FileProvider`

**crates/secrets/src/providers/mod.rs**
- `pub mod env`
- `pub mod file`
- `pub struct SecretValue`
- `pub trait SecretProvider: Send + Sync`

**crates/secrets/src/rate_limit.rs**
- `pub struct RateLimitConfig`
- `pub struct RateLimiter`

**crates/secrets/src/storage.rs**
- `pub struct EncryptedSecretStore`
- `pub struct KeyDerivation`

**crates/secrets/src/substitution.rs**
- `pub struct SecretSubstitution<'a>`
- `pub struct SecretRef`

**crates/secrets/src/validation.rs**
- `pub const MAX_SECRET_SIZE: usize = 1024 * 1024`
- `pub const MAX_SECRET_NAME_LENGTH: usize = 255`
- `pub fn validate_secret_name(name: &str) -> SecretResult<()>`
- `pub fn validate_secret_value(value: &str) -> SecretResult<()>`
- `pub fn validate_provider_name(name: &str) -> SecretResult<()>`
- `pub fn sanitize_for_logging(input: &str) -> String`
- `pub fn looks_like_secret(value: &str) -> bool`

**crates/trigger-filter/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/trigger-filter/README.md**
- `## wrkflw-trigger-filter`

**crates/trigger-filter/src/config.rs**
- `pub const DEFAULT_GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(10)`
- `pub const DEFAULT_GIT_STATE_TTL: Duration = Duration::from_secs(3)`
- `pub const DEFAULT_PATTERN_CACHE_SIZE: usize = 128`
- `pub const DEFAULT_EVENT_NAME: &str = "push"`
- `pub struct TriggerFilterConfig`

**crates/trigger-filter/src/error.rs**
- `pub enum TriggerFilterError`

**crates/trigger-filter/src/eval.rs**
- `pub fn evaluate_trigger( config: &WorkflowTriggerConfig, context: &EventContext, ) -> TriggerMatchResult`

**crates/trigger-filter/src/git.rs**
- `pub fn validate_ref_name(name: &str) -> Result<(), TriggerFilterError>`
- `pub async fn get_changed_files( base: &str, cwd: Option<&Path>, ) -> Result<Vec<String>, TriggerFilterError>`
- `pub async fn get_changed_files_with_warnings( base: &str, cwd: Option<&Path>, ) -> Result<(Vec<String>, Vec<String>), TriggerFilterError>`
- `pub async fn get_changed_files_between( base_ref: &str, head_ref: &str, cwd: Option<&Path>, ) -> Result<Vec<String>, TriggerFilterError>`
- `pub async fn get_current_branch(cwd: Option<&Path>) -> Result<Option<String>, TriggerFilterError>`
- `pub async fn get_default_diff_base( cwd: Option<&Path>, verbose: bool, ) -> Result<String, TriggerFilterError>`
- `pub enum FindRepoRootError`
- `pub fn find_repo_root_detailed() -> Result<std::path::PathBuf, FindRepoRootError>`
- `pub fn head_mtime(cwd: Option<&Path>) -> Option<std::time::SystemTime>`
- `pub async fn get_current_tag(cwd: Option<&Path>) -> Result<Option<String>, TriggerFilterError>`

**crates/trigger-filter/src/lib.rs**
- `pub mod config`
- `pub mod error`
- `pub mod eval`
- `pub mod git`
- `pub mod model`
- `pub mod parser`
- `pub mod path_matcher`
- `pub mod ref_matcher`
- `pub fn canonicalize_allowing_missing(path: &Path) -> PathBuf`
- `pub fn load_trigger_config( workflow_path: &Path, ) -> Result<WorkflowTriggerConfig, TriggerFilterError>`
- `pub fn load_trigger_configs( paths: &[PathBuf], ) -> (Vec<WorkflowTriggerConfig>, Vec<(PathBuf, String)>)`
- `pub fn filter_trigger_configs( configs: &[&WorkflowTriggerConfig], context: &EventContext, ) -> Vec<TriggerMatchResult>`
- `pub fn load_trigger_config_cached( workflow_path: &Path, config: &TriggerFilterConfig, ) -> Result<WorkflowTriggerConfig, TriggerFilterError>`
- `pub fn load_trigger_configs_cached( paths: &[PathBuf], config: &TriggerFilterConfig, ) -> (Vec<WorkflowTriggerConfig>, Vec<(PathBuf, String)>)`
- `pub fn clear_pattern_cache()`
- `pub async fn auto_detect_context( event_name: &str, diff_base: &str, cwd: Option<&Path>, ) -> Result<EventContext, TriggerFilterError>`
- `pub async fn auto_detect_context_default_base( event_name: &str, cwd: Option<&Path>, verbose: bool, ) -> Result<EventContext, TriggerFilterError>`
- `pub async fn context_from_diff_range( event_name: &str, base_ref: &str, head_ref: &str, cwd: Option<&Path>, ) -> Result<EventContext, TriggerFilterError>`
- `pub async fn context_from_changed_files( event_name: &str, changed_files: Vec<String>, cwd: Option<&Path>, ) -> Result<EventContext, TriggerFilterError>`
- `pub fn normalize_user_changed_file(raw: &str) -> Result<String, TriggerFilterError>`
- `pub fn normalize_user_changed_files(raw: &[String]) -> Result<Vec<String>, TriggerFilterError>`

**crates/trigger-filter/src/model.rs**
- `pub struct MustDrainWarnings`
- `pub struct GlobPattern`
- `pub struct EventFilter`
- `pub struct WorkflowTriggerConfig`
- `pub struct EventContext`
- `pub struct TriggerMatchResult`

**crates/trigger-filter/src/parser.rs**
- `pub fn parse_trigger_config( workflow: &WorkflowDefinition, workflow_path: PathBuf, ) -> Result<WorkflowTriggerConfig, TriggerFilterError>`

**crates/trigger-filter/src/path_matcher.rs**
- `pub fn matches_paths( changed_files: &[String], include_patterns: &[GlobPattern], exclude_patterns: &[GlobPattern], ) -> bool`

**crates/trigger-filter/src/ref_matcher.rs**
- `pub fn matches_ref( ref_name: &str, include_patterns: &[GlobPattern], exclude_patterns: &[GlobPattern], ) -> bool`

**crates/ui/Cargo.toml**
- `[package]`
- `[features]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/ui/README.md**
- `## wrkflw-ui`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**crates/ui/src/app/mod.rs**
- `pub async fn run_wrkflw_tui( path: Option<&PathBuf>, runtime_type: RuntimeType, verbose: bool, preserve_containers_on_failure: bool, show_action_messages: bool, theme: Option<ThemePreference>, platform_overrides: HashMap<String, String>, kubernetes: wrkflw_executor::KubernetesConfig, ) -> io::Result<()>`

**crates/ui/src/app/state.rs**
- `pub struct App`
- `pub struct DispatchOutcome`
- `pub enum TriggerPlatform`
- `pub enum Accent`
- `pub type DiffFilterResults = Vec<(PathBuf, Option<TriggerMatchStatus>)>`
- `pub type DiffFilterParseFailures = Vec<(PathBuf, String)>`
- `pub struct DiffFilterReport`
- `pub enum DiffFilterOutcome`
- `pub type DiffFilterReceiver = mpsc::Receiver<DiffFilterOutcome>`
- `pub struct TriggerTarget`
- `pub fn secrets_provider_count() -> usize`

**crates/ui/src/cli_style.rs**
- `pub fn success(text: &str) -> String`
- `pub fn error(text: &str) -> String`
- `pub fn warning(text: &str) -> String`
- `pub fn info(text: &str) -> String`
- `pub fn skipped(text: &str) -> String`
- `pub fn section(text: &str) -> String`
- `pub fn separator() -> String`
- `pub fn dim(text: &str) -> String`
- `pub fn job_success(name: &str) -> String`
- `pub fn job_failure(name: &str) -> String`
- `pub fn job_failure_allowed(name: &str) -> String`
- `pub fn job_skipped(name: &str) -> String`
- `pub fn step_success(name: &str) -> String`
- `pub fn step_failure(name: &str) -> String`
- `pub fn step_skipped(name: &str) -> String`
- `pub fn indent(text: &str) -> String`
- `pub fn key_value(key: &str, value: &str) -> String`

**crates/ui/src/components/button.rs**
- `pub struct Button`

**crates/ui/src/components/checkbox.rs**
- `pub struct Checkbox`

**crates/ui/src/components/dag.rs**
- `pub fn topo_levels(def: &WorkflowDefinition) -> Vec<Vec<String>>`
- `pub fn matrix_size_label(matrix: &wrkflw_matrix::MatrixConfig) -> Option<String>`

**crates/ui/src/components/mod.rs**
- `pub mod dag`
- `pub mod progress_dots`

**crates/ui/src/components/progress_bar.rs**
- `pub struct ProgressBar`

**crates/ui/src/components/progress_dots.rs**
- `pub enum DotState`
- `pub fn render(frame: &mut Frame<'_>, area: Rect, dots: &[DotState], done: usize, total: usize)`
- `pub fn synthesise( completed: &[StepStatus], total: usize, workflow_status: &WorkflowStatus, ) -> Vec<DotState>`

**crates/ui/src/handlers/mod.rs**
- `pub mod workflow`

**crates/ui/src/handlers/workflow.rs**
- `pub fn validate_workflow(path: &Path, verbose: bool) -> io::Result<()>`
- `pub async fn execute_workflow_cli( path: &Path, runtime_type: RuntimeType, verbose: bool, show_action_messages: bool, platform_overrides: HashMap<String, String>, kubernetes: wrkflw_executor::KubernetesConfig, ) -> io::Result<()>`
- `pub async fn execute_curl_trigger( workflow_name: &str, branch: Option<&str>, ) -> Result<(Vec<wrkflw_executor::JobResult>, ()), String>`
- `pub fn start_next_workflow_execution( app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, verbose: bool, )`

**crates/ui/src/lib.rs**
- `pub mod cli_style`
- `pub mod handlers`
- `pub mod app`
- `pub mod components`
- `pub mod log_processor`
- `pub mod models`
- `pub mod selection`
- `pub mod text`
- `pub mod theme`
- `pub mod utils`
- `pub mod views`

**crates/ui/src/log_processor.rs**
- `pub struct ProcessedLogEntry`
- `pub struct LogProcessingRequest`
- `pub struct LogProcessingResponse`
- `pub struct LogProcessor`

**crates/ui/src/models/mod.rs**
- `pub type ExecutionResultMsg = (usize, Result<(Vec<wrkflw_executor::JobResult>, ()), String>)`
- `pub enum TriggerMatchStatus`
- `pub struct Workflow`
- `pub enum JobState`
- `pub struct QueuedExecution`
- `pub enum WorkflowStatus`
- `pub struct WorkflowExecution`
- `pub const LIVE_TAIL_LINES: usize = 500`
- `pub struct LiveTails`
- `pub struct LiveTail`
- `pub const TIMELINE_FAILURE_TAIL: usize = 6`
- `pub enum TimelineEntry`
- `pub const TIMELINE_LINE_LIMIT: usize = 4096`
- `pub enum RunUpdate`
- `pub type TimelineMsg = (usize, u64, RunUpdate)`
- `pub struct JobExecution`
- `pub struct StepExecution`
- `pub enum StatusSeverity`
- `pub enum LogBadge`
- `pub enum LogFilterLevel`

**crates/ui/src/selection.rs**
- `pub struct Selection`
- `pub fn handle_mouse(selection: &mut Option<Selection>, mouse: MouseEvent) -> Option<KeyCode>`
- `pub fn apply(selection: &mut Option<Selection>, buf: &mut Buffer) -> Option<String>`
- `pub enum CopyMethod`
- `pub fn copy_to_clipboard(text: &str) -> io::Result<CopyMethod>`

**crates/ui/src/text.rs**
- `pub fn width(s: &str) -> usize`
- `pub fn sanitize(s: &str) -> Cow<'_, str>`
- `pub fn truncate(s: &str, max: usize) -> Cow<'_, str>`
- `pub fn truncate_left(s: &str, max: usize) -> Cow<'_, str>`
- `pub fn fit(s: &str, cells: usize) -> String`
- `pub fn pad_left(s: &str, cells: usize) -> String`
- `pub fn fit_spans(spans: Vec<Span<'static>>, cells: usize) -> Vec<Span<'static>>`
- `pub fn spans_width(spans: &[Span<'_>]) -> usize`
- `pub fn ansi_line(raw: &str, base: Style) -> Line<'static>`

**crates/ui/src/theme.rs**
- `pub fn set_accent_override(color: Option<Color>)`
- `pub fn current_accent() -> Color`
- `pub enum ThemeMode`
- `pub enum ThemePreference`
- `pub fn mode() -> ThemeMode`
- `pub fn install(flag: Option<ThemePreference>)`
- `pub struct Colors`
- `pub struct ActiveColors`
- `pub static COLORS: ActiveColors = ActiveColors`
- `pub fn title_style() -> Style`
- `pub fn selected_style() -> Style`
- `pub const SELECTED_MARK: &str = "▌ "`
- `pub fn header_style() -> Style`
- `pub fn search_highlight() -> Style`
- `pub fn dim_style() -> Style`
- `pub fn muted_style() -> Style`
- `pub fn key_style() -> Style`
- `pub fn hint_style() -> Style`
- `pub fn panel_style() -> Style`
- `pub fn workflow_status(status: &WorkflowStatus) -> (&'static str, Style)`
- `pub fn spinner(frame: usize) -> &'static str`
- `pub fn workflow_status_animated( status: &WorkflowStatus, spinner_frame: usize, ) -> (&'static str, Style)`
- `pub fn job_state(state: crate::models::JobState, spinner_frame: usize) -> (&'static str, Style)`
- `pub fn job_status(status: &JobStatus) -> (&'static str, Style)`
- `pub fn step_status(status: &StepStatus) -> (&'static str, Style)`
- `pub fn block<'a>(title: &'a str) -> Block<'a>`
- `pub fn block_focused<'a>(title: &'a str) -> Block<'a>`
- `pub fn panel<'a>(title: &'a str, focused: bool) -> Block<'a>`
- `pub fn modal_frame<'a>(title: &'a str) -> Block<'a>`
- `pub fn with_badge<'a>(block: Block<'a>, badge: impl Into<String>) -> Block<'a>`
- `pub fn section<'a>(title: impl Into<String>) -> Line<'a>`
- `pub fn raised_buttons<'a>( buttons: &[(&str, &str, bool)], max_width: usize, ) -> (Vec<Span<'a>>, Vec<Span<'a>>)`
- `pub fn scrollbar(f: &mut Frame<'_>, area: Rect, total: usize, position: usize, viewport: usize)`
- `pub fn modal(f: &mut Frame<'_>, width: u16, height: u16) -> Rect`
- `pub fn empty_state(f: &mut Frame<'_>, area: Rect, title: &str, hint: Vec<Span<'_>>)`
- `pub enum BadgeKind`
- `pub fn badge_outline<'a>(text: impl Into<String>, kind: BadgeKind) -> Span<'a>`
- `pub fn badge_solid<'a>(text: impl Into<String>, kind: BadgeKind) -> Span<'a>`
- `pub fn badge<'a>(text: &'a str, bg: Color, fg: Color) -> Span<'a>`
- `pub fn key_chip<'a>(key: impl Into<String>) -> Span<'a>`
- `pub fn pulse_style(frame: usize) -> Style`
- `pub fn log_badge(level: &str) -> Style`

**crates/ui/src/utils/mod.rs**
- `pub fn extract_job_names(path: &Path) -> Vec<String>`
- `pub fn load_workflows(dir_path: &Path) -> Vec<Workflow>`

**crates/ui/src/views/dag_tab.rs**
- `pub fn render_dag_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/execution_tab.rs**
- `pub fn render_execution_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`
- `pub(super) fn render_summary_line(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect)`

**crates/ui/src/views/help_overlay.rs**
- `pub fn render_help_content(f: &mut Frame<'_>, area: Rect, scroll: &mut usize)`
- `pub fn render_help_overlay(f: &mut Frame<'_>, scroll: &mut usize)`

**crates/ui/src/views/job_detail.rs**
- `pub const INSPECTOR_TABS: [&str`
- `pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect)`
- `pub struct StepOutputCache`

**crates/ui/src/views/logs_tab.rs**
- `pub fn render_logs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/mod.rs**
- `pub const INSPECTOR_TABS: usize = job_detail::INSPECTOR_TABS.len()`
- `pub fn render_ui(f: &mut Frame<'_>, app: &mut App)`

**crates/ui/src/views/run_header.rs**
- `pub fn render_run_header(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/runs_tab.rs**
- `pub fn render_runs_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/secrets_tab.rs**
- `pub fn render_secrets_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/status_bar.rs**
- `pub fn render_status_bar(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/title_bar.rs**
- `pub const TAB_WORKFLOWS: usize = 0`
- `pub const TAB_EXECUTION: usize = 1`
- `pub const TAB_DAG: usize = 2`
- `pub const TAB_LOGS: usize = 3`
- `pub const TAB_TRIGGER: usize = 4`
- `pub const TAB_SECRETS: usize = 5`
- `pub const TAB_RUNS: usize = 6`
- `pub const SCREEN_COUNT: usize = 7`
- `pub const TABS: [(&str, &[usize])`
- `pub const TAB_COUNT: usize = TABS.len()`
- `pub const TAB_RUN: usize = 1`
- `pub fn tab_of(screen: usize) -> usize`
- `pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/trigger_tab.rs**
- `pub fn render_trigger_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/tweaks_overlay.rs**
- `pub fn render_tweaks_overlay(f: &mut Frame<'_>, app: &App, _area: Rect)`

**crates/ui/src/views/workflows_tab.rs**
- `pub fn render_workflows_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/utils/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[target.'cfg(unix)'.dependencies]`

**crates/utils/README.md**
- `## wrkflw-utils`

**crates/utils/src/lib.rs**
- `pub const GITHUB_WORKFLOWS_DIR: &str = ".github/workflows"`
- `pub const GITEA_WORKFLOWS_DIR: &str = ".gitea/workflows"`
- `pub fn default_workflow_dirs_in(root: &Path) -> Vec<PathBuf>`
- `pub fn default_workflow_dirs() -> Vec<PathBuf>`
- `pub fn default_workflow_dir() -> PathBuf`
- `pub fn is_workflow_file(path: &Path) -> bool`
- `pub mod fd`

**crates/validators/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/validators/README.md**
- `## wrkflw-validators`

**crates/validators/src/actions.rs**
- `pub fn validate_action_reference( action_ref: &str, with_params: Option<&serde_yaml::Mapping>, job_name: &str, step_idx: usize, repo_root: Option<&Path>, result: &mut ValidationResult, )`

**crates/validators/src/expressions.rs**
- `pub fn validate_expressions(workflow: &Value, result: &mut ValidationResult)`

**crates/validators/src/gitlab.rs**
- `pub fn validate_gitlab_pipeline(pipeline: &Pipeline) -> ValidationResult`

**crates/validators/src/jobs.rs**
- `pub fn validate_jobs(jobs: &Value, repo_root: Option<&Path>, result: &mut ValidationResult)`

**crates/validators/src/lib.rs**
- `pub fn validate_env(env: &Value, context: &str, result: &mut ValidationResult)`

**crates/validators/src/matrix.rs**
- `pub fn validate_matrix(matrix: &Value, result: &mut ValidationResult)`

**crates/validators/src/steps.rs**
- `pub fn validate_steps( steps: &[Value], job_name: &str, repo_root: Option<&Path>, result: &mut ValidationResult, )`

**crates/validators/src/triggers.rs**
- `pub fn validate_triggers(on: &Value, result: &mut ValidationResult)`

**crates/watcher/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[dev-dependencies]`

**crates/watcher/README.md**
- `## wrkflw-watcher`

**crates/watcher/src/debouncer.rs**
- `pub const DEFAULT_MAX_PENDING_EVENTS: usize = 8192`
- `pub struct Debouncer`

**crates/watcher/src/error.rs**
- `pub enum WatchError`

**crates/watcher/src/lib.rs**
- `pub mod debouncer`
- `pub mod error`
- `pub mod shutdown`
- `pub mod watcher`

**crates/watcher/src/shutdown.rs**
- `pub struct ShutdownSignal`

**crates/watcher/src/watcher.rs**
- `pub const DEFAULT_MAX_CONCURRENT_EXECUTIONS: usize = 4`
- `pub const MAX_REASONABLE_CONCURRENCY: usize = 256`
- `pub struct WatchEvent`
- `pub struct WatcherConfig`
- `pub struct WorkflowWatcher`

**crates/wrkflw/Cargo.toml**
- `[package]`
- `[features]`
- `[dependencies]`
- `[lib]`
- `[[bin]]`
- `[dev-dependencies]`

**crates/wrkflw/README.md**
- `## WRKFLW (CLI and Library)`
- `# Launch the TUI (auto-loads .gitea/workflows and .github/workflows, whichever exist)`
- `# Validate all workflows in the default directory`
- `# Validate a specific file or directory`
- `# Validate multiple files and/or directories`
- `# Run a workflow (auto-detects Docker, then Podman, then falls back to emulation)`
- `# Pick a runtime explicitly`
- `# Diff-aware filtering (skip workflows whose on: block doesn't match)`
- `# Check everything a push would run, reusing what already passed`
- `# See what a run would do, then explain the last failure`
- `# Watch for changes and rerun affected workflows`
- `# Open the TUI explicitly`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**examples/secrets-demo/README.md**
- `# Secrets Management Demo`
- `# .github/workflows/secrets-demo.yml`
- `# ~/.config/wrkflw/secrets.yml`
- `# ~/.config/wrkflw/secrets.yml`
- `# Original: "token": "ghp_1234567890abcdef"`
- `# Masked:   "token": "ghp_***"`

**examples/secrets-demo/secrets-workflow.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/01-dag-diamond.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/02-dag-wide-fan.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/03-dag-linear.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/04-trigger-dispatch.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/05-matrix-inspector.yml**
- `name:`
- `on:`
- `env:`
- `jobs:`

**examples/ui-demo/06-secrets-runtime.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/07-multi-event.yml**
- `name:`
- `on:`
- `jobs:`

**examples/ui-demo/08-failing.yml**
- `name:`
- `on:`
- `jobs:`

**publish_crates.sh**
- `show_help()`
- `update_versions()`
- `test_build()`
- `publish_crates()`
- `show_changelog_info()`

**schemas/github-workflow.json**
- `"$schema": "http://json-schema.org/draft-07/schema#"`
- `"$id": "https://json.schemastore.org/github-workflow.json"`
- `"$comment": "https://help.github.com/en/github/automating-your-workflow-with-github-actions/workflow-syntax-for-github-actions"`
- `"additionalProperties": false`
- `"definitions": {`
- `"properties": {`
- `"required": ["on", "jobs"]`
- `"type": "object"`

**schemas/gitlab-ci.json**
- `"$schema": "http://json-schema.org/draft-07/schema#"`
- `"$id": "https://gitlab.com/.gitlab-ci.yml"`
- `"markdownDescription": "Gitlab has a built-in solution for doing CI called Gitlab CI. It is configured by supplying a file called `.gitlab-ci.yml`, which will list all the jobs that are going to run for the project. A full list of all options can be found [here](https://docs.gitlab.com/ee/ci/yaml/). [Learn More](https://docs.gitlab.com/ee/ci/)."`
- `"type": "object"`
- `"properties": {`
- `"patternProperties": {`
- `"additionalProperties": {`
- `"definitions": {`

**skills/wrkflw/SKILL.md**
- `# wrkflw`

**tests/README.md**
- `# Testing`
- `# All tests`
- `# Unit tests only`
- `# Integration tests of the CLI crate only`
- `# One integration test file`
- `# A specific test`

**tests/fixtures/gitlab-ci/advanced.gitlab-ci.yml**
- `stages:`
- `variables:`
- `workflow:`
- `default:`
- `setup:`
- `build:`
- `test-default:`
- `test-all-features:`
- `test-no-features:`
- `security:`
- `lint:`
- `package:`
- `deploy-staging:`
- `deploy-production:`

**tests/fixtures/gitlab-ci/basic.gitlab-ci.yml**
- `stages:`
- `variables:`
- `image:`
- `build:`
- `test:`
- `lint:`
- `deploy:`

**tests/fixtures/gitlab-ci/docker.gitlab-ci.yml**
- `stages:`
- `variables:`
- `build-docker:`
- `test-docker:`
- `security-scan:`
- `deploy-staging:`
- `deploy-production:`

**tests/fixtures/gitlab-ci/includes.gitlab-ci.yml**
- `stages:`
- `include:`
- `variables:`
- `default:`
- `production_deploy:`
- `staging_deploy:`

**tests/fixtures/gitlab-ci/invalid.gitlab-ci.yml**
- `variables:`
- `build:`
- `test:`
- `deploy:`
- `lint:`
- `cache-test:`

**tests/fixtures/gitlab-ci/minimal.gitlab-ci.yml**
- `image:`
- `build:`
- `test:`

**tests/fixtures/gitlab-ci/services.gitlab-ci.yml**
- `stages:`
- `variables:`
- `default:`
- `build:`
- `unit-tests:`
- `postgres-tests:`
- `redis-tests:`
- `mongo-tests:`
- `all-services-test:`
- `deploy:`

**tests/fixtures/gitlab-ci/workflow.gitlab-ci.yml**
- `stages:`
- `workflow:`
- `variables:`
- `default:`
- `prepare:`
- `build:`
- `debug-build:`
- `test:`
- `lint:`
- `benchmark:`
- `deploy-staging:`
- `deploy-prod:`
- `notify:`

**tests/safe_workflow.yml**
- `name:`
- `on:`
- `jobs:`

**tests/scripts/test-podman-basic.sh**
- `print_status()`
- `print_success()`
- `print_warning()`
- `print_error()`

**tests/scripts/test-preserve-containers.sh**
- `print_status()`
- `print_success()`
- `print_warning()`
- `print_error()`
- `count_wrkflw_containers()`
- `get_wrkflw_containers()`

**tests/security_comparison.yml**
- `name:`
- `on:`
- `jobs:`

**tests/security_demo.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/1-basic-workflow.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/2-reusable-workflow-caller.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/3-reusable-workflow-definition.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/4-mixed-jobs.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/5-no-name-reusable-caller.yml**
- `on:`
- `jobs:`

**tests/workflows/6-invalid-reusable-format.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/7-invalid-regular-job.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/8-cyclic-dependencies.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/cpp-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/example.yml**
- `name:`
- `on:`
- `env:`
- `jobs:`

**tests/workflows/matrix-example.yml**
- `name:`
- `triggers:`
- `env:`
- `jobs:`

**tests/workflows/multi-runtime-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/node-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/python-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/runs-on-array-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/rust-test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/test.yml**
- `name:`
- `on:`
- `jobs:`

**tests/workflows/trigger_gitlab.sh**
- `show_help()`

**tests/workflows/working-secrets-test.yml**
- `name:`
- `on:`
- `jobs:`

---

## AGENTS.md

**Language:** Markdown | **Size:** 14.1 KB | **Lines:** 96

**Declarations:**

---

## BREAKING_CHANGES.md

**Language:** Markdown | **Size:** 31.3 KB | **Lines:** 384

**Declarations:**

---

## CLAUDE.md

**Language:** Markdown | **Size:** 4.4 KB | **Lines:** 66

**Declarations:**

---

## Cargo.toml

**Language:** TOML | **Size:** 2.8 KB | **Lines:** 85

**Declarations:**

---

## INDEX.md

**Language:** Markdown | **Size:** 273.8 KB | **Lines:** 10156

**Declarations:**

---

## README.md

**Language:** Markdown | **Size:** 43.8 KB | **Lines:** 563

**Declarations:**

---

## cliff.toml

**Language:** TOML | **Size:** 3.6 KB | **Lines:** 106

**Declarations:**

---

## crates/README.md

**Language:** Markdown | **Size:** 1.9 KB | **Lines:** 47

**Declarations:**

---

## crates/evaluator/Cargo.toml

**Language:** TOML | **Size:** 529 B | **Lines:** 21

**Declarations:**

---

## crates/evaluator/README.md

**Language:** Markdown | **Size:** 835 B | **Lines:** 29

**Declarations:**

---

## crates/evaluator/src/lib.rs

**Language:** Rust | **Size:** 11.5 KB | **Lines:** 383

**Imports:**
- `colored::*`
- `serde_yaml::{self, Value}`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `wrkflw_models::ValidationResult`
- `wrkflw_validators::{validate_env, validate_expressions, validate_jobs, validate_triggers}`

**Declarations:**

`fn find_repo_root(workflow_path: &Path) -> Option<PathBuf>`

`mod tests`

---

## crates/executor/Cargo.toml

**Language:** TOML | **Size:** 1.2 KB | **Lines:** 52

**Imports:**
- `ignore`

**Declarations:**

---

## crates/executor/README.md

**Language:** Markdown | **Size:** 2.5 KB | **Lines:** 41

**Declarations:**

---

## crates/executor/src/acceleration/concurrency.rs

**Language:** Rust | **Size:** 4.0 KB | **Lines:** 117

**Imports:**
- `super::current`
- `std::sync::Arc`
- `tokio::sync::Semaphore`
- `wrkflw_parser::workflow::Job`

**Declarations:**

`struct Concurrency`
> Fields: `group: String`, `cancel_in_progress: bool`

`fn concurrency_group(job: &Job) -> Option<Concurrency>`

`pub(crate) async fn acquire_concurrency( job_name: &str, job: &Job, ) -> Option<tokio::sync::OwnedSemaphorePermit>`

`mod tests`

---

## crates/executor/src/acceleration/sidecar.rs

**Language:** Rust | **Size:** 3.6 KB | **Lines:** 106

**Imports:**
- `super::SIDECAR_PATH`
- `serde::Deserialize`
- `std::collections::HashMap`
- `std::path::Path`

**Declarations:**

**`impl Default for JobHints`**
  `fn default() -> Self`


`fn default_true() -> bool`

**`impl Sidecar`**
  `pub fn load(root: &Path) -> Result<Self, String>`

  `pub(super) fn root_only_paths(&self) -> Vec<(&str, &str)>`


`mod tests`

---

## crates/executor/src/acceleration/store.rs

**Language:** Rust | **Size:** 22.4 KB | **Lines:** 623

**Imports:**
- `super::{hex, Acceleration, Secret}`
- `hmac::{Hmac, Mac}`
- `serde::{Deserialize, Serialize}`
- `sha2::Sha256`
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `std::sync::atomic::{AtomicBool, Ordering}`

**Declarations:**

`const ENTRY_SCHEMA: u32 = 2`

`const DEFAULT_TTL_SECS: i64 = 7 * 24 * 60 * 60`

`const REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10)`

`pub(crate) struct StoredJob`
> Fields: `name: String`, `canonical_name: String`, `outputs: HashMap<String, String>`, `actions_not_run: Vec<crate::action_fidelity::UnexecutedAction>`, `repeated_from: Option<String>`

`pub(crate) struct Entry`
> Fields: `schema: u32`, `key: String`, `created_at: i64`, `jobs: Vec<StoredJob>`

**`impl Entry`**
  `fn is_usable(&self, key: &str, now: i64) -> bool`


`fn ttl_secs() -> i64`

`fn parse_ttl(value: Option<&str>) -> i64`

`struct SignedEntry`
> Fields: `body: String`, `signature: String`

`type HmacSha256 = Hmac<Sha256>`

`fn sign(key: &Secret, body: &str) -> Option<String>`

`fn unhex(text: &str) -> Option<Vec<u8>>`

`fn verify(key: &Secret, signed: &SignedEntry) -> Option<Entry>`

`pub(crate) struct ResultStore`
> Fields: `root: PathBuf`, `remote: Option<Remote>`, `push: bool`, `key: Option<Secret>`, `remote_down: AtomicBool`, `pruned: AtomicBool`

`enum Remote`
> Variants: `Dir`, `Http`

**`impl Remote`**
  `fn parse(location: &str, token: Option<Secret>) -> Option<Self>`

  `async fn get(&self, key: &str) -> Result<Option<String>, String>`

  `async fn put(&self, key: &str, body: String) -> Result<(), String>`


`fn entry_file(key: &str) -> String`

`fn write_atomically(path: &Path, body: &str) -> Result<(), String>`

**`impl ResultStore`**
  `pub(crate) fn new(root: PathBuf, accel: &Acceleration) -> Self`

  `fn prune_expired(&self)`

  `fn live_remote(&self) -> Option<&Remote>`

  `fn remote_failed(&self, error: &str)`

  `pub(crate) async fn load(&self, key: &str) -> Option<Entry>`

  `fn save_local(&self, entry: &Entry)`

  `pub(crate) async fn save(&self, key: &str, jobs: Vec<StoredJob>, shareable: bool)`


`mod tests`

---

## crates/executor/src/acceleration/tree.rs

**Language:** Rust | **Size:** 9.7 KB | **Lines:** 280

**Imports:**
- `super::{compile_globs, hex, matches_any, NEVER_INPUT}`
- `sha2::{Digest, Sha256}`
- `std::collections::{BTreeMap, HashMap, HashSet}`
- `std::io::{Read, Write}`
- `std::path::Path`
- `std::sync::Mutex`

**Declarations:**

`pub(crate) type TreeHashes = BTreeMap<String, String>`

`pub(crate) struct Tree`
> Fields: `files: TreeHashes`, `digest: String`, `selected: Mutex<HashMap<Vec<String>, String>>`

**`impl Tree`**
  `pub(crate) fn new(files: TreeHashes) -> Option<Self>`

  `pub(super) fn digest_matching(&self, patterns: &[glob::Pattern]) -> Option<String>`


`fn digest_of(files: &impl serde::Serialize) -> Option<String>`

`pub(crate) fn hash_tree(root: &Path) -> Option<TreeHashes>`

`const MAX_NESTED_REPOS: usize = 8`

`fn hash_tree_at(root: &Path, depth: usize) -> Option<TreeHashes>`

`fn is_real_dir(path: &Path) -> bool`

`fn git_paths(root: &Path, args: &[&str]) -> Option<Vec<String>>`

`fn index_blobs(root: &Path) -> Option<HashMap<String, String>>`

`fn is_plain_file(path: &Path) -> bool`

`fn file_mode<'a>(path: &Path, index_entry: Option<&'a String>) -> &'a str`

`fn hash_objects(root: &Path, paths: &[String]) -> Option<Vec<String>>`

`fn hash_path(path: &Path) -> String`

---

## crates/executor/src/acceleration.rs

**Language:** Rust | **Size:** 56.3 KB | **Lines:** 1604

**Imports:**
- `pub(crate) use concurrency::acquire_concurrency`
- `serde_json::Value`
- `sha2::{Digest, Sha256}`
- `pub use sidecar::{JobHints, Sidecar}`
- `std::collections::{BTreeMap, HashMap, HashSet}`
- `std::future::Future`
- `std::path::{Path, PathBuf}`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::{Arc, Mutex, OnceLock}`
- `store::ResultStore`
- *... and 6 more imports*

**Declarations:**

`mod concurrency`

`mod sidecar`

`mod store`

`mod tree`

`const KEY_VERSION: u32 = 3`

`const DEP_CACHE_DIRS: &[(&str, &str)] = &[ ("cargo-registry", "/root/.cargo/registry"), ("cargo-git", "/root/.cargo/git"), ("npm", "/root/.npm"), ("pip", "/root/.cache/pip"), ("go-mod", "/root/go/pkg/mod"), ]`

`const ALWAYS_INPUT: &[&str] = &[".github/actions/**", ".gitea/actions/**", SIDECAR_PATH]`

`const NEVER_INPUT: &[&str] = &[".github/workflows/**", ".gitea/workflows/**"]`

`const SECRET_REFERENCES: &[&str] = &[ "secrets.", "secrets[", "secrets)", "secrets }", "secrets}", "github.token", "github_token", "github-token", ]`

`const MUTATING_ACTIONS: &[&str] = &[ "actions/github-script", "actions/create-release", "actions/upload-release-asset", "actions/labeler", "actions/stale", "actions/first-interaction", "actions/deploy-pages", "actions/attest", "softprops/action-gh-release", "ncipollo/release-action", "peter-evans/create-pull-request", "peter-evans/create-or-update-comment", "peter-evans/repository-dispatch", "marocchino/sticky-pull-request-comment", "thollander/actions-comment-pull-request", "stefanzweifel/git-auto-commit-action", "ad-m/github-push-action", "googleapis/release-please-action", "changesets/action", "docker/login-action", "docker/build-push-action", "pypa/gh-action-pypi-publish", "js-devtools/npm-publish", "jamesives/github-pages-deploy-action", "peaceiris/actions-gh-pages", ]`

`const ARTIFACT_ACTIONS: &[&str] = &[ "actions/upload-artifact", "actions/download-artifact", "actions/upload-pages-artifact", ]`

`fn uses_one_of<'a>(job: &'a Job, actions: &[&str]) -> Option<&'a str>`

**`impl std::fmt::Debug for Secret`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


`fn is_loopback_url(url: &str) -> bool`

`fn compile_globs(patterns: &[&str]) -> Vec<glob::Pattern>`

`fn glob_options() -> glob::MatchOptions`

`fn matches_any(path: &str, patterns: &[glob::Pattern]) -> bool`

`fn input_patterns( job: &Job, hints: Option<&JobHints>, ) -> Result<Option<Vec<glob::Pattern>>, String>`

`fn local_action_patterns(job: &Job) -> Vec<glob::Pattern>`

`fn grants_write(permissions: &serde_yaml::Value) -> bool`

`pub(crate) fn uncacheable_reason( job: &Job, workflow: &WorkflowDefinition, hints: Option<&JobHints>, ) -> Option<&'static str>`

`fn searchable(value: &impl serde::Serialize) -> Option<String>`

`fn reads_secrets(job: &Job, job_text: &str, env_text: &str) -> bool`

`fn upstream_reads_secrets(job: &Job, workflow: &WorkflowDefinition, env_text: &str) -> bool`

`fn sorted(map: &HashMap<String, String>) -> BTreeMap<&String, &String>`

`fn canonical(value: Value) -> Value`

`fn hex(bytes: &[u8]) -> String`

`pub(crate) struct KeyInput<'a>`
> Fields: `lookup: &'a Lookup<'a>`, `tree: &'a Tree`, `patterns: Option<&'a [glob::Pattern]>`

`pub(crate) fn job_key(input: &KeyInput<'_>) -> Option<String>`

`fn job_key_with(input: &KeyInput<'_>, every_matrix_leg: bool) -> Option<String>`

`pub(crate) struct RunState`
> Fields: `accel: Acceleration`, `preserve_containers: bool`, `root: PathBuf`, `sidecar: Option<Sidecar>`, `store: Option<ResultStore>`, `tree: OnceCell<Option<Arc<Tree>>>`, `executed: Mutex<HashSet<String>>`, `groups: Mutex<HashMap<String, Arc<Semaphore>>>`, `dep_mounts: OnceLock<Vec<(PathBuf, PathBuf)>>`, `warned_on_host_push: AtomicBool`

`fn results_dir() -> Option<PathBuf>`

`fn warn_outside_repo_root(root: &Path)`

**`impl RunState`**
  `pub(crate) fn new( accel: Acceleration, preserve_containers: bool, root: PathBuf, ) -> Result<Self, String>`

  `async fn tree(&self) -> Option<Arc<Tree>>`

  `fn hints(&self, job_name: &str) -> Option<&JobHints>`


`pub(crate) async fn with_run_state<F: Future>(state: Arc<RunState>, future: F) -> F::Output`

`pub(crate) async fn without_run_state<F: Future>(future: F) -> F::Output`

`pub(crate) fn current() -> Option<Arc<RunState>>`

`pub(crate) fn dep_cache_mounts(mounts_are_copies: bool) -> Vec<(PathBuf, PathBuf)>`

`fn create_dep_cache_dirs(repo: &Path) -> Vec<(PathBuf, PathBuf)>`

`fn repo_cache_id(repo: &Path) -> String`

`fn dep_cache_dirs_in(base: &Path, repo: &Path) -> Vec<(PathBuf, PathBuf)>`

`pub(crate) enum Decision`
> Variants: `Run`, `Reuse`, `Skip`

`pub(crate) enum Reused`
> Variants: `Unaffected`, `Stored`

**`impl Reused`**
  `pub(crate) fn kind(&self) -> &'static str`

  `pub(crate) fn step(&self) -> &'static str`

  `pub(crate) fn note(&self) -> String`


`pub(crate) struct Lookup<'a>`
> Fields: `job_name: &'a str`, `job: &'a Job`, `workflow: &'a WorkflowDefinition`, `env_context: &'a HashMap<String, String>`, `platform_overrides: &'a HashMap<String, String>`, `needs_outputs: &'a HashMap<String, HashMap<String, String>>`, `needs_results: &'a HashMap<String, String>`, `partial: bool`

`fn unaffected( job_name: &str, job: &Job, hints: Option<&JobHints>, changed: &[String], executed: &HashSet<String>, needs_results: &HashMap<String, String>, ) -> Result<bool, String>`

`pub(crate) async fn decide(lookup: Lookup<'_>) -> Decision`

`pub(crate) fn mark_executed(job_name: &str)`

`pub(crate) async fn store(key: &str, jobs: Vec<StoredJob>, env: &HashMap<String, String>)`

`mod tests`

---

## crates/executor/src/action_fidelity.rs

**Language:** Rust | **Size:** 2.5 KB | **Lines:** 67

**Imports:**
- `serde::{Deserialize, Serialize}`

**Declarations:**

**`impl UnexecutedAction`**
  `pub fn reason(&self) -> String`

  `pub fn unverified_entry(&self, job: &str) -> serde_json::Value`


`mod tests`

---

## crates/executor/src/action_resolver.rs

**Language:** Rust | **Size:** 47.4 KB | **Lines:** 1414

**Imports:**
- `once_cell::sync::Lazy`
- `std::collections::{HashMap, VecDeque}`
- `std::ffi::OsString`
- `std::path::{Path, PathBuf}`
- `tokio::sync::RwLock`

**Declarations:**

`const MAX_CACHE_ENTRIES: usize = 256`

`struct BoundedCache`
> Fields: `map: HashMap<String, ResolvedAction>`, `order: VecDeque<String>`

**`impl BoundedCache`**
  `fn new() -> Self`

  `fn get(&self, key: &str) -> Option<&ResolvedAction>`

  `fn insert(&mut self, key: String, value: ResolvedAction)`


`static ACTION_CACHE: Lazy<RwLock<BoundedCache>> = Lazy::new(|| RwLock::new(BoundedCache::new()))`

`const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20)`

`static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(||`

`static NO_REDIRECT_CLIENT: Lazy<reqwest::Client> = Lazy::new(||`

`const GITHUB_RAW_BASE_URL: &str = "https://raw.githubusercontent.com"`

`const NO_ACTION_CACHE_ENV: &str = "WRKFLW_NO_ACTION_CACHE"`

`const ACTION_CACHE_DIR_ENV: &str = "WRKFLW_ACTION_CACHE_DIR"`

`async fn resolve_with( base_url: &str, disk_cache_dir: Option<&Path>, repo: &str, version: &str, sub_path: Option<&str>, token: Option<&str>, ) -> Result<ResolvedAction, String>`

`pub(crate) async fn seed_cache_for_test(repo: &str, version: &str, action_yaml: &str)`

`async fn fetch_action_yaml( base_url: &str, repo: &str, version: &str, sub_path: Option<&str>, token: Option<&str>, ) -> Result<String, String>`

`enum FetchError`
> Variants: `NotFound`, `Other`

**`impl std::fmt::Display for FetchError`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


`fn should_try_action_yaml(err: &FetchError) -> bool`

`async fn fetch_action_file( base_url: &str, repo: &str, version: &str, sub_path: Option<&str>, filename: &str, token: Option<&str>, ) -> Result<String, FetchError>`

`pub(crate) fn is_git_sha(version: &str) -> bool`

`fn action_cache_dir() -> Option<PathBuf>`

`fn action_cache_dir_from( no_cache: Option<OsString>, override_dir: Option<OsString>, platform_cache_dir: Option<PathBuf>, ) -> Option<PathBuf>`

`fn disk_cache_file_name(repo: &str, sub_path: Option<&str>, version: &str) -> String`

`fn read_disk_cache(path: &Path) -> Option<String>`

`fn write_disk_cache(path: &Path, body: &str)`

`fn parse_action_definition(content: &str) -> Result<ResolvedAction, String>`

`fn parse_using(using: &str, runs: &serde_yaml::Value) -> Result<ActionType, String>`

`mod tests`

---

## crates/executor/src/artifacts.rs

**Language:** Rust | **Size:** 53.7 KB | **Lines:** 1415

**Imports:**
- `std::borrow::Cow`
- `std::collections::hash_map::Entry`
- `std::collections::{BTreeSet, HashMap}`
- `std::ffi::OsStr`
- `std::path::{Component, Path, PathBuf}`
- `std::sync::Arc`
- `tokio::sync::RwLock`

**Declarations:**

`fn sanitize_artifact_name(name: &str) -> Result<String, String>`

`fn normalize(path: &Path) -> PathBuf`

`struct PathLine<'a>`
> Fields: `text: Cow<'a, str>`, `path: PathBuf`, `is_dir: bool`

**`impl<'a> PathLine<'a>`**
  `fn parse(workspace: &Path, canonical_workspace: &Path, line: &'a str) -> Self`

  `fn split_at_wildcard(&self) -> (PathBuf, Option<String>)`


`fn is_hidden(name: &OsStr) -> bool`

`fn search_root(workspace: &Path, line: &PathLine<'_>) -> PathBuf`

`struct Search<'a>`
> Fields: `pattern: Option<&'a glob::Pattern>`, `max_depth: Option<usize>`, `include_hidden: bool`, `canonical_workspace: &'a Path`

**`impl Search<'_>`**
  `fn visit(&self, dir: &Path, relative: &Path, chain: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>)`

  `fn descend( &self, dir: &Path, relative: &Path, chain: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>, )`

  `fn everything(&self) -> Self`

  `fn matches(&self, relative: &Path) -> bool`


`fn matching_files( workspace: &Path, canonical_workspace: &Path, line: &PathLine<'_>, include_hidden: bool, ) -> Result<Vec<PathBuf>, String>`

`fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf>`

`struct Selection`
> Fields: `files: BTreeSet<PathBuf>`, `root: PathBuf`

`fn select_files( workspace: &Path, path_pattern: &str, include_hidden: bool, ) -> Result<Selection, String>`

`fn copy_selection(selection: &Selection, target: &Path) -> Result<usize, String>`

`fn walk_files(dir: &Path) -> Result<Vec<PathBuf>, String>`

`struct ArtifactMetadata`
> Fields: `path: PathBuf`

**`impl ArtifactStore`**
  `pub fn new(run_dir: &Path) -> std::io::Result<Self>`

  `pub async fn upload( &self, name: &str, path_pattern: &str, workspace: &Path, options: UploadOptions, ) -> Result<usize, String>`

  `pub async fn download(&self, name: &str, target_dir: &Path) -> Result<usize, String>`

  `pub async fn list(&self) -> Vec<String>`


`mod tests`

---

## crates/executor/src/cache.rs

**Language:** Rust | **Size:** 24.6 KB | **Lines:** 670

**Imports:**
- `sha2::{Digest, Sha256}`
- `std::path::{Path, PathBuf}`

**Declarations:**

`const DEFAULT_MAX_CACHE_SIZE_BYTES: u64 = 1024 * 1024 * 1024`

`const CACHE_KEY_METADATA_FILE: &str = ".cache_key"`

**`impl CacheStore`**
  `pub fn new() -> Result<Self, String>`

  `pub fn with_root(root: PathBuf) -> std::io::Result<Self>`

  `pub fn set_max_size(&mut self, max_size: u64)`

  `pub async fn restore( &self, key: &str, restore_keys: &[String], path: &str, workspace: &Path, ) -> Option<String>`

  `pub async fn save(&self, key: &str, path: &str, workspace: &Path) -> Result<(), String>`

  `fn restore_inner( &self, key: &str, restore_keys: &[String], path: &str, workspace: &Path, ) -> Option<String>`

  `fn save_inner(&self, key: &str, path: &str, workspace: &Path) -> Result<(), String>`

  `fn cache_path_for(&self, key: &str, path: &str) -> PathBuf`

  `fn cache_path(&self, key: &str) -> PathBuf`

  `fn find_by_prefix(&self, prefix: &str, cache_path: &str) -> Option<String>`

  `fn evict_if_needed(&self)`


`fn dir_size(path: &Path) -> u64`

`fn has_dotdot_component(path: &str) -> bool`

`fn validate_cache_path(path: &str, workspace: &Path) -> bool`

`fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), String>`

`mod tests`

---

## crates/executor/src/checkout_sync.rs

**Language:** Rust | **Size:** 62.5 KB | **Lines:** 1569

**Imports:**
- `crate::engine::ExecutionError`
- `ignore::{gitignore::GitignoreBuilder, Match}`
- `std::collections::HashSet`
- `std::path::{Path, PathBuf}`

**Declarations:**

`fn create_gitignore_matcher( dir: &Path, ) -> Result<Option<ignore::gitignore::Gitignore>, ExecutionError>`

`pub(crate) fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError>`

`struct GitDirs`
> Fields: `own: PathBuf`, `common: PathBuf`

`fn git_dirs(root: &Path) -> Option<GitDirs>`

`fn mirror_git_dir(root: &Path, to_git: &Path) -> Result<(), ExecutionError>`

`const SHARED_GIT_ENTRIES: &[&str] = &[ "objects", "refs", "packed-refs", "info", "shallow", "remotes", "branches", "description", "rr-cache", "modules", "lfs", ]`

`const UNSAFE_GIT_CONFIG_KEYS: &[&str] = &["hookspath", "fsmonitor", "worktree"]`

`const UNSAFE_GIT_CONFIG_SECTIONS: &[&str] = &["include", "includeif"]`

`fn write_sanitized_git_config(from: &Path, to: &Path) -> Result<(), ExecutionError>`

`fn sanitized_git_config(config: &str) -> String`

`fn header_end(line: &str) -> Option<usize>`

`fn section_name(header: &str) -> String`

`fn value_continues(text: &str, mut in_quote: bool) -> Option<bool>`

`fn ensure_real_dir(dir: &Path) -> std::io::Result<()>`

`fn remove_entry(path: &Path) -> std::io::Result<()>`

`fn mirror_directory( from: &Path, to: &Path, link: bool, skip: &[&str], link_dirs: &[&str], ) -> Result<(), ExecutionError>`

`struct SyncRules`
> Fields: `gitignore: Option<ignore::gitignore::Gitignore>`, `never: Vec<PathBuf>`, `git_checkout: bool`, `root: PathBuf`, `tracked: Option<Tracked>`, `previously_tracked: Option<Tracked>`, `record: Option<(PathBuf, Option<String>)>`

`struct Tracked`
> Fields: `files: HashSet<PathBuf>`, `dirs: HashSet<PathBuf>`

**`impl Tracked`**
  `fn listing(dir: &Path) -> Option<String>`

  `fn parse(listing: &str) -> Self`

  `fn contains(&self, path: &Path, is_dir: bool) -> bool`


**`impl SyncRules`**
  `fn for_root(root: &Path, workspace: &Path) -> Result<Self, ExecutionError>`

  `fn record_tracked(&self)`

  `fn relative<'a>(&self, source: &'a Path) -> Option<&'a Path>`

  `fn is_tracked(&self, source: &Path, is_dir: bool) -> bool`

  `fn was_tracked(&self, source: &Path, is_dir: bool) -> bool`

  `fn syncs( &self, source: &Path, name: &str, is_dir: bool, nested: &[ignore::gitignore::Gitignore], only_tracked: bool, ) -> bool`

  `fn ignored( &self, source: &Path, is_dir: bool, nested: &[ignore::gitignore::Gitignore], ) -> bool`

  `fn may_delete( &self, source: &Path, name: &str, is_dir: bool, nested: &[ignore::gitignore::Gitignore], only_tracked: bool, ) -> bool`

  `fn nested_gitignore(dir: &Path) -> Option<ignore::gitignore::Gitignore>`

  `fn hidden(name: &str) -> bool`


`fn copy_directory_contents_with_gitignore( from: &Path, to: &Path, rules: &SyncRules, nested: &[ignore::gitignore::Gitignore], only_tracked: bool, ) -> Result<(), ExecutionError>`

`fn link_file(from: &Path, to: &Path) -> Result<(), ExecutionError>`

`fn unchanged_copy(from: &Path, to: &Path) -> bool`

`fn sync_file(from: &Path, to: &Path) -> Result<(), ExecutionError>`

`fn set_modified(path: &Path, time: std::time::SystemTime) -> std::io::Result<()>`

`fn remove_file_even_read_only(path: &Path) -> std::io::Result<()>`

`mod tests`

---

## crates/executor/src/container_guard.rs

**Language:** Rust | **Size:** 5.8 KB | **Lines:** 176

**Imports:**
- `std::process::{Command, Stdio}`

**Declarations:**

`pub(crate) struct ContainerGuard`
> Fields: `cli: &'static str`, `id: String`, `armed: bool`, `preserve: bool`, `untrack: fn(&str)`

**`impl ContainerGuard`**
  `pub(crate) fn new(cli: &'static str, id: &str, preserve: bool, untrack: fn(&str)) -> Self`

  `pub(crate) fn disarm(&mut self)`


**`impl Drop for ContainerGuard`**
  `fn drop(&mut self)`


`mod tests`

---

## crates/executor/src/dependency.rs

**Language:** Rust | **Size:** 17.5 KB | **Lines:** 511

**Imports:**
- `std::collections::{HashMap, HashSet, VecDeque}`
- `wrkflw_parser::workflow::{Job, WorkflowDefinition}`

**Declarations:**

`fn job_not_found_error(target_job: &str, jobs: &HashMap<String, Job>, kind: &str) -> String`

`mod tests`

---

## crates/executor/src/docker.rs

**Language:** Rust | **Size:** 65.1 KB | **Lines:** 1741

**Imports:**
- `crate::container_guard::ContainerGuard`
- `async_trait::async_trait`
- `bollard::{
    container::{Config, CreateContainerOptions},
    models::{ContainerWaitResponse, HostConfig},
    network::CreateNetworkOptions,
    Docker,
}`
- `futures_util::StreamExt`
- `once_cell::sync::Lazy`
- `std::collections::HashMap`
- `std::path::Path`
- `std::sync::Mutex`
- `wrkflw_logging`
- `wrkflw_runtime::container::{
    container_env, container_path, path_from_image_env, ContainerError, ContainerOutput,
    ContainerRuntime, COMBINED_IMAGE_PREFIX, LOCAL_IMAGE_PREFIX,
}`
- *... and 2 more imports*

**Declarations:**

`const LOG_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(10)`

`static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static CREATED_NETWORKS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static CUSTOMIZED_IMAGES: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()))`

`static PULLED_IMAGES: Lazy<Mutex<std::collections::HashSet<String>>> = Lazy::new(|| Mutex::new(std::collections::HashSet::new()))`

`fn already_pulled(image: &str) -> bool`

`fn mark_pulled(image: &str)`

**`impl DockerRuntime`**
  `pub fn new() -> Result<Self, ContainerError>`

  `pub fn new_with_config(preserve_containers_on_failure: bool) -> Result<Self, ContainerError>`

  `pub fn get_customized_image(base_image: &str, customization: &str) -> Option<String>`

  `pub fn set_customized_image(base_image: &str, customization: &str, new_image: &str)`

  `pub fn find_customized_image_key(image: &str, prefix: &str) -> Option<String>`

  `pub fn get_language_specific_image( base_image: &str, language: &str, version: Option<&str>, ) -> Option<String>`

  `pub fn set_language_specific_image( base_image: &str, language: &str, version: Option<&str>, new_image: &str, )`

  `pub async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`


`const PROBE_CLI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8)`

`const PROBE_DAEMON_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5)`

`const PROBE_OVERALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15)`

`struct MemoryWatch`
> Fields: `id: String`, `meter: std::sync::Arc<crate::job_memory::JobMeter>`, `sampled: std::sync::Arc<std::sync::atomic::AtomicBool>`, `broken: std::sync::Arc<std::sync::atomic::AtomicBool>`, `started: std::time::Instant`, `task: tokio::task::JoinHandle<()>`, `ended: Option<bool>`, `ran_for: Option<std::time::Duration>`

`const UNSAMPLED_AFTER: std::time::Duration = std::time::Duration::from_secs(3)`

**`impl MemoryWatch`**
  `fn start(docker: Docker, id: &str, meter: std::sync::Arc<crate::job_memory::JobMeter>) -> Self`

  `fn stop(mut self, out_of_memory: bool, ran_for: Option<std::time::Duration>)`


**`impl Drop for MemoryWatch`**
  `fn drop(&mut self)`


`fn run_time(started: &str, finished: &str) -> Option<std::time::Duration>`

`fn working_set(stats: &bollard::container::MemoryStats) -> Option<u64>`

`fn wait_exit_code( wait_result: Option<&Result<ContainerWaitResponse, bollard::errors::Error>>, ) -> i32`

`pub(crate) fn tracked_containers() -> Vec<String>`

**`impl ContainerRuntime for DockerRuntime`**
  `fn session_cli(&self) -> Option<&'static str>`

  `async fn run_container( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`

  `async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>`

  `async fn image_id(&self, image: &str) -> Option<String>`

  `fn identifies_images(&self) -> bool`


`fn build_container_config( image: &str, cmd: Vec<String>, env: Vec<String>, working_dir: &Path, host_config: HostConfig, is_windows_image: bool, entrypoint: Option<&str>, ) -> Config<String>`

**`impl DockerRuntime`**
  `async fn run_container_inner( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn image_path(&self, image: &str) -> Option<String>`

  `async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image_inner( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`


`mod tests`

---

## crates/executor/src/docker_test.rs

**Language:** Rust | **Size:** 6.4 KB | **Lines:** 198

**Imports:**
- `bollard::Docker`
- `std::{sync::Arc, path::Path}`
- `tokio::sync::Mutex`
- `crate::{
    executor::{docker::{self, DockerRuntime}, RuntimeType},
    runtime::container::{ContainerRuntime, ContainerOutput}
}`

**Declarations:**

`mod docker_cleanup_tests`

---

## crates/executor/src/environment.rs

**Language:** Rust | **Size:** 20.5 KB | **Lines:** 640

**Imports:**
- `chrono::Utc`
- `serde_json`
- `serde_yaml::Value`
- `std::{collections::HashMap, fs, io, path::Path}`
- `wrkflw_matrix::MatrixCombination`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`fn create_runner_temp(dir: &Path) -> io::Result<()>`

`const RUNNER_TEMP_DIR: &str = "runner_temp"`

`fn container_runner_temp() -> String`

`fn value_to_string(value: &Value) -> String`

`fn get_repo_name() -> String`

`fn extract_repo_from_url(url: &str) -> Option<String>`

`fn get_event_name(workflow: &WorkflowDefinition) -> String`

`fn get_workspace_path() -> String`

`fn get_current_sha() -> String`

`fn git_output(directory: &Path, args: &[&str]) -> Option<String>`

`pub(crate) fn current_branch(directory: &Path) -> Option<String>`

`pub(crate) fn exact_tag(directory: &Path) -> Option<String>`

`pub(crate) fn default_branch(directory: &Path) -> String`

`fn get_current_ref() -> String`

`pub(crate) fn get_runner_os() -> String`

`fn get_runner_arch() -> String`

`fn get_temp_dir() -> String`

`fn get_tool_cache_dir() -> String`

`fn get_ref_name(full_ref: &str) -> String`

`fn get_ref_type(full_ref: &str) -> String`

`fn get_repository_owner(repo: &str) -> String`

`fn get_actor() -> String`

`mod tests`

---

## crates/executor/src/events.rs

**Language:** Rust | **Size:** 8.5 KB | **Lines:** 260

**Imports:**
- `crate::engine::{JobStatus, StepStatus}`
- `std::future::Future`
- `std::sync::Arc`
- `wrkflw_parser::workflow::Step`

**Declarations:**

`pub(crate) fn step_display_name(step: &Step, index: usize) -> String`

**`impl FromStep`**
  `pub fn resolve(&self, steps: &[Step]) -> Option<usize>`


`pub(crate) fn emit(build: impl FnOnce() -> ExecutionEvent)`

`pub(crate) fn step_output_observers() -> Vec<Arc<dyn ExecutionObserver>>`

`pub(crate) fn first_step_for(job: &str, steps: &[Step]) -> usize`

`pub(crate) fn skipped_by_from_step(index: usize, first_step: usize, step: &Step) -> bool`

`mod tests`

---

## crates/executor/src/expression.rs

**Language:** Rust | **Size:** 130.7 KB | **Lines:** 3272

**Imports:**
- `serde_yaml::Value`
- `std::collections::{HashMap, HashSet}`
- `serde_json`

**Declarations:**

**`impl ExprValue`**
  `pub fn is_truthy(&self) -> bool`

  `pub fn to_output_string(&self) -> String`

  `fn to_number(&self) -> f64`


`fn parse_number(s: &str) -> f64`

`enum Token`
> Variants: `Ident`, `StringLit`, `NumberLit`, `True`, `False`, `Null`, `Dot`, `LParen`, `LBracket`, `RBracket`, `Star`, `RParen`, `Comma`, `Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge`, `And`, `Or`, `Not`, `Eof`

`struct Tokenizer<'a>`
> Fields: `input: &'a str`, `pos: usize`

**`impl<'a> Tokenizer<'a>`**
  `fn new(input: &'a str) -> Self`

  `fn skip_whitespace(&mut self)`

  `fn tokenize(&mut self) -> Result<Vec<Token>, String>`

  `fn peek_next_byte(&self) -> Option<u8>`

  `fn read_string(&mut self) -> Result<Token, String>`

  `fn read_number(&mut self) -> Result<Token, String>`

  `fn read_ident(&mut self) -> String`


`pub(crate) fn github_context_suffix(key: &str) -> Option<String>`

**`impl<'a> ExpressionContext<'a>`**
  `fn resolve_path(&self, parts: &[String]) -> ExprValue`

  `fn resolve(&self, parts: &[String]) -> ExprValue`


`fn yaml_value_to_expr(v: &Value) -> ExprValue`

`fn json_value_to_expr(v: &serde_json::Value) -> ExprValue`

`fn index_value(value: &ExprValue, key: &ExprValue) -> ExprValue`

`fn filter_children(value: &ExprValue) -> Vec<ExprValue>`

`struct Parser`
> Fields: `tokens: Vec<Token>`, `pos: usize`

**`impl Parser`**
  `fn new(tokens: Vec<Token>) -> Self`

  `fn peek(&self) -> &Token`

  `fn advance(&mut self) -> Token`

  `fn expect(&mut self, expected: &Token) -> Result<(), String>`

  `fn parse_expr(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_or(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_and(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_comparison(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_unary(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_primary(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_ident_or_call(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String>`

  `fn parse_postfix( &mut self, mut value: ExprValue, ctx: &ExpressionContext, ) -> Result<ExprValue, String>`


`fn expr_eq(a: &ExprValue, b: &ExprValue) -> bool`

`fn expr_cmp(a: &ExprValue, b: &ExprValue) -> Option<std::cmp::Ordering>`

`fn expr_to_json(v: &ExprValue) -> serde_json::Value`

`fn call_builtin( name: &str, args: &[ExprValue], ctx: &ExpressionContext, ) -> Result<ExprValue, String>`

`mod tests`

---

## crates/executor/src/github_env_files.rs

**Language:** Rust | **Size:** 18.6 KB | **Lines:** 532

**Imports:**
- `std::collections::HashMap`
- `std::fs`
- `std::path::Path`
- `wrkflw_runtime::container::PATH_ADDITIONS_ENV`

**Declarations:**

`fn is_valid_identifier(s: &str) -> bool`

`mod tests`

---

## crates/executor/src/job_memory.rs

**Language:** Rust | **Size:** 22.5 KB | **Lines:** 621

**Imports:**
- `std::collections::HashMap`
- `std::path::PathBuf`
- `std::sync::atomic::{AtomicBool, AtomicU64, Ordering}`
- `std::sync::{Arc, Mutex}`
- `serde::{Deserialize, Serialize}`
- `sha2::{Digest, Sha256}`

**Declarations:**

`const MIB: u64 = 1024 * 1024`

`const RUNS_KEPT: usize = 3`

`const MIN_RESERVED_BYTES: u64 = 512 * MIB`

`const HEADROOM_BYTES: u64 = 256 * MIB`

`pub(crate) const SIGKILL_EXIT: i32 = 137`

`const KILLED_CHILD_SIGNS: &[&str] = &[ // cargo, for rustc or a build script "(signal: 9, SIGKILL", // gcc and collect2, for ld or cc1 "terminated with signal 9", "Killed signal terminated program", ]`

`fn shell_reports_kill(text: &str) -> bool`

`pub(crate) fn ran_out_of_memory(exit_code: i32, oom_killed: bool, output: &[&str]) -> bool`

`pub(crate) struct JobMeter`
> Fields: `current: Mutex<HashMap<String, u64>>`, `peak: AtomicU64`, `sampled: AtomicBool`, `incomplete: AtomicBool`, `killed: AtomicBool`, `neighbour_killed: AtomicBool`, `id: Mutex<Option<String>>`

**`impl JobMeter`**
  `pub(crate) fn sample(&self, container: &str, bytes: u64)`

  `pub(crate) fn stopped(&self, container: &str)`

  `pub(crate) fn unmeasured(&self)`

  `pub(crate) fn killed(&self)`

  `pub(crate) fn is_incomplete(&self) -> bool`

  `fn outcome(&self) -> Outcome`


`enum Outcome`
> Variants: `Peak`, `Killed`, `Unknown`

`static ACTIVE: Mutex<Vec<std::sync::Weak<JobMeter>>> = Mutex::new(Vec::new())`

`struct Active(Arc<JobMeter>)`

**`impl Active`**
  `fn register(meter: &Arc<JobMeter>) -> Active`


**`impl Drop for Active`**
  `fn drop(&mut self)`


`pub(crate) async fn metered<F: std::future::Future>( future: F, succeeded: impl FnOnce(&F::Output) -> bool, ) -> F::Output`

`pub(crate) fn measure_as(id: &str)`

`pub(crate) fn meter() -> Option<Arc<JobMeter>>`

`pub(crate) fn unmeasured()`

`pub(crate) fn job_id(identity: &str) -> String`

`fn reserve_for(peaks: &[u64], highest: u64) -> Option<u64>`

`const STORE_VERSION: u32 = 1`

`const FORGET_AFTER_SECS: i64 = 90 * 24 * 60 * 60`

`struct Store`
> Fields: `version: u32`, `jobs: HashMap<String, Measured>`

**`impl Default for Store`**
  `fn default() -> Self`


`struct Measured`
> Fields: `peaks: Vec<u64>`, `highest: u64`, `at: i64`

`static STORE_LOCK: Mutex<()> = Mutex::new(())`

`fn lock_file(path: &std::path::Path, exclusive: bool) -> Option<std::fs::File>`

`fn store_path() -> Option<PathBuf>`

`fn load(path: &std::path::Path) -> Store`

`pub(crate) fn reserve(id: &str) -> Option<u64>`

`fn record(id: &str, meter: &JobMeter, passed: bool)`

`fn to_keep(outcome: Outcome, passed: bool) -> Option<Outcome>`

`fn apply(store: &mut Store, id: &str, outcome: Outcome, now: i64)`

`mod tests`

---

## crates/executor/src/job_session.rs

**Language:** Rust | **Size:** 51.3 KB | **Lines:** 1439

**Imports:**
- `crate::acceleration`
- `std::collections::HashMap`
- `std::future::Future`
- `std::path::{Path, PathBuf}`
- `std::process::Stdio`
- `std::sync::Arc`
- `std::time::Duration`
- `tokio::process::Command`
- `wrkflw_parser::workflow::{Job, Service}`
- `wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, PATH_ADDITIONS_ENV,
}`

**Declarations:**

`const SERVICE_READY_TIMEOUT: Duration = Duration::from_secs(180)`

`const SERVICE_POLL: Duration = Duration::from_millis(500)`

`pub(crate) const KEEP_ALIVE: &str = "trap 'exit 0' TERM`

`struct ServiceSpec`
> Fields: `name: String`, `image: String`, `env: HashMap<String, String>`, `volumes: Vec<String>`, `options: Option<String>`, `ports: Vec<String>`

**`impl ServiceSpec`**
  `fn new(name: &str, service: &Service) -> Self`


`pub(crate) const CONTAINER_ACTION_DIR: &str = "/github/action"`

`struct Live`
> Fields: `image: String`, `volumes: Vec<(PathBuf, PathBuf)>`, `id: String`, `action_dir: Option<PathBuf>`, `_cleanup: Containers`

`fn removal_failure(output: std::io::Result<std::process::Output>) -> Option<String>`

`struct Containers`
> Fields: `cli: &'static str`, `ids: Vec<String>`

**`impl Containers`**
  `fn add(&mut self, id: &str)`

  `fn removal_args(&self) -> Vec<String>`


**`impl Drop for Containers`**
  `fn drop(&mut self)`


`struct State`
> Fields: `live: Option<Live>`, `busy: bool`, `unusable: bool`, `failed: Option<String>`, `warned_outside: bool`

`pub(crate) struct JobSession`
> Fields: `cli: &'static str`, `services: Vec<ServiceSpec>`, `state: tokio::sync::Mutex<State>`

`fn untrack_for(cli: &str) -> fn(&str)`

`fn track(cli: &str, id: &str)`

`const ENV_LOADER: &str = "eval \"$(cat)\" && exec env \"$@\""`

`fn shell_quote(value: &str) -> String`

`fn bad_env_name(name: &str) -> Option<&'static str>`

`fn env_script<'a>(env: impl Iterator<Item = (&'a str, &'a str)>) -> Result<String, String>`

`fn exec_args(id: &str, working_dir: &Path, cmd: &[&str]) -> Vec<String>`

`fn fits_env_file(value: &str) -> bool`

`fn service_env<'a>( service: &str, env: impl Iterator<Item = (&'a str, &'a str)>, ) -> Result<(Option<tempfile::NamedTempFile>, Vec<String>), String>`

`const SERVICE_FLAGS_WITH_VALUE: &[&str] = &[ "--health-cmd", "--health-interval", "--health-timeout", "--health-retries", "--health-start-period", "--health-start-interval", "--user", "-u", "--entrypoint", "--tmpfs", "--shm-size", "--ulimit", "--env", "-e", "--label", "-l", "--workdir", "-w", "--memory", "-m", "--memory-swap", "--cpus", "--stop-signal", "--stop-timeout", "--platform", ]`

`const SERVICE_FLAGS_BARE: &[&str] = &["--no-healthcheck", "--init", "--read-only"]`

`fn service_options(service: &str, options: &str) -> Result<Vec<String>, String>`

`fn service_volume(service: &str, volume: &str) -> Result<String, String>`

`fn start_args( name: &str, image: &str, volumes: &[(PathBuf, PathBuf)], service_names: &[&str], ) -> Vec<String>`

`fn service_args( name: &str, network_owner: &str, spec: &ServiceSpec, image: &str, env_args: Vec<String>, ) -> Result<Vec<String>, String>`

`fn remapped_port(port: &str) -> bool`

`fn container_port(port: &str) -> &str`

`fn image_repository(image: &str) -> &str`

`fn check_ports(services: &[ServiceSpec]) -> Result<(), String>`

`async fn cli_output( cli: &str, args: &[String], stdin: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

`async fn wait_with_tap( child: tokio::process::Child, live: bool, ) -> std::io::Result<std::process::Output>`

`async fn run_cli( cli: &str, args: &[String], stdin: Option<&str>, live: bool, ) -> Result<ContainerOutput, ContainerError>`

`fn last_chars(text: &str, count: usize) -> String`

`enum Readiness`
> Variants: `Ready`, `Waiting`, `Failed`

`fn readiness(status: &str) -> Readiness`

**`impl JobSession`**
  `async fn wait_until_ready(&self, name: &str, label: &str) -> Result<(), String>`

  `async fn start_services( &self, job_id: &str, containers: &mut Containers, resolve: &dyn Fn(&str) -> String, ) -> Result<(), String>`

  `async fn start( &self, image: &str, volumes: &[(PathBuf, PathBuf)], resolve: &dyn Fn(&str) -> String, ) -> Result<Live, String>`

  `async fn copy_action_dir(&self, id: &str, dir: &Path) -> Result<(), String>`

  `pub(crate) async fn run( &self, image: &str, cmd: &[&str], env: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], resolve: &dyn Fn(&str) -> String, ) -> Result<Option<ContainerOutput>, String>`


`pub(crate) fn current() -> Option<Arc<JobSession>>`

`pub(crate) fn wanted_for(job: &Job, runtime: &dyn ContainerRuntime) -> Option<&'static str>`

`pub(crate) async fn with_job_session<F: Future>( job: &Job, runtime: &dyn ContainerRuntime, future: F, ) -> F::Output`

`mod tests`

---

## crates/executor/src/kubernetes/manifest.rs

**Language:** Rust | **Size:** 20.8 KB | **Lines:** 553

**Imports:**
- `super::{
    KubernetesConfig, DEADLINE_MARGIN_SECS, MAIN_CONTAINER, MANAGED_BY_LABEL, READY_MARKER,
    SESSION_ID, SESSION_LABEL, SYNC_CONTAINER, SYNC_DIR, SYNC_VOLUME, WAIT_CONTAINER,
}`
- `crate::environment::{CONTAINER_GITHUB_DIR, CONTAINER_WORKSPACE}`
- `serde_json::{json, Value}`
- `std::path::{Path, PathBuf}`

**Declarations:**

`const MIRRORED_MOUNTS: &[&str] = &[CONTAINER_WORKSPACE, CONTAINER_GITHUB_DIR]`

**`impl Mount`**
  `pub(super) fn mirrored(&self) -> bool`


`mod tests`

---

## crates/executor/src/kubernetes/mod.rs

**Language:** Rust | **Size:** 70.7 KB | **Lines:** 2036

**Imports:**
- `async_trait::async_trait`
- `manifest::{build_pod_manifest, is_nested, nested_excludes, plan_mounts, Mount, PodInput}`
- `once_cell::sync::Lazy`
- `serde_json::Value`
- `std::collections::{BTreeMap, HashSet}`
- `std::ffi::{OsStr, OsString}`
- `std::path::Path`
- `std::process::Stdio`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::{Arc, Mutex}`
- *... and 4 more imports*

**Declarations:**

`mod manifest`

`pub(crate) mod sync`

`const DEADLINE_MARGIN_SECS: u64 = 30`

`const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(5)`

`const MAIN_CONTAINER: &str = "main"`

`const WAIT_CONTAINER: &str = "wrkflw-wait"`

`const SYNC_CONTAINER: &str = "wrkflw-sync"`

`const SYNC_VOLUME: &str = "wrkflw-sync"`

`const SYNC_DIR: &str = "/wrkflw-sync"`

`const READY_MARKER: &str = "/wrkflw-sync/ready"`

`const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by"`

`const SESSION_LABEL: &str = "wrkflw.dev/session"`

`const FATAL_WAITING_REASONS: &[&str] = &[ "ErrImageNeverPull", "InvalidImageName", "CreateContainerConfigError", "CreateContainerError", ]`

`const PULL_RETRY_REASONS: &[&str] = &["ErrImagePull", "ImagePullBackOff"]`

`const PULL_RETRY_GRACE: Duration = Duration::from_secs(60)`

`const POLL_FAILURE_GRACE: Duration = Duration::from_secs(60)`

`const POLL_REQUEST_TIMEOUT: &str = "--request-timeout=30s"`

`static SESSION_ID: Lazy<String> = Lazy::new(|| uuid::Uuid::new_v4().simple().to_string())`

`static RUNNING_PODS: Lazy<Mutex<Vec<PodRef>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static PUSHED_IMAGES: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()))`

`struct PodRef`
> Fields: `kubectl: OsString`, `kubectl_args: Vec<String>`, `name: String`

`static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false)`

`fn shutdown_error() -> ContainerError`

**`impl KubernetesConfig`**
  `pub fn helper_image(&self) -> &str`

  `pub fn timeout(&self) -> Duration`

  `pub fn schedule_timeout(&self) -> Duration`

  `fn kubectl_args(&self) -> Vec<String>`

  `pub fn describe(&self) -> String`


`fn check_available_using(kubectl: &OsStr, config: &KubernetesConfig) -> Result<(), String>`

`fn cluster_answers(kubectl: &OsStr, config: &KubernetesConfig) -> bool`

`fn delete_args(name: &str) -> [&str; 5]`

`async fn delete_tracked_pod(pod: &PodRef)`

`fn track_pod(pod: &PodRef)`

`fn untrack_pod(name: &str)`

`struct PodGuard`
> Fields: `pod: PodRef`, `armed: bool`, `preserve: bool`, `cancelled: Arc<AtomicBool>`

**`impl PodGuard`**
  `fn new(pod: PodRef, preserve: bool) -> Self`

  `fn disarm(&mut self)`


**`impl Drop for PodGuard`**
  `fn drop(&mut self)`


`fn inspect_hint(name: &str) -> String`

`fn registry_ref(registry: &str, tag: &str) -> String`

`fn is_locally_built(image: &str) -> bool`

`fn pod_name() -> String`

`fn poll_interval(elapsed: Duration) -> Duration`

`fn container_state<'a>(pod: &'a Value, list: &str, container: &str) -> Option<&'a Value>`

`fn is_running(state: Option<&Value>) -> bool`

`fn terminated_exit_code(state: Option<&Value>) -> Option<i32>`

`fn fatal_waiting_reason(pod: &Value) -> Option<String>`

`fn stuck_reason(pod: &Value, name: &str, watch: &mut PullWatch) -> Option<String>`

`struct PullWatch`
> Fields: `first_failure: Option<std::time::Instant>`

**`impl PullWatch`**
  `fn check( &mut self, pod: &Value, name: &str, now: std::time::Instant, grace: Duration, ) -> Option<String>`


`struct PollFailures`
> Fields: `first_failure: Option<std::time::Instant>`

**`impl PollFailures`**
  `fn check( &mut self, name: &str, error: ContainerError, now: std::time::Instant, grace: Duration, ) -> Result<(), ContainerError>`

  `fn clear(&mut self)`


`fn waiting_reason(pod: &Value, reasons: &[&str]) -> Option<String>`

`fn unschedulable_message(pod: &Value) -> Option<String>`

`fn pod_phase(pod: &Value) -> &str`

`fn pod_failure_message(pod: &Value) -> String`

**`impl KubernetesRuntime`**
  `pub fn new(config: KubernetesConfig, preserve_pods_on_failure: bool) -> Self`

  `fn kubectl_args(&self) -> Vec<String>`

  `async fn kubectl( &self, args: &[&str], stdin: Option<Vec<u8>>, ) -> Result<std::process::Output, ContainerError>`

  `async fn kubectl_ok( &self, args: &[&str], stdin: Option<Vec<u8>>, what: &str, ) -> Result<String, ContainerError>`

  `async fn get_pod(&self, name: &str) -> Result<Value, ContainerError>`

  `async fn poll_pod( &self, name: &str, failures: &mut PollFailures, ) -> Result<Option<Value>, ContainerError>`

  `async fn delete_pod(&self, name: &str)`

  `fn pushed_ref(&self, tag: &str) -> Option<String>`

  `fn cluster_image(&self, image: &str) -> Result<String, ContainerError>`

  `async fn wait_for_init_running(&self, name: &str) -> Result<(), ContainerError>`

  `async fn wait_for_main_terminated(&self, name: &str) -> Result<i32, ContainerError>`

  `async fn clear_mount(&self, name: &str, mount: &Mount) -> Result<(), ContainerError>`

  `async fn copy_in(&self, name: &str, mount: &Mount) -> Result<(), ContainerError>`

  `async fn copy_out( &self, name: &str, mount: &Mount, excludes: Vec<String>, cancelled: Arc<AtomicBool>, ) -> Result<(), ContainerError>`

  `async fn run_container_inner( &self, name: &str, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn run_pod( &self, name: &str, manifest: &Value, working_dir: &Path, mounts: &[Mount], cancelled: Arc<AtomicBool>, ) -> Result<ContainerOutput, ContainerError>`

  `fn local_builder() -> Option<&'static str>`

  `fn registry(&self) -> Result<&str, ContainerError>`

  `async fn builder_command( &self, builder: &str, args: &[&str], timeout: Duration, what: &str, ) -> Result<(), ContainerError>`

  `async fn push_local_image(&self, builder: &str, tag: &str) -> Result<(), ContainerError>`


**`impl ContainerRuntime for KubernetesRuntime`**
  `async fn run_container( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`

  `fn mounts_are_copies(&self) -> bool`

  `async fn prepare_language_environment( &self, language: &str, _version: Option<&str>, _additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`

  `async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>`


`mod tests`

---

## crates/executor/src/kubernetes/sync.rs

**Language:** Rust | **Size:** 29.9 KB | **Lines:** 845

**Imports:**
- `super::manifest::Mount`
- `super::{SYNC_CONTAINER, WAIT_CONTAINER}`
- `std::collections::HashSet`
- `std::ffi::OsStr`
- `std::io::{self, Read}`
- `std::path::{Component, Path, PathBuf}`
- `std::process::Stdio`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `wrkflw_runtime::container::ContainerError`

**Declarations:**

`const MAX_STDERR_BYTES: usize = 64 * 1024`

`fn append_tree<W: io::Write>(builder: &mut tar::Builder<W>, root: &Path) -> io::Result<()>`

`pub(crate) fn safe_relative_path(path: &Path) -> Option<PathBuf>`

`pub(crate) fn clear_type_conflict( host: &Path, rel: &Path, incoming_is_dir: bool, ) -> io::Result<()>`

`mod tests`

---

## crates/executor/src/lib.rs

**Language:** Rust | **Size:** 1.9 KB | **Lines:** 58

**Imports:**
- `pub use acceleration::{validate_remote, Acceleration, Secret, Sidecar, SIDECAR_PATH}`
- `pub use action_fidelity::UnexecutedAction`
- `pub use docker::cleanup_resources`
- `pub use engine::{
    detect_runtime, execute_workflow, parse_platform_override, ExecutionConfig, JobResult,
    JobStatus, RuntimeType, StepResult, StepStatus,
}`
- `pub use events::{
    with_run_hooks, ExecutionEvent, ExecutionObserver, FromStep, RunHooks, EXIT_CODE_LABEL,
    STDERR_HEADER, STDOUT_HEADER,
}`
- `pub use kubernetes::{parse_key_value as parse_k8s_key_value, KubernetesConfig}`
- `pub use log_redaction::{begin_run as begin_log_scope, RunScope as LogScope}`
- `pub use runner_fidelity::RunnerSubstitution`

**Declarations:**

`pub(crate) mod artifacts`

`pub(crate) mod cache`

`pub(crate) mod checkout_sync`

`mod container_guard`

`pub(crate) mod job_memory`

`pub(crate) mod job_session`

`pub(crate) mod live_step`

`pub(crate) mod log_redaction`

`pub(crate) mod matrix_fold`

`pub(crate) mod setup_actions`

`pub(crate) mod vm_workspace`

`pub(crate) mod workflow_commands`

---

## crates/executor/src/live_step.rs

**Language:** Rust | **Size:** 23.0 KB | **Lines:** 638

**Imports:**
- `crate::engine::format_annotation_location`
- `std::sync::{Arc, Mutex}`
- `wrkflw_secrets::SecretMasker`
- `wrkflw_runtime::live_output::Stream`

**Declarations:**

`pub(crate) struct LiveStep<'a>`
> Fields: `job: String`, `step: String`, `index: usize`, `workspace_prefixes: Vec<String>`, `masker: Option<&'a SecretMasker>`

`struct StepLabel`
> Fields: `job: String`, `step: String`, `index: usize`, `workspace_prefixes: Vec<String>`

`const LIVE_LINE_LIMIT: usize = 4096`

`const LIVE_LINE_OVERLAP: usize = 1024`

`const DROPPED_COMMAND_LIMIT: usize = 1 << 20`

**`impl LiveStep<'_>`**
  `pub(crate) async fn run<F: std::future::Future>( self, future: std::pin::Pin<Box<F>>, ) -> F::Output`


`struct LiveLines`
> Fields: `step: StepLabel`, `masker: Option<SecretMasker>`, `observers: Vec<Arc<dyn crate::events::ExecutionObserver>>`, `pending: Mutex<Pending>`

`struct Pending`
> Fields: `stdout: PendingLine`, `stderr: PendingLine`

**`impl Pending`**
  `fn of(&mut self, stream: Stream) -> &mut PendingLine`


`struct PendingLine`
> Fields: `text: String`, `held_command: Option<HeldCommand>`

`struct HeldCommand`
> Fields: `text: String`, `truncated: bool`

**`impl HeldCommand`**
  `fn push(&mut self, text: &str)`

  `fn into_line(self) -> Option<String>`


**`impl LiveLines`**
  `fn push(&self, stream: Stream, chunk: &str)`

  `fn complete_lines(&self, line: &mut PendingLine, mut chunk: &str) -> Vec<String>`

  `fn flush(&self)`

  `fn mask(&self, text: &str) -> String`

  `fn send(&self, text: &str)`


`fn visible_line(line: &str) -> Option<String>`

`fn live_annotation(command: crate::workflow_commands::WorkflowCommand) -> Option<String>`

`mod tests`

---

## crates/executor/src/log_redaction.rs

**Language:** Rust | **Size:** 9.6 KB | **Lines:** 268

**Imports:**
- `once_cell::sync::Lazy`
- `std::sync::{Arc, Mutex, Once, RwLock}`
- `wrkflw_secrets::SecretMasker`

**Declarations:**

`struct RunMaskers`
> Fields: `masker: RwLock<Arc<SecretMasker>>`, `active: Mutex<usize>`

**`impl RunMaskers`**
  `fn new() -> Self`

  `fn current(&self) -> Arc<SecretMasker>`

  `fn begin(&self)`

  `fn end(&self)`


`static LOG_MASKERS: Lazy<RunMaskers> = Lazy::new(RunMaskers::new)`

`static INSTALL: Once = Once::new()`

`fn current() -> Arc<SecretMasker>`

**`impl Drop for RunScope`**
  `fn drop(&mut self)`


`pub(crate) fn register(value: &str)`

`pub(crate) fn mask(text: &str) -> String`

`pub(crate) fn args_for_log<S: AsRef<str>>(args: &[S]) -> String`

`pub(crate) fn env_names_for_log<'a>(names: impl IntoIterator<Item = &'a str>) -> String`

`pub(crate) fn register_all<'a>(values: impl IntoIterator<Item = &'a String>)`

`const UNNAMED_CREDENTIAL_MIN_LEN: usize = 16`

`const CREDENTIAL_NAME_PARTS: &[&str] = &[ "TOKEN", "SECRET", "PASSWORD", "PASSWD", "PASS", "PWD", "KEY", "AUTH", "CREDENTIAL", "PRIVATE", "CERT", "COOKIE", "SESSION", "SIGNATURE", "DSN", ]`

`pub(crate) fn register_variables<'a>( variables: impl IntoIterator<Item = (&'a String, &'a String)>, )`

`fn looks_like_credential(name: &str, value: &str) -> bool`

`mod tests`

---

## crates/executor/src/matrix_fold.rs

**Language:** Rust | **Size:** 22.5 KB | **Lines:** 606

**Imports:**
- `crate::engine::{get_effective_runner_image, JobResult, JobStatus}`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `wrkflw_matrix::MatrixCombination`
- `wrkflw_parser::workflow::Job`

**Declarations:**

`pub(crate) fn matrix_twins( job: &Job, combinations: &[MatrixCombination], platform_overrides: &HashMap<String, String>, ) -> Vec<Option<usize>>`

`fn job_body_text(job: &Job) -> Option<String>`

`fn collect_strings(value: &serde_json::Value, text: &mut String)`

`fn differing_keys(a: &HashMap<String, Value>, b: &HashMap<String, Value>) -> Vec<String>`

`enum Reads`
> Variants: `Keys`, `Any`

`fn matrix_reads(text: &str) -> Reads`

`fn is_word(c: char) -> bool`

`fn mentions<'a>(lower: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a`

`fn reads_matrix_keys(body: &str, keys: &[String]) -> bool`

`pub(crate) fn repeated_leg( name: &str, original: &JobResult, image: &str, runner: Option<&crate::runner_fidelity::RunnerSubstitution>, ) -> JobResult`

`mod tests`

---

## crates/executor/src/podman.rs

**Language:** Rust | **Size:** 38.1 KB | **Lines:** 1028

**Imports:**
- `crate::container_guard::ContainerGuard`
- `async_trait::async_trait`
- `once_cell::sync::Lazy`
- `std::collections::HashMap`
- `std::path::Path`
- `std::process::Stdio`
- `std::sync::Mutex`
- `tempfile`
- `tokio::process::Command`
- `wrkflw_logging`
- *... and 3 more imports*

**Declarations:**

`static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static CUSTOMIZED_IMAGES: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()))`

**`impl PodmanRuntime`**
  `pub fn new() -> Result<Self, ContainerError>`

  `pub fn new_with_config(preserve_containers_on_failure: bool) -> Result<Self, ContainerError>`

  `pub(crate) fn new_unchecked(preserve_containers_on_failure: bool) -> Self`

  `pub fn get_customized_image(base_image: &str, customization: &str) -> Option<String>`

  `pub fn set_customized_image(base_image: &str, customization: &str, new_image: &str)`

  `pub fn find_customized_image_key(image: &str, prefix: &str) -> Option<String>`

  `pub fn get_language_specific_image( base_image: &str, language: &str, version: Option<&str>, ) -> Option<String>`

  `pub fn set_language_specific_image( base_image: &str, language: &str, version: Option<&str>, new_image: &str, )`

  `async fn execute_podman_command( &self, args: &[&str], input: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn run_podman_command( &self, args: &[&str], input: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`


`const PROBE_CLI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8)`

`const PROBE_DAEMON_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5)`

`const PROBE_OVERALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15)`

**`impl ContainerRuntime for PodmanRuntime`**
  `fn session_cli(&self) -> Option<&'static str>`

  `async fn run_container( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`

  `async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>`

  `async fn image_id(&self, image: &str) -> Option<String>`

  `fn identifies_images(&self) -> bool`


`async fn image_path(image: &str) -> Option<String>`

**`impl PodmanRuntime`**
  `async fn run_container_inner( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image_inner( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`


---

## crates/executor/src/run_options.rs

**Language:** Rust | **Size:** 2.9 KB | **Lines:** 79

**Imports:**
- `std::sync::atomic::{AtomicBool, Ordering}`

**Declarations:**

`static FRESH_WORKSPACES: AtomicBool = AtomicBool::new(false)`

`static EVERY_MATRIX_LEG: AtomicBool = AtomicBool::new(false)`

`fn env_flag(name: &str) -> bool`

`mod tests`

---

## crates/executor/src/runner_fidelity.rs

**Language:** Rust | **Size:** 12.6 KB | **Lines:** 364

**Imports:**
- `crate::engine::RuntimeType`
- `crate::events::{emit, ExecutionEvent}`
- `serde::{Deserialize, Serialize}`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `wrkflw_parser::workflow::Job`

**Declarations:**

**`impl RunnerSubstitution`**
  `pub fn reason(&self) -> String`

  `pub fn unverified_entry(&self, job: &str) -> serde_json::Value`


`pub(crate) fn runs_on_labels( runs_on: &Option<Vec<String>>, matrix: Option<&HashMap<String, Value>>, ) -> Vec<String>`

`fn matrix_label(label: &str, matrix: Option<&HashMap<String, Value>>) -> String`

`fn fallback_label(label: &str, matrix: Option<&HashMap<String, Value>>) -> String`

`pub(crate) fn mapped_image<'a>( labels: &[String], platform_overrides: &'a HashMap<String, String>, ) -> Option<&'a String>`

**`impl RequestedOs`**
  `pub fn ran_as(&self, ran_as: &str) -> Option<RunnerSubstitution>`


`pub(crate) fn substitution( job: &Job, matrix: Option<&HashMap<String, Value>>, env: &HashMap<String, String>, platform_overrides: &HashMap<String, String>, ) -> Option<RunnerSubstitution>`

`fn label_os(label: &str) -> Option<&'static str>`

`pub(crate) fn announce(job: &str, substitution: Option<&RunnerSubstitution>)`

`mod tests`

---

## crates/executor/src/setup_actions.rs

**Language:** Rust | **Size:** 46.8 KB | **Lines:** 1149

**Imports:**
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `wrkflw_parser::workflow::Step`
- `crate::action_resolver::is_git_sha`

**Declarations:**

`pub(crate) const NO_IMPLEMENTATION: &str = "no implementation in wrkflw"`

`pub(crate) struct BuiltRuntimes`
> Fields: `installed: Vec<String>`, `not_built: Vec<(usize, NotBuilt)>`, `runtimes: Vec<SetupRuntime>`, `in_composite: bool`

`pub(crate) enum NotBuilt`
> Variants: `Refused`, `Superseded`

**`impl BuiltRuntimes`**
  `fn has(&self, language: &str) -> bool`

  `pub(crate) fn for_composite(&self) -> Self`


`pub(crate) struct SetupDetection`
> Fields: `runtimes: Vec<SetupRuntime>`, `not_built: Vec<(usize, NotBuilt)>`

`pub(crate) struct SetupRuntime`
> Fields: `language: String`, `version: String`, `install_script: String`, `floating: bool`, `env: Vec<(String, String)>`

`pub(crate) fn is_floating_version(language: &str, version: &str) -> bool`

`pub(crate) struct SetupActionDef`
> Fields: `repos: &'static [&'static str]`, `with_key: &'static str`, `default_version: &'static str`, `language: &'static str`, `version_from_ref: bool`, `version_file_key: Option<&'static str>`

`pub(crate) const SETUP_ACTIONS: &[SetupActionDef] = &[ SetupActionDef`

`pub(crate) fn node_major_for_range(range: &str) -> Option<String>`

`pub(crate) fn version_from_file(root: &Path, file: &str) -> Option<String>`

`pub(crate) fn unrun_action_cause( repository: &str, built: &BuiltRuntimes, container_runtime: bool, step_idx: usize, step: Option<&Step>, ) -> Option<&'static str>`

`fn composite_setup_cause( def: &SetupActionDef, built: &BuiltRuntimes, step: Option<&Step>, ) -> Option<&'static str>`

`fn uv_python<'a>(input: impl Fn(&str) -> Option<&'a String>) -> Result<Option<String>, String>`

`fn go_version(text: &str) -> Option<String>`

`fn contained_path(root: &Path, file: &str) -> Option<PathBuf>`

`pub(crate) fn requires_python(text: &str) -> Option<String>`

`pub(crate) fn is_safe_version(version: &str) -> bool`

`pub(crate) fn is_safe_python_spec(spec: &str) -> bool`

`pub(crate) fn detect_setup_runtimes(steps: &[Step]) -> Vec<SetupRuntime>`

`pub(crate) fn detect_setup(steps: &[Step]) -> SetupDetection`

`pub(crate) fn detect_setup_runtimes_in(steps: &[Step], root: &Path) -> Vec<SetupRuntime>`

`pub(crate) fn detect_setup_in(steps: &[Step], root: &Path) -> SetupDetection`

`pub(crate) fn node_alias(version: &str) -> Option<String>`

`pub(crate) fn get_install_script(language: &str, version: &str) -> String`

`fn go_release(version: &str) -> String`

`fn is_full_release(version: &str) -> bool`

`fn go_prerelease(version: &str) -> Option<String>`

`fn go_release_known(version: &str) -> bool`

`pub(crate) const UV_PYTHON_DIR: &str = "/opt/uv/python"`

`pub(crate) const PYTHON_SCRIPTS_DIR: &str = "/opt/uv/python-bin"`

`const PRINT_SCRIPTS_DIR: &str = "import sysconfig`

`pub(crate) const DROP_EXTERNALLY_MANAGED: &str = "import os, sysconfig`

`pub(crate) fn uv_install_command(version: &str) -> String`

`pub(crate) fn uv_install_unless_present() -> String`

`pub(crate) fn uv_python_request(version: &str) -> String`

`pub(crate) const RUNNER_NODE_MAJOR: &str = "22"`

`pub(crate) fn runner_node_install() -> String`

`pub(crate) fn rust_toolchain_extras( with: &HashMap<String, String>, resolve: impl Fn(&str) -> Option<String>, ) -> Vec<String>`

`pub(crate) fn static_input(value: &str) -> Option<String>`

---

## crates/executor/src/substitution.rs

**Language:** Rust | **Size:** 37.7 KB | **Lines:** 1028

**Imports:**
- `lazy_static::lazy_static`
- `regex::Regex`
- `serde_yaml::Value`
- `sha2::{Digest, Sha256}`
- `std::collections::HashMap`
- `std::path::Path`
- `wrkflw_parser::workflow::Step`

**Declarations:**

`fn expression_end(text: &str, start: usize) -> Option<usize>`

`fn compute_hash_files(args_raw: &str, workspace: &Path) -> Result<String, String>`

`fn files_under(dir: &Path) -> Vec<std::path::PathBuf>`

`fn expand_expressions( text: &str, workspace: &Path, ctx: &crate::expression::ExpressionContext<'_>, strict: bool, ) -> Result<String, String>`

`mod tests`

---

## crates/executor/src/vm_memory.rs

**Language:** Rust | **Size:** 20.8 KB | **Lines:** 568

**Imports:**
- `std::sync::{Arc, OnceLock}`
- `std::time::{Duration, Instant}`
- `tokio::sync::{OwnedSemaphorePermit, Semaphore}`

**Declarations:**

`const GIB: u64 = 1024 * 1024 * 1024`

`const WARN_BELOW_BYTES: u64 = 14 * GIB`

`const BYTES_PER_JOB: u64 = 4 * GIB`

`const QUERY_TIMEOUT: Duration = Duration::from_secs(10)`

`const MIB: u64 = 1024 * 1024`

`pub(crate) enum Limit`
> Variants: `Jobs`, `Memory`

`fn vm_reserved(bytes: u64) -> u64`

`fn shared(bytes: u64) -> u64`

`struct Pool`
> Fields: `places: Arc<Semaphore>`, `limit: Limit`

**`impl Pool`**
  `fn new(limit: Limit) -> Pool`

  `fn weight(&self, measured: Option<u64>) -> u32`


`fn mib(bytes: u64) -> usize`

`type RunPool = Arc<OnceLock<Option<Arc<Pool>>>>`

`pub(crate) async fn with_run_pool<F: std::future::Future>(future: F) -> F::Output`

`pub(crate) async fn job_slot(job: Option<&str>) -> Option<OwnedSemaphorePermit>`

`pub(crate) fn sizes_by_memory() -> bool`

`fn env_limit() -> Option<usize>`

`fn parse_limit(value: &str) -> Option<usize>`

`fn limit_for(bytes: u64) -> Option<usize>`

`fn gigabytes(bytes: u64) -> String`

**`impl Memory`**
  `pub fn limit(&self) -> Option<usize>`

  `fn pool_limit(&self) -> Option<Limit>`

  `pub fn warning(&self) -> Option<String>`

  `pub fn to_env(&self) -> String`

  `fn from_env(value: &str) -> Option<Memory>`


`fn warning(memory: &Memory, limit: Option<usize>) -> String`

`fn info(cli: &str, format: &str) -> Option<String>`

`fn parse_info(cli: &str, text: &str, host_is_linux: bool) -> Option<Memory>`

`fn parent_probe(value: &str, cli: &str) -> Option<Memory>`

`pub(crate) async fn check(cli: Option<&str>)`

`fn set_limit(limit: Option<Limit>)`

`async fn limit_from_memory(cli: &str) -> Option<Limit>`

`mod tests`

---

## crates/executor/src/vm_workspace.rs

**Language:** Rust | **Size:** 104.3 KB | **Lines:** 2873

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `std::io::{self, Read, Write}`
- `std::path::{Path, PathBuf}`
- `std::process::{Command, Stdio}`
- `std::sync::atomic::{AtomicU64, Ordering}`
- `std::sync::{Arc, LazyLock, Mutex}`
- `std::time::{Duration, Instant}`
- `sha2::{Digest, Sha256}`
- `wrkflw_runtime::container::ContainerRuntime`
- `crate::kubernetes::sync::{clear_type_conflict, safe_relative_path}`

**Declarations:**

`pub(crate) const MOUNT_ENV: &str = "WRKFLW_WORKSPACE_MOUNT"`

`const HELPER_IMAGE_ENV: &str = "WRKFLW_WORKSPACE_HELPER_IMAGE"`

`const DEFAULT_HELPER_IMAGE: &str = "ubuntu:latest"`

`const LABEL: &str = "wrkflw.workspace"`

`const VOLUME_PREFIX: &str = "wrkflw-ws-"`

`const ROOT: &str = "/ws"`

`enum Kind`
> Variants: `File`, `Dir`, `Link`

**`impl Kind`**
  `fn same_type(&self, other: &Kind) -> bool`


`struct Entry`
> Fields: `kind: Kind`, `mode: Option<u32>`

**`impl Entry`**
  `fn matches(&self, other: &Entry) -> bool`


`type Listing = HashMap<PathBuf, Entry>`

`fn wanted(on_by_default: bool, setting: Option<&str>) -> bool`

`fn volume_name(host: &Path) -> String`

`struct State`
> Fields: `helper: Option<String>`, `volume: Option<Listing>`, `host: Option<Listing>`, `kept: HashSet<PathBuf>`, `pending: HashSet<PathBuf>`, `unlisted: HashSet<PathBuf>`, `unusable: bool`, `used: bool`, `stalled: Option<String>`, `step_in_volume: bool`

**`impl State`**
  `fn new() -> Self`

  `fn for_volume(volume: &str) -> Self`


`fn is_kept(kept: &HashSet<PathBuf>, path: &Path) -> bool`

`fn outside(listing: &Listing, set: &HashSet<PathBuf>) -> Listing`

`fn under_any(sorted: &[PathBuf], path: &Path) -> bool`

`fn outermost(sorted: Vec<PathBuf>) -> Vec<PathBuf>`

`pub(crate) struct VmWorkspace`
> Fields: `cli: &'static str`, `host: PathBuf`, `volume: String`, `volume_path: PathBuf`, `state: Mutex<State>`

`static WORKSPACES: LazyLock<Mutex<HashMap<PathBuf, Arc<VmWorkspace>>>> = LazyLock::new(|| Mutex::new(HashMap::new()))`

`pub(crate) async fn before_step( runtime: &dyn ContainerRuntime, host: &Path, ) -> Result<Option<Arc<VmWorkspace>>, String>`

`pub(crate) async fn after_cancelled_step(host: &Path) -> Result<(), String>`

**`impl VmWorkspace`**
  `pub(crate) fn mount_in<'a>(&'a self, volumes: &mut [(&'a Path, &'a Path)])`

  `pub(crate) async fn after_step(self: &Arc<Self>) -> Result<(), String>`

  `fn lock(&self) -> std::sync::MutexGuard<'_, State>`

  `fn sync_in(&self) -> Result<bool, String>`

  `fn sync_in_locked(&self, state: &mut State) -> io::Result<()>`

  `fn sync_out(&self) -> Result<(), String>`

  `fn sync_out_locked(&self, state: &mut State) -> io::Result<()>`

  `fn helper(&self, state: &mut State) -> io::Result<String>`

  `fn list_volume(&self, helper: &str) -> io::Result<(Listing, HashSet<PathBuf>)>`

  `fn send(&self, helper: &str, paths: &[PathBuf], volume: &Listing) -> io::Result<()>`

  `fn fetch(&self, helper: &str, paths: &[PathBuf], volume: &Listing) -> io::Result<Unpacked>`


`struct SyncIn`
> Fields: `delete: Vec<PathBuf>`, `send: Vec<PathBuf>`

`fn plan_sync_in( host: &Listing, volume: &Listing, before: Option<&Listing>, kept: &HashSet<PathBuf>, on_host: impl Fn(&Path) -> bool, ) -> SyncIn`

`struct SyncOut`
> Fields: `delete: Vec<PathBuf>`, `fetch: Vec<PathBuf>`

`fn plan_sync_out(host: &Listing, volume: &Listing, kept: &HashSet<PathBuf>) -> SyncOut`

`fn parse_listing(out: &[u8]) -> Listing`

`fn path_from_bytes(bytes: &[u8]) -> PathBuf`

`fn path_from_bytes(bytes: &[u8]) -> PathBuf`

`fn path_bytes(path: &Path) -> Vec<u8>`

`fn path_bytes(path: &Path) -> Vec<u8>`

`fn nul_list(paths: &[PathBuf]) -> Vec<u8>`

`fn host_entry(meta: &std::fs::Metadata, path: &Path) -> Option<Entry>`

`fn link_target(target: PathBuf) -> PathBuf`

`fn mtime_secs(meta: &std::fs::Metadata) -> i64`

`fn mtime_secs(meta: &std::fs::Metadata) -> i64`

`fn host_mode(meta: &std::fs::Metadata) -> Option<u32>`

`fn host_mode(_meta: &std::fs::Metadata) -> Option<u32>`

`fn walk(root: &Path) -> io::Result<(Listing, Vec<PathBuf>)>`

`fn without_unreadable(volume: &Listing, unreadable: &[PathBuf]) -> Listing`

`fn remove_host_entry(target: &Path, parent_read_only: bool) -> io::Result<()>`

`fn read_only_in(volume: &Listing, dir: &Path) -> bool`

`fn with_writable_dir<T>(dir: &Path, op: impl FnOnce() -> io::Result<T>) -> io::Result<T>`

`fn make_writable(path: &Path)`

`fn append_host_entry<W: Write>( builder: &mut tar::Builder<W>, full: &Path, rel: &Path, _in_volume: Option<&Entry>, ) -> io::Result<()>`

`struct Unpacked`
> Fields: `refused: Vec<PathBuf>`, `failed: Vec<(PathBuf, io::Error)>`

`fn cannot_hold(e: &io::Error) -> bool`

`fn illegal_name_code() -> Option<i32>`

`fn os_error(e: &io::Error) -> Option<i32>`

`fn spelled_otherwise_on_host( host: &Path, rel: &Path, names: &mut HashMap<PathBuf, Option<HashSet<std::ffi::OsString>>>, ) -> bool`

`fn release_kept(host: &Path, kept: &mut HashSet<PathBuf>) -> Vec<PathBuf>`

`fn unpack<R: Read>(reader: R, host: &Path, volume: &Listing) -> io::Result<Unpacked>`

`fn unlisted_entries(stderr: &[u8]) -> Result<HashSet<PathBuf>, String>`

`fn kept_file(volume: &str) -> Option<PathBuf>`

`fn pending_file(volume: &str) -> Option<PathBuf>`

`fn forget_saved(volume: &str)`

`fn load_paths(file: Option<&Path>) -> HashSet<PathBuf>`

`fn save_paths(file: Option<&Path>, kept: &HashSet<PathBuf>)`

`fn refuse_link_parents(host: &Path, rel: &Path) -> io::Result<()>`

`fn set_dir_mode(dir: &Path, mode: Option<u32>, _entry: Option<&Entry>)`

`fn set_dir_mode(_dir: &Path, _mode: Option<u32>, _entry: Option<&Entry>)`

`const ENGINE_IDLE_LIMIT: Duration = Duration::from_secs(10 * 60)`

`const CLEANUP_IDLE_LIMIT: Duration = Duration::from_secs(60)`

`struct Progress(Arc<AtomicU64>)`

**`impl Progress`**
  `fn add(&self, bytes: usize)`

  `fn get(&self) -> u64`


`struct Counted<T>`
> Fields: `inner: T`, `progress: Progress`

**`impl<T> Counted<T>`**
  `fn new(inner: T, progress: Progress) -> Self`


**`impl<R: Read> Read for Counted<R>`**
  `fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>`


**`impl<W: Write> Write for Counted<W>`**
  `fn write(&mut self, buf: &[u8]) -> io::Result<usize>`

  `fn flush(&mut self) -> io::Result<()>`


`fn bound( mut child: std::process::Child, what: String, idle: Duration, progress: Progress, ) -> std::thread::JoinHandle<io::Result<std::process::ExitStatus>>`

`fn finish( watcher: std::thread::JoinHandle<io::Result<std::process::ExitStatus>>, ) -> io::Result<std::process::ExitStatus>`

`fn drain<R: Read + Send + 'static>( pipe: Option<R>, progress: Progress, ) -> std::thread::JoinHandle<Vec<u8>>`

`fn output_within( command: &mut Command, stdin: Option<Vec<u8>>, what: &str, idle: Duration, ) -> io::Result<std::process::Output>`

`fn only_vanished(stderr: &str) -> bool`

`fn exec(cli: &str, helper: &str, cmd: &[&str], stdin: Option<Vec<u8>>) -> io::Result<Vec<u8>>`

`fn command_failed(cmd: &[&str], stderr: &[u8]) -> io::Error`

`fn exec_output( cli: &str, helper: &str, cmd: &[&str], stdin: Option<Vec<u8>>, ) -> io::Result<std::process::Output>`

`fn track(cli: &str, id: &str)`

`fn remove_container(cli: &str, id: &str)`

`pub(crate) fn release(host: &Path, kept: bool)`

`fn held_by_stopped_containers(cli: &str, volume: &str) -> bool`

`fn stopped_state(state: &str) -> bool`

`const VOLUME_REMOVAL_TRIES: usize = 120`

`const HELD_TRIES: usize = 20`

`static PENDING_REMOVALS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new())`

`pub(crate) fn remove_now(host: &Path)`

`pub(crate) async fn wait_for_released_volumes()`

`fn labelled_volumes(cli: &str) -> Vec<String>`

`fn inspect_format() -> String`

`pub(crate) fn remove_orphaned_volumes() -> usize`

`mod tests`

---

## crates/executor/src/workflow_commands.rs

**Language:** Rust | **Size:** 16.2 KB | **Lines:** 516

**Declarations:**

`fn decode_value(s: &str) -> String`

`pub(crate) fn parse_command_line(line: &str) -> Option<WorkflowCommand>`

`fn parse_params(s: &str) -> std::collections::HashMap<String, String>`

`mod tests`

---

## crates/executor/src/workspace.rs

**Language:** Rust | **Size:** 24.3 KB | **Lines:** 697

**Imports:**
- `std::fs::File`
- `std::future::Future`
- `std::path::{Path, PathBuf}`

**Declarations:**

`pub(crate) async fn with_workflow_file<F: Future>(path: &Path, future: F) -> F::Output`

`fn workflow_file() -> Option<PathBuf>`

`pub(crate) async fn with_caller_job<F: Future>(job: &str, future: F) -> F::Output`

`fn callers() -> Option<String>`

`pub(crate) fn job_identity(repo: &Path, workflow: &str, job: &str) -> String`

`fn workflow_key(workflow: &str, callers: Option<&str>) -> String`

`pub(crate) enum JobWorkspace`
> Variants: `Kept`, `Fresh`

`static FRESH: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new())`

**`impl Drop for JobWorkspace`**
  `fn drop(&mut self)`


**`impl JobWorkspace`**
  `pub(crate) fn path(&self) -> &Path`

  `pub(crate) fn acquire(repo: &Path, workflow: &str, job: &str) -> std::io::Result<Self>`

  `fn fresh() -> std::io::Result<Self>`

  `fn keep( root: &Path, repo: &Path, file: &Path, workflow: &str, job: &str, ) -> std::io::Result<Option<Self>>`


`pub(crate) fn tracked_record(workspace: &Path) -> Option<PathBuf>`

`fn lock_soon(lock: &File) -> std::io::Result<bool>`

`fn root_from(configured: Option<std::ffi::OsString>, cache: Option<PathBuf>) -> Option<PathBuf>`

`fn clean_under(root: &Path) -> std::io::Result<CleanReport>`

`fn size_of(dir: &Path) -> u64`

`fn slug(text: &str) -> String`

`fn short_hash(text: &str) -> u64`

`mod tests`

---

## crates/github/Cargo.toml

**Language:** TOML | **Size:** 689 B | **Lines:** 27

**Declarations:**

---

## crates/github/README.md

**Language:** Markdown | **Size:** 653 B | **Lines:** 23

**Declarations:**

---

## crates/github/src/lib.rs

**Language:** Rust | **Size:** 25.2 KB | **Lines:** 722

**Imports:**
- `lazy_static::lazy_static`
- `regex::Regex`
- `reqwest::header`
- `serde_json::{self}`
- `std::collections::HashMap`
- `std::fs`
- `std::path::Path`
- `std::process::Command`
- `thiserror::Error`

**Declarations:**

`const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30)`

`fn http_client() -> Result<reqwest::Client, GithubError>`

`fn commit_id(text: &str) -> Option<String>`

`async fn list_recent_workflow_runs( repo_info: &RepoInfo, workflow_segment: &str, token: &str, ) -> Result<Vec<serde_json::Value>, GithubError>`

`fn api_token() -> Option<String>`

`async fn api_get( client: &reqwest::Client, path: &str, token: Option<&str>, ) -> Result<serde_json::Value, GithubError>`

`mod tests`

---

## crates/gitlab/Cargo.toml

**Language:** TOML | **Size:** 680 B | **Lines:** 27

**Declarations:**

---

## crates/gitlab/README.md

**Language:** Markdown | **Size:** 608 B | **Lines:** 23

**Declarations:**

---

## crates/gitlab/src/lib.rs

**Language:** Rust | **Size:** 9.2 KB | **Lines:** 284

**Imports:**
- `lazy_static::lazy_static`
- `regex::Regex`
- `reqwest::header`
- `std::collections::HashMap`
- `std::path::Path`
- `std::process::Command`
- `thiserror::Error`

**Declarations:**

`mod tests`

---

## crates/logging/Cargo.toml

**Language:** TOML | **Size:** 538 B | **Lines:** 22

**Declarations:**

---

## crates/logging/README.md

**Language:** Markdown | **Size:** 456 B | **Lines:** 22

**Declarations:**

---

## crates/logging/src/lib.rs

**Language:** Rust | **Size:** 5.3 KB | **Lines:** 179

**Imports:**
- `chrono::Local`
- `once_cell::sync::Lazy`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::{Arc, Mutex, OnceLock}`

**Declarations:**

`static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())))`

`static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)))`

`static QUIET_MODE: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)))`

`static CONSOLE_TO_STDERR: AtomicBool = AtomicBool::new(false)`

`type Redactor = Box<dyn Fn(&str) -> String + Send + Sync>`

`static REDACTOR: OnceLock<Redactor> = OnceLock::new()`

`fn redact(message: &str) -> std::borrow::Cow<'_, str>`

**`impl LogLevel`**
  `fn prefix(&self) -> &'static str`


`mod tests`

---

## crates/logging/src/symbols.rs

**Language:** Rust | **Size:** 2.1 KB | **Lines:** 43

**Declarations:**

---

## crates/matrix/Cargo.toml

**Language:** TOML | **Size:** 572 B | **Lines:** 23

**Declarations:**

---

## crates/matrix/README.md

**Language:** Markdown | **Size:** 720 B | **Lines:** 21

**Declarations:**

---

## crates/matrix/src/lib.rs

**Language:** Rust | **Size:** 40.1 KB | **Lines:** 1180

**Imports:**
- `indexmap::IndexMap`
- `serde::{de, Deserialize, Deserializer, Serialize}`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `thiserror::Error`

**Declarations:**

**`impl<'de> Deserialize<'de> for MatrixEntries`**
  `fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de>,`


`fn evaluate_to_json<F>( label: &str, expr: &str, expected: &str, evaluate: &mut F, ) -> Result<String, MatrixError> where F: FnMut(&str) -> Result<String, String>,`

`fn expression_body(value: &str) -> Option<&str>`

**`impl Default for MatrixEntries`**
  `fn default() -> Self`


**`impl MatrixEntries`**
  `fn resolve<F>(&self, section: &str, evaluate: &mut F) -> Result<Self, MatrixError> where F: FnMut(&str) -> Result<String, String>,`

  `fn literal(&self, section: &str) -> Result<&[HashMap<String, Value>], MatrixError>`


**`impl Default for MatrixConfig`**
  `fn default() -> Self`


**`impl MatrixConfig`**
  `pub fn from_expression(expr: String) -> Self`

  `pub fn resolve_expressions<F>(&self, mut evaluate: F) -> Result<Self, MatrixError> where F: FnMut(&str) -> Result<String, String>,`

  `fn evaluate_whole<F>(expr: &str, evaluate: &mut F) -> Result<Self, MatrixError> where F: FnMut(&str) -> Result<String, String>,`


`fn resolve_axis<F>(axis: &str, expr: &str, evaluate: &mut F) -> Result<Value, MatrixError> where F: FnMut(&str) -> Result<String, String>,`

**`impl MatrixCombination`**
  `pub fn new(values: HashMap<String, Value>) -> Self`

  `pub fn from_include(values: HashMap<String, Value>) -> Self`


**`impl MatrixConfig`**
  `pub fn from_combinations(combinations: Vec<HashMap<String, Value>>) -> Self`


`fn generate_base_combinations( matrix: &MatrixConfig, include: &[HashMap<String, Value>], ) -> Result<Vec<MatrixCombination>, MatrixError>`

`fn generate_combinations( param_names: &[String], param_values: &[Vec<Value>], current_depth: usize, current_combination: &mut HashMap<String, Value>, ) -> Result<Vec<MatrixCombination>, MatrixError>`

`fn apply_exclude_filters( combinations: Vec<MatrixCombination>, exclude_patterns: &[HashMap<String, Value>], ) -> Vec<MatrixCombination>`

`fn is_excluded( combination: &MatrixCombination, exclude_patterns: &[HashMap<String, Value>], ) -> bool`

`fn value_to_string(value: &Value) -> String`

`mod tests`

---

## crates/models/Cargo.toml

**Language:** TOML | **Size:** 472 B | **Lines:** 18

**Declarations:**

---

## crates/models/README.md

**Language:** Markdown | **Size:** 320 B | **Lines:** 16

**Declarations:**

---

## crates/models/src/lib.rs

**Language:** Rust | **Size:** 30.3 KB | **Lines:** 864

**Imports:**
- `serde::Serialize`

**Declarations:**

**`impl Default for ValidationResult`**
  `fn default() -> Self`


**`impl ValidationResult`**
  `pub fn new() -> Self`

  `pub fn add_issue(&mut self, issue: String)`

  `pub fn report(&mut self, rule: &str, message: impl Into<String>)`

  `pub fn warn(&mut self, rule: &str, message: impl Into<String>)`

  `pub fn warnings(&self) -> impl Iterator<Item = &str>`

  `pub fn report_at(&mut self, rule: &str, key: &str, message: impl Into<String>)`

  `pub fn push_scope(&mut self, key: impl Into<String>)`

  `pub fn push_index(&mut self, index: usize)`

  `pub fn pop_scope(&mut self)`

  `pub fn scoped<R>(&mut self, key: impl Into<String>, f: impl FnOnce(&mut Self) -> R) -> R`

  `pub fn locate_lines(&mut self, source: &str)`

  `fn push_issue( &mut self, severity: Severity, rule: &str, message: String, segments: Vec<String>, )`


`fn render_path(segments: &[String]) -> Option<String>`

`fn content_indent(line: &str) -> Option<usize>`

`fn find_key( lines: &[&str], start: usize, end: usize, parent: Option<usize>, key: &str, ) -> Option<(usize, usize)>`

`fn find_sequence_item( lines: &[&str], start: usize, end: usize, parent: Option<usize>, index: usize, ) -> Option<(usize, usize)>`

`fn block_end(lines: &[&str], line: usize, indent: usize, is_item: bool, limit: usize) -> usize`

`mod validation_tests`

---

## crates/parser/Cargo.toml

**Language:** TOML | **Size:** 714 B | **Lines:** 30

**Imports:**
- `tempfile`

**Declarations:**

---

## crates/parser/README.md

**Language:** Markdown | **Size:** 333 B | **Lines:** 13

**Declarations:**

---

## crates/parser/src/github-workflow.json

**Language:** JSON | **Size:** 91.0 KB | **Lines:** 1740

**Declarations:**

---

## crates/parser/src/gitlab-ci.json

**Language:** JSON | **Size:** 104.7 KB | **Lines:** 3014

**Declarations:**

---

## crates/parser/src/gitlab.rs

**Language:** Rust | **Size:** 65.7 KB | **Lines:** 2002

**Imports:**
- `crate::gitlab_config`
- `crate::schema::{SchemaType, SchemaValidator}`
- `crate::workflow`
- `std::collections::{HashMap, HashSet}`
- `std::path::Path`
- `thiserror::Error`
- `wrkflw_matrix::MatrixConfig`
- `wrkflw_models::gitlab::{AllowFailure, Except, Image, Job, Only, Parallel, Pipeline, Rule}`
- `wrkflw_models::ValidationResult`

**Declarations:**

**`impl JobWhen`**
  `fn parse(when: Option<&str>) -> Self`

  `pub fn blocks_later_stages(self) -> bool`

  `pub fn skip_reason(self, earlier_failure: bool) -> Option<&'static str>`


`fn allow_failure_flag(allow_failure: &AllowFailure) -> Option<bool>`

`fn matrix_variable_in_conditions(job: &Job) -> Option<String>`

`fn first_matching_rule<'a>( owner: &str, rules: &'a [Rule], variables: &HashMap<String, String>, ) -> Result<Option<&'a Rule>, SkipReason>`

`fn evaluate_or_warn( owner: &str, keyword: &str, condition: &str, variables: &HashMap<String, String>, ) -> Result<bool, SkipReason>`

`struct RefFilter<'a>`
> Fields: `refs: Vec<&'a str>`, `variables: &'a [String]`, `has_changes: bool`

**`impl<'a> RefFilter<'a>`**
  `fn new( ref_lists: [&'a Option<Vec<String>>; 3], variables: &'a Option<Vec<String>>, has_changes: bool, ) -> Self`

  `fn from_only(only: &'a Only) -> Self`

  `fn from_except(except: &'a Except) -> Self`

  `fn from_refs(refs: &'a [String]) -> Self`

  `fn is_evaluated(&self) -> bool`

  `fn matches( &self, owner: &str, vars: &HashMap<String, String>, every_key: bool, ) -> Result<bool, SkipReason>`


`fn ref_pattern_matches(pattern: &str, vars: &HashMap<String, String>) -> Result<bool, String>`

`fn ref_filter_exclusion( owner: &str, job: &Job, variables: &HashMap<String, String>, ) -> Option<SkipReason>`

`fn warn_about_ignored_clauses(owner: &str, rule: &Rule)`

`enum RuleToken`
> Variants: `OpenParen`, `CloseParen`, `And`, `Or`, `Equal`, `NotEqual`, `Match`, `NotMatch`, `Null`, `Variable`, `Text`, `Regex`

`fn tokenize_rule_condition(condition: &str) -> Result<Vec<RuleToken>, String>`

`fn compile_rule_regex(pattern: &str) -> Result<regex::Regex, String>`

`struct RuleEvaluator<'a>`
> Fields: `tokens: Vec<RuleToken>`, `position: usize`, `variables: &'a HashMap<String, String>`

**`impl RuleEvaluator<'_>`**
  `fn peek(&self) -> Option<&RuleToken>`

  `fn accept(&mut self, token: &RuleToken) -> bool`

  `fn any_of(&mut self) -> Result<bool, String>`

  `fn all_of(&mut self) -> Result<bool, String>`

  `fn term(&mut self) -> Result<bool, String>`

  `fn operand(&mut self) -> Result<Option<String>, String>`

  `fn pattern(&mut self) -> Result<Option<String>, String>`


`fn evaluate_rule_condition( condition: &str, variables: &HashMap<String, String>, ) -> Result<bool, String>`

`fn expand_variables(text: &str, variables: &HashMap<String, String>) -> String`

`fn default_list(pipeline: &Pipeline, key: &str) -> Option<Vec<String>>`

`fn job_image(job_name: &str, pipeline: &Pipeline, job: &Job) -> Option<String>`

`const MAX_PARALLEL_INSTANCES: usize = 200`

`fn check_parallel(pipeline: &Pipeline) -> Result<(), String>`

`fn matrix_issues(matrix: &[HashMap<String, serde_yaml::Value>]) -> Vec<String>`

`fn is_variable_name(name: &str) -> bool`

`fn apply_parallel(job: &mut workflow::Job, parallel: &Parallel)`

`fn expand_parallel_entry( entry: &HashMap<String, serde_yaml::Value>, ) -> Vec<HashMap<String, serde_yaml::Value>>`

`mod tests`

---

## crates/parser/src/gitlab_config.rs

**Language:** Rust | **Size:** 31.3 KB | **Lines:** 968

**Imports:**
- `regex::Regex`
- `serde::Deserialize`
- `serde_yaml::{Mapping, Value}`
- `std::collections::HashMap`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::sync::OnceLock`

**Declarations:**

`const MAX_INCLUDES: usize = 150`

`const MAX_EXTENDS_DEPTH: usize = 11`

`const MAX_REFERENCE_DEPTH: usize = 10`

`const SCRIPT_KEYS: [&str`

`const GLOBAL_KEYWORDS: [&str`

`struct Loader`
> Fields: `root: PathBuf`, `stack: Vec<PathBuf>`, `included: usize`

**`impl Loader`**
  `fn load_file(&mut self, path: &Path, inputs: Option<&Mapping>) -> Result<Value, String>`

  `fn resolve_include(&self, entry: &Value) -> Result<Vec<(PathBuf, Option<Mapping>)>, String>`

  `fn local_files(&self, pattern: &str) -> Result<Vec<PathBuf>, String>`


`fn skip_include(kind: &str, target: &str) -> Vec<(PathBuf, Option<Mapping>)>`

`fn include_entries(include: Value) -> Vec<Value>`

`fn split_documents(content: &str) -> Result<(Option<Value>, Value), String>`

`fn resolve_inputs( spec: &Value, provided: Option<&Mapping>, ) -> Result<HashMap<String, Value>, String>`

`fn interpolation_pattern() -> &'static Regex`

`fn interpolate(value: &mut Value, inputs: &HashMap<String, Value>) -> Result<(), String>`

`fn interpolate_string( text: &str, inputs: &HashMap<String, Value>, ) -> Result<Option<Value>, String>`

`fn evaluate_expression(expression: &str, inputs: &HashMap<String, Value>) -> Result<Value, String>`

`fn apply_function(function: &str, text: String) -> Result<String, String>`

`fn posix_escape(text: &str) -> String`

`fn display_value(value: &Value) -> String`

`fn key_name(key: &Value) -> String`

`fn deep_merge(base: &mut Value, overlay: Value)`

`fn is_job_key(name: &str) -> bool`

`fn resolve_extends(config: &mut Value) -> Result<(), String>`

`fn resolve_job( name: &str, map: &Mapping, resolved: &mut HashMap<String, Value>, chain: &mut Vec<String>, ) -> Result<Value, String>`

`fn resolve_references(config: &mut Value) -> Result<(), String>`

`fn resolve_references_in(value: &mut Value, root: &Value, depth: usize) -> Result<(), String>`

`fn finalize(config: &mut Value)`

`fn flatten_scripts(map: &mut Mapping)`

`fn flatten_into(items: Vec<Value>, out: &mut Vec<Value>)`

`mod tests`

---

## crates/parser/src/lib.rs

**Language:** Rust | **Size:** 86 B | **Lines:** 6

**Declarations:**

`mod gitlab_config`

---

## crates/parser/src/schema.rs

**Language:** Rust | **Size:** 4.3 KB | **Lines:** 125

**Imports:**
- `jsonschema::JSONSchema`
- `serde_json::Value`
- `std::fs`
- `std::path::Path`

**Declarations:**

`const GITHUB_WORKFLOW_SCHEMA: &str = include_str!("github-workflow.json")`

`const GITLAB_CI_SCHEMA: &str = include_str!("gitlab-ci.json")`

**`impl SchemaValidator`**
  `pub fn new() -> Result<Self, String>`

  `pub fn validate_workflow(&self, workflow_path: &Path) -> Result<(), String>`

  `pub fn validate_with_specific_schema( &self, content: &str, schema_type: SchemaType, ) -> Result<(), String>`

  `pub fn validate_json( &self, workflow_json: &Value, schema_type: SchemaType, ) -> Result<(), String>`


---

## crates/parser/src/workflow.rs

**Language:** Rust | **Size:** 38.7 KB | **Lines:** 1223

**Imports:**
- `serde::{Deserialize, Deserializer, Serialize, Serializer}`
- `std::collections::HashMap`
- `std::fs`
- `std::path::Path`
- `wrkflw_matrix::MatrixConfig`
- `super::schema::SchemaValidator`

**Declarations:**

`fn deserialize_needs<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error> where D: Deserializer<'de>,`

`fn deserialize_runs_on<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error> where D: Deserializer<'de>,`

`fn deserialize_container<'de, D>(deserializer: D) -> Result<Option<JobContainer>, D::Error> where D: Deserializer<'de>,`

**`impl serde::Serialize for ContainerCredentials`**
  `fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: serde::Serializer,`


**`impl std::fmt::Debug for ContainerCredentials`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


`fn deserialize_matrix<'de, D>(deserializer: D) -> Result<Option<MatrixConfig>, D::Error> where D: Deserializer<'de>,`

`fn serialize_matrix<S>(matrix: &Option<MatrixConfig>, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer,`

`fn deserialize_bool_or_expression<'de, D>(deserializer: D) -> Result<Option<String>, D::Error> where D: Deserializer<'de>,`

`fn deserialize_timeout_minutes<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error> where D: Deserializer<'de>,`

`fn serialize_bool_or_expression<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer,`

**`impl Job`**
  `pub fn matrix_config(&self) -> Option<&MatrixConfig>`

  `pub fn fail_fast(&self) -> bool`

  `pub fn max_parallel(&self) -> Option<usize>`


**`impl Step`**
  `pub fn with_run(name: impl Into<String>, run: impl Into<String>) -> Self`


**`impl WorkflowDefinition`**
  `pub fn resolve_action(&self, action_ref: &str) -> ActionInfo`


`fn name_from_path(path: &Path) -> String`

`fn normalize_triggers(on_value: &serde_yaml::Value) -> Result<Vec<String>, String>`

`mod tests`

---

## crates/runs/Cargo.toml

**Language:** TOML | **Size:** 905 B | **Lines:** 33

**Declarations:**

---

## crates/runs/README.md

**Language:** Markdown | **Size:** 1.2 KB | **Lines:** 29

**Declarations:**

---

## crates/runs/src/digest.rs

**Language:** Rust | **Size:** 48.9 KB | **Lines:** 1234

**Imports:**
- `crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION}`
- `crate::store::RunStore`
- `serde::Serialize`
- `std::collections::BTreeSet`
- `wrkflw_executor::{EXIT_CODE_LABEL, STDERR_HEADER, STDOUT_HEADER}`

**Declarations:**

`const MAX_ERROR_LINES: usize = 10`

`static ERROR_WORD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`static ZERO_COUNT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\b(0|no)\s*$").unwrap())`

`static ANSI: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`static OUT_OF_MEMORY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`static KILLED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`const SIGKILL_EXIT: i32 = 137`

`fn is_error_line(line: &str) -> bool`

`static PASSING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`static ERROR_PREFIX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`static FAILING: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\b(FAIL(ED|URE)?|ERROR|Failed)\b").unwrap())`

`static BRACKETED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\[[^\]]*\]|\([^)]*\)").unwrap())`

`fn reports_pass(line: &str) -> bool`

`fn cause_lines(text: &str, tail_lines: usize) -> Vec<String>`

`fn failure_cause(text: &str, exit_code: Option<i32>, tail_lines: usize) -> Option<FailureCause>`

`const SETUP_EVIDENCE_CHARS: usize = 240`

`static SETUP_ERROR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(||`

`fn setup_failure(text: &str) -> Option<FailureCause>`

`fn tail(text: &str, lines: usize) -> (Vec<String>, usize)`

`fn is_output_header(line: &str) -> bool`

`fn output_start<S: AsRef<str>>(lines: &[S]) -> usize`

`fn error_lines(text: &str, tail_lines: usize) -> Vec<String>`

`fn without_durations(line: &str) -> String`

`static PROGRESS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\[\s*\d+%\]").unwrap())`

`fn names_a_location(line: &str) -> bool`

`fn failure_signature(text: &str) -> BTreeSet<String>`

`fn failure_context(text: &str) -> BTreeSet<String>`

`static DIGITS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\d+").unwrap())`

`fn failure_lines(text: &str) -> Vec<String>`

`fn same_failure(a: &FailedStep, b: &FailedStep) -> bool`

`fn failed_steps(store: &RunStore, run: &RunRecord, tail_lines: usize) -> Vec<FailedStep>`

`fn failed_step( store: &RunStore, run: &RunRecord, job: &JobRecord, tail_lines: usize, ) -> FailedStep`

`mod tests`

---

## crates/runs/src/env.rs

**Language:** Rust | **Size:** 2.3 KB | **Lines:** 44

**Declarations:**

---

## crates/runs/src/lib.rs

**Language:** Rust | **Size:** 689 B | **Lines:** 18

**Imports:**
- `pub use digest::{failure_digest, render_digest, FailedStep, FailureDigest, DEFAULT_TAIL_LINES}`
- `pub use record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION}`
- `pub use recorder::{
    detect_agent, detect_session, emit_line, execute_recorded, mark_interrupted, record_skipped,
    repo_root_of, RecordedRun, RunMeta, RunOptions,
}`
- `pub use store::{history_disabled, home_dir, RunStore, DEFAULT_KEEP}`

**Declarations:**

`mod digest`

`mod record`

`mod recorder`

`mod store`

---

## crates/runs/src/record.rs

**Language:** Rust | **Size:** 4.9 KB | **Lines:** 141

**Imports:**
- `serde::{Deserialize, Serialize}`
- `wrkflw_executor::{RunnerSubstitution, UnexecutedAction}`

**Declarations:**

**`impl RunStatus`**
  `pub fn as_str(self) -> &'static str`


`mod tests`

---

## crates/runs/src/recorder.rs

**Language:** Rust | **Size:** 45.8 KB | **Lines:** 1290

**Imports:**
- `crate::digest::{failure_digest, FailureDigest, DEFAULT_TAIL_LINES}`
- `crate::env`
- `crate::record::{JobRecord, RunRecord, RunStatus, StepRecord, SCHEMA_VERSION}`
- `crate::store::{history_disabled, RunStore, DEFAULT_KEEP}`
- `serde_json::{json, Value}`
- `std::io::Write`
- `std::path::Path`
- `std::sync::{Arc, Mutex}`
- `std::time::{Duration, Instant}`
- `wrkflw_executor::engine::{ExecutionError, ExecutionResult}`
- *... and 1 more imports*

**Declarations:**

`const MAX_EVENT_OUTPUT_BYTES: usize = 64 * 1024`

**`impl RunMeta`**
  `pub fn new(source: &str) -> Self`


`fn non_empty_env(name: &str) -> Option<String>`

`fn agent_from(get: impl Fn(&str) -> Option<String>) -> Option<String>`

`fn session_from(get: impl Fn(&str) -> Option<String>) -> Option<String>`

**`impl std::fmt::Debug for RunOptions`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


`struct GitInfo`
> Fields: `root: Option<String>`, `sha: Option<String>`, `branch: Option<String>`, `dirty: Option<bool>`

`async fn git(cwd: &Path, args: &[&str]) -> Option<String>`

`async fn git_info(cwd: &Path) -> GitInfo`

`fn new_run_id() -> String`

`fn slug(name: &str) -> String`

`fn job_dir(name: &str) -> String`

`fn tail_bytes(text: &str, max: usize) -> (&str, bool)`

`struct Recorder`
> Fields: `store: RunStore`, `persistent: bool`, `_scratch: Option<tempfile::TempDir>`, `state: Mutex<RunRecord>`, `ndjson: bool`, `stream_output: bool`, `started: Instant`

**`impl Recorder`**
  `fn persist(&self, record: &RunRecord)`

  `fn write_log(&self, id: &str, relative: &str, text: &str) -> Option<String>`

  `fn line(&self, mut value: Value)`

  `fn job_mut<'a>(record: &'a mut RunRecord, job: &str, canonical: &str) -> &'a mut JobRecord`

  `fn reconcile(&self, record: &mut RunRecord, jobs: &[JobResult])`

  `fn finish( &self, status: RunStatus, error: Option<String>, skip_reason: Option<String>, jobs: &[JobResult], ) -> RunRecord`


**`impl ExecutionObserver for Recorder`**
  `fn wants_step_output(&self) -> bool`

  `fn on_event(&self, event: &ExecutionEvent)`


`fn event_json(event: &ExecutionEvent) -> Option<Value>`

`async fn begin(path: &Path, config: &ExecutionConfig, options: &RunOptions) -> Arc<Recorder>`

`fn keep_limit() -> usize`

`fn run_finished_json(record: &RunRecord, digest: Option<&FailureDigest>) -> Value`

`fn job_ran(job: &JobRecord) -> bool`

`fn unverified(record: &RunRecord) -> Vec<Value>`

`static INTERRUPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false)`

`fn interrupted() -> bool`

`fn final_status( result: &Result<ExecutionResult, ExecutionError>, interrupted: bool, ) -> (RunStatus, Option<String>, &[JobResult])`

`fn conclude( recorder: Arc<Recorder>, record: RunRecord, result: Result<ExecutionResult, ExecutionError>, ) -> RecordedRun`

`mod tests`

---

## crates/runs/src/store.rs

**Language:** Rust | **Size:** 10.8 KB | **Lines:** 321

**Imports:**
- `crate::env`
- `crate::record::{RunRecord, RunStatus}`
- `std::fs`
- `std::io::{self, Write}`
- `std::path::{Path, PathBuf}`

**Declarations:**

**`impl RunStore`**
  `pub fn open_default() -> Option<Self>`

  `pub fn at(root: impl Into<PathBuf>) -> Self`

  `pub fn root(&self) -> &Path`

  `pub fn run_dir(&self, id: &str) -> PathBuf`

  `pub fn list(&self) -> Vec<RunRecord>`

  `fn ids(&self) -> Vec<String>`

  `pub fn load(&self, id: &str) -> Result<RunRecord, String>`

  `pub fn resolve(&self, reference: &str, repo_root: &str) -> Result<RunRecord, String>`

  `pub fn save(&self, record: &RunRecord) -> io::Result<()>`

  `pub fn write_log(&self, id: &str, relative: &str, text: &str) -> io::Result<()>`

  `pub fn read_log(&self, id: &str, relative: &str) -> io::Result<String>`

  `pub fn prune(&self, keep: usize) -> usize`


`fn create_private_dir(path: &Path) -> io::Result<()>`

`fn create_private_dir(path: &Path) -> io::Result<()>`

`fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()>`

`fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()>`

`fn process_alive(pid: u32) -> bool`

`fn process_alive(pid: u32) -> bool`

`mod tests`

---

## crates/runtime/Cargo.toml

**Language:** TOML | **Size:** 765 B | **Lines:** 31

**Imports:**
- `ignore`

**Declarations:**

---

## crates/runtime/README.md

**Language:** Markdown | **Size:** 554 B | **Lines:** 17

**Declarations:**

---

## crates/runtime/src/container.rs

**Language:** Rust | **Size:** 18.0 KB | **Lines:** 476

**Imports:**
- `async_trait::async_trait`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `wrkflw_logging`
- `std::fmt`

**Declarations:**

`pub(crate) fn resolve_host_working_dir( container_dir: &Path, volumes: &[(&Path, &Path)], ) -> Option<PathBuf>`

`pub(crate) fn rebase_working_dir_or_error( working_dir: &Path, volumes: &[(&Path, &Path)], runtime_label: &str, ) -> Result<PathBuf, ContainerError>`

**`impl fmt::Display for ContainerError`**
  `fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`


`mod tests`

---

## crates/runtime/src/emulation.rs

**Language:** Rust | **Size:** 33.2 KB | **Lines:** 900

**Imports:**
- `crate::container::{
    rebase_working_dir_or_error, ContainerError, ContainerOutput, ContainerRuntime,
}`
- `async_trait::async_trait`
- `once_cell::sync::Lazy`
- `std::collections::HashMap`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::process::Command`
- `std::sync::Mutex`
- `tempfile::TempDir`
- `which`
- *... and 2 more imports*

**Declarations:**

`static EMULATION_WORKSPACES: Lazy<Mutex<Vec<PathBuf>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static EMULATION_PROCESSES: Lazy<Mutex<Vec<u32>>> = Lazy::new(|| Mutex::new(Vec::new()))`

**`impl Default for EmulationRuntime`**
  `fn default() -> Self`


**`impl EmulationRuntime`**
  `pub fn new() -> Self`

  `fn prepare_workspace(&self, _working_dir: &Path, volumes: &[(&Path, &Path)]) -> PathBuf`


**`impl ContainerRuntime for EmulationRuntime`**
  `async fn run_container( &self, _image: &str, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], _entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, _context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, _additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`


`fn create_gitignore_matcher( dir: &Path, ) -> Result<Option<ignore::gitignore::Gitignore>, std::io::Error>`

`fn copy_directory_contents(source: &Path, dest: &Path) -> std::io::Result<()>`

`fn copy_directory_contents_with_gitignore( source: &Path, dest: &Path, gitignore: Option<&ignore::gitignore::Gitignore>, ) -> std::io::Result<()>`

`fn check_command_available(command: &str, name: &str, install_url: &str)`

`fn add_action_env_vars( env_map: &mut HashMap<String, String>, action: &str, with_params: &Option<HashMap<String, String>>, )`

`async fn cleanup_processes()`

`async fn cleanup_workspaces()`

`mod tests`

---

## crates/runtime/src/emulation_test.rs

**Language:** Rust | **Size:** 8.7 KB | **Lines:** 241

**Imports:**
- `std::path::{Path, PathBuf}`
- `std::process::Command`
- `std::fs`
- `tokio::sync::Mutex`
- `once_cell::sync::Lazy`
- `crate::runtime::{
    container::{ContainerRuntime, ContainerOutput, ContainerError},
    emulation::{self, EmulationRuntime},
}`

**Declarations:**

`mod emulation_cleanup_tests`

---

## crates/runtime/src/lib.rs

**Language:** Rust | **Size:** 120 B | **Lines:** 7

**Declarations:**

---

## crates/runtime/src/live_output.rs

**Language:** Rust | **Size:** 10.1 KB | **Lines:** 299

**Imports:**
- `std::future::Future`
- `std::sync::Arc`
- `tokio::io::AsyncReadExt`

**Declarations:**

`type TapFn = dyn Fn(Stream, &str) + Send + Sync`

**`impl OutputTap`**
  `pub fn new(f: impl Fn(Stream, &str) + Send + Sync + 'static) -> Self`

  `pub fn send(&self, stream: Stream, chunk: &str)`


**`impl Utf8Chunks`**
  `pub fn push(&mut self, bytes: &[u8]) -> String`

  `pub fn finish(&mut self) -> String`


`fn incomplete_tail(bytes: &[u8]) -> usize`

`async fn read_live<R: tokio::io::AsyncRead + Unpin>( pipe: Option<R>, stream: Stream, tap: &OutputTap, ) -> std::io::Result<Vec<u8>>`

`mod tests`

---

## crates/runtime/src/sandbox.rs

**Language:** Rust | **Size:** 20.1 KB | **Lines:** 548

**Imports:**
- `regex::Regex`
- `std::collections::HashSet`
- `std::path::Path`
- `std::process::{Command, Stdio}`
- `std::time::Duration`
- `wrkflw_logging`

**Declarations:**

**`impl Default for SandboxConfig`**
  `fn default() -> Self`


**`impl Sandbox`**
  `pub fn new(config: SandboxConfig) -> Result<Self, SandboxError>`

  `pub async fn execute_command( &self, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, ) -> Result<crate::container::ContainerOutput, SandboxError>`

  `fn validate_command(&self, command_str: &str) -> Result<(), SandboxError>`

  `fn split_shell_command(&self, command_str: &str) -> Vec<String>`

  `fn is_shell_builtin(&self, command: &str) -> bool`

  `async fn execute_with_limits( &self, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, ) -> Result<crate::container::ContainerOutput, SandboxError>`

  `fn is_env_var_safe(&self, key: &str) -> bool`

  `fn compile_dangerous_patterns() -> Vec<Regex>`


`mod tests`

---

## crates/runtime/src/secure_emulation.rs

**Language:** Rust | **Size:** 17.4 KB | **Lines:** 477

**Imports:**
- `crate::container::{
    rebase_working_dir_or_error, ContainerError, ContainerOutput, ContainerRuntime,
}`
- `crate::sandbox::{create_workflow_sandbox_config, Sandbox, SandboxConfig, SandboxError}`
- `async_trait::async_trait`
- `std::path::Path`
- `wrkflw_logging`

**Declarations:**

**`impl Default for SecureEmulationRuntime`**
  `fn default() -> Self`


**`impl SecureEmulationRuntime`**
  `pub fn new() -> Self`

  `pub fn new_with_config(config: SandboxConfig) -> Result<Self, ContainerError>`


**`impl ContainerRuntime for SecureEmulationRuntime`**
  `fn flattens_commands(&self) -> bool`

  `async fn run_container( &self, image: &str, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, _context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, _additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`


`fn check_command_available_secure(command: &str, name: &str, install_url: &str)`

`mod tests`

---

## crates/secrets/Cargo.toml

**Language:** TOML | **Size:** 1.7 KB | **Lines:** 62

**Imports:**
- `chrono`
- `anyhow`
- `base64`
- `aes-gcm`
- `rand`
- `tracing`
- `url`
- `pbkdf2`
- `hmac`
- `sha2`
- *... and 2 more imports*

**Declarations:**

---

## crates/secrets/README.md

**Language:** Markdown | **Size:** 2.2 KB | **Lines:** 76

**Declarations:**

---

## crates/secrets/benches/masking_bench.rs

**Language:** Rust | **Size:** 2.8 KB | **Lines:** 92

**Imports:**
- `criterion::{black_box, criterion_group, criterion_main, Criterion}`
- `wrkflw_secrets::SecretMasker`

**Declarations:**

`fn bench_basic_masking(c: &mut Criterion)`

`fn bench_pattern_masking(c: &mut Criterion)`

`fn bench_large_text_masking(c: &mut Criterion)`

`fn bench_many_secrets(c: &mut Criterion)`

`fn bench_contains_secrets(c: &mut Criterion)`

---

## crates/secrets/src/config.rs

**Language:** Rust | **Size:** 8.8 KB | **Lines:** 285

**Imports:**
- `crate::rate_limit::RateLimitConfig`
- `serde::{Deserialize, Serialize}`
- `std::collections::HashMap`
- `std::path::PathBuf`

**Declarations:**

**`impl Default for SecretConfig`**
  `fn default() -> Self`


`fn default_secrets_path() -> String`

`fn resolve_secrets_path(config_dir: Option<PathBuf>, home_dir: Option<PathBuf>) -> String`

**`impl SecretConfig`**
  `pub fn from_file(path: &str) -> crate::SecretResult<Self>`

  `pub fn to_file(&self, path: &str) -> crate::SecretResult<()>`

  `pub fn from_env() -> Self`


`mod path_tests`

---

## crates/secrets/src/error.rs

**Language:** Rust | **Size:** 2.4 KB | **Lines:** 88

**Imports:**
- `thiserror::Error`

**Declarations:**

**`impl SecretError`**
  `pub fn not_found(name: impl Into<String>) -> Self`

  `pub fn provider_not_found(provider: impl Into<String>) -> Self`

  `pub fn auth_failed(provider: impl Into<String>, reason: impl Into<String>) -> Self`

  `pub fn invalid_config(msg: impl Into<String>) -> Self`

  `pub fn internal(msg: impl Into<String>) -> Self`


---

## crates/secrets/src/lib.rs

**Language:** Rust | **Size:** 7.4 KB | **Lines:** 246

**Imports:**
- `pub use config::{SecretConfig, SecretProviderConfig}`
- `pub use error::{SecretError, SecretResult}`
- `pub use manager::SecretManager`
- `pub use masking::SecretMasker`
- `pub use providers::{SecretProvider, SecretValue}`
- `pub use substitution::SecretSubstitution`

**Declarations:**

`mod tests`

---

## crates/secrets/src/manager.rs

**Language:** Rust | **Size:** 8.6 KB | **Lines:** 267

**Imports:**
- `crate::{
    config::{SecretConfig, SecretProviderConfig},
    providers::{env::EnvironmentProvider, file::FileProvider, SecretProvider, SecretValue},
    rate_limit::RateLimiter,
    validation::{validate_provider_name, validate_secret_name},
    SecretError, SecretResult,
}`
- `std::collections::HashMap`
- `std::sync::Arc`
- `tokio::sync::RwLock`

**Declarations:**

`struct CachedSecret`
> Fields: `value: SecretValue`, `expires_at: chrono::DateTime<chrono::Utc>`

**`impl SecretManager`**
  `pub async fn new(config: SecretConfig) -> SecretResult<Self>`

  `pub async fn default() -> SecretResult<Self>`

  `pub async fn get_secret(&self, name: &str) -> SecretResult<SecretValue>`

  `pub async fn get_secret_from_provider( &self, provider_name: &str, name: &str, ) -> SecretResult<SecretValue>`

  `pub async fn list_all_secrets(&self) -> SecretResult<HashMap<String, Vec<String>>>`

  `pub async fn health_check(&self) -> HashMap<String, SecretResult<()>>`

  `pub async fn clear_cache(&self)`

  `pub fn config(&self) -> &SecretConfig`

  `pub fn has_provider(&self, name: &str) -> bool`

  `pub fn provider_names(&self) -> Vec<String>`


`mod tests`

---

## crates/secrets/src/masking.rs

**Language:** Rust | **Size:** 14.8 KB | **Lines:** 448

**Imports:**
- `regex::Regex`
- `std::collections::{HashMap, HashSet}`
- `std::sync::{Arc, OnceLock, RwLock}`

**Declarations:**

`struct CompiledPatterns`
> Fields: `github_pat: Regex`, `github_app: Regex`, `github_oauth: Regex`, `aws_access_key: Regex`, `aws_secret: Regex`, `jwt: Regex`, `api_key: Regex`

**`impl CompiledPatterns`**
  `fn new() -> Self`


`static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new()`

`struct SecretData`
> Fields: `secrets: HashSet<String>`, `secret_cache: HashMap<String, String>`, `sorted_pairs: Option<Arc<Vec<(String, String)>>>`

**`impl SecretMasker`**
  `pub fn new() -> Self`

  `pub fn with_mask_char(mask_char: char) -> Self`

  `pub fn add_secret(&self, secret: impl Into<String>)`

  `pub fn add_secrets(&self, secrets: impl IntoIterator<Item = String>)`

  `pub fn remove_secret(&self, secret: &str)`

  `pub fn clear(&self)`

  `pub fn mask(&self, text: &str) -> String`

  `fn create_mask(&self, _secret: &str) -> String`

  `fn mask_patterns(&self, text: &str) -> String`

  `pub fn contains_secrets(&self, text: &str) -> bool`

  `fn has_secret_patterns(&self, text: &str) -> bool`

  `pub fn snapshot(&self) -> SecretMasker`

  `pub fn secret_count(&self) -> usize`

  `pub fn has_secret(&self, secret: &str) -> bool`


**`impl Default for SecretMasker`**
  `fn default() -> Self`


`mod tests`

---

## crates/secrets/src/providers/env.rs

**Language:** Rust | **Size:** 4.3 KB | **Lines:** 143

**Imports:**
- `crate::{
    validation::validate_secret_value, SecretError, SecretProvider, SecretResult, SecretValue,
}`
- `async_trait::async_trait`
- `std::collections::HashMap`

**Declarations:**

**`impl EnvironmentProvider`**
  `pub fn new(prefix: Option<String>) -> Self`


**`impl Default for EnvironmentProvider`**
  `fn default() -> Self`


**`impl EnvironmentProvider`**
  `fn get_env_name(&self, name: &str) -> String`


**`impl SecretProvider for EnvironmentProvider`**
  `async fn get_secret(&self, name: &str) -> SecretResult<SecretValue>`

  `async fn list_secrets(&self) -> SecretResult<Vec<String>>`

  `fn name(&self) -> &str`


`mod tests`

---

## crates/secrets/src/providers/file.rs

**Language:** Rust | **Size:** 9.4 KB | **Lines:** 288

**Imports:**
- `crate::{
    validation::validate_secret_value, SecretError, SecretProvider, SecretResult, SecretValue,
}`
- `async_trait::async_trait`
- `serde_json::Value`
- `std::collections::HashMap`
- `std::path::Path`

**Declarations:**

**`impl FileProvider`**
  `pub fn new(path: impl Into<String>) -> Self`

  `fn expand_path(&self) -> String`

  `async fn load_json_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>>`

  `async fn load_yaml_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>>`

  `async fn load_env_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>>`

  `async fn load_secrets(&self) -> SecretResult<HashMap<String, String>>`


**`impl SecretProvider for FileProvider`**
  `async fn get_secret(&self, name: &str) -> SecretResult<SecretValue>`

  `async fn list_secrets(&self) -> SecretResult<Vec<String>>`

  `fn name(&self) -> &str`


`mod tests`

---

## crates/secrets/src/providers/mod.rs

**Language:** Rust | **Size:** 2.6 KB | **Lines:** 91

**Imports:**
- `crate::{SecretError, SecretResult}`
- `async_trait::async_trait`
- `serde::{Deserialize, Serialize}`
- `std::collections::HashMap`

**Declarations:**

**`impl SecretValue`**
  `pub fn new(value: impl Into<String>) -> Self`

  `pub fn with_metadata(value: impl Into<String>, metadata: HashMap<String, String>) -> Self`

  `pub fn value(&self) -> &str`

  `pub fn is_expired(&self, ttl_seconds: u64) -> bool`


---

## crates/secrets/src/rate_limit.rs

**Language:** Rust | **Size:** 7.1 KB | **Lines:** 242

**Imports:**
- `crate::{SecretError, SecretResult}`
- `std::collections::HashMap`
- `std::sync::Arc`
- `std::time::{Duration, Instant}`
- `tokio::sync::RwLock`

**Declarations:**

**`impl Default for RateLimitConfig`**
  `fn default() -> Self`


`struct RequestTracker`
> Fields: `requests: Vec<Instant>`, `first_request: Instant`

**`impl RequestTracker`**
  `fn new() -> Self`

  `fn add_request(&mut self, now: Instant)`

  `fn cleanup_old_requests(&mut self, window_duration: Duration, now: Instant)`

  `fn request_count(&self) -> usize`


**`impl RateLimiter`**
  `pub fn new(config: RateLimitConfig) -> Self`

  `pub async fn check_rate_limit(&self, key: &str) -> SecretResult<()>`

  `pub async fn reset_rate_limit(&self, key: &str)`

  `pub async fn clear_all(&self)`

  `pub async fn get_request_count(&self, key: &str) -> usize`

  `pub fn config(&self) -> &RateLimitConfig`


**`impl Default for RateLimiter`**
  `fn default() -> Self`


`mod tests`

---

## crates/secrets/src/storage.rs

**Language:** Rust | **Size:** 13.0 KB | **Lines:** 394

**Imports:**
- `crate::{SecretError, SecretResult}`
- `aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
}`
- `base64::{engine::general_purpose, Engine as _}`
- `serde::{Deserialize, Serialize}`
- `std::collections::HashMap`

**Declarations:**

**`impl EncryptedSecretStore`**
  `pub fn new() -> SecretResult<(Self, [u8; 32])>`

  `pub fn from_data(secrets: HashMap<String, String>, salt: String) -> Self`

  `pub fn add_secret(&mut self, key: &[u8; 32], name: &str, value: &str) -> SecretResult<()>`

  `pub fn get_secret(&self, key: &[u8; 32], name: &str) -> SecretResult<String>`

  `pub fn remove_secret(&mut self, name: &str) -> bool`

  `pub fn list_secrets(&self) -> Vec<String>`

  `pub fn has_secret(&self, name: &str) -> bool`

  `pub fn secret_count(&self) -> usize`

  `pub fn clear(&mut self)`

  `fn encrypt_value(key: &[u8; 32], value: &str) -> SecretResult<String>`

  `fn decrypt_value(key: &[u8; 32], encrypted: &str) -> SecretResult<String>`

  `fn generate_salt() -> [u8; 32]`

  `fn generate_nonce() -> [u8; 12]`

  `pub fn to_json(&self) -> SecretResult<String>`

  `pub fn from_json(json: &str) -> SecretResult<Self>`

  `pub async fn save_to_file(&self, path: &str) -> SecretResult<()>`

  `pub async fn load_from_file(path: &str) -> SecretResult<Self>`


**`impl Default for EncryptedSecretStore`**
  `fn default() -> Self`


**`impl KeyDerivation`**
  `pub fn derive_key_from_password(password: &str, salt: &[u8], iterations: u32) -> [u8; 32]`

  `pub fn generate_random_key() -> [u8; 32]`


`mod tests`

---

## crates/secrets/src/substitution.rs

**Language:** Rust | **Size:** 11.3 KB | **Lines:** 320

**Imports:**
- `crate::{SecretError, SecretManager, SecretResult}`
- `regex::Regex`
- `std::collections::HashMap`

**Declarations:**

**`impl<'a> SecretSubstitution<'a>`**
  `pub fn new(manager: &'a SecretManager) -> Self`

  `pub fn allowing_missing(manager: &'a SecretManager) -> Self`

  `fn value_of(&self, lookup: SecretResult<crate::SecretValue>) -> SecretResult<String>`

  `pub async fn substitute(&mut self, text: &str) -> SecretResult<String>`

  `async fn substitute_provider_secrets(&mut self, text: &str) -> SecretResult<String>`

  `async fn substitute_default_secrets(&mut self, text: &str) -> SecretResult<String>`

  `pub fn resolved_secrets(&self) -> &HashMap<String, String>`

  `pub fn contains_secrets(text: &str) -> bool`

  `pub fn extract_secret_refs(text: &str) -> Vec<SecretRef>`


**`impl SecretRef`**
  `pub fn qualified_name(&self) -> String`

  `pub async fn lookup(&self, manager: &SecretManager) -> SecretResult<crate::SecretValue>`

  `pub fn cache_key(&self) -> String`


`mod tests`

---

## crates/secrets/src/validation.rs

**Language:** Rust | **Size:** 7.6 KB | **Lines:** 241

**Imports:**
- `crate::{SecretError, SecretResult}`
- `regex::Regex`

**Declarations:**

`mod tests`

---

## crates/secrets/tests/integration_tests.rs

**Language:** Rust | **Size:** 11.6 KB | **Lines:** 351

**Imports:**
- `std::collections::HashMap`
- `std::process`
- `tempfile::TempDir`
- `wrkflw_secrets::{
    SecretConfig, SecretManager, SecretMasker, SecretProviderConfig, SecretSubstitution,
}`

**Declarations:**

`async fn test_end_to_end_secret_workflow()`

`async fn test_error_handling()`

`async fn test_rate_limiting()`

`async fn test_concurrent_access()`

`async fn test_substitution_edge_cases()`

`async fn test_comprehensive_masking()`

---

## crates/trigger-filter/Cargo.toml

**Language:** TOML | **Size:** 583 B | **Lines:** 23

**Declarations:**

---

## crates/trigger-filter/README.md

**Language:** Markdown | **Size:** 858 B | **Lines:** 18

**Declarations:**

---

## crates/trigger-filter/src/config.rs

**Language:** Rust | **Size:** 5.3 KB | **Lines:** 136

**Imports:**
- `std::time::Duration`

**Declarations:**

**`impl Default for TriggerFilterConfig`**
  `fn default() -> Self`


**`impl TriggerFilterConfig`**
  `pub fn with_git_state_ttl(mut self, d: Duration) -> Self`

  `pub fn with_pattern_cache_size(mut self, n: usize) -> Self`

  `pub fn with_default_event(mut self, event: impl Into<String>) -> Self`


`mod tests`

---

## crates/trigger-filter/src/error.rs

**Language:** Rust | **Size:** 209 B | **Lines:** 10

**Imports:**
- `thiserror::Error`

**Declarations:**

---

## crates/trigger-filter/src/eval.rs

**Language:** Rust | **Size:** 34.2 KB | **Lines:** 936

**Imports:**
- `crate::model::{
    EventContext, EventFilter, GlobPattern, TriggerMatchResult, WorkflowTriggerConfig,
}`
- `crate::path_matcher`
- `crate::ref_matcher`

**Declarations:**

`fn branch_for_filter(context: &EventContext) -> Option<&String>`

`fn ref_filters_pass(filter: &EventFilter, context: &EventContext) -> bool`

`fn combined_pattern_sources(includes: &[GlobPattern], ignores: &[GlobPattern]) -> Vec<String>`

`fn explain_filter_failure(filter: &EventFilter, context: &EventContext) -> String`

`mod tests`

---

## crates/trigger-filter/src/git.rs

**Language:** Rust | **Size:** 51.2 KB | **Lines:** 1241

**Imports:**
- `crate::config::DEFAULT_GIT_COMMAND_TIMEOUT`
- `crate::error::TriggerFilterError`
- `std::path::Path`
- `std::time::Duration`
- `tokio::process::Command`

**Declarations:**

`const GIT_COMMAND_TIMEOUT: Duration = DEFAULT_GIT_COMMAND_TIMEOUT`

`fn git_cmd(cwd: Option<&Path>) -> Command`

`async fn run_git( mut cmd: Command, cmd_label: &str, ) -> Result<std::process::Output, TriggerFilterError>`

`fn parse_nul_separated(output: &[u8]) -> NulParseResult`

`struct NulParseResult`
> Fields: `files: Vec<String>`, `lossy_names: Vec<String>`

`fn merge_unique(mut into: Vec<String>, more: Vec<String>) -> Vec<String>`

`fn check_status( output: std::process::Output, cmd_label: &str, ) -> Result<std::process::Output, TriggerFilterError>`

`const FIND_REPO_ROOT_TIMEOUT: Duration = Duration::from_secs(5)`

**`impl std::fmt::Display for FindRepoRootError`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


**`impl std::error::Error for FindRepoRootError`**

`mod tests`

---

## crates/trigger-filter/src/lib.rs

**Language:** Rust | **Size:** 44.1 KB | **Lines:** 1025

**Imports:**
- `pub use config::TriggerFilterConfig`
- `pub use error::TriggerFilterError`
- `pub use eval::evaluate_trigger`
- `pub use git::{find_repo_root_detailed, head_mtime, FindRepoRootError}`
- `pub use model::{
    EventContext, EventFilter, GlobPattern, MustDrainWarnings, TriggerMatchResult,
    WorkflowTriggerConfig,
}`
- `pub use parser::parse_trigger_config`
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `std::sync::Mutex`
- `std::time::SystemTime`

**Declarations:**

`struct CachedTriggerConfig`
> Fields: `mtime: SystemTime`, `len: u64`, `config: WorkflowTriggerConfig`, `last_used: u64`

`static PATTERN_CACHE: Mutex<Option<PatternCache>> = Mutex::new(None)`

`struct PatternCache`
> Fields: `capacity: usize`, `tick: u64`, `entries: HashMap<PathBuf, CachedTriggerConfig>`

**`impl PatternCache`**
  `fn new(capacity: usize) -> Self`

  `fn evict_lru(&mut self)`


`mod tests`

---

## crates/trigger-filter/src/model.rs

**Language:** Rust | **Size:** 11.9 KB | **Lines:** 309

**Imports:**
- `glob::{MatchOptions, Pattern, PatternError}`
- `std::path::PathBuf`

**Declarations:**

**`impl MustDrainWarnings`**
  `pub fn new() -> Self`

  `pub fn push(&mut self, warning: String)`

  `pub fn extend<I: IntoIterator<Item = String>>(&mut self, iter: I)`

  `pub fn is_empty(&self) -> bool`

  `pub fn len(&self) -> usize`

  `pub fn iter(&self) -> std::slice::Iter<'_, String>`

  `pub fn take(&mut self) -> Vec<String>`


**`impl Clone for MustDrainWarnings`**
  `fn clone(&self) -> Self`


**`impl From<Vec<String>> for MustDrainWarnings`**
  `fn from(inner: Vec<String>) -> Self`


**`impl Drop for MustDrainWarnings`**
  `fn drop(&mut self)`


`mod tests`

**`impl GlobPattern`**
  `pub fn new(source: impl Into<String>) -> Result<Self, PatternError>`

  `pub fn match_options() -> MatchOptions`


---

## crates/trigger-filter/src/parser.rs

**Language:** Rust | **Size:** 34.6 KB | **Lines:** 969

**Imports:**
- `crate::error::TriggerFilterError`
- `crate::model::{EventFilter, GlobPattern, MustDrainWarnings, WorkflowTriggerConfig}`
- `std::path::PathBuf`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`const KNOWN_GHA_EVENTS: &[&str] = &[ "branch_protection_rule", "check_run", "check_suite", "create", "delete", "deployment", "deployment_status", "discussion", "discussion_comment", "fork", "gollum", "issue_comment", "issues", "label", "merge_group", "milestone", "page_build", "project", "project_card", "project_column", "public", "pull_request", "pull_request_review", "pull_request_review_comment", "pull_request_target", "push", "registry_package", "release", "repository_dispatch", "schedule", "status", "watch", "workflow_call", "workflow_dispatch", "workflow_run", ]`

`fn collect_unknown_event_warnings( events: &[EventFilter], workflow_path: &std::path::Path, ) -> Vec<String>`

`fn parse_events(on_raw: &serde_yaml::Value) -> Result<Vec<EventFilter>, TriggerFilterError>`

`fn parse_event_config( event_name: &str, value: &serde_yaml::Value, ) -> Result<EventFilter, TriggerFilterError>`

`fn resolve_include_and_ignore( map: &serde_yaml::Mapping, include_key: &str, ignore_key: &str, event_name: &str, ) -> Result<(Vec<GlobPattern>, Vec<GlobPattern>), TriggerFilterError>`

`fn extract_string_list( map: &serde_yaml::Mapping, key: &str, event_name: &str, ) -> Result<Vec<String>, TriggerFilterError>`

`fn yaml_kind(v: &serde_yaml::Value) -> &'static str`

`fn extract_glob_list( map: &serde_yaml::Mapping, key: &str, event_name: &str, ) -> Result<(Vec<GlobPattern>, Vec<GlobPattern>), TriggerFilterError>`

`mod tests`

---

## crates/trigger-filter/src/path_matcher.rs

**Language:** Rust | **Size:** 6.1 KB | **Lines:** 171

**Imports:**
- `crate::model::GlobPattern`

**Declarations:**

`fn matches_any_pattern(file: &str, patterns: &[GlobPattern]) -> bool`

`mod tests`

---

## crates/trigger-filter/src/ref_matcher.rs

**Language:** Rust | **Size:** 3.3 KB | **Lines:** 118

**Imports:**
- `crate::model::GlobPattern`

**Declarations:**

`mod tests`

---

## crates/ui/Cargo.toml

**Language:** TOML | **Size:** 1.5 KB | **Lines:** 57

**Imports:**
- `base64`
- `crossterm`
- `ratatui`
- `terminal-colorsaurus`
- `unicode-width`
- `reqwest`

**Declarations:**

---

## crates/ui/README.md

**Language:** Markdown | **Size:** 2.1 KB | **Lines:** 43

**Declarations:**

---

## crates/ui/src/app/mod.rs

**Language:** Rust | **Size:** 40.2 KB | **Lines:** 887

**Imports:**
- `crate::handlers::workflow::start_next_workflow_execution`
- `crate::models::{ExecutionResultMsg, QueuedExecution, Workflow, WorkflowStatus}`
- `crate::selection`
- `crate::utils::load_workflows`
- `crate::views::{
    render_ui, tab_of, TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUNS, TAB_SECRETS,
    TAB_TRIGGER, TAB_WORKFLOWS,
}`
- `crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
}`
- `ratatui::{backend::CrosstermBackend, Terminal}`
- `std::collections::HashMap`
- `std::io::{self, stdout}`
- `std::path::PathBuf`
- *... and 7 more imports*

**Declarations:**

`mod state`

`static TUI_ACTIVE: AtomicBool = AtomicBool::new(false)`

`static BACKGROUND_PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new())`

`fn install_panic_hook()`

`fn report_background_panics(app: &mut App)`

`fn run_tui_event_loop( terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, rx: &mpsc::Receiver<ExecutionResultMsg>, verbose: bool, ) -> io::Result<()>`

---

## crates/ui/src/app/state.rs

**Language:** Rust | **Size:** 208.4 KB | **Lines:** 5227

**Imports:**
- `crate::log_processor::{LogProcessingRequest, LogProcessor, ProcessedLogEntry}`
- `crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, QueuedExecution, StatusSeverity,
    StepExecution, TimelineMsg, TriggerMatchStatus, Workflow, WorkflowExecution, WorkflowStatus,
}`
- `crate::theme::{self, ThemeMode}`
- `chrono::Local`
- `crossterm::event::KeyCode`
- `ratatui::widgets::{ListState, TableState}`
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::mpsc`
- *... and 5 more imports*

**Declarations:**

`fn start_availability_probe( runtime: &RuntimeType, kubernetes: &wrkflw_executor::KubernetesConfig, ) -> Option<mpsc::Receiver<bool>>`

`const MAX_RUNS_SHOWN: usize = 200`

**`impl TriggerPlatform`**
  `pub fn as_str(&self) -> &'static str`

  `pub fn toggle(self) -> Self`


**`impl Accent`**
  `pub fn as_str(&self) -> &'static str`

  `pub fn next(self) -> Self`

  `pub fn rgb(self) -> (u8, u8, u8)`

  `pub fn rgb_for(self, mode: ThemeMode) -> (u8, u8, u8)`


**`impl App`**
  `pub fn new( runtime_type: RuntimeType, tx: mpsc::Sender<ExecutionResultMsg>, preserve_containers_on_failure: bool, show_action_messages: bool, platform_overrides: HashMap<String, String>, kubernetes: wrkflw_executor::KubernetesConfig, ) -> App`

  `pub fn toggle_selected(&mut self)`

  `pub fn toggle_emulation_mode(&mut self)`

  `pub fn cycle_diff_filter_event(&mut self)`

  `fn rerun_diff_filter_if_active(&mut self)`

  `fn abort_in_flight_evaluation(&mut self)`

  `fn spawn_evaluation(&mut self)`

  `pub fn toggle_diff_filter(&mut self)`

  `pub fn check_diff_filter_results(&mut self)`

  `pub fn toggle_validation_mode(&mut self)`

  `fn collect_availability_probe(&mut self)`

  `pub fn runtime_type_name(&self) -> &str`

  `pub fn previous_workflow(&mut self)`

  `pub fn next_workflow(&mut self)`

  `pub fn jump_workflow(&mut self, last: bool)`

  `pub fn shown_workflow(&self) -> Option<usize>`

  `pub fn jump_job(&mut self, last: bool)`

  `pub fn jump_step(&mut self, last: bool)`

  `fn job_rows(&self) -> usize`

  `pub fn previous_job(&mut self)`

  `pub fn next_job(&mut self)`

  `fn select_job_row(&mut self, row: usize)`

  `pub fn previous_step(&mut self)`

  `pub fn next_step(&mut self)`

  `pub fn switch_tab(&mut self, tab: usize)`

  `pub fn open_tab(&mut self, index: usize)`

  `pub fn refresh_runs(&mut self)`

  `pub fn match_runs_to_workflows(&mut self)`

  `pub fn last_run(&self, idx: usize) -> Option<&wrkflw_runs::RunRecord>`

  `pub fn select_next_run(&mut self)`

  `pub fn jump_run(&mut self, last: bool)`

  `pub fn select_previous_run(&mut self)`

  `fn queue_workflow(&mut self, idx: usize)`

  `pub fn queue_cursor_for_execution(&mut self)`

  `pub fn queue_selected_for_execution(&mut self)`

  `pub fn start_execution(&mut self)`

  `pub fn process_execution_result( &mut self, workflow_idx: usize, result: Result<(Vec<wrkflw_executor::JobResult>, ()), String>, )`

  `pub fn get_next_workflow_to_execute(&mut self) -> Option<(usize, Option<String>)>`

  `pub fn enter_job_selection_mode(&mut self)`

  `pub fn exit_job_selection_mode(&mut self)`

  `pub fn next_available_job(&mut self)`

  `pub fn previous_available_job(&mut self)`

  `pub fn run_from_job_selection(&mut self, target_job: Option<String>)`

  `pub fn toggle_detailed_view(&mut self)`

  `pub fn handle_log_search_input(&mut self, key: KeyCode)`

  `fn restart_log_search(&mut self)`

  `pub fn toggle_log_search(&mut self)`

  `pub fn toggle_log_filter(&mut self)`

  `pub fn clear_log_search_and_filter(&mut self)`

  `pub fn update_log_search_matches(&mut self)`

  `pub fn next_search_match(&mut self)`

  `pub fn previous_search_match(&mut self)`

  `pub fn scroll_logs_up(&mut self)`

  `pub fn scroll_logs_down(&mut self)`

  `pub fn scroll_logs_by(&mut self, delta: isize)`

  `pub fn sync_log_cursor(&mut self)`

  `pub fn scroll_logs_to_start(&mut self)`

  `pub fn scroll_logs_to_end(&mut self)`

  `pub fn scroll_help_up(&mut self)`

  `pub fn scroll_help_down(&mut self)`

  `pub fn update_running_workflow_progress(&mut self)`

  `pub fn set_error_message(&mut self, message: String)`

  `pub fn set_warning_message(&mut self, message: String)`

  `pub fn set_info_message(&mut self, message: String)`

  `pub fn set_success_message(&mut self, message: String)`

  `pub fn report_copy( &mut self, text: &str, result: std::io::Result<crate::selection::CopyMethod>, )`

  `pub fn tick(&mut self) -> bool`

  `pub fn trigger_selected_workflow(&mut self)`

  `pub fn reset_workflow_status(&mut self)`

  `pub fn request_log_processing_update(&mut self)`

  `pub fn check_log_processing_updates(&mut self)`

  `pub(crate) fn apply_log_update( &mut self, processed: Vec<crate::log_processor::ProcessedLogEntry>, matches: Vec<usize>, )`

  `pub fn mark_logs_for_update(&mut self)`

  `pub fn get_combined_logs(&self) -> Vec<String>`

  `fn add_log(&mut self, message: String)`

  `pub fn add_timestamped_log(&mut self, message: &str)`

  `fn trim_logs_to_cap(&mut self)`

  `pub fn trigger_selected_workflow_name(&self) -> Option<&str>`

  `pub fn trigger_tab_next_workflow(&mut self)`

  `pub fn trigger_tab_prev_workflow(&mut self)`

  `pub fn trigger_tab_toggle_platform(&mut self)`

  `pub fn trigger_tab_target(&mut self) -> &TriggerTarget`

  `pub fn inspecting_output(&self) -> bool`

  `pub fn trigger_editing(&self) -> bool`

  `pub fn trigger_tab_add_input(&mut self)`

  `pub fn trigger_tab_edit_inputs(&mut self)`

  `pub fn trigger_tab_edit_branch(&mut self)`

  `pub fn trigger_tab_next_field(&mut self)`

  `pub fn trigger_tab_prev_field(&mut self)`

  `pub fn trigger_tab_enter(&mut self)`

  `pub fn trigger_handle_input_key(&mut self, code: KeyCode) -> bool`

  `pub fn trigger_tab_copy_curl(&mut self)`

  `pub fn trigger_curl_preview(&self) -> String`

  `pub fn trigger_dispatch(&mut self)`

  `pub fn drain_timeline(&mut self)`

  `pub fn finish_execution( &mut self, workflow_idx: usize, result: Result<(Vec<wrkflw_executor::JobResult>, ()), String>, )`

  `pub fn drain_trigger_outcomes(&mut self)`

  `pub fn secrets_tab_next(&mut self)`

  `pub fn secrets_tab_prev(&mut self)`


`struct InFlightGuard`
> Fields: `flag: Arc<AtomicBool>`

**`impl InFlightGuard`**
  `fn arm(flag: Arc<AtomicBool>) -> Self`


**`impl Drop for InFlightGuard`**
  `fn drop(&mut self)`


`pub(crate) fn github_dispatches_body(branch: &str, inputs: &[(String, String)]) -> String`

`pub(crate) fn gitlab_pipeline_body(branch: &str, inputs: &[(String, String)]) -> String`

`fn escape_shell_single(s: &str) -> String`

`fn split_slug(slug: &str) -> Option<(String, String)>`

`fn resolve_trigger_target(platform: TriggerPlatform) -> TriggerTarget`

`async fn evaluate_diff_filter( workflow_paths: Vec<PathBuf>, event_name: String, activity_type: Option<String>, repo_root: Option<PathBuf>, ) -> DiffFilterOutcome`

`mod tests`

---

## crates/ui/src/cli_style.rs

**Language:** Rust | **Size:** 2.0 KB | **Lines:** 87

**Imports:**
- `colored::Colorize`
- `wrkflw_logging::symbols`

**Declarations:**

---

## crates/ui/src/components/button.rs

**Language:** Rust | **Size:** 1.4 KB | **Lines:** 54

**Imports:**
- `crate::theme::COLORS`
- `ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
}`

**Declarations:**

**`impl Button`**
  `pub fn new(label: &str) -> Self`

  `pub fn selected(mut self, is_selected: bool) -> Self`

  `pub fn active(mut self, is_active: bool) -> Self`

  `pub fn render(&self) -> Paragraph<'_>`


---

## crates/ui/src/components/checkbox.rs

**Language:** Rust | **Size:** 1.5 KB | **Lines:** 65

**Imports:**
- `crate::theme::{self, COLORS}`
- `ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
}`

**Declarations:**

**`impl Checkbox`**
  `pub fn new(label: &str) -> Self`

  `pub fn checked(mut self, is_checked: bool) -> Self`

  `pub fn selected(mut self, is_selected: bool) -> Self`

  `pub fn toggle(&mut self)`

  `pub fn render(&self) -> Paragraph<'_>`


---

## crates/ui/src/components/dag.rs

**Language:** Rust | **Size:** 2.6 KB | **Lines:** 80

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

---

## crates/ui/src/components/mod.rs

**Language:** Rust | **Size:** 228 B | **Lines:** 12

**Imports:**
- `pub use button::Button`
- `pub use checkbox::Checkbox`
- `pub use progress_bar::ProgressBar`

**Declarations:**

`mod button`

`mod checkbox`

`mod progress_bar`

---

## crates/ui/src/components/progress_bar.rs

**Language:** Rust | **Size:** 1.3 KB | **Lines:** 54

**Imports:**
- `crate::theme::{self, COLORS}`
- `ratatui::{
    style::{Color, Style},
    widgets::Gauge,
}`

**Declarations:**

**`impl ProgressBar`**
  `pub fn new(progress: f64) -> Self`

  `pub fn label(mut self, label: &str) -> Self`

  `pub fn color(mut self, color: Color) -> Self`

  `pub fn update(&mut self, progress: f64)`

  `pub fn render(&self) -> Gauge<'_>`


---

## crates/ui/src/components/progress_dots.rs

**Language:** Rust | **Size:** 2.5 KB | **Lines:** 86

**Imports:**
- `crate::models::WorkflowStatus`
- `crate::theme::COLORS`
- `ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `wrkflw_executor::StepStatus`

**Declarations:**

**`impl DotState`**
  `pub fn from_step(s: &StepStatus) -> Self`


`fn dot_style(state: DotState) -> Style`

---

## crates/ui/src/handlers/mod.rs

**Language:** Rust | **Size:** 42 B | **Lines:** 3

**Declarations:**

---

## crates/ui/src/handlers/workflow.rs

**Language:** Rust | **Size:** 26.5 KB | **Lines:** 684

**Imports:**
- `crate::cli_style`
- `std::collections::HashMap`
- `std::io`
- `std::path::{Path, PathBuf}`
- `wrkflw_evaluator::evaluate_workflow_file`
- `wrkflw_executor::{self, JobStatus, RuntimeType, StepStatus}`
- `{
    crate::app::App,
    crate::models::{ExecutionResultMsg, RunUpdate, TimelineMsg},
    std::sync::mpsc,
    std::thread,
}`

**Declarations:**

`struct TimelineObserver`
> Fields: `workflow_idx: usize`, `generation: u64`, `tx: mpsc::Sender<TimelineMsg>`

**`impl wrkflw_executor::ExecutionObserver for TimelineObserver`**
  `fn on_event(&self, event: &wrkflw_executor::ExecutionEvent)`

  `fn wants_step_output(&self) -> bool`


---

## crates/ui/src/lib.rs

**Language:** Rust | **Size:** 1.2 KB | **Lines:** 43

**Imports:**
- `pub use app::run_wrkflw_tui`
- `pub use handlers::workflow::execute_workflow_cli`
- `pub use handlers::workflow::validate_workflow`

**Declarations:**

`mod stress_tests`

---

## crates/ui/src/log_processor.rs

**Language:** Rust | **Size:** 11.9 KB | **Lines:** 350

**Imports:**
- `crate::models::{LogBadge, LogFilterLevel}`
- `crate::theme`
- `ratatui::{
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Row},
}`
- `std::sync::mpsc`
- `std::thread`
- `std::time::{Duration, Instant}`

**Declarations:**

**`impl ProcessedLogEntry`**
  `pub(crate) fn rendered_content(&self) -> String`

  `pub fn to_row(&self) -> Row<'static>`


**`impl LogProcessor`**
  `pub fn new() -> Self`

  `pub fn request_update( &self, request: LogProcessingRequest, ) -> Result<(), mpsc::SendError<LogProcessingRequest>>`

  `pub fn try_get_update(&self) -> Option<LogProcessingResponse>`

  `fn worker_loop( request_rx: mpsc::Receiver<LogProcessingRequest>, response_tx: mpsc::Sender<LogProcessingResponse>, )`

  `fn get_combined_logs(app_logs: &[String]) -> Vec<String>`

  `fn process_logs(all_logs: &[String], request: &LogProcessingRequest) -> LogProcessingResponse`

  `pub(crate) fn process_log_entry(log_line: &str, search_query: &str) -> ProcessedLogEntry`

  `fn highlight_search_matches(content: &str, search_query: &str) -> Vec<Span<'static>>`


**`impl Default for LogProcessor`**
  `fn default() -> Self`


`mod tests`

---

## crates/ui/src/models/mod.rs

**Language:** Rust | **Size:** 48.7 KB | **Lines:** 1339

**Imports:**
- `chrono::Local`
- `std::path::PathBuf`
- `std::sync::Arc`
- `wrkflw_executor::{JobStatus, StepStatus}`
- `wrkflw_logging::symbols`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

**`impl JobExecution`**
  `pub fn is_for(&self, key: &str) -> bool`


`fn merged_state<'a>(legs: impl IntoIterator<Item = (&'a JobStatus, bool)> + Clone) -> JobState`

**`impl Workflow`**
  `pub fn job_state(&self, key: &str) -> JobState`

  `pub fn matrix_legs(&self, key: &str) -> Option<usize>`

  `pub fn all_matrix_legs(&self) -> std::collections::HashMap<String, usize>`

  `pub fn job_row_count(&self) -> usize`

  `pub fn job_row_name(&self, row: usize) -> Option<&str>`

  `pub fn job_row_of(&self, name: &str) -> Option<usize>`

  `pub fn unreported_jobs(&self) -> Vec<&str>`


**`impl LiveTails`**
  `pub fn push(&mut self, job: &str, index: usize, step: &str, lines: Vec<String>)`

  `pub fn push_after( &mut self, job: &str, index: usize, step: &str, lines: Vec<String>, earlier: usize, )`

  `pub fn finish(&mut self, job: &str)`

  `pub fn pick(&self, job: Option<&str>) -> Option<(&str, &LiveTail)>`


**`impl WorkflowExecution`**
  `pub fn live_job_state(&self, key: &str, legs: Option<usize>) -> Option<JobState>`

  `pub fn live_tail_for(&self, name: Option<&str>) -> Option<(&str, &LiveTail)>`

  `pub fn job_durations_ms(&self) -> std::collections::HashMap<String, u64>`

  `pub fn unfinished_jobs( &self, legs: &std::collections::HashMap<String, usize>, ) -> std::collections::HashSet<String>`


**`impl TimelineEntry`**
  `pub fn from_event(event: &wrkflw_executor::ExecutionEvent) -> Option<Self>`


`fn failure_tail(output: &str) -> Vec<String>`

`fn cap_line(line: &str) -> String`

**`impl RunUpdate`**
  `pub fn from_event(event: &wrkflw_executor::ExecutionEvent) -> Option<Self>`


**`impl LogBadge`**
  `pub fn classify(log: &str) -> Self`

  `pub fn as_str(&self) -> &'static str`

  `pub fn style_key(&self) -> &'static str`


**`impl LogFilterLevel`**
  `pub fn matches(&self, log: &str) -> bool`

  `pub fn next(&self) -> Self`

  `pub fn as_str(&self) -> &str`


`mod tests`

---

## crates/ui/src/selection.rs

**Language:** Rust | **Size:** 15.2 KB | **Lines:** 445

**Imports:**
- `crate::theme::COLORS`
- `base64::{engine::general_purpose::STANDARD, Engine as _}`
- `crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind}`
- `ratatui::{buffer::Buffer, layout::Rect, style::Style}`
- `std::io::{self, Write}`
- `std::process::{Command, Stdio}`
- `std::time::{Duration, Instant}`
- `unicode_width::UnicodeWidthStr`

**Declarations:**

`const CLIPBOARD_TOOL_TIMEOUT: Duration = Duration::from_millis(500)`

`const VERTICAL_BORDERS: &str = "│┃║"`

`const BORDER_JOINS: &str = "╭╮╰╯┌┐└┘├┤┬┴┼╔╗╚╝╠╣╦╩╬┏┓┗┛┣┫┳┻╋"`

**`impl Selection`**
  `fn new(anchor: (u16, u16)) -> Self`

  `fn row_spans(&self, bounds: Rect) -> Vec<(u16, u16, u16)>`

  `fn text(&self, buf: &Buffer, bounds: Rect) -> String`


`fn symbol_in(buf: &Buffer, col: u16, row: u16, set: &str) -> bool`

`fn is_wall(buf: &Buffer, col: u16, row: u16) -> bool`

`fn enclosing_pane(buf: &Buffer, (col, row): (u16, u16)) -> Rect`

`fn osc52(text: &str, in_tmux: bool) -> String`

`fn native_tools() -> Vec<(&'static str, &'static [&'static str])>`

`fn run_tool(program: &str, args: &[&str], text: &str) -> io::Result<()>`

`mod tests`

---

## crates/ui/src/stress_tests.rs

**Language:** Rust | **Size:** 16.8 KB | **Lines:** 478

**Imports:**
- `crate::app::App`
- `crate::log_processor::ProcessedLogEntry`
- `crate::models::{
    JobExecution, StepExecution, TriggerMatchStatus, Workflow, WorkflowExecution, WorkflowStatus,
}`
- `crate::theme`
- `crate::views::{self, SCREEN_COUNT, TAB_EXECUTION, TAB_WORKFLOWS}`
- `ratatui::{backend::TestBackend, text::Span, Terminal}`
- `std::collections::HashMap`
- `std::path::PathBuf`
- `std::sync::{mpsc, Arc}`
- `wrkflw_executor::{JobStatus, RuntimeType, StepStatus}`

**Declarations:**

`const SIZES: &[(u16, u16)] = &[ (1, 1), (2, 2), (5, 3), (10, 5), (3, 80), (80, 3), (20, 8), (40, 12), (60, 20), (80, 24), (100, 30), (120, 40), (200, 60), (300, 100), ]`

`const NASTY: &[&str] = &[ "", "a", "日本語のワークフロー名前がとても長い場合", "🚀🔥 emoji-heavy ✨ workflow 🎉", "tab\tseparated\tname", "a-really-really-really-really-really-really-really-really-long-workflow-name-that-never-ends", "e\u`

`fn big_definition( dir: &std::path::Path, jobs: usize, ) -> Arc<wrkflw_parser::workflow::WorkflowDefinition>`

`fn parse_big( dir: &std::path::Path, jobs: usize, ) -> Arc<wrkflw_parser::workflow::WorkflowDefinition>`

`fn execution(jobs: usize, steps: usize) -> WorkflowExecution`

`fn timeline(jobs: usize, steps: usize) -> Vec<crate::models::TimelineEntry>`

`fn live_tails(jobs: usize) -> crate::models::LiveTails`

`fn app(workflows: usize, dir: &std::path::Path) -> App`

`type Scenario = (&'static str, fn(&mut App))`

`fn scenarios() -> Vec<Scenario>`

`fn render(app: &mut App, w: u16, h: u16)`

`fn every_screen_renders_at_every_size_without_panicking()`

`fn screen_text(terminal: &Terminal<TestBackend>) -> String`

`fn inspector_reaches_the_tail_of_output_longer_than_a_u16_scroll()`

`fn dump_screens()`

---

## crates/ui/src/text.rs

**Language:** Rust | **Size:** 14.9 KB | **Lines:** 434

**Imports:**
- `crate::theme::COLORS`
- `ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
}`
- `std::borrow::Cow`
- `unicode_width::{UnicodeWidthChar, UnicodeWidthStr}`

**Declarations:**

`const ELLIPSIS: char = '…'`

`const TAB_STOP: usize = 4`

`fn apply_sgr(mut style: Style, base: Style, params: &str) -> Style`

`fn apply_code(style: Style, base: Style, code: u16) -> Style`

`fn set_color(style: Style, code: u16, color: Option<Color>) -> Style`

`fn indexed(v: u16) -> Color`

`fn rgb_color(r: u16, g: u16, b: u16) -> Color`

`fn ansi_color(n: u16) -> Color`

`mod tests`

---

## crates/ui/src/theme.rs

**Language:** Rust | **Size:** 29.6 KB | **Lines:** 891

**Imports:**
- `ratatui::{
    layout::{Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
}`
- `std::cell::Cell`
- `std::ops::Deref`
- `std::str::FromStr`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `pub use wrkflw_logging::symbols`
- `crate::models::WorkflowStatus`
- `wrkflw_executor::{JobStatus, StepStatus}`

**Declarations:**

**`impl ThemeMode`**
  `fn colors(self) -> &'static Colors`


**`impl FromStr for ThemePreference`**
  `fn from_str(s: &str) -> Result<Self, Self::Err>`


`const THEME_ENV: &str = "WRKFLW_THEME"`

`static LIGHT_MODE: AtomicBool = AtomicBool::new(false)`

`fn detect_mode() -> ThemeMode`

`fn mode_from_colorfgbg(value: &str) -> Option<ThemeMode>`

**`impl Deref for ActiveColors`**
  `fn deref(&self) -> &Colors`


`static DARK: Colors = Colors`

`static LIGHT: Colors = Colors`

`fn boxed_title<'a>(title: &str, border_type: BorderType, edge: Color, style: Style) -> Line<'a>`

**`impl BadgeKind`**
  `pub fn fg(self) -> Color`


`mod tests`

---

## crates/ui/src/utils/mod.rs

**Language:** Rust | **Size:** 2.9 KB | **Lines:** 84

**Imports:**
- `crate::models::{Workflow, WorkflowStatus}`
- `std::path::{Path, PathBuf}`
- `std::sync::Arc`
- `wrkflw_parser::workflow::{parse_workflow, WorkflowDefinition}`
- `wrkflw_utils::is_workflow_file`

**Declarations:**

`fn load_definition(path: &Path) -> (Option<Arc<WorkflowDefinition>>, Vec<String>)`

---

## crates/ui/src/views/dag_tab.rs

**Language:** Rust | **Size:** 21.8 KB | **Lines:** 619

**Imports:**
- `crate::app::App`
- `crate::components::dag::{self, matrix_size_label}`
- `crate::models::{JobState, Workflow}`
- `crate::text`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `std::collections::{HashMap, HashSet}`
- `std::sync::Arc`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`type MatrixLabels = HashMap<String, String>`

`const MIN_CARD_W: u16 = 16`

`const MAX_CARD_W: u16 = 28`

`const GAP: u16 = 3`

`const CARD_H: u16 = 3`

`struct Timing`
> Fields: `durations: HashMap<String, u64>`, `critical: HashSet<String>`, `settled: bool`, `unfinished: HashSet<String>`

**`impl Timing`**
  `fn of(workflow: &Workflow, levels: &[Vec<String>]) -> Self`

  `fn stage_ms(&self, layer: &[String]) -> Option<u64>`


`fn critical_path(levels: &[Vec<String>], durations: &HashMap<String, u64>) -> HashSet<String>`

`fn matrix_suffix(labels: &MatrixLabels, job: &str) -> String`

`fn state_color(state: JobState) -> ratatui::style::Color`

`fn render_graph( f: &mut Frame<'_>, app: &App, workflow: &Workflow, def: &WorkflowDefinition, labels: &MatrixLabels, levels: &[Vec<String>], area: Rect, ) -> usize`

`fn render_card( f: &mut Frame<'_>, app: &App, workflow: &Workflow, labels: &MatrixLabels, timing: &Timing, name: &str, area: Rect, )`

`fn render_list( f: &mut Frame<'_>, app: &App, workflow: &Workflow, def: &WorkflowDefinition, labels: &MatrixLabels, levels: &[Vec<String>], area: Rect, ) -> usize`

`fn render_legend(f: &mut Frame<'_>, app: &App, area: Rect)`

`mod timing_tests`

---

## crates/ui/src/views/execution_tab.rs

**Language:** Rust | **Size:** 19.8 KB | **Lines:** 554

**Imports:**
- `crate::app::App`
- `crate::models::{JobExecution, JobState, Workflow, WorkflowStatus}`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `wrkflw_executor::{JobStatus, StepStatus}`

**Declarations:**

`fn selected_job<'a>(app: &App, workflow: &'a Workflow) -> Option<&'a JobExecution>`

`pub(crate) fn format_ms(ms: u64) -> String`

`pub(crate) fn format_duration(secs: i64) -> String`

`pub(crate) fn format_run_time(ms: i64, finished: bool) -> String`

`struct Counts`
> Fields: `passed: usize`, `failed: usize`, `skipped: usize`

`fn job_counts(workflow: &Workflow) -> Counts`

`fn render_jobs_pane(f: &mut Frame<'_>, app: &App, workflow: &Workflow, area: Rect)`

`fn render_steps_pane( f: &mut Frame<'_>, workflow: &Workflow, job: Option<&JobExecution>, area: Rect, )`

`pub(crate) fn step_line(i: usize, step: &crate::models::StepExecution, w: usize) -> Line<'static>`

`fn render_output_pane( f: &mut Frame<'_>, app: &App, workflow: &Workflow, selected: Option<&str>, area: Rect, )`

`fn active_job_execution(workflow: &Workflow) -> Option<&JobExecution>`

`fn render_empty_state(f: &mut Frame<'_>, area: Rect)`

`mod tests`

---

## crates/ui/src/views/help_overlay.rs

**Language:** Rust | **Size:** 7.4 KB | **Lines:** 222

**Imports:**
- `crate::text`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`

**Declarations:**

`type Section = (&'static str, Vec<(String, &'static str)>)`

`fn sections() -> Vec<Section>`

`const KEY_W: usize = 17`

`fn section_lines(section: &Section, w: usize) -> Vec<Line<'static>>`

---

## crates/ui/src/views/job_detail.rs

**Language:** Rust | **Size:** 20.1 KB | **Lines:** 573

**Imports:**
- `crate::app::App`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`
- `wrkflw_executor::JobStatus`

**Declarations:**

`const MAX_OUTPUT_LINES: usize = 5000`

**`impl StepOutputCache`**
  `fn build(key: (u64, usize, usize, usize), output: &str) -> Self`

  `fn measure(&mut self, width: u16)`

  `fn locate(&self, row: usize) -> (usize, usize)`


`struct OutputSource<'a>`
> Fields: `key: Option<(u64, usize, usize, usize)>`, `scroll_back: usize`, `cache: &'a mut Option<StepOutputCache>`

`fn render_header( f: &mut Frame<'_>, workflow_name: &str, job: &crate::models::JobExecution, active_tab: usize, area: Rect, )`

`fn render_output_pane( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected: Option<usize>, output: OutputSource<'_>, area: Rect, ) -> usize`

`fn render_steps_list( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected: Option<usize>, area: Rect, )`

`fn render_step_output( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected: Option<usize>, output: OutputSource<'_>, area: Rect, ) -> usize`

`fn render_matrix_pane( f: &mut Frame<'_>, workflow: &crate::models::Workflow, job_name: &str, area: Rect, )`

`fn format_yaml_scalar(v: &serde_yaml::Value) -> String`

`fn collapse_newlines(s: &str) -> String`

`fn combo_glyph<'a>( workflow: &'a crate::models::Workflow, job_name: &str, combo: &wrkflw_matrix::MatrixCombination, ) -> Span<'a>`

---

## crates/ui/src/views/logs_tab.rs

**Language:** Rust | **Size:** 25.1 KB | **Lines:** 697

**Imports:**
- `crate::app::App`
- `crate::models::{LogFilterLevel, TimelineEntry}`
- `crate::text`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, Paragraph, Table, TableState},
    Frame,
}`

**Declarations:**

`fn render_search_bar(f: &mut Frame<'_>, app: &App, area: Rect)`

`struct JobStory<'a>`
> Fields: `name: &'a str`, `steps: Vec<&'a TimelineEntry>`, `finished: Option<&'a TimelineEntry>`, `runner: Option<(&'a str, &'a str)>`, `not_run: Vec<usize>`, `not_run_pending: bool`, `not_run_unattached: bool`

`fn group_by_job(timeline: &[TimelineEntry]) -> Vec<JobStory<'_>>`

`fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, w: usize) -> Line<'static>`

`pub(crate) fn story_lines( timeline: &[TimelineEntry], live: Option<&crate::models::LiveTails>, w: usize, spinner_frame: usize, settled: bool, ) -> Vec<Line<'static>>`

`const STORY_LIVE_LINES: usize = 3`

`fn render_story(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`mod tests`

---

## crates/ui/src/views/mod.rs

**Language:** Rust | **Size:** 5.2 KB | **Lines:** 150

**Imports:**
- `pub use job_detail::StepOutputCache`
- `pub use title_bar::{
    tab_of, SCREEN_COUNT, TABS, TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUN, TAB_RUNS,
    TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS,
}`
- `crate::app::App`
- `ratatui::Frame`

**Declarations:**

`mod dag_tab`

`mod execution_tab`

`mod help_overlay`

`mod job_detail`

`mod logs_tab`

`mod run_header`

`mod runs_tab`

`mod secrets_tab`

`mod status_bar`

`mod title_bar`

`mod trigger_tab`

`mod tweaks_overlay`

`mod workflows_tab`

`struct AccentScope`

**`impl AccentScope`**
  `fn install(color: ratatui::style::Color) -> Self`


**`impl Drop for AccentScope`**
  `fn drop(&mut self)`


`mod tests`

---

## crates/ui/src/views/run_header.rs

**Language:** Rust | **Size:** 1.8 KB | **Lines:** 53

**Imports:**
- `crate::app::App`
- `crate::theme`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `super::{execution_tab, TAB_DAG, TAB_EXECUTION, TAB_LOGS}`

**Declarations:**

`const VIEWS: [(usize, &str, &str)`

---

## crates/ui/src/views/runs_tab.rs

**Language:** Rust | **Size:** 9.2 KB | **Lines:** 258

**Imports:**
- `crate::app::App`
- `crate::text`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`
- `wrkflw_runs::{RunRecord, RunStatus}`

**Declarations:**

`pub(crate) fn status_glyph(status: RunStatus) -> (&'static str, ratatui::style::Color)`

`fn text_status(status: &str) -> (&'static str, ratatui::style::Color)`

`pub(crate) fn when(started_at: &str) -> String`

`pub(crate) fn duration(ms: Option<u64>) -> String`

`fn run_row(run: &RunRecord, w: usize, selected: bool) -> Line<'static>`

`fn detail_lines(run: &RunRecord) -> Vec<Line<'static>>`

`mod tests`

---

## crates/ui/src/views/secrets_tab.rs

**Language:** Rust | **Size:** 8.2 KB | **Lines:** 235

**Imports:**
- `crate::app::App`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, List, ListItem, Paragraph},
    Frame,
}`
- `wrkflw_executor::RuntimeType`
- `wrkflw_secrets::{SecretConfig, SecretProviderConfig}`

**Declarations:**

`const RUNTIME_PANE_HEIGHT: u16 = 10`

`fn provider_entries() -> Vec<(String, SecretProviderConfig)>`

`fn render_providers_pane( f: &mut Frame<'_>, app: &mut App, rows: &[(String, SecretProviderConfig)], area: Rect, )`

`fn render_detail_pane( f: &mut Frame<'_>, app: &App, rows: &[(String, SecretProviderConfig)], area: Rect, )`

`fn render_runtime_pane(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn kv(key: &str, value: &str, w: usize) -> Line<'static>`

---

## crates/ui/src/views/status_bar.rs

**Language:** Rust | **Size:** 8.0 KB | **Lines:** 228

**Imports:**
- `super::{TAB_DAG, TAB_EXECUTION, TAB_LOGS, TAB_RUNS, TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS}`
- `crate::app::App`
- `crate::models::StatusSeverity`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `wrkflw_executor::RuntimeType`

**Declarations:**

`fn context_hints(app: &App) -> Vec<(&'static str, &'static str)>`

---

## crates/ui/src/views/title_bar.rs

**Language:** Rust | **Size:** 6.0 KB | **Lines:** 172

**Imports:**
- `crate::app::App`
- `crate::models::WorkflowStatus`
- `crate::text`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`

**Declarations:**

`fn fit_tabs(selected: usize, budget: usize) -> (Vec<Span<'static>>, Vec<Span<'static>>)`

`enum TabStyle`
> Variants: `Full`, `Compact`, `ActiveOnly`

`fn tab_buttons(selected: usize, style: TabStyle) -> (Vec<Span<'static>>, Vec<Span<'static>>)`

`fn live_elapsed(app: &App) -> Option<String>`

---

## crates/ui/src/views/trigger_tab.rs

**Language:** Rust | **Size:** 9.4 KB | **Lines:** 287

**Imports:**
- `crate::app::{App, TriggerPlatform}`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`

**Declarations:**

`fn token_is_set(var: &str) -> bool`

`fn render_target_pane(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn render_preview_pane(f: &mut Frame<'_>, app: &App, area: Rect)`

`const LABEL_W: usize = 12`

`fn label_span(label: &str) -> Span<'static>`

`fn editing_style() -> Style`

`fn field_row(label: &str, value: &str, room: usize, bold: bool) -> Line<'static>`

---

## crates/ui/src/views/tweaks_overlay.rs

**Language:** Rust | **Size:** 3.5 KB | **Lines:** 106

**Imports:**
- `crate::app::{Accent, App}`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`

**Declarations:**

`const PANEL_W: u16 = 44`

`const PANEL_H: u16 = 6`

`fn render_accent_row(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_shortcut_hint(f: &mut Frame<'_>, area: Rect)`

---

## crates/ui/src/views/workflows_tab.rs

**Language:** Rust | **Size:** 16.9 KB | **Lines:** 483

**Imports:**
- `crate::app::App`
- `crate::models::TriggerMatchStatus`
- `crate::text`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Frame,
}`

**Declarations:**

`fn render_workflow_list(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn render_right_column(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_preview(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_trigger_filter(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_job_selection(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn last_run_line(run: &wrkflw_runs::RunRecord, w: usize) -> Line<'static>`

---

## crates/utils/Cargo.toml

**Language:** TOML | **Size:** 537 B | **Lines:** 23

**Declarations:**

---

## crates/utils/README.md

**Language:** Markdown | **Size:** 588 B | **Lines:** 21

**Declarations:**

---

## crates/utils/src/lib.rs

**Language:** Rust | **Size:** 9.0 KB | **Lines:** 253

**Imports:**
- `std::path::{Path, PathBuf}`

**Declarations:**

`mod tests`

---

## crates/validators/Cargo.toml

**Language:** TOML | **Size:** 570 B | **Lines:** 24

**Declarations:**

---

## crates/validators/README.md

**Language:** Markdown | **Size:** 816 B | **Lines:** 30

**Declarations:**

---

## crates/validators/src/actions.rs

**Language:** Rust | **Size:** 14.3 KB | **Lines:** 555

**Imports:**
- `std::collections::HashSet`
- `std::path::Path`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_local_action_inputs( action_file: &Path, with_params: Option<&serde_yaml::Mapping>, action_ref: &str, job_name: &str, step_idx: usize, result: &mut ValidationResult, )`

`mod tests`

---

## crates/validators/src/expressions.rs

**Language:** Rust | **Size:** 13.2 KB | **Lines:** 388

**Imports:**
- `serde_yaml::{Mapping, Value}`
- `std::collections::HashSet`
- `wrkflw_models::ValidationResult`

**Declarations:**

`struct Scope`
> Fields: `steps: Option<HashSet<String>>`, `needs: Option<HashSet<String>>`, `matrix: Option<HashSet<String>>`

`fn check_job(job: &Mapping, result: &mut ValidationResult)`

`fn needs_of(job: &Mapping) -> HashSet<String>`

`fn matrix_keys(job: &Mapping) -> Option<HashSet<String>>`

`fn walk(value: &Value, scope: &Scope, result: &mut ValidationResult)`

`fn check_condition(value: &Value, scope: &Scope, result: &mut ValidationResult)`

`fn check_text(text: &str, scope: &Scope, result: &mut ValidationResult)`

`fn check_references(expression: &str, scope: &Scope, result: &mut ValidationResult)`

`fn expressions_in(text: &str) -> (Vec<&str>, bool)`

`fn closing_braces(text: &str) -> Option<usize>`

`fn without_strings(expression: &str) -> String`

`fn references(code: &str, context: &str) -> Vec<String>`

`mod tests`

---

## crates/validators/src/gitlab.rs

**Language:** Rust | **Size:** 12.6 KB | **Lines:** 380

**Imports:**
- `std::collections::HashMap`
- `wrkflw_models::gitlab::{Job, Pipeline}`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_jobs(jobs: &HashMap<String, Job>, result: &mut ValidationResult)`

`fn validate_stages(stages: &[String], jobs: &HashMap<String, Job>, result: &mut ValidationResult)`

`fn validate_dependencies(jobs: &HashMap<String, Job>, result: &mut ValidationResult)`

`fn validate_extends(jobs: &HashMap<String, Job>, result: &mut ValidationResult)`

`fn check_circular_extends( job_name: &str, jobs: &HashMap<String, Job>, visited: &mut Vec<String>, result: &mut ValidationResult, )`

`fn validate_artifacts(jobs: &HashMap<String, Job>, result: &mut ValidationResult)`

`mod tests`

---

## crates/validators/src/jobs.rs

**Language:** Rust | **Size:** 13.4 KB | **Lines:** 435

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `std::path::Path`
- `crate::{validate_env, validate_matrix, validate_steps}`
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_job( job_name: &str, job_config: &Value, jobs_map: &serde_yaml::Mapping, repo_root: Option<&Path>, result: &mut ValidationResult, )`

`fn detect_cyclic_needs(jobs_map: &serde_yaml::Mapping, result: &mut ValidationResult)`

`fn dfs_detect_cycle( node: &str, graph: &HashMap<String, Vec<String>>, visited: &mut HashSet<String>, in_stack: &mut HashSet<String>, rec_stack: &mut Vec<String>, reported_cycles: &mut HashSet<Vec<String>>, result: &mut ValidationResult, )`

`mod tests`

---

## crates/validators/src/lib.rs

**Language:** Rust | **Size:** 1.5 KB | **Lines:** 53

**Imports:**
- `pub use actions::validate_action_reference`
- `pub use expressions::validate_expressions`
- `pub use gitlab::validate_gitlab_pipeline`
- `pub use jobs::validate_jobs`
- `pub use matrix::validate_matrix`
- `pub use steps::validate_steps`
- `pub use triggers::validate_triggers`
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`mod actions`

`mod expressions`

`mod gitlab`

`mod jobs`

`mod matrix`

`mod steps`

`mod triggers`

`fn is_expression_string(v: &Value) -> bool`

`fn yaml_type_name(v: &Value) -> &'static str`

---

## crates/validators/src/matrix.rs

**Language:** Rust | **Size:** 8.7 KB | **Lines:** 256

**Imports:**
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_include_exclude(section: &Value, section_name: &str, result: &mut ValidationResult)`

`fn validate_matrix_parameter(name: &str, value: &Value, result: &mut ValidationResult)`

`fn get_value_type(value: &Value) -> &'static str`

`mod tests`

---

## crates/validators/src/steps.rs

**Language:** Rust | **Size:** 5.6 KB | **Lines:** 178

**Imports:**
- `crate::{validate_action_reference, validate_env}`
- `serde_yaml::Value`
- `std::collections::HashSet`
- `std::path::Path`
- `wrkflw_models::ValidationResult`

**Declarations:**

`mod tests`

---

## crates/validators/src/triggers.rs

**Language:** Rust | **Size:** 10.8 KB | **Lines:** 333

**Imports:**
- `serde_yaml::{Mapping, Value}`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn event_filters(event: &str) -> &'static [&'static str]`

`fn check_filters(event: &str, filters: &Mapping, result: &mut ValidationResult)`

`fn validate_cron_syntax(cron: &str, result: &mut ValidationResult)`

`fn is_valid_cron_field(field: &str, min: u32, max: u32) -> bool`

`fn is_valid_cron_atom(atom: &str, min: u32, max: u32) -> bool`

`mod tests`

---

## crates/watcher/Cargo.toml

**Language:** TOML | **Size:** 681 B | **Lines:** 26

**Declarations:**

---

## crates/watcher/README.md

**Language:** Markdown | **Size:** 801 B | **Lines:** 17

**Declarations:**

---

## crates/watcher/src/debouncer.rs

**Language:** Rust | **Size:** 13.0 KB | **Lines:** 316

**Imports:**
- `std::collections::HashSet`
- `std::path::PathBuf`
- `std::sync::atomic::{AtomicUsize, Ordering}`
- `std::sync::{Arc, Mutex}`
- `std::time::Duration`
- `tokio::sync::Notify`

**Declarations:**

**`impl Debouncer`**
  `pub fn new(duration: Duration) -> Self`

  `pub fn with_capacity(duration: Duration, max_pending: usize) -> Self`

  `pub fn notifier(&self) -> Arc<Notify>`

  `pub fn dropped_count(&self) -> usize`

  `pub fn add_event(&self, path: PathBuf)`

  `pub async fn drain(&self) -> Vec<PathBuf>`

  `pub fn has_pending(&self) -> bool`

  `fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, HashSet<PathBuf>>`


`mod tests`

---

## crates/watcher/src/error.rs

**Language:** Rust | **Size:** 265 B | **Lines:** 13

**Imports:**
- `thiserror::Error`

**Declarations:**

---

## crates/watcher/src/event_kind.rs

**Language:** Rust | **Size:** 3.4 KB | **Lines:** 79

**Imports:**
- `notify::event::{EventKind, ModifyKind}`

**Declarations:**

`pub(crate) fn is_relevant_event_kind(kind: &EventKind) -> bool`

`mod tests`

---

## crates/watcher/src/git_state.rs

**Language:** Rust | **Size:** 7.4 KB | **Lines:** 171

**Imports:**
- `std::path::Path`
- `std::sync::Mutex`
- `std::time::Instant`

**Declarations:**

`pub(crate) struct CachedGitState`
> Fields: `fetched_at: Instant`, `head_mtime: Option<std::time::SystemTime>`, `branch: Option<String>`, `tag: Option<String>`

`pub(crate) struct GitStateCache`
> Fields: `inner: Mutex<Option<CachedGitState>>`

**`impl GitStateCache`**
  `pub(crate) fn new() -> Self`

  `pub(crate) fn peek_fetched_at(&self) -> Option<Instant>`

  `pub(crate) async fn get( &self, config: &wrkflw_trigger_filter::TriggerFilterConfig, repo_root: &Path, ) -> Result<(Option<String>, Option<String>), wrkflw_trigger_filter::TriggerFilterError>`


---

## crates/watcher/src/ignore.rs

**Language:** Rust | **Size:** 10.9 KB | **Lines:** 322

**Imports:**
- `std::collections::HashSet`
- `std::path::Path`

**Declarations:**

`pub(crate) const DEFAULT_IGNORE_DIRS: &[&str] = &[ ".git", "target", "node_modules", ".build", "build", "dist", "__pycache__", ".tox", ".mypy_cache", ".pytest_cache", ".venv", "venv", ]`

`pub(crate) fn build_ignore_set(extra_ignore_dirs: &[String]) -> HashSet<String>`

`pub(crate) fn should_ignore_path( path: &Path, repo_root_raw: &Path, repo_root_canonical: &Path, ignore_dirs: &HashSet<String>, ) -> bool`

`mod tests`

---

## crates/watcher/src/lib.rs

**Language:** Rust | **Size:** 708 B | **Lines:** 21

**Imports:**
- `pub use error::WatchError`
- `pub use shutdown::ShutdownSignal`
- `pub use watcher::{WatchEvent, WatcherConfig, WorkflowWatcher, DEFAULT_MAX_CONCURRENT_EXECUTIONS}`

**Declarations:**

`pub(crate) mod event_kind`

`pub(crate) mod git_state`

`pub(crate) mod ignore`

`pub(crate) mod paths`

`pub(crate) mod reactor`

`pub(crate) mod setup`

`pub(crate) mod trigger_cache`

---

## crates/watcher/src/paths.rs

**Language:** Rust | **Size:** 4.0 KB | **Lines:** 98

**Imports:**
- `std::path::Path`

**Declarations:**

`pub(crate) fn normalize_separators(s: &str) -> String`

`pub(crate) fn display_workflow_path(wf_path: &Path, repo_root: &Path) -> String`

`mod tests`

---

## crates/watcher/src/reactor.rs

**Language:** Rust | **Size:** 42.6 KB | **Lines:** 898

**Imports:**
- `crate::debouncer::Debouncer`
- `crate::error::WatchError`
- `crate::event_kind::is_relevant_event_kind`
- `crate::ignore::{build_ignore_set, should_ignore_path}`
- `crate::paths::{display_workflow_path, normalize_separators}`
- `crate::setup::setup_watches`
- `crate::shutdown::ShutdownSignal`
- `crate::trigger_cache::{refresh_trigger_cache_blocking, TriggerCacheEntry}`
- `crate::watcher::{WatchEvent, WorkflowWatcher}`
- `futures::stream::{self, StreamExt}`
- *... and 8 more imports*

**Declarations:**

`const SUPERVISOR_WARN_THRESHOLD: usize = 8`

`const SUPERVISOR_HARD_CAP: usize = 128`

`const _: () =`

`pub(crate) async fn run_loop<F>( watcher: &WorkflowWatcher, shutdown: ShutdownSignal, on_cycle_complete: F, ) -> Result<(), WatchError> where F: Fn(WatchEvent) + Send + Sync + 'static,`

`async fn refresh_trigger_cache_async( watcher: &WorkflowWatcher, trigger_cache: HashMap<PathBuf, TriggerCacheEntry>, workflow_files: &[PathBuf], changed_paths: &[PathBuf], ) -> HashMap<PathBuf, TriggerCacheEntry>`

`async fn canonicalize_changed_paths( watcher: &WorkflowWatcher, changed_paths: &[PathBuf], repo_root_canonical: &Path, ) -> Vec<String>`

`pub(crate) async fn evaluate_and_execute( watcher: &WorkflowWatcher, configs: &[&WorkflowTriggerConfig], changed_files: Vec<String>, ) -> WatchEvent`

---

## crates/watcher/src/setup.rs

**Language:** Rust | **Size:** 9.3 KB | **Lines:** 225

**Imports:**
- `crate::error::WatchError`
- `notify::{RecommendedWatcher, RecursiveMode, Watcher}`
- `std::collections::HashSet`
- `std::path::{Path, PathBuf}`

**Declarations:**

`pub(crate) fn setup_watches( watcher: &mut RecommendedWatcher, root: &Path, ignore_dirs: &HashSet<String>, ) -> Result<(), WatchError>`

`pub(crate) fn collect_workflow_files_blocking(dir: &Path) -> Result<Vec<PathBuf>, WatchError>`

`mod tests`

---

## crates/watcher/src/shutdown.rs

**Language:** Rust | **Size:** 9.5 KB | **Lines:** 237

**Imports:**
- `std::sync::Arc`
- `tokio::sync::watch`

**Declarations:**

**`impl ShutdownSignal`**
  `pub fn new() -> Self`

  `pub fn never() -> Self`

  `pub fn trigger(&self)`

  `pub fn is_triggered(&self) -> bool`

  `pub async fn wait(&self)`


**`impl Default for ShutdownSignal`**
  `fn default() -> Self`


`mod tests`

---

## crates/watcher/src/trigger_cache.rs

**Language:** Rust | **Size:** 16.9 KB | **Lines:** 389

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `std::path::PathBuf`
- `wrkflw_trigger_filter::canonicalize_allowing_missing`
- `wrkflw_trigger_filter::{TriggerFilterConfig, WorkflowTriggerConfig}`

**Declarations:**

`pub(crate) struct TriggerCacheEntry`
> Fields: `canonical_path: PathBuf`, `config: WorkflowTriggerConfig`

`pub(crate) fn refresh_trigger_cache_blocking( trigger_cache: &mut HashMap<PathBuf, TriggerCacheEntry>, workflow_files: &[PathBuf], changed_paths: &[PathBuf], verbose: bool, tf_config: &TriggerFilterConfig, )`

`mod tests`

---

## crates/watcher/src/watcher.rs

**Language:** Rust | **Size:** 53.4 KB | **Lines:** 1233

**Imports:**
- `crate::error::WatchError`
- `crate::git_state::GitStateCache`
- `crate::setup::collect_workflow_files_blocking`
- `crate::shutdown::ShutdownSignal`
- `std::path::PathBuf`
- `std::time::Duration`
- `wrkflw_executor::ExecutionConfig`
- `wrkflw_trigger_filter::canonicalize_allowing_missing`
- `wrkflw_trigger_filter::TriggerFilterConfig`

**Declarations:**

**`impl WatcherConfig`**
  `pub fn new(workflow_dir: PathBuf, repo_root: PathBuf, execution: ExecutionConfig) -> Self`

  `pub fn with_trigger_filter_config(mut self, cfg: TriggerFilterConfig) -> Self`

  `pub fn with_max_pending_events(mut self, n: usize) -> Self`

  `pub fn with_extra_ignore_dirs(mut self, dirs: Vec<String>) -> Self`

  `pub fn with_run_options(mut self, options: wrkflw_runs::RunOptions) -> Self`

  `pub fn with_event(mut self, event: impl Into<String>) -> Self`

  `pub fn with_base_branch(mut self, base: Option<String>) -> Self`

  `pub fn with_activity_type(mut self, activity: Option<String>) -> Self`

  `pub fn with_debounce(mut self, d: Duration) -> Self`

  `pub fn with_verbose(mut self, v: bool) -> Self`

  `pub fn with_max_concurrency(mut self, n: usize) -> Self`


**`impl WorkflowWatcher`**
  `pub fn from_config(mut cfg: WatcherConfig) -> Self`

  `pub async fn collect_workflow_files(&self) -> Result<Vec<PathBuf>, WatchError>`

  `pub async fn run<F>( &self, shutdown: ShutdownSignal, on_cycle_complete: F, ) -> Result<(), WatchError> where F: Fn(WatchEvent) + Send + Sync + 'static,`

  `async fn cached_git_state( &self, ) -> Result<(Option<String>, Option<String>), wrkflw_trigger_filter::TriggerFilterError>`


`mod tests`

---

## crates/wrkflw/Cargo.toml

**Language:** TOML | **Size:** 1.8 KB | **Lines:** 78

**Imports:**
- `nix`
- `walkdir`
- `jsonc-parser`
- `toml`

**Declarations:**

---

## crates/wrkflw/README.md

**Language:** Markdown | **Size:** 7.5 KB | **Lines:** 171

**Declarations:**

---

## crates/wrkflw/src/cli_secrets.rs

**Language:** Rust | **Size:** 1.7 KB | **Lines:** 49

**Imports:**
- `std::sync::OnceLock`

**Declarations:**

`pub(crate) const SIGNING_KEY: &str = "WRKFLW_CACHE_SIGNING_KEY"`

`pub(crate) const REMOTE_TOKEN: &str = "WRKFLW_REMOTE_CACHE_TOKEN"`

`const NAMES: [&str`

`static CAPTURED: OnceLock<Vec<(&'static str, String)>> = OnceLock::new()`

`pub(crate) fn capture()`

`pub(crate) fn get(name: &str) -> Option<String>`

`pub(crate) fn pass_to(command: &mut std::process::Command)`

---

## crates/wrkflw/src/compare_cmd.rs

**Language:** Rust | **Size:** 53.1 KB | **Lines:** 1411

**Imports:**
- `crate::git::line as git_line`
- `crate::{OutputFormat, RuntimeChoice}`
- `serde_json::{json, Value}`
- `std::path::Path`
- `std::process::{Command, Stdio}`
- `wrkflw_github::{RemoteJob, RemoteRun}`
- `wrkflw_runs::SCHEMA_VERSION`
- `wrkflw_ui::cli_style`

**Declarations:**

`pub(crate) struct CompareCtx`
> Fields: `run: String`, `runtime: RuntimeChoice`, `child_args: Vec<String>`, `format: OutputFormat`

`struct ExpectedJob`
> Fields: `local: String`, `github: Option<String>`, `github_base: String`, `matrix_values: Option<Vec<String>>`

**`impl ExpectedJob`**
  `fn matches(&self, name: &str) -> bool`


`fn is_joined_in_some_order(text: &str, values: &[&str]) -> bool`

`fn value_text(value: &serde_yaml::Value) -> Option<String>`

`fn reads_matrix(name: &str) -> bool`

`fn expected_jobs(path: &Path) -> Result<Vec<ExpectedJob>, String>`

`struct Row`
> Fields: `github_name: Option<String>`, `local_name: Option<String>`, `github: Option<String>`, `local: Option<String>`, `agree: Option<bool>`, `note: Option<String>`

`fn agree(conclusion: Option<&str>, status: Option<&str>) -> Option<bool>`

`fn note_for(local: &Value, failures: &[Value]) -> Option<String>`

`fn rows( expected: &[ExpectedJob], remote: &[RemoteJob], local: &[Value], failures: &[Value], ) -> Vec<Row>`

`fn ambiguous_note(name: &str, count: usize) -> String`

`fn join_notes(first: Option<String>, second: Option<String>) -> Option<String>`

`fn workflow_path(path: &str) -> Option<&Path>`

`const TAG_EVENTS: &[&str] = &[ "push", "create", "release", "workflow_dispatch", "workflow_run", ]`

`fn head_is_tag(remote: &RemoteRun) -> Result<bool, String>`

`fn head_is_tag_with( remote: &RemoteRun, git: impl Fn(&[&str]) -> Option<String>, origin_has: impl Fn(&str) -> Result<bool, String>, ) -> Result<bool, String>`

`fn run_args( remote: &RemoteRun, path: &Path, runtime: &str, child_args: &[String], head_is_tag: bool, ) -> Vec<String>`

`fn is_pushed_over(before: &str) -> bool`

`struct Outcome`
> Fields: `disagreements: usize`, `basis: &'static str`, `problem: Option<String>`

`fn outcome(remote: &RemoteRun, local: &Value, rows: &[Row]) -> Outcome`

`fn is_unpaired(row: &Row) -> bool`

`fn unpaired_result(row: &Row) -> Option<&str>`

`fn paired_outcome( remote: &RemoteRun, status: &str, rows: &[Row], job_disagreements: usize, ) -> Outcome`

`fn report(remote: &RemoteRun, local: &Value, rows: &[Row]) -> Value`

`fn print_text(report: &Value)`

`pub(crate) async fn run(ctx: CompareCtx) -> !`

`mod tests`

---

## crates/wrkflw/src/gate_cmd.rs

**Language:** Rust | **Size:** 46.2 KB | **Lines:** 1249

**Imports:**
- `crate::{is_gitlab_pipeline, OutputFormat, RuntimeChoice}`
- `serde_json::{json, Value}`
- `std::path::{Path, PathBuf}`
- `std::process::{Command, Stdio}`
- `wrkflw_executor::vm_memory`
- `wrkflw_runs::SCHEMA_VERSION`
- `wrkflw_ui::cli_style`

**Declarations:**

`const HOOK_MARKER: &str = "# installed by wrkflw gate"`

`const SKIPPED_EXIT: i32 = 78`

`const NOTHING_CHECKED_EXIT: i32 = 79`

`fn ran_nothing(code: Option<i32>) -> bool`

`const PARTIAL_EXIT: i32 = 80`

`fn passed(code: Option<i32>) -> bool`

`pub(crate) struct GateCtx`
> Fields: `path: Option<PathBuf>`, `event: Option<String>`, `base_branch: Option<String>`, `diff_base: Option<String>`, `runtime: RuntimeChoice`, `remote_cache: Option<String>`, `install_hook: bool`, `child_args: Vec<String>`, `format: OutputFormat`

`fn shell_quote(value: &str) -> String`

`fn hook_script( program: &Path, runtime: &str, remote_cache: Option<&str>, child_args: &[String], ) -> String`

`fn git_path_query(args: &[&str]) -> Option<PathBuf>`

`fn resolve_existing(path: &Path) -> PathBuf`

`fn hook_is_local(hook: &Path, roots: &[PathBuf]) -> bool`

`fn hook_path() -> Result<PathBuf, String>`

`fn install_hook_at(path: &Path, script: &str) -> Result<(), String>`

`fn uncommitted_changes() -> Vec<String>`

`fn dirty_tree_warning(changes: &[String]) -> Option<String>`

`fn workflow_files(path: Option<&Path>) -> Result<Vec<PathBuf>, String>`

`struct Event`
> Fields: `name: String`, `base_branch: Option<String>`

**`impl Event`**
  `fn describe(&self) -> String`


`fn events(ctx: &GateCtx, branch: Option<&str>, default: Option<&str>) -> (Event, Option<Event>)`

`fn run_args(ctx: &GateCtx, file: &Path, event: &Event) -> Vec<String>`

`fn workflow_report(file: &Path, code: Option<i32>, stdout: &str) -> Value`

`fn gate_verdict(workflows: &[Value], failed: usize, unverified: usize) -> &'static str`

`fn gate_report(workflows: Vec<Value>, uncommitted: &[String]) -> Value`

`fn checked_nothing(entry: &Value) -> bool`

`fn why_nothing_ran(entry: &Value) -> String`

`fn nothing_checked_warning(first: &Event, second: Option<&Event>) -> String`

`fn probe_memory(runtime: &str) -> Option<vm_memory::Memory>`

`fn probed_env(runtime: &str, memory: Option<&vm_memory::Memory>) -> Option<String>`

`struct Launcher<'a>`
> Fields: `program: &'a Path`, `runtime: &'a str`, `probed: Option<&'a str>`

**`impl Launcher<'_>`**
  `fn check(&self, ctx: &GateCtx, file: &Path, event: &Event, json: bool) -> Value`

  `fn command(&self, ctx: &GateCtx, file: &Path, event: &Event, json: bool) -> Command`


`pub(crate) fn run(ctx: GateCtx)`

`fn check_workflow( first: &Event, second: Option<&Event>, json_mode: bool, mut check: impl FnMut(&Event) -> Value, ) -> Value`

`fn text_entry(file: &Path, code: Option<i32>) -> Value`

`mod tests`

---

## crates/wrkflw/src/git.rs

**Language:** Rust | **Size:** 6.7 KB | **Lines:** 182

**Imports:**
- `std::process::Command`

**Declarations:**

`pub(crate) fn line(args: &[&str]) -> Option<String>`

`pub(crate) fn remote_has(remote: &str, reference: &str) -> Result<bool, String>`

`const REMOTE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30)`

`fn output_within( command: &mut Command, deadline: std::time::Duration, ) -> Result<std::process::Output, String>`

`fn command(args: &[&str]) -> Command`

`fn batch_ssh_wanted(env_set: impl Fn(&str) -> bool, configured: impl Fn() -> bool) -> bool`

`fn ssh_command_configured() -> bool`

`pub(crate) fn current_branch() -> Option<String>`

`pub(crate) fn default_branch() -> Option<String>`

`mod tests`

---

## crates/wrkflw/src/json_output.rs

**Language:** Rust | **Size:** 4.7 KB | **Lines:** 145

**Imports:**
- `crate::is_gitlab_pipeline`
- `serde_json::{json, Value}`
- `std::path::{Path, PathBuf}`
- `wrkflw_models::ValidationResult`
- `wrkflw_runs::SCHEMA_VERSION`

**Declarations:**

`fn yaml_files(dir: &Path) -> std::io::Result<Vec<PathBuf>>`

`fn file_report( path: &Path, kind: &str, outcome: Result<ValidationResult, (&str, String)>, ) -> Value`

`fn yaml_error_line(message: &str) -> Option<usize>`

`fn validate_file(path: &Path, force_gitlab: bool) -> Value`

`pub(crate) fn validate(paths: &[PathBuf], force_gitlab: bool) -> bool`

`fn workflow_entry(path: &Path) -> Value`

`fn pipeline_entry(path: &Path) -> Value`

`pub(crate) fn list()`

---

## crates/wrkflw/src/lib.rs

**Language:** Rust | **Size:** 408 B | **Lines:** 12

**Imports:**
- `pub use wrkflw_evaluator as evaluator`
- `pub use wrkflw_executor as executor`
- `pub use wrkflw_github as github`
- `pub use wrkflw_gitlab as gitlab`
- `pub use wrkflw_logging as logging`
- `pub use wrkflw_matrix as matrix`
- `pub use wrkflw_models as models`
- `pub use wrkflw_parser as parser`
- `pub use wrkflw_runtime as runtime`
- `pub use wrkflw_ui as ui`
- *... and 2 more imports*

---

## crates/wrkflw/src/main.rs

**Language:** Rust | **Size:** 92.1 KB | **Lines:** 2507

**Imports:**
- `bollard::Docker`
- `clap::{Parser, Subcommand, ValueEnum}`
- `std::collections::HashMap`
- `std::path::Path`
- `std::path::PathBuf`

**Declarations:**

`mod cli_secrets`

`mod compare_cmd`

`mod gate_cmd`

`mod git`

`mod json_output`

`mod mcp_cmd`

`mod mcp_install`

`mod plan_cmd`

`mod prefilter`

`mod run_workflow_cmd`

`mod runs_cmd`

`mod watch_cmd`

`pub(crate) enum OutputFormat`
> Variants: `Text`, `Json`

**`impl OutputFormat`**
  `pub(crate) fn is_json(self) -> bool`

  `pub(crate) fn fail(self, message: &str) -> !`


`pub(crate) enum RuntimeChoice`
> Variants: `Auto`, `Docker`, `Podman`, `Kubernetes`, `Emulation`, `SecureEmulation`

**`impl RuntimeChoice`**
  `pub(crate) fn as_flag(&self) -> &'static str`


**`impl From<RuntimeChoice> for wrkflw_executor::RuntimeType`**
  `fn from(choice: RuntimeChoice) -> Self`


`struct Wrkflw`
> Fields: `command: Option<Commands>`, `verbose: bool`, `debug: bool`, `format: OutputFormat`, `theme: Option<wrkflw_ui::theme::ThemePreference>`, `platform: Vec<Option<(String, String)>>`, `fresh_workspace: bool`, `every_matrix_leg: bool`, `kubernetes: KubernetesArgs`

`struct KubernetesArgs`
> Fields: `k8s_context: Option<String>`, `k8s_namespace: Option<String>`, `k8s_node_selector: Vec<Option<(String, String)>>`, `k8s_resource: Vec<Option<(String, String)>>`, `k8s_image_pull_secret: Vec<String>`, `k8s_registry: Option<String>`, `k8s_helper_image: Option<String>`, `k8s_timeout: Option<u64>`, `k8s_schedule_timeout: Option<u64>`

**`impl KubernetesArgs`**
  `fn child_args(&self) -> Vec<String>`

  `fn to_config(&self) -> wrkflw_executor::KubernetesConfig`


`enum Commands`
> Variants: `Validate`, `Run`, `Gate`, `Compare`, `Watch`, `Tui`, `Trigger`, `TriggerGitlab`, `List`, `Plan`, `Clean`, `Runs`, `Mcp`

`enum McpCommand`
> Variants: `Serve`, `Install`

`fn parse_platform_arg(s: &str) -> Result<Option<(String, String)>, String>`

`fn parse_k8s_key_val(s: &str) -> Result<Option<(String, String)>, String>`

`fn collect_platform_overrides(entries: &[Option<(String, String)>]) -> HashMap<String, String>`

`fn global_child_args(cli: &Wrkflw, platform_overrides: &HashMap<String, String>) -> Vec<String>`

`fn hook_child_args( cli: &Wrkflw, matches: &clap::ArgMatches, platform_overrides: &HashMap<String, String>, ) -> Vec<String>`

`fn given_or_empty<T: Clone>(values: &[T], given: bool) -> Vec<T>`

`fn parse_branch(value: &str) -> Result<String, String>`

`fn parse_tag(value: &str) -> Result<String, String>`

`fn ref_name(value: &str, prefix: &str) -> Result<String, String>`

`fn event_for(event: Option<&str>, branch: Option<&str>, tag: Option<&str>) -> Option<String>`

`fn check_tag_event(event: Option<&str>, tag: Option<&str>) -> Result<(), String>`

`pub(crate) fn platform_args(overrides: &HashMap<String, String>) -> Vec<String>`

`fn parse_key_val(s: &str) -> Result<(String, String), String>`

`async fn cleanup_on_exit()`

`async fn handle_signals()`

`pub(crate) fn is_gitlab_pipeline(path: &Path) -> bool`

`fn raw_args() -> impl Iterator<Item = String>`

`fn args_request_json(args: impl Iterator<Item = String>) -> bool`

`fn exit_on_parse_error(error: clap::Error) -> !`

`const CLI_STACK_BYTES: usize = 16 * 1024 * 1024`

`fn main()`

`async fn run_cli()`

`fn validate_github_workflow(path: &Path, verbose: bool) -> bool`

`fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool`

`fn list_workflow_dir(dir: &Path, show_jobs: bool)`

`fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool)`

`fn human_bytes(bytes: u64) -> String`

`mod tests`

---

## crates/wrkflw/src/mcp_cmd.rs

**Language:** Rust | **Size:** 33.7 KB | **Lines:** 814

**Imports:**
- `serde_json::{json, Value}`
- `std::process::{Output, Stdio}`
- `std::sync::Arc`
- `std::time::{Duration, Instant}`
- `tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader}`
- `tokio::process::Command`
- `tokio::sync::Mutex`

**Declarations:**

`const SUPPORTED_PROTOCOLS: [&str`

`const DEFAULT_RUN_TIMEOUT_SECS: u64 = 1800`

`const DEFAULT_GATE_TIMEOUT_SECS: u64 = 3600`

`const QUICK_TIMEOUT_SECS: u64 = 120`

`const STOP_GRACE: Duration = Duration::from_secs(15)`

`static LIVE: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new())`

`fn live_pids() -> Vec<u32>`

`struct Live(Option<u32>)`

**`impl Live`**
  `fn track(pid: Option<u32>) -> Self`


**`impl Drop for Live`**
  `fn drop(&mut self)`


`const INSTRUCTIONS: &str = "wrkflw validates and runs GitHub Actions and GitLab CI workflows locally. \ Call validate_workflows after editing YAML, plan_workflow to see what would run and why a trigger \ would or would not fire, and run_workflow to execute. run_workflow returns a failure digest. Pass \ reuse_results while iterating, so jobs that already passed on the same files aren't run again. \ Before pushing, call gate: it runs what CI would run for the push, and on a feature branch \ checks workflows a push doesn't fire as a pull request into the default branch. Read \ `verdict`, not only `passed`: `verified` means everything ran as asked, `partial` means the \ pass left something unchecked, and `nothing_checked` means no job ran at all. Each `unverified` \ entry has a `kind` and a `reason`: `runner` is a job that ran as another OS than its runs-on asked \ for, so its OS-specific steps weren't checked`

`fn string_arg(args: &Value, key: &str) -> Option<String>`

`fn push_opt(cli: &mut Vec<String>, flag: &str, value: Option<String>)`

`fn changed_files(args: &Value) -> Option<String>`

`fn event_flags(cli: &mut Vec<String>, args: &Value)`

`fn required_path(args: &Value) -> Result<String, String>`

`fn run_ref(args: &Value) -> String`

`fn tool_command(name: &str, args: &Value) -> Result<(Vec<String>, u64), String>`

`fn tool_result(stdout: &str, stderr: &str, success: bool) -> Value`

`fn error_result(message: &str) -> Value`

`async fn call_tool(name: &str, args: &Value, agent: Option<&str>) -> Value`

`async fn run_with_deadline( child: tokio::process::Child, limit: Duration, grace: Duration, ) -> Option<std::io::Result<Output>>`

`async fn stop_groups(pids: &[u32], grace: Duration)`

`async fn shutdown_signal() -> i32`

`async fn shutdown_signal() -> i32`

`enum Stop`
> Variants: `Interrupt`, `Kill`

`fn signal_group(pid: u32, stop: Stop)`

`fn signal_group(_pid: u32, _stop: Stop)`

`fn tool( name: &str, description: &str, read_only: bool, properties: Value, required: &[&str], ) -> Value`

`fn event_properties() -> Value`

`fn tools() -> Value`

`fn negotiate(requested: Option<&str>) -> &'static str`

`async fn handle( method: &str, params: &Value, client: &Mutex<Option<String>>, ) -> Result<Value, (i64, String)>`

`async fn respond( request: Value, client: Arc<Mutex<Option<String>>>, out: Arc<Mutex<tokio::io::Stdout>>, )`

`async fn write_message(out: &Mutex<tokio::io::Stdout>, message: &Value)`

`pub(crate) async fn serve()`

`mod tests`

---

## crates/wrkflw/src/mcp_install.rs

**Language:** Rust | **Size:** 47.5 KB | **Lines:** 1322

**Imports:**
- `crate::OutputFormat`
- `clap::ValueEnum`
- `jsonc_parser::cst::{CstInputValue, CstRootNode}`
- `jsonc_parser::ParseOptions`
- `serde_json::{json, Value}`
- `std::ffi::OsString`
- `std::path::{Path, PathBuf}`
- `std::process::Command`
- `wrkflw_runs::SCHEMA_VERSION`

**Declarations:**

`const SERVER: &str = "wrkflw"`

`const SERVE_ARGS: [&str`

`pub(crate) enum Client`
> Variants: `ClaudeCode`, `Codex`, `Cursor`, `Windsurf`, `Vscode`, `Gemini`, `Opencode`, `ClaudeDesktop`, `Copilot`, `Grok`, `Kiro`, `Pi`, `Zed`

**`impl Client`**
  `fn name(self) -> String`


`enum Method`
> Variants: `Cli`, `Json`

`struct Dirs`
> Fields: `home: PathBuf`, `config: PathBuf`, `claude: PathBuf`, `copilot: PathBuf`, `grok: PathBuf`, `kiro: PathBuf`, `pi: PathBuf`, `zed: PathBuf`

**`impl Dirs`**
  `fn current() -> Option<Self>`

  `fn resolve(home: PathBuf, config: PathBuf, var: impl Fn(&str) -> Option<OsString>) -> Self`


`fn kiro_home(home: &Path, configured: Option<OsString>) -> PathBuf`

`fn expand_tilde(home: &Path, dir: OsString) -> PathBuf`

`fn zed_dir(home: &Path, config: &Path) -> PathBuf`

`struct Launch`
> Fields: `command: String`, `absolute: String`

**`impl Launch`**
  `fn detect() -> Self`

  `fn for_client(&self, client: Client) -> &str`


`fn args() -> Vec<String>`

`fn method(client: Client, dirs: &Dirs, command: &str) -> Method`

`fn opencode_config(dir: &Path) -> PathBuf`

`fn detected(method: &Method) -> bool`

`enum Status`
> Variants: `Added`, `Replaced`, `Unchanged`, `Differs`, `WouldAdd`, `WouldReplace`, `NotFound`, `Failed`

**`impl Status`**
  `fn as_str(self) -> &'static str`

  `fn is_failure(self) -> bool`

  `fn reason(self) -> Option<String>`


`struct Outcome`
> Fields: `client: Client`, `status: Status`, `target: String`, `detail: Option<String>`

`struct Policy`
> Fields: `dry_run: bool`, `force: bool`

**`impl Policy`**
  `fn settle(self, existing: Option<bool>) -> (Status, bool)`


`const LAUNCH_KEYS: [&str`

`fn same_launch(current: &Value, wanted: &Value) -> bool`

`fn to_cst(value: &Value) -> CstInputValue`

`fn upsert_json( text: Option<&str>, key: &str, entry: &Value, policy: Policy, ) -> Result<(Status, Option<String>), String>`

`fn resolve_links(path: &Path) -> PathBuf`

`fn write_atomically(path: &Path, text: &str) -> std::io::Result<()>`

`fn install_json(path: &Path, key: &str, entry: &Value, policy: Policy) -> (Status, Option<String>)`

`fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String>`

`fn check(program: &str, args: &[&str]) -> Result<(), String>`

`fn claude_code_existing(dirs: &Dirs, command: &str) -> Option<bool>`

`fn grok_existing(dirs: &Dirs, command: &str) -> Option<bool>`

`fn toml_server_matches(text: &str, command: &str) -> Option<bool>`

`fn codex_existing(command: &str) -> Option<bool>`

`fn codex_matches(stdout: &[u8], command: &str) -> bool`

`fn install_cli( program: &str, scope: &[&str], existing: Option<bool>, command: &str, policy: Policy, ) -> (Status, Option<String>)`

`fn install_one( client: Client, dirs: &Dirs, launch: &Launch, policy: Policy, named: bool, ) -> Option<Outcome>`

`fn report_json(outcomes: &[Outcome], launch: &Launch) -> Value`

`fn report(outcomes: &[Outcome], format: OutputFormat, launch: &Launch, policy: Policy)`

`pub(crate) fn install(clients: &[Client], dry_run: bool, force: bool, format: OutputFormat)`

`mod tests`

---

## crates/wrkflw/src/plan_cmd.rs

**Language:** Rust | **Size:** 12.5 KB | **Lines:** 362

**Imports:**
- `crate::{is_gitlab_pipeline, prefilter, OutputFormat, RuntimeChoice}`
- `serde_json::{json, Value}`
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `wrkflw_executor::runner_fidelity::{requested_os, runner_os_for, RequestedOs}`
- `wrkflw_parser::workflow::{Job, WorkflowDefinition}`
- `wrkflw_runs::SCHEMA_VERSION`
- `wrkflw_secrets::{SecretError, SecretManager, SecretSubstitution}`

**Declarations:**

`pub(crate) struct PlanCtx`
> Fields: `path: PathBuf`, `gitlab: bool`, `job: Option<String>`, `event: Option<String>`, `diff: bool`, `changed_files: Option<Vec<String>>`, `diff_base: Option<String>`, `diff_head: Option<String>`, `base_branch: Option<String>`, `branch: Option<String>`, `tag: Option<String>`, `activity_type: Option<String>`, `strict_filter: bool`, `no_strict_filter: bool`, `verbose: bool`, `format: OutputFormat`, `runtime: RuntimeChoice`, `platform_overrides: HashMap<String, String>`

`async fn trigger(ctx: &PlanCtx) -> Result<Value, String>`

`async fn missing_secrets(path: &Path) -> Result<Vec<String>, String>`

`fn matrix_json(job: &Job) -> Value`

`fn job_json(id: &str, job: &Job) -> Value`

`fn requested( id: &str, job: &Job, overrides: &HashMap<String, String>, ) -> Vec<(String, RequestedOs)>`

`fn unverified(requests: &[(String, RequestedOs)], runtime: RuntimeChoice) -> Vec<Value>`

`type GithubPlan = (Vec<Vec<String>>, Vec<Value>, Vec<Value>)`

`fn github_plan(ctx: &PlanCtx, workflow: &WorkflowDefinition) -> Result<GithubPlan, String>`

`fn gitlab_plan(ctx: &PlanCtx) -> Result<(Vec<Vec<String>>, Vec<Value>), String>`

`async fn build(ctx: &PlanCtx) -> Result<Value, String>`

`fn print_text(plan: &Value)`

`pub(crate) async fn run(ctx: PlanCtx)`

---

## crates/wrkflw/src/prefilter.rs

**Language:** Rust | **Size:** 40.9 KB | **Lines:** 984

**Imports:**
- `std::path::{Path, PathBuf}`

**Declarations:**

`pub(crate) enum PrefilterDecision`
> Variants: `Proceed`, `Skip`

`pub(crate) fn effective_strict_filter(strict: bool, no_strict: bool) -> bool`

`pub(crate) fn validate_event_requires_base_branch( event_name: &str, strict_filter: bool, ) -> Result<(), String>`

`pub(crate) struct PrefilterRequest<'a>`
> Fields: `workflow_path: &'a Path`, `event: Option<&'a String>`, `diff: bool`, `changed_files: Option<&'a Vec<String>>`, `diff_base: Option<&'a str>`, `diff_head: Option<&'a String>`, `base_branch: Option<&'a String>`, `branch: Option<&'a String>`, `tag: Option<&'a String>`, `activity_type: Option<&'a String>`, `verbose: bool`, `strict_filter: bool`

`pub(crate) async fn run_trigger_prefilter( req: PrefilterRequest<'_>, ) -> Result<PrefilterDecision, String>`

`pub(crate) async fn build_event_context( req: &PrefilterRequest<'_>, event_name: &str, paths_matter: bool, cwd_for_git: Option<&Path>, ) -> Result<wrkflw_trigger_filter::EventContext, String>`

`pub(crate) fn apply_base_branch( ctx: &mut wrkflw_trigger_filter::EventContext, event_name: &str, base_branch: Option<&String>, strict_filter: bool, ) -> Result<(), String>`

`mod prefilter_tests`

---

## crates/wrkflw/src/run_workflow_cmd.rs

**Language:** Rust | **Size:** 20.8 KB | **Lines:** 549

**Imports:**
- `crate::prefilter`
- `crate::{is_gitlab_pipeline, OutputFormat, RuntimeChoice}`
- `std::collections::HashMap`
- `std::path::PathBuf`

**Declarations:**

`pub(crate) struct RunCtx`
> Fields: `path: PathBuf`, `runtime: RuntimeChoice`, `show_action_messages: bool`, `preserve_containers_on_failure: bool`, `gitlab: bool`, `job: Option<String>`, `variables: HashMap<String, String>`, `event: Option<String>`, `diff: bool`, `changed_files: Option<Vec<String>>`, `diff_base: Option<String>`, `diff_head: Option<String>`, `base_branch: Option<String>`, `branch: Option<String>`, `tag: Option<String>`, `activity_type: Option<String>`, `strict_filter: bool`, `no_strict_filter: bool`, `verbose: bool`, `platform_overrides: HashMap<String, String>`, `kubernetes: wrkflw_executor::KubernetesConfig`, `format: OutputFormat`, `intent: Option<String>`, `agent: Option<String>`, `session: Option<String>`, `no_record: bool`, `from_step: Option<String>`, `reuse_results: bool`, `affected: bool`, `job_container: bool`, `dep_cache: bool`, `remote_cache: Option<String>`, `cache_push: bool`, `skip_exit_code: Option<i32>`, `nothing_checked_exit_code: Option<i32>`, `partial_exit_code: Option<i32>`, `gate_mode: bool`, `stream_output: bool`

`fn secret_from_env(name: &str) -> Option<wrkflw_executor::Secret>`

`fn acceleration_for( ctx: &RunCtx, changed_files: Option<Vec<String>>, ) -> wrkflw_executor::Acceleration`

`fn resolve_from_step( ctx: &RunCtx, is_gitlab: bool, ) -> Result<Option<wrkflw_executor::FromStep>, String>`

`pub(crate) fn replayable_argv() -> Vec<String>`

`fn strip_variable_args(args: impl Iterator<Item = String>) -> Vec<String>`

`fn variable_in_short_cluster(arg: &str) -> Option<(&str, bool)>`

`pub(crate) async fn run(ctx: RunCtx)`

`mod tests`

---

## crates/wrkflw/src/runs_cmd.rs

**Language:** Rust | **Size:** 13.5 KB | **Lines:** 423

**Imports:**
- `crate::OutputFormat`
- `clap::Subcommand`
- `serde_json::{json, Value}`
- `wrkflw_runs::{
    failure_digest, render_digest, RunRecord, RunStatus, RunStore, DEFAULT_TAIL_LINES,
    SCHEMA_VERSION,
}`

**Declarations:**

`pub(crate) enum RunsCommand`
> Variants: `List`, `Show`, `Logs`, `Failure`, `Replay`

`fn die(format: OutputFormat, message: &str) -> !`

`fn open_store(format: OutputFormat) -> RunStore`

`async fn current_repo() -> String`

`async fn resolve(store: &RunStore, reference: &str, format: OutputFormat) -> RunRecord`

`fn summary_json(run: &RunRecord) -> Value`

`fn tail_text(text: &str, lines: Option<usize>) -> String`

`fn print_summary(run: &RunRecord)`

`fn print_run(run: &RunRecord)`

`fn logs( store: &RunStore, run: &RunRecord, job: Option<&str>, step: Option<&str>, tail: Option<usize>, ) -> Vec<Value>`

`fn replay_args(argv: &[String], variables: &[String], format: OutputFormat) -> Vec<String>`

`fn replay(run: &RunRecord, variables: &[String], format: OutputFormat) -> !`

`pub(crate) async fn run(command: RunsCommand, format: OutputFormat)`

`mod tests`

---

## crates/wrkflw/src/watch_cmd.rs

**Language:** Rust | **Size:** 12.3 KB | **Lines:** 302

**Imports:**
- `crate::prefilter`
- `crate::{OutputFormat, RuntimeChoice}`
- `serde_json::json`
- `std::collections::HashMap`
- `std::path::PathBuf`

**Declarations:**

`pub(crate) struct WatchCtx`
> Fields: `path: Option<PathBuf>`, `runtime: RuntimeChoice`, `debounce: u64`, `event: String`, `show_action_messages: bool`, `preserve_containers_on_failure: bool`, `max_concurrency: usize`, `base_branch: Option<String>`, `activity_type: Option<String>`, `max_pending_events: Option<usize>`, `ignore_dirs: Vec<String>`, `strict_filter: bool`, `no_strict_filter: bool`, `verbose: bool`, `platform_overrides: HashMap<String, String>`, `kubernetes: wrkflw_executor::KubernetesConfig`, `format: OutputFormat`

`pub(crate) async fn run(ctx: WatchCtx)`

---

## crates/wrkflw/tests/acceleration_test.rs

**Language:** Rust | **Size:** 25.8 KB | **Lines:** 781

**Imports:**
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::process::{Command, Output}`
- `tempfile::TempDir`

**Declarations:**

`const NO_GIT_CONFIG: &str = "/dev/null"`

`struct Repo`
> Fields: `dir: TempDir`, `results: TempDir`, `remote: TempDir`, `log: PathBuf`

**`impl Repo`**
  `fn new(workflow: &str, sidecar: Option<&str>) -> Self`

  `fn root(&self) -> &Path`

  `fn git(&self, args: &[&str])`

  `fn wrkflw(&self, args: &[&str]) -> Output`

  `fn run(&self, flags: &[&str]) -> Output`

  `fn take_ran(&self) -> Vec<String>`


`fn assert_ok(output: &Output)`

`const TWO_JOBS: &str = r#" name: ci on: push jobs: build: runs-on: ubuntu-latest steps: - run: echo build >> LOG docs: runs-on: ubuntu-latest steps: - run: echo docs >> LOG "#`

`const HINTS: &str = r#" jobs: build:`

`fn a_second_run_of_the_same_tree_runs_nothing()`

`fn without_the_flag_every_job_runs()`

`fn a_change_reruns_only_the_jobs_that_read_it()`

`fn a_failed_job_is_never_reused()`

`fn reused_outputs_still_reach_dependent_jobs()`

`fn a_partial_run_neither_stores_nor_reuses()`

`fn a_job_reading_secrets_always_runs()`

`fn affected_skips_jobs_with_no_changed_input()`

`fn affected_sees_a_new_untracked_file()`

`fn a_reused_job_and_an_unaffected_job_report_differently()`

`fn affected_runs_a_job_without_hints()`

`fn a_broken_sidecar_stops_the_run()`

`fn a_remote_cache_needs_a_signing_key()`

`fn the_gate_fails_when_a_workflow_fails_and_passes_when_it_is_fixed()`

`fn the_gate_skips_a_workflow_the_event_does_not_trigger()`

`fn on_a_feature_branch_the_gate_checks_pull_request_ci_as_a_pull_request()`

`fn a_workflow_whose_jobs_are_all_skipped_checks_nothing_in_either_format()`

`fn the_gate_counts_a_passing_workflow_as_run()`

`fn an_emulated_result_is_not_pushed()`

`fn the_gate_installs_a_pre_push_hook()`

`fn every_matrix_leg_is_reused_and_reaches_dependents()`

`const DEPLOY: &str = r#" name: ci on: push jobs: check: runs-on: ubuntu-latest steps: - run: echo check >> LOG deploy: runs-on: ubuntu-latest needs: check steps: - run: echo deploy >> LOG`

`fn the_gate_leaves_a_job_to_ci_when_the_sidecar_says_so()`

`fn the_gate_warns_about_uncommitted_changes()`

`fn a_step_never_sees_the_cache_credentials()`

`fn a_reused_job_with_new_outputs_still_reruns_its_dependents_under_affected()`

`fn the_gate_skips_the_jobs_that_need_a_job_left_to_ci()`

`fn host_os() -> &'static str`

`fn json_lines(output: &Output) -> Vec<serde_json::Value>`

`const WINDOWS_JOB: &str = "name: ci\non: push\njobs:\n win:\n runs-on: windows-latest\n steps:\n - uses: actions/checkout@v4\n - run: echo win >> LOG`

`fn a_run_marks_a_job_that_ran_as_another_os()`

`fn the_gate_prints_one_json_object_naming_the_failed_run()`

`const OS_MATRIX: &str = "name: ci\non: push\njobs:\n build:\n runs-on: $`

`fn a_reused_os_matrix_leg_is_still_unverified()`

`fn the_gate_reports_an_installed_hook_as_json()`

`fn a_reused_partial_pass_stays_partial()`

---

## crates/wrkflw/tests/agent_cli_test.rs

**Language:** Rust | **Size:** 48.1 KB | **Lines:** 1463

**Imports:**
- `serde_json::{json, Value}`
- `std::fs`
- `std::io::{BufRead, BufReader, Write}`
- `std::path::{Path, PathBuf}`
- `std::process::{Command, Output, Stdio}`
- `tempfile::TempDir`

**Declarations:**

`const BIN: &str = env!("CARGO_BIN_EXE_wrkflw")`

`const FAILING: &str = r#" name: failing on: push: paths: ['src/**'] jobs: build: runs-on: ubuntu-latest steps: - name: export run: echo "STAGE=two" >> "$GITHUB_ENV" - name: leak env: SECRET_VALUE: $`

`const PASSING: &str = r#" name: passing on: [push] jobs: first: runs-on: ubuntu-latest steps: - run: echo one second: needs: first runs-on: ubuntu-latest strategy: matrix: n: [1, 2] steps: - name: marker run: echo "ran-$`

`const BROKEN: &str = "name: broken\non: [nonsense]\njobs:\n build:\n runs-on: ubuntu-latest\n steps:\n - name: nothing\n"`

`struct Sandbox`
> Fields: `home: TempDir`, `repo: TempDir`

**`impl Sandbox`**
  `fn new() -> Self`

  `fn write(&self, name: &str, text: &str) -> PathBuf`

  `fn command(&self, args: &[&str]) -> Command`

  `fn run(&self, args: &[&str]) -> Output`

  `fn stored_text(&self) -> String`


`fn stdout(output: &Output) -> String`

`fn json_lines(output: &Output) -> Vec<Value>`

`fn json_doc(output: &Output) -> Value`

`const RUN_FAILING: &[&str] = &[ "--format", "json", "run", "--runtime", "emulation", ".github/workflows/failing.yml", ]`

`fn run_streams_only_json_and_reports_the_failure()`

`fn stream_output_reports_lines_before_the_step_ends_and_masks_them()`

`fn stream_output_without_json_is_refused()`

`fn history_records_runs_and_answers_without_a_rerun()`

`fn listings_are_scoped_to_the_repository()`

`fn secrets_never_reach_output_or_the_store()`

`fn variables_are_not_stored_and_replay_keeps_the_arguments()`

`fn variable_values_printed_by_a_step_are_masked()`

`fn replay_works_after_a_path_separator()`

`fn replay_with_variables_works_after_a_path_separator()`

`fn matrix_legs_with_similar_names_keep_their_own_logs()`

`fn no_record_leaves_no_history_but_keeps_the_digest()`

`fn retention_prunes_the_oldest_runs()`

`fn from_step_starts_at_the_named_step()`

`fn plan_explains_without_executing()`

`fn plan_names_the_legs_that_would_run_as_another_os()`

`fn validate_reports_structured_issues_and_keeps_exit_codes()`

`fn list_reports_workflows_as_json()`

`struct Mcp`
> Fields: `child: std::process::Child`, `reader: BufReader<std::process::ChildStdout>`, `next_id: u64`

**`impl Mcp`**
  `fn start(sandbox: &Sandbox) -> Self`

  `fn send(&mut self, message: Value)`

  `fn call(&mut self, method: &str, params: Value) -> Value`

  `fn tool(&mut self, name: &str, arguments: Value) -> (Value, bool)`


**`impl Drop for Mcp`**
  `fn drop(&mut self)`


`fn mcp_server_speaks_the_protocol_and_runs_tools()`

`fn mcp_timeout_stops_the_run_and_replay_of_an_mcp_run_works()`

`fn text_mode_prints_the_digest_on_failure()`

`fn watch_streams_json_cycles_and_records_runs()`

`fn a_missing_secret_is_empty_and_named_as_on_github()`

`fn identical_matrix_failures_fold_and_paths_are_relative()`

`fn unnamed_action_steps_report_the_action()`

`fn from_step_failures_say_earlier_steps_did_not_run()`

`fn a_missing_workflow_is_an_error_and_not_a_run()`

`fn usage_errors_are_json_in_json_mode()`

`fn validate_catches_filter_and_expression_mistakes()`

`fn a_provider_failure_is_not_reported_as_a_missing_secret()`

`fn a_secret_missing_from_a_named_provider_keeps_its_provider()`

`fn matrix_failures_fold_when_only_the_script_differs()`

`fn a_reported_step_name_can_be_passed_back_to_from_step()`

`fn undefined_references_warn_without_failing_validation()`

`fn mcp_install_registers_once_and_leaves_other_commands_alone()`

`fn clean_reports_its_json_type_and_exits_1_when_a_workspace_stays()`

`const REFS: &str = r#" name: refs on: push: branches: ['release/**'] tags: ['v*'] pull_request: branches: [main] jobs: check: runs-on: ubuntu-latest steps: - name: refs run: | echo "event=$GITHUB_EVENT_NAME ref=$GITHUB_REF head=$GITHUB_HEAD_REF base=$GITHUB_BASE_REF" test "$GITHUB_EVENT_NAME" = "$`

`fn branch_and_event_reach_filters_and_steps_on_a_detached_head()`

`fn a_branch_or_tag_alone_is_a_push_to_filters_and_steps()`

`fn a_step_whose_action_did_not_run_makes_a_pass_partial()`

`fn a_local_javascript_action_is_reported_as_not_run()`

---

## crates/wrkflw/tests/container_fidelity_test.rs

**Language:** Rust | **Size:** 4.0 KB | **Lines:** 145

**Imports:**
- `std::fs`
- `std::path::Path`
- `std::process::{Command, Output}`
- `tempfile::TempDir`

**Declarations:**

`const NO_GIT_CONFIG: &str = "/dev/null"`

`const ALPINE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"`

`fn docker_answers() -> bool`

`fn repo(files: &[(&str, &str)]) -> TempDir`

`fn run(root: &Path, flags: &[&str]) -> Output`

`fn assert_ok(output: &Output)`

`fn a_github_path_entry_extends_the_containers_own_path()`

`fn a_composite_actions_changes_stay_in_the_job_container()`

---

## crates/wrkflw/tests/continue_on_error_test.rs

**Language:** Rust | **Size:** 5.5 KB | **Lines:** 239

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobResult, JobStatus, RuntimeType,
    StepStatus,
}`

**Declarations:**

`async fn run(yaml: &str) -> ExecutionResult`

`fn job<'a>(result: &'a ExecutionResult, name: &str) -> &'a JobResult`

`fn status(result: &ExecutionResult, name: &str) -> JobStatus`

`fn step_status(job: &JobResult, name: &str) -> StepStatus`

`async fn allowed_job_failure_keeps_the_run_green_and_dependents_running()`

`async fn dependents_read_an_allowed_failure_as_success()`

`async fn explicit_false_still_fails_the_run()`

`async fn unevaluable_expression_warns_and_counts_the_failure()`

`async fn services_warn_that_they_are_not_started()`

`async fn job_expression_applies_per_matrix_combination()`

`async fn allowed_matrix_failure_does_not_trigger_fail_fast()`

`async fn step_expression_is_accepted_and_evaluated()`

---

## crates/wrkflw/tests/dynamic_matrix_test.rs

**Language:** Rust | **Size:** 6.7 KB | **Lines:** 246

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
}`

**Declarations:**

`fn emulation_cfg() -> ExecutionConfig`

`async fn run(yaml: &str) -> Result<ExecutionResult, String>`

`fn job_outcomes(result: &ExecutionResult, job: &str) -> Vec<(String, bool)>`

`fn workflow(matrix: &str, output: &str) -> String`

`async fn include_from_needs_output_merges_into_axes()`

`async fn include_only_matrix_comes_from_needs_output()`

`async fn exclude_from_needs_output_filters_axes()`

`fn job_status(result: &ExecutionResult, job: &str) -> JobStatus`

`fn failure_details(result: &ExecutionResult) -> &str`

`async fn include_that_is_not_a_list_fails_the_job()`

`async fn exclude_that_is_not_a_list_fails_the_job()`

`async fn context_unavailable_in_strategy_fails_the_job()`

`async fn github_context_is_available_to_strategy()`

`async fn empty_include_only_matrix_fails_that_job_and_the_run_carries_on()`

---

## crates/wrkflw/tests/gitlab_pipeline_test.rs

**Language:** Rust | **Size:** 9.9 KB | **Lines:** 412

**Imports:**
- `std::collections::HashMap`
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
}`

**Declarations:**

`async fn run(yaml: &str, target_job: Option<&str>, variables: &[(&str, &str)]) -> ExecutionResult`

`async fn run_in( runtime_type: RuntimeType, yaml: &str, target_job: Option<&str>, variables: &[(&str, &str)], ) -> ExecutionResult`

`fn status(result: &ExecutionResult, name: &str) -> JobStatus`

`const FAILING_PIPELINE: &str = r#" stages: [first, second, third] ok: stage: first script: ["true"] breaks: stage: first script: ["exit 1"] next: stage: second script: ["true"] tolerated: stage: second when: on_failure script: ["exit 1"] allow_failure: true cleanup: stage: third when: always script: ["true"] report: stage: third when: on_failure script: ["true"] "#`

`async fn a_failed_stage_selects_the_later_jobs()`

`async fn an_allowed_failure_does_not_trigger_on_failure_jobs()`

`const GATED_PIPELINE: &str = r#" stages: [deploy] only_main: stage: deploy only: [main] script: ["true"] tag_release: stage: deploy rules: - if: $CI_COMMIT_TAG script: ["true"] manual_deploy: stage: deploy when: manual script: ["true"] "#`

`async fn variables_decide_which_gated_jobs_run()`

`async fn naming_a_job_runs_it_despite_its_gate()`

`const WORKFLOW_PIPELINE: &str = r#" workflow: rules: - if: $CI_PIPELINE_SOURCE == "web" job: script: ["true"] "#`

`async fn workflow_rules_gate_the_whole_pipeline()`

`async fn variables_reach_scripts()`

`const APPROVAL_PIPELINE: &str = r#" stages: [gate, deploy] approve: stage: gate rules: - when: manual script: ["true"] deploy: stage: deploy script: ["true"] cleanup: stage: deploy when: always script: ["true"] "#`

`async fn a_blocking_manual_job_stops_later_stages()`

`async fn an_optional_manual_job_does_not_stop_later_stages()`

`async fn a_rule_can_allow_a_failure()`

`async fn a_tag_pipeline_has_no_branch_for_rules()`

`async fn a_named_job_still_follows_the_stages_before_it()`

`async fn a_job_with_an_image_runs_through_the_shell_selector()`

`async fn a_job_with_an_image_runs_under_secure_emulation()`

`async fn a_manual_job_skipped_by_a_failure_does_not_block()`

`async fn a_rule_that_forbids_failure_stops_later_stages()`

`async fn naming_a_matrix_job_runs_past_its_matrix_condition()`

---

## crates/wrkflw/tests/matrix_fold_test.rs

**Language:** Rust | **Size:** 4.6 KB | **Lines:** 123

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
}`

**Declarations:**

`fn emulation_cfg() -> ExecutionConfig`

`async fn run(yaml: &str) -> ExecutionResult`

`fn legs(result: &ExecutionResult) -> Vec<(String, JobStatus, Option<String>)>`

`fn workflow(strategy: &str, steps: &str) -> String`

`const OS: &str = " matrix:\n os: [ubuntu-latest, macos-latest, windows-latest]"`

`async fn legs_that_only_change_the_runner_run_once_in_matrix_order()`

`async fn a_leg_repeating_a_failed_leg_fails_and_says_whose_failure_it_is()`

`async fn a_leg_whose_original_fail_fast_skipped_is_skipped_too()`

`async fn a_condition_reading_the_runner_over_several_lines_runs_every_leg()`

---

## crates/wrkflw/tests/needs_failure_propagation_test.rs

**Language:** Rust | **Size:** 13.5 KB | **Lines:** 587

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{
    execute_workflow, ExecutionConfig, ExecutionResult, JobStatus, RuntimeType,
}`

**Declarations:**

`fn write_file(path: &std::path::Path, content: &str)`

`fn emulation_cfg() -> ExecutionConfig`

`fn status_of<'a>(result: &'a ExecutionResult, job: &str) -> &'a str`

`async fn dependent_job_is_skipped_when_need_fails()`

`async fn skip_cascades_through_the_needs_chain()`

`async fn explicit_if_condition_still_runs_after_a_failed_need()`

`async fn unrelated_jobs_still_run_after_a_failure()`

`async fn run(yaml: &str) -> ExecutionResult`

`async fn status_functions_see_the_failed_need()`

`async fn needs_result_is_visible_to_job_if()`

`async fn plain_if_is_still_gated_on_needs()`

`async fn need_skipped_by_its_own_if_skips_dependents()`

`async fn any_failed_need_among_several_skips()`

`async fn ancestor_failure_propagates_past_an_always_job()`

`async fn reusable_workflow_jobs_skip_after_failed_need()`

`async fn matrix_need_with_one_failed_combination_counts_as_failed()`

`async fn fail_fast_matrix_need_counts_as_failed()`

`async fn needs_outputs_are_visible_to_job_if()`

`async fn gate_skipped_need_reports_skipped_result()`

---

## crates/wrkflw/tests/reusable_inputs_outputs_test.rs

**Language:** Rust | **Size:** 3.0 KB | **Lines:** 112

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType}`

**Declarations:**

`fn write_file(path: &std::path::Path, content: &str)`

`async fn test_reusable_workflow_inputs_and_declared_outputs()`

---

## crates/wrkflw/tests/startup_test.rs

**Language:** Rust | **Size:** 566 B | **Lines:** 16

**Imports:**
- `std::process::Command`

**Declarations:**

`fn the_binary_starts()`

---

## crates/wrkflw/tests/target_job_test.rs

**Language:** Rust | **Size:** 4.3 KB | **Lines:** 162

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw_lib::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType}`

**Declarations:**

`fn write_file(path: &std::path::Path, content: &str)`

`async fn test_target_job_runs_only_specified_job()`

`async fn test_target_job_not_found_returns_error()`

`async fn test_target_job_with_no_deps_runs_alone()`

---

## examples/secrets-demo/README.md

**Language:** Markdown | **Size:** 2.6 KB | **Lines:** 123

**Declarations:**

---

## examples/secrets-demo/secrets-workflow.yml

**Language:** YAML | **Size:** 6.9 KB | **Lines:** 213

**Declarations:**

---

## examples/ui-demo/01-dag-diamond.yml

**Language:** YAML | **Size:** 766 B | **Lines:** 36

**Declarations:**

---

## examples/ui-demo/02-dag-wide-fan.yml

**Language:** YAML | **Size:** 1.8 KB | **Lines:** 85

**Declarations:**

---

## examples/ui-demo/03-dag-linear.yml

**Language:** YAML | **Size:** 754 B | **Lines:** 40

**Declarations:**

---

## examples/ui-demo/04-trigger-dispatch.yml

**Language:** YAML | **Size:** 1.3 KB | **Lines:** 50

**Declarations:**

---

## examples/ui-demo/05-matrix-inspector.yml

**Language:** YAML | **Size:** 1.7 KB | **Lines:** 61

**Declarations:**

---

## examples/ui-demo/06-secrets-runtime.yml

**Language:** YAML | **Size:** 1.7 KB | **Lines:** 67

**Declarations:**

---

## examples/ui-demo/07-multi-event.yml

**Language:** YAML | **Size:** 1.0 KB | **Lines:** 39

**Declarations:**

---

## examples/ui-demo/08-failing.yml

**Language:** YAML | **Size:** 1.4 KB | **Lines:** 61

**Declarations:**

---

## publish_crates.sh

**Language:** Shell | **Size:** 5.1 KB | **Lines:** 179

**Declarations:**

---

## schemas/github-workflow.json

**Language:** JSON | **Size:** 90.8 KB | **Lines:** 1732

**Declarations:**

---

## schemas/gitlab-ci.json

**Language:** JSON | **Size:** 104.7 KB | **Lines:** 3012

**Declarations:**

---

## scripts/bump-crate.sh

**Language:** Shell | **Size:** 3.1 KB | **Lines:** 97

---

## scripts/conformance/plan_corpus.sh

**Language:** Shell | **Size:** 3.8 KB | **Lines:** 75

---

## skills/wrkflw/SKILL.md

**Language:** Markdown | **Size:** 3.2 KB | **Lines:** 25

**Declarations:**

---

## tests/README.md

**Language:** Markdown | **Size:** 1.5 KB | **Lines:** 42

**Declarations:**

---

## tests/cleanup_test.rs

**Language:** Rust | **Size:** 7.2 KB | **Lines:** 236

**Imports:**
- `bollard::Docker`
- `std::process::Command`
- `std::time::Duration`
- `uuid::Uuid`
- `wrkflw::{
    cleanup_on_exit,
    executor::docker,
    runtime::emulation::{self, EmulationRuntime},
}`

**Declarations:**

`fn should_skip_docker_tests() -> bool`

`fn should_skip_process_tests() -> bool`

`async fn test_docker_container_cleanup()`

`async fn test_docker_network_cleanup()`

`async fn test_emulation_workspace_cleanup()`

`async fn test_emulation_process_cleanup()`

`async fn test_cleanup_on_exit_function()`

---

## tests/fixtures/gitlab-ci/advanced.gitlab-ci.yml

**Language:** YAML | **Size:** 4.0 KB | **Lines:** 197

**Declarations:**

---

## tests/fixtures/gitlab-ci/basic.gitlab-ci.yml

**Language:** YAML | **Size:** 674 B | **Lines:** 45

**Declarations:**

---

## tests/fixtures/gitlab-ci/docker.gitlab-ci.yml

**Language:** YAML | **Size:** 2.4 KB | **Lines:** 97

**Declarations:**

---

## tests/fixtures/gitlab-ci/includes.gitlab-ci.yml

**Language:** YAML | **Size:** 883 B | **Lines:** 40

**Declarations:**

---

## tests/fixtures/gitlab-ci/invalid.gitlab-ci.yml

**Language:** YAML | **Size:** 1.4 KB | **Lines:** 57

**Declarations:**

---

## tests/fixtures/gitlab-ci/minimal.gitlab-ci.yml

**Language:** YAML | **Size:** 124 B | **Lines:** 11

**Declarations:**

---

## tests/fixtures/gitlab-ci/services.gitlab-ci.yml

**Language:** YAML | **Size:** 3.3 KB | **Lines:** 167

**Declarations:**

---

## tests/fixtures/gitlab-ci/workflow.gitlab-ci.yml

**Language:** YAML | **Size:** 4.2 KB | **Lines:** 186

**Declarations:**

---

## tests/matrix_test.rs

**Language:** Rust | **Size:** 3.8 KB | **Lines:** 125

**Imports:**
- `indexmap::IndexMap`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `wrkflw::matrix::{self, MatrixCombination, MatrixConfig, MatrixEntries}`

**Declarations:**

`fn create_test_matrix() -> MatrixConfig`

`fn test_matrix_expansion()`

`fn test_format_combination_name()`

---

## tests/reusable_workflow_execution_test.rs

**Language:** Rust | **Size:** 3.2 KB | **Lines:** 124

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw::executor::engine::{execute_workflow, ExecutionConfig, RuntimeType}`

**Declarations:**

`fn write_file(path: &std::path::Path, content: &str)`

`async fn test_local_reusable_workflow_execution_success()`

`async fn test_local_reusable_workflow_execution_failure_propagates()`

---

## tests/reusable_workflow_test.rs

**Language:** Rust | **Size:** 1.6 KB | **Lines:** 64

**Imports:**
- `std::fs`
- `tempfile::tempdir`
- `wrkflw::evaluator::evaluate_workflow_file`

**Declarations:**

`fn test_reusable_workflow_validation()`

---

## tests/safe_workflow.yml

**Language:** YAML | **Size:** 816 B | **Lines:** 35

**Declarations:**

---

## tests/scripts/test-podman-basic.sh

**Language:** Shell | **Size:** 7.0 KB | **Lines:** 215

**Declarations:**

---

## tests/scripts/test-preserve-containers.sh

**Language:** Shell | **Size:** 8.8 KB | **Lines:** 256

**Declarations:**

---

## tests/security_comparison.yml

**Language:** YAML | **Size:** 674 B | **Lines:** 29

**Declarations:**

---

## tests/security_demo.yml

**Language:** YAML | **Size:** 2.5 KB | **Lines:** 92

**Declarations:**

---

## tests/workflows/1-basic-workflow.yml

**Language:** YAML | **Size:** 389 B | **Lines:** 21

**Declarations:**

---

## tests/workflows/2-reusable-workflow-caller.yml

**Language:** YAML | **Size:** 441 B | **Lines:** 20

**Declarations:**

---

## tests/workflows/3-reusable-workflow-definition.yml

**Language:** YAML | **Size:** 863 B | **Lines:** 32

**Declarations:**

---

## tests/workflows/4-mixed-jobs.yml

**Language:** YAML | **Size:** 563 B | **Lines:** 25

**Declarations:**

---

## tests/workflows/5-no-name-reusable-caller.yml

**Language:** YAML | **Size:** 234 B | **Lines:** 12

**Declarations:**

---

## tests/workflows/6-invalid-reusable-format.yml

**Language:** YAML | **Size:** 269 B | **Lines:** 17

**Declarations:**

---

## tests/workflows/7-invalid-regular-job.yml

**Language:** YAML | **Size:** 379 B | **Lines:** 19

**Declarations:**

---

## tests/workflows/8-cyclic-dependencies.yml

**Language:** YAML | **Size:** 515 B | **Lines:** 31

**Declarations:**

---

## tests/workflows/cpp-test.yml

**Language:** YAML | **Size:** 860 B | **Lines:** 38

**Declarations:**

---

## tests/workflows/example.yml

**Language:** YAML | **Size:** 503 B | **Lines:** 26

**Declarations:**

---

## tests/workflows/matrix-example.yml

**Language:** YAML | **Size:** 1.0 KB | **Lines:** 44

**Declarations:**

---

## tests/workflows/multi-runtime-test.yml

**Language:** YAML | **Size:** 688 B | **Lines:** 27

**Declarations:**

---

## tests/workflows/node-test.yml

**Language:** YAML | **Size:** 639 B | **Lines:** 31

**Declarations:**

---

## tests/workflows/python-test.yml

**Language:** YAML | **Size:** 635 B | **Lines:** 31

**Declarations:**

---

## tests/workflows/runs-on-array-test.yml

**Language:** YAML | **Size:** 391 B | **Lines:** 18

**Declarations:**

---

## tests/workflows/rust-test.yml

**Language:** YAML | **Size:** 797 B | **Lines:** 38

**Declarations:**

---

## tests/workflows/test.yml

**Language:** YAML | **Size:** 159 B | **Lines:** 12

**Declarations:**

---

## tests/workflows/trigger_gitlab.sh

**Language:** Shell | **Size:** 2.0 KB | **Lines:** 79

**Declarations:**

---

## tests/workflows/working-secrets-test.yml

**Language:** YAML | **Size:** 1.5 KB | **Lines:** 46

**Declarations:**

