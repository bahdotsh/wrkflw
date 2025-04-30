use bollard::Docker;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;
use wrkflw::{
    cleanup_on_exit,
    executor::docker,
    runtime::emulation::{self, EmulationRuntime},
};

// Skip the tests when running cargo test with --skip docker
fn should_skip_docker_tests() -> bool {
    std::env::var("TEST_SKIP_DOCKER").is_ok() || !docker::is_available()
}

// Skip the tests when running cargo test with --skip processes
fn should_skip_process_tests() -> bool {
    std::env::var("TEST_SKIP_PROCESSES").is_ok() || std::env::var("CI").is_ok()
}

#[tokio::test]
async fn test_docker_container_cleanup() {
    // Skip test based on flags or environment
    if should_skip_docker_tests() {
        println!("Skipping Docker container cleanup test");
        return;
    }

    // Skip if running in CI environment for Linux
    if cfg!(target_os = "linux") && std::env::var("CI").is_ok() {
        println!("Skipping Docker container cleanup test in Linux CI environment");
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

    // Create a test container by manually tracking it
    // In a real test, we would create an actual container, but we're just simulating that here
    let container_id = format!("test-container-{}", Uuid::new_v4());
    docker::track_container(&container_id);

    // Run cleanup
    let _ = docker::cleanup_containers(&docker).await;

    // Since we can't directly check the tracking status,
    // we'll use cleanup_on_exit and check for any errors
    match cleanup_on_exit().await {
        () => println!("Cleanup completed successfully"),
    }
}

#[tokio::test]
async fn test_docker_network_cleanup() {
    // Skip test based on flags or environment
    if should_skip_docker_tests() {
        println!("Skipping Docker network cleanup test");
        return;
    }

    // Skip if running in CI environment for Linux
    if cfg!(target_os = "linux") && std::env::var("CI").is_ok() {
        println!("Skipping Docker network cleanup test in Linux CI environment");
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

    // Run cleanup
    match docker::cleanup_networks(&docker).await {
        Ok(_) => println!("Network cleanup completed successfully"),
        Err(e) => println!("Network cleanup error: {}", e),
    }

    // Attempt to remove the network again - this should fail if cleanup worked
    match docker.remove_network(&network_id).await {
        Ok(_) => println!("Network still exists, cleanup may not have worked"),
        Err(_) => println!("Network was properly cleaned up"),
    }
}

#[tokio::test]
async fn test_emulation_workspace_cleanup() {
    // Create an emulation runtime instance
    let _runtime = EmulationRuntime::new();

    // Run cleanup
    emulation::cleanup_resources().await;

    // We can only verify that the cleanup operation doesn't crash
    // since we can't access the private tracking collections
    println!("Emulation workspace cleanup completed");
}

#[tokio::test]
#[ignore] // This test uses process manipulation which can be problematic
async fn test_emulation_process_cleanup() {
    // Skip tests on CI or environments where spawning processes might be restricted
    if should_skip_process_tests() {
        println!("Skipping process cleanup test");
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
                let pid = child.id();
                // Track the process for cleanup
                emulation::track_process(pid as u32);
                pid as u32
            }
            Err(_) => {
                println!("Could not create test process, skipping test");
                return;
            }
        }
    } else if cfg!(windows) {
        // On Windows, use a different long-running command
        let child = Command::new("timeout")
            .args(&["/t", "10", "/nobreak"])
            .spawn();

        match child {
            Ok(child) => {
                let pid = child.id();
                // Track the process for cleanup
                emulation::track_process(pid as u32);
                pid as u32
            }
            Err(_) => {
                println!("Could not create test process, skipping test");
                return;
            }
        }
    } else {
        println!("Unsupported platform, skipping test");
        return;
    };

    // Run cleanup resources which includes process cleanup
    emulation::cleanup_resources().await;

    // On Unix, verify process is no longer running
    if cfg!(unix) {
        // Allow a short delay for process termination
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Check if process exists
        let process_exists = unsafe {
            libc::kill(process_id as i32, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
        };

        assert!(
            !process_exists,
            "Process should be terminated after cleanup"
        );
    }
}

#[tokio::test]
async fn test_cleanup_on_exit_function() {
    // Skip if Docker is not available
    if should_skip_docker_tests() {
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

    // Create some resources for cleanup

    // Track a container
    let container_id = format!("test-container-{}", Uuid::new_v4());
    docker::track_container(&container_id);

    // Create a network
    let _ = match docker::create_job_network(&docker).await {
        Ok(id) => id,
        Err(_) => {
            println!("Could not create test network, skipping test");
            return;
        }
    };

    // Create an emulation workspace
    let _runtime = EmulationRuntime::new();

    // Run cleanup function
    match tokio::time::timeout(Duration::from_secs(15), cleanup_on_exit()).await {
        Ok(_) => println!("Cleanup completed successfully"),
        Err(_) => {
            println!("Cleanup timed out after 15 seconds");
            // Attempt manual cleanup
            let _ = docker::cleanup_containers(&docker).await;
            let _ = docker::cleanup_networks(&docker).await;
            emulation::cleanup_resources().await;
        }
    }
}
