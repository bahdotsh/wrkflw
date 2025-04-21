#[cfg(test)]
mod tests {
    use crate::matrix::{self, MatrixCombination, MatrixConfig};
    use indexmap::IndexMap;
    use serde_yaml::Value;
    use std::collections::HashMap;

    fn create_test_matrix() -> MatrixConfig {
        let mut matrix = MatrixConfig::default();

        // Add basic parameters
        let mut params = IndexMap::new();

        // Add 'os' parameter with array values
        let os_array = vec![
            Value::String("ubuntu".to_string()),
            Value::String("windows".to_string()),
            Value::String("macos".to_string()),
        ];
        params.insert("os".to_string(), Value::Sequence(os_array));

        // Add 'node' parameter with array values
        let node_array = vec![
            Value::Number(serde_yaml::Number::from(14)),
            Value::Number(serde_yaml::Number::from(16)),
        ];
        params.insert("node".to_string(), Value::Sequence(node_array));

        matrix.parameters = params;

        // Add exclude pattern
        let mut exclude_item = HashMap::new();
        exclude_item.insert("os".to_string(), Value::String("windows".to_string()));
        exclude_item.insert(
            "node".to_string(),
            Value::Number(serde_yaml::Number::from(14)),
        );
        matrix.exclude = vec![exclude_item];

        // Add include pattern
        let mut include_item = HashMap::new();
        include_item.insert("os".to_string(), Value::String("ubuntu".to_string()));
        include_item.insert(
            "node".to_string(),
            Value::Number(serde_yaml::Number::from(18)),
        );
        include_item.insert("experimental".to_string(), Value::Bool(true));
        matrix.include = vec![include_item];

        // Set max-parallel
        matrix.max_parallel = Some(2);

        // Set fail-fast
        matrix.fail_fast = Some(true);

        matrix
    }

    #[test]
    fn test_matrix_expansion() {
        let matrix = create_test_matrix();

        // Expand the matrix
        let combinations = matrix::expand_matrix(&matrix).unwrap();

        // We should have 6 combinations:
        // 3 OS x 2 Node versions = 6 base combinations
        // - 1 excluded (windows + node 14)
        // + 1 included (ubuntu + node 18 + experimental)
        // = 6 total combinations
        assert_eq!(combinations.len(), 6);

        // Check that the excluded combination is not present
        let excluded =
            combinations
                .iter()
                .find(|c| match (c.values.get("os"), c.values.get("node")) {
                    (Some(Value::String(os)), Some(Value::Number(node))) => {
                        os == "windows" && node.as_u64() == Some(14)
                    }
                    _ => false,
                });
        assert!(
            excluded.is_none(),
            "Excluded combination should not be present"
        );

        // Check that the included combination is present
        let included = combinations.iter().find(|c| {
            match (
                c.values.get("os"),
                c.values.get("node"),
                c.values.get("experimental"),
            ) {
                (Some(Value::String(os)), Some(Value::Number(node)), Some(Value::Bool(exp))) => {
                    os == "ubuntu" && node.as_u64() == Some(18) && *exp
                }
                _ => false,
            }
        });
        assert!(included.is_some(), "Included combination should be present");
        assert!(
            included.unwrap().is_included,
            "Combination should be marked as included"
        );
    }

    #[test]
    fn test_format_combination_name() {
        let mut values = HashMap::new();
        values.insert("os".to_string(), Value::String("ubuntu".to_string()));
        values.insert(
            "node".to_string(),
            Value::Number(serde_yaml::Number::from(14)),
        );

        let combination = MatrixCombination {
            values,
            is_included: false,
        };

        let formatted = matrix::format_combination_name("test-job", &combination);

        // Should format as "test-job (os: ubuntu, node: 14)" or similar
        assert!(formatted.contains("test-job"));
        assert!(formatted.contains("os: ubuntu"));
        assert!(formatted.contains("node: 14"));
    }
}
