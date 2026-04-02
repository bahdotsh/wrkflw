// Status bar rendering
use crate::app::App;
use crate::theme::{self, COLORS};
use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use wrkflw_executor::RuntimeType;

// Render the status bar
pub fn render_status_bar(f: &mut Frame<'_>, app: &App, area: Rect) {
    // If we have a status message, show it as a toast
    if let Some(message) = &app.status_message {
        let is_success = message.contains("success")
            || message.contains("Success")
            || message.contains(theme::symbols::SUCCESS)
            || message.contains('\u{2705}'); // ✅ legacy

        let status_message = Paragraph::new(Line::from(vec![Span::styled(
            format!(" {} ", message),
            Style::default()
                .bg(if is_success {
                    COLORS.success
                } else {
                    COLORS.error
                })
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center);

        f.render_widget(status_message, area);
        return;
    }

    // Normal status bar
    let mut status_items = vec![];

    // Runtime mode badge
    status_items.push(theme::badge(
        app.runtime_type_name(),
        match app.runtime_type {
            RuntimeType::Docker => COLORS.runtime_docker,
            RuntimeType::Podman => COLORS.runtime_podman,
            RuntimeType::SecureEmulation => COLORS.runtime_secure,
            RuntimeType::Emulation => COLORS.runtime_emulation,
        },
        COLORS.text,
    ));

    // Container runtime status
    match app.runtime_type {
        RuntimeType::Docker => {
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
            status_items.push(theme::badge(
                if is_docker_available {
                    "Docker: Connected"
                } else {
                    "Docker: Unavailable"
                },
                if is_docker_available {
                    COLORS.success
                } else {
                    COLORS.error
                },
                COLORS.text,
            ));
        }
        RuntimeType::Podman => {
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
            status_items.push(theme::badge(
                if is_podman_available {
                    "Podman: Connected"
                } else {
                    "Podman: Unavailable"
                },
                if is_podman_available {
                    COLORS.success
                } else {
                    COLORS.error
                },
                COLORS.text,
            ));
        }
        RuntimeType::SecureEmulation => {
            status_items.push(Span::raw(" "));
            status_items.push(theme::badge(
                "\u{1F512}SECURE",
                COLORS.runtime_secure,
                COLORS.text,
            ));
        }
        RuntimeType::Emulation => {}
    }

    // Validation/execution mode badge
    status_items.push(Span::raw(" "));
    if app.validation_mode {
        status_items.push(theme::badge("Validation", COLORS.warning, ratatui::style::Color::Black));
    } else {
        status_items.push(theme::badge("Execution", COLORS.success, ratatui::style::Color::Black));
    }

    // Separator
    status_items.push(Span::styled(
        format!(" {} ", theme::symbols::SEPARATOR),
        Style::default().fg(COLORS.text_muted),
    ));

    // Context-specific help
    let help_text = build_context_help(app);
    status_items.push(Span::styled(help_text, theme::hint_style()));

    let status_bar = Paragraph::new(Line::from(status_items))
        .style(Style::default().bg(COLORS.bg_bar))
        .alignment(Alignment::Left);

    f.render_widget(status_bar, area);
}

fn build_context_help(app: &App) -> String {
    match app.selected_tab {
        0 => {
            if app.job_selection_mode {
                "Enter run \u{2502} a all \u{2502} Esc back".to_string()
            } else {
                "Space toggle \u{2502} Enter run \u{2502} J jobs \u{2502} r queue \u{2502} t trigger \u{2502} ? help \u{2502} q quit".to_string()
            }
        }
        1 => {
            if app.detailed_view {
                "Esc back \u{2502} \u{2191}\u{2193} steps \u{2502} ? help \u{2502} q quit".to_string()
            } else {
                "Enter details \u{2502} \u{2191}\u{2193} jobs \u{2502} ? help \u{2502} q quit".to_string()
            }
        }
        2 => {
            let log_count = app.logs.len() + wrkflw_logging::get_logs().len();
            if log_count > 0 {
                format!(
                    "{} logs \u{2502} \u{2191}\u{2193} scroll \u{2502} s search \u{2502} f filter \u{2502} ? help \u{2502} q quit",
                    log_count
                )
            } else {
                "No logs \u{2502} ? help \u{2502} q quit".to_string()
            }
        }
        3 => "\u{2191}\u{2193} scroll \u{2502} ? close \u{2502} q quit".to_string(),
        _ => String::new(),
    }
}
