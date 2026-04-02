use async_trait::async_trait;
use bollard::{
    container::{Config, CreateContainerOptions},
    models::HostConfig,
    network::CreateNetworkOptions,
    Docker,
};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use wrkflw_logging;
use wrkflw_runtime::container::{
    ContainerError, ContainerOutput, ContainerRuntime, COMBINED_IMAGE_PREFIX, LOCAL_IMAGE_PREFIX,
};
use wrkflw_utils;
use wrkflw_utils::fd;

static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static CREATED_NETWORKS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
// Map to track customized images for a job
#[allow(dead_code)]
static CUSTOMIZED_IMAGES: Lazy<Mutex<HashMap<String, String>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub struct DockerRuntime {
    docker: Docker,
    preserve_containers_on_failure: bool,
}

impl DockerRuntime {
    pub fn new() -> Result<Self, ContainerError> {
        Self::new_with_config(false)
    }

    pub fn new_with_config(preserve_containers_on_failure: bool) -> Result<Self, ContainerError> {
        let docker = Docker::connect_with_local_defaults().map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to connect to Docker: {}", e))
        })?;

        Ok(DockerRuntime {
            docker,
            preserve_containers_on_failure,
        })
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

    /// Prepare a language-specific environment
    #[allow(dead_code)]
    pub async fn prepare_language_environment(
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
}

pub fn is_available() -> bool {
    // Use a very short timeout for the entire availability check
    let overall_timeout = std::time::Duration::from_secs(3);

    // Spawn a thread with the timeout to prevent blocking the main thread
    let handle = std::thread::spawn(move || {
        // Use safe FD redirection utility to suppress Docker error messages
        match fd::with_stderr_to_null(|| {
            // First, check if docker CLI is available as a quick test
            if cfg!(target_os = "linux") || cfg!(target_os = "macos") {
                // Try a simple docker version command with a short timeout
                let process = std::process::Command::new("docker")
                    .arg("version")
                    .arg("--format")
                    .arg("{{.Server.Version}}")
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
                        wrkflw_logging::debug("Docker CLI is not available");
                        return false;
                    }
                }
            }

            // Try to connect to Docker daemon with a short timeout
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    wrkflw_logging::error(&format!(
                        "Failed to create runtime for Docker availability check: {}",
                        e
                    ));
                    return false;
                }
            };

            runtime.block_on(async {
                match tokio::time::timeout(std::time::Duration::from_secs(2), async {
                    match Docker::connect_with_local_defaults() {
                        Ok(docker) => {
                            // Try to ping the Docker daemon with a short timeout
                            match tokio::time::timeout(
                                std::time::Duration::from_secs(1),
                                docker.ping(),
                            )
                            .await
                            {
                                Ok(Ok(_)) => true,
                                Ok(Err(e)) => {
                                    wrkflw_logging::debug(&format!(
                                        "Docker daemon ping failed: {}",
                                        e
                                    ));
                                    false
                                }
                                Err(_) => {
                                    wrkflw_logging::debug(
                                        "Docker daemon ping timed out after 1 second",
                                    );
                                    false
                                }
                            }
                        }
                        Err(e) => {
                            wrkflw_logging::debug(&format!(
                                "Docker daemon connection failed: {}",
                                e
                            ));
                            false
                        }
                    }
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        wrkflw_logging::debug("Docker availability check timed out");
                        false
                    }
                }
            })
        }) {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::debug(
                    "Failed to redirect stderr when checking Docker availability",
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
                    wrkflw_logging::warning("Docker availability check thread panicked");
                    false
                }
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    wrkflw_logging::warning(
        "Docker availability check timed out, assuming Docker is not available",
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

// Add network to tracking
pub fn track_network(id: &str) {
    if let Ok(mut networks) = CREATED_NETWORKS.lock() {
        networks.push(id.to_string());
    }
}

// Remove network from tracking
pub fn untrack_network(id: &str) {
    if let Ok(mut networks) = CREATED_NETWORKS.lock() {
        networks.retain(|n| n != id);
    }
}

// Clean up all tracked resources
pub async fn cleanup_resources(docker: &Docker) {
    // Use a global timeout for the entire cleanup process
    let cleanup_timeout = std::time::Duration::from_secs(5);

    match tokio::time::timeout(cleanup_timeout, async {
        // Perform both cleanups in parallel for efficiency
        let (container_result, network_result) =
            tokio::join!(cleanup_containers(docker), cleanup_networks(docker));

        if let Err(e) = container_result {
            wrkflw_logging::error(&format!("Error during container cleanup: {}", e));
        }

        if let Err(e) = network_result {
            wrkflw_logging::error(&format!("Error during network cleanup: {}", e));
        }
    })
    .await
    {
        Ok(_) => wrkflw_logging::debug("Docker cleanup completed within timeout"),
        Err(_) => wrkflw_logging::warning(
            "Docker cleanup timed out, some resources may not have been removed",
        ),
    }
}

// Clean up all tracked containers
pub async fn cleanup_containers(docker: &Docker) -> Result<(), String> {
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
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            docker.stop_container(&container_id, None),
        )
        .await
        {
            Ok(Ok(_)) => wrkflw_logging::debug(&format!("Stopped container: {}", container_id)),
            Ok(Err(e)) => wrkflw_logging::warning(&format!(
                "Error stopping container {}: {}",
                container_id, e
            )),
            Err(_) => {
                wrkflw_logging::warning(&format!("Timeout stopping container: {}", container_id))
            }
        }

        // Then try to remove it
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            docker.remove_container(&container_id, None),
        )
        .await
        {
            Ok(Ok(_)) => wrkflw_logging::debug(&format!("Removed container: {}", container_id)),
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

// Clean up all tracked networks
pub async fn cleanup_networks(docker: &Docker) -> Result<(), String> {
    // Getting the networks to clean up should not take a long time
    let networks_to_cleanup =
        match tokio::time::timeout(std::time::Duration::from_millis(500), async {
            match CREATED_NETWORKS.try_lock() {
                Ok(networks) => networks.clone(),
                Err(_) => {
                    wrkflw_logging::error("Could not acquire network lock for cleanup");
                    vec![]
                }
            }
        })
        .await
        {
            Ok(networks) => networks,
            Err(_) => {
                wrkflw_logging::error("Timeout while trying to get networks for cleanup");
                vec![]
            }
        };

    if networks_to_cleanup.is_empty() {
        return Ok(());
    }

    wrkflw_logging::info(&format!(
        "Cleaning up {} networks",
        networks_to_cleanup.len()
    ));

    for network_id in networks_to_cleanup {
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            docker.remove_network(&network_id),
        )
        .await
        {
            Ok(Ok(_)) => {
                wrkflw_logging::info(&format!("Successfully removed network: {}", network_id))
            }
            Ok(Err(e)) => {
                wrkflw_logging::error(&format!("Error removing network {}: {}", network_id, e))
            }
            Err(_) => wrkflw_logging::warning(&format!("Timeout removing network: {}", network_id)),
        }

        // Always untrack the network whether or not we succeeded
        untrack_network(&network_id);
    }

    Ok(())
}

// Create a new Docker network for a job
pub async fn create_job_network(docker: &Docker) -> Result<String, ContainerError> {
    let network_name = format!("wrkflw-network-{}", uuid::Uuid::new_v4());

    let options = CreateNetworkOptions {
        name: network_name.clone(),
        driver: "bridge".to_string(),
        ..Default::default()
    };

    let network = docker
        .create_network(options)
        .await
        .map_err(|e| ContainerError::NetworkCreation(e.to_string()))?;

    // network.id is Option<String>, unwrap it safely
    let network_id = network.id.ok_or_else(|| {
        ContainerError::NetworkOperation("Network created but no ID returned".to_string())
    })?;

    track_network(&network_id);
    wrkflw_logging::info(&format!("Created Docker network: {}", network_id));

    Ok(network_id)
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
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
        wrkflw_logging::info(&format!("Docker: Running container with image: {}", image));

        // Add a global timeout for all Docker operations to prevent freezing
        let timeout_duration = std::time::Duration::from_secs(360); // Increased outer timeout to 6 minutes

        // Run the entire container operation with a timeout
        match tokio::time::timeout(
            timeout_duration,
            self.run_container_inner(image, cmd, env_vars, working_dir, volumes, entrypoint),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error("Docker operation timed out after 360 seconds");
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
        // Add a timeout for build operations.
        // Combined runtime images may need to install packages from PPAs
        // and external sources, so allow up to 10 minutes.
        // Other builds use a 2 minute timeout.
        let timeout_secs = if tag.starts_with(COMBINED_IMAGE_PREFIX) {
            600
        } else {
            120
        };
        let timeout_duration = std::time::Duration::from_secs(timeout_secs);

        match tokio::time::timeout(
            timeout_duration,
            self.build_image_inner(dockerfile, tag, context_dir),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                wrkflw_logging::error(&format!(
                    "Building image {} timed out after {} seconds",
                    tag,
                    timeout_duration.as_secs()
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
        match self.docker.inspect_image(tag).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => Ok(false),
            Err(e) => Err(ContainerError::ImageBuild(format!(
                "Failed to inspect image {}: {}",
                tag, e
            ))),
        }
    }
}

// Move the actual implementation to internal methods
impl DockerRuntime {
    async fn run_container_inner(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        // Try to pull the image if it's not available locally.
        // Skip pull for locally-built images (e.g., combined runtime images).
        if !image.starts_with(LOCAL_IMAGE_PREFIX) {
            if let Err(e) = self.pull_image_inner(image).await {
                wrkflw_logging::warning(&format!(
                    "Failed to pull image {}: {}. Attempting to continue with existing image.",
                    image, e
                ));
            }
        }

        // Collect environment variables
        let mut env: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        let mut binds = Vec::new();
        for (host_path, container_path) in volumes {
            binds.push(format!(
                "{}:{}",
                host_path.to_string_lossy(),
                container_path.to_string_lossy()
            ));
        }

        // Convert command vector to Vec<String>
        let cmd_vec: Vec<String> = cmd.iter().map(|&s| s.to_string()).collect();
        let has_cmd = !cmd_vec.is_empty();

        wrkflw_logging::debug(&format!("Running command in Docker: {:?}", cmd_vec));
        wrkflw_logging::debug(&format!("Environment: {:?}", env));
        wrkflw_logging::debug(&format!("Working directory: {}", working_dir.display()));

        // Determine platform-specific configurations
        let is_windows_image = image.contains("windows")
            || image.contains("servercore")
            || image.contains("nanoserver");
        let is_macos_emu =
            image.contains("act-") && (image.contains("catthehacker") || image.contains("nektos"));

        // Add platform-specific environment variables
        if is_macos_emu {
            // Add macOS-specific environment variables
            env.push("RUNNER_OS=macOS".to_string());
            env.push("RUNNER_ARCH=X64".to_string());
            env.push("TMPDIR=/tmp".to_string());
            env.push("HOME=/root".to_string());
            env.push("GITHUB_WORKSPACE=/github/workspace".to_string());
            env.push("PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string());
        }

        // Create appropriate container options based on platform
        let options = Some(CreateContainerOptions {
            name: format!("wrkflw-{}", uuid::Uuid::new_v4()),
            platform: if is_windows_image {
                Some("windows".to_string())
            } else {
                None
            },
        });

        // Configure host configuration based on platform
        let host_config = if is_windows_image {
            HostConfig {
                binds: Some(binds),
                isolation: Some(bollard::models::HostConfigIsolationEnum::PROCESS),
                ..Default::default()
            }
        } else {
            HostConfig {
                binds: Some(binds),
                ..Default::default()
            }
        };

        // Create container config with platform-specific settings
        let mut config = Config {
            image: Some(image.to_string()),
            // Empty cmd means "use the image's built-in ENTRYPOINT/CMD"
            cmd: if has_cmd { Some(cmd_vec) } else { None },
            env: Some(env),
            working_dir: Some(working_dir.to_string_lossy().to_string()),
            host_config: Some(host_config),
            // Windows containers need specific configuration
            user: if is_windows_image {
                Some("ContainerAdministrator".to_string())
            } else {
                None // Don't specify user for macOS emulation - use default root user
            },
            // Map appropriate entrypoint for different platforms.
            // Priority: explicit entrypoint from action.yml > macOS bash wrapper > image default.
            // Only apply the macOS bash wrapper when there is an explicit command to wrap;
            // an empty cmd means "use the image's native ENTRYPOINT/CMD"
            // and overriding it with bash would hang or discard the real entrypoint.
            entrypoint: if let Some(ep) = entrypoint.filter(|s| !s.is_empty()) {
                Some(vec![ep.to_string()])
            } else if is_macos_emu && has_cmd {
                // For macOS, ensure we use bash
                Some(vec!["bash".to_string(), "-l".to_string(), "-c".to_string()])
            } else {
                None
            },
            ..Default::default()
        };

        // Run platform-specific container setup
        if is_macos_emu {
            // Add special labels for macOS
            let mut labels = HashMap::new();
            labels.insert("wrkflw.platform".to_string(), "macos".to_string());
            config.labels = Some(labels);
        }

        // Create container with a shorter timeout
        let create_result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.docker.create_container(options, config),
        )
        .await;

        let container = match create_result {
            Ok(Ok(container)) => container,
            Ok(Err(e)) => return Err(ContainerError::ContainerStart(e.to_string())),
            Err(_) => {
                return Err(ContainerError::ContainerStart(
                    "Container creation timed out".to_string(),
                ))
            }
        };

        // Track the container before starting it to ensure cleanup even if starting fails
        track_container(&container.id);

        // Start container with a timeout
        let start_result = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.docker.start_container::<String>(&container.id, None),
        )
        .await;

        match start_result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                // Clean up the container if start fails
                let _ = self.docker.remove_container(&container.id, None).await;
                untrack_container(&container.id);
                return Err(ContainerError::ContainerExecution(e.to_string()));
            }
            Err(_) => {
                // Clean up the container if starting times out
                let _ = self.docker.remove_container(&container.id, None).await;
                untrack_container(&container.id);
                return Err(ContainerError::ContainerExecution(
                    "Container start timed out".to_string(),
                ));
            }
        }

        // Wait for container to finish with a timeout (300 seconds)
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            self.docker
                .wait_container::<String>(&container.id, None)
                .collect::<Vec<_>>(),
        )
        .await;

        let exit_code = match wait_result {
            Ok(results) => match results.first() {
                Some(Ok(exit)) => exit.status_code as i32,
                _ => -1,
            },
            Err(_) => {
                wrkflw_logging::warning("Container wait operation timed out, treating as failure");
                -1
            }
        };

        // Get logs with a timeout
        let log_options = Some(bollard::container::LogsOptions::<String> {
            stdout: true,
            stderr: true,
            ..Default::default()
        });
        let logs_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.docker
                .logs(&container.id, log_options)
                .collect::<Vec<_>>(),
        )
        .await;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Ok(logs) = logs_result {
            for log in logs.into_iter().flatten() {
                match log {
                    bollard::container::LogOutput::StdOut { message } => {
                        stdout.push_str(&String::from_utf8_lossy(&message));
                    }
                    bollard::container::LogOutput::StdErr { message } => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        } else {
            wrkflw_logging::warning("Retrieving container logs timed out");
        }

        // Clean up container with a timeout, but preserve on failure if configured
        if exit_code == 0 || !self.preserve_containers_on_failure {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.docker.remove_container(&container.id, None),
            )
            .await;
            untrack_container(&container.id);
        } else {
            // Container failed and we want to preserve it for debugging
            wrkflw_logging::info(&format!(
                "Preserving container {} for debugging (exit code: {}). Use 'docker exec -it {} bash' to inspect.",
                container.id, exit_code, container.id
            ));
            // Still untrack it from the automatic cleanup system to prevent it from being cleaned up later
            untrack_container(&container.id);
        }

        // Log detailed information about the command execution for debugging
        if exit_code != 0 {
            wrkflw_logging::info(&format!(
                "Docker command failed with exit code: {}",
                exit_code
            ));
            wrkflw_logging::debug(&format!("Failed command: {:?}", cmd));
            wrkflw_logging::debug(&format!("Working directory: {}", working_dir.display()));
            wrkflw_logging::debug(&format!("STDERR: {}", stderr));
        }

        Ok(ContainerOutput {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn pull_image_inner(&self, image: &str) -> Result<(), ContainerError> {
        let options = bollard::image::CreateImageOptions {
            from_image: image,
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            if let Err(e) = result {
                return Err(ContainerError::ImagePull(e.to_string()));
            }
        }

        Ok(())
    }

    /// Build a Docker image from a Dockerfile.
    ///
    /// `context_dir` is the Docker build context directory. Files in this
    /// directory are sent to the Docker daemon so that COPY instructions work.
    /// The Dockerfile path is made relative to this context for the build API.
    ///
    /// If a `.dockerignore` file exists in the context directory, its patterns
    /// are honoured to exclude files from the build context — reducing the
    /// amount of data sent to the daemon for large action repositories.
    ///
    /// **Note:** `follow_symlinks(false)` protects against symlink-based
    /// exfiltration but does not prevent hard-links to files outside the
    /// context.  This is acceptable because the context is always a freshly
    /// cloned repo or tempdir that we control.
    async fn build_image_inner(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError> {
        if !dockerfile.exists() {
            return Err(ContainerError::ImageBuild(format!(
                "Cannot open Dockerfile at {}",
                dockerfile.display()
            )));
        }

        // Determine the Dockerfile path relative to the context directory.
        let dockerfile_name = dockerfile
            .strip_prefix(context_dir)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|_| {
                ContainerError::ImageBuild(format!(
                    "Dockerfile {} is not within context directory {}",
                    dockerfile.display(),
                    context_dir.display()
                ))
            })?;

        // Maximum build context size (500 MB). Prevents OOM when an action
        // repo is unexpectedly large and has no .dockerignore.
        const MAX_CONTEXT_BYTES: u64 = 500 * 1024 * 1024;

        // TODO: For large build contexts (approaching MAX_CONTEXT_BYTES), consider
        // streaming the tar to a temporary file instead of holding it all in memory.
        let tar_buffer = {
            let mut tar_builder = tar::Builder::new(Vec::new());

            // Do not follow symlinks — untrusted action repos could use symlinks
            // to leak host filesystem contents into the Docker build context.
            tar_builder.follow_symlinks(false);

            // Track cumulative size so we can enforce MAX_CONTEXT_BYTES.
            let mut total_bytes: u64 = 0;

            // Build a context walker. When a .dockerignore exists, its patterns
            // are used to exclude files (same glob semantics as Docker).
            use ignore::WalkBuilder;

            let dockerignore_path = context_dir.join(".dockerignore");
            let mut walker_builder = WalkBuilder::new(context_dir);
            // Disable default .gitignore / .ignore handling — we only want
            // .dockerignore semantics when the file is present.
            walker_builder.standard_filters(false).follow_links(false);
            if dockerignore_path.exists() {
                walker_builder.add_custom_ignore_filename(".dockerignore");
            }

            for entry in walker_builder.build() {
                let entry = entry.map_err(|e| {
                    ContainerError::ImageBuild(format!("Failed to walk build context: {}", e))
                })?;
                let path = entry.path();
                let rel = match path.strip_prefix(context_dir) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                // Skip the root directory itself — tar implicitly contains it.
                if rel.as_os_str().is_empty() {
                    continue;
                }
                if path.is_dir() {
                    tar_builder.append_dir(rel, path).map_err(|e| {
                        ContainerError::ImageBuild(format!(
                            "Failed to add directory to build context tar: {}",
                            e
                        ))
                    })?;
                } else {
                    if let Ok(meta) = path.metadata() {
                        total_bytes += meta.len();
                        if total_bytes > MAX_CONTEXT_BYTES {
                            return Err(ContainerError::ImageBuild(format!(
                                "Docker build context exceeds {} MB limit. \
                                 Add a .dockerignore file to exclude unnecessary files.",
                                MAX_CONTEXT_BYTES / (1024 * 1024)
                            )));
                        }
                    }
                    tar_builder.append_path_with_name(path, rel).map_err(|e| {
                        ContainerError::ImageBuild(format!(
                            "Failed to add file to build context tar: {}",
                            e
                        ))
                    })?;
                }
            }

            tar_builder
                .into_inner()
                .map_err(|e| ContainerError::ImageBuild(e.to_string()))?
        };

        let options = bollard::image::BuildImageOptions {
            dockerfile: dockerfile_name.as_str(),
            t: tag,
            q: false,
            nocache: false,
            rm: true,
            ..Default::default()
        };

        let mut stream = self
            .docker
            .build_image(options, None, Some(tar_buffer.into()));

        while let Some(result) = stream.next().await {
            match result {
                Ok(_) => {
                    // For verbose output, we could log the build progress here
                }
                Err(e) => {
                    return Err(ContainerError::ImageBuild(e.to_string()));
                }
            }
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

#[cfg(test)]
pub fn get_tracked_networks() -> Vec<String> {
    if let Ok(networks) = CREATED_NETWORKS.lock() {
        networks.clone()
    } else {
        vec![]
    }
}
