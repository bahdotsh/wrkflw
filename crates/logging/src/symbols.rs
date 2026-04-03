// Shared Unicode symbols for consistent terminal output across all crates.
//
// These are single-cell-width Unicode characters that replace the mixed emoji
// (✅❌⏭🔒) which render at double width in many terminals.

// ── Status symbols ─────────────────────────────────────────────────

pub const SUCCESS: &str = "\u{2714}"; // ✔
pub const FAILURE: &str = "\u{2716}"; // ✖
pub const RUNNING: &str = "\u{25C9}"; // ◉
pub const SKIPPED: &str = "\u{2298}"; // ⊘
pub const NOT_STARTED: &str = "\u{25CB}"; // ○
pub const WARNING: &str = "\u{26A0}"; // ⚠
pub const INFO: &str = "\u{25CF}"; // ●
pub const DEBUG: &str = "\u{25E6}"; // ◦
pub const GEAR: &str = "\u{2699}"; // ⚙

// ── UI chrome ──────────────────────────────────────────────────────

pub const LOCK: &str = "\u{26BF}"; // ⚿
pub const BLOCKED: &str = "\u{26D4}"; // ⛔
pub const SEPARATOR: &str = "\u{2502}"; // │
pub const ARROW: &str = "\u{2192}"; // →
pub const HRULE: &str = "\u{2500}"; // ─

// ── TUI-only symbols (re-exported by theme.rs) ────────────────────

pub const SELECTED: &str = "\u{25B8} "; // ▸ (with trailing space for highlight_symbol)
pub const CHECKBOX_ON: &str = "[\u{2714}]"; // [✔]
pub const CHECKBOX_OFF: &str = "[ ]";
pub const TAB_DIVIDER: &str = " \u{2502} "; // │

// Braille spinner frames for running animation
pub const SPINNER: &[&str] = &[
    "\u{280B}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283C}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280F}",
];
