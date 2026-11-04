//! Render every screen at hostile sizes with hostile data, so a layout
//! that panics or overflows on a tiny or huge terminal fails a test
//! instead of a user's session.
use crate::app::App;
use crate::log_processor::ProcessedLogEntry;
use crate::models::{
    JobExecution, StepExecution, TriggerMatchStatus, Workflow, WorkflowExecution, WorkflowStatus,
};
use crate::theme;
use crate::views::{self, TAB_COUNT, TAB_EXECUTION, TAB_WORKFLOWS};
use ratatui::{backend::TestBackend, text::Span, Terminal};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use wrkflw_executor::{JobStatus, RuntimeType, StepStatus};

const SIZES: &[(u16, u16)] = &[
    (1, 1),
    (2, 2),
    (5, 3),
    (10, 5),
    (3, 80),
    (80, 3),
    (20, 8),
    (40, 12),
    (60, 20),
    (80, 24),
    (100, 30),
    (120, 40),
    (200, 60),
    (300, 100),
];

const NASTY: &[&str] = &[
    "",
    "a",
    "日本語のワークフロー名前がとても長い場合",
    "🚀🔥 emoji-heavy ✨ workflow 🎉",
    "tab\tseparated\tname",
    "a-really-really-really-really-really-really-really-really-long-workflow-name-that-never-ends",
    "e\u{301}\u{301}\u{301} combining",
];

fn big_definition(
    dir: &std::path::Path,
    jobs: usize,
) -> Arc<wrkflw_parser::workflow::WorkflowDefinition> {
    static DEF: std::sync::OnceLock<Arc<wrkflw_parser::workflow::WorkflowDefinition>> =
        std::sync::OnceLock::new();
    DEF.get_or_init(|| parse_big(dir, jobs)).clone()
}

fn parse_big(
    dir: &std::path::Path,
    jobs: usize,
) -> Arc<wrkflw_parser::workflow::WorkflowDefinition> {
    let mut yaml = String::from("name: big\non: [push]\njobs:\n");
    for i in 0..jobs {
        yaml.push_str(&format!(
            "  job_{i}_with_a_long_name:\n    runs-on: ubuntu-latest\n"
        ));
        if i > 0 {
            yaml.push_str(&format!("    needs: [job_{}_with_a_long_name]\n", i / 2));
        }
        yaml.push_str("    steps:\n      - run: echo hi\n");
    }
    let path = dir.join(format!("big-{jobs}.yml"));
    std::fs::write(&path, yaml).unwrap();
    Arc::new(wrkflw_parser::workflow::parse_workflow(&path).expect("big workflow parses"))
}

fn execution(jobs: usize, steps: usize) -> WorkflowExecution {
    let job_status = [JobStatus::Success, JobStatus::Failure, JobStatus::Skipped];
    let step_status = [
        StepStatus::Success,
        StepStatus::Failure,
        StepStatus::Skipped,
    ];
    WorkflowExecution {
        jobs: (0..jobs)
            .map(|j| JobExecution {
                name: format!("{} {j}", NASTY[j % NASTY.len()]),
                canonical_name: format!("job_{j}_with_a_long_name"),
                status: job_status[j % 3].clone(),
                steps: (0..steps)
                    .map(|s| StepExecution {
                        name: format!("{} step {s}", NASTY[s % NASTY.len()]),
                        status: step_status[s % 3],
                        output: (0..50)
                            .map(|l| {
                                format!("line {l}\t\x1b[31mred\x1b[0m {}", NASTY[l % NASTY.len()])
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        exit_code: (s % 3 == 1).then_some(s as i32),
                    })
                    .collect(),
                logs: (0..30)
                    .map(|l| format!("log {l} {}", NASTY[l % NASTY.len()]))
                    .collect(),
                failure_allowed: j % 4 == 0,
            })
            .collect(),
        start_time: chrono::Local::now() - chrono::Duration::seconds(3725),
        end_time: None,
        progress: 0.42,
    }
}

fn app(workflows: usize, dir: &std::path::Path) -> App {
    let (tx, _rx) = mpsc::channel();
    let mut app = App::new(
        RuntimeType::Emulation,
        tx,
        false,
        false,
        HashMap::new(),
        Default::default(),
    );
    let def = big_definition(dir, 40);
    let statuses = [
        WorkflowStatus::NotStarted,
        WorkflowStatus::Running,
        WorkflowStatus::Success,
        WorkflowStatus::Failed,
        WorkflowStatus::Skipped,
    ];
    app.workflows = (0..workflows)
        .map(|i| Workflow {
            name: format!("{}{i}", NASTY[i % NASTY.len()]),
            path: PathBuf::from(format!(
                ".github/workflows/{}/{i}.yml",
                NASTY[i % NASTY.len()]
            )),
            selected: i % 2 == 0,
            status: statuses[i % statuses.len()].clone(),
            execution_details: (i % 3 != 0).then(|| execution(12, 9)),
            job_names: (0..i % 15)
                .map(|j| format!("job-{j}-{}", NASTY[j % NASTY.len()]))
                .collect(),
            trigger_match: match i % 3 {
                0 => None,
                1 => Some(TriggerMatchStatus::Matched("paths matched src/**".into())),
                _ => Some(TriggerMatchStatus::Skipped("branch filter".into())),
            },
            definition: (i % 2 == 0).then(|| def.clone()),
        })
        .collect();
    app.processed_logs = (0..500)
        .map(|i| ProcessedLogEntry {
            timestamp: "12:00:00".into(),
            log_type: ["ERROR", "WARN", "INFO", "SUCCESS", "TRIG"][i % 5].into(),
            log_style: theme::log_badge("INFO"),
            content_spans: vec![Span::raw(format!(
                "{} {}",
                NASTY[i % NASTY.len()],
                "x".repeat(i)
            ))],
        })
        .collect();
    app
}

type Scenario = (&'static str, fn(&mut App));

fn scenarios() -> Vec<Scenario> {
    vec![
        ("plain", |_| {}),
        ("help overlay scrolled far", |a| {
            a.show_help = true;
            a.help_scroll = 10_000;
        }),
        ("tweaks", |a| a.tweaks_open = true),
        ("job selection", |a| {
            a.selected_tab = TAB_WORKFLOWS;
            a.workflow_list_state
                .select(Some(a.workflows.len().saturating_sub(1)));
            a.enter_job_selection_mode();
            a.selected_job_index = 999;
        }),
        ("running + selection past end", |a| {
            a.running = true;
            a.current_execution = Some(1);
            a.workflow_list_state.select(Some(10_000));
            a.job_list_state.select(Some(10_000));
            a.step_list_state.select(Some(10_000));
            a.step_table_state.select(Some(10_000));
            a.spinner_frame = usize::MAX;
            a.runs_selected = 10_000;
            a.secrets_list_state.select(Some(10_000));
            a.trigger_workflow_idx = 10_000;
        }),
        ("long status toast", |a| {
            a.set_error_message("x".repeat(500) + " 日本語 🚀");
        }),
        ("log search", |a| {
            a.log_search_active = true;
            a.log_search_query = "日本語🚀".repeat(40);
            a.log_search_matches = (0..500).step_by(3).collect();
            a.log_search_match_idx = 10_000;
            a.log_scroll = 10_000;
        }),
        ("trigger inputs", |a| {
            a.trigger_branch = "feature/".to_string() + &"very-long-branch-".repeat(20);
            a.trigger_inputs = (0..40)
                .map(|i| {
                    (
                        format!("key{i}{}", NASTY[i % NASTY.len()]),
                        "v".repeat(i * 5),
                    )
                })
                .collect();
            a.trigger_input_cursor = Some(39);
            a.trigger_input_on_value = true;
        }),
        ("diff filter + dag list", |a| {
            a.diff_filter_active = true;
            a.dag_list_view = true;
            a.validation_mode = true;
        }),
        ("no workflows", |a| {
            a.workflows.clear();
            a.processed_logs.clear();
            a.runs.clear();
            a.workflow_list_state.select(None);
        }),
    ]
}

fn render(app: &mut App, w: u16, h: u16) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| views::render_ui(f, app)).unwrap();
}

#[test]
fn every_screen_renders_at_every_size_without_panicking() {
    let dir = std::env::temp_dir().join(format!("wrkflw-stress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for (name, setup) in scenarios() {
        for count in [1usize, 3, 60] {
            // One app per scenario: building one spawns threads and reads
            // the run history, which would dominate if done per render.
            let mut a = app(count, &dir);
            setup(&mut a);
            if a.current_execution.is_none() {
                a.current_execution = Some(count.min(2) - 1);
            }
            for tab in 0..TAB_COUNT {
                let views: &[(bool, usize)] = if tab == TAB_EXECUTION {
                    &[(false, 0), (true, 0), (true, 1)]
                } else {
                    &[(false, 0)]
                };
                for &(detailed, sub) in views {
                    for &(w, h) in SIZES {
                        a.selected_tab = tab;
                        a.detailed_view = detailed;
                        a.step_inspector_tab = sub;
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            render(&mut a, w, h)
                        }));
                        if let Err(e) = result {
                            let msg = e
                                .downcast_ref::<String>()
                                .cloned()
                                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                                .unwrap_or_default();
                            failures.push(format!(
                                "{name} tab={tab} detailed={detailed}/{sub} {w}x{h} n={count}: {msg}"
                            ));
                        }
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        failures.is_empty(),
        "{} panics:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `cargo test -p wrkflw-ui dump_screens -- --ignored --nocapture` prints
/// every screen as text, for eyeballing a layout change.
#[test]
#[ignore]
fn dump_screens() {
    let dir = std::env::temp_dir().join(format!("wrkflw-dump-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h) = std::env::var("DUMP_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((80, 24));
    let only = std::env::var("DUMP_SCENARIO").ok();
    for (name, setup) in scenarios() {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        for tab in 0..TAB_COUNT {
            let subs: &[(bool, usize)] = if tab == TAB_EXECUTION {
                &[(false, 0), (true, 0), (true, 1)]
            } else {
                &[(false, 0)]
            };
            for &(detailed, sub) in subs {
                let mut a = app(5, &dir);
                a.selected_tab = tab;
                setup(&mut a);
                if tab == TAB_EXECUTION && a.current_execution.is_none() {
                    a.current_execution = Some(1);
                }
                a.detailed_view = detailed;
                a.step_inspector_tab = sub;
                let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
                terminal.draw(|f| views::render_ui(f, &mut a)).unwrap();
                println!("===== {name} tab={tab} detailed={detailed}/{sub}");
                let buf = terminal.backend().buffer();
                for y in 0..h {
                    let mut line = String::new();
                    let mut skip = 0;
                    for x in 0..w {
                        if skip > 0 {
                            skip -= 1;
                            continue;
                        }
                        let sym = buf[(x, y)].symbol();
                        skip = unicode_width::UnicodeWidthStr::width(sym).saturating_sub(1);
                        line.push_str(sym);
                    }
                    println!("{}", line.trim_end());
                }
            }
        }
    }
}
