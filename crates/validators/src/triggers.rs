use serde_yaml::Value;
use wrkflw_models::ValidationResult;

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
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        result.add_issue(format!(
            "Invalid cron syntax '{}': should have 5 components (minute hour day month day-of-week)",
            cron
        ));
        return;
    }

    let field_specs: [(&str, u32, u32); 5] = [
        ("minute", 0, 59),
        ("hour", 0, 23),
        ("day of month", 1, 31),
        ("month", 1, 12),
        ("day of week", 0, 7), // 7 is accepted as Sunday alias (GitHub Actions, POSIX)
    ];

    for (part, (name, min, max)) in parts.iter().zip(field_specs.iter()) {
        if !is_valid_cron_field(part, *min, *max) {
            result.add_issue(format!(
                "Invalid cron {} value '{}' in '{}': expected {}-{}, *, or a valid expression",
                name, part, cron, min, max
            ));
        }
    }
}

fn is_valid_cron_field(field: &str, min: u32, max: u32) -> bool {
    if field == "*" {
        return true;
    }

    // Handle comma-separated values: "1,15,30"
    for item in field.split(',') {
        if !is_valid_cron_atom(item, min, max) {
            return false;
        }
    }
    true
}

fn is_valid_cron_atom(atom: &str, min: u32, max: u32) -> bool {
    // Handle step values: "*/5" or "1-10/2"
    let (base, step) = if let Some((b, s)) = atom.split_once('/') {
        match s.parse::<u32>() {
            Ok(v) if v >= 1 => (b, Some(v)),
            _ => return false,
        }
    } else {
        (atom, None)
    };

    // Handle wildcard with step: "*/5"
    if base == "*" {
        return step.is_some();
    }

    // Named month values (min=1, max=12)
    let month_names = [
        "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
    ];
    // Named day-of-week values (min=0, max=7; 7 is Sunday alias)
    let dow_names = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

    let resolve_named = |s: &str| -> Option<u32> {
        let upper = s.to_uppercase();
        if min == 1 && max == 12 {
            month_names
                .iter()
                .position(|&n| n == upper)
                .map(|i| i as u32 + 1)
        } else if min == 0 && max >= 6 {
            dow_names.iter().position(|&n| n == upper).map(|i| i as u32)
        } else {
            None
        }
    };

    // Handle ranges: "1-5" or "MON-FRI"
    if let Some((start_s, end_s)) = base.split_once('-') {
        let start_val = start_s
            .parse::<u32>()
            .ok()
            .or_else(|| resolve_named(start_s));
        let end_val = end_s.parse::<u32>().ok().or_else(|| resolve_named(end_s));
        return match (start_val, end_val) {
            (Some(start), Some(end)) => start >= min && end <= max && start <= end,
            _ => false,
        };
    }

    // Named single value (step not supported for named values)
    if let Some(v) = resolve_named(base) {
        return v >= min && v <= max && step.is_none();
    }

    // Single numeric value (with optional step, e.g. "5/2" means starting at 5, every 2)
    match base.parse::<u32>() {
        Ok(v) => v >= min && v <= max,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cron_issues(cron: &str) -> Vec<String> {
        let mut result = ValidationResult::default();
        validate_cron_syntax(cron, &mut result);
        result.issues
    }

    #[test]
    fn valid_standard_crons() {
        assert!(cron_issues("0 0 * * *").is_empty());
        assert!(cron_issues("30 2 * * 1-5").is_empty());
        assert!(cron_issues("*/15 * * * *").is_empty());
        assert!(cron_issues("0 0 1 1 *").is_empty());
        assert!(cron_issues("0,30 * * * *").is_empty());
        assert!(cron_issues("1,15,30 0 1-7 * 1-5").is_empty());
        assert!(cron_issues("0-59/2 * * * *").is_empty());
    }

    #[test]
    fn rejects_out_of_range_values() {
        assert!(!cron_issues("99 99 99 99 99").is_empty());
        assert!(!cron_issues("60 * * * *").is_empty());
        assert!(!cron_issues("* 24 * * *").is_empty());
        assert!(!cron_issues("* * 32 * *").is_empty());
        assert!(!cron_issues("* * * 13 *").is_empty());
        assert!(cron_issues("* * * * 7").is_empty()); // 7 is valid (Sunday alias)
        assert!(!cron_issues("* * * * 8").is_empty());
    }

    #[test]
    fn rejects_wrong_part_count() {
        assert!(!cron_issues("* * *").is_empty());
        assert!(!cron_issues("* * * * * *").is_empty());
        assert!(!cron_issues("*").is_empty());
    }

    #[test]
    fn rejects_invalid_step() {
        assert!(!cron_issues("*/0 * * * *").is_empty());
        assert!(!cron_issues("*/abc * * * *").is_empty());
    }

    #[test]
    fn rejects_invalid_ranges_and_atoms() {
        assert!(!cron_issues("5-2 * * * *").is_empty()); // inverted range
        assert!(!cron_issues("1- * * * *").is_empty()); // partial range
        assert!(!cron_issues("1,,3 * * * *").is_empty()); // empty comma item
        assert!(!cron_issues("abc * * * *").is_empty()); // non-numeric
    }

    #[test]
    fn valid_ranges_with_steps() {
        assert!(cron_issues("1-30/5 * * * *").is_empty());
        assert!(cron_issues("0-23/2 0-23/2 * * *").is_empty());
    }

    #[test]
    fn valid_named_cron_values() {
        assert!(cron_issues("0 0 * JAN MON").is_empty());
        assert!(cron_issues("0 0 * * MON-FRI").is_empty());
        assert!(cron_issues("0 0 * JAN-MAR *").is_empty());
    }

    #[test]
    fn valid_numeric_with_step() {
        // "5/2" means starting at 5, every 2 — valid POSIX cron
        assert!(cron_issues("5/2 * * * *").is_empty());
        assert!(cron_issues("0 3/4 * * *").is_empty());
    }
}
