use async_trait::async_trait;
use std::path::{Path, PathBuf};

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

/// Rebase a container-visible working directory onto its host-side volume
/// source.
///
/// Given a `container_dir` like `/github/workspace/sub` and a `volumes` list
/// that maps `(host, container)` pairs (e.g. `(/tmp/job-xxxx, /github/workspace)`),
/// return the corresponding host path (`/tmp/job-xxxx/sub`) by locating the
/// longest `container` path that is a component-boundary prefix of
/// `container_dir` and grafting the remainder onto its `host` counterpart.
///
/// Returns `None` if no volume covers `container_dir`.
///
/// This is the mount-semantics bridge used by non-container runtimes
/// (emulation, secure_emulation) so that a `run:` step and an
/// artifact/cache handler observe the same host workspace. It is the fix
/// for #88.
pub(crate) fn resolve_host_working_dir(
    container_dir: &Path,
    volumes: &[(&Path, &Path)],
) -> Option<PathBuf> {
    let mut best: Option<(usize, PathBuf)> = None;
    for (host, container) in volumes {
        if let Ok(suffix) = container_dir.strip_prefix(container) {
            // `Path::strip_prefix` respects component boundaries, so
            // `/github/workspace-foo` is NOT matched by `/github/workspace`.
            let depth = container.components().count();
            let candidate = host.join(suffix);
            match &best {
                Some((best_depth, _)) if *best_depth >= depth => {}
                _ => best = Some((depth, candidate)),
            }
        }
    }
    best.map(|(_, path)| path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_host_working_dir_exact_match() {
        let host = Path::new("/host/tmp/job");
        let container = Path::new("/github/workspace");
        let volumes = [(host, container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/github/workspace"), &volumes),
            Some(PathBuf::from("/host/tmp/job"))
        );
    }

    #[test]
    fn resolve_host_working_dir_sub_path() {
        let host = Path::new("/host/tmp/job");
        let container = Path::new("/github/workspace");
        let volumes = [(host, container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/github/workspace/src/lib"), &volumes),
            Some(PathBuf::from("/host/tmp/job/src/lib"))
        );
    }

    #[test]
    fn resolve_host_working_dir_longest_prefix_wins() {
        let outer_host = Path::new("/host/outer");
        let outer_container = Path::new("/a");
        let inner_host = Path::new("/host/inner");
        let inner_container = Path::new("/a/b");
        // Order shouldn't matter — longest prefix always wins.
        let volumes = [(outer_host, outer_container), (inner_host, inner_container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/a/b/c"), &volumes),
            Some(PathBuf::from("/host/inner/c"))
        );
        let reversed = [(inner_host, inner_container), (outer_host, outer_container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/a/b/c"), &reversed),
            Some(PathBuf::from("/host/inner/c"))
        );
    }

    #[test]
    fn resolve_host_working_dir_no_match() {
        let host = Path::new("/host/tmp/job");
        let container = Path::new("/different");
        let volumes = [(host, container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/github/workspace"), &volumes),
            None
        );
    }

    #[test]
    fn resolve_host_working_dir_empty_volumes() {
        assert_eq!(
            resolve_host_working_dir(Path::new("/github/workspace"), &[]),
            None
        );
    }

    /// Critical: a string-prefix match would incorrectly rebase
    /// `/github/workspace-foo` onto the mount for `/github/workspace`.
    /// `Path::strip_prefix` respects component boundaries, so this must
    /// return `None`.
    #[test]
    fn resolve_host_working_dir_component_boundary_is_respected() {
        let host = Path::new("/host/tmp/job");
        let container = Path::new("/github/workspace");
        let volumes = [(host, container)];
        assert_eq!(
            resolve_host_working_dir(Path::new("/github/workspace-foo"), &volumes),
            None
        );
    }
}
