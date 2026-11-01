use crate::gitlab_config;
use crate::schema::{SchemaType, SchemaValidator};
use crate::workflow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;
use wrkflw_matrix::MatrixConfig;
use wrkflw_models::gitlab::{Image, Job, Parallel, Pipeline};
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

    check_parallel(&pipeline).map_err(GitlabParserError::InvalidStructure)?;

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

/// When a job runs relative to the outcome of the stages before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobWhen {
    OnSuccess,
    OnFailure,
    Always,
    /// Excluded from the pipeline. Includes `manual`, since nobody can press
    /// play on a local run.
    Skip,
}

impl JobWhen {
    fn parse(when: Option<&str>) -> Self {
        match when {
            None | Some("on_success") | Some("delayed") => JobWhen::OnSuccess,
            Some("on_failure") => JobWhen::OnFailure,
            Some("always") => JobWhen::Always,
            Some(_) => JobWhen::Skip,
        }
    }
}

/// Decide when a job runs. The first matching rule wins and a job whose rules
/// all miss is excluded, as in GitLab. `variables` are the job's CI variables.
pub fn job_when(job: &Job, variables: &HashMap<String, String>) -> JobWhen {
    let job_level = job.when.as_deref();
    let Some(rules) = &job.rules else {
        return JobWhen::parse(job_level);
    };
    rules
        .iter()
        .find(|rule| {
            rule.if_
                .as_deref()
                .is_none_or(|condition| rule_condition_matches(condition, variables))
        })
        .map_or(JobWhen::Skip, |rule| {
            JobWhen::parse(rule.when.as_deref().or(job_level))
        })
}

/// Evaluate a `rules:if` condition: `||` of `&&` of comparisons, without
/// parentheses. Anything else matches, with a warning, because running an extra
/// job is easier to notice than silently dropping one.
fn rule_condition_matches(condition: &str, variables: &HashMap<String, String>) -> bool {
    split_outside_quotes(condition, "||").iter().any(|any_of| {
        split_outside_quotes(any_of, "&&").iter().all(|term| {
            rule_term_matches(term.trim(), variables).unwrap_or_else(|| {
                wrkflw_logging::warning(&format!(
                    "Unsupported rules:if condition '{}': treating it as a match",
                    term.trim()
                ));
                true
            })
        })
    })
}

fn rule_term_matches(term: &str, variables: &HashMap<String, String>) -> Option<bool> {
    if term.contains(['(', ')']) && !term.contains("=~") && !term.contains("!~") {
        return None;
    }
    for (operator, negated) in [("==", false), ("!=", true)] {
        if let Some((left, right)) = term.split_once(operator) {
            let equal = rule_operand(left, variables)? == rule_operand(right, variables)?;
            return Some(equal != negated);
        }
    }
    for (operator, negated) in [("=~", false), ("!~", true)] {
        if let Some((left, right)) = term.split_once(operator) {
            let value = rule_operand(left, variables)?.unwrap_or_default();
            let pattern = right.trim().strip_prefix('/')?;
            let (pattern, flags) = pattern.rsplit_once('/')?;
            let insensitive = if flags.contains('i') { "(?i)" } else { "" };
            let regex = regex::Regex::new(&format!("{}{}", insensitive, pattern)).ok()?;
            return Some(regex.is_match(&value) != negated);
        }
    }
    let value = rule_operand(term, variables)?;
    Some(value.is_some_and(|value| !value.is_empty()))
}

/// A variable, quoted string or `null`. `None` means it isn't one of those.
fn rule_operand(operand: &str, variables: &HashMap<String, String>) -> Option<Option<String>> {
    let operand = operand.trim();
    if operand == "null" {
        return Some(None);
    }
    if let Some(name) = operand.strip_prefix('$') {
        let name = name.trim_start_matches('{').trim_end_matches('}');
        return Some(variables.get(name).cloned());
    }
    ['"', '\'']
        .iter()
        .find_map(|quote| operand.strip_prefix(*quote)?.strip_suffix(*quote))
        .map(|text| Some(text.to_string()))
}

fn split_outside_quotes<'a>(text: &'a str, separator: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quote = None;
    for (index, ch) in text.char_indices() {
        match quote {
            Some(open) if ch == open => quote = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if text[index..].starts_with(separator) => {
                parts.push(&text[start..index]);
                start = index + separator.len();
            }
            None => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A string-or-list key from the pipeline's `default:` section.
fn default_list(pipeline: &Pipeline, key: &str) -> Option<Vec<String>> {
    match pipeline.default.as_ref()?.get(key)? {
        serde_yaml::Value::String(line) => Some(vec![line.clone()]),
        serde_yaml::Value::Sequence(lines) => Some(
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

/// The image a job runs in: its own, then `default:`, then the top-level one.
fn job_image(pipeline: &Pipeline, job: &Job) -> Option<String> {
    let name = |image: &Image| match image {
        Image::Simple(name) | Image::Detailed { name, .. } => name.clone(),
    };
    job.image
        .as_ref()
        .map(name)
        .or_else(|| {
            let image = pipeline.default.as_ref()?.get("image")?;
            serde_yaml::from_value::<Image>(image.clone())
                .ok()
                .map(|image| name(&image))
        })
        .or_else(|| pipeline.image.as_ref().map(name))
}

/// The most instances GitLab creates from one job's `parallel`
const MAX_PARALLEL_INSTANCES: usize = 200;

/// Reject `parallel:matrix` mistakes the schema can't express. The schema
/// already bounds `parallel: N` and the number of matrix entries, but not how
/// many instances those entries multiply out to, or what's inside them.
fn check_parallel(pipeline: &Pipeline) -> Result<(), String> {
    let mut issues = Vec::new();
    for (job_name, job) in &pipeline.jobs {
        if let Some(Parallel::Matrix { matrix }) = &job.parallel {
            issues.extend(
                matrix_issues(matrix)
                    .into_iter()
                    .map(|issue| format!("Job '{}': parallel:matrix {}", job_name, issue)),
            );
        }
    }
    if issues.is_empty() {
        return Ok(());
    }
    issues.sort();
    Err(issues.join("; "))
}

fn matrix_issues(matrix: &[HashMap<String, serde_yaml::Value>]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut total = 0usize;
    for entry in matrix {
        if entry.is_empty() {
            issues.push("has an entry with no variables".to_string());
        }
        let mut instances = 1usize;
        for (name, value) in entry {
            if !is_variable_name(name) {
                issues.push(format!("variable '{}' is not a valid name", name));
            }
            let values = match value {
                serde_yaml::Value::Sequence(values) => values.len(),
                _ => 1,
            };
            if values == 0 {
                issues.push(format!("variable '{}' has no values", name));
            }
            instances = instances.saturating_mul(values);
        }
        total = total.saturating_add(instances);
    }
    if total > MAX_PARALLEL_INSTANCES {
        issues.push(format!(
            "expands to more than {} instances",
            MAX_PARALLEL_INSTANCES
        ));
    }
    issues
}

fn is_variable_name(name: &str) -> bool {
    !name.starts_with(|c: char| c.is_ascii_digit())
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Run a job once per `parallel` instance through a matrix, exposing each
/// instance's variables as environment variables, as GitLab does. Every
/// instance also gets `CI_NODE_INDEX` and `CI_NODE_TOTAL`.
fn apply_parallel(job: &mut workflow::Job, parallel: &Parallel) {
    let mut combinations: Vec<HashMap<String, serde_yaml::Value>> = match parallel {
        Parallel::Count(total) => vec![HashMap::new(); *total],
        Parallel::Matrix { matrix } => matrix.iter().flat_map(expand_parallel_entry).collect(),
    };
    for (index, combination) in combinations.iter_mut().enumerate() {
        combination.insert(
            "CI_NODE_INDEX".to_string(),
            serde_yaml::Value::from(index as u64 + 1),
        );
    }
    job.env
        .insert("CI_NODE_TOTAL".to_string(), combinations.len().to_string());

    let variables: HashSet<&String> = combinations.iter().flat_map(HashMap::keys).collect();
    for variable in variables {
        job.env
            .insert(variable.clone(), format!("${{{{ matrix.{} }}}}", variable));
    }
    // GitLab runs every instance even when one fails.
    job.strategy = Some(workflow::Strategy {
        matrix: Some(MatrixConfig::from_combinations(combinations)),
        fail_fast: Some(false),
        max_parallel: None,
    });
}

/// The Cartesian product of one `parallel:matrix` entry's variables. Entries
/// are independent: GitLab creates each one's instances, even if they overlap.
/// An entry with nothing to expand has no instances; `check_parallel` reports it.
fn expand_parallel_entry(
    entry: &HashMap<String, serde_yaml::Value>,
) -> Vec<HashMap<String, serde_yaml::Value>> {
    // Sorted so that each instance's CI_NODE_INDEX is stable between runs
    let mut variables: Vec<_> = entry.iter().collect();
    variables.sort_by_key(|(name, _)| name.as_str());
    let matrix = MatrixConfig {
        parameters: variables
            .into_iter()
            .map(|(name, values)| (name.clone(), values.clone()))
            .collect(),
        ..MatrixConfig::default()
    };
    wrkflw_matrix::expand_matrix(&matrix)
        .map(|combinations| combinations.into_iter().map(|c| c.values).collect())
        .unwrap_or_default()
}

/// Convert a GitLab CI/CD pipeline to a format compatible with the workflow executor
/// A `continue-on-error` value that holds whatever the job or step did.
const CONTINUE_ON_ERROR_ALWAYS: &str = "true";

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

        let image = job_image(pipeline, gitlab_job);

        // Create a new job
        let mut job = workflow::Job {
            runs_on: Some(vec!["ubuntu-latest".to_string()]), // Default runner
            needs: None,
            container: image.clone().map(|image| workflow::JobContainer {
                image,
                credentials: None,
                env: HashMap::new(),
                ports: None,
                volumes: None,
                options: None,
            }),
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
            // Pipeline images often have no bash, which GitLab falls back from.
            defaults: image.is_some().then(|| workflow::Defaults {
                run: Some(workflow::DefaultsRun {
                    shell: Some("sh".to_string()),
                    working_directory: None,
                }),
            }),
            continue_on_error: matches!(
                gitlab_job.allow_failure,
                Some(wrkflw_models::gitlab::AllowFailure::Bool(true))
            )
            .then(|| CONTINUE_ON_ERROR_ALWAYS.to_string()),
        };

        // Add job-specific environment variables
        if let Some(variables) = &gitlab_job.variables {
            job.env.extend(variables.clone());
        }

        if let Some(parallel) = &gitlab_job.parallel {
            apply_parallel(&mut job, parallel);
        }

        // Add global variables if they exist
        if let Some(variables) = &pipeline.variables {
            // Only add if not already defined at job level
            for (key, value) in variables {
                job.env.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }

        // Convert before_script to steps if it exists
        let before_script = gitlab_job
            .before_script
            .clone()
            .or_else(|| default_list(pipeline, "before_script"))
            .or_else(|| pipeline.before_script.clone());
        if let Some(before_script) = &before_script {
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
        let after_script = gitlab_job
            .after_script
            .clone()
            .or_else(|| default_list(pipeline, "after_script"))
            .or_else(|| pipeline.after_script.clone());
        if let Some(after_script) = &after_script {
            for (i, cmd) in after_script.iter().enumerate() {
                let mut step =
                    workflow::Step::with_run(format!("After script {}", i + 1), cmd.clone());
                step.continue_on_error = Some(CONTINUE_ON_ERROR_ALWAYS.to_string()); // After script should continue even if previous steps fail
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
    fn allow_failure_maps_to_continue_on_error_only_when_unconditional() {
        let file = NamedTempFile::new().unwrap();
        fs::write(
            file.path(),
            r#"
stages:
  - test

allowed:
  stage: test
  script:
    - make test
  allow_failure: true

disallowed:
  stage: test
  script:
    - make test
  allow_failure: false

by_exit_code:
  stage: test
  script:
    - make test
  allow_failure:
    exit_codes: [47]

unset:
  stage: test
  script:
    - make test
"#,
        )
        .unwrap();

        let pipeline = parse_pipeline(file.path()).unwrap();
        let workflow = convert_to_workflow_format(&pipeline);
        let value = |job: &str| workflow.jobs[job].continue_on_error.as_deref();

        assert_eq!(value("allowed"), Some("true"));
        assert_eq!(value("disallowed"), None);
        assert_eq!(value("by_exit_code"), None);
        assert_eq!(value("unset"), None);
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

    fn convert(yaml: &str) -> workflow::WorkflowDefinition {
        convert_to_workflow_format(&serde_yaml::from_str(yaml).unwrap())
    }

    fn variables(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn when_of(job_yaml: &str, vars: &[(&str, &str)]) -> JobWhen {
        let job: Job = serde_yaml::from_str(job_yaml).unwrap();
        job_when(&job, &variables(vars))
    }

    #[test]
    fn job_when_reads_the_job_level_key() {
        assert_eq!(when_of("script: [a]", &[]), JobWhen::OnSuccess);
        assert_eq!(when_of("script: [a]\nwhen: manual", &[]), JobWhen::Skip);
        assert_eq!(when_of("script: [a]\nwhen: never", &[]), JobWhen::Skip);
        assert_eq!(when_of("script: [a]\nwhen: always", &[]), JobWhen::Always);
        assert_eq!(
            when_of("script: [a]\nwhen: on_failure", &[]),
            JobWhen::OnFailure
        );
    }

    #[test]
    fn first_matching_rule_decides() {
        let job = r#"
script: [a]
rules:
  - if: '$V == "top"'
    when: never
  - when: always
"#;
        assert_eq!(when_of(job, &[("V", "top")]), JobWhen::Skip);
        assert_eq!(when_of(job, &[("V", "other")]), JobWhen::Always);
    }

    #[test]
    fn a_job_whose_rules_all_miss_is_excluded() {
        let job = "script: [a]\nwhen: always\nrules:\n  - if: '$NOPE == \"1\"'\n";
        assert_eq!(when_of(job, &[]), JobWhen::Skip);
        assert_eq!(when_of(job, &[("NOPE", "1")]), JobWhen::Always);
    }

    #[test]
    fn rule_conditions_cover_the_common_operators() {
        let vars = variables(&[("A", "main"), ("B", "x"), ("EMPTY", "")]);
        for (condition, expected) in [
            ("$A", true),
            ("$MISSING", false),
            ("$EMPTY", false),
            ("$A == 'main'", true),
            ("$A != \"main\"", false),
            ("$A == $B", false),
            ("$MISSING == null", true),
            ("$A != null", true),
            ("$A =~ /^ma/", true),
            ("$A =~ /^MA/i", true),
            ("$A !~ /^ma/", false),
            ("$A =~ /(main|dev)/", true),
            ("$A == 'main' && $B == 'x'", true),
            ("$A == 'nope' || $B == 'x'", true),
            ("$A == 'nope' && $B == 'x'", false),
            ("$A == 'a||b' || $B == 'nope'", false),
            ("${A} == 'main'", true),
            // Parentheses aren't supported, so these match with a warning.
            ("($A == 'nope')", true),
        ] {
            assert_eq!(
                rule_condition_matches(condition, &vars),
                expected,
                "{}",
                condition
            );
        }
    }

    #[test]
    fn image_and_scripts_fall_back_to_defaults() {
        let workflow = convert(
            r#"
image: top:1
default:
  image: dflt:1
  before_script: [echo before]
  after_script: echo after
inherit:
  script: [echo]
own:
  image: own:1
  before_script: [echo mine]
  script: [echo]
"#,
        );
        let inherit = &workflow.jobs["inherit"];
        assert_eq!(inherit.container.as_ref().unwrap().image, "dflt:1");
        let steps: Vec<_> = inherit
            .steps
            .iter()
            .filter_map(|s| s.run.as_deref())
            .collect();
        assert_eq!(steps, ["echo before", "echo", "echo after"]);

        let own = &workflow.jobs["own"];
        assert_eq!(own.container.as_ref().unwrap().image, "own:1");
        assert_eq!(own.steps[0].run.as_deref(), Some("echo mine"));
        assert_eq!(
            own.defaults
                .as_ref()
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .shell
                .as_deref(),
            Some("sh")
        );
    }

    #[test]
    fn jobs_without_any_image_keep_the_default_runner() {
        let workflow = convert("job:\n  script: [echo]\n");
        assert!(workflow.jobs["job"].container.is_none());
        assert!(workflow.jobs["job"].defaults.is_none());
    }

    /// Each instance's variables, sorted so that tests can compare them
    fn instances(workflow: &workflow::WorkflowDefinition) -> Vec<Vec<(String, String)>> {
        let matrix = workflow.jobs["job"].matrix_config().unwrap();
        let mut instances: Vec<Vec<(String, String)>> = wrkflw_matrix::expand_matrix(matrix)
            .unwrap()
            .into_iter()
            .map(|combination| {
                let mut values: Vec<_> = combination
                    .values
                    .into_iter()
                    .filter(|(name, _)| name != "CI_NODE_INDEX")
                    .map(|(name, value)| {
                        let value = serde_yaml::to_string(&value).unwrap();
                        (name, value.trim().to_string())
                    })
                    .collect();
                values.sort();
                values
            })
            .collect();
        instances.sort();
        instances
    }

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn parse(yaml: &str) -> Result<Pipeline, GitlabParserError> {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), yaml).unwrap();
        parse_pipeline(file.path())
    }

    fn parse_matrix(matrix: &str) -> Result<Pipeline, GitlabParserError> {
        parse(&format!(
            "job:\n  script: [echo]\n  parallel:\n    matrix:\n{}",
            matrix
        ))
    }

    #[test]
    fn parallel_matrix_expands_each_entry() {
        let workflow = convert(
            r#"
stages: [test]
job:
  stage: test
  script: [echo]
  parallel:
    matrix:
      - A: [x, y]
        B: "1"
      - C: z
"#,
        );
        let job = &workflow.jobs["job"];
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("A", "x"), ("B", "'1'")]),
                pairs(&[("A", "y"), ("B", "'1'")]),
                pairs(&[("C", "z")]),
            ]
        );
        assert_eq!(job.env["A"], "${{ matrix.A }}");
        assert_eq!(job.env["C"], "${{ matrix.C }}");
        assert!(!job.fail_fast());
    }

    #[test]
    fn parallel_matrix_keeps_overlapping_entries_apart() {
        let workflow = convert(
            r#"
job:
  script: [echo]
  parallel:
    matrix:
      - OS: [linux, windows]
      - OS: linux
        ARCH: arm64
"#,
        );
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("ARCH", "arm64"), ("OS", "linux")]),
                pairs(&[("OS", "linux")]),
                pairs(&[("OS", "windows")]),
            ]
        );
    }

    #[test]
    fn parallel_matrix_keeps_an_entry_that_another_extends() {
        let workflow = convert(
            r#"
job:
  script: [echo]
  parallel:
    matrix:
      - A: x
      - A: x
        B: [1, 2]
"#,
        );
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("A", "x")]),
                pairs(&[("A", "x"), ("B", "1")]),
                pairs(&[("A", "x"), ("B", "2")]),
            ]
        );
    }

    #[test]
    fn parallel_matrix_keeps_number_values() {
        let workflow =
            convert("job:\n  script: [echo]\n  parallel:\n    matrix:\n      - RETRIES: [1, 2]\n");
        assert_eq!(
            instances(&workflow),
            vec![pairs(&[("RETRIES", "1")]), pairs(&[("RETRIES", "2")])]
        );
    }

    #[test]
    fn parallel_matrix_overrides_a_job_variable_of_the_same_name() {
        let workflow = convert(
            r#"
variables:
  GLOBAL: global
  OS: global
job:
  variables:
    OS: job
  script: [echo]
  parallel:
    matrix:
      - OS: [linux]
"#,
        );
        let job = &workflow.jobs["job"];
        assert_eq!(job.env["OS"], "${{ matrix.OS }}");
        assert_eq!(job.env["GLOBAL"], "global");
    }

    fn node_indexes(workflow: &workflow::WorkflowDefinition) -> Vec<u64> {
        let matrix = workflow.jobs["job"].matrix_config().unwrap();
        let mut indexes: Vec<u64> = wrkflw_matrix::expand_matrix(matrix)
            .unwrap()
            .iter()
            .map(|combination| combination.values["CI_NODE_INDEX"].as_u64().unwrap())
            .collect();
        indexes.sort();
        indexes
    }

    #[test]
    fn parallel_count_sets_node_variables() {
        let workflow = convert("job:\n  script: [echo]\n  parallel: 3\n");
        let job = &workflow.jobs["job"];
        assert_eq!(node_indexes(&workflow), vec![1, 2, 3]);
        assert_eq!(job.env["CI_NODE_TOTAL"], "3");
        assert_eq!(job.env["CI_NODE_INDEX"], "${{ matrix.CI_NODE_INDEX }}");
    }

    #[test]
    fn parallel_matrix_sets_node_variables() {
        let workflow = convert(
            "job:\n  script: [echo]\n  parallel:\n    matrix:\n      - A: [x, y]\n      - B: z\n",
        );
        let job = &workflow.jobs["job"];
        assert_eq!(node_indexes(&workflow), vec![1, 2, 3]);
        assert_eq!(job.env["CI_NODE_TOTAL"], "3");
    }

    #[test]
    fn parallel_count_is_bounded_when_parsing() {
        for count in ["1", "200"] {
            let yaml = format!("job:\n  script: [echo]\n  parallel: {}\n", count);
            assert!(parse(&yaml).is_ok(), "parallel: {}", count);
        }
        for count in ["0", "201", "\"3\""] {
            let yaml = format!("job:\n  script: [echo]\n  parallel: {}\n", count);
            assert!(
                matches!(
                    parse(&yaml),
                    Err(GitlabParserError::SchemaValidationError(_))
                ),
                "parallel: {}",
                count
            );
        }
    }

    #[test]
    fn parallel_matrix_is_limited_to_200_instances() {
        let values = |count: usize| (0..count).map(|v| format!("v{}", v)).collect::<Vec<_>>();
        let entry = |count: usize| format!("      - A: [{}]\n", values(count).join(", "));

        assert!(parse_matrix(&entry(200)).is_ok());
        let error = parse_matrix(&format!("{}{}", entry(200), entry(1)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Job 'job'"), "{}", error);
        assert!(error.contains("more than 200"), "{}", error);
    }

    #[test]
    fn parallel_matrix_rejects_entries_that_run_nothing() {
        let error = parse_matrix("      - {}\n").unwrap_err().to_string();
        assert!(error.contains("no variables"), "{}", error);
        let error = parse_matrix("      - A: []\n").unwrap_err().to_string();
        assert!(error.contains("'A' has no values"), "{}", error);
    }

    #[test]
    fn parallel_matrix_rejects_invalid_variable_names() {
        let error = parse_matrix("      - \"NOT VALID\": x\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("'NOT VALID'"), "{}", error);
    }
}
