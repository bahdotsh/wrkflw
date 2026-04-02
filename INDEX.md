# Codebase Index: wrkflw

> Generated: 2026-04-02 07:55:32 UTC | Files: 158 | Lines: 44819
> Languages: C++ (1), JSON (4), Markdown (26), Python (1), Rust (75), Shell (5), TOML (16), YAML (30)

## Directory Structure

```
wrkflw/
  AGENTS.md
  BREAKING_CHANGES.md
  CLAUDE.md
  Cargo.toml
  GITLAB_USAGE.md
  INDEX.md
  README.md
  VERSION_MANAGEMENT.md
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
        dependency.rs
        docker.rs
        docker_test.rs
        engine.rs
        environment.rs
        lib.rs
        podman.rs
        substitution.rs
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
      README_SECURITY.md
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
    ui/
      Cargo.toml
      README.md
      src/
        app/
          mod.rs
          state.rs
        components/
          button.rs
          checkbox.rs
          mod.rs
          progress_bar.rs
        handlers/
          mod.rs
          workflow.rs
        lib.rs
        log_processor.rs
        models/
          mod.rs
        utils/
          mod.rs
        views/
          execution_tab.rs
          help_overlay.rs
          job_detail.rs
          logs_tab.rs
          mod.rs
          status_bar.rs
          title_bar.rs
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
    wrkflw/
      Cargo.toml
      README.md
      src/
        lib.rs
        main.rs
      tests/
        target_job_test.rs
  examples/
    secrets-demo/
      README.md
      secrets-workflow.yml
  hello.cpp
  hello.rs
  publish_crates.sh
  schemas/
    github-workflow.json
    gitlab-ci.json
  scripts/
    bump-crate.sh
  test.py
  tests/
    README.md
    TESTING_PODMAN.md
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
- `# Codebase Navigation — Use indxr MCP tools`

**BREAKING_CHANGES.md**
- `# Breaking Changes`

**CLAUDE.md**
- `# wrkflw`

**Cargo.toml**
- `[workspace]`
- `[workspace.package]`
- `[workspace.dependencies]`
- `[profile.release]`

**GITLAB_USAGE.md**
- `# Using wrkflw with GitLab Pipelines`
- `# Trigger using the default branch`
- `# Trigger on a specific branch`
- `# Trigger with custom variables`

**INDEX.md**
- `# Codebase Index: wrkflw`

**README.md**
- `# WRKFLW`
- `# Install Podman (varies by OS)`
- `# On macOS with Homebrew:`
- `# On Ubuntu/Debian:`
- `# Initialize Podman machine (macOS/Windows)`
- `# Use with wrkflw`
- `# Validate all workflow files in the default location (.github/workflows)`
- `# Validate a specific workflow file`
- `# Validate workflows in a specific directory`
- `# Validate multiple files and/or directories (GitHub and GitLab are auto-detected)`
- `# Force GitLab parsing for all provided paths`
- `# Validate with verbose output`
- `# Validate GitLab CI pipelines`
- `# Disable exit codes for custom error handling (default: enabled)`
- `# In CI/CD scripts - validation failure will cause the script to exit`
- `# For custom error handling, disable exit codes`
- `# Run a workflow with Docker (default)`
- `# Run a workflow with Podman instead of Docker`
- `# Run a workflow in emulation mode (without containers)`
- `# Run with verbose output`
- `# Preserve failed containers for debugging`
- `# Open TUI with workflows from the default directory`
- `# Open TUI with a specific directory of workflows`
- `# Open TUI with a specific workflow pre-selected`
- `# Open TUI with Podman runtime`
- `# Open TUI in emulation mode`
- `# Trigger a workflow remotely on GitHub`
- `# Trigger a pipeline remotely on GitLab`
- `# Example with validation failure`
- `# Navigate to project root and run wrkflw`
- `# This will automatically load .github/workflows files into the TUI`
- `# Preserve failed containers for debugging`
- `# Also available in TUI mode`
- `# Run workflows without root privileges`
- `# List preserved containers`
- `# Inspect a preserved container's filesystem (without executing)`
- `# Or run a new container with the same volumes`
- `# Clean up all wrkflw containers`
- `# Trigger a workflow using the default branch`
- `# Trigger a workflow on a specific branch`
- `# Trigger with input parameters`

**VERSION_MANAGEMENT.md**
- `# Version Management Guide`
- `# Internal crate dependencies`
- `# ... other crates`
- `# Internal crates`
- `# Bump all crates to the same version`
- `# Or specify exact version`
- `# Commit and tag`
- `# Bump a specific crate`
- `# The script will:`
- `# 1. Update the crate's Cargo.toml to use explicit version`
- `# 2. Update workspace dependencies`
- `# 3. Show you next steps`
- `# 1. Make your changes`
- `# 2. Bump version`
- `# 3. Commit and tag`
- `# 4. Push (this triggers GitHub Actions)`
- `# 1. Use helper script or manual method above`
- `# 2. Follow the script's suggestions`
- `# 3. Optionally publish to crates.io`
- `# Navigate to the crate`
- `# Ensure all dependencies are published first`
- `# (or available on crates.io)`
- `# Publish`
- `# Use cargo-workspaces`
- `# Solution: Check workspace dependencies match crate versions`
- `# Solution: Ensure all dependencies are published to crates.io first`
- `# Or use path dependencies only for local development`
- `# Solution: Ensure tag format matches workflow trigger`
- `# List all workspace members with versions`
- `# Check all crates`
- `# Test all crates`
- `# Show dependency tree`
- `# Show outdated dependencies`
- `# Verify publishability`

**cliff.toml**
- `[changelog]`
- `[#{{ contributor.pr_number }}]`
- `[git]`
- `[git.link]`

**crates/README.md**
- `# Wrkflw Crates`

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
- `pub fn add_matrix_context( env: &mut HashMap<String, String>, matrix_combination: &MatrixCombination, )`

**crates/executor/src/lib.rs**
- `pub mod action_resolver`
- `pub mod dependency`
- `pub mod docker`
- `pub mod engine`
- `pub mod environment`
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
- `pub enum LogLevel`
- `pub fn set_log_level(level: LogLevel)`
- `pub fn get_log_level() -> LogLevel`
- `pub fn log(level: LogLevel, message: &str)`
- `pub fn get_logs() -> Vec<String>`
- `pub fn clear_logs()`
- `pub fn debug(message: &str)`
- `pub fn info(message: &str)`
- `pub fn warning(message: &str)`
- `pub fn error(message: &str)`

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

**crates/runtime/README_SECURITY.md**
- `# Security Features in wrkflw Runtime`
- `# Use secure emulation mode (recommended)`
- `# Or via TUI`
- `# Legacy unsafe mode (not recommended)`
- `# This workflow will be blocked in secure emulation mode`
- `# This workflow will run successfully in secure emulation mode`

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
- `# Set default provider`
- `# Enable/disable secret masking`
- `# Set operation timeout`

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

**crates/ui/Cargo.toml**
- `[package]`
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

**crates/ui/src/components/button.rs**
- `pub struct Button`

**crates/ui/src/components/checkbox.rs**
- `pub struct Checkbox`

**crates/ui/src/components/progress_bar.rs**
- `pub struct ProgressBar`

**crates/ui/src/handlers/mod.rs**
- `pub mod workflow`

**crates/ui/src/handlers/workflow.rs**
- `pub fn validate_workflow(path: &Path, verbose: bool) -> io::Result<()>`
- `pub async fn execute_workflow_cli( path: &Path, runtime_type: RuntimeType, verbose: bool, show_action_messages: bool, ) -> io::Result<()>`
- `pub async fn execute_curl_trigger( workflow_name: &str, branch: Option<&str>, ) -> Result<(Vec<wrkflw_executor::JobResult>, ()), String>`
- `pub fn start_next_workflow_execution( app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, verbose: bool, )`

**crates/ui/src/lib.rs**
- `pub mod app`
- `pub mod components`
- `pub mod handlers`
- `pub mod log_processor`
- `pub mod models`
- `pub mod utils`
- `pub mod views`

**crates/ui/src/log_processor.rs**
- `pub struct ProcessedLogEntry`
- `pub struct LogProcessingRequest`
- `pub struct LogProcessingResponse`
- `pub struct LogProcessor`

**crates/ui/src/models/mod.rs**
- `pub type ExecutionResultMsg = (usize, Result<(Vec<wrkflw_executor::JobResult>, ()), String>)`
- `pub struct Workflow`
- `pub enum WorkflowStatus`
- `pub struct WorkflowExecution`
- `pub struct JobExecution`
- `pub struct StepExecution`
- `pub enum LogFilterLevel`

**crates/ui/src/utils/mod.rs**
- `pub fn load_workflows(dir_path: &Path) -> Vec<Workflow>`

**crates/ui/src/views/execution_tab.rs**
- `pub fn render_execution_tab( f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect, )`

**crates/ui/src/views/help_overlay.rs**
- `pub fn render_help_content( f: &mut Frame<CrosstermBackend<io::Stdout>>, area: Rect, scroll_offset: usize, )`
- `pub fn render_help_overlay(f: &mut Frame<CrosstermBackend<io::Stdout>>, scroll_offset: usize)`

**crates/ui/src/views/job_detail.rs**
- `pub fn render_job_detail_view( f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect, )`

**crates/ui/src/views/logs_tab.rs**
- `pub fn render_logs_tab(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect)`

**crates/ui/src/views/mod.rs**
- `pub fn render_ui(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App)`

**crates/ui/src/views/status_bar.rs**
- `pub fn render_status_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect)`

**crates/ui/src/views/title_bar.rs**
- `pub fn render_title_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect)`

**crates/ui/src/views/workflows_tab.rs**
- `pub fn render_workflows_tab( f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &mut App, area: Rect, )`

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

**crates/validators/README.md**
- `## wrkflw-validators`

**crates/validators/src/actions.rs**
- `pub fn validate_action_reference( action_ref: &str, job_name: &str, step_idx: usize, result: &mut ValidationResult, )`

**crates/validators/src/gitlab.rs**
- `pub fn validate_gitlab_pipeline(pipeline: &Pipeline) -> ValidationResult`

**crates/validators/src/jobs.rs**
- `pub fn validate_jobs(jobs: &Value, result: &mut ValidationResult)`

**crates/validators/src/matrix.rs**
- `pub fn validate_matrix(matrix: &Value, result: &mut ValidationResult)`

**crates/validators/src/steps.rs**
- `pub fn validate_steps(steps: &[Value], job_name: &str, result: &mut ValidationResult)`

**crates/validators/src/triggers.rs**
- `pub fn validate_triggers(on: &Value, result: &mut ValidationResult)`

**crates/wrkflw/Cargo.toml**
- `[package]`
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
- `# Use Podman or emulation instead of Docker`
- `# Open the TUI explicitly`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`
- `# tokio_test::block_on(async {`
- `# Ok::<_, Box<dyn std::error::Error>>(())`
- `# })?;`

**examples/secrets-demo/README.md**
- `# wrkflw Secrets Management Demo`
- `# Set secrets as environment variables`
- `# .github/workflows/secrets-demo.yml`
- `# secrets.env`
- `# ~/.wrkflw/secrets.yml`
- `# ~/.wrkflw/secrets.yml`
- `# With prefix`
- `# Direct environment variables`
- `# Original log:`
- `# "API response: {\"token\": \"ghp_1234567890abcdef\", \"status\": \"ok\"}"`
- `# Masked log:`
- `# "API response: {\"token\": \"ghp_***\", \"status\": \"ok\"}"`
- `# Check:`
- `# Check:`
- `# Secrets appearing in logs`
- `# Check:`
- `# This works directly in wrkflw`
- `# Before (environment variables)`
- `# After (wrkflw secrets)`
- `# Set in secrets.env:`
- `# API_KEY=your_key`
- `# Use in workflow:`
- `# ${{ secrets.API_KEY }}`

**examples/secrets-demo/secrets-workflow.yml**
- `name:`
- `on:`
- `jobs:`

**hello.cpp**
- `int main()`

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
- `# Testing Strategy`

**tests/TESTING_PODMAN.md**
- `# Testing Podman Support in WRKFLW`
- `# Fedora`
- `# RHEL/CentOS 8+`
- `# Using Chocolatey`
- `# Or download from https://podman.io/getting-started/installation`
- `# Should default to Docker`
- `# Should accept podman as runtime`
- `# Should accept emulation as runtime`
- `# Should reject invalid runtime`
- `# Ensure Podman is running`
- `# Test wrkflw detection`
- `# Temporarily make podman unavailable`
- `# Test fallback to emulation`
- `# Restore podman`
- `# Test same workflow with Docker`
- `# Test same workflow with emulation`
- `# Start TUI with Podman runtime`
- `# Start TUI with emulation runtime`
- `# Run a workflow that will fail`
- `# Check if containers were cleaned up`
- `# Check if failed container was preserved`
- `# Get container ID from previous step`
- `# Inspect the preserved container`
- `# Inside container: explore the environment, check files, etc.`
- `# Exit with: exit`
- `# Clean up manually`
- `# Create workflow that uses a specific image`
- `# Create a workflow that builds a custom image (if supported)`
- `# This tests the build_image functionality`
- `# Note: This test depends on language environment preparation`
- `# Compare results`
- `# Optional: Compare log outputs`

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

**Language:** Markdown | **Size:** 1.2 KB | **Lines:** 27

**Declarations:**

---

## BREAKING_CHANGES.md

**Language:** Markdown | **Size:** 1.3 KB | **Lines:** 30

**Declarations:**

---

## CLAUDE.md

**Language:** Markdown | **Size:** 4.4 KB | **Lines:** 66

**Declarations:**

---

## Cargo.toml

**Language:** TOML | **Size:** 2.2 KB | **Lines:** 73

**Declarations:**

---

## GITLAB_USAGE.md

**Language:** Markdown | **Size:** 2.2 KB | **Lines:** 83

**Declarations:**

---

## INDEX.md

**Language:** Markdown | **Size:** 85.9 KB | **Lines:** 3732

**Declarations:**

---

## README.md

**Language:** Markdown | **Size:** 24.5 KB | **Lines:** 611

**Declarations:**

---

## VERSION_MANAGEMENT.md

**Language:** Markdown | **Size:** 7.2 KB | **Lines:** 279

**Declarations:**

---

## cliff.toml

**Language:** TOML | **Size:** 3.6 KB | **Lines:** 106

**Declarations:**

---

## crates/README.md

**Language:** Markdown | **Size:** 3.0 KB | **Lines:** 97

**Declarations:**

---

## crates/evaluator/Cargo.toml

**Language:** TOML | **Size:** 499 B | **Lines:** 20

**Declarations:**

---

## crates/evaluator/README.md

**Language:** Markdown | **Size:** 793 B | **Lines:** 29

**Declarations:**

---

## crates/evaluator/src/lib.rs

**Language:** Rust | **Size:** 1.8 KB | **Lines:** 60

**Imports:**
- `colored::*`
- `serde_yaml::{self, Value}`
- `std::fs`
- `std::path::Path`
- `wrkflw_models::ValidationResult`
- `wrkflw_validators::{validate_jobs, validate_triggers}`

**Declarations:**

---

## crates/executor/Cargo.toml

**Language:** TOML | **Size:** 1.1 KB | **Lines:** 47

**Imports:**
- `ignore`

**Declarations:**

---

## crates/executor/README.md

**Language:** Markdown | **Size:** 902 B | **Lines:** 30

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

## crates/executor/src/dependency.rs

**Language:** Rust | **Size:** 17.4 KB | **Lines:** 507

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

**Language:** Rust | **Size:** 196.9 KB | **Lines:** 5511

**Imports:**
- `bollard::Docker`
- `futures::future`
- `serde_yaml::Value`
- `std::collections::HashMap`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::process::Command`
- `thiserror::Error`
- `ignore::{gitignore::GitignoreBuilder, Match}`
- `crate::action_resolver`
- *... and 12 more imports*

**Declarations:**

`fn is_gitlab_pipeline(path: &Path) -> bool`

`async fn execute_github_workflow( workflow_path: &Path, config: ExecutionConfig, ) -> Result<ExecutionResult, ExecutionError>`

`async fn execute_gitlab_pipeline( pipeline_path: &Path, config: ExecutionConfig, ) -> Result<ExecutionResult, ExecutionError>`

`fn create_gitlab_context(pipeline: &Pipeline, workspace_dir: &Path) -> HashMap<String, String>`

`fn resolve_gitlab_dependencies( pipeline: &Pipeline, workflow: &WorkflowDefinition, ) -> Result<Vec<Vec<String>>, ExecutionError>`

`fn initialize_runtime( runtime_type: RuntimeType, preserve_containers_on_failure: bool, ) -> Result<Box<dyn ContainerRuntime>, ExecutionError>`

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

`async fn build_combined_runtime_image( runtimes: &[SetupRuntime], base_image: &str, runtime: &dyn ContainerRuntime, ) -> Result<String, ExecutionError>`

`async fn resolve_runner_image( job: &Job, runtime: &dyn ContainerRuntime, ) -> Result<String, ExecutionError>`

`async fn execute_job_batch( jobs: &[String], workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, env_context: &HashMap<String, String>, verbose: bool, secret_manager: Option<&SecretManager>, secret_masker: Option<&SecretMasker>, ) -> Result<Vec<JobResult>, ExecutionError>`

`struct JobExecutionContext<'a>`
> Fields: `job_name: &'a str`, `workflow: &'a WorkflowDefinition`, `runtime: &'a dyn ContainerRuntime`, `env_context: &'a HashMap<String, String>`, `verbose: bool`, `secret_manager: Option<&'a SecretManager>`, `secret_masker: Option<&'a SecretMasker>`

`async fn execute_job_with_matrix( job_name: &str, workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, env_context: &HashMap<String, String>, verbose: bool, secret_manager: Option<&SecretManager>, secret_masker: Option<&SecretMasker>, ) -> Result<Vec<JobResult>, ExecutionError>`

`async fn execute_job(ctx: JobExecutionContext<'_>) -> Result<JobResult, ExecutionError>`

`struct MatrixExecutionContext<'a>`
> Fields: `job_name: &'a str`, `job_template: &'a Job`, `combinations: &'a [MatrixCombination]`, `max_parallel: usize`, `fail_fast: bool`, `workflow: &'a WorkflowDefinition`, `runtime: &'a dyn ContainerRuntime`, `env_context: &'a HashMap<String, String>`, `verbose: bool`, `secret_manager: Option<&'a SecretManager>`, `secret_masker: Option<&'a SecretMasker>`

`async fn execute_matrix_combinations( ctx: MatrixExecutionContext<'_>, ) -> Result<Vec<JobResult>, ExecutionError>`

`async fn execute_matrix_job( job_name: &str, job_template: &Job, combination: &MatrixCombination, workflow: &WorkflowDefinition, runtime: &dyn ContainerRuntime, base_env_context: &HashMap<String, String>, verbose: bool, ) -> Result<JobResult, ExecutionError>`

`enum StepOutcome`
> Variants: `Completed`, `Skipped`

`async fn run_step_with_guards( step: &Step, step_idx: usize, job_env: &HashMap<String, String>, workflow: &WorkflowDefinition, step_exec_ctx: StepExecutionContext<'_>, ) -> Result<StepOutcome, ExecutionError>`

`struct StepExecutionContext<'a>`
> Fields: `step: &'a workflow::Step`, `step_idx: usize`, `job_env: &'a HashMap<String, String>`, `working_dir: &'a Path`, `runtime: &'a dyn ContainerRuntime`, `workflow: &'a WorkflowDefinition`, `runner_image: &'a str`, `verbose: bool`, `matrix_combination: &'a Option<HashMap<String, Value>>`, `secret_manager: Option<&'a SecretManager>`, `secret_masker: Option<&'a SecretMasker>`, `container_config: Option<&'a JobContainer>`

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

`async fn prepare_runner_image( image: &str, runtime: &dyn ContainerRuntime, verbose: bool, ) -> Result<(), ExecutionError>`

`fn extract_language_info(image: &str) -> Option<(&'static str, Option<&str>)>`

`async fn execute_composite_action( step: &workflow::Step, action_path: &Path, job_env: &HashMap<String, String>, working_dir: &Path, runtime: &dyn ContainerRuntime, runner_image: &str, verbose: bool, ) -> Result<StepResult, ExecutionError>`

`fn convert_yaml_to_step(step_yaml: &serde_yaml::Value) -> Result<workflow::Step, String>`

`fn evaluate_job_condition( condition: &str, env_context: &HashMap<String, String>, workflow: &WorkflowDefinition, ) -> bool`

`mod tests`

---

## crates/executor/src/environment.rs

**Language:** Rust | **Size:** 7.1 KB | **Lines:** 242

**Imports:**
- `chrono::Utc`
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

`fn get_temp_dir() -> String`

`fn get_tool_cache_dir() -> String`

---

## crates/executor/src/lib.rs

**Language:** Rust | **Size:** 385 B | **Lines:** 17

**Imports:**
- `pub use docker::cleanup_resources`
- `pub use engine::{
    execute_workflow, ExecutionConfig, JobResult, JobStatus, RuntimeType, StepResult, StepStatus,
}`

**Declarations:**

---

## crates/executor/src/podman.rs

**Language:** Rust | **Size:** 34.2 KB | **Lines:** 922

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

**Language:** Rust | **Size:** 3.6 KB | **Lines:** 106

**Imports:**
- `lazy_static::lazy_static`
- `regex::Regex`
- `serde_yaml::Value`
- `std::collections::HashMap`

**Declarations:**

`mod tests`

---

## crates/github/Cargo.toml

**Language:** TOML | **Size:** 604 B | **Lines:** 24

**Declarations:**

---

## crates/github/README.md

**Language:** Markdown | **Size:** 653 B | **Lines:** 23

**Declarations:**

---

## crates/github/src/lib.rs

**Language:** Rust | **Size:** 11.1 KB | **Lines:** 340

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

`async fn list_recent_workflow_runs( repo_info: &RepoInfo, workflow_name: &str, token: &str, ) -> Result<Vec<serde_json::Value>, GithubError>`

---

## crates/gitlab/Cargo.toml

**Language:** TOML | **Size:** 618 B | **Lines:** 25

**Declarations:**

---

## crates/gitlab/README.md

**Language:** Markdown | **Size:** 608 B | **Lines:** 23

**Declarations:**

---

## crates/gitlab/src/lib.rs

**Language:** Rust | **Size:** 9.1 KB | **Lines:** 284

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

**Language:** Rust | **Size:** 2.7 KB | **Lines:** 107

**Imports:**
- `chrono::Local`
- `once_cell::sync::Lazy`
- `std::sync::{Arc, Mutex}`

**Declarations:**

`static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())))`

`static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)))`

**`impl LogLevel`**
  `fn prefix(&self) -> &'static str`


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

**Language:** Rust | **Size:** 8.3 KB | **Lines:** 264

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

**Language:** Rust | **Size:** 24.3 KB | **Lines:** 796

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

**Language:** Markdown | **Size:** 356 B | **Lines:** 13

**Declarations:**

---

## crates/runtime/README_SECURITY.md

**Language:** Markdown | **Size:** 8.4 KB | **Lines:** 258

**Declarations:**

---

## crates/runtime/src/container.rs

**Language:** Rust | **Size:** 2.7 KB | **Lines:** 89

**Imports:**
- `async_trait::async_trait`
- `std::path::Path`
- `std::fmt`

**Declarations:**

**`impl fmt::Display for ContainerError`**
  `fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result`


---

## crates/runtime/src/emulation.rs

**Language:** Rust | **Size:** 32.5 KB | **Lines:** 896

**Imports:**
- `crate::container::{ContainerError, ContainerOutput, ContainerRuntime}`
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
  `async fn run_container( &self, _image: &str, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, _volumes: &[(&Path, &Path)], _entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

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

**Language:** Rust | **Size:** 23.6 KB | **Lines:** 672

**Imports:**
- `regex::Regex`
- `std::collections::HashSet`
- `std::fs`
- `std::path::{Path, PathBuf}`
- `std::process::{Command, Stdio}`
- `std::time::Duration`
- `tempfile::TempDir`
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

  `fn setup_sandbox_environment(&self, working_dir: &Path) -> Result<PathBuf, SandboxError>`

  `fn copy_safe_files(&self, source: &Path, dest: &Path) -> Result<(), SandboxError>`

  `async fn execute_with_limits( &self, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, ) -> Result<crate::container::ContainerOutput, SandboxError>`

  `fn is_path_allowed(&self, path: &Path, write_access: bool) -> bool`

  `fn is_env_var_safe(&self, key: &str) -> bool`

  `fn should_skip_file(&self, filename: &str) -> bool`

  `fn should_skip_directory(&self, dirname: &str) -> bool`

  `fn compile_dangerous_patterns() -> Vec<Regex>`


`mod tests`

---

## crates/runtime/src/secure_emulation.rs

**Language:** Rust | **Size:** 13.3 KB | **Lines:** 359

**Imports:**
- `crate::container::{ContainerError, ContainerOutput, ContainerRuntime}`
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
  `async fn run_container( &self, image: &str, command: &[&str], env_vars: &[(&str, &str)], working_dir: &Path, _volumes: &[(&Path, &Path)], entrypoint: Option<&str>, ) -> Result<ContainerOutput, ContainerError>`

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

**Language:** Markdown | **Size:** 9.5 KB | **Lines:** 387

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

**Language:** Rust | **Size:** 7.4 KB | **Lines:** 247

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

**Language:** Rust | **Size:** 10.4 KB | **Lines:** 345

**Imports:**
- `regex::Regex`
- `std::collections::{HashMap, HashSet}`
- `std::sync::OnceLock`

**Declarations:**

`struct CompiledPatterns`
> Fields: `github_pat: Regex`, `github_app: Regex`, `github_oauth: Regex`, `aws_access_key: Regex`, `aws_secret: Regex`, `jwt: Regex`, `api_key: Regex`

**`impl CompiledPatterns`**
  `fn new() -> Self`


`static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new()`

**`impl SecretMasker`**
  `pub fn new() -> Self`

  `pub fn with_mask_char(mask_char: char) -> Self`

  `pub fn add_secret(&mut self, secret: impl Into<String>)`

  `pub fn add_secrets(&mut self, secrets: impl IntoIterator<Item = String>)`

  `pub fn remove_secret(&mut self, secret: &str)`

  `pub fn clear(&mut self)`

  `pub fn mask(&self, text: &str) -> String`

  `fn create_mask(&self, secret: &str) -> String`

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

**Language:** Rust | **Size:** 11.6 KB | **Lines:** 350

**Imports:**
- `std::collections::HashMap`
- `std::process`
- `tempfile::TempDir`
- `tokio`
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

## crates/ui/Cargo.toml

**Language:** TOML | **Size:** 841 B | **Lines:** 32

**Imports:**
- `reqwest`

**Declarations:**

---

## crates/ui/README.md

**Language:** Markdown | **Size:** 653 B | **Lines:** 23

**Declarations:**

---

## crates/ui/src/app/mod.rs

**Language:** Rust | **Size:** 21.3 KB | **Lines:** 503

**Imports:**
- `crate::handlers::workflow::start_next_workflow_execution`
- `crate::models::{ExecutionResultMsg, Workflow, WorkflowStatus}`
- `crate::utils::load_workflows`
- `crate::views::render_ui`
- `chrono::Local`
- `crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
}`
- `ratatui::{backend::CrosstermBackend, Terminal}`
- `std::io::{self, stdout}`
- `std::path::PathBuf`
- `std::sync::mpsc`
- *... and 3 more imports*

**Declarations:**

`mod state`

`fn run_tui_event_loop( terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App, tx_clone: &mpsc::Sender<ExecutionResultMsg>, rx: &mpsc::Receiver<ExecutionResultMsg>, verbose: bool, ) -> io::Result<()>`

---

## crates/ui/src/app/state.rs

**Language:** Rust | **Size:** 40.9 KB | **Lines:** 1065

**Imports:**
- `crate::log_processor::{LogProcessingRequest, LogProcessor, ProcessedLogEntry}`
- `crate::models::{
    ExecutionResultMsg, JobExecution, LogFilterLevel, StepExecution, Workflow, WorkflowExecution,
    WorkflowStatus,
}`
- `chrono::Local`
- `crossterm::event::KeyCode`
- `ratatui::widgets::{ListState, TableState}`
- `std::sync::mpsc`
- `std::time::{Duration, Instant}`
- `wrkflw_executor::{JobStatus, RuntimeType, StepStatus}`

**Declarations:**

**`impl App`**
  `pub fn new( runtime_type: RuntimeType, tx: mpsc::Sender<ExecutionResultMsg>, preserve_containers_on_failure: bool, show_action_messages: bool, ) -> App`

  `pub fn toggle_selected(&mut self)`

  `pub fn toggle_emulation_mode(&mut self)`

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

  `pub fn get_next_workflow_to_execute(&mut self) -> Option<usize>`

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

  `pub fn set_status_message(&mut self, message: String)`

  `pub fn tick(&mut self) -> bool`

  `pub fn trigger_selected_workflow(&mut self)`

  `pub fn reset_workflow_status(&mut self)`

  `pub fn request_log_processing_update(&mut self)`

  `pub fn check_log_processing_updates(&mut self)`

  `pub fn mark_logs_for_update(&mut self)`

  `pub fn get_combined_logs(&self) -> Vec<String>`

  `pub fn add_log(&mut self, message: String)`

  `pub fn add_timestamped_log(&mut self, message: &str)`


---

## crates/ui/src/components/button.rs

**Language:** Rust | **Size:** 1.3 KB | **Lines:** 53

**Imports:**
- `ratatui::{
    style::{Color, Modifier, Style},
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

**Language:** Rust | **Size:** 1.4 KB | **Lines:** 60

**Imports:**
- `ratatui::{
    style::{Color, Modifier, Style},
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

## crates/ui/src/components/mod.rs

**Language:** Rust | **Size:** 315 B | **Lines:** 12

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

**Language:** Rust | **Size:** 1.3 KB | **Lines:** 53

**Imports:**
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

## crates/ui/src/handlers/mod.rs

**Language:** Rust | **Size:** 42 B | **Lines:** 3

**Declarations:**

---

## crates/ui/src/handlers/workflow.rs

**Language:** Rust | **Size:** 22.3 KB | **Lines:** 575

**Imports:**
- `crate::app::App`
- `crate::models::{ExecutionResultMsg, WorkflowExecution, WorkflowStatus}`
- `chrono::Local`
- `std::io`
- `std::path::{Path, PathBuf}`
- `std::sync::mpsc`
- `std::thread`
- `wrkflw_evaluator::evaluate_workflow_file`
- `wrkflw_executor::{self, JobStatus, RuntimeType, StepStatus}`

**Declarations:**

---

## crates/ui/src/lib.rs

**Language:** Rust | **Size:** 674 B | **Lines:** 23

**Imports:**
- `pub use app::run_wrkflw_tui`
- `pub use handlers::workflow::execute_workflow_cli`
- `pub use handlers::workflow::validate_workflow`

**Declarations:**

---

## crates/ui/src/log_processor.rs

**Language:** Rust | **Size:** 11.2 KB | **Lines:** 330

**Imports:**
- `crate::models::LogFilterLevel`
- `ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Cell, Row},
}`
- `std::sync::mpsc`
- `std::thread`
- `std::time::{Duration, Instant}`

**Declarations:**

**`impl ProcessedLogEntry`**
  `pub fn to_row(&self) -> Row<'static>`


**`impl LogProcessor`**
  `pub fn new() -> Self`

  `pub fn request_update( &self, request: LogProcessingRequest, ) -> Result<(), mpsc::SendError<LogProcessingRequest>>`

  `pub fn try_get_update(&self) -> Option<LogProcessingResponse>`

  `fn worker_loop( request_rx: mpsc::Receiver<LogProcessingRequest>, response_tx: mpsc::Sender<LogProcessingResponse>, )`

  `fn get_combined_logs(app_logs: &[String]) -> Vec<String>`

  `fn process_logs(all_logs: &[String], request: &LogProcessingRequest) -> LogProcessingResponse`

  `fn process_log_entry(log_line: &str, search_query: &str) -> ProcessedLogEntry`

  `fn highlight_search_matches(content: &str, search_query: &str) -> Vec<Span<'static>>`


**`impl Default for LogProcessor`**
  `fn default() -> Self`


`mod tests`

---

## crates/ui/src/models/mod.rs

**Language:** Rust | **Size:** 2.8 KB | **Lines:** 100

**Imports:**
- `chrono::Local`
- `std::path::PathBuf`
- `wrkflw_executor::{JobStatus, StepStatus}`

**Declarations:**

**`impl LogFilterLevel`**
  `pub fn matches(&self, log: &str) -> bool`

  `pub fn next(&self) -> Self`

  `pub fn to_string(&self) -> &str`


---

## crates/ui/src/utils/mod.rs

**Language:** Rust | **Size:** 1.9 KB | **Lines:** 53

**Imports:**
- `crate::models::{Workflow, WorkflowStatus}`
- `std::path::{Path, PathBuf}`
- `wrkflw_utils::is_workflow_file`

**Declarations:**

---

## crates/ui/src/views/execution_tab.rs

**Language:** Rust | **Size:** 14.3 KB | **Lines:** 361

**Imports:**
- `crate::app::App`
- `crate::models::WorkflowStatus`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, List, ListItem, Paragraph},
    Frame,
}`
- `std::io`

**Declarations:**

---

## crates/ui/src/views/help_overlay.rs

**Language:** Rust | **Size:** 14.6 KB | **Lines:** 458

**Imports:**
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
}`
- `std::io`

**Declarations:**

---

## crates/ui/src/views/job_detail.rs

**Language:** Rust | **Size:** 9.6 KB | **Lines:** 211

**Imports:**
- `crate::app::App`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Row, Table},
    Frame,
}`
- `std::io`

**Declarations:**

---

## crates/ui/src/views/logs_tab.rs

**Language:** Rust | **Size:** 7.6 KB | **Lines:** 209

**Imports:**
- `crate::app::App`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
}`
- `std::io`

**Declarations:**

---

## crates/ui/src/views/mod.rs

**Language:** Rust | **Size:** 1.7 KB | **Lines:** 57

**Imports:**
- `crate::app::App`
- `ratatui::{backend::CrosstermBackend, Frame}`
- `std::io`

**Declarations:**

`mod execution_tab`

`mod help_overlay`

`mod job_detail`

`mod logs_tab`

`mod status_bar`

`mod title_bar`

`mod workflows_tab`

---

## crates/ui/src/views/status_bar.rs

**Language:** Rust | **Size:** 7.3 KB | **Lines:** 211

**Imports:**
- `crate::app::App`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
}`
- `std::io`
- `wrkflw_executor::RuntimeType`

**Declarations:**

---

## crates/ui/src/views/title_bar.rs

**Language:** Rust | **Size:** 2.5 KB | **Lines:** 74

**Imports:**
- `crate::app::App`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Tabs},
    Frame,
}`
- `std::io`

**Declarations:**

---

## crates/ui/src/views/workflows_tab.rs

**Language:** Rust | **Size:** 4.7 KB | **Lines:** 137

**Imports:**
- `crate::app::App`
- `crate::models::WorkflowStatus`
- `ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
}`
- `std::io`

**Declarations:**

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

**Language:** TOML | **Size:** 494 B | **Lines:** 20

**Declarations:**

---

## crates/validators/README.md

**Language:** Markdown | **Size:** 719 B | **Lines:** 29

**Declarations:**

---

## crates/validators/src/actions.rs

**Language:** Rust | **Size:** 2.0 KB | **Lines:** 58

**Imports:**
- `wrkflw_models::ValidationResult`

**Declarations:**

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

**Language:** Rust | **Size:** 12.0 KB | **Lines:** 354

**Imports:**
- `std::collections::{HashMap, HashSet}`
- `crate::{validate_matrix, validate_steps}`
- `serde_yaml::Value`
- `wrkflw_models::ValidationResult`

**Declarations:**

`fn detect_cyclic_needs(jobs_map: &serde_yaml::Mapping, result: &mut ValidationResult)`

`fn dfs_detect_cycle( node: &str, graph: &HashMap<String, Vec<String>>, visited: &mut HashSet<String>, in_stack: &mut HashSet<String>, rec_stack: &mut Vec<String>, reported_cycles: &mut HashSet<Vec<String>>, result: &mut ValidationResult, )`

`mod tests`

---

## crates/validators/src/lib.rs

**Language:** Rust | **Size:** 310 B | **Lines:** 15

**Imports:**
- `pub use actions::validate_action_reference`
- `pub use gitlab::validate_gitlab_pipeline`
- `pub use jobs::validate_jobs`
- `pub use matrix::validate_matrix`
- `pub use steps::validate_steps`
- `pub use triggers::validate_triggers`

**Declarations:**

`mod actions`

`mod gitlab`

`mod jobs`

`mod matrix`

`mod steps`

`mod triggers`

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

**Language:** Rust | **Size:** 3.5 KB | **Lines:** 107

**Imports:**
- `crate::validate_action_reference`
- `serde_yaml::Value`
- `std::collections::HashSet`
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

## crates/wrkflw/Cargo.toml

**Language:** TOML | **Size:** 1.5 KB | **Lines:** 65

**Imports:**
- `walkdir`

**Declarations:**

---

## crates/wrkflw/README.md

**Language:** Markdown | **Size:** 3.6 KB | **Lines:** 113

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

**Language:** Rust | **Size:** 29.2 KB | **Lines:** 783

**Imports:**
- `bollard::Docker`
- `clap::{Parser, Subcommand, ValueEnum}`
- `std::collections::HashMap`
- `std::path::Path`
- `std::path::PathBuf`

**Declarations:**

`enum RuntimeChoice`
> Variants: `Docker`, `Podman`, `Emulation`, `SecureEmulation`

**`impl From<RuntimeChoice> for wrkflw_executor::RuntimeType`**
  `fn from(choice: RuntimeChoice) -> Self`


`struct Wrkflw`
> Fields: `command: Option<Commands>`, `verbose: bool`, `debug: bool`

`enum Commands`
> Variants: `Validate`, `Run`, `Tui`, `Trigger`, `TriggerGitlab`, `List`

`fn parse_key_val(s: &str) -> Result<(String, String), String>`

`async fn cleanup_on_exit()`

`async fn handle_signals()`

`fn is_gitlab_pipeline(path: &Path) -> bool`

`async fn main()`

`fn validate_github_workflow(path: &Path, verbose: bool) -> bool`

`fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool`

`fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool)`

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

**Language:** Markdown | **Size:** 10.6 KB | **Lines:** 505

**Declarations:**

---

## examples/secrets-demo/secrets-workflow.yml

**Language:** YAML | **Size:** 6.9 KB | **Lines:** 213

**Declarations:**

---

## hello.cpp

**Language:** C++ | **Size:** 127 B | **Lines:** 6

**Declarations:**

---

## hello.rs

**Language:** Rust | **Size:** 70 B | **Lines:** 4

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

## test.py

**Language:** Python | **Size:** 53 B | **Lines:** 2

**Imports:**
- `import sys`

---

## tests/README.md

**Language:** Markdown | **Size:** 1.8 KB | **Lines:** 61

**Declarations:**

---

## tests/TESTING_PODMAN.md

**Language:** Markdown | **Size:** 13.2 KB | **Lines:** 487

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

