//! `wrkflw mcp install`: register `wrkflw mcp serve` with coding agents.
//!
//! Claude Code and Codex are changed through their own CLIs. Claude Code
//! rewrites `~/.claude.json` while it runs and Codex keeps TOML, so editing
//! either file here would race it or lose formatting. The other clients keep
//! plain JSON, which is edited in place with key order kept.

use crate::OutputFormat;
use clap::ValueEnum;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use wrkflw_runs::SCHEMA_VERSION;

const SERVER: &str = "wrkflw";
const SERVE_ARGS: [&str; 2] = ["mcp", "serve"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Client {
    ClaudeCode,
    Codex,
    Cursor,
    Windsurf,
    Vscode,
    Gemini,
    Opencode,
    ClaudeDesktop,
}

impl Client {
    /// The name on the command line, such as `claude-code`.
    fn name(self) -> String {
        self.to_possible_value()
            .expect("no client is skipped")
            .get_name()
            .to_string()
    }
}

/// How a client's config is changed.
enum Method {
    /// Through the client's CLI: `program mcp add [scope] wrkflw -- ...`.
    Cli {
        program: &'static str,
        scope: &'static [&'static str],
        /// Whether a `wrkflw` server is registered, and whether it launches `command`.
        existing: fn(&Dirs, &str) -> Option<bool>,
    },
    /// By editing `path`, adding `entry` under `key`.
    Json {
        path: PathBuf,
        key: &'static str,
        entry: Value,
    },
}

/// Where config lives. Separate so tests can point it at a temporary directory.
struct Dirs {
    home: PathBuf,
    /// The platform's per-user config directory, e.g. `~/Library/Application Support`.
    config: PathBuf,
    /// Where Claude Code keeps `.claude.json`.
    claude: PathBuf,
}

impl Dirs {
    fn current() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self {
            claude: claude_dir(&home, std::env::var_os("CLAUDE_CONFIG_DIR")),
            config: dirs::config_dir()?,
            home,
        })
    }
}

/// `CLAUDE_CONFIG_DIR` moves `.claude.json` out of the home directory.
fn claude_dir(home: &Path, configured: Option<std::ffi::OsString>) -> PathBuf {
    configured
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf())
}

/// The command a client should launch. `wrkflw` when it is on PATH, so the
/// entry survives upgrades. Claude Desktop starts servers without the shell's
/// PATH, so it always gets an absolute path.
#[derive(Clone)]
struct Launch {
    command: String,
    absolute: String,
}

impl Launch {
    fn detect() -> Self {
        let on_path = which::which(SERVER).ok();
        let absolute = on_path
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| SERVER.to_string());
        let command = if on_path.is_some() {
            SERVER.to_string()
        } else {
            absolute.clone()
        };
        Self { command, absolute }
    }

    fn for_client(&self, client: Client) -> &str {
        if client == Client::ClaudeDesktop {
            &self.absolute
        } else {
            &self.command
        }
    }
}

fn args() -> Vec<String> {
    SERVE_ARGS.iter().map(|arg| arg.to_string()).collect()
}

fn method(client: Client, dirs: &Dirs, command: &str) -> Method {
    let standard = json!({"command": command, "args": SERVE_ARGS});
    let file = |path: PathBuf, key, entry| Method::Json { path, key, entry };
    match client {
        Client::ClaudeCode => Method::Cli {
            program: "claude",
            scope: &["--scope", "user"],
            existing: claude_code_existing,
        },
        Client::Codex => Method::Cli {
            program: "codex",
            scope: &[],
            existing: |_, command| codex_existing(command),
        },
        Client::Cursor => file(dirs.home.join(".cursor/mcp.json"), "mcpServers", standard),
        Client::Windsurf => file(
            dirs.home.join(".codeium/windsurf/mcp_config.json"),
            "mcpServers",
            standard,
        ),
        Client::Vscode => file(
            dirs.config.join("Code/User/mcp.json"),
            "servers",
            json!({"type": "stdio", "command": command, "args": SERVE_ARGS}),
        ),
        Client::Gemini => file(
            dirs.home.join(".gemini/settings.json"),
            "mcpServers",
            standard,
        ),
        // opencode reads ~/.config on every platform, not the platform's config dir.
        Client::Opencode => file(
            opencode_config(&dirs.home.join(".config/opencode")),
            "mcp",
            json!({"type": "local", "command": [command, "mcp", "serve"], "enabled": true}),
        ),
        Client::ClaudeDesktop => file(
            dirs.config.join("Claude/claude_desktop_config.json"),
            "mcpServers",
            standard,
        ),
    }
}

/// `opencode.json`, or `opencode.jsonc` when only that one exists, so the entry
/// goes in the file opencode reads instead of a new file beside it.
fn opencode_config(dir: &Path) -> PathBuf {
    let json = dir.join("opencode.json");
    let jsonc = dir.join("opencode.jsonc");
    if !json.exists() && jsonc.exists() {
        jsonc
    } else {
        json
    }
}

/// A client is installed when its CLI is on PATH or its config directory exists.
fn detected(method: &Method) -> bool {
    match method {
        Method::Cli { program, .. } => which::which(program).is_ok(),
        Method::Json { path, .. } => path.parent().is_some_and(Path::is_dir),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Added,
    Replaced,
    Unchanged,
    /// Registered with a different command. Left alone without `--force`.
    Differs,
    WouldAdd,
    WouldReplace,
    NotFound,
    Failed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Added => "added",
            Status::Replaced => "replaced",
            Status::Unchanged => "unchanged",
            Status::Differs => "differs",
            Status::WouldAdd => "would_add",
            Status::WouldReplace => "would_replace",
            Status::NotFound => "not_found",
            Status::Failed => "failed",
        }
    }

    /// Whether the client was left without the wanted entry. `Differs`
    /// counts, so a script can tell that nothing was registered. Under
    /// `--dry-run` it counts too: the real run would leave the entry alone.
    fn is_failure(self) -> bool {
        matches!(self, Status::Failed | Status::NotFound | Status::Differs)
    }

    /// Why the client was left alone, when the status alone doesn't say.
    fn reason(self) -> Option<String> {
        (self == Status::Differs)
            .then(|| "registered with another command, pass --force to replace it".to_string())
    }
}

struct Outcome {
    client: Client,
    status: Status,
    /// The file changed, or the CLI used.
    target: String,
    detail: Option<String>,
}

/// What to do with an entry that is already there.
#[derive(Clone, Copy)]
struct Policy {
    dry_run: bool,
    force: bool,
}

impl Policy {
    /// The outcome for an entry that is missing (`None`), the same, or
    /// different, and whether to write it.
    fn settle(self, existing: Option<bool>) -> (Status, bool) {
        match existing {
            Some(true) => (Status::Unchanged, false),
            Some(false) if !self.force => (Status::Differs, false),
            Some(false) if self.dry_run => (Status::WouldReplace, false),
            Some(false) => (Status::Replaced, true),
            None if self.dry_run => (Status::WouldAdd, false),
            None => (Status::Added, true),
        }
    }
}

/// JSON with object key order kept, so rewriting a config keeps its layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum Ordered {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Ordered>),
    Object(IndexMap<String, Ordered>),
}

/// The keys that decide what an entry launches. Others, such as `env` or
/// `disabled`, are the user's and are kept.
const LAUNCH_KEYS: [&str; 2] = ["command", "args"];

/// Whether `current` launches what `wanted` does.
fn same_launch(current: &Ordered, wanted: &IndexMap<String, Ordered>) -> bool {
    let Ordered::Object(current) = current else {
        return false;
    };
    LAUNCH_KEYS
        .iter()
        .all(|key| current.get(*key) == wanted.get(*key))
}

/// Add `entry` as server `wrkflw` under `key` in the config `text`.
///
/// An existing entry is compared by what it launches, and on replace the
/// wanted keys are merged into it, so keys the user added survive `--force`.
/// Returns the status and, when the file should change, its new text.
fn upsert_json(
    text: Option<&str>,
    key: &str,
    entry: &Value,
    policy: Policy,
) -> Result<(Status, Option<String>), String> {
    let mut root = match text.filter(|text| !text.trim().is_empty()) {
        None => IndexMap::new(),
        Some(text) => match serde_json::from_str::<Ordered>(text) {
            Ok(Ordered::Object(root)) => root,
            Ok(_) => return Err("the file isn't a JSON object".to_string()),
            Err(e) => {
                return Err(format!(
                    "can't parse it as plain JSON ({e}). Comments aren't supported; add the entry by hand"
                ))
            }
        },
    };
    let Ordered::Object(entry) =
        serde_json::from_value(entry.clone()).map_err(|e: serde_json::Error| e.to_string())?
    else {
        return Err("the entry isn't a JSON object".to_string());
    };
    let servers = root
        .entry(key.to_string())
        .or_insert_with(|| Ordered::Object(IndexMap::new()));
    let Ordered::Object(servers) = servers else {
        return Err(format!("`{key}` isn't a JSON object"));
    };
    let existing = servers
        .get(SERVER)
        .map(|current| same_launch(current, &entry));
    let (status, write) = policy.settle(existing);
    if !write {
        return Ok((status, None));
    }
    match servers.get_mut(SERVER) {
        // `extend` keeps the position of keys already there.
        Some(Ordered::Object(current)) => current.extend(entry),
        _ => {
            servers.insert(SERVER.to_string(), Ordered::Object(entry));
        }
    }
    let indent = text.map_or(DEFAULT_INDENT, indent_of);
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    Ordered::Object(root)
        .serialize(&mut serializer)
        .map_err(|e| e.to_string())?;
    let mut text = String::from_utf8(out).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok((status, Some(text)))
}

const DEFAULT_INDENT: &str = "  ";

/// The indentation of the first indented line, so a rewrite keeps the file's style.
fn indent_of(text: &str) -> &str {
    text.lines()
        .find_map(|line| {
            let rest = line.trim_start_matches([' ', '\t']);
            let indent = &line[..line.len() - rest.len()];
            (!indent.is_empty() && !rest.is_empty()).then_some(indent)
        })
        .unwrap_or(DEFAULT_INDENT)
}

/// The file `path` finally points to. Dotfile managers symlink these configs,
/// and renaming over the link would detach it from the managed copy. Works for
/// a link whose target doesn't exist yet, unlike `canonicalize`.
fn resolve_links(path: &Path) -> PathBuf {
    let mut path = path.to_path_buf();
    // Bounded, so a link cycle can't loop forever.
    for _ in 0..32 {
        let Ok(target) = std::fs::read_link(&path) else {
            break;
        };
        path = path.parent().unwrap_or(Path::new("")).join(target);
    }
    path
}

/// Replace the file `path` resolves to, keeping its permissions. A new file
/// gets the usual 0644 rather than the temporary file's 0600.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = resolve_links(path);
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    match std::fs::metadata(&path) {
        Ok(meta) => file.as_file().set_permissions(meta.permissions())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.as_file()
                    .set_permissions(std::fs::Permissions::from_mode(0o644))?;
            }
        }
        Err(e) => return Err(e),
    }
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|e| e.error)?;
    Ok(())
}

fn install_json(path: &Path, key: &str, entry: &Value, policy: Policy) -> (Status, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return (Status::Failed, Some(e.to_string())),
    };
    match upsert_json(text.as_deref(), key, entry, policy) {
        Err(e) => (Status::Failed, Some(e)),
        Ok((status, None)) => (status, None),
        Ok((status, Some(text))) => match write_atomically(path, &text) {
            Ok(()) => (status, None),
            Err(e) => (Status::Failed, Some(e.to_string())),
        },
    }
}

fn run(program: &str, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))
}

fn check(program: &str, args: &[&str]) -> Result<(), String> {
    let output = run(program, args)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "`{program} {}` failed: {}",
        args.join(" "),
        stderr.trim()
    ))
}

/// Whether Claude Code has a user-scoped `wrkflw` server, and whether it matches.
///
/// Read from `.claude.json` rather than `claude mcp get`, which also reports
/// servers from the current project's `.mcp.json`.
fn claude_code_existing(dirs: &Dirs, command: &str) -> Option<bool> {
    let text = std::fs::read_to_string(dirs.claude.join(".claude.json")).ok()?;
    let config: Value = serde_json::from_str(&text).ok()?;
    let server = config.get("mcpServers")?.get(SERVER)?;
    Some(server["command"] == command && server["args"] == json!(SERVE_ARGS))
}

fn codex_existing(command: &str) -> Option<bool> {
    let output = run("codex", &["mcp", "get", SERVER, "--json"]).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(codex_matches(&output.stdout, command))
}

/// Whether `codex mcp get --json` output describes a server launching `command`.
fn codex_matches(stdout: &[u8], command: &str) -> bool {
    let Ok(server) = serde_json::from_slice::<Value>(stdout) else {
        return false;
    };
    let transport = &server["transport"];
    transport["command"] == command && transport["args"] == json!(SERVE_ARGS)
}

fn install_cli(
    program: &str,
    scope: &[&str],
    existing: Option<bool>,
    command: &str,
    policy: Policy,
) -> (Status, Option<String>) {
    let (status, write) = policy.settle(existing);
    if !write {
        return (status, None);
    }
    if status == Status::Replaced {
        // A failed remove shows up as the add failing on the duplicate name.
        let mut remove = vec!["mcp", "remove"];
        remove.extend(scope);
        remove.push(SERVER);
        let _ = check(program, &remove);
    }
    let mut add = vec!["mcp", "add"];
    add.extend(scope);
    add.extend([SERVER, "--", command]);
    add.extend(SERVE_ARGS);
    match check(program, &add) {
        Ok(()) => (status, None),
        Err(e) => (Status::Failed, Some(e)),
    }
}

fn install_one(
    client: Client,
    dirs: &Dirs,
    launch: &Launch,
    policy: Policy,
    named: bool,
) -> Option<Outcome> {
    let command = launch.for_client(client);
    let method = method(client, dirs, command);
    let target = match &method {
        Method::Cli { program, .. } => program.to_string(),
        Method::Json { path, .. } => path.to_string_lossy().into_owned(),
    };
    if !detected(&method) {
        // Only report a client nobody asked for if it was named.
        return named.then(|| Outcome {
            client,
            status: Status::NotFound,
            detail: Some(match &method {
                Method::Cli { program, .. } => format!("`{program}` isn't on PATH"),
                Method::Json { path, .. } => {
                    format!("{} doesn't exist", path.parent().unwrap_or(path).display())
                }
            }),
            target,
        });
    }
    let (status, detail) = match &method {
        Method::Cli {
            program,
            scope,
            existing,
        } => install_cli(program, scope, existing(dirs, command), command, policy),
        Method::Json { path, key, entry } => install_json(path, key, entry, policy),
    };
    Some(Outcome {
        client,
        status,
        target,
        detail: detail.or_else(|| status.reason()),
    })
}

fn report_json(outcomes: &[Outcome], launch: &Launch) -> Value {
    let clients: Vec<Value> = outcomes
        .iter()
        .map(|outcome| {
            json!({
                "client": outcome.client.name(),
                "status": outcome.status.as_str(),
                "target": outcome.target,
                "detail": outcome.detail,
            })
        })
        .collect();
    json!({
        "schema_version": SCHEMA_VERSION,
        "type": "mcp_install",
        "command": launch.command,
        "args": args(),
        "clients": clients,
    })
}

fn report(outcomes: &[Outcome], format: OutputFormat, launch: &Launch, policy: Policy) {
    if format.is_json() {
        wrkflw_runs::emit_line(&report_json(outcomes, launch));
        return;
    }
    if outcomes.is_empty() {
        let supported: Vec<String> = Client::value_variants().iter().map(|c| c.name()).collect();
        println!(
            "No supported coding agent found. Supported: {}.",
            supported.join(", ")
        );
        return;
    }
    for outcome in outcomes {
        let status = outcome.status.as_str().replace('_', " ");
        let detail = outcome
            .detail
            .as_ref()
            .map(|detail| format!(": {detail}"))
            .unwrap_or_default();
        println!(
            "{:<15} {status} ({}){detail}",
            outcome.client.name(),
            outcome.target
        );
    }
    if outcomes
        .iter()
        .any(|o| matches!(o.status, Status::Added | Status::Replaced))
    {
        println!("Restart the agent to load the wrkflw MCP server.");
    }
    if launch.command != SERVER {
        let registered = if policy.dry_run {
            "would be registered"
        } else {
            "was registered"
        };
        println!(
            "wrkflw isn't on PATH, so {} {registered}. Rerun this after installing it.",
            launch.command
        );
    }
}

/// Register the server with `clients`, or with every detected client when empty.
/// Exits 1 if none was found, or any client failed or kept a different entry,
/// including under `dry_run`, where it means the real run would do the same.
pub(crate) fn install(clients: &[Client], dry_run: bool, force: bool, format: OutputFormat) {
    let Some(dirs) = Dirs::current() else {
        eprintln!("Error: no home directory");
        std::process::exit(1);
    };
    let launch = Launch::detect();
    let policy = Policy { dry_run, force };
    let named = !clients.is_empty();
    let targets: Vec<Client> = if named {
        clients.to_vec()
    } else {
        Client::value_variants().to_vec()
    };
    let outcomes: Vec<Outcome> = targets
        .into_iter()
        .filter_map(|client| install_one(client, &dirs, &launch, policy, named))
        .collect();
    report(&outcomes, format, &launch, policy);
    if outcomes.is_empty() || outcomes.iter().any(|o| o.status.is_failure()) {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITE: Policy = Policy {
        dry_run: false,
        force: false,
    };

    fn entry() -> Value {
        json!({"command": "wrkflw", "args": SERVE_ARGS})
    }

    #[test]
    fn adds_to_a_missing_or_empty_file() {
        for text in [None, Some(""), Some("  \n")] {
            let (status, out) = upsert_json(text, "mcpServers", &entry(), WRITE).unwrap();
            assert_eq!(status, Status::Added);
            let out: Value = serde_json::from_str(&out.unwrap()).unwrap();
            assert_eq!(out, json!({"mcpServers": {"wrkflw": entry()}}));
        }
    }

    #[test]
    fn keeps_other_servers_and_key_order() {
        let text = r#"{"zeta": 1, "mcpServers": {"zz": {"command": "z", "env": {"B": "1", "A": "2"}}}, "alpha": true}"#;
        let (_, out) = upsert_json(Some(text), "mcpServers", &entry(), WRITE).unwrap();
        let out = out.unwrap();
        let order: Vec<usize> = [
            "\"zeta\"",
            "\"mcpServers\"",
            "\"zz\"",
            "\"B\"",
            "\"A\"",
            "\"wrkflw\"",
            "\"alpha\"",
        ]
        .iter()
        .map(|key| out.find(key).unwrap())
        .collect();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{out}");
    }

    #[test]
    fn an_identical_entry_is_unchanged_and_a_different_one_needs_force() {
        let same = r#"{"mcpServers": {"wrkflw": {"args": ["mcp", "serve"], "command": "wrkflw"}}}"#;
        let (status, out) = upsert_json(Some(same), "mcpServers", &entry(), WRITE).unwrap();
        assert_eq!((status, out), (Status::Unchanged, None));

        let other =
            r#"{"mcpServers": {"wrkflw": {"command": "/old/wrkflw", "args": ["mcp", "serve"]}}}"#;
        let (status, out) = upsert_json(Some(other), "mcpServers", &entry(), WRITE).unwrap();
        assert_eq!((status, out), (Status::Differs, None));

        let force = Policy {
            dry_run: false,
            force: true,
        };
        let (status, out) = upsert_json(Some(other), "mcpServers", &entry(), force).unwrap();
        assert_eq!(status, Status::Replaced);
        assert!(out.unwrap().contains("\"command\": \"wrkflw\""));
    }

    #[test]
    fn opencode_uses_its_jsonc_file_when_that_is_the_only_one() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        assert_eq!(opencode_config(dir), dir.join("opencode.json"));
        std::fs::write(dir.join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(opencode_config(dir), dir.join("opencode.jsonc"));
        std::fs::write(dir.join("opencode.json"), "{}").unwrap();
        assert_eq!(opencode_config(dir), dir.join("opencode.json"));
    }

    /// `codex mcp get <server> --json` from codex-cli 0.147.0, with the name and
    /// launch changed to wrkflw's.
    const CODEX_GET: &str = r#"{
  "name": "wrkflw",
  "enabled": true,
  "disabled_reason": null,
  "transport": {
    "type": "stdio",
    "command": "wrkflw",
    "args": ["mcp", "serve"],
    "env": null,
    "env_vars": [],
    "cwd": null
  },
  "enabled_tools": null,
  "disabled_tools": null,
  "startup_timeout_sec": null,
  "tool_timeout_sec": null
}"#;

    #[test]
    fn codex_get_output_is_matched_on_its_transport() {
        assert!(codex_matches(CODEX_GET.as_bytes(), "wrkflw"));
        assert!(!codex_matches(CODEX_GET.as_bytes(), "/other/wrkflw"));
        assert!(!codex_matches(b"not json", "wrkflw"));
    }

    #[test]
    fn only_the_launch_decides_and_force_keeps_the_users_keys() {
        let extra = r#"{"mcpServers": {"wrkflw": {"command": "wrkflw", "args": ["mcp", "serve"], "env": {"WRKFLW_HOME": "/x"}, "disabled": false}}}"#;
        let (status, out) = upsert_json(Some(extra), "mcpServers", &entry(), WRITE).unwrap();
        assert_eq!((status, out), (Status::Unchanged, None));

        let other = r#"{"mcpServers": {"wrkflw": {"env": {"WRKFLW_HOME": "/x"}, "command": "/old/wrkflw", "args": ["mcp", "serve"]}}}"#;
        let force = Policy {
            dry_run: false,
            force: true,
        };
        let (status, out) = upsert_json(Some(other), "mcpServers", &entry(), force).unwrap();
        assert_eq!(status, Status::Replaced);
        let out = out.unwrap();
        let written: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            written["mcpServers"]["wrkflw"],
            json!({"env": {"WRKFLW_HOME": "/x"}, "command": "wrkflw", "args": ["mcp", "serve"]})
        );
        assert!(
            out.find("\"env\"").unwrap() < out.find("\"command\"").unwrap(),
            "{out}"
        );
    }

    #[test]
    fn a_differing_entry_says_why_in_both_outputs() {
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: root.path().to_path_buf(),
            config: root.path().join("config"),
            claude: root.path().to_path_buf(),
        };
        std::fs::create_dir_all(root.path().join(".cursor")).unwrap();
        std::fs::write(
            root.path().join(".cursor/mcp.json"),
            r#"{"mcpServers": {"wrkflw": {"command": "/old", "args": ["mcp", "serve"]}}}"#,
        )
        .unwrap();
        let launch = Launch {
            command: "wrkflw".to_string(),
            absolute: "/bin/wrkflw".to_string(),
        };
        let outcome = install_one(Client::Cursor, &dirs, &launch, WRITE, false).unwrap();
        assert_eq!(outcome.status, Status::Differs);
        assert!(outcome.detail.unwrap().contains("--force"));
    }

    #[test]
    fn a_dry_run_never_writes() {
        let dry = Policy {
            dry_run: true,
            force: true,
        };
        assert_eq!(
            upsert_json(None, "servers", &entry(), dry).unwrap(),
            (Status::WouldAdd, None)
        );
        let other = r#"{"servers": {"wrkflw": {}}}"#;
        assert_eq!(
            upsert_json(Some(other), "servers", &entry(), dry).unwrap(),
            (Status::WouldReplace, None)
        );
    }

    #[test]
    fn comments_and_odd_shapes_are_refused() {
        for text in [
            "// mine\n{\"mcpServers\": {}}",
            "[]",
            r#"{"mcpServers": []}"#,
        ] {
            assert!(
                upsert_json(Some(text), "mcpServers", &entry(), WRITE).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn json_clients_are_detected_by_their_config_directory() {
        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: root.path().join("home"),
            config: root.path().join("config"),
            claude: root.path().join("home"),
        };
        let launch = Launch {
            command: "wrkflw".to_string(),
            absolute: "/bin/wrkflw".to_string(),
        };
        assert!(install_one(Client::Cursor, &dirs, &launch, WRITE, false).is_none());
        let missing = install_one(Client::Cursor, &dirs, &launch, WRITE, true).unwrap();
        assert_eq!(missing.status, Status::NotFound);

        std::fs::create_dir_all(dirs.config.join("Claude")).unwrap();
        let added = install_one(Client::ClaudeDesktop, &dirs, &launch, WRITE, false).unwrap();
        assert_eq!(added.status, Status::Added);
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dirs.config.join("Claude/claude_desktop_config.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(written["mcpServers"]["wrkflw"]["command"], "/bin/wrkflw");

        let again = install_one(Client::ClaudeDesktop, &dirs, &launch, WRITE, false).unwrap();
        assert_eq!(again.status, Status::Unchanged);
    }

    #[test]
    fn each_client_gets_its_own_entry_shape() {
        let dirs = Dirs {
            home: PathBuf::from("/h"),
            config: PathBuf::from("/c"),
            claude: PathBuf::from("/h"),
        };
        let Method::Json { key, entry, .. } = method(Client::Opencode, &dirs, "wrkflw") else {
            panic!("opencode is a file");
        };
        assert_eq!(key, "mcp");
        assert_eq!(entry["command"], json!(["wrkflw", "mcp", "serve"]));
        let Method::Json { key, entry, .. } = method(Client::Vscode, &dirs, "wrkflw") else {
            panic!("vscode is a file");
        };
        assert_eq!((key, &entry["type"]), ("servers", &json!("stdio")));
    }
    #[test]
    fn settle_covers_every_case() {
        let policy = |dry_run, force| Policy { dry_run, force };
        let cases = [
            (policy(false, false), Some(true), (Status::Unchanged, false)),
            (policy(true, true), Some(true), (Status::Unchanged, false)),
            (policy(false, false), Some(false), (Status::Differs, false)),
            (policy(true, false), Some(false), (Status::Differs, false)),
            (
                policy(true, true),
                Some(false),
                (Status::WouldReplace, false),
            ),
            (policy(false, true), Some(false), (Status::Replaced, true)),
            (policy(true, false), None, (Status::WouldAdd, false)),
            (policy(false, false), None, (Status::Added, true)),
        ];
        for (policy, existing, expected) in cases {
            assert_eq!(policy.settle(existing), expected, "{existing:?}");
        }
        assert!(Status::Differs.is_failure());
    }

    #[test]
    fn a_rewrite_keeps_the_files_indentation() {
        let text = "{\n    \"mcpServers\": {}\n}\n";
        let (_, out) = upsert_json(Some(text), "mcpServers", &entry(), WRITE).unwrap();
        assert!(out.unwrap().contains("\n        \"wrkflw\": {"));

        let tabs = "{\n\t\"a\": 1\n}";
        let (_, out) = upsert_json(Some(tabs), "mcpServers", &entry(), WRITE).unwrap();
        assert!(out.unwrap().contains("\n\t\"mcpServers\""));

        let (_, out) = upsert_json(Some("{}"), "mcpServers", &entry(), WRITE).unwrap();
        assert!(out.unwrap().contains("\n  \"mcpServers\""));
    }

    #[test]
    fn claude_code_config_follows_claude_config_dir() {
        let home = Path::new("/h");
        assert_eq!(claude_dir(home, None), home);
        assert_eq!(claude_dir(home, Some("".into())), home);
        assert_eq!(claude_dir(home, Some("/c".into())), Path::new("/c"));

        let root = tempfile::tempdir().unwrap();
        let dirs = Dirs {
            home: root.path().join("home"),
            config: root.path().join("config"),
            claude: root.path().to_path_buf(),
        };
        std::fs::write(
            root.path().join(".claude.json"),
            r#"{"mcpServers": {"wrkflw": {"command": "wrkflw", "args": ["mcp", "serve"]}}}"#,
        )
        .unwrap();
        assert_eq!(claude_code_existing(&dirs, "wrkflw"), Some(true));
        assert_eq!(claude_code_existing(&dirs, "/other"), Some(false));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_config_stays_a_link_and_keeps_its_mode() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let managed = root.path().join("dotfiles");
        let linked = root.path().join("cursor");
        std::fs::create_dir_all(&managed).unwrap();
        std::fs::create_dir_all(&linked).unwrap();

        let real = managed.join("mcp.json");
        std::fs::write(&real, "{}").unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o644)).unwrap();
        let link = linked.join("mcp.json");
        symlink(&real, &link).unwrap();
        let (status, _) = install_json(&link, "mcpServers", &entry(), WRITE);
        assert_eq!(status, Status::Added);
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert!(std::fs::read_to_string(&real).unwrap().contains("wrkflw"));
        let mode = std::fs::metadata(&real).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);

        // A new file is 0644, not the temporary file's 0600.
        let fresh = managed.join("fresh.json");
        install_json(&fresh, "mcpServers", &entry(), WRITE);
        let mode = std::fs::metadata(&fresh).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o644);

        // A relative link to a file that doesn't exist yet.
        let dangling = linked.join("other.json");
        symlink("../dotfiles/other.json", &dangling).unwrap();
        let (status, _) = install_json(&dangling, "mcpServers", &entry(), WRITE);
        assert_eq!(status, Status::Added);
        assert!(std::fs::symlink_metadata(&dangling).unwrap().is_symlink());
        assert!(managed.join("other.json").exists());
    }

    #[test]
    fn the_json_report_names_each_client() {
        let launch = Launch {
            command: "wrkflw".to_string(),
            absolute: "/bin/wrkflw".to_string(),
        };
        let outcomes = [Outcome {
            client: Client::ClaudeDesktop,
            status: Status::Differs,
            target: "/c/claude_desktop_config.json".to_string(),
            detail: None,
        }];
        assert_eq!(
            report_json(&outcomes, &launch),
            json!({
                "schema_version": SCHEMA_VERSION,
                "type": "mcp_install",
                "command": "wrkflw",
                "args": ["mcp", "serve"],
                "clients": [{
                    "client": "claude-desktop",
                    "status": "differs",
                    "target": "/c/claude_desktop_config.json",
                    "detail": null,
                }],
            })
        );
    }
}
