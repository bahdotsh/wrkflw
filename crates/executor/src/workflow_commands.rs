//! Parser for GitHub Actions workflow commands embedded in step output.
//!
//! GitHub Actions recognises special `::command::` lines in stdout to set
//! outputs, mask values, group log lines, and emit annotations. This module
//! extracts those commands from raw step output so the engine can apply their
//! effects.

/// A parsed workflow command from step output.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowCommand {
    Error {
        message: String,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
    },
    Warning {
        message: String,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
    },
    Notice {
        message: String,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
    },
    Debug {
        message: String,
    },
    Group {
        name: String,
    },
    EndGroup,
    AddMask {
        value: String,
    },
    /// Deprecated `::set-output` command (replaced by GITHUB_OUTPUT file).
    SetOutput {
        name: String,
        value: String,
    },
    /// `::save-state` command.
    SaveState {
        name: String,
        value: String,
    },
    StopCommands {
        token: String,
    },
}

/// Parse all workflow commands from step output text.
///
/// Returns the commands in the order they appear. Lines that are not
/// workflow commands are silently skipped.
pub fn parse_workflow_commands(output: &str) -> Vec<WorkflowCommand> {
    let mut commands = Vec::new();
    let mut stop_token: Option<String> = None;

    for line in output.lines() {
        let trimmed = line.trim();

        // If commands are stopped, look for the resume token
        if let Some(ref token) = stop_token {
            if trimmed == format!("::{token}::") {
                stop_token = None;
            }
            continue;
        }

        if !trimmed.starts_with("::") {
            continue;
        }

        if let Some(cmd) = parse_command_line(trimmed) {
            if let WorkflowCommand::StopCommands { ref token } = cmd {
                stop_token = Some(token.clone());
            } else {
                commands.push(cmd);
            }
        }
    }

    commands
}

/// Parse a single `::command param=val,param=val::message` line.
fn parse_command_line(line: &str) -> Option<WorkflowCommand> {
    // Format: ::command param1=val1,param2=val2::message
    // The line starts with "::" — strip it.
    let rest = line.strip_prefix("::").unwrap_or(line);

    // Find the second "::" that separates command+params from the message
    let (cmd_part, message) = if let Some(idx) = rest.find("::") {
        (&rest[..idx], rest[idx + 2..].to_string())
    } else {
        return None;
    };

    // Split command name from params (space-separated)
    let (cmd_name, params_str) = if let Some(idx) = cmd_part.find(' ') {
        (&cmd_part[..idx], &cmd_part[idx + 1..])
    } else {
        (cmd_part, "")
    };

    let params = parse_params(params_str);

    match cmd_name {
        "error" => Some(WorkflowCommand::Error {
            message,
            file: params.get("file").cloned(),
            line: params.get("line").and_then(|v| v.parse().ok()),
            col: params.get("col").and_then(|v| v.parse().ok()),
        }),
        "warning" => Some(WorkflowCommand::Warning {
            message,
            file: params.get("file").cloned(),
            line: params.get("line").and_then(|v| v.parse().ok()),
            col: params.get("col").and_then(|v| v.parse().ok()),
        }),
        "notice" => Some(WorkflowCommand::Notice {
            message,
            file: params.get("file").cloned(),
            line: params.get("line").and_then(|v| v.parse().ok()),
            col: params.get("col").and_then(|v| v.parse().ok()),
        }),
        "debug" => Some(WorkflowCommand::Debug { message }),
        "group" => Some(WorkflowCommand::Group { name: message }),
        "endgroup" => Some(WorkflowCommand::EndGroup),
        "add-mask" => Some(WorkflowCommand::AddMask { value: message }),
        "set-output" => {
            let name = params.get("name").cloned().unwrap_or_default();
            Some(WorkflowCommand::SetOutput {
                name,
                value: message,
            })
        }
        "save-state" => {
            let name = params.get("name").cloned().unwrap_or_default();
            Some(WorkflowCommand::SaveState {
                name,
                value: message,
            })
        }
        "stop-commands" => Some(WorkflowCommand::StopCommands { token: message }),
        _ => None,
    }
}

/// Parse `key=value,key=value` parameter string into a map.
fn parse_params(s: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if s.is_empty() {
        return map;
    }
    for pair in s.split(',') {
        if let Some(eq) = pair.find('=') {
            let key = pair[..eq].trim().to_string();
            let value = pair[eq + 1..].trim().to_string();
            map.insert(key, value);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_command() {
        let output = "::error file=app.js,line=10,col=5::Something went wrong";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error {
                message,
                file,
                line,
                col,
            } => {
                assert_eq!(message, "Something went wrong");
                assert_eq!(file.as_deref(), Some("app.js"));
                assert_eq!(*line, Some(10));
                assert_eq!(*col, Some(5));
            }
            _ => panic!("expected Error command"),
        }
    }

    #[test]
    fn parse_warning_no_params() {
        let output = "::warning::This is a warning";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Warning { message, file, .. } => {
                assert_eq!(message, "This is a warning");
                assert!(file.is_none());
            }
            _ => panic!("expected Warning"),
        }
    }

    #[test]
    fn parse_set_output() {
        let output = "::set-output name=version::1.2.3";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::SetOutput { name, value } => {
                assert_eq!(name, "version");
                assert_eq!(value, "1.2.3");
            }
            _ => panic!("expected SetOutput"),
        }
    }

    #[test]
    fn parse_group_endgroup() {
        let output = "::group::My Group\nsome output\n::endgroup::";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 2);
        assert!(matches!(&cmds[0], WorkflowCommand::Group { name } if name == "My Group"));
        assert!(matches!(&cmds[1], WorkflowCommand::EndGroup));
    }

    #[test]
    fn parse_add_mask() {
        let output = "::add-mask::my-secret-value";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], WorkflowCommand::AddMask { value } if value == "my-secret-value")
        );
    }

    #[test]
    fn parse_debug() {
        let output = "::debug::Debug message here";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], WorkflowCommand::Debug { message } if message == "Debug message here")
        );
    }

    #[test]
    fn parse_notice() {
        let output = "::notice file=README.md::Check this out";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Notice { message, file, .. } => {
                assert_eq!(message, "Check this out");
                assert_eq!(file.as_deref(), Some("README.md"));
            }
            _ => panic!("expected Notice"),
        }
    }

    #[test]
    fn non_command_lines_skipped() {
        let output = "regular output\n::warning::warn\nmore output\n";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
    }

    #[test]
    fn stop_commands() {
        let output =
            "::stop-commands::pause\n::warning::should be ignored\n::pause::\n::warning::visible";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], WorkflowCommand::Warning { message, .. } if message == "visible")
        );
    }

    #[test]
    fn save_state() {
        let output = "::save-state name=isPost::true";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::SaveState { name, value } => {
                assert_eq!(name, "isPost");
                assert_eq!(value, "true");
            }
            _ => panic!("expected SaveState"),
        }
    }

    #[test]
    fn multiple_commands() {
        let output = "::group::Build\nbuilding...\n::endgroup::\n::set-output name=result::ok\n::warning::slow build";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 4);
    }
}
