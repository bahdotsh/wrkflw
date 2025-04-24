use crate::logging;
use crate::runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json;
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
        image: &str,
        cmd: &[&str],
        env_vars: &[(&str, &str)],
        working_dir: &Path,
        volumes: &[(&Path, &Path)],
    ) -> Result<ContainerOutput, ContainerError> {
        // Print emulation info
        logging::info(&format!("Emulating container: {}", image));
        logging::debug(&format!("Command: {:?}", cmd));

        // Prepare the workspace
        let container_working_dir = self.prepare_workspace(working_dir, volumes);

        // Check if this is likely a Rust job (cargo command)
        let is_rust_job = cmd
            .iter()
            .any(|&c| c.contains("cargo") || c.contains("rustc") || c.contains("rustup"));

        // Check if we need to ensure Rust toolchain paths are properly set up
        if is_rust_job {
            logging::info("Rust toolchain detected, ensuring proper setup...");

            // Create a local .rustup directory if needed
            let local_rustup = container_working_dir.join(".rustup");
            if !local_rustup.exists() {
                std::fs::create_dir_all(&local_rustup).map_err(|e| {
                    ContainerError::ContainerExecution(format!(
                        "Failed to create .rustup directory: {}",
                        e
                    ))
                })?;
            }

            // Create a local .cargo directory if needed
            let local_cargo = container_working_dir.join(".cargo");
            if !local_cargo.exists() {
                std::fs::create_dir_all(&local_cargo).map_err(|e| {
                    ContainerError::ContainerExecution(format!(
                        "Failed to create .cargo directory: {}",
                        e
                    ))
                })?;
            }
        }

        // Detect if this is a long-running command that should be spawned as a detached process
        let is_long_running = cmd.iter().any(|&c| {
            c.contains("server")
                || c.contains("daemon")
                || c.contains("listen")
                || c.contains("watch")
                || c.contains("-d")
                || c.contains("--detach")
        });

        if is_long_running {
            logging::info("Detected long-running command, will run detached");

            let mut command = Command::new(cmd[0]);
            command.current_dir(&container_working_dir);

            // Add all arguments
            for arg in &cmd[1..] {
                command.arg(arg);
            }

            // Set environment variables
            for (key, value) in env_vars {
                command.env(key, value);
            }

            // Run detached
            match command.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    track_process(pid);
                    logging::info(&format!("Started detached process with PID: {}", pid));

                    return Ok(ContainerOutput {
                        stdout: format!("Started long-running process with PID: {}", pid),
                        stderr: String::new(),
                        exit_code: 0,
                    });
                }
                Err(e) => {
                    return Err(ContainerError::ContainerExecution(format!(
                        "Failed to start detached process: {}",
                        e
                    )));
                }
            }
        }

        // For Nix-specific commands, ensure Nix is installed
        let contains_nix_command = cmd.iter().any(|&arg| arg.contains("nix "));

        if contains_nix_command {
            let nix_installed = Command::new("which")
                .arg("nix")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false);

            if !nix_installed {
                logging::info("⚠️ Nix commands detected but Nix is not installed!");
                logging::info(
                    "🔄 To use this workflow, please install Nix: https://nixos.org/download.html",
                );

                return Ok(ContainerOutput {
                        stdout: String::new(),
                        stderr: "Nix is required for this workflow but not installed on your system.\nPlease install Nix first: https://nixos.org/download.html".to_string(),
                        exit_code: 1,
                    });
            } else {
                logging::info("✅ Nix is installed, proceeding with command");
            }
        }

        // Create a map of environment variables for easy manipulation
        let mut env_map: std::collections::HashMap<String, String> = env_vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        // Check if any of the commands suggest a specific action
        let mut detected_action = None;
        let mut with_params = None;

        // Extract possible action info from comments in environment variables
        for (key, value) in &env_map {
            if key == "__WRKFLW_ACTION" {
                detected_action = Some(value.clone());
            }
            if key == "__WRKFLW_WITH_PARAMS" {
                // This would be a JSON-encoded string of with parameters
                if let Ok(params) = serde_json::from_str::<HashMap<String, String>>(value) {
                    with_params = Some(params);
                }
            }
        }

        // Apply action-specific environment variables if detected
        if let Some(action) = detected_action {
            add_action_env_vars(&mut env_map, &action, &with_params);
        }

        // For Rust jobs, set up RUSTUP_HOME and CARGO_HOME to point to local directories
        if is_rust_job {
            let rustup_home = container_working_dir
                .join(".rustup")
                .to_string_lossy()
                .to_string();
            let cargo_home = container_working_dir
                .join(".cargo")
                .to_string_lossy()
                .to_string();

            env_map.insert("RUSTUP_HOME".to_string(), rustup_home);
            env_map.insert("CARGO_HOME".to_string(), cargo_home);
            logging::info(&format!(
                "Setting RUSTUP_HOME={}",
                env_map.get("RUSTUP_HOME").unwrap()
            ));
            logging::info(&format!(
                "Setting CARGO_HOME={}",
                env_map.get("CARGO_HOME").unwrap()
            ));
        }

        // Ensure we have a command
        if cmd.is_empty() {
            return Err(ContainerError::ContainerExecution(
                "No command specified".to_string(),
            ));
        }

        let has_background = cmd.iter().any(|c| c.contains(" &"));

        // For bash/sh with -c, handle specially
        if (cmd[0] == "bash" || cmd[0] == "sh")
            && cmd.len() >= 2
            && (cmd[1] == "-c" || cmd[1] == "-e" || cmd[1] == "-ec")
        {
            let shell = cmd[0];

            // Find the index of -c flag (could be -e -c or just -c)
            let c_flag_index = cmd.iter().position(|&arg| arg == "-c");

            if let Some(idx) = c_flag_index {
                // Ensure there's an argument after -c
                if idx + 1 < cmd.len() {
                    // Get the actual command
                    let command_str = cmd[idx + 1];

                    // Handle GitHub variables properly
                    let fixed_cmd = command_str
                        .replace(">>$GITHUB_OUTPUT", ">>\"$GITHUB_OUTPUT\"")
                        .replace(">>$GITHUB_ENV", ">>\"$GITHUB_ENV\"")
                        .replace(">>$GITHUB_PATH", ">>\"$GITHUB_PATH\"")
                        .replace(">>$GITHUB_STEP_SUMMARY", ">>\"$GITHUB_STEP_SUMMARY\"");

                    // If we have background processes, add a wait command
                    let final_cmd = if has_background && !fixed_cmd.contains(" wait") {
                        format!("{{ {}; }} && wait", fixed_cmd)
                    } else {
                        fixed_cmd
                    };

                    // Store a copy for logging
                    let cmd_for_logs = final_cmd.clone();

                    // Create command
                    let mut command = Command::new(shell);
                    command.current_dir(&container_working_dir);

                    // Add flags
                    for arg in cmd.iter().skip(1).take(idx) {
                        command.arg(arg);
                    }

                    // Add the command
                    command.arg(final_cmd);

                    // Set environment variables from the map
                    for (key, value) in &env_map {
                        command.env(key, value);
                    }

                    // Execute
                    let output = command
                        .output()
                        .map_err(|e| ContainerError::ContainerExecution(e.to_string()))?;

                    // Log detailed information about the command execution for debugging
                    let exit_code = output.status.code().unwrap_or(-1);
                    if exit_code != 0 {
                        logging::info(&format!("Command failed with exit code: {}", exit_code));
                        logging::debug(&format!("Failed command: {}", cmd_for_logs));
                        logging::debug(&format!(
                            "Working directory: {}",
                            container_working_dir.display()
                        ));
                        logging::debug(&format!(
                            "STDERR: {}",
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }

                    return Ok(ContainerOutput {
                        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                        exit_code,
                    });
                }
            }
        }

        if has_background {
            // For commands with background processes, use shell wrapper
            let mut shell_command = Command::new("sh");
            shell_command.current_dir(&container_working_dir);
            shell_command.arg("-c");

            // Join the original command and add trap for cleanup
            let command_str = format!("{{ {}; }} && wait", cmd.join(" "));

            // Store a copy for logging
            let cmd_for_logs = command_str.clone();

            shell_command.arg(command_str);

            // Set environment variables from the map
            for (key, value) in &env_map {
                shell_command.env(key, value);
            }

            // Log that we're running a background process
            logging::info("Emulation: Running command with background processes");

            // For commands with background processes, we could potentially track PIDs
            // However, since they're in a shell wrapper, we'd need to parse them from output

            let output = shell_command
                .output()
                .map_err(|e| ContainerError::ContainerExecution(e.to_string()))?;

            // Log detailed information about the command execution for debugging
            let exit_code = output.status.code().unwrap_or(-1);
            if exit_code != 0 {
                logging::info(&format!(
                    "Background command failed with exit code: {}",
                    exit_code
                ));
                logging::debug(&format!("Failed command: {}", cmd_for_logs));
                logging::debug(&format!(
                    "Working directory: {}",
                    container_working_dir.display()
                ));
                logging::debug(&format!(
                    "STDERR: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }

            return Ok(ContainerOutput {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code,
            });
        }

        // For all other commands
        let mut command = Command::new(cmd[0]);
        command.current_dir(&container_working_dir);

        // Log the command we're about to run
        logging::debug(&format!("Executing command: {}", cmd.join(" ")));
        logging::debug(&format!(
            "In directory: {}",
            container_working_dir.display()
        ));

        // Add all arguments
        for arg in &cmd[1..] {
            command.arg(arg);
        }

        // Set environment variables from the map
        for (key, value) in &env_map {
            command.env(key, value);
        }

        // Execute
        let output = command
            .output()
            .map_err(|e| ContainerError::ContainerExecution(e.to_string()))?;

        // Log detailed information about the command execution for debugging
        let exit_code = output.status.code().unwrap_or(-1);
        if exit_code != 0 {
            logging::info(&format!("Command failed with exit code: {}", exit_code));
            logging::debug(&format!("Failed command: {:?}", cmd));
            logging::debug(&format!(
                "Working directory: {}",
                container_working_dir.display()
            ));
            logging::debug(&format!(
                "STDERR: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        Ok(ContainerOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code,
        })
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

// Helper function for recursive directory copying
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
