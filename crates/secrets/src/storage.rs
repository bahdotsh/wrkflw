use crate::{SecretError, SecretResult};
use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Encrypted secret storage for sensitive data at rest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSecretStore {
    /// Encrypted secrets map (base64 encoded, nonce prepended to ciphertext)
    secrets: HashMap<String, String>,
    /// Salt for key derivation (base64 encoded)
    salt: String,
}

impl EncryptedSecretStore {
    /// Create a new encrypted secret store with a random key
    pub fn new() -> SecretResult<(Self, [u8; 32])> {
        let key = Aes256Gcm::generate_key(&mut OsRng);
        let salt = Self::generate_salt();

        let store = Self {
            secrets: HashMap::new(),
            salt: general_purpose::STANDARD.encode(salt),
        };

        Ok((store, key.into()))
    }

    /// Create an encrypted secret store from existing data
    pub fn from_data(secrets: HashMap<String, String>, salt: String) -> Self {
        Self { secrets, salt }
    }

    /// Add an encrypted secret
    pub fn add_secret(&mut self, key: &[u8; 32], name: &str, value: &str) -> SecretResult<()> {
        let encrypted = Self::encrypt_value(key, value)?;
        self.secrets.insert(name.to_string(), encrypted);
        Ok(())
    }

    /// Get and decrypt a secret
    pub fn get_secret(&self, key: &[u8; 32], name: &str) -> SecretResult<String> {
        let encrypted = self
            .secrets
            .get(name)
            .ok_or_else(|| SecretError::not_found(name))?;

        Self::decrypt_value(key, encrypted)
    }

    /// Remove a secret
    pub fn remove_secret(&mut self, name: &str) -> bool {
        self.secrets.remove(name).is_some()
    }

    /// List all secret names
    pub fn list_secrets(&self) -> Vec<String> {
        self.secrets.keys().cloned().collect()
    }

    /// Check if a secret exists
    pub fn has_secret(&self, name: &str) -> bool {
        self.secrets.contains_key(name)
    }

    /// Get the number of stored secrets
    pub fn secret_count(&self) -> usize {
        self.secrets.len()
    }

    /// Clear all secrets
    pub fn clear(&mut self) {
        self.secrets.clear();
    }

    /// Encrypt a value with a fresh nonce prepended to the ciphertext
    fn encrypt_value(key: &[u8; 32], value: &str) -> SecretResult<String> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let nonce_bytes = Self::generate_nonce();
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, value.as_bytes())
            .map_err(|e| SecretError::EncryptionError(format!("Encryption failed: {}", e)))?;

        // Prepend nonce to ciphertext so each secret carries its own nonce
        let mut combined = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
        combined.extend_from_slice(&nonce_bytes);
        combined.extend_from_slice(&ciphertext);

        Ok(general_purpose::STANDARD.encode(&combined))
    }

    /// Decrypt a value (nonce is extracted from the ciphertext prefix)
    fn decrypt_value(key: &[u8; 32], encrypted: &str) -> SecretResult<String> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
        let combined = general_purpose::STANDARD
            .decode(encrypted)
            .map_err(|e| SecretError::EncryptionError(format!("Invalid ciphertext: {}", e)))?;

        if combined.len() < 12 {
            return Err(SecretError::EncryptionError(
                "Ciphertext too short to contain nonce".to_string(),
            ));
        }

        let (nonce_bytes, ciphertext) = combined.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::EncryptionError(format!("Decryption failed: {}", e)))?;

        String::from_utf8(plaintext)
            .map_err(|e| SecretError::EncryptionError(format!("Invalid UTF-8: {}", e)))
    }

    /// Generate a random salt
    fn generate_salt() -> [u8; 32] {
        let mut salt = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
        salt
    }

    /// Generate a random nonce
    fn generate_nonce() -> [u8; 12] {
        let mut nonce = [0u8; 12];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
        nonce
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> SecretResult<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SecretError::internal(format!("Serialization failed: {}", e)))
    }

    /// Deserialize from JSON.
    /// Detects the old serialization format (which had a top-level `nonce` field)
    /// and returns a clear error directing users to re-create their secret store.
    pub fn from_json(json: &str) -> SecretResult<Self> {
        let raw: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| SecretError::internal(format!("Deserialization failed: {}", e)))?;

        // Detect old format: if the JSON contains a top-level "nonce" field, it's
        // the pre-per-secret-nonce format that is no longer compatible.
        if raw.get("nonce").is_some() {
            return Err(SecretError::InvalidFormat(
                "This secret store uses the old serialization format (shared nonce). \
                 It is incompatible with the current version which uses per-secret nonces. \
                 Please re-create your secret store. See BREAKING_CHANGES.md for details."
                    .to_string(),
            ));
        }

        serde_json::from_value(raw)
            .map_err(|e| SecretError::internal(format!("Deserialization failed: {}", e)))
    }

    /// Save to file
    pub async fn save_to_file(&self, path: &str) -> SecretResult<()> {
        let json = self.to_json()?;
        tokio::fs::write(path, json)
            .await
            .map_err(SecretError::IoError)
    }

    /// Load from file
    pub async fn load_from_file(path: &str) -> SecretResult<Self> {
        let json = tokio::fs::read_to_string(path)
            .await
            .map_err(SecretError::IoError)?;
        Self::from_json(&json)
    }
}

impl Default for EncryptedSecretStore {
    fn default() -> Self {
        Self {
            secrets: HashMap::new(),
            salt: general_purpose::STANDARD.encode(Self::generate_salt()),
        }
    }
}

/// Key derivation utilities
pub struct KeyDerivation;

impl KeyDerivation {
    /// Derive a key from a password using PBKDF2
    pub fn derive_key_from_password(password: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
        let mut key = [0u8; 32];
        let _ = pbkdf2::pbkdf2::<hmac::Hmac<sha2::Sha256>>(
            password.as_bytes(),
            salt,
            iterations,
            &mut key,
        );
        key
    }

    /// Generate a secure random key
    pub fn generate_random_key() -> [u8; 32] {
        Aes256Gcm::generate_key(&mut OsRng).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_encrypted_secret_store_basic() {
        let (mut store, key) = EncryptedSecretStore::new().unwrap();

        // Add a secret
        store
            .add_secret(&key, "test_secret", "secret_value")
            .unwrap();

        // Retrieve the secret
        let value = store.get_secret(&key, "test_secret").unwrap();
        assert_eq!(value, "secret_value");

        // Check metadata
        assert!(store.has_secret("test_secret"));
        assert_eq!(store.secret_count(), 1);

        let secrets = store.list_secrets();
        assert_eq!(secrets.len(), 1);
        assert!(secrets.contains(&"test_secret".to_string()));
    }

    #[tokio::test]
    async fn test_encrypted_secret_store_multiple_secrets() {
        let (mut store, key) = EncryptedSecretStore::new().unwrap();

        // Add multiple secrets
        store.add_secret(&key, "secret1", "value1").unwrap();
        store.add_secret(&key, "secret2", "value2").unwrap();
        store.add_secret(&key, "secret3", "value3").unwrap();

        // Retrieve all secrets
        assert_eq!(store.get_secret(&key, "secret1").unwrap(), "value1");
        assert_eq!(store.get_secret(&key, "secret2").unwrap(), "value2");
        assert_eq!(store.get_secret(&key, "secret3").unwrap(), "value3");

        assert_eq!(store.secret_count(), 3);
    }

    #[tokio::test]
    async fn test_encrypted_secret_store_wrong_key() {
        let (mut store, key1) = EncryptedSecretStore::new().unwrap();
        let (_, key2) = EncryptedSecretStore::new().unwrap();

        // Add secret with key1
        store
            .add_secret(&key1, "test_secret", "secret_value")
            .unwrap();

        // Try to retrieve with wrong key
        let result = store.get_secret(&key2, "test_secret");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_encrypted_secret_store_not_found() {
        let (store, key) = EncryptedSecretStore::new().unwrap();

        let result = store.get_secret(&key, "nonexistent");
        assert!(result.is_err());

        match result.unwrap_err() {
            SecretError::NotFound { name } => {
                assert_eq!(name, "nonexistent");
            }
            _ => panic!("Expected NotFound error"),
        }
    }

    #[tokio::test]
    async fn test_encrypted_secret_store_remove() {
        let (mut store, key) = EncryptedSecretStore::new().unwrap();

        store
            .add_secret(&key, "test_secret", "secret_value")
            .unwrap();
        assert!(store.has_secret("test_secret"));

        let removed = store.remove_secret("test_secret");
        assert!(removed);
        assert!(!store.has_secret("test_secret"));

        let removed_again = store.remove_secret("test_secret");
        assert!(!removed_again);
    }

    #[tokio::test]
    async fn test_encrypted_secret_store_serialization() {
        let (mut store, key) = EncryptedSecretStore::new().unwrap();

        store.add_secret(&key, "secret1", "value1").unwrap();
        store.add_secret(&key, "secret2", "value2").unwrap();

        // Serialize to JSON
        let json = store.to_json().unwrap();

        // Deserialize from JSON
        let restored_store = EncryptedSecretStore::from_json(&json).unwrap();

        // Verify secrets are still accessible
        assert_eq!(
            restored_store.get_secret(&key, "secret1").unwrap(),
            "value1"
        );
        assert_eq!(
            restored_store.get_secret(&key, "secret2").unwrap(),
            "value2"
        );
    }

    #[tokio::test]
    async fn test_each_secret_gets_unique_nonce() {
        let (mut store, key) = EncryptedSecretStore::new().unwrap();

        // Encrypt the same value twice - should produce different ciphertexts
        store.add_secret(&key, "secret_a", "same_value").unwrap();
        store.add_secret(&key, "secret_b", "same_value").unwrap();

        let encrypted_a = store.secrets.get("secret_a").unwrap();
        let encrypted_b = store.secrets.get("secret_b").unwrap();

        // Different nonces means different ciphertexts even for the same plaintext
        assert_ne!(encrypted_a, encrypted_b);

        // Both should decrypt to the same value
        assert_eq!(store.get_secret(&key, "secret_a").unwrap(), "same_value");
        assert_eq!(store.get_secret(&key, "secret_b").unwrap(), "same_value");
    }

    #[test]
    fn test_old_format_with_nonce_field_rejected() {
        // Simulate the old serialization format that had a top-level "nonce" field
        let old_format_json = r#"{
            "secrets": {},
            "salt": "dGVzdHNhbHQ=",
            "nonce": "dGVzdG5vbmNl"
        }"#;

        let result = EncryptedSecretStore::from_json(old_format_json);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("old serialization format"),
            "Expected old-format error, got: {}",
            err_msg
        );
    }

    #[test]
    fn test_key_derivation() {
        let password = "test_password";
        let salt = b"test_salt_bytes_32_chars_long!!";
        let iterations = 10000;

        let key1 = KeyDerivation::derive_key_from_password(password, salt, iterations);
        let key2 = KeyDerivation::derive_key_from_password(password, salt, iterations);

        // Same password and salt should produce same key
        assert_eq!(key1, key2);

        // Different salt should produce different key
        let different_salt = b"different_salt_bytes_32_chars!";
        let key3 = KeyDerivation::derive_key_from_password(password, different_salt, iterations);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_random_key_generation() {
        let key1 = KeyDerivation::generate_random_key();
        let key2 = KeyDerivation::generate_random_key();

        // Random keys should be different
        assert_ne!(key1, key2);

        // Keys should be 32 bytes
        assert_eq!(key1.len(), 32);
        assert_eq!(key2.len(), 32);
    }
}
