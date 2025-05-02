// Status bar rendering
use crate::app::App;
use executor::RuntimeType;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::io;

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
                RuntimeType::Emulation => Color::Magenta,
            })
            .fg(Color::White),
    ));

    // Add Docker status if relevant
    if app.runtime_type == RuntimeType::Docker {
        // Check Docker silently using safe FD redirection
        let is_docker_available =
            match utils::fd::with_stderr_to_null(executor::docker::is_available) {
                Ok(result) => result,
                Err(_) => {
                    logging::debug("Failed to redirect stderr when checking Docker availability.");
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
    let help_text = match app.selected_tab {
        0 => {
            if let Some(idx) = app.workflow_list_state.selected() {
                if idx < app.workflows.len() {
                    let workflow = &app.workflows[idx];
                    match workflow.status {
                        crate::models::WorkflowStatus::NotStarted => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   [t] Trigger Workflow  [Shift+R] Reset workflow",
                        crate::models::WorkflowStatus::Running => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   (Workflow running...)",
                        crate::models::WorkflowStatus::Success | crate::models::WorkflowStatus::Failed | crate::models::WorkflowStatus::Skipped => "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected   [Shift+R] Reset workflow",
                    }
                } else {
                    "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected"
                }
            } else {
                "[Space] Toggle selection   [Enter] Run selected   [r] Run all selected"
            }
        }
        1 => {
            if app.detailed_view {
                "[Esc] Back to jobs   [↑/↓] Navigate steps"
            } else {
                "[Enter] View details   [↑/↓] Navigate jobs"
            }
        }
        2 => {
            // For logs tab, show scrolling instructions
            let log_count = app.logs.len() + logging::get_logs().len();
            if log_count > 0 {
                // Convert to a static string for consistent return type
                let scroll_text = format!(
                    "[↑/↓] Scroll logs ({}/{}) [s] Search [f] Filter",
                    app.log_scroll + 1,
                    log_count
                );
                Box::leak(scroll_text.into_boxed_str())
            } else {
                "[No logs to display]"
            }
        }
        3 => "[?] Toggle help overlay",
        _ => "",
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
