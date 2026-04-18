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

/// Check whether `s` looks like a valid GHA environment file key: `[a-zA-Z_][a-zA-Z0-9_]*`.
///
/// This validates keys used in GITHUB_OUTPUT and GITHUB_ENV files (e.g. `MY_VAR=value`),
/// NOT step IDs — step IDs additionally allow hyphens (see `STEPS_OUTPUT_PATTERN`).
fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        // The key must be a valid identifier (no '=' allowed) to avoid ambiguity
        // with simple values that contain '<<'.
        if let Some(heredoc_sep_pos) = line.find("<<") {
            let key = &line[..heredoc_sep_pos];
            let delimiter = &line[heredoc_sep_pos + 2..];

            if !key.is_empty() && !delimiter.is_empty() && is_valid_identifier(key) {
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

/// Apply environment updates from a completed step to the job state.
///
/// - Stores step outputs keyed by step ID (for `${{ steps.<id>.outputs.<key> }}`)
/// - Merges GITHUB_ENV entries into `job_env`
/// - Prepends GITHUB_PATH entries to the PATH in `job_env`
/// - Clears per-step files so the next step starts fresh
pub fn apply_step_environment_updates(
    job_env: &mut HashMap<String, String>,
    job_user_env: &mut HashMap<String, String>,
    step_outputs_map: &mut HashMap<String, HashMap<String, String>>,
    step_id: Option<&str>,
) {
    let updates = read_step_environment_updates(job_env);

    // Store step outputs keyed by step ID for ${{ steps.<id>.outputs.<key> }}
    if let Some(id) = step_id {
        step_outputs_map.insert(id.to_string(), updates.outputs);
    }

    // Merge GITHUB_ENV entries into job_env for subsequent steps.
    // These are user-declared by definition (the step wrote them via
    // `echo KEY=VAL >> $GITHUB_ENV`), so mirror into job_user_env too.
    // GITHUB_PATH updates below modify PATH in job_env only — PATH is not
    // a user-declared env var and must not leak into toJSON(env).
    for (k, v) in updates.env_vars {
        job_user_env.insert(k.clone(), v.clone());
        job_env.insert(k, v);
    }

    // Prepend GITHUB_PATH entries to PATH for subsequent steps
    if !updates.path_entries.is_empty() {
        let current_path = job_env
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        let new_entries = updates.path_entries.join(":");
        let new_path = if current_path.is_empty() {
            new_entries
        } else {
            format!("{}:{}", new_entries, current_path)
        };
        job_env.insert("PATH".to_string(), new_path);
    }

    // Clear files so the next step doesn't re-process these entries
    clear_step_files(job_env);
}

/// Truncate environment files between steps.
///
/// GITHUB_OUTPUT is per-step (not cumulative).
/// GITHUB_ENV and GITHUB_PATH are cumulative *on disk* in real GHA, but we read back
/// and merge their contents into `job_env` after each step. To avoid re-processing
/// the same entries on the next step, we truncate them here as well.
/// GITHUB_STEP_SUMMARY is intentionally not cleared — in real GHA, step summaries are
/// cumulative (each step appends to the same file).
pub fn clear_step_files(job_env: &HashMap<String, String>) {
    for key in &["GITHUB_OUTPUT", "GITHUB_ENV", "GITHUB_PATH"] {
        if let Some(path) = job_env.get(*key) {
            let _ = fs::write(Path::new(path), "");
        }
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
        let env_path = dir.path().join("env");
        let path_path = dir.path().join("path");
        fs::write(&output_path, "version=1.2.3\n").unwrap();
        fs::write(&env_path, "MY_VAR=hello\n").unwrap();
        fs::write(&path_path, "/new/bin\n").unwrap();

        let mut env = HashMap::new();
        env.insert(
            "GITHUB_OUTPUT".to_string(),
            output_path.to_string_lossy().to_string(),
        );
        env.insert(
            "GITHUB_ENV".to_string(),
            env_path.to_string_lossy().to_string(),
        );
        env.insert(
            "GITHUB_PATH".to_string(),
            path_path.to_string_lossy().to_string(),
        );

        let updates = read_step_environment_updates(&env);
        assert_eq!(updates.outputs.get("version").unwrap(), "1.2.3");
        assert_eq!(updates.env_vars.get("MY_VAR").unwrap(), "hello");
        assert_eq!(updates.path_entries, vec!["/new/bin"]);

        clear_step_files(&env);
        assert!(fs::read_to_string(&output_path).unwrap().is_empty());
        assert!(fs::read_to_string(&env_path).unwrap().is_empty());
        assert!(fs::read_to_string(&path_path).unwrap().is_empty());
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

    #[test]
    fn parse_value_containing_heredoc_marker() {
        // A value like `url=https://example.com/path<<EOF` should be parsed as simple
        // key=value, NOT as a heredoc, because the text before `<<` contains `=` and
        // is therefore not a valid identifier.
        let content = "url=https://example.com/path<<EOF";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("url").unwrap(), "https://example.com/path<<EOF");
    }

    #[test]
    fn parse_unterminated_heredoc() {
        // Unterminated heredoc should consume to EOF and produce the collected lines.
        let content = "body<<EOF\nline1\nline2";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("body").unwrap(), "line1\nline2");
    }

    #[test]
    fn parse_heredoc_in_output_format() {
        // GITHUB_OUTPUT can use heredoc format for multiline values.
        let content = "json<<EOF\n{\"key\": \"value\"}\nEOF\nversion=1.0";
        let result = parse_github_kv_file(content);
        assert_eq!(result.get("json").unwrap(), "{\"key\": \"value\"}");
        assert_eq!(result.get("version").unwrap(), "1.0");
    }

    #[test]
    fn apply_updates_merges_env_and_path() {
        let dir = tempdir().unwrap();
        let github_dir = dir.path().join("github");
        fs::create_dir_all(&github_dir).unwrap();

        fs::write(github_dir.join("output"), "artifact=build.tar\n").unwrap();
        fs::write(github_dir.join("env"), "CC=gcc\n").unwrap();
        fs::write(github_dir.join("path"), "/opt/gcc/bin\n").unwrap();

        let mut job_env = HashMap::new();
        job_env.insert(
            "GITHUB_OUTPUT".to_string(),
            github_dir.join("output").to_string_lossy().to_string(),
        );
        job_env.insert(
            "GITHUB_ENV".to_string(),
            github_dir.join("env").to_string_lossy().to_string(),
        );
        job_env.insert(
            "GITHUB_PATH".to_string(),
            github_dir.join("path").to_string_lossy().to_string(),
        );
        job_env.insert("PATH".to_string(), "/usr/bin".to_string());

        let mut step_outputs_map = HashMap::new();
        let mut job_user_env = HashMap::new();

        apply_step_environment_updates(
            &mut job_env,
            &mut job_user_env,
            &mut step_outputs_map,
            Some("build"),
        );

        // Step outputs stored under step ID
        assert_eq!(
            step_outputs_map
                .get("build")
                .unwrap()
                .get("artifact")
                .unwrap(),
            "build.tar"
        );
        // Env merged
        assert_eq!(job_env.get("CC").unwrap(), "gcc");
        // $GITHUB_ENV writes must mirror into user_env
        assert_eq!(job_user_env.get("CC").unwrap(), "gcc");
        // Path prepended
        assert_eq!(job_env.get("PATH").unwrap(), "/opt/gcc/bin:/usr/bin");
        // PATH updates from $GITHUB_PATH must NOT appear in user_env
        assert!(
            !job_user_env.contains_key("PATH"),
            "PATH should not leak into user_env"
        );
        // Files cleared for next step
        assert!(fs::read_to_string(github_dir.join("output"))
            .unwrap()
            .is_empty());
        assert!(fs::read_to_string(github_dir.join("env"))
            .unwrap()
            .is_empty());
        assert!(fs::read_to_string(github_dir.join("path"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn apply_updates_no_duplicate_path_entries() {
        let dir = tempdir().unwrap();
        let github_dir = dir.path().join("github");
        fs::create_dir_all(&github_dir).unwrap();

        let output_path = github_dir.join("output");
        let env_path = github_dir.join("env");
        let path_path = github_dir.join("path");

        let mut job_env = HashMap::new();
        job_env.insert(
            "GITHUB_OUTPUT".to_string(),
            output_path.to_string_lossy().to_string(),
        );
        job_env.insert(
            "GITHUB_ENV".to_string(),
            env_path.to_string_lossy().to_string(),
        );
        job_env.insert(
            "GITHUB_PATH".to_string(),
            path_path.to_string_lossy().to_string(),
        );
        job_env.insert("PATH".to_string(), "/usr/bin".to_string());

        let mut step_outputs_map = HashMap::new();
        let mut job_user_env = HashMap::new();

        // Step 1 writes /opt/tool to GITHUB_PATH
        fs::write(&output_path, "").unwrap();
        fs::write(&env_path, "").unwrap();
        fs::write(&path_path, "/opt/tool\n").unwrap();
        apply_step_environment_updates(
            &mut job_env,
            &mut job_user_env,
            &mut step_outputs_map,
            None,
        );
        assert_eq!(job_env.get("PATH").unwrap(), "/opt/tool:/usr/bin");

        // Step 2 writes /opt/other to GITHUB_PATH
        fs::write(&output_path, "").unwrap();
        fs::write(&env_path, "").unwrap();
        fs::write(&path_path, "/opt/other\n").unwrap();
        apply_step_environment_updates(
            &mut job_env,
            &mut job_user_env,
            &mut step_outputs_map,
            None,
        );

        // /opt/tool should appear exactly once (not duplicated)
        let path = job_env.get("PATH").unwrap();
        assert_eq!(path, "/opt/other:/opt/tool:/usr/bin");
    }
}
