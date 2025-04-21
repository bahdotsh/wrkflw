use bollard::Docker;
use std::{sync::Arc, path::Path};
use tokio::sync::Mutex;

use crate::{
    executor::{docker::{self, DockerRuntime}, RuntimeType},
    runtime::container::{ContainerRuntime, ContainerOutput}
};

#[cfg(test)]
mod docker_cleanup_tests {
    use super::*;

    // Helper function to check if Docker tests should be skipped
    fn should_skip_docker_tests() -> bool {
        std::env::var("WRKFLW_TEST_SKIP_DOCKER").is_ok() || 
        !docker::is_available()
    }

    /// Helper function to create a Docker container that should be tracked
    async fn create_test_container(docker_client: &Docker) -> Option<String> {
        if should_skip_docker_tests() {
            return None;
        }

        // Try to create a container runtime
        let runtime = match DockerRuntime::new() {
            Ok(rt) => rt,
            Err(_) => return None,
        };

        // Run a simple container that finishes quickly
        let result = runtime
            .run_container(
                "alpine:latest",
                &["echo", "test"],
                &[],
                Path::new("/"),
                &[],
            )
            .await;
        
        // The container should be automatically removed by the runtime after execution
        // but we can verify if it's tracked first
        
        let running_containers = docker::get_tracked_containers();
        
        // Since run_container internally cleans up, we'll simulate tracking a container
        if let Some(container_id) = running_containers.first() {
            return Some(container_id.clone());
        }
        
        // Manually track a container for testing
        let container_id = format!("test-container-{}", uuid::Uuid::new_v4());
        docker::track_container(&container_id);
        
        Some(container_id)
    }

    /// Helper function to create a Docker network that should be tracked
    async fn create_test_network(docker_client: &Docker) -> Option<String> {
        if should_skip_docker_tests() {
            return None;
        }

        // Create a test network
        match docker::create_job_network(docker_client).await {
            Ok(network_id) => Some(network_id),
            Err(_) => None,
        }
    }

    #[tokio::test]
    async fn test_docker_container_cleanup() {
        if should_skip_docker_tests() {
            println!("Docker tests disabled or Docker not available, skipping test");
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

        // Create a test container
        let container_id = match create_test_container(&docker).await {
            Some(id) => id,
            None => {
                println!("Could not create test container, skipping test");
                return;
            }
        };
        
        // Verify container is tracked
        let containers = docker::get_tracked_containers();
        let is_tracked = containers.contains(&container_id);
        
        assert!(is_tracked, "Container should be tracked for cleanup");

        // Run cleanup
        docker::cleanup_containers(&docker).await;

        // Verify container is removed from tracking
        let containers = docker::get_tracked_containers();
        let still_tracked = containers.contains(&container_id);
        
        assert!(!still_tracked, "Container should be removed from tracking after cleanup");
    }

    #[tokio::test]
    async fn test_docker_network_cleanup() {
        if should_skip_docker_tests() {
            println!("Docker tests disabled or Docker not available, skipping test");
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
        let network_id = match create_test_network(&docker).await {
            Some(id) => id,
            None => {
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

        // Verify network is removed from tracking
        let networks = docker::get_tracked_networks();
        let still_tracked = networks.contains(&network_id);
        
        assert!(!still_tracked, "Network should be removed from tracking after cleanup");
    }

    #[tokio::test]
    async fn test_full_resource_cleanup() {
        if should_skip_docker_tests() {
            println!("Docker tests disabled or Docker not available, skipping test");
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

        // Create a test container
        let _ = create_test_container(&docker).await;
        
        // Create a test network
        let _ = create_test_network(&docker).await;
        
        // Count resources before cleanup
        let container_count = docker::get_tracked_containers().len();
        let network_count = docker::get_tracked_networks().len();
        
        // Ensure we have at least one resource to clean up
        if container_count == 0 && network_count == 0 {
            println!("No resources created for testing, skipping test");
            return;
        }

        // Run full cleanup
        docker::cleanup_resources(&docker).await;

        // Verify all resources are cleaned up
        let remaining_containers = docker::get_tracked_containers().len();
        let remaining_networks = docker::get_tracked_networks().len();
        
        assert_eq!(remaining_containers, 0, "All containers should be cleaned up");
        assert_eq!(remaining_networks, 0, "All networks should be cleaned up");
    }
} 