use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{validate_env, validate_matrix, validate_steps};
use serde_yaml::Value;
use wrkflw_models::ValidationResult;

pub fn validate_jobs(jobs: &Value, repo_root: Option<&Path>, result: &mut ValidationResult) {
    if let Value::Mapping(jobs_map) = jobs {
        if jobs_map.is_empty() {
            result.add_issue("'jobs' section is empty".to_string());
            return;
        }

        for (job_name, job_config) in jobs_map {
            if let Some(job_name) = job_name.as_str() {
                if let Some(job_config) = job_config.as_mapping() {
                    // Check if this is a reusable workflow job (has 'uses' field)
                    let is_reusable_workflow =
                        job_config.contains_key(Value::String("uses".to_string()));

                    // Only check for 'runs-on' if it's not a reusable workflow
                    if !is_reusable_workflow
                        && !job_config.contains_key(Value::String("runs-on".to_string()))
                    {
                        result.add_issue(format!("Job '{}' is missing 'runs-on' field", job_name));
                    }

                    // Only check for steps if it's not a reusable workflow
                    if !is_reusable_workflow {
                        match job_config.get(Value::String("steps".to_string())) {
                            Some(Value::Sequence(steps)) => {
                                if steps.is_empty() {
                                    result.add_issue(format!(
                                        "Job '{}' has empty 'steps' section",
                                        job_name
                                    ));
                                } else {
                                    validate_steps(steps, job_name, repo_root, result);
                                }
                            }
                            Some(_) => {
                                result.add_issue(format!(
                                    "Job '{}': 'steps' section is not a sequence",
                                    job_name
                                ));
                            }
                            None => {
                                result.add_issue(format!(
                                    "Job '{}' is missing 'steps' section",
                                    job_name
                                ));
                            }
                        }
                    } else {
                        // For reusable workflows, validate the 'uses' field format
                        if let Some(Value::String(uses)) =
                            job_config.get(Value::String("uses".to_string()))
                        {
                            // Simple validation for reusable workflow reference format
                            if !uses.contains('/') && !uses.contains('.') {
                                result.add_issue(format!(
                                    "Job '{}': Invalid reusable workflow reference format '{}'",
                                    job_name, uses
                                ));
                            }
                        }
                    }

                    // Check for job dependencies
                    if let Some(Value::Sequence(needs)) =
                        job_config.get(Value::String("needs".to_string()))
                    {
                        for need in needs {
                            if let Some(need_str) = need.as_str() {
                                if !jobs_map.contains_key(Value::String(need_str.to_string())) {
                                    result.add_issue(format!(
                                        "Job '{}' depends on non-existent job '{}'",
                                        job_name, need_str
                                    ));
                                }
                            }
                        }
                    } else if let Some(Value::String(need)) =
                        job_config.get(Value::String("needs".to_string()))
                    {
                        if !jobs_map.contains_key(Value::String(need.clone())) {
                            result.add_issue(format!(
                                "Job '{}' depends on non-existent job '{}'",
                                job_name, need
                            ));
                        }
                    }

                    // Validate env is a mapping, not a bare string
                    if let Some(env_val) = job_config.get(Value::String("env".to_string())) {
                        validate_env(env_val, &format!("Job '{}'", job_name), result);
                    }

                    // Validate matrix configuration if present
                    if let Some(strategy) = job_config.get(Value::String("strategy".to_string())) {
                        if let Some(strategy_map) = strategy.as_mapping() {
                            if let Some(matrix) =
                                strategy_map.get(Value::String("matrix".to_string()))
                            {
                                validate_matrix(matrix, result);
                            }
                        }
                    }
                } else {
                    result.add_issue(format!("Job '{}' configuration is not a mapping", job_name));
                }
            }
        }

        detect_cyclic_needs(jobs_map, result);
    }
}

/// Build an adjacency list from jobs' `needs` fields and detect cycles via DFS.
fn detect_cyclic_needs(jobs_map: &serde_yaml::Mapping, result: &mut ValidationResult) {
    // Build adjacency graph: job_name -> list of jobs it needs
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();

    for (job_name, job_config) in jobs_map {
        let name = match job_name.as_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        let mut deps = Vec::new();

        if let Some(config) = job_config.as_mapping() {
            if let Some(needs_val) = config.get(Value::String("needs".to_string())) {
                match needs_val {
                    Value::Sequence(seq) => {
                        for item in seq {
                            if let Some(s) = item.as_str() {
                                deps.push(s.to_string());
                            }
                        }
                    }
                    Value::String(s) => {
                        deps.push(s.clone());
                    }
                    _ => {}
                }
            }
        }

        graph.insert(name, deps);
    }

    // DFS cycle detection
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();
    let mut rec_stack = Vec::new();
    let mut reported_cycles: HashSet<Vec<String>> = HashSet::new();

    for job_name in graph.keys() {
        if !visited.contains(job_name.as_str()) {
            dfs_detect_cycle(
                job_name,
                &graph,
                &mut visited,
                &mut in_stack,
                &mut rec_stack,
                &mut reported_cycles,
                result,
            );
        }
    }
}

fn dfs_detect_cycle(
    node: &str,
    graph: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    in_stack: &mut HashSet<String>,
    rec_stack: &mut Vec<String>,
    reported_cycles: &mut HashSet<Vec<String>>,
    result: &mut ValidationResult,
) {
    visited.insert(node.to_string());
    in_stack.insert(node.to_string());
    rec_stack.push(node.to_string());

    if let Some(neighbors) = graph.get(node) {
        for neighbor in neighbors {
            if in_stack.contains(neighbor.as_str()) {
                // Found a cycle — build the cycle path from the stack
                if let Some(pos) = rec_stack.iter().position(|x| x == neighbor) {
                    // Normalize the cycle: rotate so the lexicographically smallest
                    // node is first, ensuring the same cycle isn't reported twice
                    // from different entry points.
                    let mut cycle_nodes: Vec<String> = rec_stack[pos..].to_vec();
                    if let Some(min_pos) = cycle_nodes
                        .iter()
                        .enumerate()
                        .min_by(|(_, a), (_, b)| a.cmp(b))
                        .map(|(i, _)| i)
                    {
                        cycle_nodes.rotate_left(min_pos);
                    }
                    if reported_cycles.insert(cycle_nodes.clone()) {
                        let display = cycle_nodes
                            .iter()
                            .chain(std::iter::once(&cycle_nodes[0]))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(" -> ");
                        result.add_issue(format!(
                            "Circular dependency detected in 'needs': {}",
                            display
                        ));
                    }
                }
            } else if !visited.contains(neighbor.as_str()) {
                dfs_detect_cycle(
                    neighbor,
                    graph,
                    visited,
                    in_stack,
                    rec_stack,
                    reported_cycles,
                    result,
                );
            }
        }
    }

    in_stack.remove(node);
    rec_stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;
    use wrkflw_models::ValidationResult;

    #[test]
    fn test_cyclic_needs_detected() {
        let yaml = r#"
job-a:
  runs-on: ubuntu-latest
  needs: job-b
  steps:
    - run: echo a
job-b:
  runs-on: ubuntu-latest
  needs:
    - job-c
  steps:
    - run: echo b
job-c:
  runs-on: ubuntu-latest
  needs: job-a
  steps:
    - run: echo c
"#;
        let jobs: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_jobs(&jobs, None, &mut result);

        assert!(
            result
                .issues
                .iter()
                .any(|i| i.contains("Circular dependency detected in 'needs'")),
            "Expected a cyclic needs error, got: {:?}",
            result.issues
        );
    }

    #[test]
    fn test_no_false_positive_cycle_with_cross_edge() {
        // A→B is a cycle, D→E→A is a cross-edge (NOT a cycle).
        // Previously, the DFS would leave stale entries in rec_stack after
        // detecting the A↔B cycle, causing a false positive for D→E→A.
        let yaml = r#"
job-a:
  runs-on: ubuntu-latest
  needs: job-b
  steps:
    - run: echo a
job-b:
  runs-on: ubuntu-latest
  needs: job-a
  steps:
    - run: echo b
job-d:
  runs-on: ubuntu-latest
  steps:
    - run: echo d
job-e:
  runs-on: ubuntu-latest
  needs:
    - job-d
    - job-a
  steps:
    - run: echo e
"#;
        let jobs: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_jobs(&jobs, None, &mut result);

        let cycle_issues: Vec<_> = result
            .issues
            .iter()
            .filter(|i| i.contains("Circular dependency detected in 'needs'"))
            .collect();

        // Should have exactly one cycle (A↔B), NOT a false positive involving D or E
        assert_eq!(
            cycle_issues.len(),
            1,
            "Expected exactly 1 cycle issue (A↔B), got: {:?}",
            cycle_issues
        );
        assert!(
            cycle_issues[0].contains("job-a") && cycle_issues[0].contains("job-b"),
            "Cycle should involve job-a and job-b, got: {}",
            cycle_issues[0]
        );
    }

    #[test]
    fn test_no_cycle_in_valid_dag() {
        let yaml = r#"
build:
  runs-on: ubuntu-latest
  steps:
    - run: echo build
test:
  runs-on: ubuntu-latest
  needs: build
  steps:
    - run: echo test
deploy:
  runs-on: ubuntu-latest
  needs:
    - build
    - test
  steps:
    - run: echo deploy
"#;
        let jobs: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_jobs(&jobs, None, &mut result);

        assert!(
            !result
                .issues
                .iter()
                .any(|i| i.contains("Circular dependency")),
            "Valid DAG should not have cycle issues, got: {:?}",
            result.issues
        );
    }

    #[test]
    fn test_job_env_string_is_invalid() {
        let yaml = r#"
build:
  runs-on: ubuntu-latest
  env: VAR=value
  steps:
    - run: echo build
"#;
        let jobs: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_jobs(&jobs, None, &mut result);

        assert!(!result.is_valid);
        assert!(
            result
                .issues
                .iter()
                .any(|i| i.contains("'env' must be a mapping")),
            "Expected env mapping error, got: {:?}",
            result.issues
        );
    }

    #[test]
    fn test_job_env_mapping_is_valid() {
        let yaml = r#"
build:
  runs-on: ubuntu-latest
  env:
    MY_VAR: my_value
  steps:
    - run: echo build
"#;
        let jobs: Value = serde_yaml::from_str(yaml).unwrap();
        let mut result = ValidationResult::new();
        validate_jobs(&jobs, None, &mut result);

        assert!(
            !result.issues.iter().any(|i| i.contains("env")),
            "Valid env mapping should not produce env issues, got: {:?}",
            result.issues
        );
    }
}
