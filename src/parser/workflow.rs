use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize)]
pub struct WorkflowDefinition {
    pub name: String,
    #[serde(skip, default)] // Skip deserialization of the 'on' field directly
    pub on: Vec<String>,
    #[serde(rename = "on")] // Raw access to the 'on' field for custom handling
    pub on_raw: serde_yaml::Value,
    pub jobs: HashMap<String, Job>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Job {
    #[serde(rename = "runs-on")]
    pub runs_on: String,
    #[serde(default)]
    pub needs: Option<Vec<String>>,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub env: HashMap<String, String>,
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
}

impl WorkflowDefinition {
    pub fn resolve_action(&self, action_ref: &str) -> ActionInfo {
        // Parse GitHub action reference like "actions/checkout@v3"
        let parts: Vec<&str> = action_ref.split('@').collect();

        let (repo, _) = if parts.len() > 1 {
            (parts[0], parts[1])
        } else {
            (parts[0], "main") // Default to main if no version specified
        };

        ActionInfo {
            repository: repo.to_string(),
            is_docker: repo.starts_with("docker://"),
            is_local: repo.starts_with("./"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActionInfo {
    pub repository: String,
    pub is_docker: bool,
    pub is_local: bool,
}

pub fn parse_workflow(path: &Path) -> Result<WorkflowDefinition, String> {
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
