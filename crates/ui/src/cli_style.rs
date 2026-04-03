// CLI output styling using the colored crate
// Used for non-TUI terminal output (validate, execute, list commands)

use colored::Colorize;
use wrkflw_logging::symbols;

pub fn success(text: &str) -> String {
    format!("{} {}", symbols::SUCCESS.green(), text)
}

pub fn error(text: &str) -> String {
    format!("{} {}", symbols::FAILURE.red(), text)
}

pub fn warning(text: &str) -> String {
    format!("{} {}", symbols::WARNING.yellow(), text)
}

pub fn info(text: &str) -> String {
    format!("{} {}", symbols::INFO.cyan(), text)
}

pub fn skipped(text: &str) -> String {
    format!("{} {}", symbols::SKIPPED.dimmed(), text)
}

pub fn section(text: &str) -> String {
    format!("\n{}", text.bold().underline())
}

pub fn separator() -> String {
    format!("{}", symbols::HRULE.repeat(40).dimmed())
}

pub fn dim(text: &str) -> String {
    format!("{}", text.dimmed())
}

pub fn job_success(name: &str) -> String {
    format!(
        "{} Job succeeded: {}",
        symbols::SUCCESS.green(),
        name.bold()
    )
}

pub fn job_failure(name: &str) -> String {
    format!("{} Job failed: {}", symbols::FAILURE.red(), name.bold())
}

pub fn job_skipped(name: &str) -> String {
    format!("{} Job skipped: {}", symbols::SKIPPED.dimmed(), name.bold())
}

pub fn step_success(name: &str) -> String {
    format!("  {} {}", symbols::SUCCESS.green(), name)
}

pub fn step_failure(name: &str) -> String {
    format!("  {} {}", symbols::FAILURE.red(), name)
}

pub fn step_skipped(name: &str) -> String {
    format!(
        "  {} {} {}",
        symbols::SKIPPED.dimmed(),
        name,
        "(skipped)".dimmed()
    )
}

pub fn indent(text: &str) -> String {
    format!("    {}", text.dimmed())
}

pub fn key_value(key: &str, value: &str) -> String {
    format!("{} {}", format!("{}:", key).cyan(), value)
}
