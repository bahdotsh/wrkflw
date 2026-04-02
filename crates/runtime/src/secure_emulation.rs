use crate::container::{ContainerError, ContainerOutput, ContainerRuntime};
use crate::sandbox::{create_workflow_sandbox_config, Sandbox, SandboxConfig, SandboxError};
use async_trait::async_trait;
use std::path::Path;
use wrkflw_logging;

/// Secure emulation runtime that uses sandboxing for safety
pub struct SecureEmulationRuntime {
    sandbox: Sandbox,
}

impl Default for SecureEmulationRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl SecureEmulationRuntime {
    /// Create a new secure emulation runtime with default workflow-friendly configuration
    pub fn new() -> Self {
        let config = create_workflow_sandbox_config();
        let sandbox = Sandbox::new(config).expect("Failed to create sandbox");

        wrkflw_logging::info("🔒 Initialized secure emulation runtime with sandboxing");

        Self { sandbox }
    }

    /// Create a new secure emulation runtime with custom sandbox configuration
    pub fn new_with_config(config: SandboxConfig) -> Result<Self, ContainerError> {
        let sandbox = Sandbox::new(config).map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to create sandbox: {}", e))
        })?;

        wrkflw_logging::info("🔒 Initialized secure emulation runtime with custom config");

        Ok(Self { sandbox })
    }
}

#[async_trait]
impl ContainerRuntime for SecureEmulationRuntime {
    async fn run_container(
        &self,
        image: &str,
        command: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        _volumes: &[(&Path, &Path)],
        entrypoint: Option<&str>,
    ) -> Result<ContainerOutput, ContainerError> {
        if let Some(ep) = entrypoint {
            wrkflw_logging::warning(&format!(
                "Secure emulation mode ignoring entrypoint override '{}' for image '{}'. \
                 Use --runtime docker for full Docker action support.",
                ep, image
            ));
        }

        wrkflw_logging::info(&format!(
            "🔒 Executing sandboxed command: {} (image: {})",
            command.join(" "),
            image
        ));

        // Use sandbox to execute the command safely
        let result = self
            .sandbox
            .execute_command(command, env_vars, working_dir)
            .await;

        match result {
            Ok(output) => {
                wrkflw_logging::info("✅ Sandboxed command completed successfully");
                Ok(output)
            }
            Err(SandboxError::BlockedCommand { command }) => {
                let error_msg = format!(
                    "🚫 SECURITY BLOCK: Command '{}' is not allowed in secure emulation mode. \
                     This command was blocked for security reasons. \
                     If you need to run this command, please use Docker or Podman mode instead.",
                    command
                );
                wrkflw_logging::warning(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
            Err(SandboxError::DangerousPattern { pattern }) => {
                let error_msg = format!(
                    "🚫 SECURITY BLOCK: Dangerous command pattern detected: '{}'. \
                     This command was blocked because it matches a known dangerous pattern. \
                     Please review your workflow for potentially harmful commands.",
                    pattern
                );
                wrkflw_logging::warning(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
            Err(SandboxError::ExecutionTimeout { seconds }) => {
                let error_msg = format!(
                    "⏰ Command execution timed out after {} seconds. \
                     Consider optimizing your command or increasing timeout limits.",
                    seconds
                );
                wrkflw_logging::warning(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
            Err(SandboxError::PathAccessDenied { path }) => {
                let error_msg = format!(
                    "🚫 Path access denied: '{}'. \
                     The sandbox restricts file system access for security.",
                    path
                );
                wrkflw_logging::warning(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
            Err(SandboxError::ResourceLimitExceeded { resource }) => {
                let error_msg = format!(
                    "📊 Resource limit exceeded: {}. \
                     Your command used too many system resources.",
                    resource
                );
                wrkflw_logging::warning(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
            Err(e) => {
                let error_msg = format!("Sandbox execution failed: {}", e);
                wrkflw_logging::error(&error_msg);
                Err(ContainerError::ContainerExecution(error_msg))
            }
        }
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        wrkflw_logging::info(&format!(
            "🔒 Secure emulation: Pretending to pull image {}",
            image
        ));
        Ok(())
    }

    async fn build_image(
        &self,
        dockerfile: &Path,
        tag: &str,
        _context_dir: &Path,
    ) -> Result<(), ContainerError> {
        wrkflw_logging::info(&format!(
            "🔒 Secure emulation: Pretending to build image {} from {}",
            tag,
            dockerfile.display()
        ));
        Ok(())
    }

    async fn image_exists(&self, _tag: &str) -> Result<bool, ContainerError> {
        Ok(false)
    }

    async fn prepare_language_environment(
        &self,
        language: &str,
        version: Option<&str>,
        _additional_packages: Option<Vec<String>>,
    ) -> Result<String, ContainerError> {
        // For secure emulation runtime, we'll use a simplified approach
        // that doesn't require building custom images
        let base_image = match language {
            "python" => version.map_or("python:3.11-slim".to_string(), |v| format!("python:{}", v)),
            "node" => version.map_or("node:20-slim".to_string(), |v| format!("node:{}", v)),
            "java" => version.map_or("eclipse-temurin:17-jdk".to_string(), |v| {
                format!("eclipse-temurin:{}", v)
            }),
            "go" => version.map_or("golang:1.21-slim".to_string(), |v| format!("golang:{}", v)),
            "dotnet" => version.map_or("mcr.microsoft.com/dotnet/sdk:7.0".to_string(), |v| {
                format!("mcr.microsoft.com/dotnet/sdk:{}", v)
            }),
            "rust" => version.map_or("rust:latest".to_string(), |v| format!("rust:{}", v)),
            _ => {
                return Err(ContainerError::ContainerStart(format!(
                    "Unsupported language: {}",
                    language
                )))
            }
        };

        // For emulation, we'll just return the base image
        // The actual package installation will be handled during container execution
        Ok(base_image)
    }
}

/// Handle special actions in secure emulation mode
pub async fn handle_special_action_secure(action: &str) -> Result<(), ContainerError> {
    // Extract owner, repo and version from the action
    let action_parts: Vec<&str> = action.split('@').collect();
    let action_name = action_parts[0];
    let action_version = if action_parts.len() > 1 {
        action_parts[1]
    } else {
        "latest"
    };

    wrkflw_logging::info(&format!(
        "🔒 Processing action in secure mode: {} @ {}",
        action_name, action_version
    ));

    // In secure mode, we're more restrictive about what actions we allow
    match action_name {
        // Core GitHub actions that are generally safe
        name if name.starts_with("actions/checkout") => {
            wrkflw_logging::info("✅ Checkout action - workspace files are prepared securely");
        }
        name if name.starts_with("actions/setup-node") => {
            wrkflw_logging::info("🟡 Node.js setup - using system Node.js in secure mode");
            check_command_available_secure("node", "Node.js", "https://nodejs.org/");
        }
        name if name.starts_with("actions/setup-python") => {
            wrkflw_logging::info("🟡 Python setup - using system Python in secure mode");
            check_command_available_secure("python", "Python", "https://www.python.org/downloads/");
        }
        name if name.starts_with("actions/setup-java") => {
            wrkflw_logging::info("🟡 Java setup - using system Java in secure mode");
            check_command_available_secure("java", "Java", "https://adoptium.net/");
        }
        name if name.starts_with("actions/cache") => {
            wrkflw_logging::info("🟡 Cache action - caching disabled in secure emulation mode");
        }

        // Rust-specific actions
        name if name.starts_with("actions-rs/cargo") => {
            wrkflw_logging::info("🟡 Rust cargo action - using system Rust in secure mode");
            check_command_available_secure("cargo", "Rust/Cargo", "https://rustup.rs/");
        }
        name if name.starts_with("actions-rs/toolchain") => {
            wrkflw_logging::info("🟡 Rust toolchain action - using system Rust in secure mode");
            check_command_available_secure("rustc", "Rust", "https://rustup.rs/");
        }
        name if name.starts_with("actions-rs/fmt") => {
            wrkflw_logging::info("🟡 Rust formatter action - using system rustfmt in secure mode");
            check_command_available_secure("rustfmt", "rustfmt", "rustup component add rustfmt");
        }

        // Potentially dangerous actions that we warn about
        name if name.contains("docker") || name.contains("container") => {
            wrkflw_logging::warning(&format!(
                "🚫 Docker/container action '{}' is not supported in secure emulation mode. \
                 Use Docker or Podman mode for container actions.",
                action_name
            ));
        }
        name if name.contains("ssh") || name.contains("deploy") => {
            wrkflw_logging::warning(&format!(
                "🚫 SSH/deployment action '{}' is restricted in secure emulation mode. \
                 Use Docker or Podman mode for deployment actions.",
                action_name
            ));
        }

        // Unknown actions
        _ => {
            wrkflw_logging::warning(&format!(
                "🟡 Unknown action '{}' in secure emulation mode. \
                 Some functionality may be limited or unavailable.",
                action_name
            ));
        }
    }

    Ok(())
}

/// Check if a command is available, with security-focused messaging
fn check_command_available_secure(command: &str, name: &str, install_url: &str) {
    use std::process::Command;

    let is_available = Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !is_available {
        wrkflw_logging::warning(&format!(
            "🔧 {} is required but not found on the system",
            name
        ));
        wrkflw_logging::info(&format!(
            "To use this action in secure mode, please install {}: {}",
            name, install_url
        ));
        wrkflw_logging::info(&format!(
            "Alternatively, use Docker or Podman mode for automatic {} installation",
            name
        ));
    } else {
        // Try to get version information
        if let Ok(output) = Command::new(command).arg("--version").output() {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                wrkflw_logging::info(&format!(
                    "✅ Using system {} in secure mode: {}",
                    name,
                    version.trim()
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::create_strict_sandbox_config;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_secure_emulation_blocks_dangerous_commands() {
        let config = create_strict_sandbox_config();
        let runtime = SecureEmulationRuntime::new_with_config(config).unwrap();

        // Should block dangerous commands
        let result = runtime
            .run_container(
                "alpine:latest",
                &["rm", "-rf", "/"],
                &[],
                &PathBuf::from("."),
                &[],
                None,
            )
            .await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("SECURITY BLOCK"));
    }

    #[tokio::test]
    async fn test_secure_emulation_allows_safe_commands() {
        let runtime = SecureEmulationRuntime::new();

        // Should allow safe commands
        let result = runtime
            .run_container(
                "alpine:latest",
                &["echo", "hello world"],
                &[],
                &PathBuf::from("."),
                &[],
                None,
            )
            .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.stdout.contains("hello world"));
        assert_eq!(output.exit_code, 0);
    }
}
