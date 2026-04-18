// UI utilities
use crate::models::{Workflow, WorkflowStatus};
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use wrkflw_parser::workflow::parse_workflow;
use wrkflw_utils::is_workflow_file;

/// Truncate a display string from the left so it fits within `max_width`
/// terminal columns, preserving the tail (filename) and inserting `…` at
/// the head. Uses Unicode display widths so East Asian wide characters
/// are measured correctly.
pub fn truncate_path(path: &str, max_width: usize) -> String {
    let display_width = UnicodeWidthStr::width(path);
    if display_width <= max_width {
        return path.to_string();
    }
    if max_width <= 1 {
        return "\u{2026}".to_string();
    }
    let target = max_width - 1; // one column for the "…" prefix
    let mut width = 0;
    let mut start_byte = path.len();
    for (idx, ch) in path.char_indices().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > target {
            break;
        }
        width += ch_width;
        start_byte = idx;
    }
    format!("\u{2026}{}", &path[start_byte..])
}

/// Parse a workflow file and return sorted job names, or an empty vec on failure.
pub fn extract_job_names(path: &Path) -> Vec<String> {
    parse_workflow(path)
        .map(|wf| {
            let mut names: Vec<String> = wf.jobs.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// Find and load all workflow files in a directory
pub fn load_workflows(dir_path: &Path) -> Vec<Workflow> {
    let mut workflows = Vec::new();

    // Default path is .github/workflows
    let default_workflows_dir = Path::new(".github").join("workflows");
    let is_default_dir = dir_path == default_workflows_dir || dir_path.ends_with("workflows");

    if let Ok(entries) = std::fs::read_dir(dir_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && (is_workflow_file(&path) || !is_default_dir) {
                // Get just the base name without extension
                let name = path.file_stem().map_or_else(
                    || "[unknown]".to_string(),
                    |fname| fname.to_string_lossy().into_owned(),
                );

                let job_names = extract_job_names(&path);

                workflows.push(Workflow {
                    name,
                    path,
                    selected: false,
                    status: WorkflowStatus::NotStarted,
                    execution_details: None,
                    job_names,
                    trigger_match: None,
                });
            }
        }
    }

    // Check for GitLab CI pipeline file in the root directory if we're in the default GitHub workflows dir
    if is_default_dir {
        // Look for .gitlab-ci.yml in the repository root
        let gitlab_ci_path = PathBuf::from(".gitlab-ci.yml");
        if gitlab_ci_path.exists() && gitlab_ci_path.is_file() {
            let job_names = extract_job_names(&gitlab_ci_path);

            workflows.push(Workflow {
                name: "gitlab-ci".to_string(),
                path: gitlab_ci_path,
                selected: false,
                status: WorkflowStatus::NotStarted,
                execution_details: None,
                job_names,
                trigger_match: None,
            });
        }
    }

    // Sort workflows by name
    workflows.sort_by(|a, b| a.name.cmp(&b.name));
    workflows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_path_no_change_when_fits() {
        assert_eq!(truncate_path("src/main.rs", 20), "src/main.rs");
    }

    #[test]
    fn truncate_path_left_prefix_with_ellipsis() {
        // 10 chars, max 6 → "…" + last 5 chars
        assert_eq!(truncate_path("abcdefghij", 6), "\u{2026}fghij");
    }

    #[test]
    fn truncate_path_very_narrow_returns_ellipsis() {
        assert_eq!(truncate_path("abcdefghij", 1), "\u{2026}");
        assert_eq!(truncate_path("abcdefghij", 0), "\u{2026}");
    }

    #[test]
    fn truncate_path_respects_cjk_width() {
        // Each CJK char is 2 columns wide; "日本語" = 6 cols, max 5 → "…本語" (5 cols).
        assert_eq!(truncate_path("日本語", 5), "\u{2026}本語");
    }
}
