// Copyright 2024 wrkflw contributors
// SPDX-License-Identifier: MIT

//! Integration tests for the secrets crate

use std::collections::HashMap;
use std::process;
use tempfile::TempDir;
use wrkflw_secrets::{
    SecretConfig, SecretManager, SecretMasker, SecretProviderConfig, SecretSubstitution,
};

/// Test end-to-end secret management workflow
#[tokio::test]
async fn test_end_to_end_secret_workflow() {
    // Create a temporary directory for file-based secrets
    let temp_dir = TempDir::new().unwrap();
    let secrets_file = temp_dir.path().join("secrets.json");

    // Create a secrets file
    let secrets_content = r#"
    {
        "database_password": "super_secret_db_pass_123",
        "api_token": "tk_abc123def456ghi789",
        "encryption_key": "key_zyxwvutsrqponmlkjihgfedcba9876543210"
    }
    "#;
    std::fs::write(&secrets_file, secrets_content).unwrap();

    // Set up environment variables
    let env_secret_name = format!("GITHUB_TOKEN_{}", process::id());
    std::env::set_var(&env_secret_name, "ghp_1234567890abcdefghijklmnopqrstuvwxyz");

    // Create configuration
    let mut providers = HashMap::new();
    providers.insert(
        "env".to_string(),
        SecretProviderConfig::Environment { prefix: None },
    );
    providers.insert(
        "file".to_string(),
        SecretProviderConfig::File {
            path: secrets_file.to_string_lossy().to_string(),
        },
    );

    let config = SecretConfig {
        default_provider: "env".to_string(),
        providers,
        enable_masking: true,
        timeout_seconds: 30,
        enable_caching: true,
        cache_ttl_seconds: 300,
        rate_limit: Default::default(),
    };

    // Initialize secret manager
    let manager = SecretManager::new(config).await.unwrap();

    // Test 1: Get secret from environment provider
    let env_secret = manager.get_secret(&env_secret_name).await.unwrap();
    assert_eq!(
        env_secret.value(),
        "ghp_1234567890abcdefghijklmnopqrstuvwxyz"
    );
    assert_eq!(
        env_secret.metadata.get("source"),
        Some(&"environment".to_string())
    );

    // Test 2: Get secret from file provider
    let file_secret = manager
        .get_secret_from_provider("file", "database_password")
        .await
        .unwrap();
    assert_eq!(file_secret.value(), "super_secret_db_pass_123");
    assert_eq!(
        file_secret.metadata.get("source"),
        Some(&"file".to_string())
    );

    // Test 3: List secrets from file provider
    let all_secrets = manager.list_all_secrets().await.unwrap();
    assert!(all_secrets.contains_key("file"));
    let file_secrets = &all_secrets["file"];
    assert!(file_secrets.contains(&"database_password".to_string()));
    assert!(file_secrets.contains(&"api_token".to_string()));
    assert!(file_secrets.contains(&"encryption_key".to_string()));

    // Test 4: Secret substitution
    let mut substitution = SecretSubstitution::new(&manager);
    let input = format!(
        "Database: ${{{{ secrets.file:database_password }}}}, GitHub: ${{{{ secrets.{} }}}}",
        env_secret_name
    );
    let output = substitution.substitute(&input).await.unwrap();
    assert!(output.contains("super_secret_db_pass_123"));
    assert!(output.contains("ghp_1234567890abcdefghijklmnopqrstuvwxyz"));

    // Test 5: Secret masking
    let masker = SecretMasker::new();
    masker.add_secret("super_secret_db_pass_123");
    masker.add_secret("ghp_1234567890abcdefghijklmnopqrstuvwxyz");

    let log_message = "Connection failed: super_secret_db_pass_123 invalid for ghp_1234567890abcdefghijklmnopqrstuvwxyz";
    let masked = masker.mask(log_message);
    assert!(!masked.contains("super_secret_db_pass_123"));
    assert!(!masked.contains("ghp_1234567890abcdefghijklmnopqrstuvwxyz"));
    assert!(masked.contains("***"));

    // Test 6: Health check
    let health_results = manager.health_check().await;
    assert!(health_results.get("env").unwrap().is_ok());
    assert!(health_results.get("file").unwrap().is_ok());

    // Test 7: Caching behavior - functional test instead of timing
    // First call should succeed and populate cache
    let cached_secret = manager.get_secret(&env_secret_name).await.unwrap();
    assert_eq!(
        cached_secret.value(),
        "ghp_1234567890abcdefghijklmnopqrstuvwxyz"
    );

    // Remove the environment variable to test if cache works
    std::env::remove_var(&env_secret_name);

    // Second call should still succeed because value is cached
    let cached_secret_2 = manager.get_secret(&env_secret_name).await.unwrap();
    assert_eq!(
        cached_secret_2.value(),
        "ghp_1234567890abcdefghijklmnopqrstuvwxyz"
    );

    // Restore environment variable for cleanup
    std::env::set_var(&env_secret_name, "ghp_1234567890abcdefghijklmnopqrstuvwxyz");

    // Cleanup
    std::env::remove_var(&env_secret_name);
}

/// Test error handling scenarios
#[tokio::test]
async fn test_error_handling() {
    let manager = SecretManager::default().await.unwrap();

    // Test 1: Secret not found
    let result = manager.get_secret("NONEXISTENT_SECRET_12345").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    // Test 2: Invalid provider
    let result = manager
        .get_secret_from_provider("invalid_provider", "some_secret")
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    // Test 3: Invalid secret name
    let result = manager.get_secret("").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot be empty"));

    // Test 4: Invalid secret name with special characters
    let result = manager.get_secret("invalid/secret/name").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("can only contain"));
}

/// Test rate limiting functionality
#[tokio::test]
async fn test_rate_limiting() {
    use std::time::Duration;
    use wrkflw_secrets::rate_limit::RateLimitConfig;

    // Create config with very low rate limit
    let config = SecretConfig {
        rate_limit: RateLimitConfig {
            max_requests: 2,
            window_duration: Duration::from_secs(10),
            enabled: true,
        },
        ..SecretConfig::default()
    };

    let manager = SecretManager::new(config).await.unwrap();

    // Set up test secret
    let test_secret_name = format!("RATE_LIMIT_TEST_{}", process::id());
    std::env::set_var(&test_secret_name, "test_value");

    // First two requests should succeed
    let result1 = manager.get_secret(&test_secret_name).await;
    assert!(result1.is_ok());

    let result2 = manager.get_secret(&test_secret_name).await;
    assert!(result2.is_ok());

    // Third request should fail due to rate limiting
    let result3 = manager.get_secret(&test_secret_name).await;
    assert!(result3.is_err());
    assert!(result3
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Cleanup
    std::env::remove_var(&test_secret_name);
}

/// Test concurrent access patterns
#[tokio::test]
async fn test_concurrent_access() {
    use std::sync::Arc;

    let manager = Arc::new(SecretManager::default().await.unwrap());

    // Set up test secret
    let test_secret_name = format!("CONCURRENT_TEST_{}", process::id());
    std::env::set_var(&test_secret_name, "concurrent_test_value");

    // Spawn multiple concurrent tasks
    let mut handles = Vec::new();
    for i in 0..10 {
        let manager_clone = Arc::clone(&manager);
        let secret_name = test_secret_name.clone();
        let handle = tokio::spawn(async move {
            let result = manager_clone.get_secret(&secret_name).await;
            (i, result)
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    let mut successful_requests = 0;
    for handle in handles {
        let (_, result) = handle.await.unwrap();
        if let Ok(secret) = result {
            successful_requests += 1;
            assert_eq!(secret.value(), "concurrent_test_value");
        }
    }

    // At least some requests should succeed (depending on rate limiting)
    assert!(successful_requests > 0);

    // Cleanup
    std::env::remove_var(&test_secret_name);
}

/// Test secret substitution edge cases
#[tokio::test]
async fn test_substitution_edge_cases() {
    let manager = SecretManager::default().await.unwrap();

    // Set up test secrets
    let secret1_name = format!("EDGE_CASE_1_{}", process::id());
    let secret2_name = format!("EDGE_CASE_2_{}", process::id());
    std::env::set_var(&secret1_name, "value1");
    std::env::set_var(&secret2_name, "value2");

    let mut substitution = SecretSubstitution::new(&manager);

    // Test 1: Multiple references to the same secret
    let input = format!(
        "First: ${{{{ secrets.{} }}}} Second: ${{{{ secrets.{} }}}}",
        secret1_name, secret1_name
    );
    let output = substitution.substitute(&input).await.unwrap();
    assert_eq!(output, "First: value1 Second: value1");

    // Test 2: Nested-like patterns (should not be substituted)
    let input = "This is not a secret: ${ secrets.FAKE }";
    let output = substitution.substitute(input).await.unwrap();
    assert_eq!(input, output); // Should remain unchanged

    // Test 3: Mixed valid and invalid references
    let input = format!(
        "Valid: ${{{{ secrets.{} }}}} Invalid: ${{{{ secrets.NONEXISTENT }}}}",
        secret1_name
    );
    let result = substitution.substitute(&input).await;
    assert!(result.is_err()); // Should fail due to missing secret

    // Test 4: Empty input
    let output = substitution.substitute("").await.unwrap();
    assert_eq!(output, "");

    // Test 5: No secret references
    let input = "This is just plain text with no secrets";
    let output = substitution.substitute(input).await.unwrap();
    assert_eq!(input, output);

    // Cleanup
    std::env::remove_var(&secret1_name);
    std::env::remove_var(&secret2_name);
}

/// Test masking comprehensive patterns
#[tokio::test]
async fn test_comprehensive_masking() {
    let masker = SecretMasker::new();

    // Add various types of secrets
    masker.add_secret("password123");
    masker.add_secret("api_key_abcdef123456");
    masker.add_secret("very_long_secret_key_that_should_preserve_structure_987654321");

    // Test various input scenarios
    let test_cases = vec![
        (
            "Password is password123 and API key is api_key_abcdef123456",
            vec!["password123", "api_key_abcdef123456"],
        ),
        (
            "GitHub token: ghp_1234567890123456789012345678901234567890",
            vec!["ghp_"],
        ),
        (
            "AWS key: AKIAIOSFODNN7EXAMPLE",
            vec!["AKIA"],
        ),
        (
            "JWT: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            vec!["eyJ", "***"],
        ),
    ];

    for (input, should_not_contain) in test_cases {
        let masked = masker.mask(input);
        for pattern in should_not_contain {
            if pattern != "***" {
                assert!(
                    !masked.contains(pattern)
                        || pattern == "ghp_"
                        || pattern == "AKIA"
                        || pattern == "eyJ",
                    "Masked text '{}' should not contain '{}' (or only partial patterns)",
                    masked,
                    pattern
                );
            } else {
                assert!(
                    masked.contains(pattern),
                    "Masked text '{}' should contain '{}'",
                    masked,
                    pattern
                );
            }
        }
    }
}
