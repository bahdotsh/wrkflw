//! Directory-ignore filter for the filesystem watcher.
//!
//! Extracted from `watcher.rs` so the logic (and its regression tests)
//! live next to each other without drowning in the main loop code.
//!
//! The filter is a per-event hot path: `should_ignore_path` runs for
//! every notify event, so it is allocation-free and O(components in
//! parent path). The ignore set is a `HashSet<String>` for O(1)
//! amortized lookup rather than the linear scan an earlier `&[&str]`
//! version used.

use std::collections::HashSet;
use std::path::Path;

/// Directories ignored by the filesystem watcher by default.
/// These are high-churn directories that almost never contain workflow-relevant
/// source files and would otherwise flood the event channel.
pub(crate) const DEFAULT_IGNORE_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".build",
    "build",
    "dist",
    "__pycache__",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".venv",
    "venv",
];

/// Build the combined ignore set for the watcher: every entry from
/// [`DEFAULT_IGNORE_DIRS`] plus any user-supplied `extra_ignore_dirs`.
///
/// Using a `HashSet<String>` (rather than repeatedly scanning a `&[&str]`
/// slice) keeps the per-event lookup in `should_ignore_path` to a single
/// amortized-O(1) check even when the user has added many extras.
pub(crate) fn build_ignore_set(extra_ignore_dirs: &[String]) -> HashSet<String> {
    let mut set: HashSet<String> = DEFAULT_IGNORE_DIRS.iter().map(|s| s.to_string()).collect();
    for dir in extra_ignore_dirs {
        set.insert(dir.clone());
    }
    set
}

/// Returns `true` if a path falls inside any of the ignore directories,
/// where "inside" means: a directory component (NOT the leaf filename) of the
/// path's repo-relative form matches one of the ignore names.
///
/// We deliberately skip the leaf component so a user file literally named
/// `target` (e.g. `scripts/target`) is not silently dropped — only paths that
/// have a `target/` (etc.) parent directory are filtered.
///
/// Paths that are NOT under either repo root form are left untouched (the
/// previous implementation iterated their absolute components, which would
/// incorrectly drop a valid `/home/alice/target-acquisition/...` just
/// because an absolute component happened to equal `target`).
///
/// Both `repo_root_raw` and `repo_root_canonical` are taken because notify's
/// path form is backend-dependent: Linux inotify delivers paths rooted at the
/// raw `.watch()` argument, while macOS FSEvents delivers canonicalized
/// paths. A symlinked working tree on Linux (e.g. `/home/alice/proj`
/// pointing into `/mnt/…/proj`) would otherwise silently defeat the ignore
/// filter — every `target/` event would pass `strip_prefix` against neither
/// form if we only checked canonical. We try the raw form first (the cheap
/// case that matches Linux) and fall back to canonical (macOS + symlinked
/// dereferenced tails).
pub(crate) fn should_ignore_path(
    path: &Path,
    repo_root_raw: &Path,
    repo_root_canonical: &Path,
    ignore_dirs: &HashSet<String>,
) -> bool {
    // Try raw first (Linux inotify common case), then canonical (macOS
    // FSEvents, symlinked working trees where notify dereferenced the
    // link before delivery). We accept the first strip_prefix that
    // succeeds — both forms describe the same repo, so the ignore
    // semantics are identical against either.
    let rel = match path.strip_prefix(repo_root_raw) {
        Ok(r) => r,
        Err(_) => match path.strip_prefix(repo_root_canonical) {
            Ok(r) => r,
            // Path outside both forms of the repo root shouldn't exist in
            // practice (notify scoped to the root), but if it does, leave
            // it alone rather than iterating absolute components that
            // might spuriously match (`/target-foo/...`).
            Err(_) => return false,
        },
    };
    // Compare against every component except the last (the leaf, which is
    // presumed to be a filename). Using `parent()` + component iteration
    // avoids collecting into a `Vec` — this function runs on every notify
    // event, so the hot path is worth keeping allocation-free.
    let parent = match rel.parent() {
        Some(p) => p,
        None => return false,
    };
    for component in parent.components() {
        if let std::path::Component::Normal(os) = component {
            if let Some(s) = os.to_str() {
                if ignore_dirs.contains(s) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> &'static Path {
        Path::new("/repo")
    }

    /// 3-arg adapter that builds a fresh default ignore set so existing
    /// tests stay readable. Tests that exercise user-supplied extras
    /// call `super::should_ignore_path` directly.
    fn should_ignore(path: &Path, raw: &Path, canonical: &Path) -> bool {
        super::should_ignore_path(path, raw, canonical, &build_ignore_set(&[]))
    }

    #[test]
    fn ignores_git_directory() {
        assert!(should_ignore(
            Path::new("/repo/.git/objects/pack/abc"),
            root(),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_path_outside_repo_root() {
        // Regression: previously, paths outside repo_root were iterated as
        // absolute components, so `/home/alice/target-acquisition/file.rs`
        // would match the `target` ignore entry.
        assert!(!should_ignore(
            Path::new("/home/alice/target-acquisition/file.rs"),
            root(),
            root()
        ));
        // Similarly for a directory literally named `target` outside the
        // watched root.
        assert!(!should_ignore(
            Path::new("/target/build.rs"),
            root(),
            root()
        ));
    }

    #[test]
    fn ignores_target_directory() {
        assert!(should_ignore(
            Path::new("/repo/target/debug/deps/foo"),
            root(),
            root()
        ));
    }

    #[test]
    fn ignores_node_modules() {
        assert!(should_ignore(
            Path::new("/repo/node_modules/pkg/index.js"),
            root(),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_src() {
        assert!(!should_ignore(
            Path::new("/repo/src/main.rs"),
            root(),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_workflow_files() {
        assert!(!should_ignore(
            Path::new("/repo/.github/workflows/ci.yml"),
            root(),
            root()
        ));
    }

    #[test]
    fn ignores_pycache() {
        assert!(should_ignore(
            Path::new("/repo/__pycache__/module.cpython-311.pyc"),
            root(),
            root()
        ));
    }

    #[test]
    fn does_not_ignore_file_named_target() {
        // A file literally named `target` should not be filtered out;
        // only directories named `target/` count.
        assert!(!should_ignore(
            Path::new("/repo/scripts/target"),
            root(),
            root()
        ));
    }

    #[test]
    fn ignores_target_via_raw_root_when_canonical_differs() {
        // Regression: symlinked working tree on Linux. `repo_root_raw`
        // is the symlink (`/home/alice/proj`), `repo_root_canonical`
        // is the dereferenced target (`/mnt/work/proj`). Linux inotify
        // delivers events rooted at the watched path (the raw symlink),
        // so the raw form MUST match first — if we only checked the
        // canonical form, every `target/`, `node_modules/`, `.git/`
        // event would bypass the ignore filter and flood the debouncer.
        let raw = Path::new("/home/alice/proj");
        let canonical = Path::new("/mnt/work/proj");
        assert!(should_ignore(
            Path::new("/home/alice/proj/target/debug/foo"),
            raw,
            canonical,
        ));
        // And a non-ignored path under the raw form must still pass.
        assert!(!should_ignore(
            Path::new("/home/alice/proj/src/main.rs"),
            raw,
            canonical,
        ));
    }

    #[test]
    fn ignores_target_via_canonical_root_when_raw_differs() {
        // Regression: macOS FSEvents. `repo_root_raw` is whatever the
        // user passed (`/var/folders/.../proj`), `repo_root_canonical`
        // is FSEvents' canonicalized form (`/private/var/folders/.../proj`).
        // Notify delivers canonicalized paths on macOS, so the raw form
        // WON'T match but the canonical fallback must catch the event.
        let raw = Path::new("/var/folders/xyz/proj");
        let canonical = Path::new("/private/var/folders/xyz/proj");
        assert!(should_ignore(
            Path::new("/private/var/folders/xyz/proj/target/debug/foo"),
            raw,
            canonical,
        ));
        assert!(!should_ignore(
            Path::new("/private/var/folders/xyz/proj/src/main.rs"),
            raw,
            canonical,
        ));
    }

    #[test]
    fn does_not_ignore_file_named_build() {
        assert!(!should_ignore(
            Path::new("/repo/docs/build"),
            root(),
            root()
        ));
    }

    #[test]
    fn ignores_nested_target_subdirectory() {
        assert!(should_ignore(
            Path::new("/repo/crates/foo/target/debug/build/foo"),
            root(),
            root()
        ));
    }

    #[test]
    fn extra_ignore_dirs_are_honored_alongside_defaults() {
        // User adds `.terraform` (not in DEFAULT_IGNORE_DIRS) — a
        // notify event under it must be dropped. The default entries
        // (`target`) must still fire.
        let extra = vec![".terraform".to_string(), "coverage".to_string()];
        let set = build_ignore_set(&extra);
        assert!(super::should_ignore_path(
            Path::new("/repo/.terraform/modules/foo.tf"),
            root(),
            root(),
            &set,
        ));
        assert!(super::should_ignore_path(
            Path::new("/repo/coverage/lcov.info"),
            root(),
            root(),
            &set,
        ));
        // Defaults must still work.
        assert!(super::should_ignore_path(
            Path::new("/repo/target/debug/foo"),
            root(),
            root(),
            &set,
        ));
        // A sibling with a similar name must not be silenced.
        assert!(!super::should_ignore_path(
            Path::new("/repo/.terraform-state/foo"),
            root(),
            root(),
            &set,
        ));
    }

    #[test]
    fn build_ignore_set_includes_every_default() {
        // Guards against a typo-in-merge refactor that forgot to fold
        // `DEFAULT_IGNORE_DIRS` into the HashSet. Every default entry
        // must be present after construction.
        let set = build_ignore_set(&[]);
        for d in DEFAULT_IGNORE_DIRS {
            assert!(
                set.contains(*d),
                "default ignore dir '{}' missing from combined set",
                d
            );
        }
    }
}
