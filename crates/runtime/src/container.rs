use async_trait::async_trait;
use std::path::Path;

/// Prefix for all locally-built images. Used to skip registry pulls.
pub const LOCAL_IMAGE_PREFIX: &str = "wrkflw-";

/// Prefix for combined runtime images built by `resolve_runner_image`.
pub const COMBINED_IMAGE_PREFIX: &str = "wrkflw-combined:";

#[async_trait]
pub trait ContainerRuntime {
    /// Run a command inside a container.
    ///
    /// If `cmd` is empty (`&[]`), the container runs with the image's built-in
    /// ENTRYPOINT/CMD. This is used for Docker-type GitHub Actions whose
    /// entrypoint is baked into the image.
    ///
    /// `entrypoint` optionally overrides the image's ENTRYPOINT (used when an
    /// action.yml declares `runs.entrypoint`).
    async fn run_container(
        &self,
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError>;

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError>;

    async fn build_image(
        &self,
        dockerfile: &Path,
        tag: &str,
        context_dir: &Path,
    ) -> Result<(), ContainerError>;

    async fn prepare_language_environment(
        &self,
        language: &str,
        version: Option<&str>,
        additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError>;

    /// Check whether a Docker/OCI image exists locally.
    async fn image_exists(&self, tag: &str) -> Result<bool, ContainerError>;
}

#[derive(Debug)]
#[must_use]
pub struct ContainerOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

use std::fmt;

#[derive(Debug)]
pub enum ContainerError {
    ImagePull(String),
    ImageBuild(String),
    ContainerStart(String),
    ContainerExecution(String),
    NetworkCreation(String),
    NetworkOperation(String),
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContainerError::ImagePull(msg) => write!(f, "Failed to pull image: {}", msg),
            ContainerError::ImageBuild(msg) => write!(f, "Failed to build image: {}", msg),
            ContainerError::ContainerStart(msg) => {
                write!(f, "Failed to start container: {}", msg)
            }
            ContainerError::ContainerExecution(msg) => {
                write!(f, "Container execution failed: {}", msg)
            }
            ContainerError::NetworkCreation(msg) => {
                write!(f, "Failed to create Docker network: {}", msg)
            }
            ContainerError::NetworkOperation(msg) => {
                write!(f, "Network operation failed: {}", msg)
            }
        }
    }
}
