// utils crate

use std::path::{Path, PathBuf};

/// Default workflow directory for GitHub Actions.
pub const GITHUB_WORKFLOWS_DIR: &str = ".github/workflows";

/// Default workflow directory for Gitea Actions.
pub const GITEA_WORKFLOWS_DIR: &str = ".gitea/workflows";

/// Returns the default workflow directories that exist under `root`, in
/// Gitea's lookup order: `.gitea/workflows` first, then `.github/workflows`.
///
/// Gitea itself only runs the first directory that exists, but a repository
/// mirrored to both forges legitimately has both, so commands that can cover
/// several directories should use all of them.
pub fn default_workflow_dirs_in(root: &Path) -> Vec<PathBuf> {
    [GITEA_WORKFLOWS_DIR, GITHUB_WORKFLOWS_DIR]
        .iter()
        .map(|dir| root.join(dir))
        .filter(|dir| dir.is_dir())
        .collect()
}

/// [`default_workflow_dirs_in`] relative to the current directory. Paths stay
/// relative so they read as `.gitea/workflows`, not `./.gitea/workflows`.
pub fn default_workflow_dirs() -> Vec<PathBuf> {
    default_workflow_dirs_in(Path::new(""))
}

/// The single default workflow directory, for commands that take only one.
/// Falls back to `.github/workflows` when neither directory exists, so the
/// caller's "not found" error names the conventional location.
pub fn default_workflow_dir() -> PathBuf {
    default_workflow_dirs()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from(GITHUB_WORKFLOWS_DIR))
}

pub fn is_workflow_file(path: &Path) -> bool {
    // First, check for GitLab CI files by name
    if let Some(file_name) = path.file_name() {
        let file_name_str = file_name.to_string_lossy().to_lowercase();
        if file_name_str == ".gitlab-ci.yml" || file_name_str.ends_with("gitlab-ci.yml") {
            return true;
        }
    }

    // Then check for GitHub Actions workflows
    if let Some(ext) = path.extension() {
        if ext == "yml" || ext == "yaml" {
            // Check if the file is in a .github/workflows directory
            if let Some(parent) = path.parent() {
                return parent.ends_with(".github/workflows") || parent.ends_with("workflows");
            } else {
                // Check if filename contains workflow indicators
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_lowercase())
                    .unwrap_or_default();

                return filename.contains("workflow")
                    || filename.contains("action")
                    || filename.contains("ci")
                    || filename.contains("cd");
            }
        }
    }
    false
}

/// Module for safely handling file descriptor redirection
///
/// On Unix systems (Linux, macOS), this module provides true file descriptor
/// redirection by duplicating stderr and redirecting it to /dev/null.
///
/// On Windows systems, the redirection functionality is limited due to platform
/// differences in file descriptor handling. The functions will execute without
/// error but stderr may not be fully suppressed.
pub mod fd {
    use std::io::Result;

    /// Represents a redirected stderr that can be restored
    pub struct RedirectedStderr {
        #[cfg(unix)]
        original_fd: Option<std::os::unix::io::RawFd>,
        #[cfg(unix)]
        null_fd: Option<std::os::unix::io::RawFd>,
        #[cfg(windows)]
        _phantom: std::marker::PhantomData<()>,
    }

    #[cfg(unix)]
    mod unix_impl {
        use super::*;
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;
        use nix::unistd::{close, dup, dup2};
        use std::io;
        use std::os::unix::io::RawFd;
        use std::path::Path;

        /// Standard file descriptors
        const STDERR_FILENO: RawFd = 2;

        impl RedirectedStderr {
            /// Creates a new RedirectedStderr that redirects stderr to /dev/null
            pub fn to_null() -> Result<Self> {
                // Duplicate the current stderr fd
                let stderr_backup = match dup(STDERR_FILENO) {
                    Ok(fd) => fd,
                    Err(e) => return Err(io::Error::other(e)),
                };

                // Open /dev/null
                let null_fd = match open(Path::new("/dev/null"), OFlag::O_WRONLY, Mode::empty()) {
                    Ok(fd) => fd,
                    Err(e) => {
                        let _ = close(stderr_backup); // Clean up on error
                        return Err(io::Error::other(e));
                    }
                };

                // Redirect stderr to /dev/null
                if let Err(e) = dup2(null_fd, STDERR_FILENO) {
                    let _ = close(stderr_backup); // Clean up on error
                    let _ = close(null_fd);
                    return Err(io::Error::other(e));
                }

                Ok(RedirectedStderr {
                    original_fd: Some(stderr_backup),
                    null_fd: Some(null_fd),
                })
            }
        }

        impl Drop for RedirectedStderr {
            /// Automatically restores stderr when the RedirectedStderr is dropped
            fn drop(&mut self) {
                if let Some(orig_fd) = self.original_fd.take() {
                    // Restore the original stderr
                    let _ = dup2(orig_fd, STDERR_FILENO);
                    let _ = close(orig_fd);
                }

                // Close the null fd
                if let Some(null_fd) = self.null_fd.take() {
                    let _ = close(null_fd);
                }
            }
        }
    }

    #[cfg(windows)]
    mod windows_impl {
        use super::*;

        impl RedirectedStderr {
            /// Creates a new RedirectedStderr that redirects stderr to NUL on Windows
            pub fn to_null() -> Result<Self> {
                // On Windows, we can't easily redirect stderr at the file descriptor level
                // like we can on Unix systems. This is a simplified implementation that
                // doesn't actually redirect but provides the same interface.
                // The actual stderr suppression will need to be handled differently on Windows.
                Ok(RedirectedStderr {
                    _phantom: std::marker::PhantomData,
                })
            }
        }

        impl Drop for RedirectedStderr {
            /// No-op drop implementation for Windows
            fn drop(&mut self) {
                // Nothing to restore on Windows in this simplified implementation
            }
        }
    }

    /// Run a function with stderr redirected to /dev/null (Unix) or suppressed (Windows), then restore stderr
    ///
    /// # Platform Support
    /// - **Unix (Linux, macOS)**: Fully supported - stderr is redirected to /dev/null
    /// - **Windows**: Limited support - function executes but stderr may be visible
    ///
    /// # Example
    /// ```
    /// use wrkflw_utils::fd::with_stderr_to_null;
    ///
    /// let result = with_stderr_to_null(|| {
    ///     eprintln!("This will be hidden on Unix");
    ///     42
    /// }).unwrap();
    /// assert_eq!(result, 42);
    /// ```
    pub fn with_stderr_to_null<F, T>(f: F) -> Result<T>
    where
        F: FnOnce() -> T,
    {
        #[cfg(unix)]
        {
            let _redirected = RedirectedStderr::to_null()?;
            Ok(f())
        }
        #[cfg(windows)]
        {
            // On Windows, we can't easily redirect stderr at the FD level,
            // so we just run the function without redirection.
            // This means stderr won't be suppressed on Windows, but the function will work.
            Ok(f())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_workflow_dirs_prefer_gitea_and_skip_missing() {
        let tmp = std::env::temp_dir().join(format!("wrkflw-dirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(super::default_workflow_dirs_in(&tmp).is_empty());

        let github = tmp.join(super::GITHUB_WORKFLOWS_DIR);
        std::fs::create_dir_all(&github).unwrap();
        assert_eq!(super::default_workflow_dirs_in(&tmp), vec![github.clone()]);

        let gitea = tmp.join(super::GITEA_WORKFLOWS_DIR);
        std::fs::create_dir_all(&gitea).unwrap();
        assert_eq!(super::default_workflow_dirs_in(&tmp), vec![gitea, github]);

        std::fs::remove_dir_all(&tmp).unwrap();
    }

    use super::*;

    #[test]
    fn test_fd_redirection() {
        // This test will write to stderr, which should be redirected on Unix
        // On Windows, it will just run normally without redirection
        let result = fd::with_stderr_to_null(|| {
            // This would normally appear in stderr (suppressed on Unix, visible on Windows)
            eprintln!("This should be redirected to /dev/null on Unix");
            // Return a test value to verify the function passes through the result
            42
        });

        // The function should succeed and return our test value on both platforms
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
