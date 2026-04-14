use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use wrkflw_matrix::MatrixConfig;

use super::schema::SchemaValidator;

// Custom deserializer for needs field that handles both string and array formats
fn deserialize_needs<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    let value = Option::<StringOrVec>::deserialize(deserializer)?;
    match value {
        Some(StringOrVec::String(s)) => Ok(Some(vec![s])),
        Some(StringOrVec::Vec(v)) => Ok(Some(v)),
        None => Ok(None),
    }
}

// Custom deserializer for runs-on field that handles both string and array formats
fn deserialize_runs_on<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Vec(Vec<String>),
    }

    let value = Option::<StringOrVec>::deserialize(deserializer)?;
    match value {
        Some(StringOrVec::String(s)) => Ok(Some(vec![s])),
        Some(StringOrVec::Vec(v)) => Ok(Some(v)),
        None => Ok(None),
    }
}

// Custom deserializer for container field that handles both string and object formats
fn deserialize_container<'de, D>(deserializer: D) -> Result<Option<JobContainer>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrContainer {
        String(String),
        Container(JobContainer),
    }

    let value = Option::<StringOrContainer>::deserialize(deserializer)?;
    match value {
        Some(StringOrContainer::String(image)) => Ok(Some(JobContainer {
            image,
            credentials: None,
            env: HashMap::new(),
            ports: None,
            volumes: None,
            options: None,
        })),
        Some(StringOrContainer::Container(c)) => Ok(Some(c)),
        None => Ok(None),
    }
}

#[derive(Deserialize, Clone)]
pub struct ContainerCredentials {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

impl serde::Serialize for ContainerCredentials {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ContainerCredentials", 2)?;
        state.serialize_field("username", &self.username)?;
        if self.password.is_some() {
            state.serialize_field("password", &"[REDACTED]")?;
        } else {
            state.serialize_field("password", &None::<String>)?;
        }
        state.end()
    }
}

impl std::fmt::Debug for ContainerCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerCredentials")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct JobContainer {
    pub image: String,
    #[serde(default)]
    pub credentials: Option<ContainerCredentials>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub ports: Option<Vec<String>>,
    #[serde(default)]
    pub volumes: Option<Vec<String>>,
    #[serde(default)]
    pub options: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct DefaultsRun {
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default, rename = "working-directory")]
    pub working_directory: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Defaults {
    #[serde(default)]
    pub run: Option<DefaultsRun>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkflowDefinition {
    pub name: String,
    #[serde(skip, default)] // Skip deserialization of the 'on' field directly
    pub on: Vec<String>,
    #[serde(rename = "on")] // Raw access to the 'on' field for custom handling
    pub on_raw: serde_yaml::Value,
    pub jobs: HashMap<String, Job>,
    #[serde(default)]
    pub defaults: Option<Defaults>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct Strategy {
    #[serde(default)]
    pub matrix: Option<MatrixConfig>,
    #[serde(default, rename = "fail-fast")]
    pub fail_fast: Option<bool>,
    #[serde(default, rename = "max-parallel")]
    pub max_parallel: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Job {
    #[serde(rename = "runs-on", default, deserialize_with = "deserialize_runs_on")]
    pub runs_on: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_needs")]
    pub needs: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_container")]
    pub container: Option<JobContainer>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, alias = "matrix")]
    pub strategy: Option<Strategy>,
    #[serde(default)]
    pub services: HashMap<String, Service>,
    #[serde(default, rename = "if")]
    pub if_condition: Option<String>,
    #[serde(default)]
    pub outputs: Option<HashMap<String, String>>,
    #[serde(default)]
    pub permissions: Option<HashMap<String, String>>,
    // Reusable workflow (job-level 'uses') support
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub with: Option<HashMap<String, String>>,
    #[serde(default)]
    pub secrets: Option<serde_yaml::Value>,
    #[serde(default, rename = "timeout-minutes")]
    pub timeout_minutes: Option<f64>,
    #[serde(default)]
    pub defaults: Option<Defaults>,
}

impl Job {
    /// Get the matrix config from strategy, if present
    pub fn matrix_config(&self) -> Option<&MatrixConfig> {
        self.strategy.as_ref().and_then(|s| s.matrix.as_ref())
    }

    /// Get fail-fast setting: strategy-level takes precedence, then matrix-level, default true
    pub fn fail_fast(&self) -> bool {
        self.strategy
            .as_ref()
            .and_then(|s| s.fail_fast)
            .or_else(|| self.matrix_config().and_then(|m| m.fail_fast))
            .unwrap_or(true)
    }

    /// Get max-parallel setting: strategy-level takes precedence, then matrix-level
    pub fn max_parallel(&self) -> Option<usize> {
        self.strategy
            .as_ref()
            .and_then(|s| s.max_parallel)
            .or_else(|| self.matrix_config().and_then(|m| m.max_parallel))
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Service {
    pub image: String,
    #[serde(default)]
    pub ports: Option<Vec<String>>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub volumes: Option<Vec<String>>,
    #[serde(default)]
    pub options: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Step {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub uses: Option<String>,
    #[serde(default)]
    pub run: Option<String>,
    #[serde(default)]
    pub with: Option<HashMap<String, String>>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default, rename = "continue-on-error")]
    pub continue_on_error: Option<bool>,
    #[serde(default, rename = "if")]
    pub if_condition: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "working-directory")]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default, rename = "timeout-minutes")]
    pub timeout_minutes: Option<f64>,
}

impl Step {
    /// Create a step that runs a shell command with default optional fields.
    pub fn with_run(name: impl Into<String>, run: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            uses: None,
            run: Some(run.into()),
            with: None,
            env: HashMap::new(),
            continue_on_error: None,
            if_condition: None,
            id: None,
            working_directory: None,
            shell: None,
            timeout_minutes: None,
        }
    }
}

impl WorkflowDefinition {
    pub fn resolve_action(&self, action_ref: &str) -> ActionInfo {
        // Parse GitHub action reference like "actions/checkout@v3"
        let is_docker = action_ref.starts_with("docker://");
        let is_local = action_ref.starts_with("./");

        // Docker references can contain `@sha256:digest` (e.g., `docker://alpine@sha256:abc`).
        // Don't split on `@` for Docker refs — the full string is the image reference.
        // Local paths also never have a meaningful `@version`.
        if is_docker {
            return ActionInfo {
                repository: action_ref.to_string(),
                version: String::new(),
                sub_path: None,
                is_docker: true,
                is_local: false,
            };
        }
        if is_local {
            return ActionInfo {
                repository: action_ref.to_string(),
                version: String::new(),
                sub_path: None,
                is_docker: false,
                is_local: true,
            };
        }

        // For GitHub action references, split on the first `@` to get repo and version.
        let (full_repo, version) = if let Some(at_pos) = action_ref.find('@') {
            (&action_ref[..at_pos], &action_ref[at_pos + 1..])
        } else {
            (action_ref, "main") // Default to main if no version specified
        };

        // GitHub action refs can include a sub-path: `owner/repo/path/to/action@ref`.
        // Split into the repo (`owner/repo`) and optional sub-path (`path/to/action`).
        let parts: Vec<&str> = full_repo.splitn(3, '/').collect();
        let (repo, sub_path) = if parts.len() == 3 {
            (
                format!("{}/{}", parts[0], parts[1]),
                Some(parts[2].to_string()),
            )
        } else {
            (full_repo.to_string(), None)
        };

        ActionInfo {
            repository: repo,
            version: version.to_string(),
            sub_path,
            is_docker: false,
            is_local: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActionInfo {
    /// The repository identifier (`owner/repo`), Docker image ref, or local path.
    pub repository: String,
    /// The git ref (tag, branch, or SHA) for GitHub action references.
    /// Empty for Docker refs (`docker://...`) and local paths (`./...`).
    /// Defaults to `"main"` when a GitHub action ref omits `@version`.
    pub version: String,
    /// Optional sub-path within the repository for actions like `owner/repo/path@ref`.
    /// `None` for simple `owner/repo@ref`, Docker refs, and local paths.
    pub sub_path: Option<String>,
    pub is_docker: bool,
    pub is_local: bool,
}

pub fn parse_workflow(path: &Path) -> Result<WorkflowDefinition, String> {
    // First validate against schema
    let validator = SchemaValidator::new()?;
    validator.validate_workflow(path)?;

    // If validation passes, parse the workflow
    let content =
        fs::read_to_string(path).map_err(|e| format!("Failed to read workflow file: {}", e))?;

    // Parse the YAML content
    let mut workflow: WorkflowDefinition = serde_yaml::from_str(&content)
        .map_err(|e| format!("Failed to parse workflow structure: {}", e))?;

    // Normalize the trigger events
    workflow.on = normalize_triggers(&workflow.on_raw)?;

    Ok(workflow)
}

fn normalize_triggers(on_value: &serde_yaml::Value) -> Result<Vec<String>, String> {
    let mut triggers = Vec::new();

    match on_value {
        // Simple string trigger: on: push
        serde_yaml::Value::String(event) => {
            triggers.push(event.clone());
        }
        // Array of triggers: on: [push, pull_request]
        serde_yaml::Value::Sequence(events) => {
            for event in events {
                if let Some(event_str) = event.as_str() {
                    triggers.push(event_str.to_string());
                }
            }
        }
        // Map of triggers with configuration: on: {push: {branches: [main]}}
        serde_yaml::Value::Mapping(events_map) => {
            for (event, _) in events_map {
                if let Some(event_str) = event.as_str() {
                    triggers.push(event_str.to_string());
                }
            }
        }
        _ => {
            return Err("'on' section has invalid format".to_string());
        }
    }

    Ok(triggers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolve_action_parses_version() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("actions/checkout@v4");
        assert_eq!(info.repository, "actions/checkout");
        assert_eq!(info.version, "v4");
        assert!(info.sub_path.is_none());
        assert!(!info.is_docker);
        assert!(!info.is_local);
    }

    #[test]
    fn resolve_action_defaults_version_to_main() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("owner/repo");
        assert_eq!(info.repository, "owner/repo");
        assert_eq!(info.version, "main");
        assert!(info.sub_path.is_none());
    }

    #[test]
    fn resolve_action_docker_reference() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("docker://alpine:3.18");
        assert_eq!(info.repository, "docker://alpine:3.18");
        assert_eq!(info.version, "");
        assert!(info.is_docker);
        assert!(!info.is_local);
    }

    #[test]
    fn resolve_action_local_path() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("./my-action");
        assert_eq!(info.repository, "./my-action");
        assert_eq!(info.version, "");
        assert!(!info.is_docker);
        assert!(info.is_local);
    }

    #[test]
    fn resolve_action_docker_with_digest() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        // Docker image references can use @sha256:digest — the full string is the image ref
        let info = wd.resolve_action("docker://alpine@sha256:abcdef1234567890");
        assert_eq!(info.repository, "docker://alpine@sha256:abcdef1234567890");
        assert_eq!(info.version, "");
        assert!(info.is_docker);
        assert!(!info.is_local);
    }

    #[test]
    fn resolve_action_with_sha_version() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("actions/checkout@a81bbbf8298c0fa03ea29cdc473d45769f953675");
        assert_eq!(info.repository, "actions/checkout");
        assert_eq!(info.version, "a81bbbf8298c0fa03ea29cdc473d45769f953675");
        assert!(info.sub_path.is_none());
    }

    #[test]
    fn resolve_action_with_sub_path() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("owner/repo/path/to/action@v2");
        assert_eq!(info.repository, "owner/repo");
        assert_eq!(info.version, "v2");
        assert_eq!(info.sub_path.as_deref(), Some("path/to/action"));
        assert!(!info.is_docker);
        assert!(!info.is_local);
    }

    #[test]
    fn resolve_action_with_single_sub_path() {
        let wd = WorkflowDefinition {
            name: String::new(),
            on: vec![],
            on_raw: serde_yaml::Value::Null,
            jobs: Default::default(),
            defaults: None,
            env: HashMap::new(),
        };
        let info = wd.resolve_action("github/codeql-action/init@v3");
        assert_eq!(info.repository, "github/codeql-action");
        assert_eq!(info.version, "v3");
        assert_eq!(info.sub_path.as_deref(), Some("init"));
    }

    #[test]
    fn parse_workflow_allows_null_workflow_dispatch_with_other_triggers() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: trigger-test
on:
  push:
    branches: []
    tags-ignore: []
  release:
    types: [prereleased, published]
  workflow_dispatch:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
"#;

        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path);
        assert!(
            parsed.is_ok(),
            "Expected workflow to parse successfully, got: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn parse_container_string_format() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: container-test
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    container: node:18
    steps:
      - run: echo hi
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        let container = job.container.as_ref().expect("container should be Some");
        assert_eq!(container.image, "node:18");
        assert!(container.env.is_empty());
        assert!(container.credentials.is_none());
        assert!(container.ports.is_none());
        assert!(container.volumes.is_none());
        assert!(container.options.is_none());
    }

    #[test]
    fn parse_container_object_format() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: container-test
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    container:
      image: node:18-alpine
      credentials:
        username: user
        password: pass
      env:
        NODE_ENV: production
      ports:
        - "8080:80"
      volumes:
        - /host/path:/container/path
        - /single-path
      options: "--cpus 2"
    steps:
      - run: echo hi
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        let container = job.container.as_ref().expect("container should be Some");
        assert_eq!(container.image, "node:18-alpine");
        assert_eq!(container.env.get("NODE_ENV").unwrap(), "production");
        let creds = container.credentials.as_ref().unwrap();
        assert_eq!(creds.username.as_deref(), Some("user"));
        assert_eq!(creds.password.as_deref(), Some("pass"));
        assert_eq!(
            container.ports.as_ref().unwrap(),
            &vec!["8080:80".to_string()]
        );
        let volumes = container.volumes.as_ref().unwrap();
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0], "/host/path:/container/path");
        assert_eq!(volumes[1], "/single-path");
        assert_eq!(container.options.as_deref(), Some("--cpus 2"));
    }

    #[test]
    fn parse_container_absent() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: no-container
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: echo hi
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        assert!(job.container.is_none());
    }

    #[test]
    fn parse_container_image_with_colon_in_tag() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: container-test
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    container: ghcr.io/owner/image:latest
    steps:
      - run: echo hi
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        let container = job.container.as_ref().unwrap();
        assert_eq!(container.image, "ghcr.io/owner/image:latest");
    }

    #[test]
    fn container_credentials_serialize_redacts_password() {
        let creds = ContainerCredentials {
            username: Some("user".into()),
            password: Some("super-secret".into()),
        };
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("[REDACTED]"));
        assert!(!json.contains("super-secret"));
    }

    #[test]
    fn container_credentials_serialize_null_password() {
        let creds = ContainerCredentials {
            username: Some("user".into()),
            password: None,
        };
        let json = serde_json::to_string(&creds).unwrap();
        assert!(json.contains("user"));
        assert!(!json.contains("[REDACTED]"));
    }

    #[test]
    fn parse_step_with_all_new_fields() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: step-fields-test
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - id: build-step
        name: Build
        if: github.ref == 'refs/heads/main'
        run: cargo build
        shell: bash
        working-directory: ./src
        timeout-minutes: 10.5
        continue-on-error: true
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        let step = &job.steps[0];
        assert_eq!(step.id.as_deref(), Some("build-step"));
        assert_eq!(
            step.if_condition.as_deref(),
            Some("github.ref == 'refs/heads/main'")
        );
        assert_eq!(step.shell.as_deref(), Some("bash"));
        assert_eq!(step.working_directory.as_deref(), Some("./src"));
        assert_eq!(step.timeout_minutes, Some(10.5));
        assert_eq!(step.continue_on_error, Some(true));
    }

    #[test]
    fn parse_job_timeout_minutes() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: timeout-test
on: push
jobs:
  build:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    steps:
      - run: echo hello
  no-timeout:
    runs-on: ubuntu-latest
    steps:
      - run: echo world
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let build_job = parsed.jobs.get("build").unwrap();
        assert_eq!(build_job.timeout_minutes, Some(30.0));

        let no_timeout_job = parsed.jobs.get("no-timeout").unwrap();
        assert_eq!(no_timeout_job.timeout_minutes, None);
    }

    #[test]
    fn parse_workflow_defaults() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: defaults-test
on: push
defaults:
  run:
    shell: bash
    working-directory: ./src
jobs:
  build:
    runs-on: ubuntu-latest
    defaults:
      run:
        shell: sh
        working-directory: ./app
    steps:
      - run: echo hello
  no-defaults:
    runs-on: ubuntu-latest
    steps:
      - run: echo world
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();

        // Workflow-level defaults
        let wf_defaults = parsed.defaults.as_ref().unwrap();
        let wf_run = wf_defaults.run.as_ref().unwrap();
        assert_eq!(wf_run.shell.as_deref(), Some("bash"));
        assert_eq!(wf_run.working_directory.as_deref(), Some("./src"));

        // Job-level defaults override workflow defaults
        let build_job = parsed.jobs.get("build").unwrap();
        let job_defaults = build_job.defaults.as_ref().unwrap();
        let job_run = job_defaults.run.as_ref().unwrap();
        assert_eq!(job_run.shell.as_deref(), Some("sh"));
        assert_eq!(job_run.working_directory.as_deref(), Some("./app"));

        // Job without defaults
        let no_defaults_job = parsed.jobs.get("no-defaults").unwrap();
        assert!(no_defaults_job.defaults.is_none());
    }

    #[test]
    fn parse_strategy_matrix() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: matrix-test
on: push
jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      max-parallel: 2
      matrix:
        os: [ubuntu-latest, windows-latest]
        node: [16, 18]
    steps:
      - run: echo hi
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();
        let job = parsed.jobs.get("test").unwrap();
        assert!(job.matrix_config().is_some());
        let matrix = job.matrix_config().unwrap();
        assert!(matrix.parameters.contains_key("os"));
        assert!(matrix.parameters.contains_key("node"));
        assert!(!job.fail_fast());
        assert_eq!(job.max_parallel(), Some(2));
    }

    #[test]
    fn parse_continue_on_error_workflow() {
        let temp_dir = tempdir().unwrap();
        let workflow_path = temp_dir.path().join("workflow.yml");

        let content = r#"
name: Continue On Error Test
on: [push]
jobs:
  test-continue:
    runs-on: ubuntu-latest
    steps:
      - name: Failing step with continue
        run: exit 1
        continue-on-error: true
      - name: Should still run
        run: echo "I ran after failure"
  test-if-skip:
    runs-on: ubuntu-latest
    steps:
      - name: Always runs
        run: echo "hello"
      - name: Skipped step
        if: "false"
        run: echo "should not run"
      - name: Runs after skip
        run: echo "after skip"
"#;
        fs::write(&workflow_path, content).unwrap();

        let parsed = parse_workflow(&workflow_path).unwrap();

        // Verify continue-on-error parsing
        let job = parsed.jobs.get("test-continue").unwrap();
        assert_eq!(job.steps.len(), 2);
        assert_eq!(job.steps[0].continue_on_error, Some(true));
        assert_eq!(job.steps[1].continue_on_error, None);

        // Verify step-level if condition parsing
        let job2 = parsed.jobs.get("test-if-skip").unwrap();
        assert_eq!(job2.steps.len(), 3);
        assert_eq!(job2.steps[0].if_condition, None);
        assert_eq!(job2.steps[1].if_condition.as_deref(), Some("false"));
        assert_eq!(job2.steps[2].if_condition, None);
    }
}
