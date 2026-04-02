use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Parsed results from all 4 GitHub Actions environment files after a step runs.
#[derive(Default)]
pub struct StepEnvironmentUpdates {
    /// Key-value pairs from GITHUB_OUTPUT
    pub outputs: HashMap<String, String>,
    /// Key-value pairs from GITHUB_ENV
    pub env_vars: HashMap<String, String>,
    /// Path entries from GITHUB_PATH (one per line)
    pub path_entries: Vec<String>,
    /// Accumulated markdown from GITHUB_STEP_SUMMARY
    pub step_summary: String,
}

/// Parse the GitHub Actions key-value file format used by GITHUB_OUTPUT and GITHUB_ENV.
///
/// Supports two formats:
/// - Simple: `key=value`
/// - Multiline heredoc: `key<<DELIMITER\nline1\nline2\nDELIMITER`
pub fn parse_github_kv_file(content: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // Skip empty lines
        if line.is_empty() {
            i += 1;
            continue;
        }

        // Check for heredoc format: key<<DELIMITER
        if let Some(heredoc_sep_pos) = line.find("<<") {
            let key = &line[..heredoc_sep_pos];
            let delimiter = &line[heredoc_sep_pos + 2..];

            if !key.is_empty() && !delimiter.is_empty() {
                // Collect lines until we find the delimiter
                let mut value_lines = Vec::new();
                i += 1;
                while i < lines.len() {
                    if lines[i] == delimiter {
                        break;
                    }
                    value_lines.push(lines[i]);
                    i += 1;
                }
                result.insert(key.to_string(), value_lines.join("\n"));
                i += 1; // skip the closing delimiter
                continue;
            }
        }

        // Simple key=value format — split on first '=' only
        if let Some(eq_pos) = line.find('=') {
            let key = &line[..eq_pos];
            let value = &line[eq_pos + 1..];
            if !key.is_empty() {
                result.insert(key.to_string(), value.to_string());
            }
        }

        i += 1;
    }

    result
}

/// Parse GITHUB_PATH file — one path entry per non-empty line.
pub fn parse_github_path_file(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

/// Read all 4 environment files using HOST paths from `job_env` and return parsed updates.
///
/// Missing or unreadable files are silently treated as empty — this is expected when
/// steps don't write to them.
pub fn read_step_environment_updates(job_env: &HashMap<String, String>) -> StepEnvironmentUpdates {
    let mut updates = StepEnvironmentUpdates::default();

    if let Some(path) = job_env.get("GITHUB_OUTPUT") {
        if let Ok(content) = fs::read_to_string(Path::new(path)) {
            if !content.is_empty() {
                updates.outputs = parse_github_kv_file(&content);
            }
        }
    }

    if let Some(path) = job_env.get("GITHUB_ENV") {
        if let Ok(content) = fs::read_to_string(Path::new(path)) {
            if !content.is_empty() {
                updates.env_vars = parse_github_kv_file(&content);
            }
        }
    }

    if let Some(path) = job_env.get("GITHUB_PATH") {
        if let Ok(content) = fs::read_to_string(Path::new(path)) {
            if !content.is_empty() {
                updates.path_entries = parse_github_path_file(&content);
            }
        }
    }

    if let Some(path) = job_env.get("GITHUB_STEP_SUMMARY") {
        if let Ok(content) = fs::read_to_string(Path::new(path)) {
            updates.step_summary = content;
        }
    }

    updates
}

/// Truncate the GITHUB_OUTPUT file between steps.
///
/// Step outputs are per-step (not cumulative), so we clear the file before each step.
/// GITHUB_ENV and GITHUB_PATH are cumulative and should NOT be cleared.
pub fn clear_github_output(job_env: &HashMap<String, String>) {
    if let Some(path) = job_env.get("GITHUB_OUTPUT") {
        let _ = fs::write(Path::new(path), "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_simple_kv() {
        let content = "key=value\nother=thing";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("key").unwrap(), "value");
        assert_eq!(result.get("other").unwrap(), "thing");
    }

    #[test]
    fn parse_heredoc() {
        let content = "body<<EOF\nline1\nline2\nEOF";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("body").unwrap(), "line1\nline2");
    }

    #[test]
    fn parse_heredoc_custom_delimiter() {
        let content = "msg<<DELIM_123\nhello world\nDELIM_123";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("msg").unwrap(), "hello world");
    }

    #[test]
    fn parse_mixed_formats() {
        let content = "simple=val\nmulti<<END\nfoo\nbar\nEND\nanother=baz";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("simple").unwrap(), "val");
        assert_eq!(result.get("multi").unwrap(), "foo\nbar");
        assert_eq!(result.get("another").unwrap(), "baz");
    }

    #[test]
    fn parse_empty_input() {
        let result = parse_github_kv_file("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_value_with_equals() {
        let content = "url=https://example.com?a=1&b=2";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("url").unwrap(), "https://example.com?a=1&b=2");
    }

    #[test]
    fn parse_empty_value() {
        let content = "empty=";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("empty").unwrap(), "");
    }

    #[test]
    fn parse_skips_blank_lines() {
        let content = "\nkey=value\n\nother=thing\n";
        let result = parse_github_kv_file(content);
        assert_eq!(result.len(), 2);
        assert_eq!(result.get("key").unwrap(), "value");
    }

    #[test]
    fn parse_path_file() {
        let content = "/usr/local/bin\n/opt/tools\n";
        let result = parse_github_path_file(content);
        assert_eq!(result, vec!["/usr/local/bin", "/opt/tools"]);
    }

    #[test]
    fn parse_path_file_skips_blank_lines() {
        let content = "\n/first\n\n/second\n";
        let result = parse_github_path_file(content);
        assert_eq!(result, vec!["/first", "/second"]);
    }

    #[test]
    fn read_missing_files_returns_empty() {
        let mut env = HashMap::new();
        env.insert(
            "GITHUB_OUTPUT".to_string(),
            "/nonexistent/path/output".to_string(),
        );
        let updates = read_step_environment_updates(&env);
        assert!(updates.outputs.is_empty());
        assert!(updates.env_vars.is_empty());
        assert!(updates.path_entries.is_empty());
        assert!(updates.step_summary.is_empty());
    }

    #[test]
    fn read_and_clear_round_trip() {
        let dir = tempdir().unwrap();
        let output_path = dir.path().join("output");
        fs::write(&output_path, "version=1.2.3\n").unwrap();

        let mut env = HashMap::new();
        env.insert(
            "GITHUB_OUTPUT".to_string(),
            output_path.to_string_lossy().to_string(),
        );

        let updates = read_step_environment_updates(&env);
        assert_eq!(updates.outputs.get("version").unwrap(), "1.2.3");

        clear_github_output(&env);
        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.is_empty());
    }

    #[test]
    fn read_all_four_files() {
        let dir = tempdir().unwrap();
        let github_dir = dir.path().join("github");
        fs::create_dir_all(&github_dir).unwrap();

        fs::write(github_dir.join("output"), "result=ok\n").unwrap();
        fs::write(github_dir.join("env"), "MY_VAR=hello\n").unwrap();
        fs::write(github_dir.join("path"), "/new/path\n").unwrap();
        fs::write(github_dir.join("step_summary"), "## Summary\nAll good").unwrap();

        let mut env = HashMap::new();
        env.insert(
            "GITHUB_OUTPUT".to_string(),
            github_dir.join("output").to_string_lossy().to_string(),
        );
        env.insert(
            "GITHUB_ENV".to_string(),
            github_dir.join("env").to_string_lossy().to_string(),
        );
        env.insert(
            "GITHUB_PATH".to_string(),
            github_dir.join("path").to_string_lossy().to_string(),
        );
        env.insert(
            "GITHUB_STEP_SUMMARY".to_string(),
            github_dir
                .join("step_summary")
                .to_string_lossy()
                .to_string(),
        );

        let updates = read_step_environment_updates(&env);
        assert_eq!(updates.outputs.get("result").unwrap(), "ok");
        assert_eq!(updates.env_vars.get("MY_VAR").unwrap(), "hello");
        assert_eq!(updates.path_entries, vec!["/new/path"]);
        assert_eq!(updates.step_summary, "## Summary\nAll good");
    }
}
