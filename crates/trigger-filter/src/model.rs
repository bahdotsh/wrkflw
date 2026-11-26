use glob::{MatchOptions, Pattern, PatternError};
use std::path::PathBuf;

/// A warning buffer that insists on being drained before drop.
///
/// The trigger-filter crate deliberately routes non-fatal diagnostics
/// (unknown event names, `git ls-files --others` failures, etc.)
/// through struct fields instead of calling `wrkflw_logging::warning`
/// directly — hosts own the rendering policy. That decoupling is only
/// as strong as every host's willingness to drain the field; a single
/// forgetful `filter_map(... .ok())` anywhere in the pipeline
/// reintroduces the silent-skip failure mode this crate exists to
/// plug.
///
/// `MustDrainWarnings` makes the contract self-enforcing in debug
/// builds: if a non-empty instance is dropped without its contents
/// being observed via [`MustDrainWarnings::take`], the Drop impl
/// prints an `eprintln!` naming the unobserved warnings. Tests and
/// CI catch the regression immediately; release builds carry no
/// overhead. The Drop check is a `debug_assertions` guard, not a
/// `panic!`, because panicking from Drop on a production code path
/// (e.g. a watcher evicting a cache entry at shutdown under
/// memory pressure) would be strictly worse than the silent skip
/// it is trying to prevent.
///
/// **Clone semantics.** Clones carry an independent copy of the
/// warnings — cloning does not count as observation. A cloned
/// instance must be drained separately or its own Drop check will
/// fire. This is the right behavior for the library's primary
/// clone sites (the process-wide LRU cache and `filter_trigger_configs`
/// iteration), because a clone whose warnings are silently dropped
/// is exactly the same failure mode as the original — we want both
/// paths to trip the check.
#[derive(Debug, Default)]
pub struct MustDrainWarnings {
    inner: Vec<String>,
}

impl MustDrainWarnings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, warning: String) {
        self.inner.push(warning);
    }

    pub fn extend<I: IntoIterator<Item = String>>(&mut self, iter: I) {
        self.inner.extend(iter);
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Iterate without draining. Callers that use this form MUST
    /// follow it with a drain before the instance is dropped —
    /// otherwise the debug-mode Drop check fires. Prefer
    /// [`MustDrainWarnings::take`] whenever ownership semantics allow.
    pub fn iter(&self) -> std::slice::Iter<'_, String> {
        self.inner.iter()
    }

    /// Drain the buffer and return its contents. Satisfies the
    /// Drop-time observation contract — after this, the instance
    /// can be dropped cleanly even if the returned `Vec` is never
    /// used again.
    pub fn take(&mut self) -> Vec<String> {
        std::mem::take(&mut self.inner)
    }
}

impl Clone for MustDrainWarnings {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl From<Vec<String>> for MustDrainWarnings {
    fn from(inner: Vec<String>) -> Self {
        Self { inner }
    }
}

impl Drop for MustDrainWarnings {
    fn drop(&mut self) {
        // Silent in release builds — no production overhead, no
        // stderr spam for users on platforms where the library is
        // embedded without a host that drains. The loud behaviour
        // is strictly a development/test guardrail.
        //
        // Also silent if we're already unwinding a panic:
        // eprintln-ing during panic drop would stomp on the real
        // failure message and confuse the test output.
        if cfg!(debug_assertions) && !self.inner.is_empty() && !std::thread::panicking() {
            eprintln!(
                "wrkflw-trigger-filter: {} warning(s) were dropped without being drained by the host: {:?}",
                self.inner.len(),
                self.inner,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_empties_the_buffer_and_returns_contents() {
        let mut w = MustDrainWarnings::new();
        w.push("alpha".to_string());
        w.push("beta".to_string());
        assert_eq!(w.len(), 2);
        let drained = w.take();
        assert_eq!(drained, vec!["alpha".to_string(), "beta".to_string()]);
        assert!(w.is_empty(), "take must leave the buffer empty");
        // Dropping the now-empty `w` at end of scope must NOT fire
        // the debug-mode Drop check.
    }

    #[test]
    fn dropping_after_take_is_silent() {
        // Regression pin: if a future refactor flips the Drop check
        // to fire on `len == 0 && drained == false` (a "did you
        // remember to call take?" guard), this test catches it.
        // The contract is: empty buffer at Drop = clean, regardless
        // of whether anything was ever pushed.
        let w = MustDrainWarnings::new();
        drop(w);
    }

    #[test]
    fn from_vec_roundtrip() {
        let mut w = MustDrainWarnings::from(vec!["x".to_string()]);
        let drained = w.take();
        assert_eq!(drained, vec!["x".to_string()]);
    }

    #[test]
    fn clone_produces_independent_buffers() {
        // Clone semantics are load-bearing for the library's LRU
        // cache: the cached clone must NOT count as observation of
        // the original. This test pins that cloning yields two
        // independent buffers, each of which must be drained
        // separately.
        let mut a = MustDrainWarnings::from(vec!["only-on-a".to_string()]);
        let mut b = a.clone();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        let _drain_a = a.take();
        assert!(a.is_empty());
        // b is still populated — drain it too so the Drop check
        // doesn't fire at end of scope.
        let _drain_b = b.take();
        assert!(b.is_empty());
    }
}

/// A glob pattern paired with its source string.
///
/// The source is retained so diagnostic messages can refer back to what the
/// user wrote in the workflow YAML (e.g. `branch 'main' did not match
/// ["release/*"]` instead of the compiled `Pattern`'s `Debug` output).
///
/// GitHub lets `**` sit inside a path component, as in `**.go` or
/// `docs/**.md`, where it matches any characters, `/` included. The `glob`
/// crate only takes `**` as a whole component, so such a pattern compiles
/// to a few that together match the same: `pattern` and `alternatives`.
/// Match with [`GlobPattern::matches`], which tries them all; they are
/// private, so nothing can match against only the first.
#[derive(Debug, Clone)]
pub struct GlobPattern {
    pub source: String,
    pattern: Pattern,
    alternatives: Vec<Pattern>,
}

/// At most this many patterns stand for one source pattern: each component
/// with `**` inside it doubles them, so four such components fit and a
/// fifth is refused rather than matched with less than it says.
const MAX_GLOB_ALTERNATIVES: usize = 16;

impl GlobPattern {
    pub fn new(source: impl Into<String>) -> Result<Self, PatternError> {
        let source = source.into();
        let mut compiled = expand_inner_double_stars(&source)?
            .iter()
            .map(|alternative| Pattern::new(alternative))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter();
        // `expand_inner_double_stars` always returns at least the source.
        let pattern = compiled.next().expect("at least one pattern");
        Ok(Self {
            source,
            pattern,
            alternatives: compiled.collect(),
        })
    }

    /// Whether `text`, a ref name or a path, matches, with
    /// [`GlobPattern::match_options`].
    pub fn matches(&self, text: &str) -> bool {
        let opts = Self::match_options();
        std::iter::once(&self.pattern)
            .chain(&self.alternatives)
            .any(|pattern| pattern.matches_with(text, opts))
    }

    /// Match options used for both ref and path glob matching.
    ///
    /// GitHub Actions semantics: `*` does not cross `/`, `**` does.
    pub fn match_options() -> MatchOptions {
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        }
    }
}

/// The `glob` patterns that together match what GitHub matches with
/// `source`. A component holding `**` next to other characters, `P**S`,
/// matches `P`, anything, then `S`: within one component that is `P*S`, and
/// across several `P*/**/*S`. Each `**` in a component, `a**b**c` having
/// two, makes that choice of its own. Patterns with no such component come
/// back unchanged, alone. More than [`MAX_GLOB_ALTERNATIVES`] is an error.
///
/// Errors in the rest of the pattern, such as an unclosed `[`, are found in
/// the source itself, so their position is the one the user wrote.
fn expand_inner_double_stars(source: &str) -> Result<Vec<String>, PatternError> {
    Pattern::new(&same_length_check(source))?;
    let mut expanded = vec![String::new()];
    let mut offset = 0;
    for (index, component) in source.split('/').enumerate() {
        let start = offset;
        offset += component.len() + 1;
        let choices: Vec<String> = match inner_double_star_pieces(component) {
            Some(pieces) => {
                let mut choices = vec![pieces[0].to_string()];
                for piece in &pieces[1..] {
                    choices = choices
                        .iter()
                        .flat_map(|head| {
                            [format!("{head}*{piece}"), format!("{head}*/**/*{piece}")]
                        })
                        .collect();
                }
                choices
            }
            None => vec![component.to_string()],
        };
        if expanded.len() * choices.len() > MAX_GLOB_ALTERNATIVES {
            return Err(PatternError {
                pos: start,
                msg: "too many `**` inside path components (at most 4)",
            });
        }
        expanded = expanded
            .iter()
            .flat_map(|head| {
                choices.iter().map(move |choice| {
                    if index == 0 {
                        choice.clone()
                    } else {
                        format!("{head}/{choice}")
                    }
                })
            })
            .collect();
    }
    Ok(expanded)
}

/// `component` split at each run of two or more `*` outside a `[...]`
/// class, all of which GitHub takes as `**`, or `None` when it has none or
/// is `**` alone, which `glob` takes itself. A `*` inside a class is a
/// literal character and stays where it is.
fn inner_double_star_pieces(component: &str) -> Option<Vec<&str>> {
    if component == "**" {
        return None;
    }
    let runs = star_runs(component);
    let mut pieces = Vec::new();
    let mut start = 0;
    for (at, end) in runs.into_iter().filter(|(at, end)| end - at >= 2) {
        pieces.push(&component[start..at]);
        start = end;
    }
    if pieces.is_empty() {
        return None;
    }
    pieces.push(&component[start..]);
    Some(pieces)
}

/// The byte ranges of each run of `*` in `component` outside a `[...]`
/// class, read as `glob` reads a class: a `]` right after `[` or `[!` is a
/// member, not the end, and a `[` with no `]` to close it is left for
/// `glob` to refuse.
fn star_runs(component: &str) -> Vec<(usize, usize)> {
    let bytes = component.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'[' => {
                let mut j = i + 1;
                if bytes.get(j) == Some(&b'!') {
                    j += 1;
                }
                if bytes.get(j) == Some(&b']') {
                    j += 1;
                }
                match bytes[j.min(bytes.len())..].iter().position(|&b| b == b']') {
                    Some(close) => i = j + close + 1,
                    None => i += 1,
                }
            }
            b'*' => {
                let start = i;
                while bytes.get(i) == Some(&b'*') {
                    i += 1;
                }
                runs.push((start, i));
            }
            _ => i += 1,
        }
    }
    runs
}

/// `source` with each run of `*` outside a class, in a component that has
/// `**` inside it, written `*?..`, of the same length, which `glob`
/// accepts: what is left to refuse is the rest of the pattern, at the
/// position it has in `source`.
fn same_length_check(source: &str) -> String {
    source
        .split('/')
        .map(|component| {
            if inner_double_star_pieces(component).is_none() {
                return component.to_string();
            }
            let mut out = component.to_string();
            // Same length, so each run's position holds after the last.
            for (at, end) in star_runs(component) {
                let run = format!("*{}", "?".repeat(end - at - 1));
                out.replace_range(at..end, &run);
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Parsed trigger configuration for a single event type (e.g., push, pull_request).
///
/// Marked `#[non_exhaustive]` so adding new filter axes (e.g. a
/// hypothetical future `labels:` or `authors:` filter) is a non-
/// breaking change. Construct via `..Default::default()` and the
/// builder-ish shape the rest of the crate already uses; external
/// code cannot pattern-match every field.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct EventFilter {
    pub event_name: String,
    pub branches: Vec<GlobPattern>,
    pub branches_ignore: Vec<GlobPattern>,
    pub tags: Vec<GlobPattern>,
    pub tags_ignore: Vec<GlobPattern>,
    pub paths: Vec<GlobPattern>,
    pub paths_ignore: Vec<GlobPattern>,
    pub types: Vec<String>,
}

/// Complete trigger configuration for a workflow.
///
/// `#[non_exhaustive]` so future fields (e.g. parsed `concurrency:`,
/// `env:`, or a cached source hash) can be added without forcing
/// every external reader to update. Within this crate struct-literal
/// construction is still permitted — the attribute only blocks
/// external crates, which today only read the fields and never build
/// this type themselves.
///
/// `warnings` carries non-fatal parse diagnostics (unknown event
/// names, typo detection) that the library surfaces as data rather
/// than via the global logger. Hosts that want to render them to the
/// user should iterate this field after a successful
/// [`crate::parse_trigger_config`]. Routing them through the struct
/// instead of `wrkflw_logging::warning` keeps the library decoupled
/// from the log sink — an embedder using a different logger (or
/// running silently in tests) never sees spurious output.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WorkflowTriggerConfig {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub events: Vec<EventFilter>,
    /// Parser-level diagnostics (unknown event names, typo
    /// detection). Hosts MUST drain via `warnings.take()` to satisfy
    /// the `MustDrainWarnings` contract — see the type docs for
    /// rationale.
    pub warnings: MustDrainWarnings,
}

/// Simulated event context for matching.
///
/// **Not `#[non_exhaustive]` on purpose.** The watcher crate
/// constructs this via struct literal from its own `cached_git_state`
/// path, so adding `non_exhaustive` here would break the cross-crate
/// build. If a future field is added, update the watcher's call site
/// in the same commit.
#[derive(Debug, Clone, Default)]
pub struct EventContext {
    pub event_name: String,
    /// The branch the event happened on (the head branch for `pull_request`).
    pub branch: Option<String>,
    /// The base branch the PR targets — only meaningful for `pull_request`
    /// and `pull_request_target`. GitHub Actions' `branches:` filter on a
    /// pull-request trigger matches against THIS, not [`branch`].
    pub base_branch: Option<String>,
    pub tag: Option<String>,
    pub changed_files: Vec<String>,
    /// `true` when the caller actually *ran* a diff / filesystem
    /// snapshot to populate [`changed_files`], even if the result was
    /// empty. `false` means the caller built the context without
    /// attempting to enumerate changes (e.g. `wrkflw run` without
    /// `--diff` or `--changed-files`).
    ///
    /// This distinction powers a better diagnostic in
    /// [`crate::eval::explain_filter_failure`]: an empty change set
    /// under `changed_files_explicit == true` means "the diff you
    /// requested came back empty"; under `false` it means "you didn't
    /// ask for one, so of course there's nothing to match against".
    /// The old single-case "pass --diff or --changed-files" message
    /// was actively wrong for the first scenario and sent users
    /// chasing a flag they had already passed.
    pub changed_files_explicit: bool,
    /// Activity type for events that support it (e.g., "opened", "synchronize" for pull_request)
    pub activity_type: Option<String>,
    /// Non-fatal diagnostics collected while building this context.
    ///
    /// Populated by the git helpers when a best-effort enrichment
    /// failed (the canonical example is `git ls-files --others` being
    /// rejected by a restrictive safe-directory config, which silently
    /// dropped untracked files from the changed set for the entire
    /// cycle). Hosts MUST drain this via `warnings.take()` so the
    /// `MustDrainWarnings` Drop check stays satisfied — failing to
    /// drain is exactly the silent-skip failure mode this crate has
    /// been iteratively patched to prevent, and the Drop check
    /// catches the regression in debug builds.
    pub warnings: MustDrainWarnings,
}

/// Result of trigger evaluation for a single workflow.
///
/// `#[non_exhaustive]` — external callers read this struct but never
/// build it (all construction happens inside `eval.rs`). Future
/// fields (e.g. a structured machine-readable match explanation)
/// can be added without breaking consumers.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TriggerMatchResult {
    pub workflow_path: PathBuf,
    pub workflow_name: String,
    pub matches: bool,
    pub matched_event: Option<String>,
    pub reason: String,
}

#[cfg(test)]
mod glob_pattern_tests {
    use super::GlobPattern;

    #[test]
    fn double_star_inside_a_component_matches_across_directories() {
        // lib/pq's `paths: ['**.go']`: any Go file, at any depth.
        let go = GlobPattern::new("**.go").unwrap();
        assert!(go.matches("conn.go"));
        assert!(go.matches("internal/proto/types.go"));
        assert!(!go.matches("go.mod"));
        assert!(!go.matches("README.md"));

        let docs = GlobPattern::new("docs/**.md").unwrap();
        assert!(docs.matches("docs/index.md"));
        assert!(docs.matches("docs/guide/install.md"));
        assert!(!docs.matches("src/index.md"));

        let prefixed = GlobPattern::new("release-**").unwrap();
        assert!(prefixed.matches("release-1.2"));
        assert!(prefixed.matches("release-1.2/hotfix"));
        assert!(!prefixed.matches("main"));
    }

    #[test]
    fn whole_component_patterns_are_unchanged() {
        let all = GlobPattern::new("**").unwrap();
        assert!(all.matches("feature/x"));
        let nested = GlobPattern::new("src/**/*.rs").unwrap();
        assert!(nested.matches("src/a/b.rs"));
        assert!(nested.matches("src/b.rs"));
        let star = GlobPattern::new("*.md").unwrap();
        assert!(star.matches("README.md"));
        assert!(!star.matches("docs/README.md"));
    }

    #[test]
    fn four_inner_double_stars_match_fully_and_a_fifth_is_refused() {
        let pattern = GlobPattern::new("a**/b**/c**/d**").unwrap();
        assert!(pattern.matches("a/b/c/d"));
        // Each component's `**` crosses directories, the last one's too.
        assert!(pattern.matches("a1/x/b2/c3/y/z/d4"));
        assert!(pattern.matches("a/b/c/x/d"));
        let refused = GlobPattern::new("a**/b**/c**/d**/e**").unwrap_err();
        assert_eq!(refused.pos, "a**/b**/c**/d**/".len());
    }

    #[test]
    fn several_double_stars_in_one_component_each_cross_directories() {
        let pattern = GlobPattern::new("a**b**c").unwrap();
        assert!(pattern.matches("abc"));
        assert!(pattern.matches("a1b2c"));
        assert!(pattern.matches("a/x/b/y/c"));
        assert!(pattern.matches("ax/b/yc"));
        assert!(!pattern.matches("a/c"));
        // `***` is `**`.
        assert!(GlobPattern::new("src/***.rs")
            .unwrap()
            .matches("src/a/b.rs"));
        // Two in one component count twice toward the limit.
        assert!(GlobPattern::new("a**b**/c**d**").is_ok());
        assert!(GlobPattern::new("a**b**c/d**e**f/g**").is_err());
    }

    /// `*` inside a `[...]` class is a literal character, so `**` there is
    /// no wildcard and the component is left as written.
    #[test]
    fn stars_inside_a_class_are_not_split() {
        let class = GlobPattern::new("[**]x").unwrap();
        assert!(class.matches("*x"));
        assert!(!class.matches("ax"));
        assert!(!class.matches("a/x"));
        // `]` first in a class is a member, so the class runs on past it.
        let bracket = GlobPattern::new("[]**]x").unwrap();
        assert!(bracket.matches("]x"));
        assert!(bracket.matches("*x"));
        // A real `**` beside a class still crosses directories.
        let mixed = GlobPattern::new("[ab]**.go").unwrap();
        assert!(mixed.matches("a.go"));
        assert!(mixed.matches("b/c/d.go"));
        assert!(!mixed.matches("c.go"));
    }

    #[test]
    fn an_error_elsewhere_is_placed_in_the_pattern_as_written() {
        let error = GlobPattern::new("docs/**.md/[ab").unwrap_err();
        let plain = glob::Pattern::new("docs/x*.md/[ab").unwrap_err();
        assert_eq!(error.pos, plain.pos);
        assert_eq!(error.msg, plain.msg);
    }
}
