pub mod evaluator;
pub mod executor;
pub mod github;
pub mod gitlab;
pub mod logging;
pub mod matrix;
pub mod models;
pub mod parser;
pub mod runtime;
pub mod ui;
pub mod utils;
pub mod validators;

use bollard::Docker;

/// Clean up all resources when exiting the application
/// This is used by both main.rs and in tests
pub async fn cleanup_on_exit() {
    // Clean up Docker resources if available, but don't let it block indefinitely
    match tokio::time::timeout(std::time::Duration::from_secs(3), async {
        match Docker::connect_with_local_defaults() {
            Ok(docker) => {
                let _ = executor::docker::cleanup_containers(&docker).await;
                let _ = executor::docker::cleanup_networks(&docker).await;
            }
            Err(_) => {
                // Docker not available
                logging::info("Docker not available, skipping Docker cleanup");
            }
        }
    })
    .await
    {
        Ok(_) => logging::debug("Docker cleanup completed successfully"),
        Err(_) => {
            logging::warning("Docker cleanup timed out after 3 seconds, continuing with shutdown")
        }
    }

    // Always clean up emulation resources
    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        runtime::emulation::cleanup_resources(),
    )
    .await
    {
        Ok(_) => logging::debug("Emulation cleanup completed successfully"),
        Err(_) => logging::warning("Emulation cleanup timed out, continuing with shutdown"),
    }

    logging::info("Resource cleanup completed");
}
