# Secrets Management

WRKFLW supports GitHub Actions-compatible `${{ secrets.* }}` syntax with multiple providers:

```bash
# Environment variables (simplest)
export GITHUB_TOKEN="ghp_..."
wrkflw run .github/workflows/ci.yml

# File-based secrets (JSON, YAML, or .env format)
# On Linux, put them in ~/.config/wrkflw/secrets/ (read by the `file` provider)
```

On GitHub every job has a `GITHUB_TOKEN`; here `secrets.GITHUB_TOKEN` and `github.token` are empty unless a provider has one, so a step that calls the GitHub API, such as `gh attestation verify`, fails with a 401. `WRKFLW_JOB_GITHUB_TOKEN=1` gives jobs this machine's token where no provider did: `GITHUB_TOKEN`, then `GH_TOKEN`, then what `gh auth token` prints. It is your own token, with your access, not a job's short-lived one scoped to the repository, so it is off by default: turn it on for workflows you trust.

A secret no provider has expands to an empty string, as on GitHub. `wrkflw plan` lists these, and a step that reads one ends its output with a warning.

Supported providers: environment variables, file-based, HashiCorp Vault, AWS Secrets Manager, Azure Key Vault, Google Cloud Secret Manager. See the [secrets demo](../examples/secrets-demo/) for detailed examples.
