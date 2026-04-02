# wrkflw-secrets

Secrets management for wrkflw workflow execution. Provides secure handling of secrets with multiple providers, encryption, masking, and GitHub Actions-compatible `${{ secrets.* }}` substitution.

## Features

- **Providers**: environment variables, files (JSON/YAML/.env), HashiCorp Vault, AWS Secrets Manager, Azure Key Vault, GCP Secret Manager
- **Encryption**: AES-256-GCM encrypted storage for secrets at rest
- **Masking**: automatic masking of secrets in logs (GitHub tokens, AWS keys, JWTs, etc.)
- **Substitution**: GitHub Actions-compatible `${{ secrets.* }}` and `${{ secrets.provider:name }}` syntax
- **Caching**: optional TTL-based cache for frequently accessed secrets
- **Rate limiting**: built-in protection against secret access abuse
- **Validation**: comprehensive input validation for secret names and values

## Quick Start

```rust
use wrkflw_secrets::prelude::*;

#[tokio::main]
async fn main() -> SecretResult<()> {
    let manager = SecretManager::default().await?;

    std::env::set_var("GITHUB_TOKEN", "ghp_your_token_here");
    let secret = manager.get_secret("GITHUB_TOKEN").await?;

    // Substitute in templates
    let mut sub = SecretSubstitution::new(&manager);
    let resolved = sub.substitute("Bearer ${{ secrets.GITHUB_TOKEN }}").await?;

    // Mask secrets in logs
    let mut masker = SecretMasker::new();
    masker.add_secret(secret.value());
    println!("{}", masker.mask(&resolved));

    Ok(())
}
```

## Configuration

Create `~/.wrkflw/secrets.yml`:

```yaml
default_provider: env
enable_masking: true
timeout_seconds: 30
enable_caching: true
cache_ttl_seconds: 300

providers:
  env:
    type: environment
    prefix: "WRKFLW_SECRET_"
  file:
    type: file
    path: "~/.wrkflw/secrets.json"
  vault:
    type: vault
    url: "https://vault.example.com"
    auth:
      method: token
      token: "${VAULT_TOKEN}"
    mount_path: "secret"
```

## Feature Flags

```toml
[dependencies]
wrkflw-secrets = { version = "0.7", features = ["vault-provider", "aws-provider"] }
```

Available: `env-provider` (default), `file-provider` (default), `vault-provider`, `aws-provider`, `azure-provider`, `gcp-provider`, `all-providers`.

See the [secrets demo](../../examples/secrets-demo/) for end-to-end usage examples.
