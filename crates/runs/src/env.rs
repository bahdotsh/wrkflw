//! Every environment variable the run history reads or sets.
//!
//! | Variable | Audience | Meaning |
//! |----------|----------|---------|
//! | `WRKFLW_HOME` | users | Data directory. Runs live in `<home>/runs` |
//! | `WRKFLW_NO_HISTORY` | users | Any value but `0` stops recording |
//! | `WRKFLW_RUNS_KEEP` | users | Runs to keep. Defaults to 500 |
//! | `WRKFLW_AGENT` | users | Who started the run |
//! | `WRKFLW_INTENT` | users | Why the run happened |
//! | `WRKFLW_SESSION` | users | The agent session that started the run |
//! | `AI_AGENT` | set by agents | Agent name, before any `_` |
//! | `CLAUDECODE` | set by Claude Code | Detected as the `claude-code` agent |
//! | `CLAUDE_CODE_SESSION_ID` | set by Claude Code | Session of a `claude-code` run |
//! | `CODEX_THREAD_ID` | set by Codex | Detected as `codex`, and its session |
//! | `CODEX_SANDBOX`, `CODEX_CI` | set by Codex | Detected as the `codex` agent |
//! | `GEMINI_CLI` | set by Gemini CLI | Detected as the `gemini` agent |
//! | `OPENCODE`, `OPENCODE_CLIENT` | set by opencode | Detected as the `opencode` agent |
//! | `CURSOR_AGENT` | set by Cursor's agent | Detected as the `cursor` agent |
//! | `AUGMENT_AGENT` | set by Augment | Detected as the `augment` agent |
//! | `WRKFLW_MCP` | internal | Set on children of `wrkflw mcp serve` |
//! | `WRKFLW_REPLAY_OF` | internal | Set on children of `wrkflw runs replay` |
//!
//! The internal two are process-to-process signalling, not settings.
//! Variables an agent sets are only read, never set.

pub const HOME: &str = "WRKFLW_HOME";
pub const NO_HISTORY: &str = "WRKFLW_NO_HISTORY";
pub const RUNS_KEEP: &str = "WRKFLW_RUNS_KEEP";
pub const AGENT: &str = "WRKFLW_AGENT";
pub const INTENT: &str = "WRKFLW_INTENT";
pub const SESSION: &str = "WRKFLW_SESSION";
pub const AI_AGENT: &str = "AI_AGENT";
pub const CLAUDE_CODE: &str = "CLAUDECODE";
pub const CLAUDE_CODE_SESSION: &str = "CLAUDE_CODE_SESSION_ID";
pub const CODEX_THREAD: &str = "CODEX_THREAD_ID";
pub const CODEX_SANDBOX: &str = "CODEX_SANDBOX";
pub const CODEX_CI: &str = "CODEX_CI";
pub const GEMINI_CLI: &str = "GEMINI_CLI";
pub const OPENCODE: &str = "OPENCODE";
pub const OPENCODE_CLIENT: &str = "OPENCODE_CLIENT";
pub const CURSOR_AGENT: &str = "CURSOR_AGENT";
pub const AUGMENT_AGENT: &str = "AUGMENT_AGENT";
pub const MCP: &str = "WRKFLW_MCP";
pub const REPLAY_OF: &str = "WRKFLW_REPLAY_OF";
