// Copyright 2024 wrkflw contributors
// SPDX-License-Identifier: MIT

//! # wrkflw-secrets
//!
//! Comprehensive secrets management for wrkflw workflow execution.
//! Supports multiple secret providers and secure handling throughout the execution pipeline.
//!
//! ## Features
//!
//! - **Multiple Secret Providers**: Environment variables, file-based storage, with extensibility for cloud providers
//! - **Secret Substitution**: GitHub Actions-style secret references (`${{ secrets.SECRET_NAME }}`)
//! - **Automatic Masking**: Intelligent secret detection and masking in logs and output
//! - **Rate Limiting**: Built-in protection against secret access abuse
//! - **Caching**: Configurable caching for improved performance
//! - **Input Validation**: Comprehensive validation of secret names and values
//! - **Thread Safety**: Full async/await support with thread-safe operations
//!
//! ## Quick Start
//!
//! ```rust
//! use wrkflw_secrets::{SecretManager, SecretMasker, SecretSubstitution};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Initialize the secret manager with default configuration
//!     let manager = SecretManager::default().await?;
//!     
//!     // Set an environment variable for testing
//!     std::env::set_var("API_TOKEN", "secret_api_token_123");
//!     
//!     // Retrieve a secret
//!     let secret = manager.get_secret("API_TOKEN").await?;
//!     println!("Secret value: {}", secret.value());
//!     
//!     // Use secret substitution
//!     let mut substitution = SecretSubstitution::new(&manager);
//!     let template = "Using token: ${{ secrets.API_TOKEN }}";
//!     let resolved = substitution.substitute(template).await?;
//!     println!("Resolved: {}", resolved);
//!     
//!     // Set up secret masking
//!     let masker = SecretMasker::new();
//!     masker.add_secret("secret_api_token_123");
//!     
//!     let log_message = "Failed to authenticate with token: secret_api_token_123";
//!     let masked = masker.mask(log_message);
//!     println!("Masked: {}", masked); // Will show: "Failed to authenticate with token: se***123"
//!     
//!     // Clean up
//!     std::env::remove_var("API_TOKEN");
//!     Ok(())
//! }
//! ```
//!
//! ## Configuration
//!
//! ```rust
//! use wrkflw_secrets::{SecretConfig, SecretProviderConfig, SecretManager};
//! use std::collections::HashMap;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut providers = HashMap::new();
//!     
//!     // Environment variable provider with prefix
//!     providers.insert(
//!         "env".to_string(),
//!         SecretProviderConfig::Environment {
//!             prefix: Some("MYAPP_SECRET_".to_string())
//!         }
//!     );
//!     
//!     // File-based provider
//!     providers.insert(
//!         "file".to_string(),
//!         SecretProviderConfig::File {
//!             path: "/path/to/secrets.json".to_string()
//!         }
//!     );
//!     
//!     let config = SecretConfig {
//!         default_provider: "env".to_string(),
//!         providers,
//!         enable_masking: true,
//!         timeout_seconds: 30,
//!         enable_caching: true,
//!         cache_ttl_seconds: 300,
//!         rate_limit: Default::default(),
//!     };
//!     
//!     let manager = SecretManager::new(config).await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Security Features
//!
//! ### Input Validation
//!
//! All secret names and values are validated to prevent injection attacks and ensure compliance
//! with naming conventions.
//!
//! ### Rate Limiting
//!
//! Built-in rate limiting prevents abuse and denial-of-service attacks on secret providers.
//!
//! ### Automatic Pattern Detection
//!
//! The masking system automatically detects and masks common secret patterns:
//! - GitHub Personal Access Tokens (`ghp_*`)
//! - AWS Access Keys (`AKIA*`)
//! - JWT tokens
//! - API keys and tokens
//!
//! ### Memory Safety
//!
//! Secrets are handled with care to minimize exposure in memory and logs.
//!
//! ## Provider Support
//!
//! ### Environment Variables
//!
//! ```rust
//! use wrkflw_secrets::{SecretProviderConfig, SecretManager, SecretConfig};
//!
//! // With prefix for better security
//! let provider = SecretProviderConfig::Environment {
//!     prefix: Some("MYAPP_".to_string())
//! };
//! ```
//!
//! ### File-based Storage
//!
//! Supports JSON, YAML, and environment file formats:
//!
//! ```json
//! {
//!   "database_password": "super_secret_password",
//!   "api_key": "your_api_key_here"
//! }
//! ```
//!
//! ```yaml
//! database_password: super_secret_password
//! api_key: your_api_key_here
//! ```
//!
//! ```bash
//! # Environment format
//! DATABASE_PASSWORD=super_secret_password
//! API_KEY="your_api_key_here"
//! ```

pub mod config;
pub mod error;
pub mod manager;
pub mod masking;
pub mod providers;
pub mod rate_limit;
pub mod storage;
pub mod substitution;
pub mod validation;

pub use config::{SecretConfig, SecretProviderConfig};
pub use error::{SecretError, SecretResult};
pub use manager::SecretManager;
pub use masking::SecretMasker;
pub use providers::{SecretProvider, SecretValue};
pub use substitution::SecretSubstitution;

/// Re-export commonly used types
pub mod prelude {
    pub use crate::{
        SecretConfig, SecretError, SecretManager, SecretMasker, SecretProvider, SecretResult,
        SecretSubstitution, SecretValue,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_secret_management() {
        let config = SecretConfig::default();
        let manager = SecretManager::new(config)
            .await
            .expect("Failed to create manager");

        // Use a unique test secret name to avoid conflicts
        let test_secret_name = format!(
            "TEST_SECRET_{}",
            uuid::Uuid::new_v4().to_string().replace('-', "_")
        );
        std::env::set_var(&test_secret_name, "secret_value");

        let result = manager.get_secret(&test_secret_name).await;
        assert!(result.is_ok());

        let secret = result.unwrap();
        assert_eq!(secret.value(), "secret_value");

        std::env::remove_var(&test_secret_name);
    }

    #[tokio::test]
    async fn test_secret_substitution() {
        let config = SecretConfig::default();
        let manager = SecretManager::new(config)
            .await
            .expect("Failed to create manager");

        // Use a unique test secret name to avoid conflicts
        let test_secret_name = format!(
            "GITHUB_TOKEN_{}",
            uuid::Uuid::new_v4().to_string().replace('-', "_")
        );
        std::env::set_var(&test_secret_name, "ghp_test_token");

        let mut substitution = SecretSubstitution::new(&manager);
        let input = format!("echo 'Token: ${{{{ secrets.{} }}}}'", test_secret_name);

        let result = substitution.substitute(&input).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("ghp_test_token"));

        std::env::remove_var(&test_secret_name);
    }

    #[tokio::test]
    async fn test_secret_masking() {
        let masker = SecretMasker::new();
        masker.add_secret("secret123");
        masker.add_secret("password456");

        let input = "The secret is secret123 and password is password456";
        let masked = masker.mask(input);

        assert!(masked.contains("***"));
        assert!(!masked.contains("secret123"));
        assert!(!masked.contains("password456"));
    }
}
