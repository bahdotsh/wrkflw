//! Keeps secret values out of `wrkflw_logging`.
//!
//! Step output is masked per run, but runtimes also log commands and
//! environments, and those lines go through the global logger. Every value
//! a run can read is registered here as well, and the logger masks each
//! message with this masker before storing or printing it.
//!
//! The masker belongs to runs, not to the process: it starts empty when a
//! run starts while no other run is going, and keeps every value until
//! then, so what a run logs after it finishes (its summary, cleanup) stays
//! masked. A process that runs many workflows, as the TUI does, so never
//! masks a value it learned from one run in the logs of an unrelated later
//! one: an `::add-mask::main` would otherwise turn every later "main" into
//! `***`. Overlapping runs share the masker until the last one ends.
//!
//! Plain `--variable` values stay out even within a run, since they are
//! mostly settings such as `BRANCH=main`; see [`register_variables`].

use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex, Once, RwLock};
use wrkflw_secrets::SecretMasker;

static LOG_MASKER: Lazy<RwLock<Arc<SecretMasker>>> =
    Lazy::new(|| RwLock::new(Arc::new(SecretMasker::new())));
/// How many runs are going; the masker only starts over at zero.
static ACTIVE_RUNS: Mutex<usize> = Mutex::new(0);
static INSTALL: Once = Once::new();

fn current() -> Arc<SecretMasker> {
    LOG_MASKER.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A run in progress, for as long as it is held. See the module docs.
#[must_use = "the run's values are only kept while the scope is held"]
pub(crate) struct RunScope(());

/// Start a run: the masker starts over unless another run is going.
pub(crate) fn begin_run() -> RunScope {
    let mut active = ACTIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner());
    if *active == 0 {
        *LOG_MASKER.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(SecretMasker::new());
    }
    *active += 1;
    RunScope(())
}

impl Drop for RunScope {
    fn drop(&mut self) {
        let mut active = ACTIVE_RUNS.lock().unwrap_or_else(|e| e.into_inner());
        *active = active.saturating_sub(1);
    }
}

/// Mask `value` in every log message until the next run starts.
pub(crate) fn register(value: &str) {
    if value.is_empty() {
        return;
    }
    current().add_secret(value);
    INSTALL.call_once(|| {
        let installed = wrkflw_logging::set_redactor(|message| current().mask(message));
        // Another redactor would mean these values are never applied; say
        // so loudly rather than let the logs carry them unnoticed.
        if !installed {
            wrkflw_logging::error(
                "Another log redactor is installed, so secret values may reach the logs",
            );
        }
    });
}

/// `text` with every registered value masked.
pub(crate) fn mask(text: &str) -> String {
    current().mask(text)
}

/// `args` as a debug list for a log line, each masked first.
///
/// The logger masks a message by the values themselves, but `{:?}` escapes
/// quotes and backslashes, so a secret holding either would be logged in a
/// form the logger can't recognise. Masking before formatting avoids that.
pub(crate) fn args_for_log<S: AsRef<str>>(args: &[S]) -> String {
    let masked: Vec<String> = args.iter().map(|arg| mask(arg.as_ref())).collect();
    format!("{masked:?}")
}

/// The names of `vars`, for a log line: values can be secrets, and the
/// names are what tells a reader which variables a step got.
pub(crate) fn env_names_for_log<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    names.into_iter().collect::<Vec<_>>().join(", ")
}

/// Mask each of `values` in every log message from now on.
pub(crate) fn register_all<'a>(values: impl IntoIterator<Item = &'a String>) {
    for value in values {
        register(value);
    }
}

/// Values shorter than this are only masked when their name says they are
/// a credential. Tokens and keys are longer; short values are mostly words
/// such as branch or environment names.
const UNNAMED_CREDENTIAL_MIN_LEN: usize = 16;

/// Name parts that mark a variable as a credential, whatever its length.
const CREDENTIAL_NAME_PARTS: &[&str] = &[
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASS",
    "PWD",
    "KEY",
    "AUTH",
    "CREDENTIAL",
    "PRIVATE",
    "CERT",
    "COOKIE",
    "SESSION",
    "SIGNATURE",
    "DSN",
];

/// Register user variables (`--variable NAME=value`) whose name or value
/// suggests a credential. Plain settings stay readable in the logs; step
/// output is still masked for every variable by the run's own masker.
pub(crate) fn register_variables<'a>(
    variables: impl IntoIterator<Item = (&'a String, &'a String)>,
) {
    for (name, value) in variables {
        if looks_like_credential(name, value) {
            register(value);
        }
    }
}

fn looks_like_credential(name: &str, value: &str) -> bool {
    let name = name.to_ascii_uppercase();
    // A part names a credential when it is one of the words, or ends with
    // one (`APIKEY`, `GHTOKEN`), plural or not. Matching anywhere in the
    // part would take `AUTHOR` or `KEYBOARD` for credentials too.
    let names_one = |part: &str| CREDENTIAL_NAME_PARTS.iter().any(|p| part.ends_with(p));
    let named = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| names_one(part) || part.strip_suffix('S').is_some_and(names_one));
    named || value.chars().count() >= UNNAMED_CREDENTIAL_MIN_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_values_are_masked_in_logs() {
        let _run = begin_run();
        register("log-redaction-sentinel-7f2c");
        wrkflw_logging::set_quiet_mode(true);
        wrkflw_logging::info("API_KEY=log-redaction-sentinel-7f2c");
        let logs = wrkflw_logging::get_logs();
        assert!(!logs
            .iter()
            .any(|l| l.contains("log-redaction-sentinel-7f2c")));
        assert!(logs.iter().any(|l| l.ends_with("API_KEY=***")));
    }

    #[test]
    fn credential_variables_are_recognised_by_name_or_length() {
        assert!(looks_like_credential("DEPLOY_TOKEN", "abc"));
        assert!(looks_like_credential("db_password", "hunter2"));
        assert!(looks_like_credential("AWS_SECRET_ACCESS_KEY", "x1y"));
        assert!(looks_like_credential("X", "glpat-0123456789abcdef"));
        assert!(!looks_like_credential("BRANCH", "main"));
        assert!(!looks_like_credential("ENVIRONMENT", "production"));
        assert!(!looks_like_credential("CI_DEBUG", "true"));
        assert!(looks_like_credential("APIKEY", "abc"));
        assert!(looks_like_credential("GH_TOKENS", "abc"));
        assert!(!looks_like_credential("AUTHOR", "main"));
        assert!(!looks_like_credential("KEYBOARD", "us"));
        assert!(!looks_like_credential("S", "x"));
        assert!(looks_like_credential("DB_PASS", "x"));
        assert!(looks_like_credential("ACCESS_KEYS", "x"));
    }

    #[test]
    fn debug_formatted_arguments_are_masked_before_escaping() {
        let _run = begin_run();
        let secret = r#"quo"te\back-sentinel-2e"#;
        register(secret);
        let line = args_for_log(&["psql", &format!("--password={secret}")]);
        assert_eq!(line, r#"["psql", "--password=***"]"#);
        assert_eq!(
            env_names_for_log(["PGPASSWORD", "HOME"]),
            "PGPASSWORD, HOME"
        );
    }

    #[test]
    fn a_value_from_one_run_is_not_masked_in_the_next() {
        // Other tests in this binary start runs too; while this one holds a
        // scope, none of them can start the masker over.
        let first = begin_run();
        let second_overlapping = begin_run();
        register("run-scoped-sentinel-41d");
        drop(first);
        assert_eq!(mask("x run-scoped-sentinel-41d"), "x ***");
        drop(second_overlapping);
        // Still masked after the run ends, for its summary and cleanup.
        assert_eq!(mask("x run-scoped-sentinel-41d"), "x ***");
        let _next = begin_run();
        // Another test may hold a run, which keeps the value; when none
        // does, the next run starts over.
        if *ACTIVE_RUNS.lock().unwrap() == 1 {
            assert_eq!(
                mask("x run-scoped-sentinel-41d"),
                "x run-scoped-sentinel-41d"
            );
        }
    }

    #[test]
    fn plain_variables_stay_readable_in_logs() {
        let name = "TARGET_BRANCH".to_string();
        let value = "plainvar-7c1".to_string();
        let token_name = "RELEASE_TOKEN".to_string();
        let token = "tokvar-sentinel-3b9".to_string();
        let _run = begin_run();
        register_variables([(&name, &value), (&token_name, &token)]);
        wrkflw_logging::set_quiet_mode(true);
        wrkflw_logging::info("vars plainvar-7c1 tokvar-sentinel-3b9");
        let logs = wrkflw_logging::get_logs();
        assert!(logs.iter().any(|l| l.ends_with("vars plainvar-7c1 ***")));
    }
}
