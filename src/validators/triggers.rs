use crate::models::ValidationResult;
use serde_yaml::Value;

pub fn validate_triggers(on: &Value, result: &mut ValidationResult) {
    let valid_events = vec![
        "branch_protection_rule",
        "check_run",
        "check_suite",
        "create",
        "delete",
        "deployment",
        "deployment_status",
        "discussion",
        "discussion_comment",
        "fork",
        "gollum",
        "issue_comment", // Covers comments on PRs that are not part of a diff
        "issues",
        "label",
        "merge_group",
        "milestone",
        "page_build",
        "public",
        "pull_request",
        "pull_request_review",
        "pull_request_review_comment",
        "pull_request_target",
        "push",
        "registry_package",
        "release",
        "repository_dispatch",
        "schedule",
        "status",
        "watch",
        "workflow_call",
        "workflow_dispatch",
        "workflow_run",
    ];

    match on {
        Value::String(event) => {
            if !valid_events.contains(&event.as_str()) {
                result.add_issue(format!("Unknown trigger event: '{}'", event));
            }
        }
        Value::Sequence(events) => {
            for event in events {
                if let Some(event_str) = event.as_str() {
                    if !valid_events.contains(&event_str) {
                        result.add_issue(format!("Unknown trigger event: '{}'", event_str));
                    }
                }
            }
        }
        Value::Mapping(event_map) => {
            for (event, _) in event_map {
                if let Some(event_str) = event.as_str() {
                    if !valid_events.contains(&event_str) {
                        result.add_issue(format!("Unknown trigger event: '{}'", event_str));
                    }
                }
            }

            // Check schedule syntax if present
            if let Some(Value::Sequence(schedules)) =
                event_map.get(Value::String("schedule".to_string()))
            {
                for schedule in schedules {
                    if let Some(schedule_map) = schedule.as_mapping() {
                        if let Some(Value::String(cron)) =
                            schedule_map.get(Value::String("cron".to_string()))
                        {
                            validate_cron_syntax(cron, result);
                        } else {
                            result.add_issue("Schedule is missing 'cron' expression".to_string());
                        }
                    }
                }
            }
        }
        _ => {
            result.add_issue("'on' section has invalid format".to_string());
        }
    }
}

fn validate_cron_syntax(cron: &str, result: &mut ValidationResult) {
    // Basic validation of cron syntax
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        result.add_issue(format!(
            "Invalid cron syntax '{}': should have 5 components",
            cron
        ));
    }
}
