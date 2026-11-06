//! A running step's output, streamed to the run's observers as whole,
//! masked lines.

use crate::engine::format_annotation_location;
use std::sync::{Arc, Mutex};

/// Streams one step's output to the run's observers as whole, masked
/// lines while the step runs.
pub(crate) struct LiveStep {
    pub(crate) job: String,
    pub(crate) step: String,
    pub(crate) index: usize,
    pub(crate) workspace_prefixes: Vec<String>,
}

/// Longest partial line held back waiting for its newline.
const LIVE_LINE_LIMIT: usize = 4096;

/// When a partial line outgrows [`LIVE_LINE_LIMIT`], this much of its end
/// is held back with the next chunk. The line is masked before it is cut,
/// so only a secret still arriving can straddle the cut, and the held-back
/// end keeps such a secret whole until the rest of it comes.
const LIVE_LINE_OVERLAP: usize = 1024;

impl LiveStep {
    /// `future` is boxed by the caller: a step's future is large, and laid
    /// out inline in every job future it would overflow the stack once
    /// reusable workflows nest jobs.
    pub(crate) async fn run<F: std::future::Future>(
        self,
        future: std::pin::Pin<Box<F>>,
    ) -> F::Output {
        let observers = crate::events::step_output_observers();
        if observers.is_empty() {
            return future.await;
        }
        let state = Arc::new(LiveLines {
            step: self,
            observers,
            pending: Mutex::default(),
        });
        let sink = state.clone();
        let tap = wrkflw_runtime::live_output::OutputTap::new(move |chunk| sink.push(chunk));
        let output = wrkflw_runtime::live_output::with_output_tap(tap, future).await;
        state.flush();
        output
    }
}

struct LiveLines {
    step: LiveStep,
    observers: Vec<Arc<dyn crate::events::ExecutionObserver>>,
    pending: Mutex<PendingLine>,
}

/// Output after the last newline. Lines are held until complete so a
/// secret split across two chunks is still masked.
#[derive(Default)]
struct PendingLine {
    text: String,
    /// An `::add-mask::` line outgrew [`LIVE_LINE_LIMIT`] and is being
    /// dropped up to its newline. Cutting it would show the rest of the line
    /// as output, and the rest of the line is the value it masks.
    dropping_command: bool,
}

impl LiveLines {
    fn push(&self, chunk: &str) {
        let lines = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let mut chunk = chunk;
            if pending.dropping_command {
                match chunk.find('\n') {
                    Some(at) => {
                        pending.dropping_command = false;
                        chunk = &chunk[at + 1..];
                    }
                    None => return,
                }
            }
            pending.text.push_str(chunk);
            // Only a newline ends a line. A carriage return redraws it, as
            // a progress bar does; `send` keeps the line's last state.
            let cut = match pending.text.rfind('\n') {
                Some(at) => at + 1,
                None if pending.text.len() > LIVE_LINE_LIMIT => {
                    if pending.text.starts_with("::add-mask::") {
                        pending.text.clear();
                        pending.dropping_command = true;
                        return;
                    }
                    // Mask the whole line before cutting it, so a secret
                    // already in it can't end up half on either side.
                    pending.text = crate::log_redaction::mask(&pending.text);
                    let mut at = pending.text.len().saturating_sub(LIVE_LINE_OVERLAP);
                    while !pending.text.is_char_boundary(at) {
                        at -= 1;
                    }
                    at
                }
                None => return,
            };
            let complete: String = pending.text.drain(..cut).collect();
            complete
        };
        self.send(&lines);
    }

    fn flush(&self) {
        let rest = {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            pending.dropping_command = false;
            std::mem::take(&mut pending.text)
        };
        self.send(&rest);
    }

    fn send(&self, text: &str) {
        let mut lines: Vec<String> = Vec::new();
        for line in text.split('\n') {
            let Some(line) = visible_line(line) else {
                continue;
            };
            let line = crate::log_redaction::mask(&line);
            lines.push(
                self.step
                    .workspace_prefixes
                    .iter()
                    .fold(line, |line, prefix| line.replace(prefix, "")),
            );
        }
        if lines.is_empty() {
            return;
        }
        let event = crate::events::ExecutionEvent::StepOutput {
            job: self.step.job.clone(),
            step: self.step.step.clone(),
            index: self.step.index,
            lines,
        };
        for observer in &self.observers {
            observer.on_event(&event);
        }
    }
}

/// What a line of live output shows, if anything.
///
/// A carriage return redraws the line, so a progress bar's hundred states
/// are one line showing the last of them. Workflow commands are
/// instructions to the runner, not output, though annotations are shown the
/// way GitHub shows them. `::add-mask::` has to take effect here, before
/// the lines after it are masked, not when the step ends: the value is on
/// this very line, and the step may print it again before finishing. So
/// every state of a line is read for commands, even those redrawn over.
/// A line that only looks like a command is ordinary output.
fn visible_line(line: &str) -> Option<String> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut states = line
        .split('\r')
        .filter(|state| !state.is_empty())
        .peekable();
    let mut shown = None;
    while let Some(state) = states.next() {
        let text = match crate::workflow_commands::parse_command_line(state.trim_end()) {
            None => Some(state.to_string()),
            Some(command) => live_annotation(command),
        };
        if states.peek().is_none() {
            shown = text;
        }
    }
    shown
}

/// How a workflow command reads in live output: annotations as GitHub
/// shows them, and nothing for the rest. Registers `::add-mask::` values.
fn live_annotation(command: crate::workflow_commands::WorkflowCommand) -> Option<String> {
    use crate::workflow_commands::WorkflowCommand as C;
    let (kind, message, file, line, col) = match command {
        C::AddMask { value } => {
            crate::log_redaction::register(&value);
            return None;
        }
        C::Error {
            message,
            file,
            line,
            col,
            ..
        } => ("Error", message, file, line, col),
        C::Warning {
            message,
            file,
            line,
            col,
            ..
        } => ("Warning", message, file, line, col),
        C::Notice {
            message,
            file,
            line,
            col,
            ..
        } => ("Notice", message, file, line, col),
        _ => return None,
    };
    let loc = format_annotation_location(file.as_deref(), line, col);
    Some(format!("{kind}: {loc}{message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Seen(Mutex<Vec<String>>);

    impl crate::events::ExecutionObserver for Seen {
        fn on_event(&self, event: &crate::events::ExecutionEvent) {
            if let crate::events::ExecutionEvent::StepOutput { lines, .. } = event {
                self.0.lock().unwrap().extend(lines.iter().cloned());
            }
        }
    }

    fn live(seen: Arc<Seen>) -> LiveLines {
        LiveLines {
            step: LiveStep {
                job: "build".into(),
                step: "print".into(),
                index: 0,
                workspace_prefixes: Vec::new(),
            },
            observers: vec![seen],
            pending: Mutex::default(),
        }
    }

    #[test]
    fn add_mask_takes_effect_on_the_lines_after_it_and_is_never_shown() {
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = "live-add-mask-sentinel-91ab";
        lines.push(&format!("before\n::add-mask::{value}\ntoken is {value}\n"));
        lines.push(&format!("again {value}"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert!(
            !seen.iter().any(|l| l.contains(value)),
            "the value leaked: {seen:?}"
        );
        assert!(!seen.iter().any(|l| l.starts_with("::")), "{seen:?}");
        assert_eq!(seen.first().map(String::as_str), Some("before"));
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn a_secret_across_a_forced_cut_is_still_masked() {
        let secret = "live-overlap-sentinel-4d0e";
        crate::log_redaction::register(secret);
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        // No newline, so the line is cut once it outgrows the limit, and
        // the secret arrives in two chunks either side of that cut.
        let (head, tail) = secret.split_at(10);
        lines.push(&format!("{}{head}", "x".repeat(LIVE_LINE_LIMIT)));
        lines.push(tail);
        lines.flush();
        let joined: String = seen.0.lock().unwrap().concat();
        assert!(!joined.contains(head), "half the secret leaked");
        assert!(joined.contains("***"));
    }

    #[test]
    fn a_secret_already_buffered_across_the_cut_point_is_masked() {
        let secret = "live-straddle-sentinel-8c3a"; // 27 bytes
        crate::log_redaction::register(secret);
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        // One chunk with no newline, over the limit, whose cut point
        // (LIVE_LINE_OVERLAP from the end) falls inside the secret.
        let chunk = format!(
            "{}{secret}{}",
            "x".repeat(LIVE_LINE_LIMIT),
            "y".repeat(LIVE_LINE_OVERLAP - 10)
        );
        lines.push(&chunk);
        lines.flush();
        let joined: String = seen.0.lock().unwrap().concat();
        assert!(!joined.contains(&secret[..8]), "the secret's start leaked");
        assert!(!joined.contains(&secret[8..]), "the secret's end leaked");
        assert!(joined.contains("***"));
    }

    #[test]
    fn an_add_mask_line_too_long_to_hold_is_never_shown() {
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = "z".repeat(LIVE_LINE_LIMIT + 10);
        lines.push(&format!("::add-mask::{}", &value[..LIVE_LINE_LIMIT]));
        lines.push(&format!("{}\nafter\n", &value[LIVE_LINE_LIMIT..]));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(*seen, vec!["after".to_string()]);
    }

    #[test]
    fn a_redrawn_line_shows_only_its_last_state() {
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push("Downloading 10%\rDownloading 50%");
        lines.push("\rDownloading 100%\r\n");
        lines.push("windows line\r\n");
        // A line ending in a bare carriage return is held until its newline.
        lines.push("split\r");
        lines.push("\nnext\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(
            *seen,
            vec!["Downloading 100%", "windows line", "split", "next"],
            "{seen:?}"
        );
    }

    #[test]
    fn a_mask_redrawn_over_still_takes_effect() {
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = "redrawn-mask-sentinel-5e7d";
        lines.push(&format!("::add-mask::{value}\rprogress\n"));
        lines.push(&format!("leak? {value}\n"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert!(!seen.iter().any(|l| l.contains(value)), "{seen:?}");
        assert_eq!(*seen, vec!["progress", "leak? ***"]);
    }

    #[test]
    fn annotations_show_and_lookalike_lines_are_output() {
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push("::error file=a.rs,line=3::broken\n::warning::careful\n");
        lines.push("::group::Build\n::endgroup::\n::std::mem::take\n:: plain\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(seen[0].starts_with("Error: ") && seen[0].ends_with("broken"));
        assert_eq!(seen[1], "Warning: careful");
        assert_eq!(seen[2], "::std::mem::take");
        assert_eq!(seen[3], ":: plain");
    }
}
