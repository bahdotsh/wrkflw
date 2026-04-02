use std::collections::{HashMap, HashSet, VecDeque};
use wrkflw_parser::workflow::{Job, WorkflowDefinition};

pub fn resolve_dependencies(workflow: &WorkflowDefinition) -> Result<Vec<Vec<String>>, String> {
    let jobs = &workflow.jobs;

    // Build adjacency list with String keys
    let mut dependencies: HashMap<String, HashSet<String>> = HashMap::new();
    let mut dependents: HashMap<String, HashSet<String>> = HashMap::new();

    // Initialize with empty dependencies
    for job_name in jobs.keys() {
        dependencies.insert(job_name.clone(), HashSet::new());
        dependents.insert(job_name.clone(), HashSet::new());
    }

    // Populate dependencies
    for (job_name, job) in jobs {
        if let Some(needs) = &job.needs {
            for needed_job in needs {
                if !jobs.contains_key(needed_job) {
                    return Err(format!(
                        "Job '{}' depends on non-existent job '{}'",
                        job_name, needed_job
                    ));
                }
                // Get mutable reference to the dependency set for this job, with error handling
                if let Some(deps) = dependencies.get_mut(job_name) {
                    deps.insert(needed_job.clone());
                } else {
                    return Err(format!(
                        "Internal error: Failed to update dependencies for job '{}'",
                        job_name
                    ));
                }

                // Get mutable reference to the dependents set for the needed job, with error handling
                if let Some(deps) = dependents.get_mut(needed_job) {
                    deps.insert(job_name.clone());
                } else {
                    return Err(format!(
                        "Internal error: Failed to update dependents for job '{}'",
                        needed_job
                    ));
                }
            }
        }
    }

    // Implement topological sort for execution ordering
    let mut result = Vec::new();
    let mut no_dependencies: HashSet<String> = dependencies
        .iter()
        .filter(|(_, deps)| deps.is_empty())
        .map(|(job, _)| job.clone())
        .collect();

    // Process levels of the dependency graph
    while !no_dependencies.is_empty() {
        // Current level becomes a batch of jobs that can run in parallel
        let current_level: Vec<String> = no_dependencies.iter().cloned().collect();
        result.push(current_level);

        // For the next level
        let mut next_no_dependencies = HashSet::new();

        for job in &no_dependencies {
            // For each dependent job of the current job
            // Get the set of dependents with error handling
            let dependent_jobs = match dependents.get(job) {
                Some(deps) => deps.clone(),
                None => {
                    return Err(format!(
                        "Internal error: Failed to find dependents for job '{}'",
                        job
                    ));
                }
            };

            for dependent in dependent_jobs {
                // Remove the current job from its dependencies
                if let Some(deps) = dependencies.get_mut(&dependent) {
                    deps.remove(job);

                    // Check if it's empty now to determine if it should be in the next level
                    if deps.is_empty() {
                        next_no_dependencies.insert(dependent);
                    }
                } else {
                    return Err(format!(
                        "Internal error: Failed to find dependencies for job '{}'",
                        dependent
                    ));
                }
            }
        }

        no_dependencies = next_no_dependencies;
    }

    // Check for circular dependencies
    let processed_jobs: HashSet<String> = result
        .iter()
        .flat_map(|level| level.iter().cloned())
        .collect();

    if processed_jobs.len() < jobs.len() {
        let unprocessed: Vec<&String> = jobs
            .keys()
            .filter(|j| !processed_jobs.contains(*j))
            .collect();
        return Err(format!(
            "Circular dependency detected in workflow jobs: {}",
            unprocessed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(result)
}

/// Collect a job and all its transitive dependencies via `needs` edges.
pub fn collect_transitive_deps(target_job: &str, jobs: &HashMap<String, Job>) -> HashSet<String> {
    let mut deps = HashSet::new();
    let mut queue = VecDeque::new();

    deps.insert(target_job.to_string());
    queue.push_back(target_job.to_string());

    while let Some(job_name) = queue.pop_front() {
        if let Some(job) = jobs.get(&job_name) {
            if let Some(needs) = &job.needs {
                for needed in needs {
                    if deps.insert(needed.clone()) {
                        queue.push_back(needed.clone());
                    }
                }
            }
        }
    }

    deps
}

/// Filter an execution plan to only include a target job and its transitive
/// dependencies. Returns an error if the target job doesn't exist.
pub fn filter_plan_to_job(
    plan: Vec<Vec<String>>,
    target_job: &str,
    jobs: &HashMap<String, Job>,
    kind: &str,
) -> Result<Vec<Vec<String>>, String> {
    if !jobs.contains_key(target_job) {
        return Err(job_not_found_error(target_job, jobs, kind));
    }

    let needed = collect_transitive_deps(target_job, jobs);

    Ok(plan
        .into_iter()
        .map(|batch| {
            batch
                .into_iter()
                .filter(|j| needed.contains(j))
                .collect::<Vec<_>>()
        })
        .filter(|batch| !batch.is_empty())
        .collect())
}

/// Filter a stage-ordered execution plan to only include the target job and all
/// jobs in preceding stages (implicit dependencies). This is appropriate for
/// GitLab CI/CD where stage ordering defines implicit dependencies — all jobs in
/// earlier stages must complete before later stages run.
///
/// In the target job's own stage batch, only the target job is kept; all earlier
/// stage batches are preserved in full.
pub fn filter_plan_to_job_by_stage(
    plan: Vec<Vec<String>>,
    target_job: &str,
    jobs: &HashMap<String, Job>,
    kind: &str,
) -> Result<Vec<Vec<String>>, String> {
    if !jobs.contains_key(target_job) {
        return Err(job_not_found_error(target_job, jobs, kind));
    }

    let mut result = Vec::new();
    for batch in plan {
        if batch.contains(&target_job.to_string()) {
            // Target's stage: only keep the target job itself
            result.push(vec![target_job.to_string()]);
            break;
        }
        // Earlier stage: keep all jobs (implicit dependencies)
        result.push(batch);
    }

    Ok(result)
}

fn job_not_found_error(target_job: &str, jobs: &HashMap<String, Job>, kind: &str) -> String {
    let mut available: Vec<&String> = jobs.keys().collect();
    available.sort();
    format!(
        "Job '{}' not found in {}. Available jobs: {}",
        target_job,
        kind,
        available
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_with_needs(needs: Option<Vec<&str>>) -> Job {
        Job {
            runs_on: None,
            needs: needs.map(|v| v.into_iter().map(String::from).collect()),
            container: None,
            steps: vec![],
            env: HashMap::new(),
            strategy: None,
            services: HashMap::new(),
            if_condition: None,
            outputs: None,
            permissions: None,
            uses: None,
            with: None,
            secrets: None,
        }
    }

    #[test]
    fn test_collect_transitive_deps_no_deps() {
        let mut jobs = HashMap::new();
        jobs.insert("build".to_string(), job_with_needs(None));
        jobs.insert("test".to_string(), job_with_needs(None));

        let deps = collect_transitive_deps("build", &jobs);
        assert_eq!(deps, HashSet::from(["build".to_string()]));
    }

    #[test]
    fn test_collect_transitive_deps_linear_chain() {
        let mut jobs = HashMap::new();
        jobs.insert("setup".to_string(), job_with_needs(None));
        jobs.insert("build".to_string(), job_with_needs(Some(vec!["setup"])));
        jobs.insert("deploy".to_string(), job_with_needs(Some(vec!["build"])));

        let deps = collect_transitive_deps("deploy", &jobs);
        assert_eq!(
            deps,
            HashSet::from([
                "setup".to_string(),
                "build".to_string(),
                "deploy".to_string(),
            ])
        );
    }

    #[test]
    fn test_collect_transitive_deps_diamond() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("b".to_string(), job_with_needs(Some(vec!["a"])));
        jobs.insert("c".to_string(), job_with_needs(Some(vec!["a"])));
        jobs.insert("d".to_string(), job_with_needs(Some(vec!["b", "c"])));

        let deps = collect_transitive_deps("d", &jobs);
        assert_eq!(
            deps,
            HashSet::from([
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
                "d".to_string(),
            ])
        );
    }

    #[test]
    fn test_collect_transitive_deps_partial_graph() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("b".to_string(), job_with_needs(Some(vec!["a"])));
        jobs.insert("unrelated".to_string(), job_with_needs(None));

        let deps = collect_transitive_deps("b", &jobs);
        assert_eq!(deps, HashSet::from(["a".to_string(), "b".to_string()]));
        assert!(!deps.contains("unrelated"));
    }

    #[test]
    fn test_filter_plan_to_job_not_found() {
        let jobs = HashMap::new();
        let plan = vec![vec!["a".to_string()]];

        let result = filter_plan_to_job(plan, "missing", &jobs, "workflow");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("missing"));
        assert!(err.contains("workflow"));
    }

    #[test]
    fn test_filter_plan_to_job_not_found_lists_available() {
        let mut jobs = HashMap::new();
        jobs.insert("build".to_string(), job_with_needs(None));
        jobs.insert("test".to_string(), job_with_needs(None));
        let plan = vec![vec!["build".to_string(), "test".to_string()]];

        let err = filter_plan_to_job(plan, "deploy", &jobs, "pipeline").unwrap_err();
        assert!(err.contains("build"));
        assert!(err.contains("test"));
        assert!(err.contains("pipeline"));
    }

    #[test]
    fn test_filter_plan_to_job_with_deps() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("x".to_string(), job_with_needs(None));
        jobs.insert("b".to_string(), job_with_needs(Some(vec!["a"])));
        jobs.insert("y".to_string(), job_with_needs(Some(vec!["x"])));
        jobs.insert("c".to_string(), job_with_needs(Some(vec!["b"])));

        // Plan: [a, x] -> [b, y] -> [c]
        let plan = vec![
            vec!["a".to_string(), "x".to_string()],
            vec!["b".to_string(), "y".to_string()],
            vec!["c".to_string()],
        ];

        let filtered = filter_plan_to_job(plan, "c", &jobs, "workflow").unwrap();
        // Should keep a, b, c but drop x and y
        assert_eq!(
            filtered,
            vec![
                vec!["a".to_string()],
                vec!["b".to_string()],
                vec!["c".to_string()],
            ]
        );
    }

    #[test]
    fn test_filter_plan_to_job_no_deps() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("b".to_string(), job_with_needs(None));

        let plan = vec![vec!["a".to_string(), "b".to_string()]];

        let filtered = filter_plan_to_job(plan, "a", &jobs, "workflow").unwrap();
        assert_eq!(filtered, vec![vec!["a".to_string()]]);
    }

    #[test]
    fn test_filter_plan_to_job_removes_empty_batches() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("x".to_string(), job_with_needs(None));
        jobs.insert("y".to_string(), job_with_needs(Some(vec!["x"])));

        // Plan: [a, x] -> [y]
        // Targeting "a" should produce [[a]] — batch [y] is entirely removed
        let plan = vec![
            vec!["a".to_string(), "x".to_string()],
            vec!["y".to_string()],
        ];

        let filtered = filter_plan_to_job(plan, "a", &jobs, "workflow").unwrap();
        assert_eq!(filtered, vec![vec!["a".to_string()]]);
    }

    // --- filter_plan_to_job_by_stage tests (GitLab stage-based filtering) ---

    #[test]
    fn test_filter_by_stage_not_found() {
        let jobs = HashMap::new();
        let plan = vec![vec!["a".to_string()]];

        let result = filter_plan_to_job_by_stage(plan, "missing", &jobs, "pipeline");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("missing"));
        assert!(err.contains("pipeline"));
    }

    #[test]
    fn test_filter_by_stage_target_in_first_stage() {
        let mut jobs = HashMap::new();
        jobs.insert("build".to_string(), job_with_needs(None));
        jobs.insert("lint".to_string(), job_with_needs(None));
        jobs.insert("test".to_string(), job_with_needs(None));
        jobs.insert("deploy".to_string(), job_with_needs(None));

        // Stages: [build, lint] -> [test] -> [deploy]
        let plan = vec![
            vec!["build".to_string(), "lint".to_string()],
            vec!["test".to_string()],
            vec!["deploy".to_string()],
        ];

        let filtered = filter_plan_to_job_by_stage(plan, "build", &jobs, "pipeline").unwrap();
        // Only the first stage, filtered to just "build"
        assert_eq!(filtered, vec![vec!["build".to_string()]]);
    }

    #[test]
    fn test_filter_by_stage_target_in_middle_stage() {
        let mut jobs = HashMap::new();
        jobs.insert("build".to_string(), job_with_needs(None));
        jobs.insert("lint".to_string(), job_with_needs(None));
        jobs.insert("test".to_string(), job_with_needs(None));
        jobs.insert("deploy".to_string(), job_with_needs(None));

        // Stages: [build, lint] -> [test] -> [deploy]
        let plan = vec![
            vec!["build".to_string(), "lint".to_string()],
            vec!["test".to_string()],
            vec!["deploy".to_string()],
        ];

        let filtered = filter_plan_to_job_by_stage(plan, "test", &jobs, "pipeline").unwrap();
        // Keep all of stage 1, then just "test" from stage 2, drop stage 3
        assert_eq!(
            filtered,
            vec![
                vec!["build".to_string(), "lint".to_string()],
                vec!["test".to_string()],
            ]
        );
    }

    #[test]
    fn test_filter_by_stage_target_in_last_stage() {
        let mut jobs = HashMap::new();
        jobs.insert("build".to_string(), job_with_needs(None));
        jobs.insert("test".to_string(), job_with_needs(None));
        jobs.insert("deploy".to_string(), job_with_needs(None));

        // Stages: [build] -> [test] -> [deploy]
        let plan = vec![
            vec!["build".to_string()],
            vec!["test".to_string()],
            vec!["deploy".to_string()],
        ];

        let filtered = filter_plan_to_job_by_stage(plan, "deploy", &jobs, "pipeline").unwrap();
        assert_eq!(
            filtered,
            vec![
                vec!["build".to_string()],
                vec!["test".to_string()],
                vec!["deploy".to_string()],
            ]
        );
    }

    #[test]
    fn test_filter_by_stage_filters_peers_in_target_stage() {
        let mut jobs = HashMap::new();
        jobs.insert("a".to_string(), job_with_needs(None));
        jobs.insert("b".to_string(), job_with_needs(None));
        jobs.insert("c".to_string(), job_with_needs(None));

        // All in same stage: [a, b, c]
        let plan = vec![vec!["a".to_string(), "b".to_string(), "c".to_string()]];

        let filtered = filter_plan_to_job_by_stage(plan, "b", &jobs, "pipeline").unwrap();
        // Only the target job from its stage
        assert_eq!(filtered, vec![vec!["b".to_string()]]);
    }

    #[test]
    fn test_filter_by_stage_drops_later_stages() {
        let mut jobs = HashMap::new();
        jobs.insert("compile".to_string(), job_with_needs(None));
        jobs.insert("unit_test".to_string(), job_with_needs(None));
        jobs.insert("integration_test".to_string(), job_with_needs(None));
        jobs.insert("deploy_staging".to_string(), job_with_needs(None));
        jobs.insert("deploy_prod".to_string(), job_with_needs(None));

        // Stages: [compile] -> [unit_test, integration_test] -> [deploy_staging, deploy_prod]
        let plan = vec![
            vec!["compile".to_string()],
            vec!["unit_test".to_string(), "integration_test".to_string()],
            vec!["deploy_staging".to_string(), "deploy_prod".to_string()],
        ];

        let filtered = filter_plan_to_job_by_stage(plan, "unit_test", &jobs, "pipeline").unwrap();
        assert_eq!(
            filtered,
            vec![vec!["compile".to_string()], vec!["unit_test".to_string()],]
        );
    }
}
