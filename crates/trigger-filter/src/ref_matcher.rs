use crate::model::GlobPattern;

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
    include_patterns: &[GlobPattern],
    exclude_patterns: &[GlobPattern],
) -> bool {
    // No ref filters — everything matches
    if include_patterns.is_empty() && exclude_patterns.is_empty() {
        return true;
    }

    let opts = GlobPattern::match_options();

    // Check exclusions first
    if !exclude_patterns.is_empty() {
        for gp in exclude_patterns {
            if gp.pattern.matches_with(ref_name, opts) {
                return false;
            }
        }
        // If only exclude patterns and none matched, the ref passes
        if include_patterns.is_empty() {
            return true;
        }
    }

    // Check inclusions
    for gp in include_patterns {
        if gp.pattern.matches_with(ref_name, opts) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gp(s: &str) -> GlobPattern {
        GlobPattern::new(s).unwrap_or_else(|e| panic!("invalid test glob '{}': {}", s, e))
    }

    #[test]
    fn empty_filters_match_anything() {
        assert!(matches_ref("main", &[], &[]));
        assert!(matches_ref("feature/foo", &[], &[]));
    }

    #[test]
    fn exact_branch_match() {
        assert!(matches_ref("main", &[gp("main")], &[]));
        assert!(!matches_ref("develop", &[gp("main")], &[]));
    }

    #[test]
    fn wildcard_match() {
        assert!(matches_ref("release/v1.0", &[gp("release/*")], &[]));
        assert!(matches_ref("release/v2.0", &[gp("release/*")], &[]));
        // * must not cross /
        assert!(!matches_ref("release/v1.0/hotfix", &[gp("release/*")], &[]));
    }

    #[test]
    fn double_star_matches_nested() {
        assert!(matches_ref("release/v1.0/hotfix", &[gp("release/**")], &[]));
    }

    #[test]
    fn exclude_pattern() {
        assert!(!matches_ref("main", &[], &[gp("main")]));
        assert!(matches_ref("develop", &[], &[gp("main")]));
    }

    #[test]
    fn include_and_exclude() {
        // Include release/*, but exclude release/old
        assert!(matches_ref(
            "release/v1.0",
            &[gp("release/*")],
            &[gp("release/old")]
        ));
        assert!(!matches_ref(
            "release/old",
            &[gp("release/*")],
            &[gp("release/old")]
        ));
    }

    #[test]
    fn feature_branch_pattern() {
        assert!(matches_ref("feature/login", &[gp("feature/**")], &[]));
        assert!(!matches_ref("bugfix/login", &[gp("feature/**")], &[]));
    }

    #[test]
    fn tag_version_pattern() {
        assert!(matches_ref("v1.0.0", &[gp("v*")], &[]));
        assert!(matches_ref("v2.1.3-rc1", &[gp("v*")], &[]));
        assert!(!matches_ref("release-1.0", &[gp("v*")], &[]));
    }

    #[test]
    fn exclude_rc_tags() {
        assert!(matches_ref("v1.0.0", &[gp("v*")], &[gp("v*-rc*")]));
        assert!(!matches_ref("v1.0.0-rc1", &[gp("v*")], &[gp("v*-rc*")]));
    }
}
