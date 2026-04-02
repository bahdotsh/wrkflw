# Secrets Management Demo

Demonstrates wrkflw's secrets management system with multiple providers and GitHub Actions-compatible syntax.

## Quick Start

### Environment Variables (Simplest)

```bash
export GITHUB_TOKEN="ghp_your_token_here"
export API_KEY="your_api_key"
```

Create a workflow that uses secrets:

```yaml
# .github/workflows/secrets-demo.yml
name: Secrets Demo
on: [push]

jobs:
  test-secrets:
    runs-on: ubuntu-latest
    steps:
      - name: Use GitHub Token
        run: |
          curl -H "Authorization: Bearer ${{ secrets.GITHUB_TOKEN }}" \
            https://api.github.com/user

      - name: Use API Key
        env:
          KEY: ${{ secrets.API_KEY }}
        run: echo "Using API key"
```

Run with wrkflw:

```bash
wrkflw run .github/workflows/secrets-demo.yml
```

### File-based Secrets

Create a secrets file in JSON, YAML, or `.env` format:

```json
{
  "API_KEY": "your_api_key_here",
  "DB_PASSWORD": "secure_database_password",
  "GITHUB_TOKEN": "ghp_your_github_token"
}
```

Configure wrkflw:

```yaml
# ~/.wrkflw/secrets.yml
default_provider: file
enable_masking: true
timeout_seconds: 30

providers:
  file:
    type: file
    path: "./secrets.json"
```

### External Secret Managers

For production, use HashiCorp Vault, AWS Secrets Manager, Azure Key Vault, or GCP Secret Manager:

```yaml
# ~/.wrkflw/secrets.yml
default_provider: vault
enable_masking: true
enable_caching: true
cache_ttl_seconds: 300

providers:
  vault:
    type: vault
    url: "https://vault.company.com"
    auth:
      method: token
      token: "${VAULT_TOKEN}"
    mount_path: "secret"

  aws:
    type: aws_secrets_manager
    region: "us-east-1"
```

## Secret Masking

wrkflw automatically masks secrets in logs:

```
# Original: "token": "ghp_1234567890abcdef"
# Masked:   "token": "ghp_***"
```

Auto-detected patterns: GitHub tokens (`ghp_*`, `ghs_*`, `gho_*`), AWS keys (`AKIA*`), JWTs, and generic API keys.

## Multi-Provider Usage

Reference secrets from specific providers:

```yaml
steps:
  - run: echo "${{ secrets.env:API_KEY }}"      # from env provider
  - run: echo "${{ secrets.file:DB_PASSWORD }}"  # from file provider
  - run: echo "${{ secrets.vault:api-key }}"     # from Vault
```

## Security Best Practices

- **Development**: use environment variables or file-based secrets
- **Production**: use external secret managers (Vault, AWS, Azure, GCP)
- Always enable `enable_masking: true`
- Rotate secrets regularly
- Use least-privilege access for providers

See the [`wrkflw-secrets` crate README](../../crates/secrets/README.md) for full API documentation.
