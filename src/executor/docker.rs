use crate::logging;
use crate::runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};
use async_trait::async_trait;
use bollard::{
    container::{Config, CreateContainerOptions},
    models::HostConfig,
    network::CreateNetworkOptions,
    Docker,
};
use futures_util::StreamExt;
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::Mutex;

static RUNNING_CONTAINERS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static CREATED_NETWORKS: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub struct DockerRuntime {
    docker: Docker,
}

impl DockerRuntime {
    pub fn new() -> Result<Self, ContainerError> {
        let docker = Docker::connect_with_local_defaults().map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to connect to Docker: {}", e))
        })?;

        Ok(DockerRuntime { docker })
    }
}

pub fn is_available() -> bool {
    // Use a very short timeout for the entire availability check
    let overall_timeout = std::time::Duration::from_secs(3);

    // Spawn a thread with the timeout to prevent blocking the main thread
    let handle = std::thread::spawn(move || {
        // Use safe FD redirection utility to suppress Docker error messages
        match crate::utils::fd::with_stderr_to_null(|| {
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
                        logging::debug("Docker CLI is not available");
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
                    logging::error(&format!(
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
                                    logging::debug(&format!("Docker daemon ping failed: {}", e));
                                    false
                                }
                                Err(_) => {
                                    logging::debug("Docker daemon ping timed out after 1 second");
                                    false
                                }
                            }
                        }
                        Err(e) => {
                            logging::debug(&format!("Docker daemon connection failed: {}", e));
                            false
                        }
                    }
                })
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        logging::debug("Docker availability check timed out");
                        false
                    }
                }
            })
        }) {
            Ok(result) => result,
            Err(_) => {
                logging::debug("Failed to redirect stderr when checking Docker availability");
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
                    logging::warning("Docker availability check thread panicked");
                    false
                }
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    logging::warning("Docker availability check timed out, assuming Docker is not available");
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
            logging::error(&format!("Error during container cleanup: {}", e));
        }

        if let Err(e) = network_result {
            logging::error(&format!("Error during network cleanup: {}", e));
        }
    })
    .await
    {
        Ok(_) => logging::debug("Docker cleanup completed within timeout"),
        Err(_) => {
            logging::warning("Docker cleanup timed out, some resources may not have been removed")
        }
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
                    logging::error("Could not acquire container lock for cleanup");
                    vec![]
                }
            }
        })
        .await
        {
            Ok(containers) => containers,
            Err(_) => {
                logging::error("Timeout while trying to get containers for cleanup");
                vec![]
            }
        };

    if containers_to_cleanup.is_empty() {
        return Ok(());
    }

    logging::info(&format!(
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
            Ok(Ok(_)) => logging::debug(&format!("Stopped container: {}", container_id)),
            Ok(Err(e)) => {
                logging::warning(&format!("Error stopping container {}: {}", container_id, e))
            }
            Err(_) => logging::warning(&format!("Timeout stopping container: {}", container_id)),
        }

        // Then try to remove it
        match tokio::time::timeout(
            std::time::Duration::from_millis(1000),
            docker.remove_container(&container_id, None),
        )
        .await
        {
            Ok(Ok(_)) => logging::debug(&format!("Removed container: {}", container_id)),
            Ok(Err(e)) => {
                logging::warning(&format!("Error removing container {}: {}", container_id, e))
            }
            Err(_) => logging::warning(&format!("Timeout removing container: {}", container_id)),
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
                    logging::error("Could not acquire network lock for cleanup");
                    vec![]
                }
            }
        })
        .await
        {
            Ok(networks) => networks,
            Err(_) => {
                logging::error("Timeout while trying to get networks for cleanup");
                vec![]
            }
        };

    if networks_to_cleanup.is_empty() {
        return Ok(());
    }

    logging::info(&format!(
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
            Ok(Ok(_)) => logging::info(&format!("Successfully removed network: {}", network_id)),
            Ok(Err(e)) => logging::error(&format!("Error removing network {}: {}", network_id, e)),
            Err(_) => logging::warning(&format!("Timeout removing network: {}", network_id)),
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
    logging::info(&format!("Created Docker network: {}", network_id));

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
    ) -> Result<ContainerOutput, ContainerError> {
        // Print detailed debugging info
        logging::info(&format!("Docker: Running container with image: {}", image));

        // Add a global timeout for all Docker operations to prevent freezing
        let timeout_duration = std::time::Duration::from_secs(60); // 1 minute timeout

        // Run the entire container operation with a timeout
        match tokio::time::timeout(
            timeout_duration,
            self.run_container_inner(image, cmd, env_vars, working_dir, volumes),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                logging::error("Docker operation timed out after 60 seconds");
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
                logging::warning(&format!(
                    "Pull of image {} timed out, continuing with existing image",
                    image
                ));
                // Return success to allow continuing with existing image
                Ok(())
            }
        }
    }

    async fn build_image(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError> {
        // Add a timeout for build operations
        let timeout_duration = std::time::Duration::from_secs(120); // 2 minutes timeout for builds

        match tokio::time::timeout(timeout_duration, self.build_image_inner(dockerfile, tag)).await
        {
            Ok(result) => result,
            Err(_) => {
                logging::error(&format!(
                    "Building image {} timed out after 120 seconds",
                    tag
                ));
                Err(ContainerError::ImageBuild(
                    "Operation timed out".to_string(),
                ))
            }
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
    ) -> Result<ContainerOutput, ContainerError> {
        // Check if command contains background processes
        let has_background = cmd.iter().any(|c| c.contains(" &"));

        // Check if any command uses GITHUB_ variables and needs special handling
        let uses_github_vars = cmd.iter().any(|c| c.contains("GITHUB_"));

        // If there's a command using GitHub variables, we need to wrap it properly
        let cmd_vec: Vec<String> = if uses_github_vars {
            let mut shell_cmd = Vec::new();
            shell_cmd.push("sh".to_string());
            shell_cmd.push("-c".to_string());

            // Join the original command and fix GitHub variables reference
            let command_with_fixes =
                if cmd.len() > 2 && (cmd[0] == "sh" || cmd[0] == "bash") && cmd[1] == "-c" {
                    // For shell commands, we need to modify the command string to handle GitHub variables
                    let fixed_cmd = cmd[2]
                        .replace(">>$GITHUB_OUTPUT", ">>\"$GITHUB_OUTPUT\"")
                        .replace(">>$GITHUB_ENV", ">>\"$GITHUB_ENV\"")
                        .replace(">>$GITHUB_PATH", ">>\"$GITHUB_PATH\"")
                        .replace(">>$GITHUB_STEP_SUMMARY", ">>\"$GITHUB_STEP_SUMMARY\"");

                    format!("{} ; wait", fixed_cmd)
                } else {
                    // Otherwise join all parts and add wait
                    let cmd_str: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
                    format!("{} ; wait", cmd_str.join(" "))
                };

            shell_cmd.push(command_with_fixes);
            shell_cmd
        } else if has_background {
            // If the command contains a background process, wrap it in a shell script
            // that properly manages the background process and exits when the foreground completes
            let mut shell_cmd = Vec::new();
            shell_cmd.push("sh".to_string());
            shell_cmd.push("-c".to_string());

            // Join the original command and add a wait for any child processes
            let command_with_wait =
                if cmd.len() > 2 && (cmd[0] == "sh" || cmd[0] == "bash") && cmd[1] == "-c" {
                    // For shell commands, we just need to modify the command string
                    format!("{} ; wait", cmd[2])
                } else {
                    // Otherwise join all parts and add wait
                    let cmd_str: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
                    format!("{} ; wait", cmd_str.join(" "))
                };

            shell_cmd.push(command_with_wait);
            shell_cmd
        } else {
            // No background processes, use original command
            cmd.iter().map(|s| s.to_string()).collect()
        };

        // Always try to pull the image first - with a timeout
        match self.pull_image(image).await {
            Ok(_) => logging::info(&format!("🐳 Successfully pulled image: {}", image)),
            Err(e) => logging::error(&format!("🐳 Warning: Failed to pull image: {}. Continuing with existing image if available.", e)),
        }
        // Map env vars to format Docker expects
        let env: Vec<String> = env_vars
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Setup volume bindings
        let mut binds = Vec::new();
        for (host, container) in volumes {
            binds.push(format!(
                "{}:{}",
                host.to_string_lossy(),
                container.to_string_lossy()
            ));
        }

        // Create container
        let options = Some(CreateContainerOptions {
            name: format!("wrkflw-{}", uuid::Uuid::new_v4()),
            platform: None,
        });

        let host_config = HostConfig {
            binds: Some(binds),
            ..Default::default()
        };

        let config = Config {
            image: Some(image.to_string()),
            cmd: Some(cmd_vec),
            env: Some(env),
            working_dir: Some(working_dir.to_string_lossy().to_string()),
            host_config: Some(host_config),
            ..Default::default()
        };

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

        // Wait for container to finish with a timeout (30 seconds)
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
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
                logging::warning("Container wait operation timed out, treating as failure");
                -1
            }
        };

        // Get logs with a timeout
        let logs_result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.docker
                .logs::<String>(&container.id, None)
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
            logging::warning("Retrieving container logs timed out");
        }

        // Clean up container with a timeout
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.docker.remove_container(&container.id, None),
        )
        .await;
        untrack_container(&container.id);

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

    async fn build_image_inner(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError> {
        let _context_dir = dockerfile.parent().unwrap_or(Path::new("."));

        let tar_buffer = {
            let mut tar_builder = tar::Builder::new(Vec::new());

            // Add Dockerfile to tar
            if let Ok(file) = std::fs::File::open(dockerfile) {
                let mut header = tar::Header::new_gnu();
                let metadata = file.metadata().map_err(|e| {
                    ContainerError::ContainerExecution(format!(
                        "Failed to get file metadata: {}",
                        e
                    ))
                })?;
                let modified_time = metadata
                    .modified()
                    .map_err(|e| {
                        ContainerError::ContainerExecution(format!(
                            "Failed to get file modification time: {}",
                            e
                        ))
                    })?
                    .elapsed()
                    .map_err(|e| {
                        ContainerError::ContainerExecution(format!(
                            "Failed to get elapsed time since modification: {}",
                            e
                        ))
                    })?
                    .as_secs();
                header.set_size(metadata.len());
                header.set_mode(0o644);
                header.set_mtime(modified_time);
                header.set_cksum();

                tar_builder
                    .append_data(&mut header, "Dockerfile", file)
                    .map_err(|e| ContainerError::ImageBuild(e.to_string()))?;
            } else {
                return Err(ContainerError::ImageBuild(format!(
                    "Cannot open Dockerfile at {}",
                    dockerfile.display()
                )));
            }

            tar_builder
                .into_inner()
                .map_err(|e| ContainerError::ImageBuild(e.to_string()))?
        };

        let options = bollard::image::BuildImageOptions {
            dockerfile: "Dockerfile",
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
