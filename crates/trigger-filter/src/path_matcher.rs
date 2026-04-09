use glob::Pattern;

/// Check if any changed file matches the path filter configuration.
///
/// GitHub Actions semantics:
/// - If `include` is non-empty: at least one changed file must match at least one pattern.
/// - If `exclude` is non-empty: files matching any exclude pattern are filtered out first.
/// - If both are empty: always matches (no path filter active).
pub fn matches_paths(
    changed_files: &[String],
    include_patterns: &[String],
    exclude_patterns: &[String],
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

/// Check if a file path matches any of the given glob patterns.
fn matches_any_pattern(file: &str, patterns: &[String]) -> bool {
    for pattern_str in patterns {
        if match_github_glob(file, pattern_str) {
            return true;
        }
    }
    false
}

/// Match a file path against a GitHub Actions glob pattern.
///
/// GitHub Actions special rules:
/// - `*` matches any character except `/`
/// - `**` matches zero or more directories
/// - Patterns without `/` match against the filename only
///   (e.g., `*.rs` matches `src/main.rs`)
fn match_github_glob(file: &str, pattern: &str) -> bool {
    // If the pattern contains no path separator, match against the filename only
    // in addition to the full path.
    let opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true, // * must not match /
        require_literal_leading_dot: false,
    };

    if let Ok(pat) = Pattern::new(pattern) {
        if pat.matches_with(file, opts) {
            return true;
        }

        // If pattern has no `/`, also try matching just the filename
        if !pattern.contains('/') {
            if let Some(filename) = file.rsplit('/').next() {
                if pat.matches_with(filename, opts) {
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

    #[test]
    fn empty_filters_always_match() {
        assert!(matches_paths(&["src/main.rs".into()], &[], &[]));
    }

    #[test]
    fn no_changed_files_never_match() {
        assert!(!matches_paths(&[], &["src/**".into()], &[]));
    }

    #[test]
    fn include_pattern_matches() {
        let files = vec!["src/main.rs".into(), "README.md".into()];
        assert!(matches_paths(&files, &["src/**".into()], &[]));
    }

    #[test]
    fn include_pattern_no_match() {
        let files = vec!["docs/guide.md".into(), "README.md".into()];
        assert!(!matches_paths(&files, &["src/**".into()], &[]));
    }

    #[test]
    fn exclude_pattern_filters_files() {
        let files = vec!["src/main.rs".into(), "docs/guide.md".into()];
        // paths-ignore: docs/** — src/main.rs remains, so workflow triggers
        assert!(matches_paths(&files, &[], &["docs/**".into()]));
    }

    #[test]
    fn exclude_pattern_removes_all_files() {
        let files = vec!["docs/guide.md".into(), "docs/api.md".into()];
        // paths-ignore: docs/** — nothing remains
        assert!(!matches_paths(&files, &[], &["docs/**".into()]));
    }

    #[test]
    fn double_star_matches_nested() {
        let files = vec!["src/deeply/nested/file.rs".into()];
        assert!(matches_paths(&files, &["src/**".into()], &[]));
    }

    #[test]
    fn pattern_without_slash_matches_filename() {
        let files = vec!["src/main.rs".into()];
        assert!(matches_paths(&files, &["*.rs".into()], &[]));
    }

    #[test]
    fn exact_file_match() {
        let files = vec!["Cargo.toml".into()];
        assert!(matches_paths(&files, &["Cargo.toml".into()], &[]));
    }

    #[test]
    fn combined_include_exclude() {
        let files = vec!["src/main.rs".into(), "src/test_helpers.rs".into()];
        // Include src/**, exclude **/test_*
        assert!(matches_paths(
            &files,
            &["src/**".into()],
            &["**/test_*".into()]
        ));
        // Only test file — included by src/** but excluded by test_*
        let files2 = vec!["src/test_helpers.rs".into()];
        assert!(!matches_paths(
            &files2,
            &["src/**".into()],
            &["**/test_*".into()]
        ));
    }

    #[test]
    fn star_does_not_match_slash() {
        let files = vec!["src/sub/file.rs".into()];
        // * must not cross directory boundaries (GitHub Actions semantics)
        assert!(!matches_paths(&files, &["src/*".into()], &[]));
        // ** should cross directory boundaries
        assert!(matches_paths(&files, &["src/**/*.rs".into()], &[]));
        assert!(matches_paths(&files, &["src/**".into()], &[]));
    }

    #[test]
    fn md_extension_ignore() {
        let files = vec!["README.md".into(), "CHANGELOG.md".into()];
        assert!(!matches_paths(&files, &[], &["*.md".into()]));
    }

    #[test]
    fn invalid_glob_pattern_is_silently_skipped() {
        // An unclosed bracket is an invalid glob — it should not panic or match
        let files = vec!["src/main.rs".into()];
        assert!(!matches_paths(&files, &["[unclosed".into()], &[]));
    }

    #[test]
    fn invalid_exclude_pattern_is_silently_skipped() {
        // Invalid exclude pattern should not filter anything out
        let files = vec!["src/main.rs".into()];
        assert!(matches_paths(&files, &[], &["[bad".into()]));
    }
}
