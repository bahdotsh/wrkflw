use chrono::Local;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

// Thread-safe log storage
static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

// Current log level
static LOG_LEVEL: Lazy<Arc<Mutex<LogLevel>>> = Lazy::new(|| Arc::new(Mutex::new(LogLevel::Info)));

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
            LogLevel::Debug => "🔍",
            LogLevel::Info => "ℹ️",
            LogLevel::Warning => "⚠️",
            LogLevel::Error => "❌",
        }
    }
}

// Set the current log level
pub fn set_log_level(level: LogLevel) {
    if let Ok(mut current_level) = LOG_LEVEL.lock() {
        *current_level = level;
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

    // Print to console if the message level is >= the current log level
    // This ensures Debug messages only show up when the Debug level is set
    if let Ok(current_level) = LOG_LEVEL.lock() {
        if level >= *current_level {
            // Print to stdout/stderr based on level
            match level {
                LogLevel::Error | LogLevel::Warning => eprintln!("{}", formatted),
                _ => println!("{}", formatted),
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
        vec![format!("[{}] ❌ Error accessing logs", timestamp)]
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
