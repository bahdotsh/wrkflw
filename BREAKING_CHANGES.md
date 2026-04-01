# Breaking Changes

## EncryptedSecretStore serialization format (v0.7.3)

The `EncryptedSecretStore` struct in `crates/secrets/src/storage.rs` has changed its serialization format:

- The shared `nonce` field has been **removed** from the struct.
- Each secret now stores its own random nonce **prepended to the ciphertext** (12 bytes nonce + ciphertext, then base64-encoded).

### Why

The previous design reused a single nonce across all secrets encrypted with the same key. Nonce reuse under AES-GCM is a critical vulnerability — it allows an attacker to XOR ciphertexts to recover plaintext differences and potentially forge authenticated messages.

### Impact

- Any `EncryptedSecretStore` serialized with the old format (containing a top-level `nonce` field) **cannot be deserialized** by the new code.
- Old ciphertexts (which did not have the nonce prepended) **cannot be decrypted** by the new code.

### Migration

There is no automatic migration. Users who have persisted encrypted secret stores must re-create them:

1. Decrypt all secrets using the old code (if still accessible).
2. Upgrade to the new version.
3. Re-encrypt and store the secrets.

### Affected API

- `EncryptedSecretStore::from_data` — dropped the `nonce: String` parameter.
- `EncryptedSecretStore` JSON serialization — no longer includes the `nonce` field.
