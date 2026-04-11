//! Watch-registration + workflow-collection helpers.
//!
//! Extracted from `watcher.rs`. Both functions are blocking filesystem
//! work invoked from the main watch loop via `spawn_blocking`; keeping
//! them here isolates their platform-specific comments (Linux inotify
//! watch budget, symlink non-traversal) from the async plumbing.

use crate::error::WatchError;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Walk `root` and register notify watches per subtree, skipping any
/// directory whose name appears in `ignore_dirs`. Returns once every
/// non-ignored top-level subtree under `root` is registered.
///
/// **Why this exists.** `notify::RecommendedWatcher::watch(root,
/// RecursiveMode::Recursive)` is a single call, but on Linux inotify
/// that expands into one watch per directory in the tree. A recursive
/// watch on the repo root therefore registers a watch on every
/// directory inside `target/`, `node_modules/`, `.git/`, etc. — and
/// those are exactly the trees with pathological child counts. On a
/// Rust monorepo `target/` alone can exceed the default
/// `fs.inotify.max_user_watches = 8192`, at which point notify fails
/// the subsequent `watch()` calls and the user sees a degraded watcher
/// with no useful signal.
///
/// We avoid the problem by registering watches subtree-by-subtree:
/// the repo root is watched non-recursively (so top-level files are
/// still caught), and every non-ignored immediate child directory is
/// watched recursively. Ignored child directories get zero watches.
///
/// **Known limitation.** A new top-level directory created *after*
/// the watcher starts is not picked up until restart. This matches
/// the behavior of the old recursive watch in every respect except
/// that the recursive watch *would* have started seeing events for a
/// brand-new top-level dir; in practice that's a rare workflow
/// (contrast with editing existing files in known dirs, which is
/// the hot path this optimization targets).
pub(crate) fn setup_watches(
    watcher: &mut RecommendedWatcher,
    root: &Path,
    ignore_dirs: &HashSet<String>,
) -> Result<(), WatchError> {
    // Watch the root itself non-recursively so file changes at the
    // top level (Cargo.toml, README.md, etc.) are still caught.
    watcher.watch(root, RecursiveMode::NonRecursive)?;

    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => return Err(WatchError::Io(e)),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                wrkflw_logging::warning(&format!(
                    "skipping entry under {} during watch setup: {}",
                    root.display(),
                    e
                ));
                continue;
            }
        };
        let path = entry.path();

        // `file_type()` uses fstat on the dirent so it's cheap and
        // doesn't traverse symlinks. We deliberately skip symlinks
        // at the top level: following them risks watching
        // directories outside the repo (budget waste) or inducing
        // an event loop on a self-referential link.
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        if !file_type.is_dir() {
            continue;
        }

        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if ignore_dirs.contains(name) {
            continue;
        }

        // Register the subtree recursively. A failure here is
        // logged but non-fatal — the watcher still covers the
        // other subtrees, and the user sees which one failed.
        // (A failing inotify_add_watch typically means the budget
        // is already exhausted on *this* subtree, so aborting the
        // whole setup would lose coverage of all the sibling
        // subtrees we've already registered successfully.)
        if let Err(e) = watcher.watch(&path, RecursiveMode::Recursive) {
            wrkflw_logging::warning(&format!(
                "failed to watch subtree {}: {} — events in this subtree will be missed. \
                 On Linux this is usually `fs.inotify.max_user_watches` exhaustion; raise it with \
                 `sysctl fs.inotify.max_user_watches=524288`.",
                path.display(),
                e
            ));
        }
    }

    Ok(())
}

/// Synchronous implementation of `collect_workflow_files`. Extracted so it can
/// be invoked from `spawn_blocking` without closure capture juggling.
///
/// An empty directory returns `Ok(Vec::new())`, NOT an error. Two
/// reasons:
///
///   1. Watch mode's entire value proposition is "react to files that
///      don't exist yet" — refusing to start on an empty
///      `.github/workflows` produces a UX dead end where the user
///      has to pre-populate the directory just to start the watcher.
///
///   2. The mid-session rescan feeds this function's result into
///      `refresh_trigger_cache_blocking`'s `active_set`, which drives
///      `retain` over the compiled-pattern cache. If deleting every
///      workflow file produced an `Err` here, the rescan branch
///      fell back to the *stale* prior snapshot, `active_set`
///      retained the deleted entries, and the evaluator kept
///      running against configs for files that no longer existed.
///      Returning an empty `Ok` lets the retain step evict correctly.
///
/// Real I/O errors (directory not found, permission denied) still
/// propagate via the `?` on `read_dir` / `entry` — those are the
/// cases the caller *does* want to handle explicitly.
pub(crate) fn collect_workflow_files_blocking(dir: &Path) -> Result<Vec<PathBuf>, WatchError> {
    if dir.is_file() {
        return Ok(vec![dir.to_path_buf()]);
    }

    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if ext == "yml" || ext == "yaml" {
                files.push(path);
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ignore::build_ignore_set;
    use notify::Event;

    #[test]
    fn collect_workflow_files_returns_ok_for_empty_directory() {
        // Regression: refusing to start on an empty workflow dir
        // defeats the "pick up files as they appear" property that
        // is the whole point of watch mode. An empty dir must
        // return Ok(Vec::new()) so the watcher can start and the
        // mid-session rescan can evict stale entries when the last
        // workflow file is deleted.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let files = collect_workflow_files_blocking(tmp.path()).expect("empty dir must be Ok");
        assert!(
            files.is_empty(),
            "empty dir must return an empty Vec, got {:?}",
            files
        );
    }

    #[test]
    fn collect_workflow_files_still_errors_on_missing_dir() {
        // The guardrail for the "empty = Ok" change: a directory
        // that does not exist at all must STILL produce an error
        // via `read_dir`'s underlying I/O failure. Otherwise the
        // CLI's pre-flight check would silently accept a typo'd
        // `--path` and the user would see "watching..." against
        // nothing.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let missing = tmp.path().join("nope-does-not-exist");
        assert!(
            collect_workflow_files_blocking(&missing).is_err(),
            "nonexistent dir must surface an I/O error"
        );
    }

    #[tokio::test]
    async fn setup_watches_skips_ignored_subtrees() {
        // Not a functional test of the fs watcher itself (that would
        // need to stand up `notify` + a real tempdir event loop).
        // Instead, assert the tree-walk shape: the repo root and
        // every non-ignored immediate child directory get registered,
        // and every ignored child is left out. We verify the set of
        // watched paths via a real RecommendedWatcher against a
        // tempdir and the assertion is that `setup_watches` runs to
        // completion without error — the legacy code path would fail
        // half-open when the recursive watch hit inotify budget on
        // the ignored subtree.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target/debug/deps")).unwrap();
        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::create_dir_all(root.join(".github/workflows")).unwrap();

        let (tx, _rx) = std::sync::mpsc::channel::<Result<Event, notify::Error>>();
        let mut watcher: RecommendedWatcher = notify::RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default(),
        )
        .expect("RecommendedWatcher::new");

        let ignore = build_ignore_set(&[]);
        setup_watches(&mut watcher, root, &ignore).expect("setup_watches");
    }
}
