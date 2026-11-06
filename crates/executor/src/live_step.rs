//! A running step's output, streamed to the run's observers as whole,
//! masked lines.

use crate::engine::format_annotation_location;
use std::sync::{Arc, Mutex};
use wrkflw_secrets::SecretMasker;

/// Streams one step's output to the run's observers as whole, masked
/// lines while the step runs.
pub(crate) struct LiveStep<'a> {
    pub(crate) job: String,
    pub(crate) step: String,
    pub(crate) index: usize,
    pub(crate) workspace_prefixes: Vec<String>,
    /// The run's masker. It holds every `--variable` value, which the
    /// process-wide log masker only takes when it looks like a credential.
    pub(crate) masker: Option<&'a SecretMasker>,
}

/// What [`LiveLines`] needs of the step it streams.
struct StepLabel {
    job: String,
    step: String,
    index: usize,
    workspace_prefixes: Vec<String>,
}

/// Longest partial line held back waiting for its newline.
const LIVE_LINE_LIMIT: usize = 4096;

/// When a partial line outgrows [`LIVE_LINE_LIMIT`], this much of its end
/// is held back with the next chunk. The line is masked before it is cut,
/// so only a secret still arriving can straddle the cut, and the held-back
/// end keeps such a secret whole until the rest of it comes.
const LIVE_LINE_OVERLAP: usize = 1024;

/// Longest workflow command line collected while it is held back. An
/// `::add-mask::` value longer than this is still masked once the step ends;
/// collecting it live is only worth it for values a step may print again
/// before then.
const DROPPED_COMMAND_LIMIT: usize = 1 << 20;

impl LiveStep<'_> {
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
        // The tap outlives this borrow, so it masks with a copy. Values
        // added during the step come from `::add-mask::`, which the log
        // masker below takes as the line is read.
        let masker = self.masker.map(SecretMasker::snapshot);
        let state = Arc::new(LiveLines {
            step: StepLabel {
                job: self.job,
                step: self.step,
                index: self.index,
                workspace_prefixes: self.workspace_prefixes,
            },
            masker,
            observers,
            pending: Mutex::default(),
        });
        let sink = state.clone();
        let tap = wrkflw_runtime::live_output::OutputTap::new(move |stream, chunk| {
            sink.push(stream, chunk)
        });
        let output = wrkflw_runtime::live_output::with_output_tap(tap, future).await;
        state.flush();
        output
    }
}

struct LiveLines {
    step: StepLabel,
    masker: Option<SecretMasker>,
    observers: Vec<Arc<dyn crate::events::ExecutionObserver>>,
    /// Held while lines are sent too, so observers see them in the order
    /// they were printed, whichever reader thread printed them.
    pending: Mutex<Pending>,
}

use wrkflw_runtime::live_output::Stream;

/// Each stream's partial line. Stdout and stderr are written
/// independently, so a partial line on one is no part of the other's.
#[derive(Default)]
struct Pending {
    stdout: PendingLine,
    stderr: PendingLine,
}

impl Pending {
    fn of(&mut self, stream: Stream) -> &mut PendingLine {
        match stream {
            Stream::Stdout => &mut self.stdout,
            Stream::Stderr => &mut self.stderr,
        }
    }
}

/// Output after the last newline. Lines are held until complete so a
/// secret split across two chunks is still masked.
#[derive(Default)]
struct PendingLine {
    text: String,
    /// A workflow command line outgrew [`LIVE_LINE_LIMIT`] and is held back
    /// up to its newline. Cutting it would show the rest of the line as
    /// output, and for `::add-mask::` the rest of the line is the value it
    /// masks. The line is read whole once it ends.
    held_command: Option<HeldCommand>,
}

struct HeldCommand {
    text: String,
    /// More than [`DROPPED_COMMAND_LIMIT`] arrived; the rest was dropped.
    truncated: bool,
}

impl HeldCommand {
    fn push(&mut self, text: &str) {
        let room = DROPPED_COMMAND_LIMIT.saturating_sub(self.text.len());
        if text.len() <= room {
            self.text.push_str(text);
            return;
        }
        let mut at = room;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        self.text.push_str(&text[..at]);
        self.truncated = true;
    }

    /// The line to read, now that it ended. A truncated `::add-mask::` is
    /// not read at all: masking a prefix of the value would leave the rest
    /// of it showing. Its value is masked once the step ends.
    fn into_line(self) -> Option<String> {
        let last_state = self.text.rsplit('\r').next().unwrap_or("");
        if self.truncated && last_state.starts_with("::add-mask::") {
            return None;
        }
        Some(self.text)
    }
}

impl LiveLines {
    fn push(&self, stream: Stream, chunk: &str) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        for text in self.complete_lines(pending.of(stream), chunk) {
            self.send(&text);
        }
    }

    /// The whole lines `chunk` completes on `line`, as texts for [`send`],
    /// in order.
    ///
    /// [`send`]: LiveLines::send
    fn complete_lines(&self, line: &mut PendingLine, mut chunk: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(command) = line.held_command.as_mut() {
            let (line_end, after) = match chunk.find('\n') {
                Some(at) => (&chunk[..at], Some(&chunk[at + 1..])),
                None => (chunk, None),
            };
            command.push(line_end);
            let Some(after) = after else {
                return out;
            };
            if let Some(command) = line.held_command.take().and_then(HeldCommand::into_line) {
                out.push(command + "\n");
            }
            chunk = after;
        }
        line.text.push_str(chunk);
        // Only a newline ends a line. A carriage return redraws it, as
        // a progress bar does; `send` keeps the line's last state.
        let cut = match line.text.rfind('\n') {
            Some(at) => at + 1,
            None if line.text.len() > LIVE_LINE_LIMIT => {
                // What the line shows now is its last state.
                let last_state = line.text.rsplit('\r').next().unwrap_or("");
                if last_state.starts_with("::") {
                    let mut command = HeldCommand {
                        text: String::new(),
                        truncated: false,
                    };
                    command.push(&std::mem::take(&mut line.text));
                    line.held_command = Some(command);
                    return out;
                }
                // Mask the whole line before cutting it, so a secret
                // already in it can't end up half on either side.
                line.text = self.mask(&line.text);
                let mut at = line.text.len().saturating_sub(LIVE_LINE_OVERLAP);
                while !line.text.is_char_boundary(at) {
                    at -= 1;
                }
                // Cut where the line was redrawn, if it was, so what is
                // shown is a whole state rather than half of one.
                if let Some(redraw) = line.text[..at].rfind('\r').filter(|&r| r > 0) {
                    at = redraw + 1;
                }
                at
            }
            None => return out,
        };
        out.push(line.text.drain(..cut).collect());
        out
    }

    fn flush(&self) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        for stream in [Stream::Stdout, Stream::Stderr] {
            let line = pending.of(stream);
            let command = line.held_command.take().and_then(HeldCommand::into_line);
            let rest = std::mem::take(&mut line.text);
            if let Some(command) = command {
                self.send(&(command + "\n"));
            }
            self.send(&rest);
        }
    }

    /// `text` with the run's secrets and every logged-masked value hidden.
    fn mask(&self, text: &str) -> String {
        let text = match &self.masker {
            Some(masker) => masker.mask(text),
            None => text.to_string(),
        };
        crate::log_redaction::mask(&text)
    }

    /// Send the lines of `text`. A final newline ends the last line rather
    /// than starting an empty one; every other empty line is output.
    fn send(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = text.strip_suffix('\n').unwrap_or(text);
        let mut lines: Vec<String> = Vec::new();
        for line in text.split('\n') {
            let Some(line) = visible_line(line) else {
                continue;
            };
            let line = self.mask(&line);
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
    // A line with nothing on it is a blank line of output.
    let mut shown = Some(String::new());
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

    const OUT: Stream = Stream::Stdout;
    const ERR: Stream = Stream::Stderr;

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
            step: StepLabel {
                job: "build".into(),
                step: "print".into(),
                index: 0,
                workspace_prefixes: Vec::new(),
            },
            masker: None,
            observers: vec![seen],
            pending: Mutex::default(),
        }
    }

    #[test]
    fn add_mask_takes_effect_on_the_lines_after_it_and_is_never_shown() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = "live-add-mask-sentinel-91ab";
        lines.push(
            OUT,
            &format!("before\n::add-mask::{value}\ntoken is {value}\n"),
        );
        lines.push(OUT, &format!("again {value}"));
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
        let _run = crate::log_redaction::begin_run();
        let secret = "live-overlap-sentinel-4d0e";
        crate::log_redaction::register(secret);
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        // No newline, so the line is cut once it outgrows the limit, and
        // the secret arrives in two chunks either side of that cut.
        let (head, tail) = secret.split_at(10);
        lines.push(OUT, &format!("{}{head}", "x".repeat(LIVE_LINE_LIMIT)));
        lines.push(OUT, tail);
        lines.flush();
        let joined: String = seen.0.lock().unwrap().concat();
        assert!(!joined.contains(head), "half the secret leaked");
        assert!(joined.contains("***"));
    }

    #[test]
    fn a_secret_already_buffered_across_the_cut_point_is_masked() {
        let _run = crate::log_redaction::begin_run();
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
        lines.push(OUT, &chunk);
        lines.flush();
        let joined: String = seen.0.lock().unwrap().concat();
        assert!(!joined.contains(&secret[..8]), "the secret's start leaked");
        assert!(!joined.contains(&secret[8..]), "the secret's end leaked");
        assert!(joined.contains("***"));
    }

    #[test]
    fn an_add_mask_line_too_long_to_hold_is_never_shown() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        // Distinct from any other test's text: the log masker is global.
        let value = format!("long-mask-{}", "q7".repeat(LIVE_LINE_LIMIT));
        let cut = LIVE_LINE_LIMIT;
        lines.push(OUT, &format!("::add-mask::{}", &value[..cut]));
        lines.push(OUT, &format!("{}\nafter\n", &value[cut..]));
        // The value took effect although its line was too long to hold.
        lines.push(OUT, &format!("again {value}\n"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(seen.first().map(String::as_str), Some("after"));
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert!(!seen[1].contains(&value), "the long masked value leaked");
    }

    #[test]
    fn every_value_the_run_masks_is_masked_live() {
        let _run = crate::log_redaction::begin_run();
        // A plain `--variable` value: the run masks it in stored output,
        // but the log masker leaves it alone because it isn't a credential.
        let masker = SecretMasker::new();
        masker.add_secret("plainvalue-live-5c2");
        let seen = Arc::new(Seen::default());
        let mut lines = live(seen.clone());
        lines.masker = Some(masker.snapshot());
        lines.push(OUT, "branch is plainvalue-live-5c2\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert!(!seen[0].contains("plainvalue-live-5c2"), "{seen:?}");
    }

    #[test]
    fn a_redrawn_line_shows_only_its_last_state() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push(OUT, "Downloading 10%\rDownloading 50%");
        lines.push(OUT, "\rDownloading 100%\r\n");
        lines.push(OUT, "windows line\r\n");
        // A line ending in a bare carriage return is held until its newline.
        lines.push(OUT, "split\r");
        lines.push(OUT, "\nnext\n");
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
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = "redrawn-mask-sentinel-5e7d";
        lines.push(OUT, &format!("::add-mask::{value}\rprogress\n"));
        lines.push(OUT, &format!("leak? {value}\n"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert!(!seen.iter().any(|l| l.contains(value)), "{seen:?}");
        assert_eq!(*seen, vec!["progress", "leak? ***"]);
    }

    #[test]
    fn annotations_show_and_lookalike_lines_are_output() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push(
            OUT,
            "::error file=a.rs,line=3::broken\n::warning::careful\n",
        );
        lines.push(
            OUT,
            "::group::Build\n::endgroup::\n::std::mem::take\n:: plain\n",
        );
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(seen[0].starts_with("Error: ") && seen[0].ends_with("broken"));
        assert_eq!(seen[1], "Warning: careful");
        assert_eq!(seen[2], "::std::mem::take");
        assert_eq!(seen[3], ":: plain");
    }

    #[test]
    fn stdout_and_stderr_partial_lines_stay_apart() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push(OUT, "Progress: 42%");
        lines.push(ERR, "warning: foo\n");
        lines.push(OUT, " done\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(*seen, vec!["warning: foo", "Progress: 42% done"]);
    }

    #[test]
    fn blank_lines_are_output() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push(OUT, "one\n\ntwo\n");
        lines.push(OUT, "\n");
        lines.push(OUT, "three");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(*seen, vec!["one", "", "two", "", "three"]);
    }

    #[test]
    fn a_long_redrawn_line_is_cut_where_it_was_redrawn() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let mut bar = String::new();
        let mut n = 0;
        while bar.len() <= LIVE_LINE_LIMIT {
            bar.push_str(&format!("Downloading {n:05}\r"));
            n += 1;
        }
        lines.push(OUT, &bar);
        lines.push(OUT, "Downloaded\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        for line in seen.iter() {
            assert!(
                line == "Downloaded" || (line.starts_with("Downloading ") && line.len() == 17),
                "a partial state was shown: {line:?}"
            );
        }
        assert_eq!(seen.last().map(String::as_str), Some("Downloaded"));
    }

    #[test]
    fn a_long_add_mask_after_a_redraw_is_held_to_its_newline() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = format!("redraw-long-mask-{}", "z9".repeat(LIVE_LINE_LIMIT));
        let line = format!("progress\r::add-mask::{value}");
        let (first, second) = line.split_at(LIVE_LINE_LIMIT + 10);
        lines.push(OUT, first);
        lines.push(OUT, &format!("{second}\nagain {value}\n"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        let joined = seen.concat();
        assert!(
            !joined.contains("z9z9"),
            "part of the value leaked: {seen:?}"
        );
        assert_eq!(*seen, vec!["again ***"]);
    }

    #[test]
    fn a_truncated_add_mask_registers_no_prefix_of_its_value() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let value = format!("huge-mask-{}", "w".repeat(DROPPED_COMMAND_LIMIT));
        lines.push(OUT, &format!("::add-mask::{value}\nafter huge-mask-www\n"));
        lines.flush();
        let seen = seen.0.lock().unwrap();
        // A prefix of the value was not registered, so text that merely
        // shares that prefix is not masked; the step's end masks the value.
        assert_eq!(*seen, vec!["after huge-mask-www"]);
    }

    #[test]
    fn a_long_lookalike_command_line_is_still_shown() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        let line = format!(":: {}", "o".repeat(LIVE_LINE_LIMIT + 5));
        lines.push(OUT, &line);
        lines.push(OUT, "\nnext\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(*seen, vec![line, "next".to_string()]);
    }

    #[test]
    fn a_long_set_output_line_shows_nothing() {
        let _run = crate::log_redaction::begin_run();
        let seen = Arc::new(Seen::default());
        let lines = live(seen.clone());
        lines.push(
            OUT,
            &format!("::set-output name=x::{}", "v".repeat(LIVE_LINE_LIMIT)),
        );
        lines.push(OUT, "vvv\nnext\n");
        lines.flush();
        let seen = seen.0.lock().unwrap();
        assert_eq!(*seen, vec!["next"]);
    }
}
