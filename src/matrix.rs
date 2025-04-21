use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MatrixConfig {
    #[serde(flatten)]
    pub parameters: IndexMap<String, Value>,
    #[serde(default)]
    pub include: Vec<HashMap<String, Value>>,
    #[serde(default)]
    pub exclude: Vec<HashMap<String, Value>>,
    #[serde(default, rename = "max-parallel")]
    pub max_parallel: Option<usize>,
    #[serde(default, rename = "fail-fast")]
    pub fail_fast: Option<bool>,
}

impl Default for MatrixConfig {
    fn default() -> Self {
        Self {
            parameters: IndexMap::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            max_parallel: None,
            fail_fast: Some(true),
        }
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
}

/// Expands a matrix configuration into a list of all valid combinations
pub fn expand_matrix(matrix: &MatrixConfig) -> Result<Vec<MatrixCombination>, MatrixError> {
    let mut combinations = Vec::new();

    // Step 1: Generate base combinations from parameter arrays
    let param_combinations = generate_base_combinations(matrix)?;

    // Step 2: Filter out any combinations that match the exclude patterns
    let filtered_combinations = apply_exclude_filters(param_combinations, &matrix.exclude);
    combinations.extend(filtered_combinations);

    // Step 3: Add any combinations from the include section
    for include_item in &matrix.include {
        combinations.push(MatrixCombination::from_include(include_item.clone()));
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
