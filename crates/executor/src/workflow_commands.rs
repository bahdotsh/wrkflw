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
        end_line: Option<u32>,
        col: Option<u32>,
        end_column: Option<u32>,
        title: Option<String>,
    },
    Warning {
        message: String,
        file: Option<String>,
        line: Option<u32>,
        end_line: Option<u32>,
        col: Option<u32>,
        end_column: Option<u32>,
        title: Option<String>,
    },
    Notice {
        message: String,
        file: Option<String>,
        line: Option<u32>,
        end_line: Option<u32>,
        col: Option<u32>,
        end_column: Option<u32>,
        title: Option<String>,
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

/// Decode GitHub Actions percent-encoded values.
///
/// GitHub Actions encodes: `%25` → `%`, `%0A` → `\n`, `%0D` → `\r`,
/// `%3A` → `:`, `%2C` → `,`, `%3B` → `;`.
fn decode_value(s: &str) -> String {
    // Decode %25 (percent) LAST to avoid double-decode: if input contains
    // `%250A`, decoding %25 first would turn it into `%0A`, which the next
    // step would incorrectly decode to `\n`. GitHub Actions decodes %25 last.
    s.replace("%0A", "\n")
        .replace("%0a", "\n")
        .replace("%0D", "\r")
        .replace("%0d", "\r")
        .replace("%3A", ":")
        .replace("%3a", ":")
        .replace("%2C", ",")
        .replace("%2c", ",")
        .replace("%3B", ";")
        .replace("%3b", ";")
        .replace("%25", "%")
}

/// Parse all workflow commands from step output text.
///
/// Returns the commands in the order they appear. Lines that are not
/// workflow commands are silently skipped.
pub fn parse_workflow_commands(output: &str) -> Vec<WorkflowCommand> {
    let mut commands = Vec::new();
    // Pre-computed resume sentinel to avoid allocation per line
    let mut resume_sentinel: Option<String> = None;

    for line in output.lines() {
        // GitHub Actions only recognizes commands starting at column 0 —
        // indented lines must not be treated as commands.
        let trimmed = line.trim_end();

        // If commands are stopped, look for the resume token
        if let Some(ref sentinel) = resume_sentinel {
            if trimmed == sentinel.as_str() {
                resume_sentinel = None;
            }
            continue;
        }

        if !trimmed.starts_with("::") {
            continue;
        }

        if let Some(cmd) = parse_command_line(trimmed) {
            if let WorkflowCommand::StopCommands { ref token } = cmd {
                if token.is_empty() {
                    // Empty token creates an unmatchable sentinel — skip
                    continue;
                }
                resume_sentinel = Some(format!("::{}::", token));
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
    let (cmd_part, raw_message) = if let Some(idx) = rest.find("::") {
        (&rest[..idx], rest[idx + 2..].to_string())
    } else {
        return None;
    };

    // Decode percent-encoded values in the message
    let message = decode_value(&raw_message);

    // Split command name from params (space-separated)
    let (cmd_name, params_str) = if let Some(idx) = cmd_part.find(' ') {
        (&cmd_part[..idx], &cmd_part[idx + 1..])
    } else {
        (cmd_part, "")
    };

    let params = parse_params(params_str);

    // Helper to extract common annotation parameters
    let build_annotation = |kind: &str,
                            message: String,
                            params: &std::collections::HashMap<String, String>|
     -> Option<WorkflowCommand> {
        let file = params.get("file").map(|v| decode_value(v));
        let line = params.get("line").and_then(|v| v.parse().ok());
        let end_line = params.get("endLine").and_then(|v| v.parse().ok());
        let col = params.get("col").and_then(|v| v.parse().ok());
        let end_column = params.get("endColumn").and_then(|v| v.parse().ok());
        let title = params.get("title").map(|v| decode_value(v));
        Some(match kind {
            "error" => WorkflowCommand::Error {
                message,
                file,
                line,
                end_line,
                col,
                end_column,
                title,
            },
            "warning" => WorkflowCommand::Warning {
                message,
                file,
                line,
                end_line,
                col,
                end_column,
                title,
            },
            _ => WorkflowCommand::Notice {
                message,
                file,
                line,
                end_line,
                col,
                end_column,
                title,
            },
        })
    };

    match cmd_name {
        "error" => build_annotation("error", message, &params),
        "warning" => build_annotation("warning", message, &params),
        "notice" => build_annotation("notice", message, &params),
        "debug" => Some(WorkflowCommand::Debug { message }),
        "group" => Some(WorkflowCommand::Group { name: message }),
        "endgroup" => Some(WorkflowCommand::EndGroup),
        "add-mask" => Some(WorkflowCommand::AddMask { value: message }),
        "set-output" => {
            let name = params
                .get("name")
                .map(|v| decode_value(v))
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            Some(WorkflowCommand::SetOutput {
                name,
                value: message,
            })
        }
        "save-state" => {
            let name = params
                .get("name")
                .map(|v| decode_value(v))
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
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
                ..
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
    fn parse_error_with_end_line_and_title() {
        let output =
            "::error file=app.js,line=5,endLine=10,col=1,endColumn=20,title=Syntax Error::bad code";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error {
                message,
                file,
                line,
                end_line,
                col,
                end_column,
                title,
            } => {
                assert_eq!(message, "bad code");
                assert_eq!(file.as_deref(), Some("app.js"));
                assert_eq!(*line, Some(5));
                assert_eq!(*end_line, Some(10));
                assert_eq!(*col, Some(1));
                assert_eq!(*end_column, Some(20));
                assert_eq!(title.as_deref(), Some("Syntax Error"));
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
    fn parse_set_output_rejects_empty_name() {
        let output = "::set-output::some value";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 0);
    }

    #[test]
    fn parse_save_state_rejects_empty_name() {
        let output = "::save-state::some value";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 0);
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
    fn stop_commands_empty_token_ignored() {
        // An empty stop-commands token should be ignored (not permanently stop parsing)
        let output = "::stop-commands::\n::warning::still visible";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        assert!(
            matches!(&cmds[0], WorkflowCommand::Warning { message, .. } if message == "still visible")
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

    #[test]
    fn url_decoding_in_message() {
        let output = "::error::Line1%0ALine2%0ALine3";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error { message, .. } => {
                assert_eq!(message, "Line1\nLine2\nLine3");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn url_decoding_percent_and_colon() {
        let output = "::add-mask::secret%3Avalue%25encoded";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::AddMask { value } => {
                assert_eq!(value, "secret:value%encoded");
            }
            _ => panic!("expected AddMask"),
        }
    }

    #[test]
    fn url_decoding_in_file_param() {
        let output = "::error file=src%3Amain.rs,line=10::oops";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error { file, .. } => {
                assert_eq!(file.as_deref(), Some("src:main.rs"));
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn url_decoding_no_double_decode() {
        // %250A is a literal percent-encoded "%0A" — it should decode to "%0A",
        // NOT to a newline. This verifies that %25 is decoded last.
        let output = "::error::before%250Aafter";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error { message, .. } => {
                assert_eq!(message, "before%0Aafter");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn url_decoding_comma_and_semicolon() {
        let output = "::error::item1%2Citem2%3Bitem3";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Error { message, .. } => {
                assert_eq!(message, "item1,item2;item3");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn url_decoding_comma_and_semicolon_lowercase() {
        let output = "::warning::a%2cb%3bc";
        let cmds = parse_workflow_commands(output);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            WorkflowCommand::Warning { message, .. } => {
                assert_eq!(message, "a,b;c");
            }
            _ => panic!("expected Warning"),
        }
    }
}
