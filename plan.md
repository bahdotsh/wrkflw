# GHA Emulation Gaps: Integration Plan

## Overview

Three fully-implemented modules (`artifacts.rs`, `cache.rs`, `workflow_commands.rs`) exist in `crates/executor/src/` but are dead code — never called from the engine. This plan tracks integrating them and the follow-up work needed afterward.

---

## Current PR: Integrate Dead-Code Modules

### Step 1: Thread ArtifactStore Through Execution Contexts

`ArtifactStore` must be shared across steps and jobs (upload in step A, download in step B). It uses `Arc<RwLock<>>` internally.

- Add `Clone` derive to `ArtifactStore`
- Create in `execute_github_workflow()` after `workspace_dir`
- Add `artifact_store` parameter/field to: `execute_job_batch()`, `execute_job_with_matrix()`, `execute_matrix_job()`, `JobExecutionContext`, `MatrixExecutionContext`, `StepExecutionContext`
- Thread through all call sites (~6 production + ~15 test sites)

### Step 2: Artifact Emulation (`actions/upload-artifact`, `actions/download-artifact`)

- Add emulation branches in `execute_step()` after `actions/checkout` (line ~2480)
- Extract `with` params (`name`, `path`), preprocess expressions
- Upload: `artifact_store.upload(name, path_pattern, workspace)`
- Download: `artifact_store.download(name, target_dir)` or download all if name absent
- Add `preprocess_with_value()` helper for `${{ }}` resolution in `with` params

### Step 3: Cache Emulation (`actions/cache`)

- Add emulation branch in `execute_step()` after artifact handling
- `CacheStore::new()` created on-demand (filesystem-backed, no threading needed)
- Extract `with` params (`key`, `path`, `restore-keys`)
- Restore if cache exists, write `cache-hit=true/false` to GITHUB_OUTPUT file
- On miss, attempt save immediately (simplified vs. GHA's dual-phase model)
- Update `emulation.rs` stub to reflect cache is now supported

### Step 4: Workflow Commands Processing

- Create `process_workflow_commands()` helper in engine.rs
- Parse `::command::` lines from step stdout via `workflow_commands::parse_workflow_commands()`
- Handle: `SetOutput` → insert into step_outputs_map, `Error/Warning/Notice` → log with location, `Debug` → debug log, `AddMask` → debug log (full support deferred)
- Call in all 3 step loops: `execute_job`, `execute_matrix_job`, `execute_composite_action`
- Insert BEFORE `apply_step_environment_updates` so GITHUB_OUTPUT file takes precedence over deprecated `::set-output::`

### Step 5: Cleanup

- Remove `#[allow(dead_code)]` from `secret_masker` field in `StepExecutionContext`
- Dead-code warnings for artifacts/cache disappear naturally once integrated

---

## Follow-Up PR 1: SecretMasker Integration

**Problem:** `SecretMasker` is threaded as `Option<&SecretMasker>` (immutable), but `add_secret(&mut self)` requires mutability. This blocks:
- `::add-mask::` workflow command support (dynamically adding secrets at runtime)
- Output masking (calling `masker.mask()` on step output before logging)

**Plan:**
- [ ] Wrap `SecretMasker` in `Arc<Mutex<SecretMasker>>`
- [ ] Change creation at `execute_github_workflow` line ~154
- [ ] Change all context struct fields from `Option<&'a SecretMasker>` to `Option<Arc<Mutex<SecretMasker>>>`
- [ ] In `process_workflow_commands`, acquire lock and call `add_secret()` for `AddMask` commands
- [ ] In both step loops, apply `masker.lock().unwrap().mask(&result.output)` before logging
- [ ] Touches every function signature in the call chain — significant but mechanical churn

## Follow-Up PR 2: Cache Dual-Phase Support

**Problem:** Real GHA has a dual-phase cache model — restore at pre-step, save at post-step (via post action hook). Our model does restore-or-save immediately, which fails when the cached path doesn't exist yet (e.g., `node_modules` before `npm install`).

**Plan:**
- [ ] Add post-step hook infrastructure to the engine
- [ ] Cache restore happens immediately (current behavior)
- [ ] Cache save is deferred to post-step: after all steps complete, save paths that were restored with a cache miss
- [ ] Track "pending saves" in a per-job structure

## Follow-Up PR 3: Artifact Store Improvements

**Problem:** `artifacts.rs` uses synchronous `std::fs` in async context. `walk_files` follows symlinks without protection.

**Plan:**
- [ ] Add symlink protection to `walk_files` (canonicalize paths, reject escapes)
- [ ] Consider `tokio::fs` or `spawn_blocking` for file I/O if performance is an issue
- [ ] Add retention/size-limit support to match GHA artifact behavior
