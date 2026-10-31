use crate::gitlab_config;
use crate::schema::{SchemaType, SchemaValidator};
use crate::workflow;
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use wrkflw_models::gitlab::Pipeline;
use wrkflw_models::ValidationResult;

#[derive(Error, Debug)]
pub enum GitlabParserError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Invalid pipeline structure: {0}")]
    InvalidStructure(String),

    #[error("Schema validation error: {0}")]
    SchemaValidationError(String),
}

/// Parse a GitLab CI/CD pipeline file
///
/// Local includes, `spec:inputs`, `extends`, `!reference` tags and hidden
/// (`.`-prefixed) templates are resolved first, so the returned pipeline
/// contains only the jobs GitLab would actually create.
pub fn parse_pipeline(pipeline_path: &Path) -> Result<Pipeline, GitlabParserError> {
    // Assemble the full configuration from the pipeline file and its includes
    let config =
        gitlab_config::load_pipeline(pipeline_path).map_err(GitlabParserError::InvalidStructure)?;

    // Validate against schema
    let validator = SchemaValidator::new().map_err(GitlabParserError::SchemaValidationError)?;
    let config_json = serde_json::to_value(&config)
        .map_err(|e| GitlabParserError::InvalidStructure(e.to_string()))?;

    validator
        .validate_json(&config_json, SchemaType::GitLab)
        .map_err(GitlabParserError::SchemaValidationError)?;

    // Parse the resolved configuration
    let pipeline: Pipeline = serde_yaml::from_value(config)?;

    // Return the parsed pipeline
    Ok(pipeline)
}

/// Validate the basic structure of a GitLab CI/CD pipeline
pub fn validate_pipeline_structure(pipeline: &Pipeline) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Check for at least one job
    if pipeline.jobs.is_empty() {
        result.add_issue("Pipeline must contain at least one job".to_string());
    }

    // Check for script in jobs
    for (job_name, job) in &pipeline.jobs {
        // Skip template jobs
        if let Some(true) = job.template {
            continue;
        }

        // Check for script or extends
        if job.script.is_none() && job.extends.is_none() && job.trigger.is_none() {
            result.add_issue(format!(
                "Job '{}' must have a script or trigger section or extend another job",
                job_name
            ));
        }
    }

    // Check that referenced stages are defined
    if let Some(stages) = &pipeline.stages {
        for (job_name, job) in &pipeline.jobs {
            if let Some(stage) = &job.stage {
                if !stages.contains(stage) {
                    result.add_issue(format!(
                        "Job '{}' references undefined stage '{}'",
                        job_name, stage
                    ));
                }
            }
        }
    }

    // Check that job dependencies exist
    for (job_name, job) in &pipeline.jobs {
        if let Some(dependencies) = &job.dependencies {
            for dependency in dependencies {
                if !pipeline.jobs.contains_key(dependency) {
                    result.add_issue(format!(
                        "Job '{}' depends on undefined job '{}'",
                        job_name, dependency
                    ));
                }
            }
        }
    }

    // Check that job extensions exist
    for (job_name, job) in &pipeline.jobs {
        if let Some(extends) = &job.extends {
            for extend in extends {
                if !pipeline.jobs.contains_key(extend) {
                    result.add_issue(format!(
                        "Job '{}' extends undefined job '{}'",
                        job_name, extend
                    ));
                }
            }
        }
    }

    result
}

/// Convert a GitLab CI/CD pipeline to a format compatible with the workflow executor
pub fn convert_to_workflow_format(pipeline: &Pipeline) -> workflow::WorkflowDefinition {
    // Create a new workflow with required fields
    let mut workflow = workflow::WorkflowDefinition {
        name: "Converted GitLab CI Pipeline".to_string(),
        on: vec!["push".to_string()], // Default trigger
        on_raw: serde_yaml::Value::String("push".to_string()),
        jobs: HashMap::new(),
        defaults: None,
        env: HashMap::new(),
    };

    // Convert each GitLab job to a GitHub Actions job
    for (job_name, gitlab_job) in &pipeline.jobs {
        // Skip template jobs
        if let Some(true) = gitlab_job.template {
            continue;
        }

        // Create a new job
        let mut job = workflow::Job {
            runs_on: Some(vec!["ubuntu-latest".to_string()]), // Default runner
            needs: None,
            container: None,
            steps: Vec::new(),
            env: HashMap::new(),
            strategy: None,
            services: HashMap::new(),
            if_condition: None,
            outputs: None,
            permissions: None,
            uses: None,
            with: None,
            secrets: None,
            timeout_minutes: None,
            defaults: None,
            continue_on_error: matches!(
                gitlab_job.allow_failure,
                Some(wrkflw_models::gitlab::AllowFailure::Bool(true))
            )
            .then(|| "true".to_string()),
        };

        // Add job-specific environment variables
        if let Some(variables) = &gitlab_job.variables {
            job.env.extend(variables.clone());
        }

        // Add global variables if they exist
        if let Some(variables) = &pipeline.variables {
            // Only add if not already defined at job level
            for (key, value) in variables {
                job.env.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }

        // Convert before_script to steps if it exists
        if let Some(before_script) = &gitlab_job.before_script {
            for (i, cmd) in before_script.iter().enumerate() {
                job.steps.push(workflow::Step::with_run(
                    format!("Before script {}", i + 1),
                    cmd.clone(),
                ));
            }
        }

        // Convert main script to steps
        if let Some(script) = &gitlab_job.script {
            for (i, cmd) in script.iter().enumerate() {
                job.steps.push(workflow::Step::with_run(
                    format!("Run script line {}", i + 1),
                    cmd.clone(),
                ));
            }
        }

        // Convert after_script to steps if it exists
        if let Some(after_script) = &gitlab_job.after_script {
            for (i, cmd) in after_script.iter().enumerate() {
                let mut step =
                    workflow::Step::with_run(format!("After script {}", i + 1), cmd.clone());
                step.continue_on_error = Some("true".to_string()); // After script should continue even if previous steps fail
                job.steps.push(step);
            }
        }

        // Add services if they exist
        if let Some(services) = &gitlab_job.services {
            for (i, service) in services.iter().enumerate() {
                let service_name = format!("service-{}", i);
                let service_image = match service {
                    wrkflw_models::gitlab::Service::Simple(name) => name.clone(),
                    wrkflw_models::gitlab::Service::Detailed { name, .. } => name.clone(),
                };

                let service = workflow::Service {
                    image: service_image,
                    ports: None,
                    env: HashMap::new(),
                    volumes: None,
                    options: None,
                };

                job.services.insert(service_name, service);
            }
        }

        // Add the job to the workflow
        workflow.jobs.insert(job_name.clone(), job);
    }

    workflow
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    // use std::path::PathBuf; // unused
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_pipeline() {
        // Create a temporary file with a simple GitLab CI/CD pipeline
        let file = NamedTempFile::new().unwrap();
        let content = r#"
stages:
  - build
  - test

build_job:
  stage: build
  script:
    - echo "Building..."
    - make build

test_job:
  stage: test
  script:
    - echo "Testing..."
    - make test
"#;
        fs::write(&file, content).unwrap();

        // Parse the pipeline
        let pipeline = parse_pipeline(file.path()).unwrap();

        // Validate basic structure
        assert_eq!(pipeline.stages.as_ref().unwrap().len(), 2);
        assert_eq!(pipeline.jobs.len(), 2);

        // Check job contents
        let build_job = pipeline.jobs.get("build_job").unwrap();
        assert_eq!(build_job.stage.as_ref().unwrap(), "build");
        assert_eq!(build_job.script.as_ref().unwrap().len(), 2);

        let test_job = pipeline.jobs.get("test_job").unwrap();
        assert_eq!(test_job.stage.as_ref().unwrap(), "test");
        assert_eq!(test_job.script.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_parse_pipeline_with_templates_and_includes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".gitlab")).unwrap();
        fs::write(
            dir.path().join(".gitlab/templates.yml"),
            r#"
.build_template:
  stage: build
  script:
    - make build
  allow_failure:
    exit_codes: [47]
"#,
        )
        .unwrap();
        let path = dir.path().join(".gitlab-ci.yml");
        fs::write(
            &path,
            r#"
include:
  - local: .gitlab/templates.yml

stages:
  - build
  - deploy

build_job:
  extends: .build_template

downstream:
  stage: deploy
  trigger:
    project: group/downstream
"#,
        )
        .unwrap();

        let pipeline = parse_pipeline(&path).unwrap();

        let mut names: Vec<_> = pipeline.jobs.keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["build_job", "downstream"]);

        let build_job = &pipeline.jobs["build_job"];
        assert_eq!(build_job.stage.as_deref(), Some("build"));
        assert_eq!(build_job.script.as_ref().unwrap(), &["make build"]);
        assert!(build_job.extends.is_none());

        let result = validate_pipeline_structure(&pipeline);
        assert!(result.is_valid, "{:?}", result.issues);
    }
}
