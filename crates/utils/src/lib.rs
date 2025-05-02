// utils crate

use std::path::Path;

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
pub mod fd {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use nix::unistd::{close, dup, dup2};
    use std::io::{self, Result};
    use std::os::unix::io::RawFd;
    use std::path::Path;

    /// Standard file descriptors
    const STDERR_FILENO: RawFd = 2;

    /// Represents a redirected stderr that can be restored
    pub struct RedirectedStderr {
        original_fd: Option<RawFd>,
        null_fd: Option<RawFd>,
    }

    impl RedirectedStderr {
        /// Creates a new RedirectedStderr that redirects stderr to /dev/null
        pub fn to_null() -> Result<Self> {
            // Duplicate the current stderr fd
            let stderr_backup = match dup(STDERR_FILENO) {
                Ok(fd) => fd,
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            };

            // Open /dev/null
            let null_fd = match open(Path::new("/dev/null"), OFlag::O_WRONLY, Mode::empty()) {
                Ok(fd) => fd,
                Err(e) => {
                    let _ = close(stderr_backup); // Clean up on error
                    return Err(io::Error::new(io::ErrorKind::Other, e));
                }
            };

            // Redirect stderr to /dev/null
            if let Err(e) = dup2(null_fd, STDERR_FILENO) {
                let _ = close(stderr_backup); // Clean up on error
                let _ = close(null_fd);
                return Err(io::Error::new(io::ErrorKind::Other, e));
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

    /// Run a function with stderr redirected to /dev/null, then restore stderr
    pub fn with_stderr_to_null<F, T>(f: F) -> Result<T>
    where
        F: FnOnce() -> T,
    {
        let _redirected = RedirectedStderr::to_null()?;
        Ok(f())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fd_redirection() {
        // This test will write to stderr, which should be redirected
        let result = fd::with_stderr_to_null(|| {
            // This would normally appear in stderr
            eprintln!("This should be redirected to /dev/null");
            // Return a test value to verify the function passes through the result
            42
        });

        // The function should succeed and return our test value
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
