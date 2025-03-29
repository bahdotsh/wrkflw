use crate::parser::workflow::WorkflowDefinition;
use std::collections::{HashMap, HashSet};

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
                dependencies
                    .get_mut(job_name)
                    .unwrap()
                    .insert(needed_job.clone());
                dependents
                    .get_mut(needed_job)
                    .unwrap()
                    .insert(job_name.clone());
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
            for dependent in dependents.get(job).unwrap().clone() {
                // Remove the current job from its dependencies
                dependencies.get_mut(&dependent).unwrap().remove(job);

                // If no more dependencies, add to next level
                if dependencies.get(&dependent).unwrap().is_empty() {
                    next_no_dependencies.insert(dependent);
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
        return Err("Circular dependency detected in workflow jobs".to_string());
    }

    Ok(result)
}
