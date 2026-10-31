// matrix crate

use indexmap::IndexMap;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use thiserror::Error;

/// The value of a matrix `include` or `exclude` key: either a literal list, or
/// an expression such as `${{ fromJSON(needs.setup.outputs.matrix) }}` that can
/// only be evaluated once the job is about to run.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MatrixEntries {
    List(Vec<HashMap<String, Value>>),
    Expression(String),
}

impl<'de> Deserialize<'de> for MatrixEntries {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match Value::deserialize(deserializer)? {
            Value::String(expr) if is_expression(&expr) => Ok(MatrixEntries::Expression(expr)),
            list @ Value::Sequence(_) => serde_yaml::from_value(list)
                .map(MatrixEntries::List)
                .map_err(de::Error::custom),
            _ => Err(de::Error::custom(
                "matrix include/exclude must be an array of objects or a ${{ }} expression",
            )),
        }
    }
}

/// Check `expr` against what `strategy` allows and evaluate it to JSON text.
/// An upstream output that was never set evaluates to nothing, which would
/// otherwise surface as a JSON parser error about line 1, column 0.
fn evaluate_to_json<F>(
    label: &str,
    expr: &str,
    expected: &str,
    evaluate: &mut F,
) -> Result<String, MatrixError>
where
    F: FnMut(&str) -> Result<String, String>,
{
    check_strategy_expression(expr)
        .map_err(|e| MatrixError::ExpressionError(format!("{} {}", label, e)))?;
    let json = evaluate(expr)
        .map_err(|e| MatrixError::ExpressionError(format!("{} '{}': {}", label, expr, e)))?;
    if json.trim().is_empty() {
        return Err(MatrixError::ExpressionError(format!(
            "{} '{}' must evaluate to {}, got an empty value (is the upstream output set?)",
            label, expr, expected
        )));
    }
    Ok(json)
}

/// Whether `value` is a single `${{ ... }}` expression.
pub fn is_expression(value: &str) -> bool {
    expression_body(value).is_some()
}

fn expression_body(value: &str) -> Option<&str> {
    let body = value.trim().strip_prefix("${{")?.strip_suffix("}}")?;
    (!body.contains("${{")).then_some(body)
}

/// Check that an expression only uses what GitHub Actions allows under
/// `jobs.<job_id>.strategy`: the `github`, `needs`, `vars` and `inputs` contexts,
/// and no special functions. Returns an error naming whatever else it uses.
pub fn check_strategy_expression(expr: &str) -> Result<(), String> {
    const CONTEXTS: [&str; 4] = ["github", "needs", "vars", "inputs"];
    const LITERALS: [&str; 3] = ["true", "false", "null"];
    const SPECIAL_FUNCTIONS: [&str; 5] = ["success", "failure", "always", "cancelled", "hashfiles"];

    let body = expression_body(expr).unwrap_or(expr);
    let bytes = body.as_bytes();
    let mut disallowed: Vec<&str> = Vec::new();
    let mut previous = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\'' {
            // A doubled quote reads as two adjacent strings, which is equivalent here
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                i += 1;
            }
            i += 1;
            previous = byte;
        } else if byte.is_ascii_digit() {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                i += 1;
            }
            previous = byte;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-'))
            {
                i += 1;
            }
            let word = &body[start..i];
            let lower = word.to_ascii_lowercase();
            let is_call = body[i..].trim_start().starts_with('(');
            // Property names follow a dot, and only the root names a context
            let allowed = previous == b'.'
                || if is_call {
                    !SPECIAL_FUNCTIONS.contains(&lower.as_str())
                } else {
                    CONTEXTS.contains(&lower.as_str()) || LITERALS.contains(&lower.as_str())
                };
            if !allowed && !disallowed.contains(&word) {
                disallowed.push(word);
            }
            previous = byte;
        } else {
            if !byte.is_ascii_whitespace() {
                previous = byte;
            }
            i += 1;
        }
    }

    if disallowed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "'{}' uses {}, not available in strategy (allowed contexts: github, needs, vars, inputs)",
            expr,
            disallowed.join(", ")
        ))
    }
}

impl Default for MatrixEntries {
    fn default() -> Self {
        MatrixEntries::List(Vec::new())
    }
}

impl MatrixEntries {
    /// Replace an expression with the list it evaluates to. `evaluate` receives
    /// the expression and returns its result as JSON text.
    fn resolve<F>(&self, section: &str, evaluate: &mut F) -> Result<Self, MatrixError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        let MatrixEntries::Expression(expr) = self else {
            return Ok(self.clone());
        };
        let json = evaluate_to_json(section, expr, "an array of objects", evaluate)?;
        let entries = serde_json::from_str(&json).map_err(|e| {
            MatrixError::ExpressionError(format!(
                "{} '{}' must evaluate to an array of objects, got '{}': {}",
                section, expr, json, e
            ))
        })?;
        Ok(MatrixEntries::List(entries))
    }

    fn literal(&self, section: &str) -> Result<&[HashMap<String, Value>], MatrixError> {
        match self {
            MatrixEntries::List(entries) => Ok(entries),
            MatrixEntries::Expression(expr) => Err(MatrixError::ExpressionError(format!(
                "{} '{}' is only known at run time",
                section, expr
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MatrixConfig {
    #[serde(flatten)]
    pub parameters: IndexMap<String, Value>,
    #[serde(default)]
    pub include: MatrixEntries,
    #[serde(default)]
    pub exclude: MatrixEntries,
    #[serde(default, rename = "max-parallel")]
    pub max_parallel: Option<usize>,
    #[serde(default, rename = "fail-fast")]
    pub fail_fast: Option<bool>,
    /// Set when the whole matrix is one expression, such as
    /// `matrix: ${{ fromJSON(needs.setup.outputs.matrix) }}`. Every other field
    /// is empty until [`MatrixConfig::resolve_expressions`] replaces this config.
    /// A bare matrix config can't serialize this, because it is a string rather
    /// than a mapping: `Strategy` in the parser crate writes it back.
    #[serde(skip)]
    pub expression: Option<String>,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            expression: None,
            parameters: IndexMap::new(),
            include: MatrixEntries::default(),
            exclude: MatrixEntries::default(),
            max_parallel: None,
            fail_fast: Some(true),
        }
    }
}

impl MatrixConfig {
    /// A matrix given as one `${{ }}` expression, known only at run time.
    pub fn from_expression(expr: String) -> Self {
        Self {
            expression: Some(expr),
            // Same as a parsed mapping, so `Job::fail_fast` applies its own default
            fail_fast: None,
            ..Self::default()
        }
    }

    /// Returns a copy with every expression (the whole matrix, an axis, or
    /// `include`/`exclude`) replaced by what it evaluates to, ready for
    /// [`expand_matrix`].
    pub fn resolve_expressions<F>(&self, mut evaluate: F) -> Result<Self, MatrixError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        if let Some(expr) = &self.expression {
            return Self::evaluate_whole(expr, &mut evaluate);
        }
        let mut parameters = IndexMap::with_capacity(self.parameters.len());
        for (axis, value) in &self.parameters {
            let value = match value {
                Value::String(expr) if is_expression(expr) => {
                    resolve_axis(axis, expr, &mut evaluate)?
                }
                other => other.clone(),
            };
            parameters.insert(axis.clone(), value);
        }
        Ok(Self {
            parameters,
            include: self.include.resolve("include", &mut evaluate)?,
            exclude: self.exclude.resolve("exclude", &mut evaluate)?,
            ..self.clone()
        })
    }

    /// The result is data, as on GitHub: a string in it that looks like an
    /// expression is never evaluated, so `include`/`exclude` must be lists.
    fn evaluate_whole<F>(expr: &str, evaluate: &mut F) -> Result<Self, MatrixError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        let json = evaluate_to_json("matrix", expr, "an object", evaluate)?;
        let matrix: Self = serde_json::from_str(&json).map_err(|e| {
            MatrixError::ExpressionError(format!(
                "matrix '{}' must evaluate to an object, got '{}': {}",
                expr, json, e
            ))
        })?;
        for (section, entries) in [("include", &matrix.include), ("exclude", &matrix.exclude)] {
            if let MatrixEntries::Expression(value) = entries {
                return Err(MatrixError::ExpressionError(format!(
                    "matrix '{}' must give {} as an array of objects, got '{}'",
                    expr, section, value
                )));
            }
        }
        Ok(matrix)
    }
}

/// Evaluate an axis such as `os: ${{ fromJSON(needs.setup.outputs.os) }}`.
fn resolve_axis<F>(axis: &str, expr: &str, evaluate: &mut F) -> Result<Value, MatrixError>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let label = format!("matrix axis {}", axis);
    let json = evaluate_to_json(&label, expr, "an array", evaluate)?;
    match serde_json::from_str(&json) {
        Ok(value @ Value::Sequence(_)) => Ok(value),
        _ => Err(MatrixError::ExpressionError(format!(
            "{} '{}' must evaluate to an array, got '{}'",
            label, expr, json
        ))),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatrixCombination {
    pub values: HashMap<String, Value>,
    pub is_included: bool, // Whether this was added via the include section
}

impl MatrixCombination {
    pub fn new(values: HashMap<String, Value>) -> Self {
        Self {
            values,
            is_included: false,
        }
    }

    pub fn from_include(values: HashMap<String, Value>) -> Self {
        Self {
            values,
            is_included: true,
        }
    }
}

#[derive(Error, Debug)]
pub enum MatrixError {
    #[error("Invalid matrix parameter format: {0}")]
    InvalidParameterFormat(String),

    #[error("Failed to expand matrix: {0}")]
    ExpansionError(String),

    #[error("Invalid matrix expression: {0}")]
    ExpressionError(String),
}

/// Expands a matrix configuration into a list of all valid combinations
pub fn expand_matrix(matrix: &MatrixConfig) -> Result<Vec<MatrixCombination>, MatrixError> {
    if let Some(expr) = &matrix.expression {
        return Err(MatrixError::ExpressionError(format!(
            "matrix '{}' is only known at run time",
            expr
        )));
    }
    let mut combinations = Vec::new();
    let include = matrix.include.literal("include")?;
    let exclude = matrix.exclude.literal("exclude")?;

    // Step 1: Generate base combinations from parameter arrays
    let param_combinations = generate_base_combinations(matrix, include)?;

    // Step 2: Filter out any combinations that match the exclude patterns
    let filtered_combinations = apply_exclude_filters(param_combinations, exclude);
    combinations.extend(filtered_combinations);

    // Step 3: Process include entries per GitHub Actions semantics:
    // If an include entry matches all shared keys of an existing combination,
    // merge extra keys into it. Otherwise, add as a new standalone combination.
    for include_item in include {
        let mut merged = false;
        for combo in &mut combinations {
            let all_shared_keys_match = include_item.iter().all(|(key, value)| {
                match combo.values.get(key) {
                    Some(existing_value) => existing_value == value,
                    None => true, // Key not in base combo = no conflict, it's a new key to add
                }
            });
            // Only merge if there's at least one matching key (not purely new keys)
            let has_matching_key = include_item
                .keys()
                .any(|key| combo.values.contains_key(key));
            if all_shared_keys_match && has_matching_key {
                // Merge extra keys into the existing combination.
                // or_insert_with is intentional: per GitHub Actions semantics, include
                // entries add new keys but do NOT override existing matrix values.
                for (key, value) in include_item {
                    combo
                        .values
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
                merged = true;
                // Don't break — merge into ALL matching combinations per GitHub Actions semantics
            }
        }
        if !merged {
            combinations.push(MatrixCombination::from_include(include_item.clone()));
        }
    }

    if combinations.is_empty() {
        return Err(MatrixError::ExpansionError(
            "No valid combinations found after applying filters".to_string(),
        ));
    }

    Ok(combinations)
}

/// Generates all possible combinations of the base matrix parameters
fn generate_base_combinations(
    matrix: &MatrixConfig,
    include: &[HashMap<String, Value>],
) -> Result<Vec<MatrixCombination>, MatrixError> {
    // Extract parameter arrays and prepare for combination generation
    let mut param_arrays: IndexMap<String, Vec<Value>> = IndexMap::new();

    for (param_name, param_value) in &matrix.parameters {
        match param_value {
            Value::Sequence(array) => {
                param_arrays.insert(param_name.clone(), array.clone());
            }
            _ => {
                // Handle non-array parameters
                let single_value = vec![param_value.clone()];
                param_arrays.insert(param_name.clone(), single_value);
            }
        }
    }

    if param_arrays.is_empty() {
        // A matrix may consist of nothing but `include` entries
        if !include.is_empty() {
            return Ok(Vec::new());
        }
        return Err(MatrixError::InvalidParameterFormat(
            "Matrix has no valid parameters".to_string(),
        ));
    }

    // Generate the Cartesian product of all parameter arrays
    let param_names: Vec<String> = param_arrays.keys().cloned().collect();
    let param_values: Vec<Vec<Value>> = param_arrays.values().cloned().collect();

    // Generate all combinations using itertools
    let combinations = if !param_values.is_empty() {
        generate_combinations(&param_names, &param_values, 0, &mut HashMap::new())?
    } else {
        vec![]
    };

    Ok(combinations)
}

/// Recursive function to generate combinations using depth-first approach
fn generate_combinations(
    param_names: &[String],
    param_values: &[Vec<Value>],
    current_depth: usize,
    current_combination: &mut HashMap<String, Value>,
) -> Result<Vec<MatrixCombination>, MatrixError> {
    if current_depth == param_names.len() {
        // We've reached a complete combination
        return Ok(vec![MatrixCombination::new(current_combination.clone())]);
    }

    let mut result = Vec::new();
    let param_name = &param_names[current_depth];
    let values = &param_values[current_depth];

    for value in values {
        current_combination.insert(param_name.clone(), value.clone());

        let mut new_combinations = generate_combinations(
            param_names,
            param_values,
            current_depth + 1,
            current_combination,
        )?;

        result.append(&mut new_combinations);
    }

    // Remove this level's parameter to backtrack
    current_combination.remove(param_name);

    Ok(result)
}

/// Filters out combinations that match any of the exclude patterns
fn apply_exclude_filters(
    combinations: Vec<MatrixCombination>,
    exclude_patterns: &[HashMap<String, Value>],
) -> Vec<MatrixCombination> {
    if exclude_patterns.is_empty() {
        return combinations;
    }

    combinations
        .into_iter()
        .filter(|combination| !is_excluded(combination, exclude_patterns))
        .collect()
}

/// Checks if a combination matches any exclude pattern
fn is_excluded(
    combination: &MatrixCombination,
    exclude_patterns: &[HashMap<String, Value>],
) -> bool {
    for exclude in exclude_patterns {
        let mut excluded = true;

        for (key, value) in exclude {
            match combination.values.get(key) {
                Some(combo_value) if combo_value == value => {
                    // This exclude condition matches
                    continue;
                }
                _ => {
                    // This exclude condition doesn't match
                    excluded = false;
                    break;
                }
            }
        }

        if excluded {
            return true;
        }
    }

    false
}

/// Formats a combination name for display, e.g. "test (ubuntu, node 14)"
pub fn format_combination_name(job_name: &str, combination: &MatrixCombination) -> String {
    let params = combination
        .values
        .iter()
        .map(|(k, v)| format!("{}: {}", k, value_to_string(v)))
        .collect::<Vec<_>>()
        .join(", ");

    format!("{} ({})", job_name, params)
}

/// Converts a serde_yaml::Value to a string for display
fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Sequence(seq) => {
            let items = seq
                .iter()
                .map(value_to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", items)
        }
        Value::Mapping(map) => {
            let items = map
                .iter()
                .map(|(k, v)| format!("{}: {}", value_to_string(k), value_to_string(v)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{}}}", items)
        }
        Value::Null => "null".to_string(),
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val(s: &str) -> Value {
        Value::String(s.to_string())
    }

    fn make_matrix(params: Vec<(&str, Vec<&str>)>) -> MatrixConfig {
        let mut parameters = IndexMap::new();
        for (name, values) in params {
            parameters.insert(
                name.to_string(),
                Value::Sequence(values.into_iter().map(val).collect()),
            );
        }
        MatrixConfig {
            expression: None,
            parameters,
            include: MatrixEntries::default(),
            exclude: MatrixEntries::default(),
            max_parallel: None,
            fail_fast: Some(true),
        }
    }

    #[test]
    fn include_merges_into_matching_combination() {
        let mut matrix = make_matrix(vec![("os", vec!["ubuntu", "windows"])]);
        // Include entry that matches os=ubuntu and adds a new key
        let mut include_entry = HashMap::new();
        include_entry.insert("os".to_string(), val("ubuntu"));
        include_entry.insert("compiler".to_string(), val("gcc"));
        matrix.include = MatrixEntries::List(vec![include_entry]);

        let combos = expand_matrix(&matrix).unwrap();

        // Should have 2 combos: ubuntu (with compiler=gcc merged) and windows
        assert_eq!(combos.len(), 2);

        let ubuntu = combos
            .iter()
            .find(|c| c.values.get("os") == Some(&val("ubuntu")))
            .unwrap();
        assert_eq!(ubuntu.values.get("compiler"), Some(&val("gcc")));

        let windows = combos
            .iter()
            .find(|c| c.values.get("os") == Some(&val("windows")))
            .unwrap();
        assert_eq!(windows.values.get("compiler"), None);
    }

    #[test]
    fn include_adds_standalone_when_no_match() {
        let mut matrix = make_matrix(vec![("os", vec!["ubuntu", "windows"])]);
        // Include entry with a value that doesn't match any existing combo
        let mut include_entry = HashMap::new();
        include_entry.insert("os".to_string(), val("macos"));
        include_entry.insert("special".to_string(), val("true"));
        matrix.include = MatrixEntries::List(vec![include_entry]);

        let combos = expand_matrix(&matrix).unwrap();

        // Should have 3 combos: ubuntu, windows, macos (standalone)
        assert_eq!(combos.len(), 3);

        let macos = combos
            .iter()
            .find(|c| c.values.get("os") == Some(&val("macos")))
            .unwrap();
        assert_eq!(macos.values.get("special"), Some(&val("true")));
        assert!(macos.is_included);
    }

    #[test]
    fn include_merges_into_all_matching_combinations() {
        let mut matrix = make_matrix(vec![
            ("os", vec!["ubuntu", "windows"]),
            ("node", vec!["16", "18"]),
        ]);
        // Include entry matching os=ubuntu — should merge into both (ubuntu, 16) and (ubuntu, 18)
        let mut include_entry = HashMap::new();
        include_entry.insert("os".to_string(), val("ubuntu"));
        include_entry.insert("extra".to_string(), val("yes"));
        matrix.include = MatrixEntries::List(vec![include_entry]);

        let combos = expand_matrix(&matrix).unwrap();

        // 4 base combos, no new standalone (merged into 2 existing)
        assert_eq!(combos.len(), 4);

        let ubuntu_combos: Vec<_> = combos
            .iter()
            .filter(|c| c.values.get("os") == Some(&val("ubuntu")))
            .collect();
        assert_eq!(ubuntu_combos.len(), 2);
        for combo in &ubuntu_combos {
            assert_eq!(combo.values.get("extra"), Some(&val("yes")));
        }

        let windows_combos: Vec<_> = combos
            .iter()
            .filter(|c| c.values.get("os") == Some(&val("windows")))
            .collect();
        for combo in &windows_combos {
            assert_eq!(combo.values.get("extra"), None);
        }
    }

    #[test]
    fn include_with_only_new_keys_adds_standalone() {
        let mut matrix = make_matrix(vec![("os", vec!["ubuntu"])]);
        // Include entry with only keys that don't exist in base combos
        let mut include_entry = HashMap::new();
        include_entry.insert("arch".to_string(), val("arm64"));
        matrix.include = MatrixEntries::List(vec![include_entry]);

        let combos = expand_matrix(&matrix).unwrap();

        // Should have 2: the base ubuntu + standalone arm64
        assert_eq!(combos.len(), 2);
    }

    #[test]
    fn exclude_removes_matching_combinations() {
        let mut matrix = make_matrix(vec![
            ("os", vec!["ubuntu", "windows"]),
            ("node", vec!["16", "18"]),
        ]);
        let mut exclude_entry = HashMap::new();
        exclude_entry.insert("os".to_string(), val("windows"));
        exclude_entry.insert("node".to_string(), val("16"));
        matrix.exclude = MatrixEntries::List(vec![exclude_entry]);

        let combos = expand_matrix(&matrix).unwrap();

        // 4 - 1 excluded = 3
        assert_eq!(combos.len(), 3);
        assert!(!combos.iter().any(|c| {
            c.values.get("os") == Some(&val("windows")) && c.values.get("node") == Some(&val("16"))
        }));
    }

    #[test]
    fn include_only_matrix_expands_to_its_entries() {
        let mut entry = HashMap::new();
        entry.insert("os".to_string(), val("ubuntu"));
        let matrix = MatrixConfig {
            include: MatrixEntries::List(vec![entry]),
            ..MatrixConfig::default()
        };

        let combos = expand_matrix(&matrix).unwrap();

        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].values.get("os"), Some(&val("ubuntu")));
    }

    #[test]
    fn empty_matrix_is_still_an_error() {
        assert!(expand_matrix(&MatrixConfig::default()).is_err());
    }

    #[test]
    fn expressions_deserialize_alongside_axes() {
        let matrix: MatrixConfig = serde_yaml::from_str(
            "os: [ubuntu]\ninclude: ${{ fromJSON(needs.a.outputs.m) }}\nexclude:\n  - os: ubuntu\n",
        )
        .unwrap();

        assert_eq!(
            matrix.include,
            MatrixEntries::Expression("${{ fromJSON(needs.a.outputs.m) }}".to_string())
        );
        assert!(matches!(matrix.exclude, MatrixEntries::List(ref l) if l.len() == 1));
        assert!(matrix.parameters.contains_key("os"));
    }

    #[test]
    fn expand_rejects_unresolved_expressions() {
        let mut matrix = make_matrix(vec![("os", vec!["ubuntu"])]);
        matrix.include = MatrixEntries::Expression("${{ needs.a.outputs.m }}".to_string());

        assert!(matches!(
            expand_matrix(&matrix),
            Err(MatrixError::ExpressionError(_))
        ));
    }

    #[test]
    fn resolved_include_merges_numbers_with_yaml_axes() {
        let matrix: MatrixConfig = serde_yaml::from_str(
            "os: [ubuntu]\nnode: [14, 16]\ninclude: ${{ needs.a.outputs.m }}\n",
        )
        .unwrap();

        let resolved = matrix
            .resolve_expressions(|_| Ok(r#"[{"node": 14, "npm": 6}]"#.to_string()))
            .unwrap();
        let combos = expand_matrix(&resolved).unwrap();

        assert_eq!(combos.len(), 2);
        assert_eq!(
            combos
                .iter()
                .filter(|c| c.values.contains_key("npm"))
                .count(),
            1
        );
    }

    #[test]
    fn resolved_exclude_filters_combinations() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu, windows]\nexclude: ${{ needs.a.outputs.m }}\n")
                .unwrap();

        let resolved = matrix
            .resolve_expressions(|_| Ok(r#"[{"os": "windows"}]"#.to_string()))
            .unwrap();
        let combos = expand_matrix(&resolved).unwrap();

        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].values.get("os"), Some(&val("ubuntu")));
    }

    #[test]
    fn resolve_rejects_non_list_results() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ needs.a.outputs.m }}\n").unwrap();

        for bad in ["", "not json", "5", r#"{"os": "ubuntu"}"#, "[1, 2]"] {
            let result = matrix.resolve_expressions(|_| Ok(bad.to_string()));
            assert!(
                matches!(result, Err(MatrixError::ExpressionError(_))),
                "{:?} should be rejected",
                bad
            );
        }
    }

    #[test]
    fn empty_result_is_reported_as_empty() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ needs.a.outputs.m }}\n").unwrap();

        let err = matrix
            .resolve_expressions(|_| Ok("  ".to_string()))
            .unwrap_err();

        assert!(err.to_string().contains("got an empty value"), "{}", err);
    }

    #[test]
    fn resolve_refuses_contexts_strategy_cannot_see() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ fromJSON(env.X) }}\n").unwrap();

        let err = matrix
            .resolve_expressions(|_| panic!("must not be evaluated"))
            .unwrap_err();

        assert!(err.to_string().contains("uses env"), "{}", err);
    }

    #[test]
    fn strategy_expressions_allow_github_needs_vars_and_inputs() {
        for expr in [
            "${{ fromJSON(needs.a-b.outputs.m) }}",
            "${{ fromJSON(inputs.list) }}",
            "${{ fromJSON(vars.LIST) }}",
            "${{ github.event_name == 'push' && fromJSON(needs.a.outputs.env) || null }}",
            "${{ contains(github.ref, 'env.X') && 'it''s env' }}",
            "${{ NEEDS.a.outputs.m }}",
            "${{ needs.a.outputs['matrix'] == 1.5 }}",
        ] {
            assert_eq!(check_strategy_expression(expr), Ok(()), "{}", expr);
        }
    }

    #[test]
    fn strategy_expressions_reject_other_contexts_and_special_functions() {
        for (expr, name) in [
            ("${{ env.X }}", "env"),
            ("${{ fromJSON(secrets.LIST) }}", "secrets"),
            ("${{ matrix.os }}", "matrix"),
            ("${{ steps.a.outputs.b }}", "steps"),
            ("${{ runner.os }}", "runner"),
            ("${{ success() && needs.a.outputs.m }}", "success"),
            ("${{ hashFiles('x') }}", "hashFiles"),
        ] {
            let err = check_strategy_expression(expr).unwrap_err();
            assert!(err.contains(&format!("uses {}", name)), "{}", err);
        }
    }

    #[test]
    fn non_expression_values_are_rejected_when_parsing() {
        for bad in [
            "not-an-expression",
            "5",
            "{ os: ubuntu }",
            "'${{ a }} ${{ b }}'",
        ] {
            let err =
                serde_yaml::from_str::<MatrixConfig>(&format!("os: [ubuntu]\ninclude: {}\n", bad))
                    .unwrap_err();
            assert!(
                err.to_string().contains("array of objects"),
                "{}: {}",
                bad,
                err
            );
        }
    }

    #[test]
    fn resolve_propagates_evaluation_errors() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ needs.a.outputs.m }}\n").unwrap();

        let err = matrix
            .resolve_expressions(|_| Err("bad token".to_string()))
            .unwrap_err();

        assert!(err.to_string().contains("bad token"));
    }

    #[test]
    fn whole_matrix_expression_resolves_with_axes_and_include() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
        assert!(expand_matrix(&matrix).is_err());

        let resolved = matrix
            .resolve_expressions(|_| {
                Ok(r#"{"os":["a","b"],"n":[1],"include":[{"os":"c","n":9}]}"#.to_string())
            })
            .unwrap();
        assert_eq!(expand_matrix(&resolved).unwrap().len(), 3);
    }

    #[test]
    fn whole_matrix_expression_keeps_axis_order() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
        let resolved = matrix
            .resolve_expressions(|_| Ok(r#"{"zeta":["a"],"alpha":["b"]}"#.to_string()))
            .unwrap();
        let axes: Vec<_> = resolved.parameters.keys().collect();
        assert_eq!(axes, ["zeta", "alpha"]);
    }

    #[test]
    fn whole_matrix_result_is_not_evaluated_again() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
        let mut calls = 0;
        let resolved = matrix
            .resolve_expressions(|_| {
                calls += 1;
                Ok(r#"{"tpl":"${{ github.sha }}"}"#.to_string())
            })
            .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(
            resolved.parameters["tpl"],
            Value::String("${{ github.sha }}".to_string())
        );
    }

    #[test]
    fn whole_matrix_result_cannot_defer_include_or_exclude() {
        for section in ["include", "exclude"] {
            let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
            let json = format!(r#"{{"os":["a"],"{}":"${{{{ github.sha }}}}"}}"#, section);
            let err = matrix
                .resolve_expressions(|_| Ok(json.clone()))
                .unwrap_err()
                .to_string();
            assert!(err.contains(section), "{}", err);
            assert!(err.contains("array of objects"), "{}", err);
        }
    }

    #[test]
    fn whole_matrix_and_axis_report_an_empty_value() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
        let err = matrix
            .resolve_expressions(|_| Ok("  ".to_string()))
            .unwrap_err();
        assert!(err.to_string().contains("got an empty value"), "{}", err);

        let mut matrix = MatrixConfig::default();
        matrix.parameters.insert(
            "os".to_string(),
            Value::String("${{ fromJSON(needs.a.outputs.os) }}".to_string()),
        );
        let err = matrix
            .resolve_expressions(|_| Ok(String::new()))
            .unwrap_err();
        assert!(err.to_string().contains("got an empty value"), "{}", err);
    }

    #[test]
    fn whole_matrix_expression_must_be_an_object() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(needs.a.outputs.m) }}".into());
        let err = matrix
            .resolve_expressions(|_| Ok("[1,2]".to_string()))
            .unwrap_err();
        assert!(err.to_string().contains("must evaluate to an object"));
    }

    #[test]
    fn whole_matrix_expression_checks_allowed_contexts() {
        let matrix = MatrixConfig::from_expression("${{ fromJSON(steps.a.outputs.m) }}".into());
        assert!(matrix
            .resolve_expressions(|_| Ok("{}".to_string()))
            .is_err());
    }

    #[test]
    fn axis_expression_resolves_to_its_array() {
        let mut matrix = MatrixConfig::default();
        matrix.parameters.insert(
            "os".to_string(),
            Value::String("${{ fromJSON(needs.a.outputs.os) }}".to_string()),
        );
        let resolved = matrix
            .resolve_expressions(|_| Ok(r#"["a","b"]"#.to_string()))
            .unwrap();
        assert_eq!(expand_matrix(&resolved).unwrap().len(), 2);

        let err = matrix
            .resolve_expressions(|_| Ok("".to_string()))
            .unwrap_err();
        assert!(err.to_string().contains("must evaluate to an array"));
    }
}
