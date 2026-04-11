use crate::model::GlobPattern;

/// Check if any changed file matches the path filter configuration.
///
/// GitHub Actions semantics:
/// - If `include` is non-empty: at least one changed file must match at least one pattern.
/// - If `exclude` is non-empty: files matching any exclude pattern are filtered out first.
/// - If both are empty: always matches (no path filter active).
pub fn matches_paths(
    changed_files: &[String],
    include_patterns: &[GlobPattern],
    exclude_patterns: &[GlobPattern],
) -> bool {
    // No path filters at all — everything matches
    if include_patterns.is_empty() && exclude_patterns.is_empty() {
        return true;
    }

    // No changed files — nothing can match
    if changed_files.is_empty() {
        return false;
    }

    // Filter out excluded files first
    let remaining: Vec<&String> = if exclude_patterns.is_empty() {
        changed_files.iter().collect()
    } else {
        changed_files
            .iter()
            .filter(|f| !matches_any_pattern(f, exclude_patterns))
            .collect()
    };

    // If we only have exclude patterns (paths-ignore), check if any files remain
    if include_patterns.is_empty() {
        return !remaining.is_empty();
    }

    // Check if any remaining file matches an include pattern
    remaining
        .iter()
        .any(|f| matches_any_pattern(f, include_patterns))
}

/// Match a file path against a list of GlobPatterns using GitHub Actions semantics:
/// - `*` matches any character except `/`
/// - `**` matches zero or more directories
///
/// **No bare-filename fallback.** An earlier version of this matcher tried
/// the leaf filename as a second pass for patterns without `/`, so `*.rs`
/// would match `src/main.rs`. That diverged from GitHub Actions, where `*`
/// does not cross `/` — `*.rs` matches only top-level `.rs` files; to match
/// `.rs` files anywhere, write `**/*.rs`. Silently matching under different
/// rules than the production runner defeats the point of a trigger filter,
/// so the fallback was removed. The regression is pinned by
/// `star_pattern_does_not_match_nested_file_gha_semantics`.
fn matches_any_pattern(file: &str, patterns: &[GlobPattern]) -> bool {
    let opts = GlobPattern::match_options();
    patterns
        .iter()
        .any(|gp| gp.pattern.matches_with(file, opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(s: &str) -> GlobPattern {
        GlobPattern::new(s).unwrap_or_else(|e| panic!("invalid test glob '{}': {}", s, e))
    }

    #[test]
    fn empty_filters_always_match() {
        assert!(matches_paths(&["src/main.rs".into()], &[], &[]));
    }

    #[test]
    fn no_changed_files_never_match() {
        assert!(!matches_paths(&[], &[gp("src/**")], &[]));
    }

    #[test]
    fn include_pattern_matches() {
        let files = vec!["src/main.rs".into(), "README.md".into()];
        assert!(matches_paths(&files, &[gp("src/**")], &[]));
    }

    #[test]
    fn include_pattern_no_match() {
        let files = vec!["docs/guide.md".into(), "README.md".into()];
        assert!(!matches_paths(&files, &[gp("src/**")], &[]));
    }

    #[test]
    fn exclude_pattern_filters_files() {
        let files = vec!["src/main.rs".into(), "docs/guide.md".into()];
        // paths-ignore: docs/** — src/main.rs remains, so workflow triggers
        assert!(matches_paths(&files, &[], &[gp("docs/**")]));
    }

    #[test]
    fn exclude_pattern_removes_all_files() {
        let files = vec!["docs/guide.md".into(), "docs/api.md".into()];
        // paths-ignore: docs/** — nothing remains
        assert!(!matches_paths(&files, &[], &[gp("docs/**")]));
    }

    #[test]
    fn double_star_matches_nested() {
        let files = vec!["src/deeply/nested/file.rs".into()];
        assert!(matches_paths(&files, &[gp("src/**")], &[]));
    }

    #[test]
    fn star_pattern_does_not_match_nested_file_gha_semantics() {
        // GitHub Actions semantics: `*.rs` only matches top-level files
        // because `*` does not cross `/`. To match `.rs` files anywhere
        // in the tree, write `**/*.rs` (the standard GHA idiom).
        //
        // wrkflw previously had a bare-filename fallback that made
        // `*.rs` match `src/main.rs` — more permissive than GHA, which
        // meant workflows behaved differently locally vs on GitHub.
        // This test pins the corrected behavior so a future refactor
        // cannot silently reintroduce the divergence.
        let nested = vec!["src/main.rs".into()];
        assert!(
            !matches_paths(&nested, &[gp("*.rs")], &[]),
            "'*.rs' must NOT match 'src/main.rs' under GHA semantics — \
             `*` does not cross `/`"
        );

        // Top-level files DO match (no directory traversal needed).
        let top_level = vec!["main.rs".into()];
        assert!(matches_paths(&top_level, &[gp("*.rs")], &[]));

        // `**/*.rs` is the correct GHA idiom for matching at any depth.
        assert!(matches_paths(&nested, &[gp("**/*.rs")], &[]));
    }

    #[test]
    fn exact_file_match() {
        let files = vec!["Cargo.toml".into()];
        assert!(matches_paths(&files, &[gp("Cargo.toml")], &[]));
    }

    #[test]
    fn combined_include_exclude() {
        let files = vec!["src/main.rs".into(), "src/test_helpers.rs".into()];
        // Include src/**, exclude **/test_*
        assert!(matches_paths(&files, &[gp("src/**")], &[gp("**/test_*")]));
        // Only test file — included by src/** but excluded by test_*
        let files2 = vec!["src/test_helpers.rs".into()];
        assert!(!matches_paths(&files2, &[gp("src/**")], &[gp("**/test_*")]));
    }

    #[test]
    fn star_does_not_match_slash() {
        let files = vec!["src/sub/file.rs".into()];
        // * must not cross directory boundaries (GitHub Actions semantics)
        assert!(!matches_paths(&files, &[gp("src/*")], &[]));
        // ** should cross directory boundaries
        assert!(matches_paths(&files, &[gp("src/**/*.rs")], &[]));
        assert!(matches_paths(&files, &[gp("src/**")], &[]));
    }

    #[test]
    fn md_extension_ignore() {
        let files = vec!["README.md".into(), "CHANGELOG.md".into()];
        assert!(!matches_paths(&files, &[], &[gp("*.md")]));
    }
}
