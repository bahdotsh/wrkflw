// Secrets & runtime — screen 7 from the design.
//
// Honesty note: we don't have a rich secrets metadata store (last-used
// timestamps, length, scope etc. are not persisted anywhere). We also
// don't read the user's real secrets config file yet — the tab shows
// `SecretConfig::default()`, i.e. the two providers that are always
// wired (env + file). The providers badge therefore says "defaults" so
// the user isn't misled into believing a customised config has been
// loaded.
//
// The design included a "reveal 5s" cleartext toggle on individual
// secret values. We don't render that: there are no per-secret values
// at this layer to reveal, only provider-source descriptors
// (a filesystem path or an env-var prefix). Masking a filename is
// theatre — it doesn't protect anything and confuses the user about
// what "masking" means here. When `SecretManager::list_known_keys()`
// lands the reveal toggle can come back attached to actual values.
//
// A future PR can flesh this out — e.g. plumb through a real config
// loader plus a key-list for the left pane — and this layout will
// accommodate it without restructure.

use crate::app::App;
use crate::text;
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{HighlightSpacing, List, ListItem, Paragraph},
    Frame,
};
use wrkflw_executor::RuntimeType;
use wrkflw_secrets::{SecretConfig, SecretProviderConfig};

pub fn render_secrets_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let rows = provider_entries();
    let (left, right) = if area.width >= 90 {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Min(0)])
            .split(area);
        (body[0], body[1])
    } else {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(rows.len() as u16 + 2),
                Constraint::Min(0),
            ])
            .split(area);
        (body[0], body[1])
    };
    render_providers_pane(f, app, &rows, left);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(0)])
        .split(right);
    render_detail_pane(f, app, &rows, right[0]);
    render_runtime_pane(f, app, right[1]);
}

fn provider_entries() -> Vec<(String, SecretProviderConfig)> {
    let cfg = SecretConfig::default();
    let mut rows: Vec<(String, SecretProviderConfig)> = cfg.providers.into_iter().collect();
    // Deterministic order — HashMap iteration isn't stable.
    rows.sort_by(|(a, _), (b, _)| a.cmp(b));
    rows
}

fn render_providers_pane(
    f: &mut Frame<'_>,
    app: &mut App,
    rows: &[(String, SecretProviderConfig)],
    area: Rect,
) {
    let block = theme::with_badge(theme::block_focused("Secret providers"), "defaults");
    let inner = block.inner(area);
    let w = inner.width as usize;

    let items: Vec<ListItem> = rows
        .iter()
        .map(|(name, cfg)| {
            let detail = match cfg {
                SecretProviderConfig::Environment { prefix: Some(p) } => format!("{p}*"),
                SecretProviderConfig::Environment { prefix: None } => "environment".to_string(),
                SecretProviderConfig::File { path } => path.clone(),
            };
            let used = 2 + text::width(name) + 2;
            ListItem::new(Line::from(vec![
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    text::truncate_left(&detail, w.saturating_sub(used)).into_owned(),
                    theme::muted_style(),
                ),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol(theme::SELECTED_MARK)
        .highlight_spacing(HighlightSpacing::Always);

    if app
        .secrets_list_state
        .selected()
        .is_none_or(|i| i >= rows.len())
    {
        app.secrets_list_state
            .select((!rows.is_empty()).then_some(0));
    }
    f.render_stateful_widget(list, area, &mut app.secrets_list_state);
}

fn render_detail_pane(
    f: &mut Frame<'_>,
    app: &App,
    rows: &[(String, SecretProviderConfig)],
    area: Rect,
) {
    let sel = app.secrets_list_state.selected().unwrap_or(0);
    let Some((name, cfg)) = rows.get(sel) else {
        f.render_widget(theme::block("Provider"), area);
        return;
    };

    let title = format!("Provider · {}", name);
    let block = theme::block(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Source descriptors (a path or an env-var prefix) aren't secret
    // values, so they're shown plainly.
    let w = inner.width as usize;
    let mut lines = match cfg {
        SecretProviderConfig::Environment { prefix } => vec![
            kv("Kind", "environment variables", w),
            kv(
                "Reads",
                &prefix
                    .as_ref()
                    .map(|p| format!("variables starting with {p}"))
                    .unwrap_or_else(|| "any variable".to_string()),
                w,
            ),
        ],
        SecretProviderConfig::File { path } => vec![kv("Kind", "file", w), kv("Path", path, w)],
    };
    lines.push(kv("Masking", "values are hidden in logs", w));
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_runtime_pane(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = theme::with_badge(theme::block("Runtime"), "e to cycle");
    let inner = block.inner(area);
    f.render_widget(block, area);
    let w = inner.width as usize;

    let options = [
        ("Docker", BadgeKind::Docker, RuntimeType::Docker),
        ("Podman", BadgeKind::Podman, RuntimeType::Podman),
        ("Kubernetes", BadgeKind::Kubernetes, RuntimeType::Kubernetes),
        (
            "Secure emulation",
            BadgeKind::Secure,
            RuntimeType::SecureEmulation,
        ),
        ("Emulation", BadgeKind::Emulation, RuntimeType::Emulation),
    ];
    let mut lines: Vec<Line> = options
        .iter()
        .map(|(label, kind, runtime)| {
            if app.runtime_type == *runtime {
                Line::from(vec![
                    Span::styled(theme::SELECTED_MARK, Style::default().fg(kind.fg())),
                    Span::styled(
                        label.to_string(),
                        Style::default().fg(kind.fg()).add_modifier(Modifier::BOLD),
                    ),
                ])
            } else {
                Line::from(Span::styled(format!("  {label}"), theme::muted_style()))
            }
        })
        .collect();

    let reachable = match app.runtime_type {
        RuntimeType::Docker | RuntimeType::Podman | RuntimeType::Kubernetes => {
            if app.runtime_available {
                "reachable"
            } else {
                "not reachable · runs will fail"
            }
        }
        RuntimeType::SecureEmulation => "always · sandboxed on this machine",
        _ => "always · runs directly on this machine",
    };
    lines.push(Line::from(""));
    lines.push(kv("Available", reachable, w));
    lines.push(kv(
        "On failure",
        if app.preserve_containers_on_failure {
            "containers are kept"
        } else {
            "containers are removed"
        },
        w,
    ));
    f.render_widget(Paragraph::new(lines), inner);
}

fn kv(key: &str, value: &str, w: usize) -> Line<'static> {
    const KEY_W: usize = 12;
    Line::from(vec![
        Span::styled(format!("{key:<KEY_W$}"), theme::muted_style()),
        Span::styled(
            text::truncate(value, w.saturating_sub(KEY_W)).into_owned(),
            Style::default().fg(COLORS.text),
        ),
    ])
}
