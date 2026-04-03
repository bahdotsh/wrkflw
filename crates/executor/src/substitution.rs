use lazy_static::lazy_static;
use regex::Regex;
use serde_yaml::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

lazy_static! {
    static ref MATRIX_PATTERN: Regex =
        Regex::new(r"\$\{\{\s*matrix\.([a-zA-Z0-9_]+)\s*\}\}").unwrap();
    static ref HASH_FILES_PATTERN: Regex =
        Regex::new(r"\$\{\{\s*hashFiles\(([^)]+)\)\s*\}\}").unwrap();
    /// Matches any `${{ ... }}` expression. Handles single `}` inside format
    /// placeholders like `{0}` by requiring the closing `}}` pair.
    static ref EXPRESSION_PATTERN: Regex =
        Regex::new(r"\$\{\{(?:[^}]|\}[^}])*\}\}").unwrap();
}

/// Preprocesses a command string to replace GitHub-style matrix variable references
/// with their values from the environment
pub fn preprocess_command(command: &str, matrix_values: &HashMap<String, Value>) -> String {
    // Replace matrix references like ${{ matrix.os }} with their values
    let result = MATRIX_PATTERN.replace_all(command, |caps: &regex::Captures| {
        let var_name = &caps[1];

        // Get the value from matrix context
        if let Some(value) = matrix_values.get(var_name) {
            // Convert value to string
            match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => format!("\\${{{{ matrix.{} }}}}", var_name), // Escape $ for shell
            }
        } else {
            // Keep original if not found but escape $ to prevent shell errors
            format!("\\${{{{ matrix.{} }}}}", var_name)
        }
    });

    result.into_owned()
}

/// Apply variable substitution to step run commands
pub fn process_step_run(run: &str, matrix_combination: &Option<HashMap<String, Value>>) -> String {
    if let Some(matrix) = matrix_combination {
        preprocess_command(run, matrix)
    } else {
        // Escape $ in GitHub expression syntax to prevent shell interpretation
        MATRIX_PATTERN
            .replace_all(run, |caps: &regex::Captures| {
                let var_name = &caps[1];
                format!("\\${{{{ matrix.{} }}}}", var_name)
            })
            .to_string()
    }
}

/// Replace `${{ hashFiles(...) }}` expressions with the SHA-256 hash of matched files.
///
/// Accepts one or more comma-separated, quoted glob patterns. Files are matched
/// relative to `workspace`, sorted lexicographically, and hashed in order to
/// produce a deterministic digest — matching GitHub Actions behavior.
///
/// Returns `Err` if any matched file cannot be read.
pub fn preprocess_hash_files(text: &str, workspace: &Path) -> Result<String, String> {
    let mut error: Option<String> = None;
    let result = HASH_FILES_PATTERN
        .replace_all(text, |caps: &regex::Captures| {
            if error.is_some() {
                return String::new();
            }
            let args_raw = &caps[1];
            match compute_hash_files(args_raw, workspace) {
                Ok(hash) => hash,
                Err(e) => {
                    error = Some(e);
                    String::new()
                }
            }
        })
        .into_owned();
    match error {
        Some(e) => Err(e),
        None => Ok(result),
    }
}

/// Compute a SHA-256 hash of the contents of all files matching the given glob patterns.
///
/// `args_raw` is the raw argument string inside `hashFiles(...)`, e.g.
/// `'**/package-lock.json', '**/yarn.lock'`.
///
/// Returns `Ok(hash)` on success or `Err(message)` if any matched file cannot be read.
fn compute_hash_files(args_raw: &str, workspace: &Path) -> Result<String, String> {
    // Parse comma-separated, quoted patterns
    let patterns: Vec<&str> = args_raw
        .split(',')
        .map(|s| s.trim().trim_matches('\'').trim_matches('"'))
        .filter(|s| !s.is_empty())
        .collect();

    if patterns.is_empty() {
        return Ok(String::new());
    }

    // Reject patterns containing path traversal components
    for pattern in &patterns {
        if pattern.split('/').any(|seg| seg == "..") {
            return Err(format!(
                "hashFiles: pattern '{}' contains '..' path traversal",
                pattern
            ));
        }
    }

    // Collect all matching files
    let mut matched_files = Vec::new();
    for pattern in &patterns {
        let full_pattern = workspace.join(pattern).to_string_lossy().to_string();
        if let Ok(entries) = glob::glob(&full_pattern) {
            for entry in entries.flatten() {
                if entry.is_file() {
                    matched_files.push(entry);
                }
            }
        }
    }

    if matched_files.is_empty() {
        // GHA returns the SHA-256 of empty input when no files match
        return Ok(format!("{:x}", Sha256::new().finalize()));
    }

    // Sort for deterministic output (GHA sorts lexicographically)
    matched_files.sort();
    matched_files.dedup();

    // Hash all file contents (stream to avoid loading large files into memory)
    let mut hasher = Sha256::new();
    for path in &matched_files {
        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("hashFiles: could not read '{}': {}", path.display(), e))?;
        std::io::copy(&mut file, &mut hasher)
            .map_err(|e| format!("hashFiles: could not read '{}': {}", path.display(), e))?;
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Apply all expression substitutions to a text string.
///
/// `hashFiles()` is resolved first (needs filesystem access), then all remaining
/// `${{ ... }}` expressions are evaluated through the expression evaluator which
/// handles env, inputs, github, runner, matrix, and steps contexts as well as
/// operators and built-in functions.
///
/// Expressions that fail evaluation are replaced with empty string as a safety
/// fallback, matching GitHub Actions behavior.
///
/// Returns `Err` if a `hashFiles()` expression fails (e.g. unreadable file).
pub fn preprocess_expressions(
    text: &str,
    workspace: &Path,
    matrix_combination: &Option<HashMap<String, Value>>,
    step_outputs: &HashMap<String, HashMap<String, String>>,
    env_context: &HashMap<String, String>,
) -> Result<String, String> {
    use crate::expression::{evaluate, ExpressionContext};

    // Resolve hashFiles first (needs filesystem access not available in the
    // expression evaluator)
    let result = preprocess_hash_files(text, workspace)?;

    let ctx = ExpressionContext {
        env_context,
        step_outputs,
        matrix_combination,
    };

    // Evaluate all remaining ${{ ... }} expressions through the expression evaluator
    let result = EXPRESSION_PATTERN
        .replace_all(&result, |caps: &regex::Captures| {
            let full_match = &caps[0];
            // Extract the inner expression (strip "${{" and "}}")
            let inner = &full_match[3..full_match.len() - 2];
            match evaluate(inner, &ctx) {
                Ok(val) => val.to_output_string(),
                Err(e) => {
                    wrkflw_logging::debug(&format!(
                        "Expression evaluation failed for '{}': {} — substituting empty string",
                        inner.trim(),
                        e
                    ));
                    String::new()
                }
            }
        })
        .into_owned();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_preprocess_simple_matrix_vars() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));
        matrix.insert(
            "node".to_string(),
            Value::Number(serde_yaml::Number::from(14)),
        );

        let cmd = "echo \"Running on ${{ matrix.os }} with Node ${{ matrix.node }}\"";
        let processed = preprocess_command(cmd, &matrix);

        assert_eq!(processed, "echo \"Running on ubuntu-latest with Node 14\"");
    }

    #[test]
    fn test_preprocess_with_missing_vars() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));

        let cmd = "echo \"Running on ${{ matrix.os }} with Node ${{ matrix.node }}\"";
        let processed = preprocess_command(cmd, &matrix);

        // Missing vars should be escaped
        assert_eq!(
            processed,
            "echo \"Running on ubuntu-latest with Node \\${{ matrix.node }}\""
        );
    }

    #[test]
    fn test_preprocess_preserves_other_text() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu-latest".to_string()));

        let cmd = "echo \"Starting job\" && echo \"OS: ${{ matrix.os }}\" && echo \"Done!\"";
        let processed = preprocess_command(cmd, &matrix);

        assert_eq!(
            processed,
            "echo \"Starting job\" && echo \"OS: ubuntu-latest\" && echo \"Done!\""
        );
    }

    #[test]
    fn test_process_without_matrix() {
        let cmd = "echo \"Value: ${{ matrix.value }}\"";
        let processed = process_step_run(cmd, &None);

        assert_eq!(processed, "echo \"Value: \\${{ matrix.value }}\"");
    }

    #[test]
    fn hash_files_single_pattern() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("package-lock.json"), "lock-content").unwrap();
        fs::write(dir.path().join("other.txt"), "other").unwrap();

        let text = "${{ hashFiles('package-lock.json') }}";
        let result = preprocess_hash_files(text, dir.path()).unwrap();

        assert!(!result.is_empty());
        assert!(!result.contains("hashFiles"));
        // Hash should be 64 hex chars (SHA-256)
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_files_multiple_patterns() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.lock"), "aaa").unwrap();
        fs::write(dir.path().join("b.json"), "bbb").unwrap();

        let text = "${{ hashFiles('*.lock', '*.json') }}";
        let result = preprocess_hash_files(text, dir.path()).unwrap();

        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_files_no_matches_returns_hash_of_empty() {
        let dir = tempdir().unwrap();

        let text = "${{ hashFiles('nonexistent-*.xyz') }}";
        let result = preprocess_hash_files(text, dir.path()).unwrap();

        // GHA returns SHA-256 of empty input when no files match
        let expected = format!("{:x}", Sha256::new().finalize());
        assert_eq!(result, expected);
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_files_deterministic() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::write(dir.path().join("b.txt"), "world").unwrap();

        let text = "${{ hashFiles('*.txt') }}";
        let r1 = preprocess_hash_files(text, dir.path()).unwrap();
        let r2 = preprocess_hash_files(text, dir.path()).unwrap();

        assert_eq!(r1, r2);
    }

    #[test]
    fn hash_files_glob_recursive() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("deep.lock"), "deep-content").unwrap();

        let text = "${{ hashFiles('**/deep.lock') }}";
        let result = preprocess_hash_files(text, dir.path()).unwrap();

        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_files_inline_with_other_text() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.lock"), "lockfile").unwrap();

        let text = "cache-key-${{ hashFiles('Cargo.lock') }}-suffix";
        let result = preprocess_hash_files(text, dir.path()).unwrap();

        assert!(result.starts_with("cache-key-"));
        assert!(result.ends_with("-suffix"));
        assert!(!result.contains("hashFiles"));
    }

    #[test]
    fn preprocess_expressions_combines_hash_and_matrix() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.lock"), "lockfile").unwrap();

        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu".to_string()));

        let text = "${{ matrix.os }}-${{ hashFiles('Cargo.lock') }}";
        let result = preprocess_expressions(
            text,
            dir.path(),
            &Some(matrix),
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert!(result.starts_with("ubuntu-"));
        assert!(!result.contains("hashFiles"));
        assert!(!result.contains("matrix"));
    }

    #[test]
    fn hash_files_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("legit.txt"), "content").unwrap();

        let text = "${{ hashFiles('../../etc/passwd') }}";
        let result = preprocess_hash_files(text, dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));
    }

    #[test]
    fn hash_files_rejects_mid_path_traversal() {
        let dir = tempdir().unwrap();

        let result = compute_hash_files("'subdir/../../etc/passwd'", dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path traversal"));
    }

    // -- step outputs via expression evaluator --

    #[test]
    fn step_output_substitution() {
        let dir = tempdir().unwrap();
        let mut step_outputs = HashMap::new();
        let mut build_outputs = HashMap::new();
        build_outputs.insert("version".to_string(), "1.2.3".to_string());
        step_outputs.insert("build".to_string(), build_outputs);

        let text = "Version is ${{ steps.build.outputs.version }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &step_outputs, &HashMap::new())
                .unwrap();
        assert_eq!(result, "Version is 1.2.3");
    }

    #[test]
    fn step_output_missing_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "Value: ${{ steps.unknown.outputs.key }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "Value: ");
    }

    #[test]
    fn step_output_missing_key_returns_empty() {
        let dir = tempdir().unwrap();
        let mut step_outputs = HashMap::new();
        step_outputs.insert("build".to_string(), HashMap::new());

        let text = "${{ steps.build.outputs.missing }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &step_outputs, &HashMap::new())
                .unwrap();
        assert_eq!(result, "");
    }

    // -- env context via expression evaluator --

    #[test]
    fn env_context_substitution() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello".to_string());

        let text = "Value: ${{ env.MY_VAR }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "Value: hello");
    }

    #[test]
    fn env_context_missing_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "${{ env.MISSING }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "");
    }

    // -- combined contexts --

    #[test]
    fn combined_substitutions() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("lock"), "content").unwrap();

        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu".to_string()));

        let mut step_outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("tag".to_string(), "v1".to_string());
        step_outputs.insert("build".to_string(), build_out);

        let mut env = HashMap::new();
        env.insert("CI".to_string(), "true".to_string());

        let text = "${{ matrix.os }}-${{ steps.build.outputs.tag }}-${{ env.CI }}";
        let result =
            preprocess_expressions(text, dir.path(), &Some(matrix), &step_outputs, &env).unwrap();
        assert_eq!(result, "ubuntu-v1-true");
    }

    // -- inputs context via expression evaluator --

    #[test]
    fn inputs_context_substitution() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("INPUT_TOOLCHAIN".to_string(), "stable".to_string());
        env.insert("INPUT_COMPONENTS".to_string(), "rustfmt".to_string());

        let text =
            "rustup toolchain install ${{ inputs.toolchain }} --component ${{ inputs.components }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(
            result,
            "rustup toolchain install stable --component rustfmt"
        );
    }

    #[test]
    fn inputs_context_hyphenated_name() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("INPUT_NODE_VERSION".to_string(), "18".to_string());

        let text = "${{ inputs.node-version }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "18");
    }

    #[test]
    fn inputs_context_missing_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "${{ inputs.missing }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "");
    }

    // -- github context via expression evaluator --

    #[test]
    fn github_context_substitution() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("GITHUB_REPOSITORY".to_string(), "owner/repo".to_string());
        env.insert("GITHUB_REF_NAME".to_string(), "main".to_string());

        let text = "${{ github.repository }}/${{ github.ref_name }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "owner/repo/main");
    }

    #[test]
    fn github_context_missing_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "${{ github.token }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "");
    }

    // -- runner context via expression evaluator --

    #[test]
    fn runner_context_substitution() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());
        env.insert("RUNNER_TEMP".to_string(), "/tmp/runner".to_string());

        let text = "${{ runner.os }} ${{ runner.temp }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "Linux /tmp/runner");
    }

    #[test]
    fn runner_context_missing_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "${{ runner.arch }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "");
    }

    // -- all contexts via preprocess_expressions --

    #[test]
    fn preprocess_expressions_includes_inputs_and_github() {
        let dir = tempdir().unwrap();

        let mut env = HashMap::new();
        env.insert("INPUT_TOOLCHAIN".to_string(), "nightly".to_string());
        env.insert("GITHUB_REPOSITORY".to_string(), "foo/bar".to_string());
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());

        let text = "${{ inputs.toolchain }}-${{ github.repository }}-${{ runner.os }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "nightly-foo/bar-Linux");
    }

    #[test]
    fn preprocess_expressions_unknown_context_returns_empty() {
        let dir = tempdir().unwrap();

        let text = "echo ${{ unknown_context.value }}";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &HashMap::new())
                .unwrap();
        assert_eq!(result, "echo ");
    }

    // -- complex expressions --

    #[test]
    fn preprocess_expressions_evaluates_complex_expression() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("INPUT_COMPONENTS".to_string(), "rustfmt".to_string());

        let mut step_outputs = HashMap::new();
        let mut parse_out = HashMap::new();
        parse_out.insert("toolchain".to_string(), "nightly".to_string());
        step_outputs.insert("parse".to_string(), parse_out);

        // This is the dtolnay/rust-toolchain pattern that triggered the original bug
        let text = "rustup toolchain install nightly${{ steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || '' }}";
        let result = preprocess_expressions(text, dir.path(), &None, &step_outputs, &env).unwrap();
        assert_eq!(result, "rustup toolchain install nightly --allow-downgrade");
    }

    #[test]
    fn preprocess_expressions_evaluates_comparison_to_empty() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("INPUT_COMPONENTS".to_string(), "rustfmt".to_string());

        let mut step_outputs = HashMap::new();
        let mut parse_out = HashMap::new();
        parse_out.insert("toolchain".to_string(), "stable".to_string());
        step_outputs.insert("parse".to_string(), parse_out);

        let text = "rustup toolchain install stable${{ steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || '' }}";
        let result = preprocess_expressions(text, dir.path(), &None, &step_outputs, &env).unwrap();
        // stable != nightly, so the expression evaluates to ''
        assert_eq!(result, "rustup toolchain install stable");
    }

    #[test]
    fn preprocess_expressions_no_spaces() {
        let dir = tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());

        // dtolnay/rust-toolchain uses ${{runner.os}} without spaces
        let text = "if [[ ${{runner.os}} == macOS ]]; then echo mac; fi";
        let result =
            preprocess_expressions(text, dir.path(), &None, &HashMap::new(), &env).unwrap();
        assert_eq!(result, "if [[ Linux == macOS ]]; then echo mac; fi");
    }
}
