use crate::{
    validation::validate_secret_value, SecretError, SecretProvider, SecretResult, SecretValue,
};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// File-based secret provider
pub struct FileProvider {
    path: String,
}

impl FileProvider {
    /// Create a new file provider
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    /// Expand tilde in path
    fn expand_path(&self) -> String {
        if self.path.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(&self.path[2..]).to_string_lossy().to_string();
            }
        }
        self.path.clone()
    }

    /// Load secrets from JSON file
    async fn load_json_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let json: Value = serde_json::from_str(&content)?;

        let mut secrets = HashMap::new();
        if let Value::Object(obj) = json {
            for (key, value) in obj {
                if let Value::String(secret_value) = value {
                    secrets.insert(key, secret_value);
                } else {
                    secrets.insert(key, value.to_string());
                }
            }
        }

        Ok(secrets)
    }

    /// Load secrets from YAML file
    async fn load_yaml_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;

        let mut secrets = HashMap::new();
        if let serde_yaml::Value::Mapping(map) = yaml {
            for (key, value) in map {
                if let (serde_yaml::Value::String(k), v) = (key, value) {
                    let secret_value = match v {
                        serde_yaml::Value::String(s) => s,
                        _ => serde_yaml::to_string(&v)?.trim().to_string(),
                    };
                    secrets.insert(k, secret_value);
                }
            }
        }

        Ok(secrets)
    }

    /// Load secrets from environment-style file
    async fn load_env_secrets(&self, file_path: &Path) -> SecretResult<HashMap<String, String>> {
        let content = tokio::fs::read_to_string(file_path).await?;
        let mut secrets = HashMap::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim();

                // Handle quoted values (length check prevents panic on single-char values like `"`)
                let value = if value.len() >= 2
                    && ((value.starts_with('"') && value.ends_with('"'))
                        || (value.starts_with('\'') && value.ends_with('\'')))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };

                secrets.insert(key, value.to_string());
            }
        }

        Ok(secrets)
    }

    /// Load all secrets from the configured path
    async fn load_secrets(&self) -> SecretResult<HashMap<String, String>> {
        let expanded_path = self.expand_path();
        let path = Path::new(&expanded_path);

        if !path.exists() {
            return Ok(HashMap::new());
        }

        if path.is_file() {
            // Single file - determine format by extension
            if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
                match extension.to_lowercase().as_str() {
                    "json" => self.load_json_secrets(path).await,
                    "yml" | "yaml" => self.load_yaml_secrets(path).await,
                    "env" => self.load_env_secrets(path).await,
                    _ => {
                        // Default to environment format for unknown extensions
                        self.load_env_secrets(path).await
                    }
                }
            } else {
                // No extension, try environment format
                self.load_env_secrets(path).await
            }
        } else {
            // Directory - load from multiple files
            let mut all_secrets = HashMap::new();
            let mut entries = tokio::fs::read_dir(path).await?;

            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    if let Some(extension) = entry_path.extension().and_then(|ext| ext.to_str()) {
                        let secrets = match extension.to_lowercase().as_str() {
                            "json" => self.load_json_secrets(&entry_path).await?,
                            "yml" | "yaml" => self.load_yaml_secrets(&entry_path).await?,
                            "env" => self.load_env_secrets(&entry_path).await?,
                            _ => continue, // Skip unknown file types
                        };
                        all_secrets.extend(secrets);
                    }
                }
            }

            Ok(all_secrets)
        }
    }
}

#[async_trait]
impl SecretProvider for FileProvider {
    async fn get_secret(&self, name: &str) -> SecretResult<SecretValue> {
        let secrets = self.load_secrets().await?;

        if let Some(value) = secrets.get(name) {
            // Validate the secret value
            validate_secret_value(value)?;

            let mut metadata = HashMap::new();
            metadata.insert("source".to_string(), "file".to_string());
            metadata.insert("file_path".to_string(), self.expand_path());

            Ok(SecretValue::with_metadata(value.clone(), metadata))
        } else {
            Err(SecretError::not_found(name))
        }
    }

    async fn list_secrets(&self) -> SecretResult<Vec<String>> {
        let secrets = self.load_secrets().await?;
        Ok(secrets.keys().cloned().collect())
    }

    fn name(&self) -> &str {
        "file"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_json_file(dir: &TempDir, content: &str) -> String {
        let file_path = dir.path().join("secrets.json");
        tokio::fs::write(&file_path, content).await.unwrap();
        file_path.to_string_lossy().to_string()
    }

    async fn create_test_env_file(dir: &TempDir, content: &str) -> String {
        let file_path = dir.path().join("secrets.env");
        tokio::fs::write(&file_path, content).await.unwrap();
        file_path.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn test_file_provider_json() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_json_file(
            &temp_dir,
            r#"
            {
                "API_KEY": "secret_api_key",
                "DB_PASSWORD": "secret_password"
            }
        "#,
        )
        .await;

        let provider = FileProvider::new(file_path);

        let result = provider.get_secret("API_KEY").await;
        assert!(result.is_ok());

        let secret = result.unwrap();
        assert_eq!(secret.value(), "secret_api_key");
        assert_eq!(secret.metadata.get("source"), Some(&"file".to_string()));
    }

    #[tokio::test]
    async fn test_file_provider_env_format() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_env_file(
            &temp_dir,
            r#"
            # This is a comment
            API_KEY=secret_api_key
            DB_PASSWORD="quoted password"
            GITHUB_TOKEN='single quoted token'
        "#,
        )
        .await;

        let provider = FileProvider::new(file_path);

        let api_key = provider.get_secret("API_KEY").await.unwrap();
        assert_eq!(api_key.value(), "secret_api_key");

        let password = provider.get_secret("DB_PASSWORD").await.unwrap();
        assert_eq!(password.value(), "quoted password");

        let token = provider.get_secret("GITHUB_TOKEN").await.unwrap();
        assert_eq!(token.value(), "single quoted token");
    }

    #[tokio::test]
    async fn test_file_provider_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_json_file(&temp_dir, "{}").await;

        let provider = FileProvider::new(file_path);

        let result = provider.get_secret("NONEXISTENT").await;
        assert!(result.is_err());

        match result.unwrap_err() {
            SecretError::NotFound { name } => {
                assert_eq!(name, "NONEXISTENT");
            }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_file_provider_list_secrets() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = create_test_json_file(
            &temp_dir,
            r#"
            {
                "SECRET_1": "value1",
                "SECRET_2": "value2",
                "SECRET_3": "value3"
            }
        "#,
        )
        .await;

        let provider = FileProvider::new(file_path);

        let secrets = provider.list_secrets().await.unwrap();
        assert_eq!(secrets.len(), 3);
        assert!(secrets.contains(&"SECRET_1".to_string()));
        assert!(secrets.contains(&"SECRET_2".to_string()));
        assert!(secrets.contains(&"SECRET_3".to_string()));
    }
}
