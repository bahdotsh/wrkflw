//! Path-form helpers used across the watcher pipeline.
//!
//! Extracted from `watcher.rs` so the platform quirks (macOS
//! `/private/var` prefix, symlinked working trees, Windows path
//! separators) are documented and tested in one place.

use std::path::{Path, PathBuf};

/// Canonicalize `path`, tolerating the case where the target was deleted.
/// Walks back to the nearest canonicalizable ancestor, then re-appends the
/// missing components. This keeps deleted files root-relative on platforms
/// where the raw path would fail `strip_prefix` (macOS `/private/var` vs
/// `/var`, symlinked working trees).
pub(crate) fn canonicalize_allowing_missing(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    // Walk up until we find an ancestor we can canonicalize; collect the
    // missing tail so we can re-join it.
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cursor: &Path = path;
    while let Some(parent) = cursor.parent() {
        if let Some(leaf) = cursor.file_name() {
            tail.push(leaf);
        }
        if let Ok(canonical_parent) = std::fs::canonicalize(parent) {
            let mut result = canonical_parent;
            for seg in tail.into_iter().rev() {
                result.push(seg);
            }
            return result;
        }
        cursor = parent;
    }
    path.to_path_buf()
}

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

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_leaf() {
        // The leaf does not exist, but its parent is a real canonicalizable
        // directory — the fallback must walk up and re-join the missing leaf.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let deleted = root.join("missing.txt");
        assert!(!deleted.exists());

        let canonical = canonicalize_allowing_missing(&deleted);
        assert!(
            canonical.ends_with("missing.txt"),
            "canonical should retain the leaf, got {}",
            canonical.display()
        );
        let expected_parent = std::fs::canonicalize(root).unwrap();
        assert_eq!(canonical.parent(), Some(expected_parent.as_path()));
    }

    #[test]
    fn canonicalize_allowing_missing_handles_deleted_subdir_leaf() {
        // Parent directory also missing, grandparent exists — must walk up
        // one more level and re-join both segments.
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path();
        let deeper = root.join("gone").join("missing.txt");

        let canonical = canonicalize_allowing_missing(&deeper);
        assert!(canonical.ends_with("gone/missing.txt"));
        let expected_root = std::fs::canonicalize(root).unwrap();
        assert_eq!(
            canonical.strip_prefix(&expected_root).ok(),
            Some(Path::new("gone/missing.txt"))
        );
    }

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
