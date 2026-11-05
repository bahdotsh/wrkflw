pub mod symbols;

use chrono::Local;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

// Thread-safe log storage
static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

// Current log level
static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)));

// When true, log() stores messages but does not print to stdout/stderr.
// Enable this while a TUI owns the terminal to prevent display corruption.
static QUIET_MODE: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)));

// When true, every console log line goes to stderr, so stdout can carry a
// machine-readable stream.
static CONSOLE_TO_STDERR: AtomicBool = AtomicBool::new(false);

/// Send all console logging to stderr. Used by `--format json` and the MCP
/// server, where stdout is a protocol.
pub fn set_console_to_stderr(enabled: bool) {
    CONSOLE_TO_STDERR.store(enabled, Ordering::Relaxed);
}

/// Rewrites a message before it is stored or printed. Installed once by the
/// executor, which knows the secret values a run can read.
type Redactor = Box<dyn Fn(&str) -> String + Send + Sync>;

static REDACTOR: OnceLock<Redactor> = OnceLock::new();

/// Install the function every log message passes through before it is
/// stored or printed, so secret values never reach a log. Only the first
/// call wins; later calls return `false` and change nothing.
pub fn set_redactor(redactor: impl Fn(&str) -> String + Send + Sync + 'static) -> bool {
    REDACTOR.set(Box::new(redactor)).is_ok()
}

fn redact(message: &str) -> std::borrow::Cow<'_, str> {
    match REDACTOR.get() {
        Some(redactor) => std::borrow::Cow::Owned(redactor(message)),
        None => std::borrow::Cow::Borrowed(message),
    }
}

// Log levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    fn prefix(&self) -> &'static str {
        match self {
            LogLevel::Debug => symbols::DEBUG,
            LogLevel::Info => symbols::INFO,
            LogLevel::Warning => symbols::WARNING,
            LogLevel::Error => symbols::FAILURE,
        }
    }
}

// Set the current log level
pub fn set_log_level(level: LogLevel) {
    if let Ok(mut current_level) = LOG_LEVEL.lock() {
        *current_level = level;
    }
}

/// Suppress console output (stdout/stderr) from log calls.
/// Messages are still stored and available via `get_logs()`.
/// Call with `true` before entering TUI mode, `false` after leaving.
pub fn set_quiet_mode(quiet: bool) {
    if let Ok(mut q) = QUIET_MODE.lock() {
        *q = quiet;
    }
}

// Get the current log level
pub fn get_log_level() -> LogLevel {
    if let Ok(level) = LOG_LEVEL.lock() {
        *level
    } else {
        // Default to Info if we can't get the lock
        LogLevel::Info
    }
}

// Log a message with timestamp and level
pub fn log(level: LogLevel, message: &str) {
    let timestamp = Local::now().format("%H:%M:%S").to_string();

    // Always include timestamp in [HH:MM:SS] format to ensure consistency
    let message = redact(message);
    let formatted = format!("[{}] {} {}", timestamp, level.prefix(), message);

    if let Ok(mut logs) = LOGS.lock() {
        logs.push(formatted.clone());
    }

    // Print to console unless quiet mode is active (TUI owns the terminal)
    let is_quiet = QUIET_MODE.lock().map(|q| *q).unwrap_or(false);
    if !is_quiet {
        if let Ok(current_level) = LOG_LEVEL.lock() {
            if level >= *current_level {
                match level {
                    LogLevel::Error | LogLevel::Warning => eprintln!("{}", formatted),
                    _ if CONSOLE_TO_STDERR.load(Ordering::Relaxed) => {
                        eprintln!("{}", formatted)
                    }
                    _ => println!("{}", formatted),
                }
            }
        }
    }
}

// Get all logs
pub fn get_logs() -> Vec<String> {
    if let Ok(logs) = LOGS.lock() {
        logs.clone()
    } else {
        // If we can't access logs, return an error message with timestamp
        let timestamp = Local::now().format("%H:%M:%S").to_string();
        vec![format!(
            "[{}] {} Error accessing logs",
            timestamp,
            symbols::FAILURE
        )]
    }
}

// Clear all logs
#[allow(dead_code)]
pub fn clear_logs() {
    if let Ok(mut logs) = LOGS.lock() {
        logs.clear();
    }
}

// Convenience functions for different log levels
#[allow(dead_code)]
pub fn debug(message: &str) {
    log(LogLevel::Debug, message);
}

pub fn info(message: &str) {
    log(LogLevel::Info, message);
}

pub fn warning(message: &str) {
    log(LogLevel::Warning, message);
}

pub fn error(message: &str) {
    log(LogLevel::Error, message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redactor_masks_stored_logs() {
        assert!(set_redactor(|m| m.replace("hunter2-sentinel", "***")));
        set_quiet_mode(true);
        info("password=hunter2-sentinel");
        let logs = get_logs();
        assert!(logs.iter().any(|l| l.ends_with("password=***")));
        assert!(!logs.iter().any(|l| l.contains("hunter2-sentinel")));
        // The first redactor stays in place.
        assert!(!set_redactor(|m| m.to_string()));
    }
}
