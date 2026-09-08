# Codebase Index: wrkflw

> Generated: 2026-09-08 17:28:51 UTC | Files: 202 | Lines: 72893
> Languages: JSON (4), Markdown (24), Rust (113), Shell (5), TOML (18), YAML (38)

## Directory Structure

```
wrkflw/
  BREAKING_CHANGES.md
  CLAUDE.md
  Cargo.toml
  INDEX.md
  README.md
  RELEASE_POST.md
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
        action_resolver.rs
        artifacts.rs
        cache.rs
        dependency.rs
        docker.rs
        docker_test.rs
        engine.rs
        environment.rs
        expression.rs
        github_env_files.rs
        lib.rs
        podman.rs
        substitution.rs
        workflow_commands.rs
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
        lib.rs
        schema.rs
        workflow.rs
    runtime/
      Cargo.toml
      README.md
      src/
        container.rs
        emulation.rs
        emulation_test.rs
        lib.rs
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
          timing.rs
        handlers/
          mod.rs
          workflow.rs
        lib.rs
        log_processor.rs
        models/
          mod.rs
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
        lib.rs
        main.rs
        prefilter.rs
        run_workflow_cmd.rs
        watch_cmd.rs
      tests/
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
- `# Launch the TUI (auto-detects .github/workflows)`
- `# Validate workflows`
- `# Run a workflow`
- `# Rerun workflows automatically on file changes`
- `# List detected workflows and pipelines`
- `# Validate all workflows in .github/workflows`
- `# Validate specific files or directories`
- `# Validate multiple paths`
- `# GitLab pipelines`
- `# Verbose output`
- `# Run with auto-detection (default: tries Docker, then Podman, then emulation)`
- `# Run with Docker explicitly`
- `# Run with Podman`
- `# Run in emulation mode (no containers)`
- `# Run in sandboxed secure emulation`
- `# Run a specific job`
- `# List jobs in a workflow`
- `# Preserve failed containers for debugging`
- `# Auto-detect changed files from git (vs origin/HEAD, main/master, or HEAD~1)`
- `# Pin the diff range`
- `# Supply changed files explicitly (e.g. from a CI wrapper)`
- `# Simulate a pull_request — `--base-branch` is required under strict mode`
- `# Opt out of strict rejection (legacy warn-and-proceed)`
- `# Watch .github/workflows for changes and rerun affected workflows`
- `# Watch a specific path, simulate pull_request, and cap concurrency`
- `# Ignore extra directories on top of the built-in list`
- `# Open TUI with default directory`
- `# Open with specific runtime`
- `# GitHub (requires GITHUB_TOKEN env var)`
- `# GitLab (requires GITLAB_TOKEN env var)`
- `# Environment variables (simplest)`
- `# File-based secrets (JSON, YAML, or .env format)`
- `# Configure in ~/.wrkflw/secrets.yml`

**RELEASE_POST.md**
- `# wrkflw v0.8.0 — run GitHub Actions locally, now with a real `${{ ... }}` evaluator`

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

**crates/executor/src/action_resolver.rs**
- `pub enum ActionType`
- `pub struct ResolvedAction`
- `pub async fn resolve_remote_action( repo: &str, version: &str, sub_path: Option<&str>, ) -> Result<ResolvedAction, String>`

**crates/executor/src/artifacts.rs**
- `pub struct ArtifactStore`

**crates/executor/src/cache.rs**
- `pub struct CacheStore`

**crates/executor/src/dependency.rs**
- `pub fn resolve_dependencies(workflow: &WorkflowDefinition) -> Result<Vec<Vec<String>>, String>`
- `pub fn collect_transitive_deps(target_job: &str, jobs: &HashMap<String, Job>) -> HashSet<String>`
- `pub fn filter_plan_to_job( plan: Vec<Vec<String>>, target_job: &str, jobs: &HashMap<String, Job>, kind: &str, ) -> Result<Vec<Vec<String>>, String>`
- `pub fn filter_plan_to_job_by_stage( plan: Vec<Vec<String>>, target_job: &str, jobs: &HashMap<String, Job>, kind: &str, ) -> Result<Vec<Vec<String>>, String>`

**crates/executor/src/docker.rs**
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

**crates/executor/src/engine.rs**
- `pub async fn execute_workflow( workflow_path: &Path, config: ExecutionConfig, ) -> Result<ExecutionResult, ExecutionError>`
- `pub fn detect_runtime(runtime_type: RuntimeType) -> RuntimeType`
- `pub enum RuntimeType`
- `pub struct ExecutionConfig`
- `pub struct ExecutionResult`
- `pub struct JobResult`
- `pub enum JobStatus`
- `pub struct StepResult`
- `pub enum StepStatus`
- `pub enum ExecutionError`

**crates/executor/src/environment.rs**
- `pub fn setup_github_environment_files(workspace_dir: &Path) -> io::Result<()>`
- `pub fn create_github_context( workflow: &WorkflowDefinition, workspace_dir: &Path, ) -> HashMap<String, String>`
- `pub fn add_job_context(env: &mut HashMap<String, String>, job_name: &str)`
- `pub fn add_matrix_context( env: &mut HashMap<String, String>, matrix_combination: &MatrixCombination, )`

**crates/executor/src/expression.rs**
- `pub enum ExprValue`
- `pub struct ExpressionContext<'a>`
- `pub fn evaluate(expr: &str, ctx: &ExpressionContext) -> Result<ExprValue, String>`
- `pub fn evaluate_as_bool(expr: &str, ctx: &ExpressionContext) -> Result<bool, String>`

**crates/executor/src/github_env_files.rs**
- `pub struct StepEnvironmentUpdates`
- `pub fn parse_github_kv_file(content: &str) -> HashMap<String, String>`
- `pub fn parse_github_path_file(content: &str) -> Vec<String>`
- `pub fn read_step_environment_updates(job_env: &HashMap<String, String>) -> StepEnvironmentUpdates`
- `pub fn apply_step_environment_updates( job_env: &mut HashMap<String, String>, job_user_env: &mut HashMap<String, String>, step_outputs_map: &mut HashMap<String, HashMap<String, String>>, step_id: Option<&str>, )`
- `pub fn clear_step_files(job_env: &HashMap<String, String>)`

**crates/executor/src/lib.rs**
- `pub mod action_resolver`
- `pub mod dependency`
- `pub mod docker`
- `pub mod engine`
- `pub mod environment`
- `pub mod expression`
- `pub mod github_env_files`
- `pub mod podman`
- `pub mod substitution`

**crates/executor/src/podman.rs**
- `pub struct PodmanRuntime`
- `pub fn is_available() -> bool`
- `pub fn track_container(id: &str)`
- `pub fn untrack_container(id: &str)`
- `pub async fn cleanup_resources()`
- `pub async fn cleanup_containers() -> Result<(), String>`
- `pub fn get_tracked_containers() -> Vec<String>`

**crates/executor/src/substitution.rs**
- `pub fn preprocess_command(command: &str, matrix_values: &HashMap<String, Value>) -> String`
- `pub fn process_step_run(run: &str, matrix_combination: &Option<HashMap<String, Value>>) -> String`
- `pub fn apply_matrix_to_steps(steps: &[Step], matrix_values: &HashMap<String, Value>) -> Vec<Step>`
- `pub fn preprocess_hash_files(text: &str, workspace: &Path) -> Result<String, String>`
- `pub fn preprocess_expressions( text: &str, workspace: &Path, ctx: &crate::expression::ExpressionContext<'_>, ) -> Result<String, String>`

**crates/executor/src/workflow_commands.rs**
- `pub enum WorkflowCommand`
- `pub fn parse_workflow_commands(output: &str) -> Vec<WorkflowCommand>`

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
- `pub struct ValidationResult`
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
- `pub fn convert_to_workflow_format(pipeline: &Pipeline) -> workflow::WorkflowDefinition`

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

**crates/runtime/Cargo.toml**
- `[package]`
- `[dependencies]`

**crates/runtime/README.md**
- `## wrkflw-runtime`

**crates/runtime/src/container.rs**
- `pub const LOCAL_IMAGE_PREFIX: &str = "wrkflw-"`
- `pub const COMBINED_IMAGE_PREFIX: &str = "wrkflw-combined:"`
- `pub trait ContainerRuntime`
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
- `pub mod sandbox`
- `pub mod secure_emulation`

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

**crates/ui/README.md**
- `## wrkflw-ui`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**crates/ui/src/app/mod.rs**
- `pub async fn run_wrkflw_tui( path: Option<&PathBuf>, runtime_type: RuntimeType, verbose: bool, preserve_containers_on_failure: bool, show_action_messages: bool, ) -> io::Result<()>`

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
- `pub enum NodeState`
- `pub fn topo_levels(def: &WorkflowDefinition) -> Vec<Vec<String>>`
- `pub fn render<F: Fn(&str) -> NodeState>( frame: &mut Frame<'_>, area: Rect, def: Option<&WorkflowDefinition>, state_of: F, spinner_frame: usize, )`

**crates/ui/src/components/mod.rs**
- `pub mod dag`
- `pub mod progress_dots`
- `pub mod timing`

**crates/ui/src/components/progress_bar.rs**
- `pub struct ProgressBar`

**crates/ui/src/components/progress_dots.rs**
- `pub enum DotState`
- `pub fn render(frame: &mut Frame<'_>, area: Rect, dots: &[DotState], done: usize, total: usize)`
- `pub fn synthesise( completed: &[StepStatus], total: usize, workflow_status: &WorkflowStatus, ) -> Vec<DotState>`

**crates/ui/src/components/timing.rs**
- `pub struct TimingRow<'a>`
- `pub fn render(frame: &mut Frame<'_>, area: Rect, rows: &[TimingRow])`

**crates/ui/src/handlers/mod.rs**
- `pub mod workflow`

**crates/ui/src/handlers/workflow.rs**
- `pub fn validate_workflow(path: &Path, verbose: bool) -> io::Result<()>`
- `pub async fn execute_workflow_cli( path: &Path, runtime_type: RuntimeType, verbose: bool, show_action_messages: bool, ) -> io::Result<()>`
- `pub async fn execute_curl_trigger( workflow_name: &str, branch: Option<&str>, ) -> Result<(Vec<wrkflw_executor::JobResult>, ()), String>`
- `pub fn start_next_workflow_execution( app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, verbose: bool, )`

**crates/ui/src/lib.rs**
- `pub mod cli_style`
- `pub mod handlers`
- `pub mod app`
- `pub mod components`
- `pub mod log_processor`
- `pub mod models`
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
- `pub struct QueuedExecution`
- `pub enum WorkflowStatus`
- `pub struct WorkflowExecution`
- `pub struct JobExecution`
- `pub struct StepExecution`
- `pub enum StatusSeverity`
- `pub enum LogBadge`
- `pub enum LogFilterLevel`

**crates/ui/src/theme.rs**
- `pub fn set_accent_override(color: Option<Color>)`
- `pub fn current_accent() -> Color`
- `pub struct Colors`
- `pub const COLORS: Colors = Colors`
- `pub fn title_style() -> Style`
- `pub fn selected_style() -> Style`
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
- `pub fn job_status(status: &JobStatus) -> (&'static str, Style)`
- `pub fn step_status(status: &StepStatus) -> (&'static str, Style)`
- `pub fn block<'a>(title: &'a str) -> Block<'a>`
- `pub fn block_focused<'a>(title: &'a str) -> Block<'a>`
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
- `pub fn render_dag_tab(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/execution_tab.rs**
- `pub fn render_execution_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/help_overlay.rs**
- `pub fn render_help_content(f: &mut Frame<'_>, area: Rect, scroll_offset: usize)`
- `pub fn render_help_overlay(f: &mut Frame<'_>, scroll_offset: usize)`

**crates/ui/src/views/job_detail.rs**
- `pub fn render_job_detail_view(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/logs_tab.rs**
- `pub fn render_logs_tab(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/mod.rs**
- `pub fn render_ui(f: &mut Frame<'_>, app: &mut App)`

**crates/ui/src/views/secrets_tab.rs**
- `pub fn render_secrets_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/status_bar.rs**
- `pub fn render_status_bar(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/title_bar.rs**
- `pub const TAB_LABELS: [&str`
- `pub const TAB_COUNT: usize = TAB_LABELS.len()`
- `pub const TAB_WORKFLOWS: usize = 0`
- `pub const TAB_EXECUTION: usize = 1`
- `pub const TAB_DAG: usize = 2`
- `pub const TAB_LOGS: usize = 3`
- `pub const TAB_TRIGGER: usize = 4`
- `pub const TAB_SECRETS: usize = 5`
- `pub const TAB_HELP: usize = 6`
- `pub fn render_title_bar(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/trigger_tab.rs**
- `pub fn render_trigger_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/ui/src/views/tweaks_overlay.rs**
- `pub fn render_tweaks_overlay(f: &mut Frame<'_>, app: &App, area: Rect)`

**crates/ui/src/views/workflows_tab.rs**
- `pub fn render_workflows_tab(f: &mut Frame<'_>, app: &mut App, area: Rect)`

**crates/utils/Cargo.toml**
- `[package]`
- `[dependencies]`
- `[target.'cfg(unix)'.dependencies]`

**crates/utils/README.md**
- `## wrkflw-utils`

**crates/utils/src/lib.rs**
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

**crates/wrkflw/README.md**
- `## WRKFLW (CLI and Library)`
- `# Launch the TUI (auto-loads .github/workflows)`
- `# Validate all workflows in the default directory`
- `# Validate a specific file or directory`
- `# Validate multiple files and/or directories`
- `# Run a workflow (Docker by default)`
- `# Use Podman, emulation, or sandboxed secure emulation instead of Docker`
- `# Diff-aware filtering (skip workflows whose on: block doesn't match)`
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
- `# ~/.wrkflw/secrets.yml`
- `# ~/.wrkflw/secrets.yml`
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

**tests/README.md**
- `# Testing`
- `# All tests`
- `# Unit tests only`
- `# Integration tests only`
- `# End-to-end tests only`
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

## BREAKING_CHANGES.md

**Language:** Markdown | **Size:** 6.6 KB | **Lines:** 166

**Declarations:**

---

## CLAUDE.md

**Language:** Markdown | **Size:** 4.4 KB | **Lines:** 66

**Declarations:**

---

## Cargo.toml

**Language:** TOML | **Size:** 2.4 KB | **Lines:** 78

**Declarations:**

---

## INDEX.md

**Language:** Markdown | **Size:** 145.0 KB | **Lines:** 5741

**Declarations:**

---

## README.md

**Language:** Markdown | **Size:** 10.6 KB | **Lines:** 288

**Declarations:**

---

## RELEASE_POST.md

**Language:** Markdown | **Size:** 3.8 KB | **Lines:** 59

**Declarations:**

---

## cliff.toml

**Language:** TOML | **Size:** 3.6 KB | **Lines:** 106

**Declarations:**

---

## crates/README.md

**Language:** Markdown | **Size:** 1.7 KB | **Lines:** 46

**Declarations:**

---

## crates/evaluator/Cargo.toml

**Language:** TOML | **Size:** 499 B | **Lines:** 20

**Declarations:**

---

## crates/evaluator/README.md

**Language:** Markdown | **Size:** 835 B | **Lines:** 29

**Declarations:**

---

## crates/evaluator/src/lib.rs

**Language:** Rust | **Size:** 2.5 KB | **Lines:** 80

**Imports:**
- `colored::*`
- `serde_yaml::{self, Value}`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `wrkflw_models::ValidationResult`
- `wrkflw_validators::{validate_env, validate_jobs, validate_triggers}`

**Declarations:**

`fn find_repo_root(workflow_path: &Path) -> Option<PathBuf>`

---

## crates/executor/Cargo.toml

**Language:** TOML | **Size:** 1.2 KB | **Lines:** 49

**Imports:**
- `ignore`

**Declarations:**

---

## crates/executor/README.md

**Language:** Markdown | **Size:** 1.7 KB | **Lines:** 37

**Declarations:**

---

## crates/executor/src/action_resolver.rs

**Language:** Rust | **Size:** 23.4 KB | **Lines:** 736

**Imports:**
- `once_cell::sync::Lazy`
- `std::collections::{HashMap, VecDeque}`
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

`static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(||`

`static NO_REDIRECT_CLIENT: Lazy<reqwest::Client> = Lazy::new(||`

`const GITHUB_RAW_BASE_URL: &str = "https://raw.githubusercontent.com"`

`async fn fetch_and_parse( base_url: &str, repo: &str, version: &str, sub_path: Option<&str>, filename: &str, token: Option<&str>, ) -> Result<ResolvedAction, String>`

`fn parse_action_definition(content: &str) -> Result<ResolvedAction, String>`

`fn parse_using(using: &str, runs: &serde_yaml::Value) -> Result<ActionType, String>`

`mod tests`

---

## crates/executor/src/artifacts.rs

**Language:** Rust | **Size:** 10.3 KB | **Lines:** 295

**Imports:**
- `std::collections::HashMap`
- `std::path::{Path, PathBuf}`
- `std::sync::Arc`
- `tokio::sync::RwLock`

**Declarations:**

`fn sanitize_artifact_name(name: &str) -> Result<String, String>`

`fn walk_files(dir: &Path) -> Result<Vec<PathBuf>, String>`

`struct ArtifactMetadata`
> Fields: `path: PathBuf`

**`impl ArtifactStore`**
  `pub fn new(run_dir: &Path) -> std::io::Result<Self>`

  `pub async fn upload( &self, name: &str, path_pattern: &str, workspace: &Path, ) -> Result<usize, String>`

  `pub async fn download(&self, name: &str, target_dir: &Path) -> Result<usize, String>`

  `pub async fn list(&self) -> Vec<String>`


`mod tests`

---

## crates/executor/src/cache.rs

**Language:** Rust | **Size:** 24.6 KB | **Lines:** 668

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

## crates/executor/src/dependency.rs

**Language:** Rust | **Size:** 17.5 KB | **Lines:** 509

**Imports:**
- `std::collections::{HashMap, HashSet, VecDeque}`
- `wrkflw_parser::workflow::{Job, WorkflowDefinition}`

**Declarations:**

`fn job_not_found_error(target_job: &str, jobs: &HashMap<String, Job>, kind: &str) -> String`

`mod tests`

---

## crates/executor/src/docker.rs

**Language:** Rust | **Size:** 49.0 KB | **Lines:** 1303

**Imports:**
- `async_trait::async_trait`
- `bollard::{
    container::{Config, CreateContainerOptions},
    models::HostConfig,
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
    ContainerError, ContainerOutput, ContainerRuntime, COMBINED_IMAGE_PREFIX, LOCAL_IMAGE_PREFIX,
}`
- `wrkflw_utils`
- *... and 1 more imports*

**Declarations:**

`static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static CREATED_NETWORKS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()))`

`static CUSTOMIZED_IMAGES: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()))`

**`impl DockerRuntime`**
  `pub fn new() -> Result<Self, ContainerError>`

  `pub fn new_with_config(preserve_containers_on_failure: bool) -> Result<Self, ContainerError>`

  `pub fn get_customized_image(base_image: &str, customization: &str) -> Option<String>`

  `pub fn set_customized_image(base_image: &str, customization: &str, new_image: &str)`

  `pub fn find_customized_image_key(image: &str, prefix: &str) -> Option<String>`

  `pub fn get_language_specific_image( base_image: &str, language: &str, version: Option<&str>, ) -> Option<String>`

  `pub fn set_language_specific_image( base_image: &str, language: &str, version: Option<&str>, new_image: &str, )`

  `pub async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`


**`impl ContainerRuntime for DockerRuntime`**
  `async fn run_container( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`

  `async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>`


**`impl DockerRuntime`**
  `async fn run_container_inner( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image_inner( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`


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

## crates/executor/src/engine.rs

**Language:** Rust | **Size:** 341.9 KB | **Lines:** 9436

**Imports:**
- `bollard::Docker`
- `futures::future`
- `once_cell::sync::Lazy`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::sync::{Arc, Mutex}`
- `thiserror::Error`
- `ignore::{gitignore::GitignoreBuilder, Match}`
- *... and 13 more imports*

**Declarations:**

`fn is_gitlab_pipeline(path: &Path) -> bool`

`async fn execute_github_workflow( workflow_path: &Path, mut config: ExecutionConfig, ) -> Result<ExecutionResult, ExecutionError>`

`async fn execute_gitlab_pipeline( pipeline_path: &Path, mut config: ExecutionConfig, ) -> Result<ExecutionResult, ExecutionError>`

`fn create_gitlab_context(pipeline: &Pipeline, workspace_dir: &Path) -> HashMap<String, String>`

`fn resolve_gitlab_dependencies( pipeline: &Pipeline, workflow: &WorkflowDefinition, ) -> Result<Vec<Vec<String>>, ExecutionError>`

`fn initialize_runtime( runtime_type: RuntimeType, preserve_containers_on_failure: bool, ) -> Result<Box<dyn ContainerRuntime>, ExecutionError>`

**`impl std::fmt::Display for JobStatus`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


**`impl StepResult`**
  `fn new(name: String, status: StepStatus, output: String) -> Self`


**`impl std::fmt::Display for StepStatus`**
  `fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result`


**`impl From<String> for ExecutionError`**
  `fn from(err: String) -> Self`


`enum PreparedAction`
> Variants: `NativeDocker`, `Image`, `Composite`

`async fn prepare_action( action: &ActionInfo, runtime: &dyn ContainerRuntime, ) -> Result<PreparedAction, ExecutionError>`

`async fn execute_native_docker_step( ctx: &StepExecutionContext<'_>, step_env: &mut HashMap<String, String>, step_name: String, uses: &str, image: String, entrypoint: Option<String>, args: Vec<String>, ) -> Result<StepResult, ExecutionError>`

`fn sanitize_sub_path(raw: &str) -> Result<(), String>`

`fn sanitize_dockerfile_rel(raw: &str) -> Result<String, String>`

`fn extract_docker_runs_config( definition: Option<&serde_yaml::Value>, ) -> Result<(Option<String>, Vec<String>), String>`

`async fn shallow_clone( repo_url: &str, git_ref: &str, target_dir: &Path, ) -> Result<(), ExecutionError>`

`fn is_git_sha(git_ref: &str) -> bool`

`fn determine_action_image(repository: &str) -> String`

`struct SetupRuntime`
> Fields: `language: String`, `version: String`, `install_script: String`

`struct SetupActionDef`
> Fields: `repos: &'static [&'static str]`, `with_key: &'static str`, `default_version: &'static str`, `language: &'static str`, `version_from_ref: bool`

`const SETUP_ACTIONS: &[SetupActionDef] = &[ SetupActionDef`

`fn is_safe_version(version: &str) -> bool`

`fn detect_setup_runtimes(steps: &[Step]) -> Vec<SetupRuntime>`

`fn get_install_script(language: &str, version: &str) -> String`

`fn generate_combined_dockerfile(runtimes: &[SetupRuntime], base_image: &str) -> String`

`fn fnv1a_hash(data: &[u8]) -> u64`

`fn combined_image_tag(runtimes: &[SetupRuntime], dockerfile: &str) -> String`

`static IMAGE_BUILD_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = Lazy::new(|| Mutex::new(HashMap::new()))`

`async fn build_combined_runtime_image( runtimes: &[SetupRuntime], base_image: &str, runtime: &dyn ContainerRuntime, ) -> Result<String, ExecutionError>`

`async fn resolve_runner_image( job: &Job, steps: &[Step], runtime: &dyn ContainerRuntime, ) -> Result<String, ExecutionError>`

`async fn execute_job_batch( jobs: &[String], workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, env_context: &HashMap<String, String>, user_env: &HashMap<String, String>, verbose: bool, secret_manager: Option<&SecretManager>, secret_masker: Option<&SecretMasker>, all_job_outputs: &HashMap<String, HashMap<String, String>>, all_job_results: &HashMap<String, String>, artifact_store: &crate::artifacts::ArtifactStore, cache_store: &crate::cache::CacheStore, ) -> Result<Vec<JobResult>, ExecutionError>`

`struct JobExecutionContext<'a>`
> Fields: `job_name: &'a str`, `workflow: &'a WorkflowDefinition`, `runtime: &'a dyn ContainerRuntime`, `env_context: &'a HashMap<String, String>`, `user_env: &'a HashMap<String, String>`, `verbose: bool`, `services: JobServices<'a>`

`async fn execute_job_with_matrix( job_name: &str, workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, env_context: &HashMap<String, String>, user_env: &HashMap<String, String>, verbose: bool, secret_manager: Option<&SecretManager>, secret_masker: Option<&SecretMasker>, all_job_outputs: &HashMap<String, HashMap<String, String>>, all_job_results: &HashMap<String, String>, artifact_store: &crate::artifacts::ArtifactStore, cache_store: &crate::cache::CacheStore, ) -> Result<Vec<JobResult>, ExecutionError>`

`async fn execute_job(ctx: JobExecutionContext<'_>) -> Result<JobResult, ExecutionError>`

`struct MatrixExecutionContext<'a>`
> Fields: `job_name: &'a str`, `job_template: &'a Job`, `combinations: &'a [MatrixCombination]`, `max_parallel: usize`, `fail_fast: bool`, `workflow: &'a WorkflowDefinition`, `runtime: &'a dyn ContainerRuntime`, `env_context: &'a HashMap<String, String>`, `user_env: &'a HashMap<String, String>`, `verbose: bool`, `services: JobServices<'a>`

`async fn execute_matrix_combinations( ctx: MatrixExecutionContext<'_>, ) -> Result<Vec<JobResult>, ExecutionError>`

`async fn execute_matrix_job( job_name: &str, job_template: &Job, combination: &MatrixCombination, workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, base_env_context: &HashMap<String, String>, base_user_env: &HashMap<String, String>, verbose: bool, services: &JobServices<'_>, ) -> Result<JobResult, ExecutionError>`

`enum StepOutcome`
> Variants: `Completed`, `Skipped`

`struct PendingCacheSave`
> Fields: `key: String`, `path: String`, `workspace: std::path::PathBuf`

`pub(crate) struct JobServices<'a>`
> Fields: `secret_manager: Option<&'a SecretManager>`, `secret_masker: Option<&'a SecretMasker>`, `secrets_context: &'a HashMap<String, String>`, `needs_context: &'a HashMap<String, HashMap<String, String>>`, `needs_results: &'a HashMap<String, String>`, `artifact_store: &'a crate::artifacts::ArtifactStore`, `cache_store: &'a crate::cache::CacheStore`

`async fn flush_pending_cache_saves( pending: &std::sync::Mutex<Vec<PendingCacheSave>>, cache_store: &crate::cache::CacheStore, )`

`struct StepLoopState`
> Fields: `step_results: Vec<StepResult>`, `job_logs: String`, `step_outputs_map: HashMap<String, HashMap<String, String>>`, `step_status_map: HashMap<String, (String, String)>`, `job_status_str: String`

**`impl StepLoopState`**
  `fn new() -> Self`

  `fn process_outcome( &mut self, outcome: StepOutcome, step: &workflow::Step, verbose: bool, job_env: &mut HashMap<String, String>, job_user_env: &mut HashMap<String, String>, secret_masker: Option<&SecretMasker>, ) -> bool`


`fn record_step_status( step_id: Option<&str>, result: &StepResult, step_status_map: &mut HashMap<String, (String, String)>, job_status_str: &mut String, )`

`fn process_workflow_commands( output: &str, step_id: Option<&str>, step_outputs_map: &mut HashMap<String, HashMap<String, String>>, secret_masker: Option<&SecretMasker>, )`

`fn format_annotation_location(file: Option<&str>, line: Option<u32>, col: Option<u32>) -> String`

`async fn run_step_with_guards( step: &Step, step_idx: usize, job_env: &HashMap<String, String>, workflow: &WorkflowDefinition, step_exec_ctx: StepExecutionContext<'_>, ) -> Result<StepOutcome, ExecutionError>`

`fn sanitize_timeout_minutes(raw: Option<f64>, default: f64) -> f64`

`struct StepExecutionContext<'a>`
> Fields: `step: &'a workflow::Step`, `step_idx: usize`, `job_env: &'a HashMap<String, String>`, `job_user_env: &'a HashMap<String, String>`, `working_dir: &'a Path`, `runtime: &'a dyn ContainerRuntime`, `workflow: &'a WorkflowDefinition`, `runner_image: &'a str`, `verbose: bool`, `matrix_combination: &'a Option<HashMap<String, Value>>`, `container_config: Option<&'a JobContainer>`, `workflow_defaults: Option<&'a workflow::Defaults>`, `job_defaults: Option<&'a workflow::Defaults>`, `step_outputs: &'a HashMap<String, HashMap<String, String>>`, `step_statuses: &'a HashMap<String, (String, String)>`, `job_status: &'a str`, `services: JobServices<'a>`, `pending_cache_saves: &'a std::sync::Mutex<Vec<PendingCacheSave>>`

**`impl<'a> StepExecutionContext<'a>`**
  `fn expr_context(&self) -> crate::expression::ExpressionContext<'_>`

  `fn expr_context_with_env<'e>( &self, env: &'e HashMap<String, String>, user_env: &'e HashMap<String, String>, ) -> crate::expression::ExpressionContext<'e> where 'a: 'e,`


`fn preprocess_with_value(value: &str, ctx: &StepExecutionContext<'_>) -> String`

`async fn handle_upload_artifact( step_name: &str, ctx: &StepExecutionContext<'_>, ) -> Result<StepResult, ExecutionError>`

`async fn handle_download_artifact( step_name: &str, ctx: &StepExecutionContext<'_>, ) -> Result<StepResult, ExecutionError>`

`async fn handle_cache_action( step_name: &str, ctx: &StepExecutionContext<'_>, ) -> Result<StepResult, ExecutionError>`

`async fn execute_step(ctx: StepExecutionContext<'_>) -> Result<StepResult, ExecutionError>`

`fn create_gitignore_matcher( dir: &Path, ) -> Result<Option<ignore::gitignore::Gitignore>, ExecutionError>`

`fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError>`

`fn copy_directory_contents_with_gitignore( from: &Path, to: &Path, gitignore: Option<&ignore::gitignore::Gitignore>, ) -> Result<(), ExecutionError>`

`fn get_runner_image(runs_on: &str) -> String`

`fn get_runner_image_from_opt(runs_on: &Option<Vec<String>>) -> String`

`fn get_effective_runner_image(job: &Job) -> String`

`struct StepContainerContext`
> Fields: `owned_volume_paths: Vec<VolumePathPair>`, `github_mount: Option<VolumePathPair>`

**`impl StepContainerContext`**
  `fn build_volumes<'a>( &'a self, working_dir: &'a Path, container_workspace: &'a Path, ) -> Vec<(&'a Path, &'a Path)>`


`fn prepare_step_container_context( step_env: &mut HashMap<String, String>, job_env: &HashMap<String, String>, container_config: Option<&JobContainer>, ) -> StepContainerContext`

`type VolumePathPair = (PathBuf, PathBuf)`

`fn prepare_container_mounts( step_env: &mut HashMap<String, String>, job_env: &HashMap<String, String>, container_config: Option<&JobContainer>, ) -> (Vec<VolumePathPair>, Option<VolumePathPair>)`

`fn warn_unsupported_container_fields(container: &JobContainer)`

`async fn execute_reusable_workflow_job( ctx: &JobExecutionContext<'_>, uses: &str, with: Option<&HashMap<String, String>>, secrets: Option<&serde_yaml::Value>, ) -> Result<JobResult, ExecutionError>`

`async fn run_called_workflow( ctx: &JobExecutionContext<'_>, called: &WorkflowDefinition, uses: &str, with: Option<&HashMap<String, String>>, secrets: Option<&serde_yaml::Value>, workflow_path: &Path, ) -> Result<JobResult, ExecutionError>`

`fn aggregate_reusable_workflow_outputs( job_outputs: &HashMap<String, HashMap<String, String>>, ) -> HashMap<String, String>`

`async fn prepare_runner_image( image: &str, runtime: &dyn ContainerRuntime, verbose: bool, ) -> Result<(), ExecutionError>`

`fn extract_language_info(image: &str) -> Option<(&'static str, Option<&str>)>`

`async fn execute_composite_action( step: &workflow::Step, action_path: &Path, job_env: &HashMap<String, String>, job_user_env: &HashMap<String, String>, working_dir: &Path, runtime: &dyn ContainerRuntime, runner_image: &str, verbose: bool, services: &JobServices<'_>, pending_cache_saves: &std::sync::Mutex<Vec<PendingCacheSave>>, ) -> Result<StepResult, ExecutionError>`

`fn propagate_composite_outputs( action_def: &serde_yaml::Value, composite_step_outputs: &HashMap<String, HashMap<String, String>>, action_env: &HashMap<String, String>, action_user_env: &HashMap<String, String>, caller_job_env: &HashMap<String, String>, working_dir: &Path, job_status: &str, )`

`fn generate_heredoc_delimiter(value: &str) -> String`

`fn convert_yaml_to_step(step_yaml: &serde_yaml::Value) -> Result<workflow::Step, String>`

`fn evaluate_job_condition( condition: &str, env_context: &HashMap<String, String>, user_env: &HashMap<String, String>, _workflow: &WorkflowDefinition, ) -> bool`

`fn evaluate_condition_with_context( condition: &str, ctx: &crate::expression::ExpressionContext<'_>, ) -> bool`

`fn build_needs_context( job: &Job, all_outputs: &HashMap<String, HashMap<String, String>>, all_results: &HashMap<String, String>, ) -> ( HashMap<String, HashMap<String, String>>, HashMap<String, String>, )`

`fn resolve_job_outputs( job: &Job, step_outputs_map: &HashMap<String, HashMap<String, String>>, step_status_map: &HashMap<String, (String, String)>, env_context: &HashMap<String, String>, user_env: &HashMap<String, String>, job_status: &str, working_dir: &Path, ) -> HashMap<String, String>`

`async fn resolve_secrets_for_context( secret_manager: &SecretManager, job: &Job, ) -> HashMap<String, String>`

`mod tests`

---

## crates/executor/src/environment.rs

**Language:** Rust | **Size:** 12.3 KB | **Lines:** 432

**Imports:**
- `chrono::Utc`
- `serde_json`
- `serde_yaml::Value`
- `std::{collections::HashMap, fs, io, path::Path}`
- `wrkflw_matrix::MatrixCombination`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`fn value_to_string(value: &Value) -> String`

`fn get_repo_name() -> String`

`fn extract_repo_from_url(url: &str) -> Option<String>`

`fn get_event_name(workflow: &WorkflowDefinition) -> String`

`fn get_workspace_path() -> String`

`fn get_current_sha() -> String`

`fn get_current_ref() -> String`

`fn get_runner_os() -> String`

`fn get_runner_arch() -> String`

`fn get_temp_dir() -> String`

`fn get_tool_cache_dir() -> String`

`fn get_ref_name(full_ref: &str) -> String`

`fn get_ref_type(full_ref: &str) -> String`

`fn get_repository_owner(repo: &str) -> String`

`fn get_actor() -> String`

`mod tests`

---

## crates/executor/src/expression.rs

**Language:** Rust | **Size:** 109.3 KB | **Lines:** 2711

**Imports:**
- `serde_yaml::Value`
- `std::collections::{HashMap, HashSet}`
- `serde_json`

**Declarations:**

**`impl ExprValue`**
  `pub fn is_truthy(&self) -> bool`

  `pub fn to_output_string(&self) -> String`


`enum Token`
> Variants: `Ident`, `StringLit`, `NumberLit`, `True`, `False`, `Null`, `Dot`, `LParen`, `RParen`, `Comma`, `Eq`, `Ne`, `Lt`, `Le`, `Gt`, `Ge`, `And`, `Or`, `Not`, `Eof`

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
  `fn resolve(&self, parts: &[String]) -> ExprValue`


`fn yaml_value_to_expr(v: &Value) -> ExprValue`

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


`fn expr_eq(a: &ExprValue, b: &ExprValue) -> bool`

`fn expr_cmp(a: &ExprValue, b: &ExprValue) -> Option<std::cmp::Ordering>`

`fn expr_to_json(v: &ExprValue) -> serde_json::Value`

`fn call_builtin( name: &str, args: &[ExprValue], ctx: &ExpressionContext, ) -> Result<ExprValue, String>`

`mod tests`

---

## crates/executor/src/github_env_files.rs

**Language:** Rust | **Size:** 17.9 KB | **Lines:** 519

**Imports:**
- `std::collections::HashMap`
- `std::fs`
- `std::path::Path`

**Declarations:**

`fn is_valid_identifier(s: &str) -> bool`

`mod tests`

---

## crates/executor/src/lib.rs

**Language:** Rust | **Size:** 533 B | **Lines:** 23

**Imports:**
- `pub use docker::cleanup_resources`
- `pub use engine::{
    detect_runtime, execute_workflow, ExecutionConfig, JobResult, JobStatus, RuntimeType,
    StepResult, StepStatus,
}`

**Declarations:**

`pub(crate) mod artifacts`

`pub(crate) mod cache`

`pub(crate) mod workflow_commands`

---

## crates/executor/src/podman.rs

**Language:** Rust | **Size:** 34.4 KB | **Lines:** 926

**Imports:**
- `async_trait::async_trait`
- `once_cell::sync::Lazy`
- `std::collections::HashMap`
- `std::path::Path`
- `std::process::Stdio`
- `std::sync::Mutex`
- `tempfile`
- `tokio::process::Command`
- `wrkflw_logging`
- `wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, LOCAL_IMAGE_PREFIX,
}`
- *... and 2 more imports*

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


**`impl ContainerRuntime for PodmanRuntime`**
  `async fn run_container( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`

  `async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>`


**`impl PodmanRuntime`**
  `async fn run_container_inner( &self, image: &str, cmd: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image_inner( &self, dockerfile: &Path, tag: &str, context_dir: &Path, ) -> Result<(), ContainerError>`


---

## crates/executor/src/substitution.rs

**Language:** Rust | **Size:** 30.4 KB | **Lines:** 829

**Imports:**
- `lazy_static::lazy_static`
- `regex::Regex`
- `serde_yaml::Value`
- `sha2::{Digest, Sha256}`
- `std::collections::HashMap`
- `std::path::Path`
- `wrkflw_parser::workflow::Step`

**Declarations:**

`fn compute_hash_files(args_raw: &str, workspace: &Path) -> Result<String, String>`

`mod tests`

---

## crates/executor/src/workflow_commands.rs

**Language:** Rust | **Size:** 16.1 KB | **Lines:** 515

**Declarations:**

`fn decode_value(s: &str) -> String`

`fn parse_command_line(line: &str) -> Option<WorkflowCommand>`

`fn parse_params(s: &str) -> std::collections::HashMap<String, String>`

`mod tests`

---

## crates/github/Cargo.toml

**Language:** TOML | **Size:** 636 B | **Lines:** 25

**Declarations:**

---

## crates/github/README.md

**Language:** Markdown | **Size:** 653 B | **Lines:** 23

**Declarations:**

---

## crates/github/src/lib.rs

**Language:** Rust | **Size:** 15.6 KB | **Lines:** 452

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

`async fn list_recent_workflow_runs( repo_info: &RepoInfo, workflow_segment: &str, token: &str, ) -> Result<Vec<serde_json::Value>, GithubError>`

`mod tests`

---

## crates/gitlab/Cargo.toml

**Language:** TOML | **Size:** 650 B | **Lines:** 26

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

**Language:** TOML | **Size:** 508 B | **Lines:** 21

**Declarations:**

---

## crates/logging/README.md

**Language:** Markdown | **Size:** 456 B | **Lines:** 22

**Declarations:**

---

## crates/logging/src/lib.rs

**Language:** Rust | **Size:** 3.4 KB | **Lines:** 127

**Imports:**
- `chrono::Local`
- `once_cell::sync::Lazy`
- `std::sync::{Arc, Mutex}`

**Declarations:**

`static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())))`

`static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)))`

`static QUIET_MODE: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)))`

**`impl LogLevel`**
  `fn prefix(&self) -> &'static str`


---

## crates/logging/src/symbols.rs

**Language:** Rust | **Size:** 2.0 KB | **Lines:** 42

**Declarations:**

---

## crates/matrix/Cargo.toml

**Language:** TOML | **Size:** 514 B | **Lines:** 21

**Declarations:**

---

## crates/matrix/README.md

**Language:** Markdown | **Size:** 532 B | **Lines:** 20

**Declarations:**

---

## crates/matrix/src/lib.rs

**Language:** Rust | **Size:** 13.6 KB | **Lines:** 422

**Imports:**
- `indexmap::IndexMap`
- `serde::{Deserialize, Serialize}`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `thiserror::Error`

**Declarations:**

**`impl Default for MatrixConfig`**
  `fn default() -> Self`


**`impl MatrixCombination`**
  `pub fn new(values: HashMap<String, Value>) -> Self`

  `pub fn from_include(values: HashMap<String, Value>) -> Self`


`fn generate_base_combinations( matrix: &MatrixConfig, ) -> Result<Vec<MatrixCombination>, MatrixError>`

`fn generate_combinations( param_names: &[String], param_values: &[Vec<Value>], current_depth: usize, current_combination: &mut HashMap<String, Value>, ) -> Result<Vec<MatrixCombination>, MatrixError>`

`fn apply_exclude_filters( combinations: Vec<MatrixCombination>, exclude_patterns: &[HashMap<String, Value>], ) -> Vec<MatrixCombination>`

`fn is_excluded( combination: &MatrixCombination, exclude_patterns: &[HashMap<String, Value>], ) -> bool`

`fn value_to_string(value: &Value) -> String`

`mod tests`

---

## crates/models/Cargo.toml

**Language:** TOML | **Size:** 442 B | **Lines:** 17

**Declarations:**

---

## crates/models/README.md

**Language:** Markdown | **Size:** 320 B | **Lines:** 16

**Declarations:**

---

## crates/models/src/lib.rs

**Language:** Rust | **Size:** 14.8 KB | **Lines:** 444

**Declarations:**

**`impl Default for ValidationResult`**
  `fn default() -> Self`


**`impl ValidationResult`**
  `pub fn new() -> Self`

  `pub fn add_issue(&mut self, issue: String)`


---

## crates/parser/Cargo.toml

**Language:** TOML | **Size:** 607 B | **Lines:** 26

**Imports:**
- `tempfile`

**Declarations:**

---

## crates/parser/README.md

**Language:** Markdown | **Size:** 333 B | **Lines:** 13

**Declarations:**

---

## crates/parser/src/github-workflow.json

**Language:** JSON | **Size:** 90.1 KB | **Lines:** 1719

**Declarations:**

---

## crates/parser/src/gitlab-ci.json

**Language:** JSON | **Size:** 104.7 KB | **Lines:** 3012

**Declarations:**

---

## crates/parser/src/gitlab.rs

**Language:** Rust | **Size:** 8.4 KB | **Lines:** 268

**Imports:**
- `crate::schema::{SchemaType, SchemaValidator}`
- `crate::workflow`
- `std::collections::HashMap`
- `std::fs`
- `std::path::Path`
- `thiserror::Error`
- `wrkflw_models::gitlab::Pipeline`
- `wrkflw_models::ValidationResult`

**Declarations:**

`mod tests`

---

## crates/parser/src/lib.rs

**Language:** Rust | **Size:** 67 B | **Lines:** 5

**Declarations:**

---

## crates/parser/src/schema.rs

**Language:** Rust | **Size:** 3.8 KB | **Lines:** 111

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


---

## crates/parser/src/workflow.rs

**Language:** Rust | **Size:** 27.5 KB | **Lines:** 911

**Imports:**
- `serde::{Deserialize, Deserializer, Serialize}`
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


**`impl Job`**
  `pub fn matrix_config(&self) -> Option<&MatrixConfig>`

  `pub fn fail_fast(&self) -> bool`

  `pub fn max_parallel(&self) -> Option<usize>`


**`impl Step`**
  `pub fn with_run(name: impl Into<String>, run: impl Into<String>) -> Self`


**`impl WorkflowDefinition`**
  `pub fn resolve_action(&self, action_ref: &str) -> ActionInfo`


`fn normalize_triggers(on_value: &serde_yaml::Value) -> Result<Vec<String>, String>`

`mod tests`

---

## crates/runtime/Cargo.toml

**Language:** TOML | **Size:** 735 B | **Lines:** 30

**Imports:**
- `ignore`

**Declarations:**

---

## crates/runtime/README.md

**Language:** Markdown | **Size:** 554 B | **Lines:** 17

**Declarations:**

---

## crates/runtime/src/container.rs

**Language:** Rust | **Size:** 12.4 KB | **Lines:** 326

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

**Language:** Rust | **Size:** 30.1 KB | **Lines:** 828

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

**Language:** Rust | **Size:** 99 B | **Lines:** 6

**Declarations:**

---

## crates/runtime/src/sandbox.rs

**Language:** Rust | **Size:** 19.5 KB | **Lines:** 534

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

**Language:** Rust | **Size:** 17.3 KB | **Lines:** 473

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
  `async fn run_container( &self, image: &str, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

  `async fn pull_image(&self, image: &str) -> Result<(), ContainerError>`

  `async fn build_image( &self, dockerfile: &Path, tag: &str, _context_dir: &Path, ) -> Result<(), ContainerError>`

  `async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError>`

  `async fn prepare_language_environment( &self, language: &str, version: Option<&str>, _additional_packages: Option<Vec<String>>, ) -> Result<String, ContainerError>`


`fn check_command_available_secure(command: &str, name: &str, install_url: &str)`

`mod tests`

---

## crates/secrets/Cargo.toml

**Language:** TOML | **Size:** 1.7 KB | **Lines:** 61

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

**Language:** Rust | **Size:** 6.1 KB | **Lines:** 203

**Imports:**
- `crate::rate_limit::RateLimitConfig`
- `serde::{Deserialize, Serialize}`
- `std::collections::HashMap`

**Declarations:**

**`impl Default for SecretConfig`**
  `fn default() -> Self`


**`impl SecretConfig`**
  `pub fn from_file(path: &str) -> crate::SecretResult<Self>`

  `pub fn to_file(&self, path: &str) -> crate::SecretResult<()>`

  `pub fn from_env() -> Self`


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

**Language:** Rust | **Size:** 13.6 KB | **Lines:** 417

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

**Language:** Rust | **Size:** 8.8 KB | **Lines:** 252

**Imports:**
- `crate::{SecretManager, SecretResult}`
- `regex::Regex`
- `std::collections::HashMap`

**Declarations:**

**`impl<'a> SecretSubstitution<'a>`**
  `pub fn new(manager: &'a SecretManager) -> Self`

  `pub async fn substitute(&mut self, text: &str) -> SecretResult<String>`

  `async fn substitute_provider_secrets(&mut self, text: &str) -> SecretResult<String>`

  `async fn substitute_default_secrets(&mut self, text: &str) -> SecretResult<String>`

  `pub fn resolved_secrets(&self) -> &HashMap<String, String>`

  `pub fn contains_secrets(text: &str) -> bool`

  `pub fn extract_secret_refs(text: &str) -> Vec<SecretRef>`


**`impl SecretRef`**
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

**Language:** TOML | **Size:** 553 B | **Lines:** 22

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

**Language:** Rust | **Size:** 32.5 KB | **Lines:** 896

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

**Language:** Rust | **Size:** 34.6 KB | **Lines:** 968

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

**Language:** TOML | **Size:** 1.1 KB | **Lines:** 43

**Imports:**
- `crossterm`
- `ratatui`
- `reqwest`

**Declarations:**

---

## crates/ui/README.md

**Language:** Markdown | **Size:** 1.1 KB | **Lines:** 28

**Declarations:**

---

## crates/ui/src/app/mod.rs

**Language:** Rust | **Size:** 30.8 KB | **Lines:** 682

**Imports:**
- `crate::handlers::workflow::start_next_workflow_execution`
- `crate::models::{ExecutionResultMsg, QueuedExecution, Workflow, WorkflowStatus}`
- `crate::utils::load_workflows`
- `crate::views::{
    render_ui, TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_HELP, TAB_LOGS, TAB_SECRETS, TAB_TRIGGER,
    TAB_WORKFLOWS,
}`
- `crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
}`
- `ratatui::{backend::CrosstermBackend, Terminal}`
- `std::io::{self, stdout}`
- `std::path::PathBuf`
- `std::sync::mpsc`
- `std::time::{Duration, Instant}`
- *... and 2 more imports*

**Declarations:**

`mod state`

`fn run_tui_event_loop( terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, rx: &mpsc::Receiver<ExecutionResultMsg>, verbose: bool, ) -> io::Result<()>`

---

## crates/ui/src/app/state.rs

**Language:** Rust | **Size:** 176.9 KB | **Lines:** 4388

**Imports:**
- `crate::log_processor::{LogProcessingRequest, LogProcessor, ProcessedLogEntry}`
- `crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, QueuedExecution, StatusSeverity,
    StepExecution, TriggerMatchStatus, Workflow, WorkflowExecution, WorkflowStatus,
}`
- `chrono::Local`
- `crossterm::event::KeyCode`
- `ratatui::widgets::{ListState, TableState}`
- `std::path::{Path, PathBuf}`
- `std::sync::atomic::{AtomicBool, Ordering}`
- `std::sync::mpsc`
- `std::sync::Arc`
- `std::time::{Duration, Instant}`
- *... and 3 more imports*

**Declarations:**

**`impl TriggerPlatform`**
  `pub fn as_str(&self) -> &'static str`

  `pub fn toggle(self) -> Self`


**`impl Accent`**
  `pub fn as_str(&self) -> &'static str`

  `pub fn next(self) -> Self`

  `pub fn rgb(self) -> (u8, u8, u8)`


**`impl App`**
  `pub fn new( runtime_type: RuntimeType, tx: mpsc::Sender<ExecutionResultMsg>, preserve_containers_on_failure: bool, show_action_messages: bool, ) -> App`

  `pub fn toggle_selected(&mut self)`

  `pub fn toggle_emulation_mode(&mut self)`

  `pub fn cycle_diff_filter_event(&mut self)`

  `fn rerun_diff_filter_if_active(&mut self)`

  `fn abort_in_flight_evaluation(&mut self)`

  `fn spawn_evaluation(&mut self)`

  `pub fn toggle_diff_filter(&mut self)`

  `pub fn check_diff_filter_results(&mut self)`

  `pub fn toggle_validation_mode(&mut self)`

  `pub fn runtime_type_name(&self) -> &str`

  `pub fn previous_workflow(&mut self)`

  `pub fn next_workflow(&mut self)`

  `pub fn previous_job(&mut self)`

  `pub fn next_job(&mut self)`

  `pub fn previous_step(&mut self)`

  `pub fn next_step(&mut self)`

  `pub fn switch_tab(&mut self, tab: usize)`

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

  `pub fn toggle_log_search(&mut self)`

  `pub fn toggle_log_filter(&mut self)`

  `pub fn clear_log_search_and_filter(&mut self)`

  `pub fn update_log_search_matches(&mut self)`

  `pub fn next_search_match(&mut self)`

  `pub fn previous_search_match(&mut self)`

  `pub fn scroll_logs_up(&mut self)`

  `pub fn scroll_logs_down(&mut self)`

  `pub fn scroll_help_up(&mut self)`

  `pub fn scroll_help_down(&mut self)`

  `pub fn update_running_workflow_progress(&mut self)`

  `pub fn set_error_message(&mut self, message: String)`

  `pub fn set_warning_message(&mut self, message: String)`

  `pub fn set_info_message(&mut self, message: String)`

  `pub fn set_success_message(&mut self, message: String)`

  `pub fn tick(&mut self) -> bool`

  `pub fn trigger_selected_workflow(&mut self)`

  `pub fn reset_workflow_status(&mut self)`

  `pub fn request_log_processing_update(&mut self)`

  `pub fn check_log_processing_updates(&mut self)`

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

  `pub fn trigger_editing(&self) -> bool`

  `pub fn trigger_tab_add_input(&mut self)`

  `pub fn trigger_tab_edit_branch(&mut self)`

  `pub fn trigger_tab_next_field(&mut self)`

  `pub fn trigger_tab_prev_field(&mut self)`

  `pub fn trigger_tab_enter(&mut self)`

  `pub fn trigger_handle_input_key(&mut self, code: KeyCode) -> bool`

  `pub fn trigger_tab_copy_curl(&mut self)`

  `pub fn trigger_curl_preview(&self) -> String`

  `pub fn trigger_dispatch(&mut self)`

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

**Language:** Rust | **Size:** 1.8 KB | **Lines:** 78

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

**Language:** Rust | **Size:** 6.4 KB | **Lines:** 205

**Imports:**
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `std::collections::{HashMap, HashSet}`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`fn state_color(s: NodeState) -> ratatui::style::Color`

`fn state_glyph(s: NodeState, spinner_frame: usize) -> &'static str`

`fn truncate(s: &str, n: usize) -> String`

---

## crates/ui/src/components/mod.rs

**Language:** Rust | **Size:** 244 B | **Lines:** 13

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

## crates/ui/src/components/timing.rs

**Language:** Rust | **Size:** 3.4 KB | **Lines:** 112

**Imports:**
- `crate::theme::COLORS`
- `ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `wrkflw_executor::JobStatus`

**Declarations:**

`fn bar_props(s: Option<JobStatus>) -> (ratatui::style::Color, f32)`

`fn pad_right(s: &str, n: usize) -> String`

`fn pad_left(s: &str, n: usize) -> String`

`fn summarise(rows: &[TimingRow]) -> String`

---

## crates/ui/src/handlers/mod.rs

**Language:** Rust | **Size:** 42 B | **Lines:** 3

**Declarations:**

---

## crates/ui/src/handlers/workflow.rs

**Language:** Rust | **Size:** 25.3 KB | **Lines:** 666

**Imports:**
- `crate::cli_style`
- `std::io`
- `std::path::{Path, PathBuf}`
- `wrkflw_evaluator::evaluate_workflow_file`
- `wrkflw_executor::{self, JobStatus, RuntimeType, StepStatus}`
- `{
    crate::app::App,
    crate::models::{ExecutionResultMsg, WorkflowExecution, WorkflowStatus},
    chrono::Local,
    std::sync::mpsc,
    std::thread,
}`

**Declarations:**

---

## crates/ui/src/lib.rs

**Language:** Rust | **Size:** 980 B | **Lines:** 35

**Imports:**
- `pub use app::run_wrkflw_tui`
- `pub use handlers::workflow::execute_workflow_cli`
- `pub use handlers::workflow::validate_workflow`

**Declarations:**

---

## crates/ui/src/log_processor.rs

**Language:** Rust | **Size:** 11.5 KB | **Lines:** 340

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

**Language:** Rust | **Size:** 10.9 KB | **Lines:** 313

**Imports:**
- `chrono::Local`
- `std::path::PathBuf`
- `std::sync::Arc`
- `wrkflw_executor::{JobStatus, StepStatus}`
- `wrkflw_logging::symbols`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

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

## crates/ui/src/theme.rs

**Language:** Rust | **Size:** 9.9 KB | **Lines:** 311

**Imports:**
- `ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders},
}`
- `std::cell::Cell`
- `pub use wrkflw_logging::symbols`
- `crate::models::WorkflowStatus`
- `wrkflw_executor::{JobStatus, StepStatus}`

**Declarations:**

**`impl BadgeKind`**
  `pub fn fg(self) -> Color`


---

## crates/ui/src/utils/mod.rs

**Language:** Rust | **Size:** 3.0 KB | **Lines:** 84

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

**Language:** Rust | **Size:** 16.0 KB | **Lines:** 449

**Imports:**
- `crate::app::App`
- `crate::components::dag::{self, NodeState}`
- `crate::models::WorkflowStatus`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`
- `wrkflw_executor::JobStatus`
- `wrkflw_parser::workflow::WorkflowDefinition`

**Declarations:**

`fn render_empty_state(f: &mut Frame<'_>, area: Rect, msg: &str)`

`fn render_header(f: &mut Frame<'_>, app: &App, workflow: &crate::models::Workflow, area: Rect)`

`fn state_for_job( app: &App, workflow: &crate::models::Workflow, workflow_idx: usize, name: &str, ) -> NodeState`

`fn render_graph( f: &mut Frame<'_>, app: &App, def: &WorkflowDefinition, workflow: &crate::models::Workflow, workflow_idx: usize, area: Rect, )`

`fn render_topo_list( f: &mut Frame<'_>, app: &App, def: &WorkflowDefinition, workflow: &crate::models::Workflow, workflow_idx: usize, area: Rect, )`

`fn render_legend(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn truncate(s: &str, n: usize) -> String`

---

## crates/ui/src/views/execution_tab.rs

**Language:** Rust | **Size:** 16.4 KB | **Lines:** 464

**Imports:**
- `crate::app::App`
- `crate::components::{
    dag,
    progress_dots::{self, DotState},
    timing::{self, TimingRow},
}`
- `crate::models::WorkflowStatus`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`
- `wrkflw_executor::{JobStatus, RuntimeType, StepStatus}`

**Declarations:**

`const RIGHT_PANE_WIDTH: u16 = 40`

`const LEFT_PANE_WIDTH: u16 = 30`

`fn render_summary_strip(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect)`

`fn render_jobs_pane(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect)`

`fn render_steps_pane(f: &mut Frame<'_>, workflow: &crate::models::Workflow, area: Rect)`

`fn render_live_output_pane(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_dag_pane(f: &mut Frame<'_>, app: &App, idx: usize, area: Rect)`

`fn render_timing_pane(f: &mut Frame<'_>, workflow: &crate::models::Workflow, area: Rect)`

`fn active_job_execution( workflow: &crate::models::Workflow, ) -> Option<&crate::models::JobExecution>`

`fn active_job_name(workflow: &crate::models::Workflow) -> Option<String>`

`fn render_empty_state(f: &mut Frame<'_>, area: Rect)`

---

## crates/ui/src/views/help_overlay.rs

**Language:** Rust | **Size:** 10.4 KB | **Lines:** 286

**Imports:**
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
}`

**Declarations:**

`fn section_header<'a>(title: &'a str) -> Vec<Line<'a>>`

`fn key_line<'a>(key: &'a str, desc: &'a str) -> Line<'a>`

---

## crates/ui/src/views/job_detail.rs

**Language:** Rust | **Size:** 20.3 KB | **Lines:** 591

**Imports:**
- `crate::app::App`
- `crate::components::timing::{self, TimingRow}`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`
- `wrkflw_executor::{JobStatus, StepStatus}`

**Declarations:**

`const TABS: [&str`

`fn render_breadcrumb( f: &mut Frame<'_>, workflow_name: &str, job: &crate::models::JobExecution, area: Rect, )`

`fn render_tab_strip(f: &mut Frame<'_>, active: usize, area: Rect)`

`fn render_output_pane( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected_step: Option<usize>, area: Rect, )`

`fn render_steps_list( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected: Option<usize>, area: Rect, )`

`fn render_step_stdout( f: &mut Frame<'_>, job: &crate::models::JobExecution, selected: Option<usize>, area: Rect, )`

`fn render_env_pane(f: &mut Frame<'_>, area: Rect)`

`fn render_files_pane(f: &mut Frame<'_>, area: Rect)`

`fn render_matrix_pane( f: &mut Frame<'_>, workflow: &crate::models::Workflow, job_name: &str, area: Rect, )`

`fn format_yaml_scalar(v: &serde_yaml::Value) -> String`

`fn collapse_newlines(s: &str) -> String`

`fn inherited_combo_glyph<'a>(workflow: &'a crate::models::Workflow, job_name: &'a str) -> Span<'a>`

`fn render_timeline_pane(f: &mut Frame<'_>, job: &crate::models::JobExecution, area: Rect)`

---

## crates/ui/src/views/logs_tab.rs

**Language:** Rust | **Size:** 4.2 KB | **Lines:** 126

**Imports:**
- `crate::app::App`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
    Frame,
}`

**Declarations:**

---

## crates/ui/src/views/mod.rs

**Language:** Rust | **Size:** 4.2 KB | **Lines:** 124

**Imports:**
- `pub use title_bar::{
    TAB_COUNT, TAB_DAG, TAB_EXECUTION, TAB_HELP, TAB_LOGS, TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS,
}`
- `crate::app::App`
- `ratatui::Frame`

**Declarations:**

`mod dag_tab`

`mod execution_tab`

`mod help_overlay`

`mod job_detail`

`mod logs_tab`

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

## crates/ui/src/views/secrets_tab.rs

**Language:** Rust | **Size:** 9.8 KB | **Lines:** 292

**Imports:**
- `crate::app::App`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
}`
- `wrkflw_executor::RuntimeType`
- `wrkflw_secrets::{SecretConfig, SecretProviderConfig}`

**Declarations:**

`fn render_header(f: &mut Frame<'_>, area: Rect)`

`fn provider_entries() -> Vec<(String, SecretProviderConfig)>`

`fn render_providers_pane( f: &mut Frame<'_>, app: &mut App, rows: &[(String, SecretProviderConfig)], area: Rect, )`

`fn render_detail_pane( f: &mut Frame<'_>, app: &App, rows: &[(String, SecretProviderConfig)], area: Rect, )`

`fn render_runtime_pane( f: &mut Frame<'_>, app: &App, rows: &[(String, SecretProviderConfig)], area: Rect, )`

`fn kv<'a>(key: &'a str, value: impl Into<String>) -> Line<'a>`

---

## crates/ui/src/views/status_bar.rs

**Language:** Rust | **Size:** 5.7 KB | **Lines:** 180

**Imports:**
- `super::{TAB_DAG, TAB_EXECUTION, TAB_HELP, TAB_LOGS, TAB_SECRETS, TAB_TRIGGER, TAB_WORKFLOWS}`
- `crate::app::App`
- `crate::models::StatusSeverity`
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

**Language:** Rust | **Size:** 4.9 KB | **Lines:** 132

**Imports:**
- `crate::app::App`
- `crate::models::WorkflowStatus`
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

`fn live_elapsed(app: &App) -> Option<String>`

---

## crates/ui/src/views/trigger_tab.rs

**Language:** Rust | **Size:** 11.1 KB | **Lines:** 332

**Imports:**
- `crate::app::{App, TriggerPlatform}`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame,
}`

**Declarations:**

`fn token_is_set(var: &str) -> bool`

`fn render_header(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_target_pane(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn render_preview_pane(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn field_row<'a>(label: &'a str, value: &'a str) -> Line<'a>`

`fn field_row_hl<'a>(label: &'a str, value: &'a str, hint: &str) -> Line<'a>`

---

## crates/ui/src/views/tweaks_overlay.rs

**Language:** Rust | **Size:** 3.8 KB | **Lines:** 117

**Imports:**
- `crate::app::{Accent, App}`
- `crate::theme::{self, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
}`

**Declarations:**

`fn render_accent_row(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_shortcut_hint(f: &mut Frame<'_>, area: Rect)`

---

## crates/ui/src/views/workflows_tab.rs

**Language:** Rust | **Size:** 14.0 KB | **Lines:** 404

**Imports:**
- `crate::app::App`
- `crate::models::{TriggerMatchStatus, WorkflowStatus}`
- `crate::theme::{self, BadgeKind, COLORS}`
- `ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
}`

**Declarations:**

`fn render_workflow_list(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn render_right_column(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_preview(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn build_chain(def: &wrkflw_parser::workflow::WorkflowDefinition) -> String`

`fn render_trigger_filter(f: &mut Frame<'_>, app: &App, area: Rect)`

`fn render_quick_actions(f: &mut Frame<'_>, area: Rect)`

`fn render_job_selection(f: &mut Frame<'_>, app: &mut App, area: Rect)`

`fn workflow_status_badge(status: &WorkflowStatus) -> Span<'static>`

---

## crates/utils/Cargo.toml

**Language:** TOML | **Size:** 507 B | **Lines:** 22

**Declarations:**

---

## crates/utils/README.md

**Language:** Markdown | **Size:** 562 B | **Lines:** 21

**Declarations:**

---

## crates/utils/src/lib.rs

**Language:** Rust | **Size:** 6.8 KB | **Lines:** 199

**Imports:**
- `std::path::Path`

**Declarations:**

`mod tests`

---

## crates/validators/Cargo.toml

**Language:** TOML | **Size:** 540 B | **Lines:** 23

**Declarations:**

---

## crates/validators/README.md

**Language:** Markdown | **Size:** 719 B | **Lines:** 29

**Declarations:**

---

## crates/validators/src/actions.rs

**Language:** Rust | **Size:** 13.4 KB | **Lines:** 531

**Imports:**
- `std::collections::HashSet`
- `std::path::Path`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_local_action_inputs( action_file: &Path, with_params: Option<&serde_yaml::Mapping>, action_ref: &str, job_name: &str, step_idx: usize, result: &mut ValidationResult, )`

`mod tests`

---

## crates/validators/src/gitlab.rs

**Language:** Rust | **Size:** 7.8 KB | **Lines:** 235

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

---

## crates/validators/src/jobs.rs

**Language:** Rust | **Size:** 13.4 KB | **Lines:** 405

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `std::path::Path`
- `crate::{validate_env, validate_matrix, validate_steps}`
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn detect_cyclic_needs(jobs_map: &serde_yaml::Mapping, result: &mut ValidationResult)`

`fn dfs_detect_cycle( node: &str, graph: &HashMap<String, Vec<String>>, visited: &mut HashSet<String>, in_stack: &mut HashSet<String>, rec_stack: &mut Vec<String>, reported_cycles: &mut HashSet<Vec<String>>, result: &mut ValidationResult, )`

`mod tests`

---

## crates/validators/src/lib.rs

**Language:** Rust | **Size:** 1.4 KB | **Lines:** 48

**Imports:**
- `pub use actions::validate_action_reference`
- `pub use gitlab::validate_gitlab_pipeline`
- `pub use jobs::validate_jobs`
- `pub use matrix::validate_matrix`
- `pub use steps::validate_steps`
- `pub use triggers::validate_triggers`
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`mod actions`

`mod gitlab`

`mod jobs`

`mod matrix`

`mod steps`

`mod triggers`

`fn is_expression_string(v: &Value) -> bool`

`fn yaml_type_name(v: &Value) -> &'static str`

---

## crates/validators/src/matrix.rs

**Language:** Rust | **Size:** 4.2 KB | **Lines:** 119

**Imports:**
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_include_exclude(section: &Value, section_name: &str, result: &mut ValidationResult)`

`fn validate_matrix_parameter(name: &str, value: &Value, result: &mut ValidationResult)`

`fn get_value_type(value: &Value) -> &'static str`

---

## crates/validators/src/steps.rs

**Language:** Rust | **Size:** 5.4 KB | **Lines:** 174

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

**Language:** Rust | **Size:** 8.3 KB | **Lines:** 263

**Imports:**
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn validate_cron_syntax(cron: &str, result: &mut ValidationResult)`

`fn is_valid_cron_field(field: &str, min: u32, max: u32) -> bool`

`fn is_valid_cron_atom(atom: &str, min: u32, max: u32) -> bool`

`mod tests`

---

## crates/watcher/Cargo.toml

**Language:** TOML | **Size:** 622 B | **Lines:** 24

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

**Language:** Rust | **Size:** 42.0 KB | **Lines:** 886

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

**Language:** Rust | **Size:** 51.6 KB | **Lines:** 1189

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

**Language:** TOML | **Size:** 1.6 KB | **Lines:** 69

**Imports:**
- `walkdir`

**Declarations:**

---

## crates/wrkflw/README.md

**Language:** Markdown | **Size:** 4.8 KB | **Lines:** 131

**Declarations:**

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

**Language:** Rust | **Size:** 36.0 KB | **Lines:** 972

**Imports:**
- `bollard::Docker`
- `clap::{Parser, Subcommand, ValueEnum}`
- `std::collections::HashMap`
- `std::path::Path`
- `std::path::PathBuf`

**Declarations:**

`mod prefilter`

`mod run_workflow_cmd`

`mod watch_cmd`

`pub(crate) enum RuntimeChoice`
> Variants: `Auto`, `Docker`, `Podman`, `Emulation`, `SecureEmulation`

**`impl From<RuntimeChoice> for wrkflw_executor::RuntimeType`**
  `fn from(choice: RuntimeChoice) -> Self`


`struct Wrkflw`
> Fields: `command: Option<Commands>`, `verbose: bool`, `debug: bool`

`enum Commands`
> Variants: `Validate`, `Run`, `Watch`, `Tui`, `Trigger`, `TriggerGitlab`, `List`

`fn parse_key_val(s: &str) -> Result<(String, String), String>`

`async fn cleanup_on_exit()`

`async fn handle_signals()`

`pub(crate) fn is_gitlab_pipeline(path: &Path) -> bool`

`async fn main()`

`fn validate_github_workflow(path: &Path, verbose: bool) -> bool`

`fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool`

`fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool)`

`mod tests`

---

## crates/wrkflw/src/prefilter.rs

**Language:** Rust | **Size:** 37.8 KB | **Lines:** 898

**Imports:**
- `std::path::{Path, PathBuf}`

**Declarations:**

`pub(crate) enum PrefilterDecision`
> Variants: `Proceed`, `Skip`

`pub(crate) fn effective_strict_filter(strict: bool, no_strict: bool) -> bool`

`pub(crate) fn validate_event_requires_base_branch( event_name: &str, strict_filter: bool, ) -> Result<(), String>`

`pub(crate) struct PrefilterRequest<'a>`
> Fields: `workflow_path: &'a Path`, `event: Option<&'a String>`, `diff: bool`, `changed_files: Option<&'a Vec<String>>`, `diff_base: Option<&'a str>`, `diff_head: Option<&'a String>`, `base_branch: Option<&'a String>`, `activity_type: Option<&'a String>`, `verbose: bool`, `strict_filter: bool`

`pub(crate) async fn run_trigger_prefilter( req: PrefilterRequest<'_>, ) -> Result<PrefilterDecision, String>`

`pub(crate) async fn build_event_context( req: &PrefilterRequest<'_>, event_name: &str, cwd_for_git: Option<&Path>, ) -> Result<wrkflw_trigger_filter::EventContext, String>`

`pub(crate) fn apply_base_branch( ctx: &mut wrkflw_trigger_filter::EventContext, event_name: &str, base_branch: Option<&String>, strict_filter: bool, ) -> Result<(), String>`

`mod prefilter_tests`

---

## crates/wrkflw/src/run_workflow_cmd.rs

**Language:** Rust | **Size:** 8.6 KB | **Lines:** 221

**Imports:**
- `crate::prefilter`
- `crate::{is_gitlab_pipeline, RuntimeChoice}`
- `std::path::PathBuf`

**Declarations:**

`pub(crate) struct RunCtx`
> Fields: `path: PathBuf`, `runtime: RuntimeChoice`, `show_action_messages: bool`, `preserve_containers_on_failure: bool`, `gitlab: bool`, `job: Option<String>`, `event: Option<String>`, `diff: bool`, `changed_files: Option<Vec<String>>`, `diff_base: Option<String>`, `diff_head: Option<String>`, `base_branch: Option<String>`, `activity_type: Option<String>`, `strict_filter: bool`, `no_strict_filter: bool`, `verbose: bool`

`pub(crate) async fn run(ctx: RunCtx)`

---

## crates/wrkflw/src/watch_cmd.rs

**Language:** Rust | **Size:** 10.3 KB | **Lines:** 248

**Imports:**
- `crate::prefilter`
- `crate::RuntimeChoice`
- `std::path::PathBuf`

**Declarations:**

`pub(crate) struct WatchCtx`
> Fields: `path: Option<PathBuf>`, `runtime: RuntimeChoice`, `debounce: u64`, `event: String`, `show_action_messages: bool`, `preserve_containers_on_failure: bool`, `max_concurrency: usize`, `base_branch: Option<String>`, `activity_type: Option<String>`, `max_pending_events: Option<usize>`, `ignore_dirs: Vec<String>`, `strict_filter: bool`, `no_strict_filter: bool`, `verbose: bool`

`pub(crate) async fn run(ctx: WatchCtx)`

---

## crates/wrkflw/tests/target_job_test.rs

**Language:** Rust | **Size:** 3.6 KB | **Lines:** 141

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

**Language:** Markdown | **Size:** 2.5 KB | **Lines:** 123

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

**Language:** JSON | **Size:** 89.9 KB | **Lines:** 1711

**Declarations:**

---

## schemas/gitlab-ci.json

**Language:** JSON | **Size:** 104.7 KB | **Lines:** 3012

**Declarations:**

---

## scripts/bump-crate.sh

**Language:** Shell | **Size:** 3.1 KB | **Lines:** 97

---

## tests/README.md

**Language:** Markdown | **Size:** 974 B | **Lines:** 37

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
- `wrkflw::matrix::{self, MatrixCombination, MatrixConfig}`

**Declarations:**

`fn create_test_matrix() -> MatrixConfig`

`fn test_matrix_expansion()`

`fn test_format_combination_name()`

---

## tests/reusable_workflow_execution_test.rs

**Language:** Rust | **Size:** 3.1 KB | **Lines:** 122

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

