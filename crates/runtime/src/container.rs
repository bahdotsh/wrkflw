use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use wrkflw_logging;

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
                // Equal-depth ties can only occur when two volume entries
                // share the same `container` prefix (two distinct container
                // paths that are both strict component-boundary prefixes of
                // the same `container_dir` must have different component
                // counts, because one has to contain the other). In that
                // duplicate-entry case, first-seen wins — the `>=` is the
                // intentional conflict resolution, not a typo for `>`.
                Some((best_depth, _)) if *best_depth >= depth => {}
                _ => best = Some((depth, candidate)),
            }
        }
    }
    best.map(|(_, path)| path)
}

/// Resolve the host working directory for a non-container runtime call, or
/// return a `ContainerError` describing why it couldn't.
///
/// This is the shared wiring used by `EmulationRuntime::run_container` and
/// `SecureEmulationRuntime::run_container`. It enforces one invariant: when
/// `volumes` covers `working_dir`, the volume mapping **always wins** over
/// any accidentally-existing host path — so a dev-environment quirk like
/// `/github/workspace` happening to exist on the host cannot silently skip
/// the rebase and reintroduce #88.
///
/// - If a volume covers `working_dir`, rebase it, `create_dir_all` the host
///   side if it doesn't exist yet (matching docker's bind-mount behavior of
///   creating the mount target on first access), and return the host path.
/// - If no volume covers `working_dir` but `working_dir` itself exists on
///   the host, accept it as a caller-provided host path.
/// - Otherwise, return a loud, descriptive error. No silent fallback.
///
/// `runtime_label` is used as a prefix in log and error messages so the
/// reader can tell which runtime produced a given line.
pub(crate) fn rebase_working_dir_or_error(
    working_dir: &Path,
    volumes: &[(&Path, &Path)],
    runtime_label: &str,
) -> Result<PathBuf, ContainerError> {
    match resolve_host_working_dir(working_dir, volumes) {
        Some(host) => {
            if !host.exists() {
                fs::create_dir_all(&host).map_err(|e| {
                    ContainerError::ContainerExecution(format!(
                        "{}: failed to create host working directory '{}': {}",
                        runtime_label,
                        host.display(),
                        e
                    ))
                })?;
            }
            wrkflw_logging::info(&format!(
                "{}: rebased container path '{}' to host path '{}' via volume mount",
                runtime_label,
                working_dir.display(),
                host.display()
            ));
            Ok(host)
        }
        None if working_dir.exists() => Ok(working_dir.to_path_buf()),
        None => Err(ContainerError::ContainerExecution(format!(
            "{}: container working dir '{}' is not covered by any volume mount; \
             caller must pass volumes",
            runtime_label,
            working_dir.display()
        ))),
    }
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

    /// Core invariant of `rebase_working_dir_or_error`: when a volume
    /// covers `working_dir`, the rebase wins even if `working_dir` also
    /// happens to exist on the host. Without this, a dev environment with
    /// a real `/github/workspace` directory would silently skip the rebase
    /// and reintroduce the #88 class of bug.
    #[test]
    fn rebase_prefers_volume_mapping_over_accidentally_existing_host_path() {
        // `container_dir` points at a real host tempdir (so `.exists()`
        // returns true), but we also supply a volume mapping that claims
        // that same path for a different host location. The volume must win.
        let existing = tempfile::tempdir().unwrap();
        let mapped = tempfile::tempdir().unwrap();
        let volumes = [(mapped.path(), existing.path())];

        let resolved = rebase_working_dir_or_error(existing.path(), &volumes, "test").unwrap();
        assert_eq!(resolved, mapped.path().to_path_buf());
    }

    #[test]
    fn rebase_accepts_existing_host_path_when_no_volume_covers_it() {
        let host_dir = tempfile::tempdir().unwrap();
        let resolved = rebase_working_dir_or_error(host_dir.path(), &[], "test").unwrap();
        assert_eq!(resolved, host_dir.path().to_path_buf());
    }

    #[test]
    fn rebase_errors_loudly_when_no_volume_and_path_does_not_exist() {
        let err = rebase_working_dir_or_error(
            Path::new("/definitely/does/not/exist/wrkflw-test"),
            &[],
            "test",
        )
        .expect_err("should error");
        let msg = err.to_string();
        assert!(
            msg.contains("not covered by any volume mount"),
            "unexpected error: {}",
            msg
        );
        assert!(msg.contains("test:"), "error should carry runtime label");
    }

    #[test]
    fn rebase_creates_host_side_of_mount_if_missing() {
        // Simulate `working-directory: sub` pointing at a container subdir
        // that hasn't been created yet. The helper must `create_dir_all`
        // the host side so the subsequent `Command` can `current_dir` into
        // it. This matches docker's bind-mount behavior.
        let host_root = tempfile::tempdir().unwrap();
        let container_root = Path::new("/github/workspace");
        let volumes = [(host_root.path(), container_root)];
        let container_sub = Path::new("/github/workspace/sub/nested");

        let resolved = rebase_working_dir_or_error(container_sub, &volumes, "test").unwrap();
        let expected = host_root.path().join("sub/nested");
        assert_eq!(resolved, expected);
        assert!(expected.exists(), "helper should have created the subdir");
    }
}
