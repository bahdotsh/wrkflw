use chrono::Local;
use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

// Thread-safe log storage
static LOGS: Lazy<Arc<Mutex<Vec<String>>>> = Lazy::new(|| Arc::new(Mutex::new(Vec::new())));

// Log levels
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Success,
}

impl LogLevel {
    fn prefix(&self) -> &'static str {
        match self {
            LogLevel::Debug => "🔍",
            LogLevel::Info => "ℹ️",
            LogLevel::Warning => "⚠️",
            LogLevel::Error => "❌",
            LogLevel::Success => "✅",
        }
    }
    
    fn name(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Success => "SUCCESS",
        }
    }
}

// Log a message with timestamp and level
pub fn log(level: LogLevel, message: &str) {
    let timestamp = Local::now().format("%H:%M:%S").to_string();
    
    // Always include timestamp in [HH:MM:SS] format to ensure consistency
    let formatted = format!("[{}] {} {}", timestamp, level.prefix(), message);

    if let Ok(mut logs) = LOGS.lock() {
        logs.push(formatted);
    }

    // In verbose mode or when not in TUI, we might still want to print to console
    // This can be controlled by a setting
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

pub fn success(message: &str) {
    log(LogLevel::Success, message);
}
