# Breaking Changes

## Shell now matches GitHub Actions invocation (v0.7.3)

The `bash` shell now executes with `bash --noprofile --norc -e -o pipefail -c`, matching GitHub Actions behavior. The `sh` shell uses `sh -e -c`. This means:

- Scripts exit immediately on the first command that returns a non-zero exit code (`-e` / errexit)
- In bash, a failure in any command of a pipeline causes the whole pipeline to fail (`-o pipefail`)
- User profile/rc files are not sourced (`--noprofile --norc`)

### Why

GitHub Actions runs `bash` steps with `bash --noprofile --norc -e -o pipefail {0}`. The previous wrkflw behavior of `bash -c` (without `-e` or `pipefail`) allowed scripts to silently continue past failing commands, which diverged from GHA semantics and could mask real failures.

### Impact

Multi-command `run:` scripts that relied on intermediate commands failing without aborting the step will now fail at the first non-zero exit. Piped commands where an earlier stage fails will also now fail. For example:

```yaml
- run: |
    false        # This now aborts the step
    echo "This no longer runs"

- run: |
    false | echo "pipeline now fails too"
```

### Migration

If a step intentionally tolerates command failures, either:

- Append `|| true` to the specific command: `might-fail || true`
- Use `continue-on-error: true` on the step
- Add `set +e` or `set +o pipefail` at the top of the script to opt out selectively

---

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
