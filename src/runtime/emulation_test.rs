use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use tokio::sync::Mutex;
use once_cell::sync::Lazy;

use crate::runtime::{
    container::{ContainerRuntime, ContainerOutput, ContainerError},
    emulation::{self, EmulationRuntime},
};

#[cfg(test)]
mod emulation_cleanup_tests {
    use super::*;

    /// Create a process and workspace that need to be tracked for cleanup
    async fn setup_emulation_resources() -> (Option<u32>, Option<PathBuf>) {
        // Create an emulation runtime to generate a workspace
        let runtime = EmulationRuntime::new();
        
        // Get the workspace path (normally this is tracked automatically)
        let workspaces = emulation::get_tracked_workspaces();
        let workspace_path = if !workspaces.is_empty() {
            Some(workspaces[0].clone())
        } else {
            None
        };
        
        // Try to spawn a long-running background process for testing
        let process_id = if cfg!(unix) {
            // Use sleep on Unix to create a long-running process
            let child = Command::new("sh")
                .arg("-c")
                .arg("sleep 300 &")  // Run sleep for 300 seconds in background
                .spawn();
                
            match child {
                Ok(child) => {
                    // Get the PID and track it
                    let pid = child.id();
                    emulation::track_process(pid);
                    Some(pid)
                },
                Err(_) => None
            }
        } else if cfg!(windows) {
            // Use timeout on Windows (equivalent to sleep)
            let child = Command::new("cmd")
                .arg("/C")
                .arg("start /b timeout /t 300")  // Run timeout for 300 seconds
                .spawn();
                
            match child {
                Ok(child) => {
                    // Get the PID and track it
                    let pid = child.id();
                    emulation::track_process(pid);
                    Some(pid)
                },
                Err(_) => None
            }
        } else {
            None
        };
        
        (process_id, workspace_path)
    }

    /// Check if a process with the given PID is still running
    fn is_process_running(pid: u32) -> bool {
        if cfg!(unix) {
            // On Unix, use kill -0 to check if process exists
            let output = Command::new("kill")
                .arg("-0")
                .arg(&pid.to_string())
                .output();
                
            matches!(output, Ok(output) if output.status.success())
        } else if cfg!(windows) {
            // On Windows, use tasklist to find the process
            let output = Command::new("tasklist")
                .arg("/FI")
                .arg(format!("PID eq {}", pid))
                .arg("/NH")
                .output();
                
            matches!(output, Ok(output) if String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
        } else {
            false
        }
    }

    #[tokio::test]
    async fn test_emulation_process_cleanup() {
        // Skip tests on CI or environments where spawning processes might be restricted
        if std::env::var("CI").is_ok() {
            println!("Running in CI environment, skipping test");
            return;
        }
        
        // Set up test resources
        let (process_id, _) = setup_emulation_resources().await;
        
        // Skip if we couldn't create a process
        let process_id = match process_id {
            Some(id) => id,
            None => {
                println!("Could not create test process, skipping test");
                return;
            }
        };
        
        // Verify process is tracked
        let processes = emulation::get_tracked_processes();
        let is_tracked = processes.contains(&process_id);
        
        assert!(is_tracked, "Process should be tracked for cleanup");
        
        // Run cleanup
        emulation::cleanup_resources().await;
        
        // Verify process is removed from tracking
        let processes = emulation::get_tracked_processes();
        let still_tracked = processes.contains(&process_id);
        
        assert!(!still_tracked, "Process should be removed from tracking after cleanup");
        
        // Verify process is no longer running
        assert!(!is_process_running(process_id), "Process should be terminated after cleanup");
    }

    #[tokio::test]
    async fn test_emulation_workspace_cleanup() {
        // Create an emulation runtime instance which will automatically create and track a workspace
        let runtime = EmulationRuntime::new();
        
        // Get the workspace path 
        let workspaces = emulation::get_tracked_workspaces();
        if workspaces.is_empty() {
            println!("No workspace was tracked, skipping test");
            return;
        }
        
        let workspace_path = &workspaces[0];
        
        // Verify workspace exists
        assert!(workspace_path.exists(), "Workspace should exist before cleanup");
        
        // Run cleanup
        emulation::cleanup_resources().await;
        
        // Verify workspace is removed from tracking
        let workspaces = emulation::get_tracked_workspaces();
        let still_tracked = workspaces.iter().any(|w| w == workspace_path);
        
        assert!(!still_tracked, "Workspace should be removed from tracking after cleanup");
        
        // Verify workspace directory is deleted
        assert!(!workspace_path.exists(), "Workspace directory should be deleted after cleanup");
    }

    #[tokio::test]
    async fn test_run_container_with_emulation() {
        // Create an emulation runtime
        let runtime = EmulationRuntime::new();
        
        // Run a simple command in emulation mode
        let result = runtime
            .run_container(
                "alpine:latest",  // In emulation mode, image is just for logging
                &["echo", "test cleanup"],
                &[],
                Path::new("/"),
                &[(Path::new("."), Path::new("/github/workspace"))],
            )
            .await;
            
        // Verify command executed successfully
        match result {
            Ok(output) => {
                assert!(output.stdout.contains("test cleanup"), "Command output should contain test message");
                assert_eq!(output.exit_code, 0, "Command should exit with status 0");
            },
            Err(e) => {
                panic!("Failed to run command in emulation mode: {}", e);
            }
        }
        
        // Count resources before cleanup
        let workspaces_count = emulation::get_tracked_workspaces().len();
        
        assert!(workspaces_count > 0, "At least one workspace should be tracked");
        
        // Run cleanup
        emulation::cleanup_resources().await;
        
        // Verify all resources are cleaned up
        let remaining_workspaces = emulation::get_tracked_workspaces().len();
        
        assert_eq!(remaining_workspaces, 0, "All workspaces should be cleaned up");
    }

    #[tokio::test]
    async fn test_full_resource_cleanup() {
        // Skip tests on CI or environments where spawning processes might be restricted
        if std::env::var("CI").is_ok() {
            println!("Running in CI environment, skipping test");
            return;
        }
        
        // Set up test resources
        let (process_id, _) = setup_emulation_resources().await;
        
        // Create an additional emulation runtime to have more workspaces
        let runtime = EmulationRuntime::new();
        
        // Count resources before cleanup
        let process_count = emulation::get_tracked_processes().len();
        let workspace_count = emulation::get_tracked_workspaces().len();
        
        // Ensure we have at least one resource to clean up
        assert!(process_count > 0 || workspace_count > 0, 
               "At least one process or workspace should be tracked");
        
        // Run full cleanup
        emulation::cleanup_resources().await;
        
        // Verify all resources are cleaned up
        let remaining_processes = emulation::get_tracked_processes().len();
        let remaining_workspaces = emulation::get_tracked_workspaces().len();
        
        assert_eq!(remaining_processes, 0, "All processes should be cleaned up");
        assert_eq!(remaining_workspaces, 0, "All workspaces should be cleaned up");
        
        // If we had a process, verify it's not running anymore
        if let Some(pid) = process_id {
            assert!(!is_process_running(pid), "Process should be terminated after cleanup");
        }
    }
} 