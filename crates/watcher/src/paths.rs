//! Path-form helpers used across the watcher pipeline.
//!
//! Extracted from `watcher.rs` so the platform quirks (macOS
//! `/private/var` prefix, symlinked working trees, Windows path
//! separators) are documented and tested in one place.
//!
//! `canonicalize_allowing_missing` used to live here too, but it was
//! promoted to `wrkflw-trigger-filter` so the process-wide compiled-
//! config LRU could canonicalize its own cache keys. All watcher call
//! sites now reach it via `wrkflw_trigger_filter::canonicalize_allowing_missing`.

use std::path::Path;

/// Normalize a path-like string so any platform separator is replaced
/// with `/`. Used after `strip_prefix` on change events so downstream
/// glob matching (`path_matcher`) sees the forward-slash form GitHub
/// Actions' `paths:` filters are written against.
///
/// On Unix this is a no-op: backslash is a valid filename byte and we
/// must not rewrite it. On Windows, notify delivers `\`-separated
/// paths, which `glob::Pattern` with `require_literal_separator: true`
/// rejects against a `src/**`-style filter — every Windows user would
/// see "0 triggered" without this. The function is gated on
/// `MAIN_SEPARATOR` so the Unix pass-through branch is compile-time free.
pub(crate) fn normalize_separators(s: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        s.to_string()
    } else {
        s.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// Render `wf_path` as a repo-relative path for user-visible TRIGGERED
/// / SKIPPED output. Falls back to the raw path when the workflow is
/// not inside the repo root — an unusual state, but it can happen with
/// a symlink pointing outside the tree, and we prefer an ugly label
/// over a silent drop.
pub(crate) fn display_workflow_path(wf_path: &Path, repo_root: &Path) -> String {
    wf_path
        .strip_prefix(repo_root)
        .unwrap_or(wf_path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two `canonicalize_allowing_missing_*` tests that lived here
    // were moved to `wrkflw-trigger-filter`'s test module alongside
    // the function itself. See `crates/trigger-filter/src/lib.rs`.

    #[test]
    fn normalize_separators_converts_backslashes_on_windows_forms() {
        // Even on non-Windows hosts we can assert the function's
        // contract by forcing a backslash-containing input: the
        // implementation is gated on `MAIN_SEPARATOR`, so on Unix
        // the input comes back unchanged (contract: only the MAIN
        // separator is rewritten, stray `\` in a filename on Unix
        // is legal and must be preserved). The Windows-host branch
        // is pinned by inspection; we can't cross-compile-test it
        // from here without a `#[cfg(windows)]` branch.
        if std::path::MAIN_SEPARATOR == '\\' {
            assert_eq!(normalize_separators("src\\main.rs"), "src/main.rs");
            assert_eq!(
                normalize_separators("crates\\foo\\src\\lib.rs"),
                "crates/foo/src/lib.rs"
            );
        } else {
            // Unix pass-through: backslash is a valid filename byte.
            assert_eq!(normalize_separators("src/main.rs"), "src/main.rs");
            assert_eq!(
                normalize_separators("weird\\filename.txt"),
                "weird\\filename.txt"
            );
        }
    }

    #[test]
    fn display_workflow_path_returns_repo_relative_when_possible() {
        let repo = Path::new("/home/alice/proj");
        let wf = Path::new("/home/alice/proj/.github/workflows/ci.yml");
        assert_eq!(
            display_workflow_path(wf, repo),
            ".github/workflows/ci.yml",
            "workflow inside repo must render relative"
        );

        // A workflow somehow outside the repo root falls back to
        // absolute — we prefer an ugly label to a silent drop.
        let outside = Path::new("/tmp/elsewhere/ci.yml");
        assert_eq!(
            display_workflow_path(outside, repo),
            outside.display().to_string()
        );
    }
}
