# Breaking Changes

> The entries below ship in the next release (post-v0.7.3, currently unreleased on `main`).

## `wrkflw run --event` requires change-set input by default (Unreleased)

`wrkflw run` now supports trigger-aware filtering via `--event`, `--diff`,
and `--changed-files`. When any of those flags is passed, the CLI runs a
prefilter that evaluates the workflow's `on:` block against the simulated
event context before execution — if the triggers don't match, the
workflow is skipped with a clean `exit 0`.

Under the new **`--strict-filter` default (on)**, running with `--event`
alone — without any way for wrkflw to know the change set — is rejected
with `exit 1`. Previously the CLI proceeded silently with an empty
change set, which meant every workflow gated on `paths:` was reported as
"not triggering" for reasons the user could not see.

Strict mode also rejects simulating `pull_request` / `pull_request_target`
without `--base-branch`, because GitHub Actions evaluates `branches:`
filters on PR events against the target branch, and the same silent-skip
mode applies there.

### Why

`wrkflw run --event push` used to proceed with `changed_files = []`,
causing every `paths:`-gated workflow to be silently rejected at
evaluation time. Users would then file "why didn't my workflow fire?"
issues for the non-obvious reason that no change set had been supplied.
Strict mode turns that silent failure into a loud, actionable error up
front — it is the default countermeasure for the same class of silent
skip the rest of the trigger-filter work patched iteratively.

### Impact

Scripts that invoked `wrkflw run --event <name> <workflow>` without
also passing `--diff` or `--changed-files` will now fail with:

```
Error: --event was supplied without --diff or --changed-files, so no
changed files are known and any workflow with a `paths:` filter would
be silently skipped. Pass --diff to auto-detect from git, --changed-files
to supply them explicitly, or --no-strict-filter to proceed anyway.
```

Scripts that invoked `wrkflw run --event pull_request <workflow>` without
`--base-branch` will similarly fail with a pointer at the missing flag.

`wrkflw watch --event pull_request` behaves the same way: missing
`--base-branch` is rejected under strict mode.

### Migration

Pick the option that matches your intent:

- **CI script that wants to run only workflows the diff would trigger:**
  add `--diff` (auto-detect base branch via `origin/HEAD` → `main` →
  `master` → `HEAD~1`) or pin with `--diff-base <ref>`.

  ```bash
  wrkflw run --diff --event push .github/workflows/ci.yml
  ```

- **CI script that has its own change set (e.g. from `git diff` in a
  wrapper):** pass it explicitly.

  ```bash
  wrkflw run --event push --changed-files src/main.rs,Cargo.toml \
      .github/workflows/ci.yml
  ```

- **Simulating a pull request locally:** add `--base-branch`.

  ```bash
  wrkflw run --event pull_request --base-branch main --diff \
      .github/workflows/ci.yml
  ```

- **Legacy warn-and-proceed behavior (not recommended):** opt out with
  `--no-strict-filter`. This restores the pre-strict-filter behavior of
  logging a warning and running every workflow anyway. Use this only if
  your scripts have already adapted to the old silent-skip semantics and
  you cannot change them right now.

### Prefilter exit codes

- `0` — triggers matched, workflow ran to completion (or was skipped
  because its triggers did not match the event). A skipped workflow
  prints `Workflow skipped: <reason>` and exits 0 to match GitHub
  Actions' own "nothing to do" semantics.
- `1` — something went wrong: flag validation, git error, strict-filter
  rejection, or workflow execution failure.

### Affected CLI

- `wrkflw run` — new flags: `--event`, `--diff`, `--changed-files`,
  `--diff-base`, `--diff-head`, `--base-branch`, `--activity-type`,
  `--strict-filter` (default on), `--no-strict-filter`.
- `wrkflw watch` — new subcommand. Same strict-filter gate for
  `--event pull_request` + missing `--base-branch`.

---

## Shell now matches GitHub Actions invocation (Unreleased)

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

## EncryptedSecretStore serialization format (Unreleased)

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
