use crate::runtime::container::{ContainerError, ContainerOutput, ContainerRuntime};
use async_trait::async_trait;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct EmulationRuntime {
    workspace: TempDir,
}

impl EmulationRuntime {
    pub fn new() -> Self {
        // Create a temporary workspace to simulate container isolation
        let workspace =
            tempfile::tempdir().expect("Failed to create temporary workspace for emulation");

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
                    for entry in fs::read_dir(host_path).expect("Failed to read source directory") {
                        if let Ok(entry) = entry {
                            let source = entry.path();
                            let dest = target_path.join(source.file_name().unwrap());

                            if source.is_file() {
                                fs::copy(&source, &dest).expect("Failed to copy file");
                            } else {
                                // We could make this recursive if needed
                                fs::create_dir_all(&dest).expect("Failed to create subdirectory");
                            }
                        }
                    }
                }
            } else if host_path.is_file() {
                // Copy individual file
                fs::copy(host_path, &target_path).expect("Failed to copy file");
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
        println!("🔄 Emulating container: {}", image);

        // Prepare the workspace
        let container_working_dir = self.prepare_workspace(working_dir, volumes);

        // For Nix-specific commands, ensure Nix is installed
        let contains_nix_command = cmd.iter().any(|&arg| arg.contains("nix "));

        if contains_nix_command {
            println!("🔄 Emulation: Detected Nix command, checking if Nix is installed");

            let nix_installed = Command::new("which")
                .arg("nix")
                .output()
                .map(|output| output.status.success())
                .unwrap_or(false);

            if !nix_installed {
                println!("⚠️ Nix commands detected but Nix is not installed!");
                println!(
                    "🔄 To use this workflow, please install Nix: https://nixos.org/download.html"
                );

                return Ok(ContainerOutput {
                        stdout: String::new(),
                        stderr: "Nix is required for this workflow but not installed on your system.\nPlease install Nix first: https://nixos.org/download.html".to_string(),
                        exit_code: 1,
                    });
            } else {
                println!("✅ Nix is installed, proceeding with command");
            }
        }

        // Ensure we have a command
        if cmd.is_empty() {
            return Err(ContainerError::ContainerExecutionFailed(
                "No command specified".to_string(),
            ));
        }

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

                    // Create command
                    let mut command = Command::new(shell);
                    command.current_dir(&container_working_dir);

                    // Add flags
                    for i in 1..idx + 1 {
                        command.arg(cmd[i]);
                    }

                    // Add the command
                    command.arg(command_str);

                    // Set environment variables
                    for (key, value) in env_vars {
                        command.env(key, value);
                    }

                    // Execute
                    let output = command
                        .output()
                        .map_err(|e| ContainerError::ContainerExecutionFailed(e.to_string()))?;

                    return Ok(ContainerOutput {
                        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                        exit_code: output.status.code().unwrap_or(-1),
                    });
                }
            }
        }

        // For all other commands
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

        // Execute
        let output = command
            .output()
            .map_err(|e| ContainerError::ContainerExecutionFailed(e.to_string()))?;

        Ok(ContainerOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    async fn pull_image(&self, image: &str) -> Result<(), ContainerError> {
        println!("🔄 Emulation: Pretending to pull image {}", image);
        Ok(())
    }

    async fn build_image(&self, dockerfile: &Path, tag: &str) -> Result<(), ContainerError> {
        println!(
            "🔄 Emulation: Pretending to build image {} from {}",
            tag,
            dockerfile.display()
        );
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
        let file_name = path.file_name().unwrap();
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
    if action.starts_with("cachix/install-nix-action") {
        println!("🔄 Emulating cachix/install-nix-action");

        // In emulation mode, check if nix is installed
        let nix_installed = Command::new("which")
            .arg("nix")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false);

        if !nix_installed {
            println!("🔄 Emulation: Nix is required but not installed.");
            println!(
                "🔄 To use this workflow, please install Nix: https://nixos.org/download.html"
            );
            println!("🔄 Continuing emulation, but nix commands will fail.");
        } else {
            println!("🔄 Emulation: Using system-installed Nix");
        }
        Ok(())
    } else {
        // Ignore other actions in emulation mode
        Ok(())
    }
}
