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
    fn name(self) -> &'static str {
        match self {
            Client::ClaudeCode => "claude-code",
            Client::Codex => "codex",
            Client::Cursor => "cursor",
            Client::Windsurf => "windsurf",
            Client::Vscode => "vscode",
            Client::Gemini => "gemini",
            Client::Opencode => "opencode",
            Client::ClaudeDesktop => "claude-desktop",
        }
    }
}

/// How a client's config is changed.
enum Method {
    /// Through the client's CLI.
    Cli(&'static str),
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
}

impl Dirs {
    fn current() -> Option<Self> {
        Some(Self {
            home: dirs::home_dir()?,
            config: dirs::config_dir()?,
        })
    }
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
        let absolute = which::which(SERVER)
            .ok()
            .or_else(|| std::env::current_exe().ok())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| SERVER.to_string());
        let command = if which::which(SERVER).is_ok() {
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
        Client::ClaudeCode => Method::Cli("claude"),
        Client::Codex => Method::Cli("codex"),
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
            dirs.home.join(".config/opencode/opencode.json"),
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

/// A client is installed when its CLI is on PATH or its config directory exists.
fn detected(method: &Method) -> bool {
    match method {
        Method::Cli(program) => which::which(program).is_ok(),
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

    fn is_failure(self) -> bool {
        matches!(self, Status::Failed | Status::NotFound)
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

/// Add `entry` as server `wrkflw` under `key` in the config `text`.
///
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
    let entry: Ordered = serde_json::from_value(entry.clone()).map_err(|e| e.to_string())?;
    let servers = root
        .entry(key.to_string())
        .or_insert_with(|| Ordered::Object(IndexMap::new()));
    let Ordered::Object(servers) = servers else {
        return Err(format!("`{key}` isn't a JSON object"));
    };
    let existing = servers.get(SERVER).map(|current| *current == entry);
    let (status, write) = policy.settle(existing);
    if !write {
        return Ok((status, None));
    }
    servers.insert(SERVER.to_string(), entry);
    let mut text =
        serde_json::to_string_pretty(&Ordered::Object(root)).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok((status, Some(text)))
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
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
/// Read from `~/.claude.json` rather than `claude mcp get`, which also reports
/// servers from the current project's `.mcp.json`.
fn claude_code_existing(dirs: &Dirs, command: &str) -> Option<bool> {
    let text = std::fs::read_to_string(dirs.home.join(".claude.json")).ok()?;
    let config: Value = serde_json::from_str(&text).ok()?;
    let server = config.get("mcpServers")?.get(SERVER)?;
    Some(server["command"] == command && server["args"] == json!(SERVE_ARGS))
}

fn codex_existing(command: &str) -> Option<bool> {
    let output = run("codex", &["mcp", "get", SERVER, "--json"]).ok()?;
    if !output.status.success() {
        return None;
    }
    let server: Value = serde_json::from_slice(&output.stdout).ok()?;
    let transport = &server["transport"];
    Some(transport["command"] == command && transport["args"] == json!(SERVE_ARGS))
}

fn install_cli(
    client: Client,
    dirs: &Dirs,
    command: &str,
    policy: Policy,
) -> (Status, Option<String>) {
    let existing = match client {
        Client::ClaudeCode => claude_code_existing(dirs, command),
        _ => codex_existing(command),
    };
    let (status, write) = policy.settle(existing);
    if !write {
        return (status, None);
    }
    let mut serve = vec!["--", command];
    serve.extend(SERVE_ARGS);
    let result = match client {
        Client::ClaudeCode => {
            if status == Status::Replaced {
                let _ = check("claude", &["mcp", "remove", "--scope", "user", SERVER]);
            }
            let mut add = vec!["mcp", "add", "--scope", "user", SERVER];
            add.extend(&serve);
            check("claude", &add)
        }
        _ => {
            if status == Status::Replaced {
                let _ = check("codex", &["mcp", "remove", SERVER]);
            }
            let mut add = vec!["mcp", "add", SERVER];
            add.extend(&serve);
            check("codex", &add)
        }
    };
    match result {
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
        Method::Cli(program) => program.to_string(),
        Method::Json { path, .. } => path.to_string_lossy().into_owned(),
    };
    if !detected(&method) {
        // Only report a client nobody asked for if it was named.
        return named.then(|| Outcome {
            client,
            status: Status::NotFound,
            detail: Some(match &method {
                Method::Cli(program) => format!("`{program}` isn't on PATH"),
                Method::Json { path, .. } => {
                    format!("{} doesn't exist", path.parent().unwrap_or(path).display())
                }
            }),
            target,
        });
    }
    let (status, detail) = match &method {
        Method::Cli(_) => install_cli(client, dirs, command, policy),
        Method::Json { path, key, entry } => install_json(path, key, entry, policy),
    };
    Some(Outcome {
        client,
        status,
        target,
        detail,
    })
}

fn report(outcomes: &[Outcome], format: OutputFormat, launch: &Launch) {
    if format.is_json() {
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
        wrkflw_runs::emit_line(&json!({
            "schema_version": SCHEMA_VERSION,
            "type": "mcp_install",
            "command": launch.command,
            "args": args(),
            "clients": clients,
        }));
        return;
    }
    if outcomes.is_empty() {
        let supported: Vec<&str> = Client::value_variants().iter().map(|c| c.name()).collect();
        println!(
            "No supported coding agent found. Supported: {}.",
            supported.join(", ")
        );
        return;
    }
    for outcome in outcomes {
        let status = outcome.status.as_str().replace('_', " ");
        let detail = match (&outcome.detail, outcome.status) {
            (Some(detail), _) => format!(": {detail}"),
            (None, Status::Differs) => {
                ": registered with another command, pass --force to replace it".to_string()
            }
            _ => String::new(),
        };
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
        println!(
            "wrkflw isn't on PATH, so {} was registered. Rerun this after installing it.",
            launch.command
        );
    }
}

/// Register the server with `clients`, or with every detected client when empty.
/// Exits 1 if any client failed or none was found.
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
    report(&outcomes, format, &launch);
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
}
