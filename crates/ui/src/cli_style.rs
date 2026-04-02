// CLI output styling using the colored crate
// Used for non-TUI terminal output (validate, execute, list commands)

use colored::Colorize;

pub fn success(text: &str) -> String {
    format!("{} {}", "\u{2714}".green(), text)
}

pub fn error(text: &str) -> String {
    format!("{} {}", "\u{2716}".red(), text)
}

pub fn warning(text: &str) -> String {
    format!("{} {}", "\u{26A0}".yellow(), text)
}

pub fn info(text: &str) -> String {
    format!("{} {}", "\u{25CF}".cyan(), text)
}

pub fn skipped(text: &str) -> String {
    format!("{} {}", "\u{2298}".dimmed(), text)
}

pub fn section(text: &str) -> String {
    format!("\n{}", text.bold().underline())
}

pub fn separator() -> String {
    format!("{}", "\u{2500}".repeat(40).dimmed())
}

pub fn dim(text: &str) -> String {
    format!("{}", text.dimmed())
}

pub fn job_success(name: &str) -> String {
    format!("{} Job succeeded: {}", "\u{2714}".green(), name.bold())
}

pub fn job_failure(name: &str) -> String {
    format!("{} Job failed: {}", "\u{2716}".red(), name.bold())
}

pub fn job_skipped(name: &str) -> String {
    format!("{} Job skipped: {}", "\u{2298}".dimmed(), name.bold())
}

pub fn step_success(name: &str) -> String {
    format!("  {} {}", "\u{2714}".green(), name)
}

pub fn step_failure(name: &str) -> String {
    format!("  {} {}", "\u{2716}".red(), name)
}

pub fn step_skipped(name: &str) -> String {
    format!("  {} {} {}", "\u{2298}".dimmed(), name, "(skipped)".dimmed())
}

pub fn indent(text: &str) -> String {
    format!("    {}", text.dimmed())
}

pub fn key_value(key: &str, value: &str) -> String {
    format!("{} {}", format!("{}:", key).cyan(), value)
}
