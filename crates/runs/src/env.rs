//! Every environment variable the run history reads or sets.
//!
//! | Variable | Audience | Meaning |
//! |----------|----------|---------|
//! | `WRKFLW_HOME` | users | Data directory. Runs live in `<home>/runs` |
//! | `WRKFLW_NO_HISTORY` | users | Any value but `0` stops recording |
//! | `WRKFLW_RUNS_KEEP` | users | Runs to keep. Defaults to 500 |
//! | `WRKFLW_AGENT` | users | Who started the run |
//! | `WRKFLW_INTENT` | users | Why the run happened |
//! | `CLAUDECODE` | set by Claude Code | Detected as the `claude-code` agent |
//! | `WRKFLW_MCP` | internal | Set on children of `wrkflw mcp serve` |
//! | `WRKFLW_REPLAY_OF` | internal | Set on children of `wrkflw runs replay` |
//!
//! The internal two are process-to-process signalling, not settings.

pub const HOME: &str = "WRKFLW_HOME";
pub const NO_HISTORY: &str = "WRKFLW_NO_HISTORY";
pub const RUNS_KEEP: &str = "WRKFLW_RUNS_KEEP";
pub const AGENT: &str = "WRKFLW_AGENT";
pub const INTENT: &str = "WRKFLW_INTENT";
pub const CLAUDE_CODE: &str = "CLAUDECODE";
pub const MCP: &str = "WRKFLW_MCP";
pub const REPLAY_OF: &str = "WRKFLW_REPLAY_OF";
