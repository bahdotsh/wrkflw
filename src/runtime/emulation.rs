use crate::logging;
use crate::runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use tempfile::TempDir;

// Global collection of resources to clean up
static EMULATION_WORKSPACES: Lazy<Mutex<Vec<PathBuf>>> = Lazy::new(|| Mutex::new(Vec::new()));
static EMULATION_PROCESSES: Lazy<Mutex<Vec<u32>>> = Lazy::new(|| Mutex::new(Vec::new()));

pub struct EmulationRuntime {
    #[allow(dead_code)]
    workspace: TempDir,
}

impl EmulationRuntime {
    pub fn new() -> Self {
        // Create a temporary workspace to simulate container isolation
        let workspace =
            tempfile::tempdir().expect("Failed to create temporary workspace for emulation");

        // Track this workspace for cleanup
        if let Ok(mut workspaces) = EMULATION_WORKSPACES.lock() {
            workspaces.push(workspace.path().to_path_buf());
        }

        EmulationRuntime { workspace }
    }

    #[allow(dead_code)]
    fn prepare_workspace(&self, _working_dir: &Path, volumes: &[(&Path, &Path)]) -> PathBuf {
        // Get the container root - this is the emulation workspace directory
        let container_root = self.workspace.path().to_path_buf();

        // Make sure we have a github/workspace subdirectory which is where
        // commands will be executed
        let github_workspace = container_root.join("github").join("workspace");
        fs::create_dir_all(&github_workspace)
            .expect("Failed to create github/workspace directory structure");

        // Map all volumes
        for (host_path, container_path) in volumes {
            // Determine target path - if it starts with /github/workspace, it goes to our workspace dir
            let target_path = if container_path.starts_with("/github/workspace") {
                // Map /github/workspace to our github_workspace directory
                let rel_path = container_path
                    .strip_prefix("/github/workspace")
                    .unwrap_or(Path::new(""));
                github_workspace.join(rel_path)
            } else if container_path.starts_with("/") {
                // Other absolute paths go under container_root
                container_root.join(container_path.strip_prefix("/").unwrap_or(container_path))
            } else {
                // Relative paths go directly under container_root
                container_root.join(container_path)
            };

            // Create parent directories
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).expect("Failed to create directory structure");
            }

            // For directories, copy content recursively
            if host_path.is_dir() {
                // If the host path is the project root and container path is the workspace,
                // we want to copy all project files to the github/workspace directory
                if *container_path == Path::new("/github/workspace") {
                    // Use a recursive copy function to copy all files and directories
                    copy_directory_contents(host_path, &github_workspace)
                        .expect("Failed to copy project files to workspace");
                } else {
                    // Create the target directory
                    fs::create_dir_all(&target_path).expect("Failed to create target directory");

                    // Copy files in this directory (not recursive for simplicity)
                    for entry in fs::read_dir(host_path)
                        .expect("Failed to read source directory")
                        .flatten()
                    {
                        let source = entry.path();
                        let file_name = match source.file_name() {
                            Some(name) => name,
                            None => {
                                eprintln!(
                                    "Warning: Could not get file name from path: {:?}",
                                    source
                                );
                                continue; // Skip this file
                            }
                        };
                        let dest = target_path.join(file_name);

                        if source.is_file() {
                            if let Err(e) = fs::copy(&source, &dest) {
                                eprintln!(
                                    "Warning: Failed to copy file from {:?} to {:?}: {}",
                                    &source, &dest, e
                                );
                            }
                        } else {
                            // We could make this recursive if needed
                            fs::create_dir_all(&dest).expect("Failed to create subdirectory");
                        }
                    }
                }
            } else if host_path.is_file() {
                // Copy individual file
                let file_name = match host_path.file_name() {
                    Some(name) => name,
                    None => {
                        eprintln!(
                            "Warning: Could not get file name from path: {:?}",
                            host_path
                        );
                        continue; // Skip this file
                    }
                };
                let dest = target_path.join(file_name);
                if let Err(e) = fs::copy(host_path, &dest) {
                    eprintln!(
                        "Warning: Failed to copy file from {:?} to {:?}: {}",
                        host_path, &dest, e
                    );
                }
            }
        }

        // Return the github/workspace directory for command execution
        github_workspace
    }
}

#[async_trait]
impl ContainerRuntime for EmulationRuntime {
    async fn run_container(
        &self,
        _image: &str,
        command: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        _volumes: &[(&Path, &Path)],
    ) -> Result<ContainerOutput, ContainerError> {
        // Build command string
        let mut command_str = String::new();
        for part in command {
            if !command_str.is_empty() {
                command_str.push(' ');
            }
            command_str.push_str(part);
        }

        // Log the command being executed
        logging::info(&format!("Executing command in container: {}", command_str));

        // Special handling for Rust/Cargo actions
        if command_str.contains("rust") || command_str.contains("cargo") {
            logging::debug(&format!("Executing Rust command: {}", command_str));

            let mut cmd = Command::new("cargo");
            let parts = command_str.split_whitespace().collect::<Vec<&str>>();

            let current_dir = working_dir.to_str().unwrap_or(".");
            cmd.current_dir(current_dir);

            // Add environment variables
            for (key, value) in env_vars {
                cmd.env(key, value);
            }

            // Add command arguments
            if parts.len() > 1 {
                cmd.args(&parts[1..]);
            }

            match cmd.output() {
                Ok(output_result) => {
                    let exit_code = output_result.status.code().unwrap_or(-1);
                    let output = String::from_utf8_lossy(&output_result.stdout).to_string();
                    let error = String::from_utf8_lossy(&output_result.stderr).to_string();

                    logging::debug(&format!("Command exit code: {}", exit_code));

                    if exit_code != 0 {
                        let mut error_details = format!(
                            "Command failed with exit code: {}\nCommand: {}\n\nError output:\n{}",
                            exit_code, command_str, error
                        );

                        // Add environment variables to error details
                        error_details.push_str("\n\nEnvironment variables:\n");
                        for (key, value) in env_vars {
                            if key.starts_with("GITHUB_") || key.starts_with("RUST") {
                                error_details.push_str(&format!("{}={}\n", key, value));
                            }
                        }

                        return Err(ContainerError::ContainerExecution(error_details));
                    }

                    return Ok(ContainerOutput {
                        stdout: output,
                        stderr: error,
                        exit_code,
                    });
                }
                Err(e) => {
                    return Err(ContainerError::ContainerExecution(format!(
                        "Failed to execute Rust command: {}",
                        e
                    )));
                }
            }
        }

        // For other commands, use a shell
        let mut cmd = Command::new("sh");
        cmd.arg("-c");
        cmd.arg(&command_str);
        cmd.current_dir(working_dir.to_str().unwrap_or("."));

        // Add environment variables
        for (key, value) in env_vars {
            cmd.env(key, value);
        }

        match cmd.output() {
            Ok(output_result) => {
                let exit_code = output_result.status.code().unwrap_or(-1);
                let output = String::from_utf8_lossy(&output_result.stdout).to_string();
                let error = String::from_utf8_lossy(&output_result.stderr).to_string();

                logging::debug(&format!("Command completed with exit code: {}", exit_code));

                if exit_code != 0 {
                    let mut error_details = format!(
                        "Command failed with exit code: {}\nCommand: {}\n\nError output:\n{}",
                        exit_code, command_str, error
                    );

                    // Add environment variables to error details
                    error_details.push_str("\n\nEnvironment variables:\n");
                    for (key, value) in env_vars {
                        if key.starts_with("GITHUB_") {
                            error_details.push_str(&format!("{}={}\n", key, value));
                        }
                    }

                    return Err(ContainerError::ContainerExecution(error_details));
                }

                Ok(ContainerOutput {
                    stdout: format!(
                        "Emulated container execution with command: {}\n\nOutput:\n{}",
                        command_str, output
                    ),
                    stderr: error,
                    exit_code,
                })
            }
            Err(e) => {
                return Err(ContainerError::ContainerExecution(format!(
                    "Failed to execute command: {}\nError: {}",
                    command_str, e
                )));
            }
        }
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        logging::info(&format!("🔄 Emulation: Pretending to pull image {}", image));
        Ok(())
    }

    async fn build_image(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError> {
        logging::info(&format!(
            "🔄 Emulation: Pretending to build image {} from {}",
            tag,
            dockerfile.display()
        ));
        Ok(())
    }
}

#[allow(dead_code)]
fn copy_directory_contents(source: &Path, dest: &Path) -> std::io::Result<()> {
    // Create the destination directory if it doesn't exist
    fs::create_dir_all(dest)?;

    // Iterate through all entries in the source directory
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = match path.file_name() {
            Some(name) => name,
            None => {
                eprintln!("Warning: Could not get file name from path: {:?}", path);
                continue; // Skip this file
            }
        };
        let dest_path = dest.join(file_name);

        // Skip hidden files (except .gitignore and .github might be useful)
        let file_name_str = file_name.to_string_lossy();
        if file_name_str.starts_with(".")
            && file_name_str != ".gitignore"
            && file_name_str != ".github"
        {
            continue;
        }

        // Skip target directory for Rust projects
        if file_name_str == "target" {
            continue;
        }

        if path.is_dir() {
            // Recursively copy subdirectories
            copy_directory_contents(&path, &dest_path)?;
        } else {
            // Copy files
            fs::copy(&path, &dest_path)?;
        }
    }

    Ok(())
}

pub async fn handle_special_action(action: &str) -> Result<(), ContainerError> {
    // Extract owner, repo and version from the action
    let action_parts: Vec<&str> = action.split('@').collect();
    let action_name = action_parts[0];
    let action_version = if action_parts.len() > 1 {
        action_parts[1]
    } else {
        "latest"
    };

    logging::info(&format!(
        "🔄 Processing action: {} @ {}",
        action_name, action_version
    ));

    // Handle specific known actions with special requirements
    if action.starts_with("cachix/install-nix-action") {
        logging::info("🔄 Emulating cachix/install-nix-action");

        // In emulation mode, check if nix is installed
        let nix_installed = Command::new("which")
            .arg("nix")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !nix_installed {
            logging::info("🔄 Emulation: Nix is required but not installed.");
            logging::info(
                "🔄 To use this workflow, please install Nix: https://nixos.org/download.html",
            );
            logging::info("🔄 Continuing emulation, but nix commands will fail.");
        } else {
            logging::info("🔄 Emulation: Using system-installed Nix");
        }
    } else if action.starts_with("actions-rs/cargo@") {
        // For actions-rs/cargo action, ensure Rust is available
        logging::info(&format!("🔄 Detected Rust cargo action: {}", action));

        // Verify Rust/cargo is installed
        check_command_available("cargo", "Rust/Cargo", "https://rustup.rs/");
    } else if action.starts_with("actions-rs/toolchain@") {
        // For actions-rs/toolchain action, check for Rust installation
        logging::info(&format!("🔄 Detected Rust toolchain action: {}", action));

        check_command_available("rustc", "Rust", "https://rustup.rs/");
    } else if action.starts_with("actions-rs/fmt@") {
        // For actions-rs/fmt action, check if rustfmt is available
        logging::info(&format!("🔄 Detected Rust formatter action: {}", action));

        check_command_available("rustfmt", "rustfmt", "rustup component add rustfmt");
    } else if action.starts_with("actions/setup-node@") {
        // Node.js setup action
        logging::info(&format!("🔄 Detected Node.js setup action: {}", action));

        check_command_available("node", "Node.js", "https://nodejs.org/");
    } else if action.starts_with("actions/setup-python@") {
        // Python setup action
        logging::info(&format!("🔄 Detected Python setup action: {}", action));

        check_command_available("python", "Python", "https://www.python.org/downloads/");
    } else if action.starts_with("actions/setup-java@") {
        // Java setup action
        logging::info(&format!("🔄 Detected Java setup action: {}", action));

        check_command_available("java", "Java", "https://adoptium.net/");
    } else if action.starts_with("actions/checkout@") {
        // Git checkout action - this is handled implicitly by our workspace setup
        logging::info("🔄 Detected checkout action - workspace files are already prepared");
    } else if action.starts_with("actions/cache@") {
        // Cache action - can't really emulate caching effectively
        logging::info(
            "🔄 Detected cache action - caching is not fully supported in emulation mode",
        );
    } else {
        // Generic action we don't have special handling for
        logging::info(&format!(
            "🔄 Action '{}' has no special handling in emulation mode",
            action_name
        ));
    }

    // Always return success - the actual command execution will happen in execute_step
    Ok(())
}

// Helper function to check if a command is available on the system
fn check_command_available(command: &str, name: &str, install_url: &str) {
    let is_available = Command::new("which")
        .arg(command)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);

    if !is_available {
        logging::warning(&format!("{} is required but not found on the system", name));
        logging::info(&format!(
            "To use this action, please install {}: {}",
            name, install_url
        ));
        logging::info(&format!(
            "Continuing emulation, but {} commands will fail",
            name
        ));
    } else {
        // Try to get version information
        if let Ok(output) = Command::new(command).arg("--version").output() {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                logging::info(&format!("🔄 Using system {}: {}", name, version.trim()));
            }
        }
    }
}

// Add a function to help set up appropriate environment variables for different actions
#[allow(dead_code)]
fn add_action_env_vars(
    env_map: &mut HashMap<String, String>,
    action: &str,
    with_params: &Option<HashMap<String, String>>,
) {
    if let Some(params) = with_params {
        if action.starts_with("actions/setup-node") {
            // For Node.js actions, add NODE_VERSION
            if let Some(version) = params.get("node-version") {
                env_map.insert("NODE_VERSION".to_string(), version.clone());
            }

            // Set NPM/Yarn paths if needed
            env_map.insert(
                "NPM_CONFIG_PREFIX".to_string(),
                "/tmp/.npm-global".to_string(),
            );
            env_map.insert("PATH".to_string(), "/tmp/.npm-global/bin:$PATH".to_string());
        } else if action.starts_with("actions/setup-python") {
            // For Python actions, add PYTHON_VERSION
            if let Some(version) = params.get("python-version") {
                env_map.insert("PYTHON_VERSION".to_string(), version.clone());
            }

            // Set pip cache directories
            env_map.insert("PIP_CACHE_DIR".to_string(), "/tmp/.pip-cache".to_string());
        } else if action.starts_with("actions/setup-java") {
            // For Java actions, add JAVA_VERSION
            if let Some(version) = params.get("java-version") {
                env_map.insert("JAVA_VERSION".to_string(), version.clone());
            }

            // Set JAVA_HOME
            env_map.insert(
                "JAVA_HOME".to_string(),
                "/usr/lib/jvm/default-java".to_string(),
            );
        }
    }
}

// Function to clean up emulation resources
pub async fn cleanup_resources() {
    cleanup_processes().await;
    cleanup_workspaces().await;
}

// Clean up any tracked processes
async fn cleanup_processes() {
    let processes_to_cleanup = {
        if let Ok(processes) = EMULATION_PROCESSES.lock() {
            processes.clone()
        } else {
            vec![]
        }
    };

    for pid in processes_to_cleanup {
        logging::info(&format!("Cleaning up emulated process: {}", pid));

        #[cfg(unix)]
        {
            // On Unix-like systems, use kill command
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .output();
        }

        #[cfg(windows)]
        {
            // On Windows, use taskkill
            let _ = Command::new("taskkill")
                .arg("/F")
                .arg("/PID")
                .arg(&pid.to_string())
                .output();
        }

        // Remove from tracking
        if let Ok(mut processes) = EMULATION_PROCESSES.lock() {
            processes.retain(|p| *p != pid);
        }
    }
}

// Clean up any tracked workspaces
async fn cleanup_workspaces() {
    let workspaces_to_cleanup = {
        if let Ok(workspaces) = EMULATION_WORKSPACES.lock() {
            workspaces.clone()
        } else {
            vec![]
        }
    };

    for workspace_path in workspaces_to_cleanup {
        logging::info(&format!(
            "Cleaning up emulation workspace: {}",
            workspace_path.display()
        ));

        // Only attempt to remove if it exists
        if workspace_path.exists() {
            match fs::remove_dir_all(&workspace_path) {
                Ok(_) => logging::info("Successfully removed workspace directory"),
                Err(e) => logging::error(&format!("Error removing workspace: {}", e)),
            }
        }

        // Remove from tracking
        if let Ok(mut workspaces) = EMULATION_WORKSPACES.lock() {
            workspaces.retain(|w| *w != workspace_path);
        }
    }
}

// Add process to tracking
#[allow(dead_code)]
pub fn track_process(pid: u32) {
    if let Ok(mut processes) = EMULATION_PROCESSES.lock() {
        processes.push(pid);
    }
}

// Remove process from tracking
#[allow(dead_code)]
pub fn untrack_process(pid: u32) {
    if let Ok(mut processes) = EMULATION_PROCESSES.lock() {
        processes.retain(|p| *p != pid);
    }
}

// Track additional workspace paths if needed
#[allow(dead_code)]
pub fn track_workspace(path: &Path) {
    if let Ok(mut workspaces) = EMULATION_WORKSPACES.lock() {
        workspaces.push(path.to_path_buf());
    }
}

// Remove workspace from tracking
#[allow(dead_code)]
pub fn untrack_workspace(path: &Path) {
    if let Ok(mut workspaces) = EMULATION_WORKSPACES.lock() {
        workspaces.retain(|w| *w != path);
    }
}

// Public accessor functions for testing
#[cfg(test)]
pub fn get_tracked_workspaces() -> Vec<PathBuf> {
    if let Ok(workspaces) = EMULATION_WORKSPACES.lock() {
        workspaces.clone()
    } else {
        vec![]
    }
}

#[cfg(test)]
pub fn get_tracked_processes() -> Vec<u32> {
    if let Ok(processes) = EMULATION_PROCESSES.lock() {
        processes.clone()
    } else {
        vec![]
    }
}
