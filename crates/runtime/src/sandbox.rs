use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use wrkflw_logging;

/// Configuration for sandbox execution
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Maximum execution time for commands
    pub max_execution_time: Duration,
    /// Maximum memory usage in MB
    pub max_memory_mb: u64,
    /// Maximum CPU usage percentage
    pub max_cpu_percent: u64,
    /// Allowed commands (whitelist)
    pub allowed_commands: HashSet<String>,
    /// Blocked commands (blacklist)
    pub blocked_commands: HashSet<String>,
    /// Allowed file system paths (read-only)
    pub allowed_read_paths: HashSet<PathBuf>,
    /// Allowed file system paths (read-write)
    pub allowed_write_paths: HashSet<PathBuf>,
    /// Whether to enable network access
    pub allow_network: bool,
    /// Maximum number of processes
    pub max_processes: u32,
    /// Whether to enable strict mode (more restrictive)
    pub strict_mode: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let mut allowed_commands = HashSet::new();

        // Basic safe commands
        allowed_commands.insert("echo".to_string());
        allowed_commands.insert("printf".to_string());
        allowed_commands.insert("cat".to_string());
        allowed_commands.insert("head".to_string());
        allowed_commands.insert("tail".to_string());
        allowed_commands.insert("grep".to_string());
        allowed_commands.insert("sed".to_string());
        allowed_commands.insert("awk".to_string());
        allowed_commands.insert("sort".to_string());
        allowed_commands.insert("uniq".to_string());
        allowed_commands.insert("wc".to_string());
        allowed_commands.insert("cut".to_string());
        allowed_commands.insert("tr".to_string());
        allowed_commands.insert("which".to_string());
        allowed_commands.insert("pwd".to_string());
        allowed_commands.insert("env".to_string());
        allowed_commands.insert("date".to_string());
        allowed_commands.insert("basename".to_string());
        allowed_commands.insert("dirname".to_string());

        // File operations (safe variants)
        allowed_commands.insert("ls".to_string());
        allowed_commands.insert("find".to_string());
        allowed_commands.insert("mkdir".to_string());
        allowed_commands.insert("touch".to_string());
        allowed_commands.insert("cp".to_string());
        allowed_commands.insert("mv".to_string());

        // Development tools
        allowed_commands.insert("git".to_string());
        allowed_commands.insert("cargo".to_string());
        allowed_commands.insert("rustc".to_string());
        allowed_commands.insert("rustfmt".to_string());
        allowed_commands.insert("clippy".to_string());
        allowed_commands.insert("npm".to_string());
        allowed_commands.insert("yarn".to_string());
        allowed_commands.insert("node".to_string());
        allowed_commands.insert("python".to_string());
        allowed_commands.insert("python3".to_string());
        allowed_commands.insert("pip".to_string());
        allowed_commands.insert("pip3".to_string());
        allowed_commands.insert("java".to_string());
        allowed_commands.insert("javac".to_string());
        allowed_commands.insert("maven".to_string());
        allowed_commands.insert("gradle".to_string());
        allowed_commands.insert("go".to_string());
        allowed_commands.insert("dotnet".to_string());

        // Compression tools
        allowed_commands.insert("tar".to_string());
        allowed_commands.insert("gzip".to_string());
        allowed_commands.insert("gunzip".to_string());
        allowed_commands.insert("zip".to_string());
        allowed_commands.insert("unzip".to_string());

        let mut blocked_commands = HashSet::new();

        // Dangerous system commands
        blocked_commands.insert("rm".to_string());
        blocked_commands.insert("rmdir".to_string());
        blocked_commands.insert("dd".to_string());
        blocked_commands.insert("mkfs".to_string());
        blocked_commands.insert("fdisk".to_string());
        blocked_commands.insert("mount".to_string());
        blocked_commands.insert("umount".to_string());
        blocked_commands.insert("sudo".to_string());
        blocked_commands.insert("su".to_string());
        blocked_commands.insert("passwd".to_string());
        blocked_commands.insert("chown".to_string());
        blocked_commands.insert("chmod".to_string());
        blocked_commands.insert("chgrp".to_string());
        blocked_commands.insert("chroot".to_string());

        // Network and system tools
        blocked_commands.insert("nc".to_string());
        blocked_commands.insert("netcat".to_string());
        blocked_commands.insert("wget".to_string());
        blocked_commands.insert("curl".to_string());
        blocked_commands.insert("ssh".to_string());
        blocked_commands.insert("scp".to_string());
        blocked_commands.insert("rsync".to_string());

        // Process control
        blocked_commands.insert("kill".to_string());
        blocked_commands.insert("killall".to_string());
        blocked_commands.insert("pkill".to_string());
        blocked_commands.insert("nohup".to_string());
        blocked_commands.insert("screen".to_string());
        blocked_commands.insert("tmux".to_string());

        // System modification
        blocked_commands.insert("systemctl".to_string());
        blocked_commands.insert("service".to_string());
        blocked_commands.insert("crontab".to_string());
        blocked_commands.insert("at".to_string());
        blocked_commands.insert("reboot".to_string());
        blocked_commands.insert("shutdown".to_string());
        blocked_commands.insert("halt".to_string());
        blocked_commands.insert("poweroff".to_string());

        Self {
            max_execution_time: Duration::from_secs(300), // 5 minutes
            max_memory_mb: 512,
            max_cpu_percent: 80,
            allowed_commands,
            blocked_commands,
            allowed_read_paths: HashSet::new(),
            allowed_write_paths: HashSet::new(),
            allow_network: false,
            max_processes: 10,
            strict_mode: true,
        }
    }
}

/// Sandbox error types
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Command blocked by security policy: {command}")]
    BlockedCommand { command: String },

    #[error("Dangerous command pattern detected: {pattern}")]
    DangerousPattern { pattern: String },

    #[error("Path access denied: {path}")]
    PathAccessDenied { path: String },

    #[error("Resource limit exceeded: {resource}")]
    ResourceLimitExceeded { resource: String },

    #[error("Execution timeout after {seconds} seconds")]
    ExecutionTimeout { seconds: u64 },

    #[error("Sandbox setup failed: {reason}")]
    SandboxSetupError { reason: String },

    #[error("Command execution failed: {reason}")]
    ExecutionError { reason: String },
}

/// Secure sandbox for executing commands in emulation mode
pub struct Sandbox {
    config: SandboxConfig,
    workspace: TempDir,
    dangerous_patterns: Vec<Regex>,
}

impl Sandbox {
    /// Create a new sandbox with the given configuration
    pub fn new(config: SandboxConfig) -> Result<Self, SandboxError> {
        let workspace = tempfile::tempdir().map_err(|e| SandboxError::SandboxSetupError {
            reason: format!("Failed to create sandbox workspace: {}", e),
        })?;

        let dangerous_patterns = Self::compile_dangerous_patterns();

        wrkflw_logging::info(&format!(
            "Created new sandbox with workspace: {}",
            workspace.path().display()
        ));

        Ok(Self {
            config,
            workspace,
            dangerous_patterns,
        })
    }

    /// Execute a command in the sandbox
    pub async fn execute_command(
        &self,
        command: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
    ) -> Result<crate::container::ContainerOutput, SandboxError> {
        if command.is_empty() {
            return Err(SandboxError::ExecutionError {
                reason: "Empty command".to_string(),
            });
        }

        let command_str = command.join(" ");

        // Step 1: Validate command
        self.validate_command(&command_str)?;

        // Step 2: Setup sandbox environment
        let sandbox_dir = self.setup_sandbox_environment(working_dir)?;

        // Step 3: Execute with limits
        self.execute_with_limits(command, env_vars, &sandbox_dir)
            .await
    }

    /// Validate that a command is safe to execute
    fn validate_command(&self, command_str: &str) -> Result<(), SandboxError> {
        // Check for dangerous patterns first
        for pattern in &self.dangerous_patterns {
            if pattern.is_match(command_str) {
                wrkflw_logging::warning(&format!(
                    "{} Blocked dangerous command pattern: {}",
                    wrkflw_logging::symbols::BLOCKED,
                    command_str
                ));
                return Err(SandboxError::DangerousPattern {
                    pattern: command_str.to_string(),
                });
            }
        }

        // Split command by shell operators to validate each part
        let command_parts = self.split_shell_command(command_str);

        for part in command_parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            // Extract the base command from this part
            let base_command = part.split_whitespace().next().unwrap_or("");
            let command_name = Path::new(base_command)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(base_command);

            // Skip shell built-ins and operators
            if self.is_shell_builtin(command_name) {
                continue;
            }

            // Check blocked commands
            if self.config.blocked_commands.contains(command_name) {
                wrkflw_logging::warning(&format!(
                    "{} Blocked command: {}",
                    wrkflw_logging::symbols::BLOCKED,
                    command_name
                ));
                return Err(SandboxError::BlockedCommand {
                    command: command_name.to_string(),
                });
            }

            // In strict mode, only allow whitelisted commands
            if self.config.strict_mode && !self.config.allowed_commands.contains(command_name) {
                wrkflw_logging::warning(&format!(
                    "{} Command not in whitelist (strict mode): {}",
                    wrkflw_logging::symbols::BLOCKED,
                    command_name
                ));
                return Err(SandboxError::BlockedCommand {
                    command: command_name.to_string(),
                });
            }
        }

        wrkflw_logging::info(&format!(
            "{} Command validation passed: {}",
            wrkflw_logging::symbols::SUCCESS,
            command_str
        ));
        Ok(())
    }

    /// Split shell command by operators while preserving quoted strings
    fn split_shell_command(&self, command_str: &str) -> Vec<String> {
        // Simple split by common shell operators
        // This is not a full shell parser but handles most cases
        let separators = ["&&", "||", ";", "|"];
        let mut parts = vec![command_str.to_string()];

        for separator in separators {
            let mut new_parts = Vec::new();
            for part in parts {
                let split_parts: Vec<String> = part
                    .split(separator)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                new_parts.extend(split_parts);
            }
            parts = new_parts;
        }

        parts
    }

    /// Check if a command is a shell built-in
    fn is_shell_builtin(&self, command: &str) -> bool {
        let builtins = [
            "true", "false", "test", "[", "echo", "printf", "cd", "pwd", "export", "set", "unset",
            "alias", "history", "jobs", "fg", "bg", "wait", "read",
        ];
        builtins.contains(&command)
    }

    /// Setup isolated sandbox environment
    fn setup_sandbox_environment(&self, working_dir: &Path) -> Result<PathBuf, SandboxError> {
        let sandbox_root = self.workspace.path();
        let sandbox_workspace = sandbox_root.join("workspace");

        // Create sandbox directory structure
        fs::create_dir_all(&sandbox_workspace).map_err(|e| SandboxError::SandboxSetupError {
            reason: format!("Failed to create sandbox workspace: {}", e),
        })?;

        // Copy allowed files to sandbox (if working_dir exists and is allowed)
        if working_dir.exists() && self.is_path_allowed(working_dir, false) {
            self.copy_safe_files(working_dir, &sandbox_workspace)?;
        }

        wrkflw_logging::info(&format!(
            "Sandbox environment ready: {}",
            sandbox_workspace.display()
        ));

        Ok(sandbox_workspace)
    }

    /// Copy files safely to sandbox, excluding dangerous files
    fn copy_safe_files(&self, source: &Path, dest: &Path) -> Result<(), SandboxError> {
        for entry in fs::read_dir(source).map_err(|e| SandboxError::SandboxSetupError {
            reason: format!("Failed to read source directory: {}", e),
        })? {
            let entry = entry.map_err(|e| SandboxError::SandboxSetupError {
                reason: format!("Failed to read directory entry: {}", e),
            })?;

            let path = entry.path();
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");

            // Skip dangerous or sensitive files
            if self.should_skip_file(file_name) {
                continue;
            }

            let dest_path = dest.join(file_name);

            if path.is_file() {
                fs::copy(&path, &dest_path).map_err(|e| SandboxError::SandboxSetupError {
                    reason: format!("Failed to copy file: {}", e),
                })?;
            } else if path.is_dir() && !self.should_skip_directory(file_name) {
                fs::create_dir_all(&dest_path).map_err(|e| SandboxError::SandboxSetupError {
                    reason: format!("Failed to create directory: {}", e),
                })?;
                self.copy_safe_files(&path, &dest_path)?;
            }
        }

        Ok(())
    }

    /// Execute command with resource limits and monitoring
    async fn execute_with_limits(
        &self,
        command: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
    ) -> Result<crate::container::ContainerOutput, SandboxError> {
        // Join command parts and execute via shell for proper handling of operators
        let command_str = command.join(" ");

        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg(&command_str);
        cmd.current_dir(working_dir);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Set environment variables (filtered)
        for (key, value) in env_vars {
            if self.is_env_var_safe(key) {
                cmd.env(key, value);
            }
        }

        // Add sandbox-specific environment variables
        cmd.env("WRKFLW_SANDBOXED", "true");
        cmd.env("WRKFLW_SANDBOX_MODE", "strict");

        // Execute with timeout
        let timeout_duration = self.config.max_execution_time;

        wrkflw_logging::info(&format!(
            "🏃 Executing sandboxed command: {} (timeout: {}s)",
            command.join(" "),
            timeout_duration.as_secs()
        ));

        let start_time = std::time::Instant::now();

        let result = tokio::time::timeout(timeout_duration, async {
            let output = cmd.output().map_err(|e| SandboxError::ExecutionError {
                reason: format!("Command execution failed: {}", e),
            })?;

            Ok(crate::container::ContainerOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
            })
        })
        .await;

        let execution_time = start_time.elapsed();

        match result {
            Ok(output_result) => {
                wrkflw_logging::info(&format!(
                    "{} Sandboxed command completed in {:.2}s",
                    wrkflw_logging::symbols::SUCCESS,
                    execution_time.as_secs_f64()
                ));
                output_result
            }
            Err(_) => {
                wrkflw_logging::warning(&format!(
                    "{} Sandboxed command timed out after {:.2}s",
                    wrkflw_logging::symbols::WARNING,
                    timeout_duration.as_secs_f64()
                ));
                Err(SandboxError::ExecutionTimeout {
                    seconds: timeout_duration.as_secs(),
                })
            }
        }
    }

    /// Check if a path is allowed for access
    fn is_path_allowed(&self, path: &Path, write_access: bool) -> bool {
        let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if write_access {
            self.config
                .allowed_write_paths
                .iter()
                .any(|allowed| abs_path.starts_with(allowed))
        } else {
            self.config
                .allowed_read_paths
                .iter()
                .any(|allowed| abs_path.starts_with(allowed))
                || self
                    .config
                    .allowed_write_paths
                    .iter()
                    .any(|allowed| abs_path.starts_with(allowed))
        }
    }

    /// Check if an environment variable is safe to pass through
    fn is_env_var_safe(&self, key: &str) -> bool {
        // Block dangerous environment variables
        let dangerous_env_vars = [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            "PATH",
            "HOME",
            "SHELL",
        ];

        !dangerous_env_vars.contains(&key)
    }

    /// Check if a file should be skipped during copying
    fn should_skip_file(&self, filename: &str) -> bool {
        let dangerous_files = [
            ".ssh",
            ".gnupg",
            ".aws",
            ".docker",
            "id_rsa",
            "id_ed25519",
            "credentials",
            "config",
            ".env",
            ".secrets",
        ];

        dangerous_files
            .iter()
            .any(|pattern| filename.contains(pattern))
            || filename.starts_with('.') && filename != ".gitignore" && filename != ".github"
    }

    /// Check if a directory should be skipped
    fn should_skip_directory(&self, dirname: &str) -> bool {
        let skip_dirs = [
            "target",
            "node_modules",
            ".git",
            ".cargo",
            ".npm",
            ".cache",
            "build",
            "dist",
            "tmp",
            "temp",
        ];

        skip_dirs.contains(&dirname)
    }

    /// Compile regex patterns for dangerous command detection
    fn compile_dangerous_patterns() -> Vec<Regex> {
        let patterns = [
            r"rm\s+.*-rf?\s*/",       // rm -rf /
            r"dd\s+.*of=/dev/",       // dd ... of=/dev/...
            r">\s*/dev/sd[a-z]",      // > /dev/sda
            r"mkfs\.",                // mkfs.ext4, etc.
            r"fdisk\s+/dev/",         // fdisk /dev/...
            r"mount\s+.*\s+/",        // mount ... /
            r"chroot\s+/",            // chroot /
            r"sudo\s+",               // sudo commands
            r"su\s+",                 // su commands
            r"bash\s+-c\s+.*rm.*-rf", // bash -c "rm -rf ..."
            r"sh\s+-c\s+.*rm.*-rf",   // sh -c "rm -rf ..."
            r"eval\s+.*rm.*-rf",      // eval "rm -rf ..."
            r":\(\)\{.*;\};:",        // Fork bomb
            r"/proc/sys/",            // /proc/sys access
            r"/etc/passwd",           // /etc/passwd access
            r"/etc/shadow",           // /etc/shadow access
            r"nc\s+.*-e",             // netcat with exec
            r"wget\s+.*\|\s*sh",      // wget ... | sh
            r"curl\s+.*\|\s*sh",      // curl ... | sh
        ];

        patterns
            .iter()
            .filter_map(|pattern| {
                Regex::new(pattern)
                    .map_err(|e| {
                        wrkflw_logging::warning(&format!(
                            "Invalid regex pattern {}: {}",
                            pattern, e
                        ));
                        e
                    })
                    .ok()
            })
            .collect()
    }
}

/// Create a default sandbox configuration for CI/CD workflows
pub fn create_workflow_sandbox_config() -> SandboxConfig {
    let mut allowed_read_paths = HashSet::new();
    allowed_read_paths.insert(PathBuf::from("."));

    let mut allowed_write_paths = HashSet::new();
    allowed_write_paths.insert(PathBuf::from("."));

    SandboxConfig {
        max_execution_time: Duration::from_secs(1800), // 30 minutes
        max_memory_mb: 2048,                           // 2GB
        max_processes: 50,
        allow_network: true,
        strict_mode: false,
        allowed_read_paths,
        allowed_write_paths,
        ..Default::default()
    }
}

/// Create a strict sandbox configuration for untrusted code
pub fn create_strict_sandbox_config() -> SandboxConfig {
    let mut allowed_read_paths = HashSet::new();
    allowed_read_paths.insert(PathBuf::from("."));

    let mut allowed_write_paths = HashSet::new();
    allowed_write_paths.insert(PathBuf::from("."));

    // Very limited command set
    let allowed_commands = ["echo", "cat", "ls", "pwd", "date"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    SandboxConfig {
        max_execution_time: Duration::from_secs(60), // 1 minute
        max_memory_mb: 128,                          // 128MB
        max_processes: 5,
        allow_network: false,
        strict_mode: true,
        allowed_read_paths,
        allowed_write_paths,
        allowed_commands,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dangerous_pattern_detection() {
        let sandbox = Sandbox::new(SandboxConfig::default()).unwrap();

        // Should block dangerous commands
        assert!(sandbox.validate_command("rm -rf /").is_err());
        assert!(sandbox
            .validate_command("dd if=/dev/zero of=/dev/sda")
            .is_err());
        assert!(sandbox.validate_command("sudo rm -rf /home").is_err());
        assert!(sandbox.validate_command("bash -c 'rm -rf /'").is_err());

        // Should allow safe commands
        assert!(sandbox.validate_command("echo hello").is_ok());
        assert!(sandbox.validate_command("ls -la").is_ok());
        assert!(sandbox.validate_command("cargo build").is_ok());
    }

    #[test]
    fn test_command_whitelist() {
        let config = create_strict_sandbox_config();
        let sandbox = Sandbox::new(config).unwrap();

        // Should allow whitelisted commands
        assert!(sandbox.validate_command("echo hello").is_ok());
        assert!(sandbox.validate_command("ls").is_ok());

        // Should block non-whitelisted commands
        assert!(sandbox.validate_command("git clone").is_err());
        assert!(sandbox.validate_command("cargo build").is_err());
    }

    #[test]
    fn test_file_filtering() {
        let sandbox = Sandbox::new(SandboxConfig::default()).unwrap();

        // Should skip dangerous files
        assert!(sandbox.should_skip_file("id_rsa"));
        assert!(sandbox.should_skip_file(".ssh"));
        assert!(sandbox.should_skip_file("credentials"));

        // Should allow safe files
        assert!(!sandbox.should_skip_file("Cargo.toml"));
        assert!(!sandbox.should_skip_file("README.md"));
        assert!(!sandbox.should_skip_file(".gitignore"));
    }
}
