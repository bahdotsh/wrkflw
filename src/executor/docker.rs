use crate::logging;
use crate::runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};
use async_trait::async_trait;
use bollard::{
    container::{Config, CreateContainerOptions},
    models::HostConfig,
    Docker,
};
use futures_util::StreamExt;
use std::path::Path;

pub struct DockerRuntime {
    docker: Docker,
}

impl DockerRuntime {
    pub fn new() -> Self {
        let docker = Docker::connect_with_local_defaults().expect("Failed to connect to Docker");

        DockerRuntime { docker }
    }
}

pub fn is_available() -> bool {
    match Docker::connect_with_local_defaults() {
        Ok(docker) => match futures::executor::block_on(async { docker.ping().await }) {
            Ok(_) => true,
            Err(e) => {
                logging::error(&format!("Docker ping failed: {}", e));
                false
            }
        },
        Err(e) => {
            logging::error(&format!("Docker connection failed: {}", e));
            false
        }
    }
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

        // Always try to pull the image first
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

        let cmd_vec: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();

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

        let container = self
            .docker
            .create_container(options, config)
            .await
            .map_err(|e| ContainerError::ContainerStartFailed(e.to_string()))?;

        // Start container
        self.docker
            .start_container::<String>(&container.id, None)
            .await
            .map_err(|e| ContainerError::ContainerExecutionFailed(e.to_string()))?;

        // Wait for container to finish
        let wait_result = self
            .docker
            .wait_container::<String>(&container.id, None)
            .collect::<Vec<_>>()
            .await;

        let exit_code = match wait_result.first() {
            Some(Ok(exit)) => exit.status_code as i32,
            _ => -1,
        };

        // Get logs
        let logs = self
            .docker
            .logs::<String>(&container.id, None)
            .collect::<Vec<_>>()
            .await;

        let mut stdout = String::new();
        let mut stderr = String::new();

        for log_result in logs {
            if let Ok(log) = log_result {
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
        }

        // Clean up container
        let _ = self.docker.remove_container(&container.id, None).await;

        Ok(ContainerOutput {
            stdout,
            stderr,
            exit_code,
        })
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        let options = bollard::image::CreateImageOptions {
            from_image: image,
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            if let Err(e) = result {
                return Err(ContainerError::ImagePullFailed(e.to_string()));
            }
        }

        Ok(())
    }

    async fn build_image(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError> {
        let _context_dir = dockerfile.parent().unwrap_or(Path::new("."));

        let tar_buffer = {
            let mut tar_builder = tar::Builder::new(Vec::new());

            // Add Dockerfile to tar
            if let Ok(file) = std::fs::File::open(dockerfile) {
                let mut header = tar::Header::new_gnu();
                let metadata = file.metadata().unwrap();
                header.set_size(metadata.len());
                header.set_mode(0o644);
                header.set_mtime(metadata.modified().unwrap().elapsed().unwrap().as_secs());
                header.set_cksum();

                tar_builder
                    .append_data(&mut header, "Dockerfile", file)
                    .map_err(|e| ContainerError::ImageBuildFailed(e.to_string()))?;
            } else {
                return Err(ContainerError::ImageBuildFailed(format!(
                    "Cannot open Dockerfile at {}",
                    dockerfile.display()
                )));
            }

            tar_builder
                .into_inner()
                .map_err(|e| ContainerError::ImageBuildFailed(e.to_string()))?
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
                    return Err(ContainerError::ImageBuildFailed(e.to_string()));
                }
            }
        }

        Ok(())
    }
}
