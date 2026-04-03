pub mod symbols;

use chrono::Local;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

// Thread-safe log storage
static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

// Current log level
static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)));

// When true, log() stores messages but does not print to stdout/stderr.
// Enable this while a TUI owns the terminal to prevent display corruption.
static QUIET_MODE: Lazy<Arc<Mutex<bool>>> = Lazy::new(|| Arc::new(Mutex::new(false)));

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
