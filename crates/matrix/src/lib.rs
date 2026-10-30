// matrix crate

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use thiserror::Error;

/// The value of a matrix `include` or `exclude` key: either a literal list, or
/// an expression such as `${{ fromJSON(needs.setup.outputs.matrix) }}` that can
/// only be evaluated once the job is about to run.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum MatrixEntries {
    List(Vec<HashMap<String, Value>>),
    Expression(String),
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
        let json = evaluate(expr)
            .map_err(|e| MatrixError::ExpressionError(format!("{} '{}': {}", section, expr, e)))?;
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
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            parameters: IndexMap::new(),
            include: MatrixEntries::default(),
            exclude: MatrixEntries::default(),
            max_parallel: None,
            fail_fast: Some(true),
        }
    }
}

impl MatrixConfig {
    /// Returns a copy with any `include`/`exclude` expression replaced by the
    /// list it evaluates to, ready for [`expand_matrix`].
    pub fn resolve_expressions<F>(&self, mut evaluate: F) -> Result<Self, MatrixError>
    where
        F: FnMut(&str) -> Result<String, String>,
    {
        Ok(Self {
            include: self.include.resolve("include", &mut evaluate)?,
            exclude: self.exclude.resolve("exclude", &mut evaluate)?,
            ..self.clone()
        })
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
        matrix.include = MatrixEntries::Expression("${{ x }}".to_string());

        assert!(matches!(
            expand_matrix(&matrix),
            Err(MatrixError::ExpressionError(_))
        ));
    }

    #[test]
    fn resolved_include_merges_numbers_with_yaml_axes() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\nnode: [14, 16]\ninclude: ${{ x }}\n").unwrap();

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
            serde_yaml::from_str("os: [ubuntu, windows]\nexclude: ${{ x }}\n").unwrap();

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
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ x }}\n").unwrap();

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
    fn resolve_propagates_evaluation_errors() {
        let matrix: MatrixConfig =
            serde_yaml::from_str("os: [ubuntu]\ninclude: ${{ x }}\n").unwrap();

        let err = matrix
            .resolve_expressions(|_| Err("bad token".to_string()))
            .unwrap_err();

        assert!(err.to_string().contains("bad token"));
    }
}
