//! Trigger prefilter orchestration for the `wrkflw run` / `wrkflw watch`
//! commands.
//!
//! Extracted from `main.rs` so the flag-matrix decision logic
//! (`--diff` vs `--diff-base` vs `--changed-files`, strict vs
//! non-strict, `pull_request` vs `push`) lives in one module with
//! its tests. The orchestrator functions here return `Result` rather
//! than calling `std::process::exit` from inside their bodies — the
//! two CLI commands own the exit policy. This is what makes the
//! flag matrix unit-testable: before this shape the only way to
//! exercise a branch was to spawn a subprocess and observe the exit
//! code.
//!
//! Shared between `run_workflow_cmd` and `watch_cmd`:
//!   - [`effective_strict_filter`] — resolves the
//!     `--strict-filter` / `--no-strict-filter` flag pair.
//!   - [`validate_event_requires_base_branch`] — the rejection
//!     helper for `pull_request` / `pull_request_target` without a
//!     `--base-branch`. Both commands invoke this so the error text
//!     and warning text stay in exactly one place.
//!
//! Only used by `run_workflow_cmd`:
//!   - [`PrefilterDecision`], [`PrefilterRequest`],
//!     [`run_trigger_prefilter`], [`build_event_context`],
//!     [`apply_base_branch`].

use std::path::{Path, PathBuf};

/// Decision returned by [`run_trigger_prefilter`].
///
/// Previously the prefilter called `std::process::exit` from half a
/// dozen sites deep inside its body, which made the flag-matrix
/// untestable — a unit test would have to spawn a subprocess just to
/// observe the exit code. Returning a plain enum lets the orchestrator
/// own the decision and hand `main()` the responsibility of calling
/// `process::exit`. The side-effects that need to happen before the
/// decision (warning drains, verbose logging) still live in the
/// orchestrator body; only the exit is deferred.
#[derive(Debug)]
pub(crate) enum PrefilterDecision {
    /// The workflow's triggers matched the event context — main
    /// should proceed to execute the workflow.
    Proceed,
    /// The workflow's triggers did NOT match — main should print the
    /// reason (already formatted for the user) and exit 0.
    Skip { reason: String },
}

/// Resolve the effective `--strict-filter` / `--no-strict-filter`
/// bool toggle. `--no-strict-filter` wins over the default-true
/// `--strict-filter` via clap's `conflicts_with`, so the effective
/// value is `strict AND NOT no_strict`. Extracted so the two call
/// sites (`wrkflw run` and `wrkflw watch`) cannot drift apart — if
/// a third host grows the same flag pair, it gets the same
/// coalescing for free.
pub(crate) fn effective_strict_filter(strict: bool, no_strict: bool) -> bool {
    strict && !no_strict
}

/// Reject `pull_request` / `pull_request_target` invocations that
/// have no `--base-branch` under strict mode, or warn-and-proceed
/// under non-strict mode.
///
/// Shared between `apply_base_branch` (which first stamps any
/// supplied base onto the event context, then delegates the
/// validation here on the `None` branch) and the `wrkflw watch`
/// command's pre-flight CLI check. The two hosts used to keep
/// independent copies of the same string, which was exactly the
/// kind of drift the prefilter pattern was introduced to prevent.
///
/// The wording is intentionally host-neutral — it names the event
/// rather than the verb ("simulating" / "watching") so the same
/// text reads correctly from both `wrkflw run` and `wrkflw watch`.
/// An earlier revision hard-coded "simulating", which was wrong for
/// the watch command and regressed watch-mode diagnostics; see the
/// helper's `non_pr_event` / `pull_request_target` test pair for
/// the pin.
///
/// Returns `Ok(())` for non-PR events regardless of flag state.
/// Returns `Err(String)` under `strict_filter = true` with the
/// caller-ready error text (no `Error:` prefix — the orchestrator
/// adds one). Logs a warning under `strict_filter = false` and
/// returns `Ok(())` so the legacy warn-and-proceed path is
/// preserved.
pub(crate) fn validate_event_requires_base_branch(
    event_name: &str,
    strict_filter: bool,
) -> Result<(), String> {
    if !matches!(event_name, "pull_request" | "pull_request_target") {
        return Ok(());
    }
    if strict_filter {
        return Err(format!(
            "event `{}` without --base-branch is rejected under --strict-filter: \
             `branches:` filters on pull_request events evaluate against the PR target \
             branch, and without one every such workflow is silently reported as not \
             triggering. Pass --base-branch <name>, or use --no-strict-filter to proceed.",
            event_name
        ));
    }
    wrkflw_logging::warning(&format!(
        "event `{}` without --base-branch: workflows that use `branches:` to constrain \
         the PR target branch will be reported as not triggering. \
         --no-strict-filter allowed this to proceed.",
        event_name,
    ));
    Ok(())
}

/// Bundled inputs for the `wrkflw run` trigger prefilter.
///
/// Grouping these into a single struct collapses the previous 8-argument
/// `run_trigger_prefilter_or_exit` into a more reviewable shape, and lets
/// the orchestrator pass the request through to its private helpers
/// ([`build_event_context`], [`apply_base_branch`]) without dragging an
/// ever-growing positional list.
pub(crate) struct PrefilterRequest<'a> {
    pub(crate) workflow_path: &'a Path,
    pub(crate) event: Option<&'a String>,
    pub(crate) diff: bool,
    pub(crate) changed_files: Option<&'a Vec<String>>,
    /// `None` means the user did not pass `--diff-base` and we should fall
    /// back to `auto_detect_context_default_base` (origin/HEAD → main →
    /// master → HEAD~1). Previously this was a `&str` defaulting to
    /// `"HEAD"`, which made the smart detection unreachable from the CLI
    /// and silently restricted `--diff` to uncommitted-only changes.
    pub(crate) diff_base: Option<&'a str>,
    pub(crate) diff_head: Option<&'a String>,
    pub(crate) base_branch: Option<&'a String>,
    pub(crate) activity_type: Option<&'a String>,
    pub(crate) verbose: bool,
    /// When true, known-incomplete filter contexts (missing changed
    /// files, missing base branch on a PR event) exit with a
    /// diagnostic instead of log-warning-and-proceeding. The review
    /// flagged the old warn-and-proceed as exactly the silent-skip
    /// mode the rest of this PR had been patching; strict mode is
    /// the default-on countermeasure.
    pub(crate) strict_filter: bool,
}

/// Build an event context from the user's CLI flags and decide
/// whether the workflow should run.
///
/// Returns:
/// - `Ok(PrefilterDecision::Proceed)` — triggers matched, main should
///   continue into the executor path.
/// - `Ok(PrefilterDecision::Skip { reason })` — triggers did not match,
///   main should print the reason and exit 0.
/// - `Err(msg)` — something went wrong building the context or parsing
///   the workflow, main should print the message and exit 1.
///
/// All `std::process::exit` calls have been lifted out of this
/// function so the decision logic is testable without spawning a
/// subprocess — the flag matrix (`--diff` vs `--diff-base` vs
/// `--changed-files`, strict vs non-strict, pull_request vs push) is
/// the sort of thing that benefits most from unit tests, and the old
/// shape made that impossible.
pub(crate) async fn run_trigger_prefilter(
    req: PrefilterRequest<'_>,
) -> Result<PrefilterDecision, String> {
    // `wrkflw run` expects a single workflow file. Catch directory paths up
    // front with a clear error; otherwise the user sees a confusing
    // "Error parsing workflow" from the YAML parser further down.
    if !req.workflow_path.is_file() {
        if req.workflow_path.is_dir() {
            return Err(format!(
                "--diff/--event/--changed-files require a single workflow file, not a directory.\n\
                 Hint: point at a specific .yml file, or use `wrkflw watch {}` for directory-wide watching.",
                req.workflow_path.display()
            ));
        } else {
            return Err(format!(
                "workflow file not found: {}",
                req.workflow_path.display()
            ));
        }
    }

    let event_name = req.event.cloned().unwrap_or_else(|| "push".to_string());

    // Root git operations at the git repo root when possible, so behavior
    // is consistent regardless of the directory the user ran `wrkflw`
    // from. Falls back to process CWD if we're not inside a repo.
    //
    // `find_repo_root_detailed` is a sync shell-out not covered by
    // `GIT_COMMAND_TIMEOUT`; wrap in `spawn_blocking` so a hung git
    // (credential prompt, stuck network mount) cannot freeze the reactor.
    //
    // We use the classified `_detailed` form so each failure mode
    // surfaces its own diagnostic. `NotInRepository` is a legitimate
    // soft failure (the user may have passed `--changed-files` without
    // needing any git helper) — fall back to `None` and let the
    // downstream git calls decide whether they need a repo. Every
    // other failure (git-not-installed, timeout, other) is loud and
    // fatal because the user has something actionable to fix.
    let repo_root: Option<PathBuf> =
        match tokio::task::spawn_blocking(wrkflw_trigger_filter::find_repo_root_detailed).await {
            Ok(Ok(p)) => Some(p),
            Ok(Err(wrkflw_trigger_filter::FindRepoRootError::NotInRepository)) => None,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(join_err) => return Err(format!("find_repo_root task panicked: {}", join_err)),
        };
    let cwd_for_git: Option<&Path> = repo_root.as_deref();

    let mut event_context = build_event_context(&req, &event_name, cwd_for_git).await?;
    apply_base_branch(
        &mut event_context,
        &event_name,
        req.base_branch,
        req.strict_filter,
    )?;
    // Stamp `--activity-type` onto the context. `EventContext::activity_type`
    // is the field GitHub Actions matches its `types:` filter against —
    // without it, every workflow with `types: [opened, ...]` is silently
    // rejected for "no activity type in context", which is exactly the
    // silent-skip failure mode this PR is built to prevent.
    if let Some(activity) = req.activity_type {
        event_context.activity_type = Some(activity.clone());
    }

    // Surface any non-fatal warnings collected while building the
    // context (e.g. `git ls-files --others` failed, so untracked
    // files were dropped). The trigger-filter crate no longer logs
    // these itself — it collects them as data and hands them to
    // hosts via `EventContext::warnings`, so we own the rendering
    // policy here and can stay consistent with the rest of the CLI's
    // colorization.
    //
    // `take()` (rather than read-only iteration) is load-bearing:
    // `EventContext::warnings` is a `MustDrainWarnings` whose Drop
    // check fires in debug builds if a non-empty buffer is dropped
    // without being observed. Draining satisfies the contract and
    // guarantees the CLI path cannot silently reintroduce the
    // warning-loss failure mode the rest of this PR has been
    // plugging.
    for w in event_context.warnings.take() {
        wrkflw_logging::warning(&w);
    }

    if req.verbose {
        wrkflw_logging::info(&format!(
            "Trigger filter: event={}, branch={:?}, base_branch={:?}, activity_type={:?}, changed_files={:?}",
            event_context.event_name,
            event_context.branch,
            event_context.base_branch,
            event_context.activity_type,
            event_context.changed_files
        ));
    }

    // Parse workflow and evaluate trigger before executing.
    //
    // `load_trigger_config` performs blocking file I/O + YAML parsing
    // (documented in `wrkflw_trigger_filter::lib.rs`). Move it onto a
    // blocking thread so we don't stall the tokio reactor. The latency
    // hit for a single file is small, but the contract should match
    // the watcher and TUI, both of which already do this — drifting
    // here is exactly how the silent-failure holes accumulated.
    let workflow_path_owned = req.workflow_path.to_path_buf();
    let tf_config = wrkflw_trigger_filter::TriggerFilterConfig::default();
    let mut trigger_config = tokio::task::spawn_blocking(move || {
        // Route through the shared LRU cache so every wrkflw entry
        // point (CLI prefilter, TUI diff-filter, watcher hot loop)
        // contends over the same compiled-pattern store. Unifying
        // the three call sites was a review ask to prevent drift —
        // the same file never pays the YAML-parse cost twice.
        wrkflw_trigger_filter::load_trigger_config_cached(&workflow_path_owned, &tf_config)
    })
    .await
    .map_err(|e| format!("workflow parse task panicked: {}", e))?
    .map_err(|e| format!("parsing workflow: {}", e))?;
    // Drain parser-collected diagnostics (unknown event names, etc.)
    // — the library decouples from the log sink by design, so every
    // host must drain this field or reintroduce the silent-skip
    // failure mode. `take()` also satisfies the `MustDrainWarnings`
    // Drop-check contract that catches the regression in debug
    // builds.
    for w in trigger_config.warnings.take() {
        wrkflw_logging::warning(&w);
    }
    let match_result = wrkflw_trigger_filter::evaluate_trigger(&trigger_config, &event_context);

    if !match_result.matches {
        return Ok(PrefilterDecision::Skip {
            reason: match_result.reason,
        });
    }
    wrkflw_logging::info(&format!("Trigger matched: {}", match_result.reason));
    Ok(PrefilterDecision::Proceed)
}

/// Pick the right context-builder based on which flags the user supplied.
///
/// Returns a `Result<EventContext, String>` so the orchestrator owns the
/// `process::exit` policy — previously each branch called `exit` from
/// deep in the helper, which made the flag-matrix logic impossible to
/// unit-test without spawning a subprocess. The error string is ready
/// to be printed verbatim with an `Error:` prefix.
pub(crate) async fn build_event_context(
    req: &PrefilterRequest<'_>,
    event_name: &str,
    cwd_for_git: Option<&Path>,
) -> Result<wrkflw_trigger_filter::EventContext, String> {
    if let Some(files) = req.changed_files {
        // Validate every user-supplied entry before handing it to
        // the trigger-filter. Absolute paths, drive letters, and
        // `..` components break the repo-relative glob contract the
        // evaluator assumes; catching them up front produces a
        // "your flag was wrong" message instead of a session-long
        // "nothing matched" mystery.
        let normalized = wrkflw_trigger_filter::normalize_user_changed_files(files)
            .map_err(|e| format!("invalid --changed-files entry: {}", e))?;
        return wrkflw_trigger_filter::context_from_changed_files(
            event_name,
            normalized,
            cwd_for_git,
        )
        .await
        .map_err(|e| format!("failed to build event context: {}", e));
    }

    if req.diff {
        // Three branches:
        //   1. `--diff-head` set: explicit two-ref range. Honour
        //      `--diff-base` if given, default the base end of the range
        //      to `HEAD` so the range is well-formed.
        //   2. `--diff-base` set, no `--diff-head`: auto-detect against
        //      that base ref (working tree vs <base>).
        //   3. Neither: smart-detect via origin/HEAD → main → master →
        //      HEAD~1. This is the path the user gets from `--diff` alone,
        //      which previously was wired to "HEAD" and silently restricted
        //      the diff to uncommitted changes only.
        return if let Some(head) = req.diff_head {
            let base = req.diff_base.unwrap_or("HEAD");
            wrkflw_trigger_filter::context_from_diff_range(event_name, base, head, cwd_for_git)
                .await
        } else if let Some(base) = req.diff_base {
            wrkflw_trigger_filter::auto_detect_context(event_name, base, cwd_for_git).await
        } else {
            wrkflw_trigger_filter::auto_detect_context_default_base(
                event_name,
                cwd_for_git,
                req.verbose,
            )
            .await
        }
        .map_err(|e| format!("failed to get git diff: {}", e));
    }

    // `--event` was passed alone (no `--diff`, no `--changed-files`).
    // Running with an empty changed-files set means every `paths:`
    // filter silently rejects — the exact silent-skip failure mode
    // the rest of this PR has been plugging. In strict mode (the
    // default) refuse to proceed so CI scripts fail loudly and the
    // operator has something actionable to fix.
    if req.strict_filter {
        return Err(
            "--event was supplied without --diff or --changed-files, so no changed files \
             are known and any workflow with a `paths:` filter would be silently skipped. \
             Pass --diff to auto-detect from git, --changed-files to supply them \
             explicitly, or --no-strict-filter to proceed anyway."
                .to_string(),
        );
    }
    wrkflw_logging::warning(
        "--event was supplied without --diff or --changed-files; \
         path filters will not match because no changed files are known. \
         --no-strict-filter allowed this to proceed.",
    );
    wrkflw_trigger_filter::context_from_changed_files(event_name, vec![], cwd_for_git)
        .await
        .map_err(|e| format!("failed to build event context: {}", e))
}

/// Stamp the user-supplied `--base-branch` onto the event context, or
/// delegate to [`validate_event_requires_base_branch`] to reject /
/// warn when the event needs one but the user did not pass it.
///
/// The rejection + warning text lives in
/// [`validate_event_requires_base_branch`] so the watch command can
/// call it directly (without building an `EventContext`) and both
/// hosts produce the same diagnostics.
pub(crate) fn apply_base_branch(
    ctx: &mut wrkflw_trigger_filter::EventContext,
    event_name: &str,
    base_branch: Option<&String>,
    strict_filter: bool,
) -> Result<(), String> {
    if let Some(base) = base_branch {
        ctx.base_branch = Some(base.clone());
        return Ok(());
    }
    validate_event_requires_base_branch(event_name, strict_filter)
}

#[cfg(test)]
mod prefilter_tests {
    //! Unit coverage for the `run_trigger_prefilter` decision logic.
    //!
    //! These tests exist specifically because the previous
    //! `run_trigger_prefilter_or_exit` shape called `std::process::exit`
    //! from inside every failure branch, making the flag matrix
    //! impossible to exercise without spawning a subprocess. The
    //! refactor that returns `Result<PrefilterDecision, String>`
    //! lets us pin the "path is a directory", "path does not
    //! exist", and "workflow parses but does not match" branches
    //! here in-process.
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn directory_path_returns_err_with_watch_hint() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let dir = tmp.path().to_path_buf();
        let empty_files: Option<Vec<String>> = None;
        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &dir,
            event: Some(&event),
            diff: false,
            changed_files: empty_files.as_ref(),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("directory path must produce an Err");
        assert!(
            err.contains("single workflow file"),
            "err must explain the single-file constraint, got: {}",
            err
        );
        assert!(
            err.contains("wrkflw watch"),
            "err must suggest `wrkflw watch` for directory-wide watching, got: {}",
            err
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_path_returns_err_with_not_found() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let missing = tmp.path().join("does-not-exist.yml");
        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &missing,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("missing path must produce an Err");
        assert!(
            err.contains("not found"),
            "err must name the not-found case, got: {}",
            err
        );
    }

    /// Build a bare-bones git repo in `dir` with one committed file
    /// on branch `main`. Mirrors the `init_repo` helper in
    /// `crates/trigger-filter/src/git.rs` tests — duplicated here
    /// rather than lifted because this crate has no test-helpers
    /// module and a single-use helper doesn't justify one.
    fn init_repo_for_test(dir: &Path) -> bool {
        use std::process::Command as StdCommand;
        let status = StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "init", "--initial-branch=main"])
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            return false;
        }
        for (k, v) in [("user.email", "t@t.t"), ("user.name", "t")] {
            if StdCommand::new("git")
                .args(["-C", dir.to_str().unwrap(), "config", k, v])
                .status()
                .map(|s| !s.success())
                .unwrap_or(true)
            {
                return false;
            }
        }
        let path = dir.join("a.txt");
        if std::fs::write(&path, "1").is_err() {
            return false;
        }
        if StdCommand::new("git")
            .args(["-C", dir.to_str().unwrap(), "add", "a.txt"])
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return false;
        }
        if StdCommand::new("git")
            .args([
                "-C",
                dir.to_str().unwrap(),
                "commit",
                "-m",
                "init",
                "--no-gpg-sign",
            ])
            .status()
            .map(|s| !s.success())
            .unwrap_or(true)
        {
            return false;
        }
        true
    }

    fn git_available() -> bool {
        use std::process::Command as StdCommand;
        StdCommand::new("git")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn build_event_context_defaults_diff_base_to_head_when_only_diff_head_set() {
        // Regression pin for the `--diff-head` without `--diff-base`
        // branch at `build_event_context`'s `if let Some(head) =
        // req.diff_head` arm: the base end of the two-ref range
        // defaults to `"HEAD"` so the constructed range is
        // well-formed. Without a test this branch was reachable
        // from the CLI but never exercised in-process, and a
        // refactor that flipped the default to `"origin/HEAD"`
        // (or anything else) would silently break the flag matrix.
        //
        // We call `build_event_context` directly instead of
        // `run_trigger_prefilter` because the latter shells out to
        // `find_repo_root_detailed` against the process CWD — global
        // state that's not safe under cargo's parallel test runner.
        // The direct call takes a `cwd_for_git: Option<&Path>` which
        // we point at the tempdir repo, giving the test full
        // isolation.
        if !git_available() {
            return;
        }
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let repo = tmp.path().to_path_buf();
        if !init_repo_for_test(&repo) {
            return;
        }

        // Write a minimal workflow so the prefilter has something to
        // point at if the test ever extends to parsing. Not strictly
        // needed for `build_event_context`, which never reads the
        // file, but keeps the setup close to a real CLI invocation.
        let wf = repo.join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\non: push\njobs:\n  b:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write ci.yml");

        let event = "push".to_string();
        let head = "HEAD".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: true,
            changed_files: None,
            diff_base: None,
            diff_head: Some(&head),
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };

        let ctx = build_event_context(&req, "push", Some(&repo)).await.expect(
            "build_event_context must succeed when --diff-head=HEAD and --diff-base is absent",
        );

        // The branch under test constructs a range `base..head` and
        // runs `git diff --name-only` on it. Base defaults to HEAD,
        // so the range is `HEAD..HEAD` — an empty diff against a
        // fresh repo. The key invariants:
        //   1. No error (the branch was reached and git ran cleanly).
        //   2. `changed_files_explicit == true` (caller asked for a
        //      two-ref diff, so an empty result is authoritative —
        //      the diagnostic layer must NOT suggest passing --diff).
        //   3. `changed_files.is_empty()` (HEAD..HEAD trivially empty).
        assert!(
            ctx.changed_files_explicit,
            "two-ref diff must mark changed_files as explicit"
        );
        assert!(
            ctx.changed_files.is_empty(),
            "HEAD..HEAD diff must be empty, got {:?}",
            ctx.changed_files
        );

        // Drain warnings to satisfy MustDrainWarnings (none expected,
        // but the contract is the same as every other host).
        let mut ctx = ctx;
        let _ = ctx.warnings.take();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skip_decision_returned_when_trigger_does_not_match() {
        // A push workflow gated on `paths: ['irrelevant/**']` with an
        // explicit empty --changed-files list must resolve to
        // `Skip`, not an error. This is the load-bearing "user got
        // a clean exit 0 because their edit did not touch the
        // filter's paths" scenario the executor path depends on.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'irrelevant/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let changed: Vec<String> = vec!["src/main.rs".to_string()];
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: Some(&changed),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let decision = run_trigger_prefilter(req)
            .await
            .expect("should not error on a valid workflow");
        match decision {
            PrefilterDecision::Skip { reason } => {
                assert!(
                    reason.contains("paths"),
                    "skip reason must mention the paths filter, got: {}",
                    reason
                );
            }
            PrefilterDecision::Proceed => {
                panic!("expected Skip for non-matching paths, got Proceed");
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn strict_filter_rejects_event_alone_without_diff_or_changed_files() {
        // Regression pin for the strict-filter default-on gate in
        // `build_event_context`: passing `--event push` with neither
        // `--diff` nor `--changed-files` means the caller could not
        // supply a change set, so every `paths:`-gated workflow would
        // be silently rejected at evaluation time. Under strict mode
        // (the default) this must be a hard error up front instead,
        // pointing the user at the three escape hatches.
        //
        // This is the load-bearing CLI behavior change the
        // BREAKING_CHANGES.md entry documents — keeping the rejection
        // behavior pinned here prevents a future refactor from
        // silently flipping it back to warn-and-proceed.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        // Any parseable workflow works; `build_event_context` fails
        // before parsing.
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'src/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: true,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("strict mode must reject --event without --diff/--changed-files");
        assert!(
            err.contains("--diff") && err.contains("--changed-files"),
            "error must point the user at the three escape hatches, got: {}",
            err
        );
        assert!(
            err.contains("--no-strict-filter"),
            "error must name the legacy opt-out, got: {}",
            err
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn non_strict_filter_allows_event_alone_with_warning_and_empty_change_set() {
        // Mirror of the strict-mode test: with `--no-strict-filter`
        // the caller opts back into the legacy warn-and-proceed
        // behavior, and the prefilter must build a context with an
        // empty change set rather than erroring. We don't assert on
        // the log output (wrkflw_logging::warning goes to a global
        // sink), just that the path does not error and that a
        // workflow gated on paths: will resolve to Skip cleanly.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  push:\n    paths:\n      - 'src/**'\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        let event = "push".to_string();
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: None,
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: false,
        };
        let decision = run_trigger_prefilter(req)
            .await
            .expect("non-strict mode must not error on --event alone");
        match decision {
            PrefilterDecision::Skip { reason } => {
                // Empty change set against `paths: ['src/**']` must
                // surface as a Skip whose reason mentions the paths
                // filter — not a Proceed (which would run the
                // workflow against a phantom empty change set).
                assert!(
                    reason.contains("paths"),
                    "non-strict empty change set must Skip on a paths-gated \
                     workflow, got reason: {}",
                    reason
                );
            }
            PrefilterDecision::Proceed => {
                panic!(
                    "non-strict mode with empty change set must Skip a \
                     paths-gated workflow, got Proceed"
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn strict_filter_rejects_pull_request_without_base_branch() {
        // Regression pin for `apply_base_branch` under strict mode:
        // simulating pull_request or pull_request_target without
        // --base-branch is the same silent-skip shape as --event
        // alone — every `branches:` filter on the event is
        // deterministically rejected because GHA evaluates those
        // against the PR target. Strict mode must refuse to proceed
        // instead of warn-and-continue.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let wf = tmp.path().join("ci.yml");
        std::fs::write(
            &wf,
            "name: ci\n\
             on:\n  pull_request:\n    branches:\n      - main\n\
             jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
        )
        .expect("write workflow");

        // Pass `--changed-files` so `build_event_context` doesn't
        // reject on the "no change set" path — we want the error to
        // come from `apply_base_branch` specifically.
        let event = "pull_request".to_string();
        let changed: Vec<String> = vec!["src/main.rs".to_string()];
        let req = PrefilterRequest {
            workflow_path: &wf,
            event: Some(&event),
            diff: false,
            changed_files: Some(&changed),
            diff_base: None,
            diff_head: None,
            base_branch: None,
            activity_type: None,
            verbose: false,
            strict_filter: true,
        };
        let err = run_trigger_prefilter(req)
            .await
            .expect_err("strict mode must reject pull_request without --base-branch");
        assert!(
            err.contains("--base-branch"),
            "error must point the user at --base-branch, got: {}",
            err
        );
        assert!(
            err.contains("pull_request"),
            "error must name the offending event, got: {}",
            err
        );
    }

    #[test]
    fn validate_event_requires_base_branch_is_noop_for_non_pr_events() {
        // `push`, `workflow_dispatch`, `schedule`, etc. never
        // evaluate `branches:` against a PR target, so
        // `--base-branch` is irrelevant and the helper must
        // return `Ok(())` regardless of flag state. This is the
        // host-agnostic passthrough both `wrkflw run` and
        // `wrkflw watch` depend on.
        for event in ["push", "workflow_dispatch", "schedule", "release"] {
            assert!(
                validate_event_requires_base_branch(event, true).is_ok(),
                "strict mode must not reject non-PR event `{}`",
                event
            );
            assert!(
                validate_event_requires_base_branch(event, false).is_ok(),
                "non-strict mode must not reject non-PR event `{}`",
                event
            );
        }
    }

    #[test]
    fn validate_event_requires_base_branch_rejects_pull_request_target_under_strict() {
        // `pull_request_target` carries the same `branches:`
        // semantics as `pull_request`; the helper must cover
        // both so watch-mode users who pass
        // `--event pull_request_target` get the same diagnostic
        // as the run command.
        let err = validate_event_requires_base_branch("pull_request_target", true)
            .expect_err("strict mode must reject pull_request_target without --base-branch");
        assert!(
            err.contains("pull_request_target"),
            "error must name the pull_request_target event explicitly, got: {}",
            err
        );
        assert!(
            err.contains("--base-branch"),
            "error must point the user at --base-branch, got: {}",
            err
        );
    }

    #[test]
    fn validate_event_requires_base_branch_wording_is_host_neutral() {
        // Regression pin: the helper used to hard-code
        // "simulating" in both the strict error and the
        // non-strict warning. That reads correctly from
        // `wrkflw run` but was semantically wrong for
        // `wrkflw watch`, which is not simulating anything.
        // The wording is now host-neutral; this test enforces
        // it so a future reword can't silently reintroduce a
        // host-specific verb.
        let err = validate_event_requires_base_branch("pull_request", true)
            .expect_err("strict mode must reject pull_request without --base-branch");
        assert!(
            !err.to_lowercase().contains("simulating"),
            "error text must not use the host-specific verb `simulating`, got: {}",
            err
        );
        assert!(
            !err.to_lowercase().contains("watching"),
            "error text must not use the host-specific verb `watching`, got: {}",
            err
        );
        // Non-strict warn path: no `Result` to inspect, so we
        // can only assert the call returns `Ok(())` and trust
        // the body's compile-time string for the wording. The
        // regression that motivated this test was specifically
        // a hard-coded "simulating" literal, so pinning the
        // error branch is enough to catch any reword that
        // forgets to update both.
        assert!(validate_event_requires_base_branch("pull_request", false).is_ok());
    }
}
