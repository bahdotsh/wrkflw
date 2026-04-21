// Secrets & runtime — screen 7 from the design.
//
// Honesty note: we don't have a rich secrets metadata store (last-used
// timestamps, length, scope etc. are not persisted anywhere). We also
// don't read the user's real secrets config file yet — the tab shows
// `SecretConfig::default()`, i.e. the two providers that are always
// wired (env + file). The header badge therefore says "defaults" so
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
use crate::theme::{self, BadgeKind, COLORS};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, Paragraph, Wrap},
    Frame,
};
use wrkflw_executor::RuntimeType;
use wrkflw_secrets::{SecretConfig, SecretProviderConfig};

pub fn render_secrets_tab(f: &mut Frame<'_>, app: &mut App, area: Rect) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(area);

    render_header(f, outer[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Min(0)])
        .split(outer[1]);

    // One provider read per frame — the three panes share the same slice.
    let rows = provider_entries();
    render_providers_pane(f, app, &rows, body[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Min(0)])
        .split(body[1]);
    render_detail_pane(f, app, &rows, right[0]);
    render_runtime_pane(f, app, &rows, right[1]);
}

fn render_header(f: &mut Frame<'_>, area: Rect) {
    let spans = vec![
        Span::styled(
            "Secrets & runtime",
            Style::default()
                .fg(COLORS.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(COLORS.text_muted)),
        theme::badge_outline("masking: on", BadgeKind::Success),
        Span::raw(" "),
        // "defaults" rather than "configured": the tab reads
        // `SecretConfig::default()` unconditionally — a custom
        // config file is not loaded yet. Labelling this "configured"
        // would be the exact kind of quiet UI lie PR #104 set out to
        // avoid.
        theme::badge_outline("default providers", BadgeKind::Info),
    ];
    f.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Left),
        area,
    );
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
    let block = theme::block_focused("Providers");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = rows
        .iter()
        .map(|(name, cfg)| {
            let kind = match cfg {
                SecretProviderConfig::Environment { prefix } => match prefix {
                    Some(p) => format!("env (prefix: {})", p),
                    None => "env".to_string(),
                },
                SecretProviderConfig::File { path } => format!("file → {}", path),
            };
            let spans = vec![
                Span::styled("◉ ", Style::default().fg(COLORS.warning)),
                Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(COLORS.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  ", Style::default()),
                Span::styled(kind, Style::default().fg(COLORS.text_dim)),
            ];
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(COLORS.bg_selected)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(" ▸ ");

    f.render_stateful_widget(list, inner, &mut app.secrets_list_state);
}

fn render_detail_pane(
    f: &mut Frame<'_>,
    app: &App,
    rows: &[(String, SecretProviderConfig)],
    area: Rect,
) {
    let sel = app.secrets_list_state.selected().unwrap_or(0);
    let (name, cfg) = match rows.get(sel) {
        Some(r) => r,
        None => {
            let block = theme::block("Detail");
            f.render_widget(block, area);
            return;
        }
    };

    let title = format!("Provider · {}", name);
    let block = theme::block(&title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    // Source descriptor (file path or env-var prefix) — shown plainly.
    // These aren't secret values, so bullet-masking them would be
    // theatre (see module header). Real per-secret values will come
    // later and land in a dedicated row with an actual reveal toggle.
    let (source_label, source_value) = match cfg {
        SecretProviderConfig::Environment { prefix } => (
            "Source".to_string(),
            prefix
                .clone()
                .map(|p| format!("env vars matching {}*", p))
                .unwrap_or_else(|| "any env var".to_string()),
        ),
        SecretProviderConfig::File { path } => ("Path".to_string(), path.clone()),
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}: ", source_label),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(source_value, Style::default().fg(COLORS.text)),
    ]));
    lines.push(Line::from(""));

    let kind_label = match cfg {
        SecretProviderConfig::Environment { .. } => "environment",
        SecretProviderConfig::File { .. } => "file",
    };
    lines.push(kv("Kind", kind_label));
    lines.push(kv("Masking", "applies to resolved values (not shown here)"));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_runtime_pane(
    f: &mut Frame<'_>,
    app: &App,
    rows: &[(String, SecretProviderConfig)],
    area: Rect,
) {
    let block = theme::block("Runtime");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Pills row.
    let pill = |label: &str, kind: BadgeKind, active: bool| -> Vec<Span<'_>> {
        if active {
            vec![theme::badge_solid(label.to_string(), kind), Span::raw(" ")]
        } else {
            vec![
                theme::badge_outline(label.to_string(), kind),
                Span::raw(" "),
            ]
        }
    };
    let mut pills: Vec<Span> = Vec::new();
    pills.extend(pill(
        "Docker",
        BadgeKind::Docker,
        matches!(app.runtime_type, RuntimeType::Docker),
    ));
    pills.extend(pill(
        "Podman",
        BadgeKind::Podman,
        matches!(app.runtime_type, RuntimeType::Podman),
    ));
    pills.extend(pill(
        "Emulation",
        BadgeKind::Emulation,
        matches!(app.runtime_type, RuntimeType::Emulation),
    ));
    pills.extend(pill(
        "Secure-emu",
        BadgeKind::Secure,
        matches!(app.runtime_type, RuntimeType::SecureEmulation),
    ));
    let mut lines: Vec<Line> = vec![Line::from(pills)];
    lines.push(Line::from(""));
    lines.push(kv("Active", app.runtime_type_name()));
    lines.push(kv(
        "Available",
        if app.runtime_available {
            "yes"
        } else {
            "no (will use emulation)"
        },
    ));
    lines.push(kv(
        "Preserve on failure",
        if app.preserve_containers_on_failure {
            "on"
        } else {
            "off"
        },
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "PROVIDERS · routing",
        Style::default()
            .fg(COLORS.highlight)
            .add_modifier(Modifier::BOLD),
    )]));
    for (name, cfg) in rows {
        let right = match cfg {
            SecretProviderConfig::Environment { prefix } => prefix
                .clone()
                .map(|p| format!("{}*", p))
                .unwrap_or_else(|| "$*".to_string()),
            SecretProviderConfig::File { path } => path.clone(),
        };
        lines.push(Line::from(vec![
            Span::styled(name.clone(), Style::default().fg(theme::current_accent())),
            Span::raw(" → "),
            Span::styled(right, Style::default().fg(COLORS.text)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        theme::key_chip("e"),
        Span::raw(" "),
        Span::styled("cycle runtime", Style::default().fg(COLORS.text_dim)),
    ]));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn kv<'a>(key: &'a str, value: impl Into<String>) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("  {:<20}", key),
            Style::default().fg(COLORS.text_muted),
        ),
        Span::styled(value.into(), Style::default().fg(COLORS.text)),
    ])
}
