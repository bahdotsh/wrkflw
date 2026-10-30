use crate::rate_limit::RateLimitConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for the secrets management system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretConfig {
    /// Default secret provider to use when none is specified
    pub default_provider: String,

    /// Configuration for each secret provider
    pub providers: HashMap<String, SecretProviderConfig>,

    /// Whether to enable secret masking in logs
    pub enable_masking: bool,

    /// Timeout for secret operations in seconds
    pub timeout_seconds: u64,

    /// Whether to cache secrets for performance
    pub enable_caching: bool,

    /// Cache TTL in seconds
    pub cache_ttl_seconds: u64,

    /// Rate limiting configuration
    #[serde(skip)]
    pub rate_limit: RateLimitConfig,
}

impl Default for SecretConfig {
    fn default() -> Self {
        let mut providers = HashMap::new();

        // Add default environment variable provider
        providers.insert(
            "env".to_string(),
            SecretProviderConfig::Environment { prefix: None },
        );

        // Add default file provider
        providers.insert(
            "file".to_string(),
            SecretProviderConfig::File {
                path: default_secrets_path(),
            },
        );

        Self {
            default_provider: "env".to_string(),
            providers,
            enable_masking: true,
            timeout_seconds: 30,
            enable_caching: true,
            cache_ttl_seconds: 300, // 5 minutes
            rate_limit: RateLimitConfig::default(),
        }
    }
}

/// Default location of the file provider: `<config dir>/wrkflw/secrets`, where
/// `<config dir>` is `$XDG_CONFIG_HOME` (or `~/.config`) on Linux.
fn default_secrets_path() -> String {
    resolve_secrets_path(dirs::config_dir(), dirs::home_dir())
}

/// Picks the default secrets path. An existing `~/.wrkflw/secrets` keeps being
/// used until the new location exists, so upgrading never hides a user's secrets.
fn resolve_secrets_path(config_dir: Option<PathBuf>, home_dir: Option<PathBuf>) -> String {
    let preferred = config_dir.map(|dir| dir.join("wrkflw").join("secrets"));
    let legacy = home_dir.map(|home| home.join(".wrkflw").join("secrets"));

    let path = match (preferred, legacy) {
        (Some(preferred), Some(legacy)) if !preferred.exists() && legacy.exists() => legacy,
        (Some(preferred), _) => preferred,
        (None, legacy) => legacy.unwrap_or_else(|| PathBuf::from("~/.wrkflw/secrets")),
    };
    path.to_string_lossy().into_owned()
}

/// Configuration for different types of secret providers
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecretProviderConfig {
    /// Environment variables provider
    Environment {
        /// Optional prefix for environment variables (e.g., "WRKFLW_SECRET_")
        prefix: Option<String>,
    },

    /// File-based secret storage
    File {
        /// Path to the secrets file or directory
        path: String,
    },
    // Cloud providers are planned for future implementation
    // /// HashiCorp Vault provider
    // #[cfg(feature = "vault-provider")]
    // Vault {
    //     /// Vault server URL
    //     url: String,
    //     /// Authentication method
    //     auth: VaultAuth,
    //     /// Optional mount path (defaults to "secret")
    //     mount_path: Option<String>,
    // },

    // /// AWS Secrets Manager provider
    // #[cfg(feature = "aws-provider")]
    // AwsSecretsManager {
    //     /// AWS region
    //     region: String,
    //     /// Optional role ARN to assume
    //     role_arn: Option<String>,
    // },

    // /// Azure Key Vault provider
    // #[cfg(feature = "azure-provider")]
    // AzureKeyVault {
    //     /// Key Vault URL
    //     vault_url: String,
    //     /// Authentication method
    //     auth: AzureAuth,
    // },

    // /// Google Cloud Secret Manager provider
    // #[cfg(feature = "gcp-provider")]
    // GcpSecretManager {
    //     /// GCP project ID
    //     project_id: String,
    //     /// Optional service account key file path
    //     key_file: Option<String>,
    // },
}

// Cloud provider authentication types are planned for future implementation
// /// HashiCorp Vault authentication methods
// #[cfg(feature = "vault-provider")]
// #[derive(Debug, Clone, Serialize, Deserialize)]
// #[serde(tag = "method", rename_all = "snake_case")]
// pub enum VaultAuth {
//     /// Token-based authentication
//     Token { token: String },
//     /// AppRole authentication
//     AppRole { role_id: String, secret_id: String },
//     /// Kubernetes authentication
//     Kubernetes {
//         role: String,
//         jwt_path: Option<String>,
//     },
// }

// /// Azure authentication methods
// #[cfg(feature = "azure-provider")]
// #[derive(Debug, Clone, Serialize, Deserialize)]
// #[serde(tag = "method", rename_all = "snake_case")]
// pub enum AzureAuth {
//     /// Service Principal authentication
//     ServicePrincipal {
//         client_id: String,
//         client_secret: String,
//         tenant_id: String,
//     },
//     /// Managed Identity authentication
//     ManagedIdentity,
//     /// Azure CLI authentication
//     AzureCli,
// }

impl SecretConfig {
    /// Load configuration from a file
    pub fn from_file(path: &str) -> crate::SecretResult<Self> {
        let content = std::fs::read_to_string(path)?;

        if path.ends_with(".json") {
            Ok(serde_json::from_str(&content)?)
        } else if path.ends_with(".yml") || path.ends_with(".yaml") {
            Ok(serde_yaml::from_str(&content)?)
        } else {
            Err(crate::SecretError::invalid_config(
                "Unsupported config file format. Use .json, .yml, or .yaml",
            ))
        }
    }

    /// Save configuration to a file
    pub fn to_file(&self, path: &str) -> crate::SecretResult<()> {
        let content = if path.ends_with(".json") {
            serde_json::to_string_pretty(self)?
        } else if path.ends_with(".yml") || path.ends_with(".yaml") {
            serde_yaml::to_string(self)?
        } else {
            return Err(crate::SecretError::invalid_config(
                "Unsupported config file format. Use .json, .yml, or .yaml",
            ));
        };

        std::fs::write(path, content)?;
        Ok(())
    }

    /// Load configuration from environment variables
    pub fn from_env() -> Self {
        let mut config = Self::default();

        // Override default provider if specified
        if let Ok(provider) = std::env::var("WRKFLW_DEFAULT_SECRET_PROVIDER") {
            config.default_provider = provider;
        }

        // Override masking setting
        if let Ok(masking) = std::env::var("WRKFLW_SECRET_MASKING") {
            config.enable_masking = masking.parse().unwrap_or(true);
        }

        // Override timeout
        if let Ok(timeout) = std::env::var("WRKFLW_SECRET_TIMEOUT") {
            config.timeout_seconds = timeout.parse().unwrap_or(30);
        }

        config
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    fn secrets_path(config: &std::path::Path, home: &std::path::Path) -> String {
        resolve_secrets_path(Some(config.to_path_buf()), Some(home.to_path_buf()))
    }

    #[test]
    fn prefers_config_dir_when_nothing_exists() {
        let config = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();

        assert_eq!(
            secrets_path(config.path(), home.path()),
            config.path().join("wrkflw/secrets").to_string_lossy()
        );
    }

    #[test]
    fn keeps_legacy_path_when_only_it_exists() {
        let config = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".wrkflw/secrets")).unwrap();

        assert_eq!(
            secrets_path(config.path(), home.path()),
            home.path().join(".wrkflw/secrets").to_string_lossy()
        );
    }

    #[test]
    fn prefers_config_dir_when_both_exist() {
        let config = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(config.path().join("wrkflw/secrets")).unwrap();
        std::fs::create_dir_all(home.path().join(".wrkflw/secrets")).unwrap();

        assert_eq!(
            secrets_path(config.path(), home.path()),
            config.path().join("wrkflw/secrets").to_string_lossy()
        );
    }

    #[test]
    fn falls_back_to_unexpanded_legacy_path_without_any_directory() {
        assert_eq!(resolve_secrets_path(None, None), "~/.wrkflw/secrets");
    }
}
