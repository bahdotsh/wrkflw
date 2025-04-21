use std::path::Path;

pub fn is_workflow_file(path: &Path) -> bool {
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
    use std::fs::File;
    use std::io::{self, Result};
    use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
    use std::process::Command;

    /// Represents a redirected stderr that can be restored
    pub struct RedirectedStderr {
        original_fd: Option<File>,
        null_file: Option<File>,
    }

    impl RedirectedStderr {
        /// Creates a new RedirectedStderr that redirects stderr to /dev/null
        pub fn to_null() -> Result<Self> {
            // Open original stderr as a File to keep a copy
            let stderr_fd = 2; // STDERR_FILENO
            let stderr_file = unsafe { 
                // This is still unsafe but in a controlled, limited scope
                File::from_raw_fd(libc::dup(stderr_fd))
            };
            
            // Open /dev/null
            let null_file = File::open("/dev/null")?;
            
            // Duplicate the /dev/null file descriptor to stderr
            let null_fd = null_file.as_raw_fd();
            unsafe {
                // This is still unsafe but in a controlled, limited scope
                libc::dup2(null_fd, stderr_fd);
            }
            
            Ok(RedirectedStderr {
                original_fd: Some(stderr_file),
                null_file: Some(null_file),
            })
        }
    }

    impl Drop for RedirectedStderr {
        /// Automatically restores stderr when the RedirectedStderr is dropped
        fn drop(&mut self) {
            if let Some(orig_file) = self.original_fd.take() {
                let stderr_fd = 2; // STDERR_FILENO
                let orig_fd = orig_file.as_raw_fd();
                
                // Restore the original stderr
                unsafe {
                    libc::dup2(orig_fd, stderr_fd);
                }
                
                // Let the File be closed automatically when it goes out of scope
                // This prevents us from having to call libc::close directly
            }
            
            // Let null_file be closed automatically when it's dropped
            self.null_file.take();
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
    
    /// Run a command with stderr redirected to /dev/null
    pub fn run_command_without_stderr(command: &mut Command) -> Result<std::process::Output> {
        with_stderr_to_null(|| command.output())?.map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    
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
