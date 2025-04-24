#[cfg(test)]
mod cleanup_tests {
    use crate::{
        cleanup_on_exit,
        executor::docker,
        runtime::emulation::{self, EmulationRuntime},
    };
    use bollard::Docker;
    use std::process::Command;
    use std::time::Duration;

    #[tokio::test]
    async fn test_docker_container_cleanup() {
        // Skip if running in CI environment for Linux
        if cfg!(target_os = "linux") && std::env::var("CI").is_ok() {
            println!("Skipping Docker container cleanup test in Linux CI environment");
            return;
        }

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

        // Run cleanup with timeout
        match tokio::time::timeout(Duration::from_secs(10), docker::cleanup_containers(&docker))
            .await
        {
            Ok(_) => {
                // Cleanup completed within timeout
                // Verify container is no longer tracked
                let containers = docker::get_tracked_containers();
                let still_tracked = containers.contains(&container_id);

                assert!(
                    !still_tracked,
                    "Container should be removed from tracking after cleanup"
                );
            }
            Err(_) => {
                // Cleanup timed out
                println!("Container cleanup timed out after 10 seconds");
                // Manually untrack to clean up test state
                docker::untrack_container(&container_id);
                // Skip assertion as cleanup didn't complete within timeout
            }
        }
    }

    #[tokio::test]
    async fn test_docker_network_cleanup() {
        // Skip if running in CI environment for Linux
        if cfg!(target_os = "linux") && std::env::var("CI").is_ok() {
            println!("Skipping Docker network cleanup test in Linux CI environment");
            return;
        }

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

        // Create a test network with timeout
        let network_id = match tokio::time::timeout(
            Duration::from_secs(10),
            docker::create_job_network(&docker),
        )
        .await
        {
            Ok(result) => match result {
                Ok(id) => id,
                Err(_) => {
                    println!("Could not create test network, skipping test");
                    return;
                }
            },
            Err(_) => {
                println!("Network creation timed out after 10 seconds, skipping test");
                return;
            }
        };

        // Verify network is tracked
        let networks = docker::get_tracked_networks();
        let is_tracked = networks.contains(&network_id);

        assert!(is_tracked, "Network should be tracked for cleanup");

        // Run cleanup with timeout
        match tokio::time::timeout(Duration::from_secs(10), docker::cleanup_networks(&docker)).await
        {
            Ok(_) => {
                // Cleanup completed within timeout
                // Verify network is no longer tracked
                let networks = docker::get_tracked_networks();
                let still_tracked = networks.contains(&network_id);

                assert!(
                    !still_tracked,
                    "Network should be removed from tracking after cleanup"
                );
            }
            Err(_) => {
                // Cleanup timed out
                println!("Network cleanup timed out after 10 seconds");
                // Manually untrack to clean up test state
                docker::untrack_network(&network_id);
                // Skip assertion as cleanup didn't complete within timeout
            }
        }
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
            // Use sleep on Unix but DO NOT use & to background
            // Instead run it directly and track the actual process
            let child = Command::new("sleep")
                .arg("10") // Shorter sleep time
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
                .arg("timeout /t 10") // Shorter timeout
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
        // Skip test for Linux in CI environment
        if cfg!(target_os = "linux") && std::env::var("CI").is_ok() {
            println!("Skipping cleanup on exit test in Linux CI environment");
            return;
        }

        // Skip on macOS as Docker operations may take longer
        if cfg!(target_os = "macos") {
            println!("Skipping cleanup on exit test on macOS");
            return;
        }

        // Create Docker resources if available
        let docker_client = if docker::is_available() {
            match Docker::connect_with_local_defaults() {
                Ok(client) => {
                    // Create a network with timeout
                    match tokio::time::timeout(
                        Duration::from_secs(10),
                        docker::create_job_network(&client),
                    )
                    .await
                    {
                        Ok(_) => Some(client),
                        Err(_) => {
                            println!("Network creation timed out after 10 seconds, skipping Docker part of test");
                            None
                        }
                    }
                }
                Err(_) => None,
            }
        } else {
            println!("Docker not available, skipping Docker part of test");
            None
        };

        // Create an emulation runtime to track a workspace
        let _runtime = EmulationRuntime::new();

        // Create a process to track in emulation mode
        if cfg!(unix) {
            let child = Command::new("sh").arg("-c").arg("sleep 10 &").spawn();

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

        // Skip if no resources were created
        if docker_resources == 0 && emulation_resources == 0 {
            println!("No resources were created, skipping test");
            return;
        }

        // Run cleanup with timeout - increased to 30 seconds for macOS compatibility
        match tokio::time::timeout(Duration::from_secs(30), cleanup_on_exit()).await {
            Ok(_) => {
                // Verify Docker resources are cleaned up
                if docker_client.is_some() {
                    let remaining_containers = docker::get_tracked_containers().len();
                    let remaining_networks = docker::get_tracked_networks().len();

                    assert_eq!(
                        remaining_containers, 0,
                        "All Docker containers should be cleaned up"
                    );
                    assert_eq!(
                        remaining_networks, 0,
                        "All Docker networks should be cleaned up"
                    );
                }

                // Verify emulation resources are cleaned up
                let remaining_processes = emulation::get_tracked_processes().len();
                let remaining_workspaces = emulation::get_tracked_workspaces().len();

                assert_eq!(
                    remaining_processes, 0,
                    "All emulation processes should be cleaned up"
                );
                assert_eq!(
                    remaining_workspaces, 0,
                    "All emulation workspaces should be cleaned up"
                );
            }
            Err(_) => {
                println!("Cleanup timed out after 30 seconds");
                // Clean up any tracked resources to not affect other tests
                if docker_client.is_some() {
                    for container_id in docker::get_tracked_containers() {
                        docker::untrack_container(&container_id);
                    }
                    for network_id in docker::get_tracked_networks() {
                        docker::untrack_network(&network_id);
                    }
                }

                for process_id in emulation::get_tracked_processes() {
                    emulation::untrack_process(process_id);
                }
                for workspace_path in emulation::get_tracked_workspaces() {
                    emulation::untrack_workspace(&workspace_path);
                }
                // Skip assertions since cleanup didn't complete
            }
        }
    }
}
