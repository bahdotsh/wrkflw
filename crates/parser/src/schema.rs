use jsonschema::JSONSchema;
use serde_json::Value;
use std::fs;
use std::path::Path;

const GITHUB_WORKFLOW_SCHEMA: &str = include_str!("../../../schemas/github-workflow.json");
const GITLAB_CI_SCHEMA: &str = include_str!("../../../schemas/gitlab-ci.json");

#[derive(Debug, Clone, Copy)]
pub enum SchemaType {
    GitHub,
    GitLab,
}

pub struct SchemaValidator {
    github_schema: JSONSchema,
    gitlab_schema: JSONSchema,
}

impl SchemaValidator {
    pub fn new() -> Result<Self, String> {
        let github_schema_json: Value = serde_json::from_str(GITHUB_WORKFLOW_SCHEMA)
            .map_err(|e| format!("Failed to parse GitHub workflow schema: {}", e))?;

        let gitlab_schema_json: Value = serde_json::from_str(GITLAB_CI_SCHEMA)
            .map_err(|e| format!("Failed to parse GitLab CI schema: {}", e))?;

        let github_schema = JSONSchema::compile(&github_schema_json)
            .map_err(|e| format!("Failed to compile GitHub JSON schema: {}", e))?;

        let gitlab_schema = JSONSchema::compile(&gitlab_schema_json)
            .map_err(|e| format!("Failed to compile GitLab JSON schema: {}", e))?;

        Ok(Self {
            github_schema,
            gitlab_schema,
        })
    }

    pub fn validate_workflow(&self, workflow_path: &Path) -> Result<(), String> {
        // Determine the schema type based on the filename
        let schema_type = if workflow_path.file_name().is_some_and(|name| {
            let name_str = name.to_string_lossy();
            name_str.ends_with(".gitlab-ci.yml") || name_str.ends_with(".gitlab-ci.yaml")
        }) {
            SchemaType::GitLab
        } else {
            SchemaType::GitHub
        };

        // Read the workflow file
        let content = fs::read_to_string(workflow_path)
            .map_err(|e| format!("Failed to read workflow file: {}", e))?;

        // Parse YAML to JSON Value
        let workflow_json: Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse workflow YAML: {}", e))?;

        // Validate against the appropriate schema
        let validation_result = match schema_type {
            SchemaType::GitHub => self.github_schema.validate(&workflow_json),
            SchemaType::GitLab => self.gitlab_schema.validate(&workflow_json),
        };

        // Handle validation errors
        if let Err(errors) = validation_result {
            let schema_name = match schema_type {
                SchemaType::GitHub => "GitHub workflow",
                SchemaType::GitLab => "GitLab CI",
            };
            let mut error_msg = format!("{} validation failed:\n", schema_name);
            for error in errors {
                error_msg.push_str(&format!("- {}\n", error));
            }
            return Err(error_msg);
        }

        Ok(())
    }

    pub fn validate_with_specific_schema(
        &self,
        content: &str,
        schema_type: SchemaType,
    ) -> Result<(), String> {
        // Parse YAML to JSON Value
        let workflow_json: Value =
            serde_yaml::from_str(content).map_err(|e| format!("Failed to parse YAML: {}", e))?;

        // Validate against the appropriate schema
        let validation_result = match schema_type {
            SchemaType::GitHub => self.github_schema.validate(&workflow_json),
            SchemaType::GitLab => self.gitlab_schema.validate(&workflow_json),
        };

        // Handle validation errors
        if let Err(errors) = validation_result {
            let schema_name = match schema_type {
                SchemaType::GitHub => "GitHub workflow",
                SchemaType::GitLab => "GitLab CI",
            };
            let mut error_msg = format!("{} validation failed:\n", schema_name);
            for error in errors {
                error_msg.push_str(&format!("- {}\n", error));
            }
            return Err(error_msg);
        }

        Ok(())
    }
}
