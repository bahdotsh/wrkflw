use bollard::Docker;
use clap::{Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

mod cli_secrets;
mod gate_cmd;
mod json_output;
mod mcp_cmd;
mod plan_cmd;
mod prefilter;
mod run_workflow_cmd;
mod runs_cmd;
mod watch_cmd;

/// How a command reports its result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Human-readable text (default)
    #[default]
    Text,
    /// JSON on stdout. Commands that stream, like `run`, emit one object per line
    Json,
}

impl OutputFormat {
    pub(crate) fn is_json(self) -> bool {
        self == OutputFormat::Json
    }
}

#[derive(Debug, Clone, ValueEnum)]
pub(crate) enum RuntimeChoice {
    /// Detect Docker first, then Podman, then fall back to emulation (default)
    Auto,
    /// Use Docker containers for isolation
    Docker,
    /// Use Podman containers for isolation
    Podman,
    /// Run each step as a pod in a Kubernetes cluster (see --k8s-* options)
    Kubernetes,
    /// Use process emulation mode (no containers, UNSAFE)
    Emulation,
    /// Use secure emulation mode with sandboxing (recommended for untrusted code)
    SecureEmulation,
}

impl RuntimeChoice {
    /// The value `--runtime` takes for this choice.
    pub(crate) fn as_flag(&self) -> &'static str {
        match self {
            RuntimeChoice::Auto => "auto",
            RuntimeChoice::Docker => "docker",
            RuntimeChoice::Podman => "podman",
            RuntimeChoice::Kubernetes => "kubernetes",
            RuntimeChoice::Emulation => "emulation",
            RuntimeChoice::SecureEmulation => "secure-emulation",
        }
    }
}

impl From<RuntimeChoice> for wrkflw_executor::RuntimeType {
    fn from(choice: RuntimeChoice) -> Self {
        match choice {
            RuntimeChoice::Auto => wrkflw_executor::RuntimeType::Auto,
            RuntimeChoice::Docker => wrkflw_executor::RuntimeType::Docker,
            RuntimeChoice::Podman => wrkflw_executor::RuntimeType::Podman,
            RuntimeChoice::Kubernetes => wrkflw_executor::RuntimeType::Kubernetes,
            RuntimeChoice::Emulation => wrkflw_executor::RuntimeType::Emulation,
            RuntimeChoice::SecureEmulation => wrkflw_executor::RuntimeType::SecureEmulation,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "wrkflw",
    about = "GitHub & GitLab CI/CD validator and executor",
    version,
    long_about = "A CI/CD validator and executor that runs workflows locally.\n\nExamples:\n  wrkflw validate                             # Validate all workflows in .github/workflows\n  wrkflw run .github/workflows/build.yml      # Run a specific workflow\n  wrkflw run .gitlab-ci.yml                   # Run a GitLab CI pipeline\n  wrkflw --verbose run .github/workflows/build.yml  # Run with more output\n  wrkflw --debug run .github/workflows/build.yml    # Run with detailed debug information\n  wrkflw run --runtime emulation .github/workflows/build.yml  # Use emulation mode instead of containers\n  wrkflw run --runtime podman .github/workflows/build.yml     # Use Podman instead of Docker\n  wrkflw run --runtime kubernetes --k8s-namespace ci --k8s-resource nvidia.com/gpu=1 .github/workflows/train.yml  # Run steps as pods in a cluster\n  wrkflw run --preserve-containers-on-failure .github/workflows/build.yml  # Keep failed containers for debugging"
)]
struct Wrkflw {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run in verbose mode with detailed output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Run in debug mode with extensive execution details
    #[arg(short, long, global = true)]
    debug: bool,

    /// Output format. `json` keeps stdout machine-readable and sends logs to stderr
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: OutputFormat,

    /// TUI color theme: auto, dark or light. Defaults to WRKFLW_THEME, then auto,
    /// which follows the terminal background
    #[cfg(feature = "tui")]
    #[arg(long, global = true, value_name = "THEME")]
    theme: Option<wrkflw_ui::theme::ThemePreference>,

    /// Run `runs-on` LABEL in IMAGE instead of the built-in image, e.g.
    /// ubuntu-latest=catthehacker/ubuntu:act-latest. Repeatable or comma-separated.
    /// A job-level `container:` still wins
    #[arg(
        short = 'P',
        long = "platform",
        global = true,
        value_name = "LABEL=IMAGE",
        env = "WRKFLW_PLATFORM",
        value_delimiter = ',',
        value_parser = parse_platform_arg
    )]
    platform: Vec<Option<(String, String)>>,

    #[command(flatten)]
    kubernetes: KubernetesArgs,
}

/// Cluster settings for `--runtime kubernetes`. Global so they sit next to
/// `--platform` and apply to run, watch and tui alike.
#[derive(Debug, Default, clap::Args)]
#[command(next_help_heading = "Kubernetes options")]
struct KubernetesArgs {
    /// kubeconfig context to run pods in (default: current context)
    #[arg(
        long = "k8s-context",
        global = true,
        value_name = "NAME",
        env = "WRKFLW_K8S_CONTEXT"
    )]
    k8s_context: Option<String>,

    /// Namespace to create pods in (default: the context's namespace)
    #[arg(
        long = "k8s-namespace",
        global = true,
        value_name = "NAME",
        env = "WRKFLW_K8S_NAMESPACE"
    )]
    k8s_namespace: Option<String>,

    /// nodeSelector applied to every pod, e.g. accelerator=a100. Repeatable or comma-separated
    #[arg(
        long = "k8s-node-selector",
        global = true,
        value_name = "KEY=VALUE",
        env = "WRKFLW_K8S_NODE_SELECTOR",
        value_delimiter = ',',
        value_parser = parse_k8s_key_val
    )]
    k8s_node_selector: Vec<Option<(String, String)>>,

    /// Resource limit and request for the step container, e.g. nvidia.com/gpu=1 or memory=8Gi.
    /// Repeatable or comma-separated
    #[arg(
        long = "k8s-resource",
        global = true,
        value_name = "NAME=QUANTITY",
        env = "WRKFLW_K8S_RESOURCE",
        value_delimiter = ',',
        value_parser = parse_k8s_key_val
    )]
    k8s_resource: Vec<Option<(String, String)>>,

    /// imagePullSecrets name added to every pod. Repeatable or comma-separated
    #[arg(
        long = "k8s-image-pull-secret",
        global = true,
        value_name = "NAME",
        env = "WRKFLW_K8S_IMAGE_PULL_SECRET",
        value_delimiter = ','
    )]
    k8s_image_pull_secret: Vec<String>,

    /// Registry prefix that images built locally are pushed to so the cluster can pull them,
    /// e.g. registry.example.com/ci. Needed for setup-* actions and Dockerfile actions
    #[arg(
        long = "k8s-registry",
        global = true,
        value_name = "REGISTRY",
        env = "WRKFLW_K8S_REGISTRY"
    )]
    k8s_registry: Option<String>,

    /// Image for the helper containers that sync the workspace; needs sh and tar
    #[arg(
        long = "k8s-helper-image",
        global = true,
        value_name = "IMAGE",
        env = "WRKFLW_K8S_HELPER_IMAGE"
    )]
    k8s_helper_image: Option<String>,

    /// Per-step timeout in seconds, from pod creation to workspace sync-back
    #[arg(
        long = "k8s-timeout",
        global = true,
        value_name = "SECONDS",
        env = "WRKFLW_K8S_TIMEOUT",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    k8s_timeout: Option<u64>,

    /// Seconds a pod may stay unschedulable before the step fails. Raise it if an autoscaler
    /// needs longer to bring up a node
    #[arg(
        long = "k8s-schedule-timeout",
        global = true,
        value_name = "SECONDS",
        env = "WRKFLW_K8S_SCHEDULE_TIMEOUT",
        value_parser = clap::value_parser!(u64).range(1..)
    )]
    k8s_schedule_timeout: Option<u64>,
}

impl KubernetesArgs {
    fn to_config(&self) -> wrkflw_executor::KubernetesConfig {
        wrkflw_executor::KubernetesConfig {
            context: self.k8s_context.clone(),
            namespace: self.k8s_namespace.clone(),
            node_selector: self.k8s_node_selector.iter().flatten().cloned().collect(),
            resources: self.k8s_resource.iter().flatten().cloned().collect(),
            image_pull_secrets: self
                .k8s_image_pull_secret
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            registry: self.k8s_registry.clone(),
            helper_image: self.k8s_helper_image.clone(),
            timeout_secs: self.k8s_timeout,
            schedule_timeout_secs: self.k8s_schedule_timeout,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate workflow or pipeline files
    Validate {
        /// Path(s) to workflow/pipeline file(s) or directory(ies) (defaults to .gitea/workflows and .github/workflows if none provided)
        #[arg(value_name = "path", num_args = 0..)]
        paths: Vec<PathBuf>,

        /// Explicitly validate as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,

        /// Set exit code to 1 on validation failure
        #[arg(long = "exit-code", default_value_t = true)]
        exit_code: bool,

        /// Don't set exit code to 1 on validation failure (overrides --exit-code)
        #[arg(long = "no-exit-code", conflicts_with = "exit_code")]
        no_exit_code: bool,
    },

    /// Execute workflow or pipeline files locally
    Run {
        /// Path to workflow/pipeline file to execute
        path: PathBuf,

        /// Container runtime to use (docker, podman, kubernetes, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "auto")]
        runtime: RuntimeChoice,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,

        /// Explicitly run as GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,

        /// Run only a specific job by name. A GitLab job runs even if its
        /// `rules:`, `only:`, `except:` or `when:` would exclude it.
        #[arg(long)]
        job: Option<String>,

        /// Set a GitLab pipeline variable as KEY=value (repeatable). It wins
        /// over the pipeline's and jobs' own, and `rules:` see it.
        #[arg(short = 'V', long = "variable", value_parser = parse_key_val)]
        variables: Vec<(String, String)>,

        /// Simulate a specific event type for trigger filtering (e.g., push, pull_request)
        #[arg(long)]
        event: Option<String>,

        /// Use git diff to determine changed files for trigger filtering
        #[arg(long)]
        diff: bool,

        /// Manually specify changed files (comma-separated) for trigger filtering
        #[arg(long, value_delimiter = ',')]
        changed_files: Option<Vec<String>>,

        /// Base ref for diff comparison.
        ///
        /// Omit to auto-detect: tries `origin/HEAD`, then `main`/`master`,
        /// then `HEAD~1`. Pass `HEAD` to compare working tree against the
        /// last commit (uncommitted changes only).
        #[arg(long)]
        diff_base: Option<String>,

        /// Head ref for diff comparison (default: working tree)
        #[arg(long)]
        diff_head: Option<String>,

        /// Target/base branch for pull_request events (e.g. main).
        /// GitHub Actions evaluates `branches:` filters on `pull_request`
        /// against the base branch — set this to simulate a PR locally.
        #[arg(long)]
        base_branch: Option<String>,

        /// Activity type for events that support it (e.g. `opened`,
        /// `synchronize` for pull_request). Required when simulating an
        /// event whose workflows use `types:` filters — without it, every
        /// such workflow is reported as skipped for "no activity type".
        #[arg(long)]
        activity_type: Option<String>,

        /// Reject degraded filter contexts (missing base branch on
        /// `pull_request`, `--event` without changed-file input, etc.)
        /// with a hard error instead of a log warning.
        ///
        /// Defaults to `true` so the CLI fails loudly on a
        /// silently-under-filtered run — the opposite of the
        /// warn-and-proceed behavior that produced "why did my
        /// workflow not fire?" tickets. Pass `--no-strict-filter` to
        /// opt back into the legacy warning behavior for scripts that
        /// have already adapted to it.
        #[arg(long = "strict-filter", default_value_t = true)]
        strict_filter: bool,

        /// Opposite of `--strict-filter`; re-enables the legacy
        /// warn-and-proceed behavior for degraded contexts. Kept as
        /// a separate flag instead of `--no-strict-filter` so clap's
        /// `conflicts_with` makes the intent explicit at the call
        /// site.
        #[arg(long = "no-strict-filter", conflicts_with = "strict_filter")]
        no_strict_filter: bool,

        /// Why this run is happening. Stored on the run record
        #[arg(long, env = wrkflw_runs::env::INTENT, value_name = "TEXT")]
        intent: Option<String>,

        /// Who started this run. Defaults to WRKFLW_AGENT, then a detected coding agent
        #[arg(long, value_name = "NAME")]
        agent: Option<String>,

        /// Don't record this run in the history
        #[arg(long)]
        no_record: bool,

        /// With --job, start at this step (name or 1-based position). Earlier steps are
        /// skipped, except actions/checkout, so their outputs and files are missing
        #[arg(long, value_name = "STEP", requires = "job")]
        from_step: Option<String>,

        /// Don't run a job whose fingerprint already passed. See .wrkflw/affected.yml
        #[arg(long)]
        reuse_results: bool,

        /// Also skip jobs whose .wrkflw/affected.yml paths have no changed file.
        /// Needs --event. Changed files come from --changed-files, or the git diff
        #[arg(long, requires = "event")]
        affected: bool,

        /// Run a job's steps in one container instead of one per step
        #[arg(long)]
        job_container: bool,

        /// Mount cargo, npm, pip and Go caches from the host into containers
        #[arg(long)]
        dep_cache: bool,

        /// Directory or URL of shared results. Needs WRKFLW_CACHE_SIGNING_KEY
        #[arg(long, env = "WRKFLW_REMOTE_CACHE", value_name = "LOCATION")]
        remote_cache: Option<String>,

        /// Upload passing results to --remote-cache
        #[arg(long)]
        cache_push: bool,

        /// Exit with this code when the workflow's triggers don't match, so
        /// `gate` can tell a skip from a pass
        #[arg(long, hide = true, value_name = "CODE")]
        skip_exit_code: Option<i32>,

        /// Run as `gate` does: skip jobs .wrkflw/affected.yml marks `gate: false`
        #[arg(long, hide = true)]
        gate_mode: bool,
    },

    /// Run what a push would run, and fail if any of it fails.
    ///
    /// Each workflow that fires for the event runs with result reuse, a job
    /// container and dependency caches. Use it before pushing, or as a hook.
    Gate {
        /// Directory or file of workflows (defaults to the detected directory)
        path: Option<PathBuf>,

        /// Event to simulate
        #[arg(long, default_value = "push")]
        event: String,

        /// Target branch, for a pull_request event
        #[arg(long)]
        base_branch: Option<String>,

        /// Compare against this ref (default: detected, as with --diff)
        #[arg(long)]
        diff_base: Option<String>,

        /// Container runtime to use
        #[arg(short, long, value_enum, default_value = "auto")]
        runtime: RuntimeChoice,

        /// Directory or URL of shared results. Needs WRKFLW_CACHE_SIGNING_KEY
        #[arg(long, env = "WRKFLW_REMOTE_CACHE", value_name = "LOCATION")]
        remote_cache: Option<String>,

        /// Install this as the repository's pre-push hook, then exit
        #[arg(long)]
        install_hook: bool,
    },

    /// Watch for file changes and re-run affected workflows.
    ///
    /// On Ctrl+C the watcher drains the current cycle gracefully:
    /// workflows already executing finish, the trigger-filter state
    /// is flushed, and the signal is passed through to the cleanup
    /// handler that reaps Docker containers and tempdirs. A hard
    /// exit only happens if the graceful drain is still running
    /// after ~10s — long enough for normal teardown, short enough
    /// that a hung subprocess cannot wedge the session.
    Watch {
        /// Path to workflow file or directory (defaults to .gitea/workflows if it exists, else .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, kubernetes, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "auto")]
        runtime: RuntimeChoice,

        /// Debounce interval in milliseconds
        #[arg(long, default_value = "500")]
        debounce: u64,

        /// Event type to simulate (default: push)
        #[arg(long, default_value = "push")]
        event: String,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,

        /// Maximum number of workflows that may execute concurrently per cycle
        #[arg(long, default_value_t = wrkflw_watcher::DEFAULT_MAX_CONCURRENT_EXECUTIONS)]
        max_concurrency: usize,

        /// Target/base branch for pull_request events (e.g. main).
        /// Required if you watch with `--event pull_request` and any workflow
        /// uses `branches:` to constrain the target branch.
        #[arg(long)]
        base_branch: Option<String>,

        /// Activity type for events that support it (e.g. `opened`,
        /// `synchronize` for pull_request). Required when watching an
        /// event whose workflows use `types:` filters — without it, every
        /// such workflow is silently rejected for "no activity type".
        #[arg(long)]
        activity_type: Option<String>,

        /// Upper bound on the debouncer's pending-event set. Events
        /// past this count during a churn burst are dropped and
        /// surfaced as a per-cycle warning so the user sees that
        /// something was missed. Omit the flag to use the debouncer's
        /// built-in default, which is sized for typical workloads.
        ///
        /// The flag is `Option<usize>` rather than `usize` with a
        /// sentinel `0 = default` value because `--max-pending-events 0`
        /// reads as "unbounded" to most users — the convention
        /// violation was flagged in review. `0` is now explicitly
        /// rejected at startup (warning + fall through to default)
        /// since a zero cap would drop every event and render the
        /// watcher useless.
        #[arg(long)]
        max_pending_events: Option<usize>,

        /// Extra directory names to ignore in addition to the built-in
        /// list (`.git`, `target`, `node_modules`, `.build`, `build`,
        /// `dist`, `__pycache__`, `.tox`, `.mypy_cache`, `.pytest_cache`,
        /// `.venv`, `venv`). Matched by directory-component name, not
        /// glob or path — a user file literally named `.terraform` is
        /// never silenced; only events whose parent path contains a
        /// `.terraform/` component are dropped. Pass multiple times
        /// or as a comma-separated list: `--ignore-dir .terraform
        /// --ignore-dir coverage` or `--ignore-dir .terraform,coverage`.
        #[arg(long = "ignore-dir", value_delimiter = ',')]
        ignore_dirs: Vec<String>,

        /// Reject degraded filter contexts (missing base branch on
        /// `pull_request`, unknown events, etc.) with a hard error
        /// instead of a log warning. Defaults to `true` so watch
        /// mode fails loudly on misconfiguration rather than running
        /// a session-long "0 triggered" stream.
        #[arg(long = "strict-filter", default_value_t = true)]
        strict_filter: bool,

        /// Opposite of `--strict-filter`; re-enables the legacy
        /// warn-and-proceed behavior for degraded contexts.
        #[arg(long = "no-strict-filter", conflicts_with = "strict_filter")]
        no_strict_filter: bool,
    },

    /// Open TUI interface to manage workflows
    #[cfg(feature = "tui")]
    Tui {
        /// Path to workflow file or directory (defaults to .gitea/workflows and .github/workflows)
        path: Option<PathBuf>,

        /// Container runtime to use (docker, podman, kubernetes, emulation, secure-emulation)
        #[arg(short, long, value_enum, default_value = "auto")]
        runtime: RuntimeChoice,

        /// Show 'Would execute GitHub action' messages in emulation mode
        #[arg(long, default_value_t = false)]
        show_action_messages: bool,

        /// Preserve Docker containers on failure for debugging (Docker mode only)
        #[arg(long)]
        preserve_containers_on_failure: bool,
    },

    /// Trigger a GitHub workflow remotely
    Trigger {
        /// Name of the workflow file (without .yml extension)
        workflow: String,

        /// Branch to run the workflow on
        #[arg(short, long)]
        branch: Option<String>,

        /// Key-value inputs for the workflow in format key=value
        #[arg(short, long, value_parser = parse_key_val)]
        input: Option<Vec<(String, String)>>,
    },

    /// Trigger a GitLab pipeline remotely
    TriggerGitlab {
        /// Branch to run the pipeline on
        #[arg(short, long)]
        branch: Option<String>,

        /// Key-value variables for the pipeline in format key=value
        #[arg(short = 'V', long, value_parser = parse_key_val)]
        variable: Option<Vec<(String, String)>>,
    },

    /// List available workflows and pipelines
    List {
        /// Show jobs within each workflow/pipeline
        #[arg(long)]
        jobs: bool,
    },

    /// Show what a run would do without executing anything
    Plan {
        /// Path to workflow/pipeline file
        path: PathBuf,

        /// Explicitly treat as a GitLab CI/CD pipeline
        #[arg(long)]
        gitlab: bool,

        /// Plan only this job and what it needs
        #[arg(long)]
        job: Option<String>,

        /// Event to evaluate the trigger against (e.g., push, pull_request)
        #[arg(long)]
        event: Option<String>,

        /// Use git diff to determine changed files
        #[arg(long)]
        diff: bool,

        /// Changed files (comma-separated)
        #[arg(long, value_delimiter = ',')]
        changed_files: Option<Vec<String>>,

        /// Base ref for diff comparison
        #[arg(long)]
        diff_base: Option<String>,

        /// Head ref for diff comparison (default: working tree)
        #[arg(long)]
        diff_head: Option<String>,

        /// Target branch for pull_request events
        #[arg(long)]
        base_branch: Option<String>,

        /// Activity type for `types:` filters (e.g. opened)
        #[arg(long)]
        activity_type: Option<String>,

        /// Reject degraded filter contexts instead of warning
        #[arg(long = "strict-filter", default_value_t = true)]
        strict_filter: bool,

        /// Warn about degraded filter contexts instead of rejecting them
        #[arg(long = "no-strict-filter", conflicts_with = "strict_filter")]
        no_strict_filter: bool,
    },

    /// Inspect and replay recorded runs
    Runs {
        #[command(subcommand)]
        command: runs_cmd::RunsCommand,
    },

    /// Model Context Protocol server for coding agents
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum McpCommand {
    /// Serve over stdio until stdin closes
    Serve,
}

// An empty value is skipped rather than rejected, so an exported but empty
// WRKFLW_PLATFORM or a trailing comma can't fail every command.
fn parse_platform_arg(s: &str) -> Result<Option<(String, String)>, String> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    wrkflw_executor::parse_platform_override(s).map(Some)
}

// Same leniency as `--platform`: an exported but empty WRKFLW_K8S_* or a
// trailing comma is skipped rather than failing every command.
fn parse_k8s_key_val(s: &str) -> Result<Option<(String, String)>, String> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    wrkflw_executor::parse_k8s_key_value(s).map(Some)
}

// Later entries win, so repeating a label behaves the same whatever its case.
fn collect_platform_overrides(entries: &[Option<(String, String)>]) -> HashMap<String, String> {
    entries.iter().flatten().cloned().collect()
}

// Parser function for key-value pairs
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{}`", s))?;

    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

// Make this function public for testing? Or move to a utils/cleanup mod?
// Or call wrkflw_executor::cleanup and wrkflw_runtime::cleanup directly?
// Let's try calling them directly for now.
async fn cleanup_on_exit() {
    // Clean up Docker resources if available, but don't let it block indefinitely.
    // Runs alongside the Kubernetes sweep: these timeouts plus the emulation one
    // would otherwise add up to the whole of handle_signals' hard exit budget,
    // and a stuck daemon must not cost a GPU pod its delete.
    let docker_cleanup = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => {
                // Assuming cleanup_resources exists in executor crate
                wrkflw_executor::cleanup_resources(&docker).await;
            }
            Err(_) => {
                // Docker not available
                wrkflw_logging::info("Docker not available, skipping Docker cleanup");
            }
        }
    });
    // Delete any pods a Kubernetes run still has in flight
    let kubernetes_cleanup = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        wrkflw_executor::kubernetes::cleanup_resources(),
    );

    let (docker_result, kubernetes_result) = tokio::join!(docker_cleanup, kubernetes_cleanup);
    match docker_result {
        Ok(_) => wrkflw_logging::debug("Docker cleanup completed successfully"),
        Err(_) => wrkflw_logging::warning(
            "Docker cleanup timed out after 3 seconds, continuing with shutdown",
        ),
    }
    match kubernetes_result {
        Ok(_) => wrkflw_logging::debug("Kubernetes cleanup completed successfully"),
        Err(_) => wrkflw_logging::warning("Kubernetes cleanup timed out, continuing with shutdown"),
    }

    // Always clean up emulation resources
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        // Assuming cleanup_resources exists in wrkflw_runtime::emulation module
        wrkflw_runtime::emulation::cleanup_resources(),
    )
    .await
    {
        Ok(_) => wrkflw_logging::debug("Emulation cleanup completed successfully"),
        Err(_) => wrkflw_logging::warning("Emulation cleanup timed out, continuing with shutdown"),
    }

    wrkflw_logging::info("Resource cleanup completed");
}

async fn handle_signals() {
    // Set up a hard exit timer in case cleanup takes too long
    // This ensures the app always exits even if Docker operations are stuck
    let hard_exit_time = std::time::Duration::from_secs(10);

    // Wait for Ctrl+C
    match tokio::signal::ctrl_c().await {
        Ok(_) => {
            eprintln!("Received Ctrl+C, shutting down and cleaning up...");
        }
        Err(e) => {
            // Log the error but continue with cleanup
            eprintln!("Warning: Failed to properly listen for ctrl+c event: {}", e);
            eprintln!("Shutting down and cleaning up...");
        }
    }

    // Set up a watchdog thread that will force exit if cleanup takes too long
    // This is important because Docker operations can sometimes hang indefinitely
    let _ = std::thread::spawn(move || {
        std::thread::sleep(hard_exit_time);
        eprintln!(
            "Cleanup taking too long (over {} seconds), forcing exit...",
            hard_exit_time.as_secs()
        );
        wrkflw_logging::error("Forced exit due to cleanup timeout");
        std::process::exit(1);
    });

    // Clean up containers
    cleanup_on_exit().await;

    // Exit with success status - the force exit thread will be terminated automatically
    std::process::exit(0);
}

/// Determines if a file is a GitLab CI/CD pipeline based on its name and content
pub(crate) fn is_gitlab_pipeline(path: &Path) -> bool {
    // First check the file name
    if let Some(file_name) = path.file_name() {
        if let Some(file_name_str) = file_name.to_str() {
            if file_name_str == ".gitlab-ci.yml" || file_name_str.ends_with("gitlab-ci.yml") {
                return true;
            }
        }
    }

    // Check if file is in .gitlab/ci directory
    if let Some(parent) = path.parent() {
        if let Some(parent_str) = parent.to_str() {
            if parent_str.ends_with(".gitlab/ci")
                && path
                    .extension()
                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
            {
                return true;
            }
        }
    }

    // If file exists, check the content
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            // GitLab CI/CD pipelines typically have stages, before_script, after_script at the top level
            if content.contains("stages:")
                || content.contains("before_script:")
                || content.contains("after_script:")
            {
                // Check for GitHub Actions specific keys that would indicate it's not GitLab
                if !content.contains("on:")
                    && !content.contains("runs-on:")
                    && !content.contains("uses:")
                {
                    return true;
                }
            }
        }
    }

    false
}

/// The arguments as text. Clap rejects arguments that aren't valid UTF-8, so
/// this runs on exactly the ones `std::env::args` would panic on.
fn raw_args() -> impl Iterator<Item = String> {
    std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
}

/// Whether the raw arguments ask for JSON. Parsing failed, so this is all
/// there is to go on.
fn args_request_json(args: impl Iterator<Item = String>) -> bool {
    let args: Vec<String> = args.take_while(|arg| arg != "--").collect();
    args.iter().any(|arg| arg == "--format=json")
        || args.windows(2).any(|pair| pair == ["--format", "json"])
}

/// Report a command line clap rejected, keeping the promise that stdout is
/// JSON in JSON mode, and exit 2. Help and version are not failures.
fn exit_on_parse_error(error: clap::Error) -> ! {
    use clap::error::ErrorKind;
    let is_usage_error = !matches!(
        error.kind(),
        ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    );
    if !is_usage_error || !args_request_json(raw_args()) {
        error.exit();
    }
    wrkflw_runs::emit_line(&serde_json::json!({
        "schema_version": wrkflw_runs::SCHEMA_VERSION,
        "type": "error",
        "message": error.render().to_string().trim(),
    }));
    let _ = error.print();
    std::process::exit(2);
}

/// Stack for the thread that runs the CLI and for the runtime's workers.
///
/// Windows gives the main thread 1 MiB, and a debug build overflows that in
/// clap's synchronous help and parsing frames before any command runs, so
/// boxing the future alone isn't enough. Other platforms default to 8 MiB.
const CLI_STACK_BYTES: usize = 16 * 1024 * 1024;

fn main() {
    // Before the thread below exists. See `cli_secrets`.
    cli_secrets::capture();
    let cli = std::thread::Builder::new()
        .name("wrkflw-main".to_string())
        .stack_size(CLI_STACK_BYTES)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .thread_stack_size(CLI_STACK_BYTES)
                .build()
                .expect("failed to start the async runtime");
            // On the heap too: the future for every command is large.
            runtime.block_on(Box::pin(run_cli()));
        })
        .expect("failed to start the main thread");
    if cli.join().is_err() {
        // The panic hook has already reported it.
        std::process::exit(101);
    }
}

async fn run_cli() {
    // Gracefully handle Broken pipe (EPIPE) when output is piped (e.g., to `head`)
    let default_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut is_broken_pipe = false;
        if let Some(s) = info.payload().downcast_ref::<&str>() {
            if s.contains("Broken pipe") {
                is_broken_pipe = true;
            }
        }
        if let Some(s) = info.payload().downcast_ref::<String>() {
            if s.contains("Broken pipe") {
                is_broken_pipe = true;
            }
        }
        if is_broken_pipe {
            // Treat as a successful, short-circuited exit
            std::process::exit(0);
        }
        // Fallback to the default hook for all other panics
        default_panic_hook(info);
    }));

    let cli = Wrkflw::try_parse().unwrap_or_else(|e| exit_on_parse_error(e));
    let verbose = cli.verbose;
    let debug = cli.debug;

    // stdout is a protocol in these modes, so logs must stay off it.
    let format = cli.format;
    if format.is_json() || matches!(cli.command, Some(Commands::Mcp { .. })) {
        wrkflw_logging::set_console_to_stderr(true);
    }

    // Set log level based on command line flags
    if debug {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Debug);
        wrkflw_logging::debug("Debug mode enabled - showing detailed logs");
    } else if verbose {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Info);
        wrkflw_logging::info("Verbose mode enabled");
    } else {
        wrkflw_logging::set_log_level(wrkflw_logging::LogLevel::Warning);
    }

    // Setup a Ctrl+C handler that runs in the background
    tokio::spawn(handle_signals());

    let platform_overrides = collect_platform_overrides(&cli.platform);
    let kubernetes = cli.kubernetes.to_config();

    match &cli.command {
        Some(Commands::Validate {
            paths,
            gitlab,
            exit_code,
            no_exit_code,
        }) => {
            // Determine the paths to validate. With none provided, cover every
            // default directory that exists; if there are none, fall through
            // with the conventional path so the "does not exist" error names it.
            let validate_paths: Vec<PathBuf> = if paths.is_empty() {
                let dirs = wrkflw_utils::default_workflow_dirs();
                if dirs.is_empty() {
                    vec![wrkflw_utils::default_workflow_dir()]
                } else {
                    dirs
                }
            } else {
                paths.clone()
            };

            // Determine if we're validating a GitLab pipeline based on the --gitlab flag or file detection
            let force_gitlab = *gitlab;
            let mut validation_failed = false;

            if format.is_json() {
                let failed = json_output::validate(&validate_paths, force_gitlab);
                if failed && *exit_code && !*no_exit_code {
                    std::process::exit(1);
                }
                return;
            }

            for validate_path in validate_paths {
                // Check if the path exists; if not, mark failure but continue
                if !validate_path.exists() {
                    eprintln!("Error: Path does not exist: {}", validate_path.display());
                    validation_failed = true;
                    continue;
                }

                if validate_path.is_dir() {
                    // Validate all workflow files in the directory
                    let rd = match std::fs::read_dir(&validate_path) {
                        Ok(rd) => rd,
                        Err(e) => {
                            eprintln!(
                                "Failed to read directory {}: {}",
                                validate_path.display(),
                                e
                            );
                            validation_failed = true;
                            continue;
                        }
                    };
                    let entries = rd
                        .filter_map(|entry| entry.ok())
                        .filter(|entry| {
                            entry.path().is_file()
                                && entry
                                    .path()
                                    .extension()
                                    .is_some_and(|ext| ext == "yml" || ext == "yaml")
                        })
                        .collect::<Vec<_>>();

                    println!(
                        "Validating {} workflow file(s) in {}...",
                        entries.len(),
                        validate_path.display()
                    );

                    for entry in entries {
                        let path = entry.path();
                        let is_gitlab = force_gitlab || is_gitlab_pipeline(&path);

                        let file_failed = if is_gitlab {
                            validate_gitlab_pipeline(&path, verbose)
                        } else {
                            validate_github_workflow(&path, verbose)
                        };

                        if file_failed {
                            validation_failed = true;
                        }
                    }
                } else {
                    // Validate a single workflow file
                    let is_gitlab = force_gitlab || is_gitlab_pipeline(&validate_path);

                    let file_failed = if is_gitlab {
                        validate_gitlab_pipeline(&validate_path, verbose)
                    } else {
                        validate_github_workflow(&validate_path, verbose)
                    };

                    if file_failed {
                        validation_failed = true;
                    }
                }
            }

            // Set exit code if validation failed and exit_code flag is true (and no_exit_code is false)
            if validation_failed && *exit_code && !*no_exit_code {
                std::process::exit(1);
            }
        }
        Some(Commands::Run {
            path,
            runtime,
            show_action_messages,
            preserve_containers_on_failure,
            gitlab,
            job,
            variables,
            event,
            diff,
            changed_files,
            diff_base,
            diff_head,
            base_branch,
            activity_type,
            strict_filter,
            no_strict_filter,
            intent,
            agent,
            no_record,
            from_step,
            reuse_results,
            affected,
            job_container,
            dep_cache,
            remote_cache,
            cache_push,
            skip_exit_code,
            gate_mode,
        }) => {
            run_workflow_cmd::run(run_workflow_cmd::RunCtx {
                path: path.clone(),
                runtime: runtime.clone(),
                show_action_messages: *show_action_messages,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                gitlab: *gitlab,
                job: job.clone(),
                variables: variables.iter().cloned().collect(),
                event: event.clone(),
                diff: *diff,
                changed_files: changed_files.clone(),
                diff_base: diff_base.clone(),
                diff_head: diff_head.clone(),
                base_branch: base_branch.clone(),
                activity_type: activity_type.clone(),
                strict_filter: *strict_filter,
                no_strict_filter: *no_strict_filter,
                verbose,
                platform_overrides,
                kubernetes,
                format,
                intent: intent.clone(),
                agent: agent.clone(),
                no_record: *no_record,
                from_step: from_step.clone(),
                reuse_results: *reuse_results,
                affected: *affected,
                job_container: *job_container,
                dep_cache: *dep_cache,
                remote_cache: remote_cache.clone(),
                cache_push: *cache_push,
                skip_exit_code: *skip_exit_code,
                gate_mode: *gate_mode,
            })
            .await;
        }
        Some(Commands::Watch {
            path,
            runtime,
            debounce,
            event,
            show_action_messages,
            preserve_containers_on_failure,
            max_concurrency,
            base_branch,
            activity_type,
            max_pending_events,
            ignore_dirs,
            strict_filter,
            no_strict_filter,
        }) => {
            watch_cmd::run(watch_cmd::WatchCtx {
                path: path.clone(),
                runtime: runtime.clone(),
                debounce: *debounce,
                event: event.clone(),
                show_action_messages: *show_action_messages,
                preserve_containers_on_failure: *preserve_containers_on_failure,
                max_concurrency: *max_concurrency,
                base_branch: base_branch.clone(),
                activity_type: activity_type.clone(),
                max_pending_events: *max_pending_events,
                ignore_dirs: ignore_dirs.clone(),
                strict_filter: *strict_filter,
                no_strict_filter: *no_strict_filter,
                verbose,
                platform_overrides,
                kubernetes,
                format,
            })
            .await;
        }
        Some(Commands::TriggerGitlab { branch, variable }) => {
            // Convert optional Vec<(String, String)> to Option<HashMap<String, String>>
            let variables = variable
                .as_ref()
                .map(|v| v.iter().cloned().collect::<HashMap<String, String>>());

            // Trigger the pipeline
            if let Err(e) = wrkflw_gitlab::trigger_pipeline(branch.as_deref(), variables).await {
                eprintln!("Error triggering GitLab pipeline: {}", e);
                std::process::exit(1);
            }
        }
        #[cfg(feature = "tui")]
        Some(Commands::Tui {
            path,
            runtime,
            show_action_messages,
            preserve_containers_on_failure,
        }) => {
            // Set runtime type based on the runtime choice
            let runtime_type = runtime.clone().into();

            // Call the TUI implementation from the ui crate
            if let Err(e) = wrkflw_ui::run_wrkflw_tui(
                path.as_ref(),
                runtime_type,
                verbose,
                *preserve_containers_on_failure,
                *show_action_messages,
                cli.theme,
                platform_overrides,
                kubernetes,
            )
            .await
            {
                eprintln!("Error running TUI: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::Trigger {
            workflow,
            branch,
            input,
        }) => {
            // Convert optional Vec<(String, String)> to Option<HashMap<String, String>>
            let inputs = input
                .as_ref()
                .map(|i| i.iter().cloned().collect::<HashMap<String, String>>());

            // Trigger the workflow
            if let Err(e) =
                wrkflw_github::trigger_workflow(workflow, branch.as_deref(), inputs).await
            {
                eprintln!("Error triggering GitHub workflow: {}", e);
                std::process::exit(1);
            }
        }
        Some(Commands::List { jobs }) => {
            if format.is_json() {
                json_output::list();
            } else {
                list_workflows_and_pipelines(verbose, *jobs);
            }
        }
        Some(Commands::Gate {
            path,
            event,
            base_branch,
            diff_base,
            runtime,
            remote_cache,
            install_hook,
        }) => {
            gate_cmd::run(gate_cmd::GateCtx {
                path: path.clone(),
                event: event.clone(),
                base_branch: base_branch.clone(),
                diff_base: diff_base.clone(),
                runtime: runtime.clone(),
                remote_cache: remote_cache.clone(),
                install_hook: *install_hook,
            });
        }
        Some(Commands::Plan {
            path,
            gitlab,
            job,
            event,
            diff,
            changed_files,
            diff_base,
            diff_head,
            base_branch,
            activity_type,
            strict_filter,
            no_strict_filter,
        }) => {
            plan_cmd::run(plan_cmd::PlanCtx {
                path: path.clone(),
                gitlab: *gitlab,
                job: job.clone(),
                event: event.clone(),
                diff: *diff,
                changed_files: changed_files.clone(),
                diff_base: diff_base.clone(),
                diff_head: diff_head.clone(),
                base_branch: base_branch.clone(),
                activity_type: activity_type.clone(),
                strict_filter: *strict_filter,
                no_strict_filter: *no_strict_filter,
                verbose,
                format,
            })
            .await;
        }
        Some(Commands::Runs { command }) => {
            runs_cmd::run(command.clone(), format).await;
        }
        Some(Commands::Mcp {
            command: McpCommand::Serve,
        }) => {
            mcp_cmd::serve().await;
        }
        None => {
            #[cfg(feature = "tui")]
            {
                // Launch TUI by default when no command is provided
                let runtime_type = wrkflw_executor::RuntimeType::Auto;

                // Call the TUI implementation from the ui crate with default path
                if let Err(e) = wrkflw_ui::run_wrkflw_tui(
                    None,
                    runtime_type,
                    verbose,
                    false,
                    false,
                    cli.theme,
                    platform_overrides,
                    kubernetes,
                )
                .await
                {
                    eprintln!("Error running TUI: {}", e);
                    std::process::exit(1);
                }
            }
            #[cfg(not(feature = "tui"))]
            {
                use clap::CommandFactory;
                Wrkflw::command().print_help().unwrap();
                println!();
            }
        }
    }
}

/// Validate a GitHub workflow file
/// Returns true if validation failed, false if it passed
fn validate_github_workflow(path: &Path, verbose: bool) -> bool {
    use wrkflw_ui::cli_style;
    print!("Validating GitHub workflow file: {}... ", path.display());

    match wrkflw_evaluator::evaluate_workflow_file(path, verbose) {
        Ok(result) => {
            if result.is_valid {
                println!("{}", cli_style::success("Valid"));
                if verbose {
                    println!("{}", cli_style::dim("  All validation checks passed"));
                }
                for warning in result.warnings() {
                    println!("{}", cli_style::indent(&format!("warning: {}", warning)));
                }
            } else {
                println!("{}", cli_style::error("Invalid"));
                for (i, issue) in result.issues.iter().enumerate() {
                    println!("{}", cli_style::indent(&format!("{}. {}", i + 1, issue)));
                }
            }
            !result.is_valid
        }
        Err(e) => {
            println!("{}", cli_style::error("Error"));
            eprintln!("  {}", e);
            true
        }
    }
}

/// Validate a GitLab CI/CD pipeline file
/// Returns true if validation failed, false if it passed
fn validate_gitlab_pipeline(path: &Path, verbose: bool) -> bool {
    use wrkflw_ui::cli_style;
    print!("Validating GitLab CI pipeline file: {}... ", path.display());

    match wrkflw_parser::gitlab::parse_pipeline(path) {
        Ok(pipeline) => {
            println!("{}", cli_style::success("Valid syntax"));

            let validation_result = wrkflw_validators::validate_gitlab_pipeline(&pipeline);

            if !validation_result.is_valid {
                println!("{}", cli_style::warning("Validation issues:"));
                for issue in validation_result.issues {
                    println!("{}", cli_style::indent(&format!("- {}", issue)));
                }
                true
            } else {
                if verbose {
                    println!("{}", cli_style::success("All validation checks passed"));
                }
                false // Validation passed
            }
        }
        Err(e) => {
            println!("{}", cli_style::error("Invalid"));
            eprintln!("Validation failed: {}", e);
            true
        }
    }
}

/// Print the workflow files in `dir`, optionally with their job names.
fn list_workflow_dir(dir: &Path, show_jobs: bool) {
    use colored::Colorize;
    use wrkflw_ui::cli_style;

    match std::fs::read_dir(dir) {
        Ok(rd) => {
            let entries: Vec<_> = rd
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry.path().is_file()
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|ext| ext == "yml" || ext == "yaml")
                })
                .collect();

            if entries.is_empty() {
                println!(
                    "{}",
                    cli_style::dim(&format!("  No workflow files found in {}", dir.display()))
                );
            } else {
                for (i, entry) in entries.iter().enumerate() {
                    let is_last = i == entries.len() - 1;
                    let connector = if is_last {
                        "\u{2514}\u{2500}\u{2500}"
                    } else {
                        "\u{251C}\u{2500}\u{2500}"
                    };
                    println!("{} {}", connector.dimmed(), entry.path().display());
                    if show_jobs {
                        let prefix = if is_last { "    " } else { "\u{2502}   " };
                        match wrkflw_parser::workflow::parse_workflow(&entry.path()) {
                            Ok(workflow) => {
                                let mut job_names: Vec<&String> = workflow.jobs.keys().collect();
                                job_names.sort();
                                println!(
                                    "{}{}",
                                    prefix.dimmed(),
                                    format!(
                                        "Jobs: {}",
                                        job_names
                                            .iter()
                                            .map(|s| s.as_str())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                    .dimmed()
                                );
                            }
                            Err(e) => {
                                eprintln!("{}Could not parse workflow: {}", prefix, e);
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            eprintln!(
                "{}",
                cli_style::error(&format!(
                    "Failed to read directory {}: {}",
                    dir.display(),
                    e
                ))
            );
        }
    }
}

/// List available workflows and pipelines in the repository
fn list_workflows_and_pipelines(verbose: bool, show_jobs: bool) {
    use colored::Colorize;
    use wrkflw_ui::cli_style;

    // Check for Gitea and GitHub workflows
    let dirs = wrkflw_utils::default_workflow_dirs();
    for dir in &dirs {
        let title = if dir.starts_with(".gitea") {
            "Gitea Workflows"
        } else {
            "GitHub Workflows"
        };
        println!("{}", title.bold().cyan());
        list_workflow_dir(dir, show_jobs);
    }
    if dirs.is_empty() {
        println!(
            "{}",
            cli_style::dim(
                "GitHub/Gitea Workflows: No .github/workflows or .gitea/workflows directory found"
            )
        );
    }

    // Check for GitLab CI pipeline
    let gitlab_path = PathBuf::from(".gitlab-ci.yml");
    if gitlab_path.exists() && gitlab_path.is_file() {
        println!("\n{}", "GitLab CI Pipeline".bold().cyan());
        println!(
            "{} {}",
            "\u{2514}\u{2500}\u{2500}".dimmed(),
            gitlab_path.display()
        );
        if show_jobs {
            match wrkflw_parser::gitlab::parse_pipeline(Path::new(".gitlab-ci.yml")) {
                Ok(pipeline) => {
                    let mut job_names: Vec<&String> = pipeline.jobs.keys().collect();
                    job_names.sort();
                    println!(
                        "    {}",
                        format!(
                            "Jobs: {}",
                            job_names
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                        .dimmed()
                    );
                }
                Err(e) => {
                    eprintln!("    Could not parse pipeline: {}", e);
                }
            }
        }
    } else {
        println!(
            "{}",
            cli_style::dim("GitLab CI Pipeline: No .gitlab-ci.yml file found")
        );
    }

    // Check for other GitLab CI pipeline files
    if verbose {
        println!(
            "\n{}",
            cli_style::info("Searching for other GitLab CI pipeline files...")
        );

        let entries = walkdir::WalkDir::new(".")
            .follow_links(true)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry.path().is_file()
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with("gitlab-ci.yml")
                    && entry.path() != gitlab_path
            })
            .collect::<Vec<_>>();

        if !entries.is_empty() {
            println!("{}", "Additional GitLab CI Pipeline files:".bold());
            for entry in entries {
                println!(
                    "{} {}",
                    "\u{2514}\u{2500}\u{2500}".dimmed(),
                    entry.path().display()
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_is_only_requested_ahead_of_the_path_separator() {
        let requests =
            |args: &[&str]| super::args_request_json(args.iter().map(|arg| arg.to_string()));
        assert!(requests(&["--format", "json", "run"]));
        assert!(requests(&["run", "--format=json"]));
        assert!(!requests(&["run", "--", "--format", "json"]));
        assert!(!requests(&["run", "--format", "text"]));
    }

    use super::*;

    #[test]
    fn runtime_choice_auto_maps_to_runtime_type_auto() {
        let rt: wrkflw_executor::RuntimeType = RuntimeChoice::Auto.into();
        assert_eq!(rt, wrkflw_executor::RuntimeType::Auto);
    }

    #[test]
    fn runtime_choice_all_variants_map_correctly() {
        use wrkflw_executor::RuntimeType;
        assert_eq!(RuntimeType::from(RuntimeChoice::Auto), RuntimeType::Auto);
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Docker),
            RuntimeType::Docker
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Podman),
            RuntimeType::Podman
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Kubernetes),
            RuntimeType::Kubernetes
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::Emulation),
            RuntimeType::Emulation
        );
        assert_eq!(
            RuntimeType::from(RuntimeChoice::SecureEmulation),
            RuntimeType::SecureEmulation
        );
    }

    fn platform_overrides_from(args: &[&str]) -> HashMap<String, String> {
        let cli = Wrkflw::try_parse_from(args).unwrap();
        collect_platform_overrides(&cli.platform)
    }

    fn k8s_config_from(args: &[&str]) -> wrkflw_executor::KubernetesConfig {
        Wrkflw::try_parse_from(args).unwrap().kubernetes.to_config()
    }

    #[test]
    fn k8s_flags_default_to_an_empty_config() {
        let cfg = k8s_config_from(&["wrkflw", "run", "ci.yml"]);
        assert_eq!(cfg, wrkflw_executor::KubernetesConfig::default());
    }

    #[test]
    fn k8s_flags_build_the_config_and_work_after_a_subcommand() {
        let cfg = k8s_config_from(&[
            "wrkflw",
            "run",
            "ci.yml",
            "--runtime",
            "kubernetes",
            "--k8s-context",
            "gpu",
            "--k8s-namespace",
            "ci",
            "--k8s-node-selector",
            "accelerator=a100,zone=eu",
            "--k8s-node-selector",
            "zone=us",
            "--k8s-resource",
            "nvidia.com/gpu=1",
            "--k8s-image-pull-secret",
            "regcred, ,other",
            "--k8s-registry",
            "registry.example.com/ci",
            "--k8s-helper-image",
            "alpine:3.19",
            "--k8s-timeout",
            "120",
            "--k8s-schedule-timeout",
            "900",
        ]);
        assert_eq!(cfg.context.as_deref(), Some("gpu"));
        assert_eq!(cfg.namespace.as_deref(), Some("ci"));
        assert_eq!(
            cfg.node_selector.get("accelerator").map(String::as_str),
            Some("a100")
        );
        assert_eq!(
            cfg.node_selector.get("zone").map(String::as_str),
            Some("us"),
            "later wins"
        );
        assert_eq!(
            cfg.resources.get("nvidia.com/gpu").map(String::as_str),
            Some("1")
        );
        assert_eq!(cfg.image_pull_secrets, vec!["regcred", "other"]);
        assert_eq!(cfg.registry.as_deref(), Some("registry.example.com/ci"));
        assert_eq!(cfg.helper_image.as_deref(), Some("alpine:3.19"));
        assert_eq!(cfg.timeout_secs, Some(120));
        assert_eq!(cfg.schedule_timeout_secs, Some(900));
    }

    #[test]
    fn k8s_flags_skip_empty_entries_and_reject_malformed_pairs() {
        let cfg = k8s_config_from(&["wrkflw", "--k8s-node-selector", "", "--k8s-resource", " "]);
        assert!(cfg.node_selector.is_empty());
        assert!(cfg.resources.is_empty());
        assert!(Wrkflw::try_parse_from(["wrkflw", "--k8s-resource", "gpu"]).is_err());
        assert!(Wrkflw::try_parse_from(["wrkflw", "--k8s-node-selector", "=x"]).is_err());
    }

    #[test]
    fn k8s_timeout_rejects_zero() {
        assert!(Wrkflw::try_parse_from(["wrkflw", "--k8s-timeout", "0"]).is_err());
        assert!(Wrkflw::try_parse_from(["wrkflw", "--k8s-schedule-timeout", "0"]).is_err());
        assert_eq!(
            k8s_config_from(&["wrkflw", "--k8s-timeout", "1"]).timeout_secs,
            Some(1)
        );
    }

    #[test]
    fn runtime_choice_accepts_kubernetes() {
        use wrkflw_executor::RuntimeType;
        let cli =
            Wrkflw::try_parse_from(["wrkflw", "run", "ci.yml", "--runtime", "kubernetes"]).unwrap();
        match cli.command {
            Some(Commands::Run { runtime, .. }) => {
                assert_eq!(RuntimeType::from(runtime), RuntimeType::Kubernetes)
            }
            other => panic!("unexpected command: {:?}", other),
        }
    }

    #[test]
    fn platform_flag_accepts_repeats_and_commas() {
        let map = platform_overrides_from(&[
            "wrkflw",
            "-P",
            "a=one:1,b=two:2",
            "--platform",
            "c=three:3",
        ]);
        assert_eq!(map.len(), 3);
        assert_eq!(map["a"], "one:1");
        assert_eq!(map["b"], "two:2");
        assert_eq!(map["c"], "three:3");
    }

    #[test]
    fn platform_flag_works_after_a_subcommand() {
        let map = platform_overrides_from(&["wrkflw", "run", "ci.yml", "-P", "a=one:1"]);
        assert_eq!(map["a"], "one:1");
    }

    #[test]
    fn platform_flag_skips_empty_entries() {
        assert!(platform_overrides_from(&["wrkflw", "-P", ""]).is_empty());
        let map = platform_overrides_from(&["wrkflw", "-P", "a=one:1,"]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn platform_flag_last_value_wins_across_label_case() {
        let map = platform_overrides_from(&["wrkflw", "-P", "Ubuntu=one:1", "-P", "ubuntu=two:2"]);
        assert_eq!(map.len(), 1);
        assert_eq!(map["ubuntu"], "two:2");
    }

    // The only test that sets this variable. Every other platform test passes
    // -P, which replaces it, so running alongside them is safe.
    #[test]
    fn platform_env_applies_unless_the_flag_is_given() {
        std::env::set_var("WRKFLW_PLATFORM", "a=env:1,b=env:2");
        let from_env = platform_overrides_from(&["wrkflw"]);
        let from_flag = platform_overrides_from(&["wrkflw", "-P", "a=flag:1"]);
        std::env::set_var("WRKFLW_PLATFORM", "");
        let from_empty = platform_overrides_from(&["wrkflw"]);
        std::env::remove_var("WRKFLW_PLATFORM");

        assert_eq!(from_env.len(), 2);
        assert_eq!(from_env["a"], "env:1");
        assert_eq!(from_env["b"], "env:2");
        assert_eq!(from_flag.len(), 1);
        assert_eq!(from_flag["a"], "flag:1");
        assert!(from_empty.is_empty());
    }

    #[test]
    fn platform_flag_rejects_malformed_values() {
        assert!(Wrkflw::try_parse_from(["wrkflw", "-P", "no-equals"]).is_err());
    }
}
