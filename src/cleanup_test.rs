#[cfg(test)]
mod cleanup_tests {
    use crate::{
        cleanup_on_exit,
        executor::docker,
        runtime::emulation::{self, EmulationRuntime},
    };
    use bollard::Docker;
    use std::process::Command;

    #[tokio::test]
    async fn test_docker_container_cleanup() {
        // Skip if Docker is not available
        if !docker::is_available() {
            println!("Docker not available, skipping test");
            return;
        }

        // Connect to Docker
        let docker = match Docker::connect_with_local_defaults() {
            Ok(client) => client,
            Err(_) => {
                println!("Could not connect to Docker, skipping test");
                return;
            }
        };

        // Create a test container by tracking it
        let container_id = format!("test-container-{}", uuid::Uuid::new_v4());
        docker::track_container(&container_id);

        // Verify container is tracked
        let containers = docker::get_tracked_containers();
        let is_tracked = containers.contains(&container_id);

        assert!(is_tracked, "Container should be tracked for cleanup");

        // Run cleanup
        docker::cleanup_containers(&docker).await;

        // Verify container is no longer tracked
        let containers = docker::get_tracked_containers();
        let still_tracked = containers.contains(&container_id);

        assert!(
            !still_tracked,
            "Container should be removed from tracking after cleanup"
        );
    }

    #[tokio::test]
    async fn test_docker_network_cleanup() {
        // Skip if Docker is not available
        if !docker::is_available() {
            println!("Docker not available, skipping test");
            return;
        }

        // Connect to Docker
        let docker = match Docker::connect_with_local_defaults() {
            Ok(client) => client,
            Err(_) => {
                println!("Could not connect to Docker, skipping test");
                return;
            }
        };

        // Create a test network
        let network_id = match docker::create_job_network(&docker).await {
            Ok(id) => id,
            Err(_) => {
                println!("Could not create test network, skipping test");
                return;
            }
        };

        // Verify network is tracked
        let networks = docker::get_tracked_networks();
        let is_tracked = networks.contains(&network_id);

        assert!(is_tracked, "Network should be tracked for cleanup");

        // Run cleanup
        docker::cleanup_networks(&docker).await;

        // Verify network is no longer tracked
        let networks = docker::get_tracked_networks();
        let still_tracked = networks.contains(&network_id);

        assert!(
            !still_tracked,
            "Network should be removed from tracking after cleanup"
        );
    }

    #[tokio::test]
    async fn test_emulation_workspace_cleanup() {
        // Create an emulation runtime instance
        let _runtime = EmulationRuntime::new();

        // Get the workspace path
        let workspaces = emulation::get_tracked_workspaces();
        if workspaces.is_empty() {
            println!("No workspace was tracked, skipping test");
            return;
        }

        let workspace_path = &workspaces[0];

        // Verify workspace exists
        assert!(
            workspace_path.exists(),
            "Workspace should exist before cleanup"
        );

        // Run cleanup
        emulation::cleanup_resources().await;

        // Verify workspace is removed from tracking
        let workspaces = emulation::get_tracked_workspaces();
        let still_tracked = workspaces.iter().any(|w| w == workspace_path);

        assert!(
            !still_tracked,
            "Workspace should be removed from tracking after cleanup"
        );

        // Verify workspace directory is deleted
        assert!(
            !workspace_path.exists(),
            "Workspace directory should be deleted after cleanup"
        );
    }

    #[tokio::test]
    async fn test_emulation_process_cleanup() {
        // Skip tests on CI or environments where spawning processes might be restricted
        if std::env::var("CI").is_ok() {
            println!("Running in CI environment, skipping test");
            return;
        }

        // Create a process for testing
        let process_id = if cfg!(unix) {
            // Use sleep on Unix to create a long-running process
            let child = Command::new("sh")
                .arg("-c")
                .arg("sleep 30 &") // Run sleep for 30 seconds in background
                .spawn();

            match child {
                Ok(child) => {
                    // Get the PID and track it
                    let pid = child.id();
                    emulation::track_process(pid);
                    Some(pid)
                }
                Err(_) => None,
            }
        } else if cfg!(windows) {
            // Use timeout on Windows (equivalent to sleep)
            let child = Command::new("cmd")
                .arg("/C")
                .arg("start /b timeout /t 30") // Run timeout for 30 seconds
                .spawn();

            match child {
                Ok(child) => {
                    // Get the PID and track it
                    let pid = child.id();
                    emulation::track_process(pid);
                    Some(pid)
                }
                Err(_) => None,
            }
        } else {
            None
        };

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

        assert!(
            !still_tracked,
            "Process should be removed from tracking after cleanup"
        );
    }

    #[tokio::test]
    async fn test_cleanup_on_exit_function() {
        // Skip test on CI where we may not have permission
        if std::env::var("CI").is_ok() {
            println!("Running in CI environment, skipping test");
            return;
        }

        // Create Docker resources if available
        let docker_client = match Docker::connect_with_local_defaults() {
            Ok(client) => {
                // Create a network
                let _ = docker::create_job_network(&client).await;
                Some(client)
            }
            Err(_) => None,
        };

        // Create an emulation runtime to track a workspace
        let _runtime = EmulationRuntime::new();

        // Create a process to track in emulation mode
        if cfg!(unix) {
            let child = Command::new("sh").arg("-c").arg("sleep 30 &").spawn();

            if let Ok(child) = child {
                emulation::track_process(child.id());
            }
        }

        // Count initial resource tracking
        let docker_resources = if docker_client.is_some() {
            let containers = docker::get_tracked_containers().len();
            let networks = docker::get_tracked_networks().len();
            containers + networks
        } else {
            0
        };

        let emulation_resources = {
            let processes = emulation::get_tracked_processes().len();
            let workspaces = emulation::get_tracked_workspaces().len();
            processes + workspaces
        };

        // Verify we have resources to clean up
        let total_resources = docker_resources + emulation_resources;
        if total_resources == 0 {
            println!("No resources were created for testing, skipping test");
            return;
        }

        // Run the main cleanup function
        cleanup_on_exit().await;

        // Add a small delay to ensure async cleanup operations complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Check if Docker resources were cleaned up
        let docker_resources_after = if docker_client.is_some() {
            let containers = docker::get_tracked_containers().len();
            let networks = docker::get_tracked_networks().len();
            containers + networks
        } else {
            0
        };

        // Check if emulation resources were cleaned up
        let emulation_resources_after = {
            let processes = emulation::get_tracked_processes().len();
            let workspaces = emulation::get_tracked_workspaces().len();
            processes + workspaces
        };

        // Verify all resources were cleaned up
        assert_eq!(
            docker_resources_after, 0,
            "All Docker resources should be cleaned up"
        );
        assert_eq!(
            emulation_resources_after, 0,
            "All emulation resources should be cleaned up"
        );
    }
}
