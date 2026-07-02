use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use tempfile;
use tokio::process::Command;
use wrkflw_logging;
use wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, LOCAL_IMAGE_PREFIX,
};
use wrkflw_utils;
use wrkflw_utils::fd;

static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
// Map to track customized images for a job
#[allow(dead_code)]
static CUSTOMIZED_IMAGES: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct PodmanRuntime {
    preserve_containers_on_failure: bool,
}

impl PodmanRuntime {
    pub fn new() -> Result<Self, ContainerError> {
        Self::new_with_config(false)
    }

    pub fn new_with_config(preserve_containers_on_failure: bool) -> Result<Self, ContainerError> {
        if !is_available() {
            return Err(ContainerError::ContainerStart(
                "Podman is not available on this system".to_string(),
            ));
        }
        Ok(Self::new_unchecked(preserve_containers_on_failure))
    }

    /// Construct without re-probing availability. Callers must have already verified
    /// that Podman is available via `is_available()`.
    pub(crate) fn new_unchecked(preserve_containers_on_failure: bool) -> Self {
        PodmanRuntime {
            preserve_containers_on_failure,
        }
    }

    // Add a method to store and retrieve customized images (e.g., with Python installed)
    #[allow(dead_code)]
    pub fn get_customized_image(base_image: &str, customization: &str) -> Option<String> {
        let key = format!("{}:{}", base_image, customization);
        match CUSTOMIZED_IMAGES.lock() {
            Ok(images) => images.get(&key).cloned(),
            Err(e) => {
                wrkflw_logging::error(&format!("Failed to acquire lock: {}", e));
                None
            }
        }
    }

    #[allow(dead_code)]
    pub fn set_customized_image(base_image: &str, customization: &str, new_image: &str) {
        let key = format!("{}:{}", base_image, customization);
        if let Err(e) = CUSTOMIZED_IMAGES.lock().map(|mut images| {
            images.insert(key, new_image.to_string());
        }) {
            wrkflw_logging::error(&format!("Failed to acquire lock: {}", e));
        }
    }

    /// Find a customized image key by prefix
    #[allow(dead_code)]
    pub fn find_customized_image_key(image: &str, prefix: &str) -> Option<String> {
        let image_keys = match CUSTOMIZED_IMAGES.lock() {
            Ok(keys) => keys,
            Err(e) => {
                wrkflw_logging::error(&format!("Failed to acquire lock: {}", e));
                return None;
            }
        };

        // Look for any key that starts with the prefix
        for (key, _) in image_keys.iter() {
            if key.starts_with(prefix) {
                return Some(key.clone());
            }
        }

        None
    }

    /// Get a customized image with language-specific dependencies
    pub fn get_language_specific_image(
        base_image: &str,
        language: &str,
        version: Option<&str>,
    ) -> Option<String> {
        let key = match (language, version) {
            ("python", Some(ver)) => format!("python:{}", ver),
            ("node", Some(ver)) => format!("node:{}", ver),
            ("java", Some(ver)) => format!("eclipse-temurin:{}", ver),
            ("go", Some(ver)) => format!("golang:{}", ver),
            ("dotnet", Some(ver)) => format!("mcr.microsoft.com/dotnet/sdk:{}", ver),
            ("rust", Some(ver)) => format!("rust:{}", ver),
            (lang, Some(ver)) => format!("{}:{}", lang, ver),
            (lang, None) => lang.to_string(),
        };

        match CUSTOMIZED_IMAGES.lock() {
            Ok(images) => images.get(&key).cloned(),
            Err(e) => {
                wrkflw_logging::error(&format!("Failed to acquire lock: {}", e));
                None
            }
        }
    }

    /// Set a customized image with language-specific dependencies
    pub fn set_language_specific_image(
        base_image: &str,
        language: &str,
        version: Option<&str>,
        new_image: &str,
    ) {
        let key = match (language, version) {
            ("python", Some(ver)) => format!("python:{}", ver),
            ("node", Some(ver)) => format!("node:{}", ver),
            ("java", Some(ver)) => format!("eclipse-temurin:{}", ver),
            ("go", Some(ver)) => format!("golang:{}", ver),
            ("dotnet", Some(ver)) => format!("mcr.microsoft.com/dotnet/sdk:{}", ver),
            ("rust", Some(ver)) => format!("rust:{}", ver),
            (lang, Some(ver)) => format!("{}:{}", lang, ver),
            (lang, None) => lang.to_string(),
        };

        if let Err(e) = CUSTOMIZED_IMAGES.lock().map(|mut images| {
            images.insert(key, new_image.to_string());
        }) {
            wrkflw_logging::error(&format!("Failed to acquire lock: {}", e));
        }
    }

    /// Execute a podman command with proper error handling and timeout
    async fn execute_podman_command(
        &self,
        args: &[&str],
        input: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        let timeout_duration = std::time::Duration::from_secs(360); // 6 minutes timeout

        let result = tokio::time::timeout(timeout_duration, async {
            let mut cmd = Command::new("podman");
            cmd.args(args);

            if input.is_some() {
                cmd.stdin(Stdio::piped());
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            wrkflw_logging::debug(&format!(
                "Running Podman command: podman {}",
                args.join(" ")
            ));

            let mut child = cmd.spawn().map_err(|e| {
                ContainerError::ContainerStart(format!("Failed to spawn podman command: {}", e))
            })?;

            // Send input if provided
            if let Some(input_data) = input {
                if let Some(stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    let mut stdin = stdin;
                    stdin.write_all(input_data.as_bytes()).await.map_err(|e| {
                        ContainerError::ContainerExecution(format!(
                            "Failed to write to stdin: {}",
                            e
                        ))
                    })?;
                    stdin.shutdown().await.map_err(|e| {
                        ContainerError::ContainerExecution(format!("Failed to close stdin: {}", e))
                    })?;
                }
            }

            let output = child.wait_with_output().await.map_err(|e| {
                ContainerError::ContainerExecution(format!("Podman command failed: {}", e))
            })?;

            Ok(ContainerOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
            })
        })
        .await;

        match result {
            Ok(output) => output,
            Err(_) => {
                wrkflw_logging::error("Podman operation timed out after 360 seconds");
                Err(ContainerError::ContainerExecution(
                    "Operation timed out".to_string(),
                ))
            }
        }
    }
}

pub fn is_available() -> bool {
    // Use a very short timeout for the entire availability check
    let overall_timeout = std::time::Duration::from_secs(3);

    // Spawn a thread with the timeout to prevent blocking the main thread
    let handle = std::thread::spawn(move || {
        // Use safe FD redirection utility to suppress Podman error messages
        match fd::with_stderr_to_null(|| {
            // First, check if podman CLI is available as a quick test
            if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
                // Try a simple podman version command with a short timeout
                let process = std::process::Command::new("podman")
                    .arg("version")
                    .arg("--format")
                    .arg("{{.Version}}")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();

                match process {
                    Ok(mut child) => {
                        // Set a very short timeout for the process
                        let status = std::thread::scope(|_| {
                            // Try to wait for a short time
                            for _ in 0..10 {
                                match child.try_wait() {
                                    Ok(Some(status)) => return status.success(),
                                    Ok(None) => {
                                        std::thread::sleep(std::time::Duration::from_millis(100))
                                    }
                                    Err(_) => return false,
                                }
                            }
                            // Kill it if it takes too long
                            let _ = child.kill();
                            false
                        });

                        if !status {
                            return false;
                        }
                    }
                    Err(_) => {
                        wrkflw_logging::debug("Podman CLI is not available");
                        return false;
                    }
                }
            }

            // Try to run a simple podman command to check if the daemon is responsive
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    wrkflw_logging::error(&format!(
                        "Failed to create runtime for Podman availability check: {}",
                        e
                    ));
                    return false;
                }
            };

            runtime.block_on(async {
                match tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    let mut cmd = Command::new("podman");
                    cmd.args(["info", "--format", "{{.Host.Hostname}}"]);
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());

                    match tokio::time::timeout(std::time::Duration::from_secs(1), cmd.output())
                        .await
                    {
                        Ok(Ok(output)) => {
                            if output.status.success() {
                                true
                            } else {
                                wrkflw_logging::debug("Podman info command failed");
                                false
                            }
                        }
                        Ok(Err(e)) => {
                            wrkflw_logging::debug(&format!("Podman info command error: {}", e));
                            false
                        }
                        Err(_) => {
                            wrkflw_logging::debug("Podman info command timed out after 1 second");
                            false
                        }
                    }
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::debug("Podman availability check timed out");
                        false
                    }
                }
            })
        }) {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::debug(
                    "Failed to redirect stderr when checking Podman availability",
                );
                false
            }
        }
    });

    // Manual implementation of join with timeout
    let start = std::time::Instant::now();

    while start.elapsed() < overall_timeout {
        if handle.is_finished() {
            return match handle.join() {
                Ok(result) => result,
                Err(_) => {
                    wrkflw_logging::warning("Podman availability check thread panicked");
                    false
                }
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    wrkflw_logging::warning(
        "Podman availability check timed out, assuming Podman is not available",
    );
    false
}

// Add container to tracking
pub fn track_container(id: &str) {
    if let Ok(mut containers) = RUNNING_CONTAINERS.lock() {
        containers.push(id.to_string());
    }
}

// Remove container from tracking
pub fn untrack_container(id: &str) {
    if let Ok(mut containers) = RUNNING_CONTAINERS.lock() {
        containers.retain(|c| c != id);
    }
}

// Clean up all tracked resources
pub async fn cleanup_resources() {
    // Use a global timeout for the entire cleanup process
    let cleanup_timeout = std::time::Duration::from_secs(5);

    match tokio::time::timeout(cleanup_timeout, cleanup_containers()).await {
        Ok(result) => {
            if let Err(e) = result {
                wrkflw_logging::error(&format!("Error during container cleanup: {}", e));
            }
        }
        Err(_) => wrkflw_logging::warning(
            "Podman cleanup timed out, some resources may not have been removed",
        ),
    }
}

// Clean up all tracked containers
pub async fn cleanup_containers() -> Result<(), String> {
    // Getting the containers to clean up should not take a long time
    let containers_to_cleanup =
        match tokio::time::timeout(std::time::Duration::from_millis(500), async {
            match RUNNING_CONTAINERS.try_lock() {
                Ok(containers) => containers.clone(),
                Err(_) => {
                    wrkflw_logging::error("Could not acquire container lock for cleanup");
                    vec![]
                }
            }
        })
        .await
        {
            Ok(containers) => containers,
            Err(_) => {
                wrkflw_logging::error("Timeout while trying to get containers for cleanup");
                vec![]
            }
        };

    if containers_to_cleanup.is_empty() {
        return Ok(());
    }

    wrkflw_logging::info(&format!(
        "Cleaning up {} containers",
        containers_to_cleanup.len()
    ));

    // Process each container with a timeout
    for container_id in containers_to_cleanup {
        // First try to stop the container
        let stop_result = tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            Command::new("podman")
                .args(["stop", &container_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output(),
        )
        .await;

        match stop_result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    wrkflw_logging::debug(&format!("Stopped container: {}", container_id));
                } else {
                    wrkflw_logging::warning(&format!("Error stopping container {}", container_id));
                }
            }
            Ok(Err(e)) => wrkflw_logging::warning(&format!(
                "Error stopping container {}: {}",
                container_id, e
            )),
            Err(_) => {
                wrkflw_logging::warning(&format!("Timeout stopping container: {}", container_id))
            }
        }

        // Then try to remove it
        let remove_result = tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            Command::new("podman")
                .args(["rm", &container_id])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output(),
        )
        .await;

        match remove_result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    wrkflw_logging::debug(&format!("Removed container: {}", container_id));
                } else {
                    wrkflw_logging::warning(&format!("Error removing container {}", container_id));
                }
            }
            Ok(Err(e)) => wrkflw_logging::warning(&format!(
                "Error removing container {}: {}",
                container_id, e
            )),
            Err(_) => {
                wrkflw_logging::warning(&format!("Timeout removing container: {}", container_id))
            }
        }

        // Always untrack the container whether or not we succeeded to avoid future cleanup attempts
        untrack_container(&container_id);
    }

    Ok(())
}

#[async_trait]
impl ContainerRuntime for PodmanRuntime {
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        // Print detailed debugging info
        wrkflw_logging::info(&format!("Podman: Running container with image: {}", image));

        let timeout_duration = std::time::Duration::from_secs(360); // 6 minutes timeout

        // Run the entire container operation with a timeout
        match tokio::time::timeout(
            timeout_duration,
            self.run_container_inner(image, cmd, env_vars, working_dir, volumes, entrypoint),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error("Podman operation timed out after 360 seconds");
                Err(ContainerError::ContainerExecution(
                    "Operation timed out".to_string(),
                ))
            }
        }
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        // Add a timeout for pull operations
        let timeout_duration = std::time::Duration::from_secs(30);

        match tokio::time::timeout(timeout_duration, self.pull_image_inner(image)).await {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::warning(&format!(
                    "Pull of image {} timed out, continuing with existing image",
                    image
                ));
                // Return success to allow continuing with existing image
                Ok(())
            }
        }
    }

    async fn build_image(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError> {
        // Add a timeout for build operations
        let timeout_duration = std::time::Duration::from_secs(120); // 2 minutes timeout for builds

        match tokio::time::timeout(
            timeout_duration,
            self.build_image_inner(dockerfile, tag, context_dir),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error(&format!(
                    "Building image {} timed out after 120 seconds",
                    tag
                ));
                Err(ContainerError::ImageBuild(
                    "Operation timed out".to_string(),
                ))
            }
        }
    }

    async fn prepare_language_environment(
        &self,
        language: &str,
        version: Option<&str>,
        additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError> {
        // Check if we already have a customized image for this language and version
        let key = format!("{}-{}", language, version.unwrap_or("latest"));
        if let Some(customized_image) = Self::get_language_specific_image("", language, version) {
            return Ok(customized_image);
        }

        // Create a temporary Dockerfile for customization
        let temp_dir = tempfile::tempdir().map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to create temp directory: {}", e))
        })?;

        let dockerfile_path = temp_dir.path().join("Dockerfile");
        let mut dockerfile_content = String::new();

        // Add language-specific setup based on the language
        match language {
            "python" => {
                let base_image =
                    version.map_or("python:3.11-slim".to_string(), |v| format!("python:{}", v));
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));
                dockerfile_content.push_str(
                    "RUN apt-get update && apt-get install -y --no-install-recommends \\\n",
                );
                dockerfile_content.push_str("    build-essential \\\n");
                dockerfile_content.push_str("    && rm -rf /var/lib/apt/lists/*\n");

                if let Some(packages) = additional_packages {
                    for package in packages {
                        dockerfile_content.push_str(&format!("RUN pip install {}\n", package));
                    }
                }
            }
            "node" => {
                let base_image =
                    version.map_or("node:20-slim".to_string(), |v| format!("node:{}", v));
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));
                dockerfile_content.push_str(
                    "RUN apt-get update && apt-get install -y --no-install-recommends \\\n",
                );
                dockerfile_content.push_str("    build-essential \\\n");
                dockerfile_content.push_str("    && rm -rf /var/lib/apt/lists/*\n");

                if let Some(packages) = additional_packages {
                    for package in packages {
                        dockerfile_content.push_str(&format!("RUN npm install -g {}\n", package));
                    }
                }
            }
            "java" => {
                let base_image = version.map_or("eclipse-temurin:17-jdk".to_string(), |v| {
                    format!("eclipse-temurin:{}", v)
                });
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));
                dockerfile_content.push_str(
                    "RUN apt-get update && apt-get install -y --no-install-recommends \\\n",
                );
                dockerfile_content.push_str("    maven \\\n");
                dockerfile_content.push_str("    && rm -rf /var/lib/apt/lists/*\n");
            }
            "go" => {
                let base_image =
                    version.map_or("golang:1.21-slim".to_string(), |v| format!("golang:{}", v));
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));
                dockerfile_content.push_str(
                    "RUN apt-get update && apt-get install -y --no-install-recommends \\\n",
                );
                dockerfile_content.push_str("    git \\\n");
                dockerfile_content.push_str("    && rm -rf /var/lib/apt/lists/*\n");

                if let Some(packages) = additional_packages {
                    for package in packages {
                        dockerfile_content.push_str(&format!("RUN go install {}\n", package));
                    }
                }
            }
            "dotnet" => {
                let base_image = version
                    .map_or("mcr.microsoft.com/dotnet/sdk:7.0".to_string(), |v| {
                        format!("mcr.microsoft.com/dotnet/sdk:{}", v)
                    });
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));

                if let Some(packages) = additional_packages {
                    for package in packages {
                        dockerfile_content
                            .push_str(&format!("RUN dotnet tool install -g {}\n", package));
                    }
                }
            }
            "rust" => {
                let base_image =
                    version.map_or("rust:latest".to_string(), |v| format!("rust:{}", v));
                dockerfile_content.push_str(&format!("FROM {}\n\n", base_image));
                dockerfile_content.push_str(
                    "RUN apt-get update && apt-get install -y --no-install-recommends \\\n",
                );
                dockerfile_content.push_str("    build-essential \\\n");
                dockerfile_content.push_str("    && rm -rf /var/lib/apt/lists/*\n");

                if let Some(packages) = additional_packages {
                    for package in packages {
                        dockerfile_content.push_str(&format!("RUN cargo install {}\n", package));
                    }
                }
            }
            _ => {
                return Err(ContainerError::ContainerStart(format!(
                    "Unsupported language: {}",
                    language
                )));
            }
        }

        // Write the Dockerfile
        std::fs::write(&dockerfile_path, dockerfile_content).map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to write Dockerfile: {}", e))
        })?;

        // Build the customized image
        let image_tag = format!("wrkflw-{}-{}", language, version.unwrap_or("latest"));
        self.build_image(&dockerfile_path, &image_tag, temp_dir.path())
            .await?;

        // Store the customized image
        Self::set_language_specific_image("", language, version, &image_tag);

        Ok(image_tag)
    }

    async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError> {
        let output = Command::new("podman")
            .args(["image", "exists", tag])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| {
                ContainerError::ImageBuild(format!("Failed to check image {}: {}", tag, e))
            })?;
        Ok(output.success())
    }
}

// Implementation of internal methods
impl PodmanRuntime {
    async fn run_container_inner(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        wrkflw_logging::debug(&format!("Running command in Podman: {:?}", cmd));
        wrkflw_logging::debug(&format!("Environment: {:?}", env_vars));
        wrkflw_logging::debug(&format!("Working directory: {}", working_dir.display()));

        // Generate a unique container name
        let container_name = format!("wrkflw-{}", uuid::Uuid::new_v4());

        // Build the podman run command and store temporary strings
        let working_dir_str = working_dir.to_string_lossy().to_string();
        let mut env_strings = Vec::new();
        let mut volume_strings = Vec::new();

        // Prepare environment variable strings
        for (key, value) in env_vars {
            env_strings.push(format!("{}={}", key, value));
        }

        // Prepare volume mount strings
        for (host_path, container_path) in volumes {
            volume_strings.push(format!(
                "{}:{}",
                host_path.to_string_lossy(),
                container_path.to_string_lossy()
            ));
        }

        let mut args = vec!["run", "--name", &container_name, "-w", &working_dir_str];

        // Skip registry pull for locally-built images (e.g., combined runtime images).
        if image.starts_with(LOCAL_IMAGE_PREFIX) {
            args.push("--pull=never");
        }

        // Only use --rm if we don't want to preserve containers on failure
        // When preserve_containers_on_failure is true, we skip --rm so failed containers remain
        if !self.preserve_containers_on_failure {
            args.insert(1, "--rm"); // Insert after "run"
        }

        // Add environment variables
        for env_string in &env_strings {
            args.push("-e");
            args.push(env_string);
        }

        // Add volume mounts
        for volume_string in &volume_strings {
            args.push("-v");
            args.push(volume_string);
        }

        // Override entrypoint if specified by action.yml
        let ep_string;
        if let Some(ep) = entrypoint.filter(|s| !s.is_empty()) {
            ep_string = ep.to_string();
            args.push("--entrypoint");
            args.push(&ep_string);
        }

        // Add the image
        args.push(image);

        // Add the command. If cmd is empty, nothing is appended and the
        // image's built-in ENTRYPOINT/CMD is used.
        args.extend(cmd);

        // Track the container (even though we use --rm, track it for consistency)
        track_container(&container_name);

        // Execute the command
        let result = self.execute_podman_command(&args, None).await;

        // Handle container cleanup based on result and settings
        match &result {
            Ok(output) => {
                if output.exit_code == 0 {
                    // Success - always clean up successful containers
                    if self.preserve_containers_on_failure {
                        // We didn't use --rm, so manually remove successful container
                        let cleanup_result = tokio::time::timeout(
                            std::time::Duration::from_millis(1000),
                            Command::new("podman")
                                .args(["rm", &container_name])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .output(),
                        )
                        .await;

                        match cleanup_result {
                            Ok(Ok(cleanup_output)) => {
                                if !cleanup_output.status.success() {
                                    wrkflw_logging::debug(&format!(
                                        "Failed to remove successful container {}",
                                        container_name
                                    ));
                                }
                            }
                            _ => wrkflw_logging::debug(&format!(
                                "Timeout removing successful container {}",
                                container_name
                            )),
                        }
                    }
                    // If not preserving, container was auto-removed with --rm
                    untrack_container(&container_name);
                } else {
                    // Failed container
                    if self.preserve_containers_on_failure {
                        // Failed and we want to preserve - don't clean up but untrack from auto-cleanup
                        wrkflw_logging::info(&format!(
                            "Preserving failed container {} for debugging (exit code: {}). Use 'podman exec -it {} bash' to inspect.",
                            container_name, output.exit_code, container_name
                        ));
                        untrack_container(&container_name);
                    } else {
                        // Failed but we don't want to preserve - container was auto-removed with --rm
                        untrack_container(&container_name);
                    }
                }
            }
            Err(_) => {
                // Command failed to execute properly - clean up if container exists and not preserving
                if !self.preserve_containers_on_failure {
                    // Container was created with --rm, so it should be auto-removed
                    untrack_container(&container_name);
                } else {
                    // Container was created without --rm, try to clean it up since execution failed
                    let cleanup_result = tokio::time::timeout(
                        std::time::Duration::from_millis(1000),
                        Command::new("podman")
                            .args(["rm", "-f", &container_name])
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .output(),
                    )
                    .await;

                    match cleanup_result {
                        Ok(Ok(_)) => wrkflw_logging::debug(&format!(
                            "Cleaned up failed execution container {}",
                            container_name
                        )),
                        _ => wrkflw_logging::debug(&format!(
                            "Failed to clean up execution failure container {}",
                            container_name
                        )),
                    }
                    untrack_container(&container_name);
                }
            }
        }

        match &result {
            Ok(output) => {
                if output.exit_code != 0 {
                    wrkflw_logging::info(&format!(
                        "Podman command failed with exit code: {}",
                        output.exit_code
                    ));
                    wrkflw_logging::debug(&format!("Failed command: {:?}", cmd));
                    wrkflw_logging::debug(&format!("Working directory: {}", working_dir.display()));
                    wrkflw_logging::debug(&format!("STDERR: {}", output.stderr));
                }
            }
            Err(e) => {
                wrkflw_logging::error(&format!("Podman execution error: {}", e));
            }
        }

        result
    }

    async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError> {
        let args = vec!["pull", image];
        let output = self.execute_podman_command(&args, None).await?;

        if output.exit_code != 0 {
            return Err(ContainerError::ImagePull(format!(
                "Failed to pull image {}: {}",
                image, output.stderr
            )));
        }

        Ok(())
    }

    async fn build_image_inner(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError> {
        let dockerfile_str = dockerfile.to_string_lossy().to_string();
        let context_dir_str = context_dir.to_string_lossy().to_string();
        let args = vec!["build", "-f", &dockerfile_str, "-t", tag, &context_dir_str];

        let output = self.execute_podman_command(&args, None).await?;

        if output.exit_code != 0 {
            return Err(ContainerError::ImageBuild(format!(
                "Failed to build image {}: {}",
                tag, output.stderr
            )));
        }

        Ok(())
    }
}

// Public accessor functions for testing
#[cfg(test)]
pub fn get_tracked_containers() -> Vec<String> {
    if let Ok(containers) = RUNNING_CONTAINERS.lock() {
        containers.clone()
    } else {
        vec![]
    }
}
