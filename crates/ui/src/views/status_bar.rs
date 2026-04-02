// Status bar rendering
use crate::app::App;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::io;
use wrkflw_executor::RuntimeType;

// Render the status bar
pub fn render_status_bar(f: &mut Frame<CrosstermBackend<io::Stdout>>, app: &App, area: Rect) {
    // If we have a status message, show it instead of the normal status bar
    if let Some(message) = &app.status_message {
        // Determine if this is a success message (starts with ✅)
        let is_success = message.starts_with("✅");

        let status_message = Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", message),
            Style::default()
                .bg(if is_success { Color::Green } else { Color::Red })
                .fg(Color::White)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )]))
        .alignment(Alignment::Center);

        f.render_widget(status_message, area);
        return;
    }

    // Normal status bar
    let mut status_items = vec![];

    // Add mode info
    status_items.push(Span::styled(
        format!(" {} ", app.runtime_type_name()),
        Style::default()
            .bg(match app.runtime_type {
                RuntimeType::Docker => Color::Blue,
                RuntimeType::Podman => Color::Cyan,
                RuntimeType::SecureEmulation => Color::Green,
                RuntimeType::Emulation => Color::Red,
            })
            .fg(Color::White),
    ));

    // Add container runtime status if relevant
    match app.runtime_type {
        RuntimeType::Docker => {
            // Check Docker silently using safe FD redirection
            let is_docker_available = match wrkflw_utils::fd::with_stderr_to_null(
                wrkflw_executor::docker::is_available,
            ) {
                Ok(result) => result,
                Err(_) => {
                    wrkflw_logging::debug(
                        "Failed to redirect stderr when checking Docker availability.",
                    );
                    false
                }
            };

            status_items.push(Span::raw(" "));
            status_items.push(Span::styled(
                if is_docker_available {
                    " Docker: Connected "
                } else {
                    " Docker: Not Available "
                },
                Style::default()
                    .bg(if is_docker_available {
                        Color::Green
                    } else {
                        Color::Red
                    })
                    .fg(Color::White),
            ));
        }
        RuntimeType::Podman => {
            // Check Podman silently using safe FD redirection
            let is_podman_available = match wrkflw_utils::fd::with_stderr_to_null(
                wrkflw_executor::podman::is_available,
            ) {
                Ok(result) => result,
                Err(_) => {
                    wrkflw_logging::debug(
                        "Failed to redirect stderr when checking Podman availability.",
                    );
                    false
                }
            };

            status_items.push(Span::raw(" "));
            status_items.push(Span::styled(
                if is_podman_available {
                    " Podman: Connected "
                } else {
                    " Podman: Not Available "
                },
                Style::default()
                    .bg(if is_podman_available {
                        Color::Green
                    } else {
                        Color::Red
                    })
                    .fg(Color::White),
            ));
        }
        RuntimeType::SecureEmulation => {
            status_items.push(Span::styled(
                " 🔒SECURE ",
                Style::default().bg(Color::Green).fg(Color::White),
            ));
        }
        RuntimeType::Emulation => {
            // No need to check anything for emulation mode
        }
    }

    // Add validation/execution mode
    status_items.push(Span::raw(" "));
    status_items.push(Span::styled(
        format!(
            " {} ",
            if app.validation_mode {
                "Validation"
            } else {
                "Execution"
            }
        ),
        Style::default()
            .bg(if app.validation_mode {
                Color::Yellow
            } else {
                Color::Green
            })
            .fg(Color::Black),
    ));

    // Add context-specific help based on current tab
    status_items.push(Span::raw(" "));
    let help_text: String = match app.selected_tab {
        0 => {
            if app.job_selection_mode {
                "[Enter] Run job   [a] Run all jobs   [Esc] Back to workflows".to_string()
            } else if let Some(idx) = app.workflow_list_state.selected() {
                if idx < app.workflows.len() {
                    let workflow = &app.workflows[idx];
                    match workflow.status {
                        crate::models::WorkflowStatus::NotStarted => "[Space] Toggle   [Enter] Run   [J] Select jobs   [r] Run selected   [t] Trigger   [Shift+R] Reset".to_string(),
                        crate::models::WorkflowStatus::Running => "[Space] Toggle   [Enter] Run   [J] Select jobs   [r] Run selected   (Running...)".to_string(),
                        crate::models::WorkflowStatus::Success | crate::models::WorkflowStatus::Failed | crate::models::WorkflowStatus::Skipped => "[Space] Toggle   [Enter] Run   [J] Select jobs   [r] Run selected   [Shift+R] Reset".to_string(),
                    }
                } else {
                    "[Space] Toggle   [Enter] Run   [J] Select jobs   [r] Run selected".to_string()
                }
            } else {
                "[Space] Toggle   [Enter] Run   [J] Select jobs   [r] Run selected".to_string()
            }
        }
        1 => {
            if app.detailed_view {
                "[Esc] Back to jobs   [↑/↓] Navigate steps".to_string()
            } else {
                "[Enter] View details   [↑/↓] Navigate jobs".to_string()
            }
        }
        2 => {
            // For logs tab, show scrolling instructions
            let log_count = app.logs.len() + wrkflw_logging::get_logs().len();
            if log_count > 0 {
                format!(
                    "[↑/↓] Scroll logs ({}/{}) [s] Search [f] Filter",
                    app.log_scroll + 1,
                    log_count
                )
            } else {
                "[No logs to display]".to_string()
            }
        }
        3 => "[↑/↓] Scroll help   [?] Toggle help overlay".to_string(),
        _ => String::new(),
    };
    status_items.push(Span::styled(
        format!(" {} ", help_text),
        Style::default().fg(Color::White),
    ));

    // Show keybindings for common actions
    status_items.push(Span::raw(" "));
    status_items.push(Span::styled(
        " [Tab] Switch tabs ",
        Style::default().fg(Color::White),
    ));
    status_items.push(Span::styled(
        " [?] Help ",
        Style::default().fg(Color::White),
    ));
    status_items.push(Span::styled(
        " [q] Quit ",
        Style::default().fg(Color::White),
    ));

    let status_bar = Paragraph::new(Line::from(status_items))
        .style(Style::default().bg(Color::DarkGray))
        .alignment(Alignment::Left);

    f.render_widget(status_bar, area);
}
