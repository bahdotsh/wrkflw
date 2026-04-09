use glob::Pattern;

/// Check if a branch or tag name matches the filter configuration.
///
/// GitHub Actions semantics:
/// - If `include` is non-empty: the ref must match at least one pattern.
/// - If `exclude` is non-empty: the ref must NOT match any exclude pattern.
/// - If both are empty: any ref matches (no ref filter active).
/// - `*` matches any character except `/`.
/// - `**` matches everything including `/`.
pub fn matches_ref(
    ref_name: &str,
    include_patterns: &[String],
    exclude_patterns: &[String],
) -> bool {
    // No ref filters — everything matches
    if include_patterns.is_empty() && exclude_patterns.is_empty() {
        return true;
    }

    let opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true, // * must not match /
        require_literal_leading_dot: false,
    };

    // Check exclusions first
    if !exclude_patterns.is_empty() {
        for pattern_str in exclude_patterns {
            if let Ok(pat) = Pattern::new(pattern_str) {
                if pat.matches_with(ref_name, opts) {
                    return false;
                }
            }
        }
        // If only exclude patterns and none matched, the ref passes
        if include_patterns.is_empty() {
            return true;
        }
    }

    // Check inclusions
    for pattern_str in include_patterns {
        if let Ok(pat) = Pattern::new(pattern_str) {
            if pat.matches_with(ref_name, opts) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_filters_match_anything() {
        assert!(matches_ref("main", &[], &[]));
        assert!(matches_ref("feature/foo", &[], &[]));
    }

    #[test]
    fn exact_branch_match() {
        assert!(matches_ref("main", &["main".into()], &[]));
        assert!(!matches_ref("develop", &["main".into()], &[]));
    }

    #[test]
    fn wildcard_match() {
        assert!(matches_ref("release/v1.0", &["release/*".into()], &[]));
        assert!(matches_ref("release/v2.0", &["release/*".into()], &[]));
        // * must not cross /
        assert!(!matches_ref(
            "release/v1.0/hotfix",
            &["release/*".into()],
            &[]
        ));
    }

    #[test]
    fn double_star_matches_nested() {
        assert!(matches_ref(
            "release/v1.0/hotfix",
            &["release/**".into()],
            &[]
        ));
    }

    #[test]
    fn exclude_pattern() {
        assert!(!matches_ref("main", &[], &["main".into()]));
        assert!(matches_ref("develop", &[], &["main".into()]));
    }

    #[test]
    fn include_and_exclude() {
        // Include release/*, but exclude release/old
        assert!(matches_ref(
            "release/v1.0",
            &["release/*".into()],
            &["release/old".into()]
        ));
        assert!(!matches_ref(
            "release/old",
            &["release/*".into()],
            &["release/old".into()]
        ));
    }

    #[test]
    fn feature_branch_pattern() {
        assert!(matches_ref("feature/login", &["feature/**".into()], &[]));
        assert!(!matches_ref("bugfix/login", &["feature/**".into()], &[]));
    }

    #[test]
    fn tag_version_pattern() {
        assert!(matches_ref("v1.0.0", &["v*".into()], &[]));
        assert!(matches_ref("v2.1.3-rc1", &["v*".into()], &[]));
        assert!(!matches_ref("release-1.0", &["v*".into()], &[]));
    }

    #[test]
    fn exclude_rc_tags() {
        assert!(matches_ref("v1.0.0", &["v*".into()], &["v*-rc*".into()]));
        assert!(!matches_ref(
            "v1.0.0-rc1",
            &["v*".into()],
            &["v*-rc*".into()]
        ));
    }
}
