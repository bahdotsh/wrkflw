//! Keeps secret values out of `wrkflw_logging`.
//!
//! Step output is masked per run, but runtimes also log commands and
//! environments, and those lines go through the global logger. Every value
//! a run can read is registered here as well, and the logger masks each
//! message with this process-wide masker before storing or printing it.
//! Values are never unregistered: masking a value a later run no longer
//! uses is harmless, while unmasking one too early would leak it.

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

/// Mask each of `values` in every log message from now on.
pub(crate) fn register_all<'a>(values: impl IntoIterator<Item = &'a String>) {
    for value in values {
        register(value);
    }
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
}
