//! Keeps secret values out of `wrkflw_logging`.
//!
//! Step output is masked per run, but runtimes also log commands and
//! environments, and those lines go through the global logger. Every value
//! a run can read is registered here as well, and the logger masks each
//! message with this process-wide masker before storing or printing it.
//! Values are never unregistered: masking a value a later run no longer
//! uses is harmless, while unmasking one too early would leak it. Because
//! of that, only values that are, or plausibly are, credentials belong here:
//! a value such as `main` would otherwise be masked in every later log line
//! of the process.

use once_cell::sync::Lazy;
use std::sync::Once;
use wrkflw_secrets::SecretMasker;

static LOG_MASKER: Lazy<SecretMasker> = Lazy::new(SecretMasker::new);
static INSTALL: Once = Once::new();

/// Mask `value` in every log message from now on.
pub(crate) fn register(value: &str) {
    if value.is_empty() {
        return;
    }
    LOG_MASKER.add_secret(value);
    INSTALL.call_once(|| {
        wrkflw_logging::set_redactor(|message| LOG_MASKER.mask(message));
    });
}

/// `text` with every registered value masked.
pub(crate) fn mask(text: &str) -> String {
    LOG_MASKER.mask(text)
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
    let named = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| CREDENTIAL_NAME_PARTS.iter().any(|p| part.contains(p)));
    named || value.chars().count() >= UNNAMED_CREDENTIAL_MIN_LEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_values_are_masked_in_logs() {
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
    }

    #[test]
    fn plain_variables_stay_readable_in_logs() {
        let name = "TARGET_BRANCH".to_string();
        let value = "plainvar-7c1".to_string();
        let token_name = "RELEASE_TOKEN".to_string();
        let token = "tokvar-sentinel-3b9".to_string();
        register_variables([(&name, &value), (&token_name, &token)]);
        wrkflw_logging::set_quiet_mode(true);
        wrkflw_logging::info("vars plainvar-7c1 tokvar-sentinel-3b9");
        let logs = wrkflw_logging::get_logs();
        assert!(logs.iter().any(|l| l.ends_with("vars plainvar-7c1 ***")));
    }
}
