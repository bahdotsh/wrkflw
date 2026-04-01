use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// Compiled regex patterns for common secret formats
struct CompiledPatterns {
    github_pat: Regex,
    github_app: Regex,
    github_oauth: Regex,
    aws_access_key: Regex,
    aws_secret: Regex,
    jwt: Regex,
    api_key: Regex,
}

impl CompiledPatterns {
    fn new() -> Self {
        Self {
            github_pat: Regex::new(r"ghp_[a-zA-Z0-9]{36}").unwrap(),
            github_app: Regex::new(r"ghs_[a-zA-Z0-9]{36}").unwrap(),
            github_oauth: Regex::new(r"gho_[a-zA-Z0-9]{36}").unwrap(),
            aws_access_key: Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            aws_secret: Regex::new(r"[A-Za-z0-9/+=]{40}").unwrap(),
            jwt: Regex::new(r"eyJ[a-zA-Z0-9_-]*\.eyJ[a-zA-Z0-9_-]*\.[a-zA-Z0-9_-]*").unwrap(),
            api_key: Regex::new(r"(?i)(api[_-]?key|token)[\s:=]+[a-zA-Z0-9_-]{16,}").unwrap(),
        }
    }
}

/// Global compiled patterns (initialized once)
static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new();

/// Secret masking utility to prevent secrets from appearing in logs
pub struct SecretMasker {
    secrets: HashSet<String>,
    secret_cache: HashMap<String, String>, // Cache masked versions
    mask_char: char,
    min_length: usize,
}

impl SecretMasker {
    /// Create a new secret masker
    pub fn new() -> Self {
        Self {
            secrets: HashSet::new(),
            secret_cache: HashMap::new(),
            mask_char: '*',
            min_length: 3, // Don't mask very short strings
        }
    }

    /// Create a new secret masker with custom mask character
    pub fn with_mask_char(mask_char: char) -> Self {
        Self {
            secrets: HashSet::new(),
            secret_cache: HashMap::new(),
            mask_char,
            min_length: 3,
        }
    }

    /// Add a secret to be masked
    pub fn add_secret(&mut self, secret: impl Into<String>) {
        let secret = secret.into();
        if secret.len() >= self.min_length {
            let masked = self.create_mask(&secret);
            self.secret_cache.insert(secret.clone(), masked);
            self.secrets.insert(secret);
        }
    }

    /// Add multiple secrets to be masked
    pub fn add_secrets(&mut self, secrets: impl IntoIterator<Item = String>) {
        for secret in secrets {
            self.add_secret(secret);
        }
    }

    /// Remove a secret from masking
    pub fn remove_secret(&mut self, secret: &str) {
        self.secrets.remove(secret);
        self.secret_cache.remove(secret);
    }

    /// Clear all secrets
    pub fn clear(&mut self) {
        self.secrets.clear();
        self.secret_cache.clear();
    }

    /// Mask secrets in the given text
    pub fn mask(&self, text: &str) -> String {
        let mut result = text.to_string();

        // Use cached masked versions for better performance
        for secret in &self.secrets {
            if !secret.is_empty() {
                if let Some(masked) = self.secret_cache.get(secret) {
                    result = result.replace(secret, masked);
                }
            }
        }

        // Also mask potential tokens and keys with regex patterns
        result = self.mask_patterns(&result);

        result
    }

    /// Create a mask for a secret, preserving some structure for debugging
    fn create_mask(&self, secret: &str) -> String {
        let len = secret.len();

        if len <= 3 {
            // Very short secrets - mask completely
            self.mask_char.to_string().repeat(3)
        } else if len <= 8 {
            // Short secrets - show first character
            let first = secret.chars().next().unwrap_or(self.mask_char);
            format!("{}{}", first, self.mask_char.to_string().repeat(len - 1))
        } else {
            // Longer secrets - show first 2 and last 2 characters
            let chars: Vec<char> = secret.chars().collect();
            let first_two = chars.iter().take(2).collect::<String>();
            let last_two = chars.iter().skip(len - 2).collect::<String>();
            let middle_mask = self.mask_char.to_string().repeat(len - 4);
            format!("{}{}{}", first_two, middle_mask, last_two)
        }
    }

    /// Mask common patterns that look like secrets
    fn mask_patterns(&self, text: &str) -> String {
        let patterns = PATTERNS.get_or_init(CompiledPatterns::new);
        let mut result = text.to_string();

        // GitHub Personal Access Tokens
        result = patterns
            .github_pat
            .replace_all(&result, "ghp_***")
            .to_string();

        // GitHub App tokens
        result = patterns
            .github_app
            .replace_all(&result, "ghs_***")
            .to_string();

        // GitHub OAuth tokens
        result = patterns
            .github_oauth
            .replace_all(&result, "gho_***")
            .to_string();

        // AWS Access Key IDs
        result = patterns
            .aws_access_key
            .replace_all(&result, "AKIA***")
            .to_string();

        // AWS Secret Access Keys (basic pattern)
        // Only mask if it's clearly in a secret context (basic heuristic)
        if text.to_lowercase().contains("secret") || text.to_lowercase().contains("key") {
            result = patterns.aws_secret.replace_all(&result, "***").to_string();
        }

        // JWT tokens (basic pattern)
        result = patterns
            .jwt
            .replace_all(&result, "eyJ***.eyJ***.***")
            .to_string();

        // API keys with common prefixes
        result = patterns
            .api_key
            .replace_all(&result, "${1}=***")
            .to_string();

        result
    }

    /// Check if text contains any secrets
    pub fn contains_secrets(&self, text: &str) -> bool {
        for secret in &self.secrets {
            if text.contains(secret) {
                return true;
            }
        }

        // Also check for common patterns
        self.has_secret_patterns(text)
    }

    /// Check if text contains common secret patterns
    fn has_secret_patterns(&self, text: &str) -> bool {
        let patterns = PATTERNS.get_or_init(CompiledPatterns::new);

        patterns.github_pat.is_match(text)
            || patterns.github_app.is_match(text)
            || patterns.github_oauth.is_match(text)
            || patterns.aws_access_key.is_match(text)
            || patterns.jwt.is_match(text)
    }

    /// Get the number of secrets being tracked
    pub fn secret_count(&self) -> usize {
        self.secrets.len()
    }

    /// Check if a specific secret is being tracked
    pub fn has_secret(&self, secret: &str) -> bool {
        self.secrets.contains(secret)
    }
}

impl Default for SecretMasker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_masking() {
        let mut masker = SecretMasker::new();
        masker.add_secret("secret123");
        masker.add_secret("password456");

        let input = "The secret is secret123 and password is password456";
        let masked = masker.mask(input);

        assert!(!masked.contains("secret123"));
        assert!(!masked.contains("password456"));
        assert!(masked.contains("***"));
    }

    #[test]
    fn test_preserve_structure() {
        let mut masker = SecretMasker::new();
        masker.add_secret("verylongsecretkey123");

        let input = "Key: verylongsecretkey123";
        let masked = masker.mask(input);

        // Should preserve first 2 and last 2 characters
        assert!(masked.contains("ve"));
        assert!(masked.contains("23"));
        assert!(masked.contains("***"));
        assert!(!masked.contains("verylongsecretkey123"));
    }

    #[test]
    fn test_github_token_patterns() {
        let masker = SecretMasker::new();

        let input = "Token: ghp_1234567890123456789012345678901234567890";
        let masked = masker.mask(input);

        assert!(!masked.contains("ghp_1234567890123456789012345678901234567890"));
        assert!(masked.contains("ghp_***"));
    }

    #[test]
    fn test_aws_access_key_patterns() {
        let masker = SecretMasker::new();

        let input = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE";
        let masked = masker.mask(input);

        assert!(!masked.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(masked.contains("AKIA***"));
    }

    #[test]
    fn test_jwt_token_patterns() {
        let masker = SecretMasker::new();

        let input = "JWT: eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let masked = masker.mask(input);

        assert!(masked.contains("eyJ***.eyJ***.***"));
        assert!(!masked.contains("SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"));
    }

    #[test]
    fn test_contains_secrets() {
        let mut masker = SecretMasker::new();
        masker.add_secret("secret123");

        assert!(masker.contains_secrets("The secret is secret123"));
        assert!(!masker.contains_secrets("No secrets here"));
        assert!(masker.contains_secrets("Token: ghp_1234567890123456789012345678901234567890"));
    }

    #[test]
    fn test_short_secrets() {
        let mut masker = SecretMasker::new();
        masker.add_secret("ab"); // Too short, should not be added
        masker.add_secret("abc"); // Minimum length

        assert_eq!(masker.secret_count(), 1);
        assert!(!masker.has_secret("ab"));
        assert!(masker.has_secret("abc"));
    }

    #[test]
    fn test_custom_mask_char() {
        let mut masker = SecretMasker::with_mask_char('X');
        masker.add_secret("secret123");

        let input = "The secret is secret123";
        let masked = masker.mask(input);

        assert!(masked.contains("XX"));
        assert!(!masked.contains("**"));
    }

    #[test]
    fn test_remove_secret() {
        let mut masker = SecretMasker::new();
        masker.add_secret("secret123");
        masker.add_secret("password456");

        assert_eq!(masker.secret_count(), 2);

        masker.remove_secret("secret123");
        assert_eq!(masker.secret_count(), 1);
        assert!(!masker.has_secret("secret123"));
        assert!(masker.has_secret("password456"));
    }

    #[test]
    fn test_clear_secrets() {
        let mut masker = SecretMasker::new();
        masker.add_secret("secret123");
        masker.add_secret("password456");

        assert_eq!(masker.secret_count(), 2);

        masker.clear();
        assert_eq!(masker.secret_count(), 0);
    }
}
