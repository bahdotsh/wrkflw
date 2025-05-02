use jsonschema::JSONSchema;
use serde_json::Value;
use std::fs;
use std::path::Path;

const GITHUB_WORKFLOW_SCHEMA: &str = include_str!("../../../schemas/github-workflow.json");

pub struct SchemaValidator {
    schema: JSONSchema,
}

impl SchemaValidator {
    pub fn new() -> Result<Self, String> {
        let schema_json: Value = serde_json::from_str(GITHUB_WORKFLOW_SCHEMA)
            .map_err(|e| format!("Failed to parse GitHub workflow schema: {}", e))?;

        let schema = JSONSchema::compile(&schema_json)
            .map_err(|e| format!("Failed to compile JSON schema: {}", e))?;

        Ok(Self { schema })
    }

    pub fn validate_workflow(&self, workflow_path: &Path) -> Result<(), String> {
        // Read the workflow file
        let content = fs::read_to_string(workflow_path)
            .map_err(|e| format!("Failed to read workflow file: {}", e))?;

        // Parse YAML to JSON Value
        let workflow_json: Value = serde_yaml::from_str(&content)
            .map_err(|e| format!("Failed to parse workflow YAML: {}", e))?;

        // Validate against schema
        if let Err(errors) = self.schema.validate(&workflow_json) {
            let mut error_msg = String::from("Workflow validation failed:\n");
            for error in errors {
                error_msg.push_str(&format!("- {}\n", error));
            }
            return Err(error_msg);
        }

        Ok(())
    }
}
