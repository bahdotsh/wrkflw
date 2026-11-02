use crate::{SecretError, SecretManager, SecretResult};
use regex::Regex;
use std::collections::HashMap;

lazy_static::lazy_static! {
    /// Regex to match GitHub-style secret references: ${{ secrets.SECRET_NAME }}
    static ref SECRET_PATTERN: Regex = Regex::new(
        r"\$\{\{\s*secrets\.([a-zA-Z0-9_][a-zA-Z0-9_-]*)\s*\}\}"
    ).unwrap();

    /// Regex to match provider-specific secret references: ${{ secrets.provider:SECRET_NAME }}
    static ref PROVIDER_SECRET_PATTERN: Regex = Regex::new(
        r"\$\{\{\s*secrets\.([a-zA-Z0-9_][a-zA-Z0-9_-]*):([a-zA-Z0-9_][a-zA-Z0-9_-]*)\s*\}\}"
    ).unwrap();
}

/// Secret substitution engine for replacing secret references in text
pub struct SecretSubstitution<'a> {
    manager: &'a SecretManager,
    resolved_secrets: HashMap<String, String>,
    allow_missing: bool,
}

impl<'a> SecretSubstitution<'a> {
    /// Create a new secret substitution engine
    pub fn new(manager: &'a SecretManager) -> Self {
        Self {
            manager,
            resolved_secrets: HashMap::new(),
            allow_missing: false,
        }
    }

    /// Like [`Self::new`], but a secret no provider has expands to an empty
    /// string, as it does on GitHub. Any other provider failure is still an
    /// error, because that secret may well exist.
    pub fn allowing_missing(manager: &'a SecretManager) -> Self {
        Self {
            allow_missing: true,
            ..Self::new(manager)
        }
    }

    /// The value of a lookup, or an empty one for a secret that is allowed
    /// to be missing.
    fn value_of(&self, lookup: SecretResult<crate::SecretValue>) -> SecretResult<String> {
        match lookup {
            Ok(secret) => Ok(secret.value().to_string()),
            Err(SecretError::NotFound { .. }) if self.allow_missing => Ok(String::new()),
            Err(e) => Err(e),
        }
    }

    /// Substitute all secret references in the given text
    pub async fn substitute(&mut self, text: &str) -> SecretResult<String> {
        let mut result = text.to_string();

        // First, handle provider-specific secrets: ${{ secrets.provider:SECRET_NAME }}
        result = self.substitute_provider_secrets(&result).await?;

        // Then handle default provider secrets: ${{ secrets.SECRET_NAME }}
        result = self.substitute_default_secrets(&result).await?;

        Ok(result)
    }

    /// Substitute provider-specific secret references
    async fn substitute_provider_secrets(&mut self, text: &str) -> SecretResult<String> {
        let mut result = text.to_string();

        for captures in PROVIDER_SECRET_PATTERN.captures_iter(text) {
            let full_match = captures.get(0).unwrap().as_str();
            let provider = captures.get(1).unwrap().as_str();
            let secret_name = captures.get(2).unwrap().as_str();

            let cache_key = format!("{}:{}", provider, secret_name);

            let secret_value = if let Some(cached) = self.resolved_secrets.get(&cache_key) {
                cached.clone()
            } else {
                let lookup = self
                    .manager
                    .get_secret_from_provider(provider, secret_name)
                    .await;
                let value = self.value_of(lookup)?;
                self.resolved_secrets.insert(cache_key, value.clone());
                value
            };

            result = result.replace(full_match, &secret_value);
        }

        Ok(result)
    }

    /// Substitute default provider secret references
    async fn substitute_default_secrets(&mut self, text: &str) -> SecretResult<String> {
        let mut result = text.to_string();

        for captures in SECRET_PATTERN.captures_iter(text) {
            let full_match = captures.get(0).unwrap().as_str();
            let secret_name = captures.get(1).unwrap().as_str();

            let secret_value = if let Some(cached) = self.resolved_secrets.get(secret_name) {
                cached.clone()
            } else {
                let lookup = self.manager.get_secret(secret_name).await;
                let value = self.value_of(lookup)?;
                self.resolved_secrets
                    .insert(secret_name.to_string(), value.clone());
                value
            };

            result = result.replace(full_match, &secret_value);
        }

        Ok(result)
    }

    /// Get all resolved secrets (for masking purposes)
    pub fn resolved_secrets(&self) -> &HashMap<String, String> {
        &self.resolved_secrets
    }

    /// Check if text contains secret references
    pub fn contains_secrets(text: &str) -> bool {
        SECRET_PATTERN.is_match(text) || PROVIDER_SECRET_PATTERN.is_match(text)
    }

    /// Extract all secret references from text without resolving them
    pub fn extract_secret_refs(text: &str) -> Vec<SecretRef> {
        let mut refs = Vec::new();

        // Extract provider-specific references
        for captures in PROVIDER_SECRET_PATTERN.captures_iter(text) {
            let full_match = captures.get(0).unwrap().as_str();
            let provider = captures.get(1).unwrap().as_str();
            let name = captures.get(2).unwrap().as_str();

            refs.push(SecretRef {
                full_text: full_match.to_string(),
                provider: Some(provider.to_string()),
                name: name.to_string(),
            });
        }

        // Extract default provider references
        for captures in SECRET_PATTERN.captures_iter(text) {
            let full_match = captures.get(0).unwrap().as_str();
            let name = captures.get(1).unwrap().as_str();

            refs.push(SecretRef {
                full_text: full_match.to_string(),
                provider: None,
                name: name.to_string(),
            });
        }

        refs
    }
}

/// A reference to a secret found in text
#[derive(Debug, Clone, PartialEq)]
pub struct SecretRef {
    /// The full text of the secret reference (e.g., "${{ secrets.API_KEY }}")
    pub full_text: String,
    /// The provider name, if specified
    pub provider: Option<String>,
    /// The secret name
    pub name: String,
}

impl SecretRef {
    /// Get the cache key for this secret reference
    pub fn cache_key(&self) -> String {
        match &self.provider {
            Some(provider) => format!("{}:{}", provider, self.name),
            None => self.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SecretError, SecretManager};

    #[tokio::test]
    async fn test_basic_secret_substitution() {
        // Use unique secret names to avoid test conflicts
        let github_token_name = format!("GITHUB_TOKEN_{}", std::process::id());
        let api_key_name = format!("API_KEY_{}", std::process::id());

        std::env::set_var(&github_token_name, "ghp_test_token");
        std::env::set_var(&api_key_name, "secret_api_key");

        let manager = SecretManager::default().await.unwrap();
        let mut substitution = SecretSubstitution::new(&manager);

        let input = format!(
            "Token: ${{{{ secrets.{} }}}}, API: ${{{{ secrets.{} }}}}",
            github_token_name, api_key_name
        );
        let result = substitution.substitute(&input).await.unwrap();

        assert_eq!(result, "Token: ghp_test_token, API: secret_api_key");

        std::env::remove_var(&github_token_name);
        std::env::remove_var(&api_key_name);
    }

    #[tokio::test]
    async fn test_provider_specific_substitution() {
        // Use unique secret name to avoid test conflicts
        let vault_secret_name = format!("VAULT_SECRET_{}", std::process::id());
        std::env::set_var(&vault_secret_name, "vault_value");

        let manager = SecretManager::default().await.unwrap();
        let mut substitution = SecretSubstitution::new(&manager);

        let input = format!("Value: ${{{{ secrets.env:{} }}}}", vault_secret_name);
        let result = substitution.substitute(&input).await.unwrap();

        assert_eq!(result, "Value: vault_value");

        std::env::remove_var(&vault_secret_name);
    }

    #[tokio::test]
    async fn test_extract_secret_refs() {
        let input = "Token: ${{ secrets.GITHUB_TOKEN }}, Vault: ${{ secrets.vault:API_KEY }}";
        let refs = SecretSubstitution::extract_secret_refs(input);

        assert_eq!(refs.len(), 2);

        let github_ref = &refs.iter().find(|r| r.name == "GITHUB_TOKEN").unwrap();
        assert_eq!(github_ref.provider, None);
        assert_eq!(github_ref.full_text, "${{ secrets.GITHUB_TOKEN }}");

        let vault_ref = &refs.iter().find(|r| r.name == "API_KEY").unwrap();
        assert_eq!(vault_ref.provider, Some("vault".to_string()));
        assert_eq!(vault_ref.full_text, "${{ secrets.vault:API_KEY }}");
    }

    #[tokio::test]
    async fn missing_secrets_are_empty_only_when_allowed() {
        let name = format!("WRKFLW_UNSET_{}", std::process::id());
        let text = format!("[${{{{ secrets.{name} }}}}]");
        let manager = SecretManager::default().await.unwrap();

        let error = SecretSubstitution::new(&manager)
            .substitute(&text)
            .await
            .unwrap_err();
        assert!(matches!(error, SecretError::NotFound { .. }));

        let result = SecretSubstitution::allowing_missing(&manager)
            .substitute(&text)
            .await
            .unwrap();
        assert_eq!(result, "[]");
    }

    #[tokio::test]
    async fn only_a_missing_secret_is_allowed() {
        let manager = SecretManager::default().await.unwrap();
        let error = SecretSubstitution::allowing_missing(&manager)
            .substitute("${{ secrets.no_such_provider:TOKEN }}")
            .await
            .unwrap_err();
        assert!(matches!(error, SecretError::ProviderNotFound { .. }));
    }

    #[tokio::test]
    async fn test_contains_secrets() {
        assert!(SecretSubstitution::contains_secrets(
            "${{ secrets.API_KEY }}"
        ));
        assert!(SecretSubstitution::contains_secrets(
            "${{ secrets.vault:SECRET }}"
        ));
        assert!(!SecretSubstitution::contains_secrets("${{ matrix.os }}"));
        assert!(!SecretSubstitution::contains_secrets("No secrets here"));
    }

    #[tokio::test]
    async fn test_secret_substitution_error_handling() {
        let manager = SecretManager::default().await.unwrap();
        let mut substitution = SecretSubstitution::new(&manager);

        let input = "Token: ${{ secrets.NONEXISTENT_SECRET }}";
        let result = substitution.substitute(input).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            SecretError::NotFound { name } => {
                assert_eq!(name, "NONEXISTENT_SECRET");
            }
            _ => panic!("Expected NotFound error"),
        }
    }
}
