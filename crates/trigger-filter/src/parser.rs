use crate::error::TriggerFilterError;
use crate::model::{EventFilter, GlobPattern, WorkflowTriggerConfig};
use std::path::PathBuf;
use wrkflw_parser::workflow::WorkflowDefinition;

/// Parse the `on_raw` YAML value from a WorkflowDefinition into structured trigger config.
///
/// Glob patterns in `branches`, `tags`, `paths`, and their `*-ignore` counterparts
/// are compiled here so that invalid patterns surface as a `ParseError` instead of
/// silently never matching at evaluation time.
pub fn parse_trigger_config(
    workflow: &WorkflowDefinition,
    workflow_path: PathBuf,
) -> Result<WorkflowTriggerConfig, TriggerFilterError> {
    let events = parse_events(&workflow.on_raw)?;
    Ok(WorkflowTriggerConfig {
        workflow_path,
        workflow_name: workflow.name.clone(),
        events,
    })
}

fn parse_events(on_raw: &serde_yaml::Value) -> Result<Vec<EventFilter>, TriggerFilterError> {
    match on_raw {
        // on: push
        serde_yaml::Value::String(event) => Ok(vec![EventFilter {
            event_name: event.clone(),
            ..Default::default()
        }]),

        // on: [push, pull_request]
        serde_yaml::Value::Sequence(events) => {
            let mut filters = Vec::new();
            for event in events {
                if let Some(name) = event.as_str() {
                    filters.push(EventFilter {
                        event_name: name.to_string(),
                        ..Default::default()
                    });
                }
            }
            Ok(filters)
        }

        // on: { push: { branches: [main], paths: [src/**] } }
        serde_yaml::Value::Mapping(map) => {
            let mut filters = Vec::new();
            for (key, value) in map {
                let event_name = key
                    .as_str()
                    .ok_or_else(|| {
                        TriggerFilterError::ParseError("Event name must be a string".to_string())
                    })?
                    .to_string();

                let filter = parse_event_config(&event_name, value)?;
                filters.push(filter);
            }
            Ok(filters)
        }

        _ => Err(TriggerFilterError::ParseError(
            "'on' section has invalid format".to_string(),
        )),
    }
}

fn parse_event_config(
    event_name: &str,
    value: &serde_yaml::Value,
) -> Result<EventFilter, TriggerFilterError> {
    // null or empty config means no filters
    if value.is_null() || value == &serde_yaml::Value::Mapping(serde_yaml::Mapping::new()) {
        return Ok(EventFilter {
            event_name: event_name.to_string(),
            ..Default::default()
        });
    }

    let map = match value.as_mapping() {
        Some(m) => m,
        None => {
            return Ok(EventFilter {
                event_name: event_name.to_string(),
                ..Default::default()
            })
        }
    };

    Ok(EventFilter {
        event_name: event_name.to_string(),
        branches: extract_glob_list(map, "branches", event_name)?,
        branches_ignore: extract_glob_list(map, "branches-ignore", event_name)?,
        tags: extract_glob_list(map, "tags", event_name)?,
        tags_ignore: extract_glob_list(map, "tags-ignore", event_name)?,
        paths: extract_glob_list(map, "paths", event_name)?,
        paths_ignore: extract_glob_list(map, "paths-ignore", event_name)?,
        types: extract_string_list(map, "types"),
    })
}

fn extract_string_list(map: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    let value = match map.get(serde_yaml::Value::String(key.to_string())) {
        Some(v) => v,
        None => return Vec::new(),
    };

    match value {
        serde_yaml::Value::String(s) => vec![s.clone()],
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Extract a list of strings from a YAML mapping and compile each as a glob pattern.
///
/// Compilation failures are surfaced as `TriggerFilterError::ParseError` so that
/// a typo like `paths: [src/**.rs]` is reported to the user instead of silently
/// causing the workflow to never trigger.
fn extract_glob_list(
    map: &serde_yaml::Mapping,
    key: &str,
    event_name: &str,
) -> Result<Vec<GlobPattern>, TriggerFilterError> {
    let raw = extract_string_list(map, key);
    raw.into_iter()
        .map(|source| {
            GlobPattern::new(&source).map_err(|e| {
                TriggerFilterError::ParseError(format!(
                    "Invalid glob pattern '{}' under '{}.{}': {}",
                    source, event_name, key, e
                ))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_on_raw(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn parse_string_trigger() {
        let raw = make_on_raw("push");
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "push");
        assert!(events[0].paths.is_empty());
    }

    #[test]
    fn parse_sequence_trigger() {
        let raw = make_on_raw("[push, pull_request]");
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_name, "push");
        assert_eq!(events[1].event_name, "pull_request");
    }

    #[test]
    fn parse_mapping_with_paths() {
        let raw = make_on_raw(
            r#"
push:
  branches: [main, release/**]
  paths:
    - 'src/**'
    - 'Cargo.toml'
pull_request:
  paths-ignore:
    - 'docs/**'
    - '*.md'
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);

        let push = events.iter().find(|e| e.event_name == "push").unwrap();
        let branch_sources: Vec<&str> = push.branches.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(branch_sources, vec!["main", "release/**"]);
        let path_sources: Vec<&str> = push.paths.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(path_sources, vec!["src/**", "Cargo.toml"]);

        let pr = events
            .iter()
            .find(|e| e.event_name == "pull_request")
            .unwrap();
        let pi: Vec<&str> = pr.paths_ignore.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(pi, vec!["docs/**", "*.md"]);
    }

    #[test]
    fn parse_mapping_with_null_config() {
        let raw = make_on_raw(
            r#"
workflow_dispatch:
push:
  branches: [main]
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);
        let wd = events
            .iter()
            .find(|e| e.event_name == "workflow_dispatch")
            .unwrap();
        assert!(wd.paths.is_empty());
        assert!(wd.branches.is_empty());
    }

    #[test]
    fn parse_mapping_with_tags() {
        let raw = make_on_raw(
            r#"
push:
  tags:
    - 'v*'
  tags-ignore:
    - 'v*-rc*'
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events[0].tags[0].source, "v*");
        assert_eq!(events[0].tags_ignore[0].source, "v*-rc*");
    }

    #[test]
    fn parse_mapping_with_types() {
        let raw = make_on_raw(
            r#"
pull_request:
  types:
    - opened
    - synchronize
    - reopened
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "pull_request");
        assert_eq!(events[0].types, vec!["opened", "synchronize", "reopened"]);
    }

    #[test]
    fn parse_single_string_type() {
        let raw = make_on_raw(
            r#"
issues:
  types: opened
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events[0].types, vec!["opened"]);
    }

    #[test]
    fn invalid_glob_pattern_surfaces_as_parse_error() {
        // Unclosed bracket is an invalid glob — should fail at parse time, not silently
        let raw = make_on_raw(
            r#"
push:
  paths:
    - '[unclosed'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid glob"), "got: {}", msg);
        assert!(msg.contains("[unclosed"), "got: {}", msg);
        assert!(msg.contains("push.paths"), "got: {}", msg);
    }

    #[test]
    fn invalid_branches_glob_surfaces_as_parse_error() {
        let raw = make_on_raw(
            r#"
push:
  branches:
    - 'main'
    - '[bad'
"#,
        );
        assert!(parse_events(&raw).is_err());
    }
}
