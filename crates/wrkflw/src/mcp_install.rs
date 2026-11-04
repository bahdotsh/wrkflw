//! `wrkflw mcp install`: register `wrkflw mcp serve` with coding agents.
//!
//! Claude Code, Codex and Grok Build are changed through their own CLIs.
//! Claude Code rewrites `~/.claude.json` while it runs, and Codex and Grok keep
//! TOML, so editing those files here would race the agent or lose formatting.
//! The other clients keep JSON, which is edited in place. Comments, key order
//! and layout are kept, so a commented file such as Zed's settings survives.

use crate::OutputFormat;
use clap::ValueEnum;
use jsonc_parser::cst::{CstInputValue, CstRootNode};
use jsonc_parser::ParseOptions;
use serde_json::{json, Value};
use std::ffi::OsString;
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
    /// GitHub Copilot CLI.
    Copilot,
    /// Grok Build.
    Grok,
    /// Kiro CLI.
    Kiro,
    Pi,
    Zed,
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
        /// The directory whose existence means the client is installed.
        marker: PathBuf,
    },
}

/// Where config lives. Separate so tests can point it at a temporary directory.
struct Dirs {
    home: PathBuf,
    /// The platform's per-user config directory, e.g. `~/Library/Application Support`.
    config: PathBuf,
    /// Where Claude Code keeps `.claude.json`.
    claude: PathBuf,
    /// `$COPILOT_HOME`, else `~/.copilot`.
    copilot: PathBuf,
    /// `$GROK_HOME`, else `~/.grok`.
    grok: PathBuf,
    /// `$KIRO_HOME` when Kiro accepts it, else `~/.kiro`.
    kiro: PathBuf,
    /// `$PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
    pi: PathBuf,
    /// Zed's config directory, which holds `settings.json`.
    zed: PathBuf,
}

impl Dirs {
    fn current() -> Option<Self> {
        Some(Self::resolve(
            dirs::home_dir()?,
            dirs::config_dir()?,
            |name| std::env::var_os(name),
        ))
    }

    /// The directories for `home` and `config`, with overrides read through `var`.
    fn resolve(home: PathBuf, config: PathBuf, var: impl Fn(&str) -> Option<OsString>) -> Self {
        let set = |name: &str| var(name).filter(|value| !value.is_empty());
        Self {
            claude: set("CLAUDE_CONFIG_DIR").map_or_else(|| home.clone(), PathBuf::from),
            copilot: set("COPILOT_HOME").map_or_else(|| home.join(".copilot"), PathBuf::from),
            grok: set("GROK_HOME").map_or_else(|| home.join(".grok"), PathBuf::from),
            kiro: kiro_home(&home, set("KIRO_HOME")),
            pi: set("PI_CODING_AGENT_DIR")
                .map_or_else(|| home.join(".pi/agent"), |dir| expand_tilde(&home, dir)),
            zed: zed_dir(&home, &config, set("XDG_CONFIG_HOME")),
            home,
            config,
        }
    }
}

/// Kiro ignores a `KIRO_HOME` that is relative, a filesystem root, or the home
/// directory or one of its ancestors, and falls back to `~/.kiro`.
fn kiro_home(home: &Path, configured: Option<OsString>) -> PathBuf {
    match configured.map(PathBuf::from) {
        Some(dir) if dir.is_absolute() && dir.parent().is_some() && !home.starts_with(&dir) => dir,
        _ => home.join(".kiro"),
    }
}

/// pi expands a leading `~` in `PI_CODING_AGENT_DIR` itself.
fn expand_tilde(home: &Path, dir: OsString) -> PathBuf {
    let dir = PathBuf::from(dir);
    match dir.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => dir,
    }
}

/// Zed keeps settings in `~/.config/zed` on macOS, the XDG config directory on
/// Linux and the BSDs, and `%APPDATA%\Zed` on Windows.
fn zed_dir(home: &Path, config: &Path, xdg: Option<OsString>) -> PathBuf {
    if cfg!(windows) {
        config.join("Zed")
    } else if cfg!(target_os = "macos") {
        home.join(".config/zed")
    } else {
        xdg.map_or_else(|| home.join(".config"), PathBuf::from)
            .join("zed")
    }
}

/// The command a client should launch. `wrkflw` when it is on PATH, so the
/// entry survives upgrades. Claude Desktop and Kiro start servers without the
/// shell's PATH, so they always get an absolute path.
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
        if matches!(client, Client::ClaudeDesktop | Client::Kiro) {
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
    // With an env object, as the client itself writes one.
    let with_env = json!({"command": command, "args": SERVE_ARGS, "env": {}});
    let file = |path: PathBuf, key, entry| Method::Json {
        marker: path.parent().unwrap_or(&path).to_path_buf(),
        path,
        key,
        entry,
    };
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
        Client::Copilot => file(
            dirs.copilot.join("mcp-config.json"),
            "mcpServers",
            json!({"type": "local", "command": command, "args": SERVE_ARGS, "env": {}, "tools": ["*"]}),
        ),
        Client::Grok => Method::Cli {
            program: "grok",
            scope: &[],
            existing: grok_existing,
        },
        // The file `kiro-cli mcp add --scope global` writes. Detected by Kiro's
        // home, since `settings/` may not exist yet.
        Client::Kiro => Method::Json {
            path: dirs.kiro.join("settings/mcp.json"),
            key: "mcpServers",
            entry: with_env,
            marker: dirs.kiro.clone(),
        },
        Client::Pi => file(dirs.pi.join("mcp.json"), "mcpServers", standard),
        Client::Zed => file(dirs.zed.join("settings.json"), "context_servers", with_env),
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
        Method::Json { marker, .. } => marker.is_dir(),
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

/// The keys that decide what an entry launches. Others, such as `env` or
/// `disabled`, are the user's and are kept.
const LAUNCH_KEYS: [&str; 2] = ["command", "args"];

/// Whether `current` launches what `wanted` does.
fn same_launch(current: &Value, wanted: &Value) -> bool {
    current.is_object()
        && LAUNCH_KEYS
            .iter()
            .all(|key| current.get(*key) == wanted.get(*key))
}

/// `value` as a node to insert into a config.
fn to_cst(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(value) => CstInputValue::Number(value.to_string()),
        Value::String(value) => CstInputValue::String(value.clone()),
        Value::Array(items) => CstInputValue::Array(items.iter().map(to_cst).collect()),
        Value::Object(map) => CstInputValue::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), to_cst(value)))
                .collect(),
        ),
    }
}

/// Add `entry` as server `wrkflw` under `key` in the config `text`.
///
/// The text is edited, not re-serialised, so comments, trailing commas, key
/// order and indentation all survive. An existing entry is compared by what it
/// launches, and on replace the wanted keys are merged into it, so keys the
/// user added survive `--force`. Returns the status and, when the file should
/// change, its new text.
fn upsert_json(
    text: Option<&str>,
    key: &str,
    entry: &Value,
    policy: Policy,
) -> Result<(Status, Option<String>), String> {
    let text = text.filter(|text| !text.trim().is_empty());
    let root = CstRootNode::parse(text.unwrap_or("{}"), &ParseOptions::default())
        .map_err(|e| format!("can't parse it as JSON ({e}); add the entry by hand"))?;
    let Some(config) = root.object_value() else {
        return Err("the file isn't a JSON object".to_string());
    };
    let servers = match config.get(key) {
        None => None,
        Some(prop) => Some(
            prop.object_value()
                .ok_or_else(|| format!("`{key}` isn't a JSON object"))?,
        ),
    };
    let existing = servers
        .as_ref()
        .and_then(|servers| servers.get(SERVER))
        .map(|prop| {
            prop.value()
                .and_then(|current| current.to_serde_value())
                .is_some_and(|current| same_launch(&current, entry))
        });
    let (status, write) = policy.settle(existing);
    if !write {
        return Ok((status, None));
    }
    let Value::Object(fields) = entry else {
        return Err("the entry isn't a JSON object".to_string());
    };
    let servers = servers.unwrap_or_else(|| config.object_value_or_set(key));
    match servers.get(SERVER) {
        // Set in place, which keeps the position of keys already there.
        Some(prop) => match prop.object_value() {
            Some(current) => {
                for (name, value) in fields {
                    match current.get(name) {
                        Some(field) => field.set_value(to_cst(value)),
                        None => {
                            current.append(name, to_cst(value));
                        }
                    }
                }
            }
            None => prop.set_value(to_cst(entry)),
        },
        None => {
            servers.append(SERVER, to_cst(entry));
        }
    }
    let mut text = root.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok((status, Some(text)))
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

/// Replace the file `path` resolves to, keeping its permissions, and create
/// its directory if needed. A new file gets the usual 0644 rather than the
/// temporary file's 0600.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = resolve_links(path);
    let dir = path.parent().unwrap_or(Path::new("."));
    // A client named on the command line may not have its directory yet.
    std::fs::create_dir_all(dir)?;
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

/// Whether Grok Build's `config.toml` has a `wrkflw` server, and whether it
/// launches `command`. `grok mcp add` writes `[mcp_servers.<name>]` there.
fn grok_existing(dirs: &Dirs, command: &str) -> Option<bool> {
    let text = std::fs::read_to_string(dirs.grok.join("config.toml")).ok()?;
    toml_server_matches(&text, command)
}

/// The `wrkflw` entry under `[mcp_servers]` in `text`: `None` when there is
/// none, else whether it launches `command`.
fn toml_server_matches(text: &str, command: &str) -> Option<bool> {
    let config: toml::Table = text.parse().ok()?;
    let server = config.get("mcp_servers")?.get(SERVER)?;
    let args: Option<Vec<&str>> = server
        .get("args")
        .and_then(|args| args.as_array())
        .map(|args| args.iter().filter_map(|arg| arg.as_str()).collect());
    Some(
        server.get("command").and_then(|c| c.as_str()) == Some(command)
            && args.as_deref() == Some(&SERVE_ARGS[..]),
    )
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
        match (&method, named) {
            // Skip a client nobody asked for.
            (_, false) => return None,
            // A named client gets its config file even before it has one, so
            // wrkflw can be set up before the agent first runs.
            (Method::Json { .. }, true) => {}
            (Method::Cli { program, .. }, true) => {
                return Some(Outcome {
                    client,
                    status: Status::NotFound,
                    detail: Some(format!("`{program}` isn't on PATH")),
                    target,
                })
            }
        }
    }
    let (status, detail) = match &method {
        Method::Cli {
            program,
            scope,
            existing,
        } => install_cli(program, scope, existing(dirs, command), command, policy),
        Method::Json {
            path, key, entry, ..
        } => install_json(path, key, entry, policy),
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

    /// Directories under `home` and `config`, with no overrides set.
    fn dirs_in(home: impl Into<PathBuf>, config: impl Into<PathBuf>) -> Dirs {
        Dirs::resolve(home.into(), config.into(), |_| None)
    }

    fn launch() -> Launch {
        Launch {
            command: "wrkflw".to_string(),
            absolute: "/bin/wrkflw".to_string(),
        }
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
        let dirs = dirs_in(root.path(), root.path().join("config"));
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
    fn odd_shapes_are_refused() {
        for text in ["[]", r#"{"mcpServers": []}"#, "{\"mcpServers\": {"] {
            assert!(
                upsert_json(Some(text), "mcpServers", &entry(), WRITE).is_err(),
                "{text}"
            );
        }
    }

    #[test]
    fn json_clients_are_detected_by_their_config_directory() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path().join("home"), root.path().join("config"));
        let launch = Launch {
            command: "wrkflw".to_string(),
            absolute: "/bin/wrkflw".to_string(),
        };
        assert!(install_one(Client::Cursor, &dirs, &launch, WRITE, false).is_none());

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
        let dirs = dirs_in("/h", "/c");
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
        let claude = |value: Option<&str>| {
            Dirs::resolve("/h".into(), "/c".into(), |name| {
                (name == "CLAUDE_CONFIG_DIR").then(|| value.map(OsString::from))?
            })
            .claude
        };
        assert_eq!(claude(None), Path::new("/h"));
        assert_eq!(claude(Some("")), Path::new("/h"));
        assert_eq!(claude(Some("/cc")), Path::new("/cc"));

        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_path_buf();
        let dirs = Dirs::resolve(
            root.path().join("home"),
            root.path().join("config"),
            |name| (name == "CLAUDE_CONFIG_DIR").then(|| path.clone().into_os_string()),
        );
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

    #[test]
    fn comments_trailing_commas_and_layout_survive() {
        // The shape of the settings file Zed creates.
        let text = "// Zed settings\n//\n// See https://zed.dev/docs\n{\n    \"vim_mode\": true, // mine\n    \"agent\": {\n        \"model\": \"x\",\n    },\n}\n";
        let wanted = json!({"command": "wrkflw", "args": SERVE_ARGS, "env": {}});
        let (status, out) = upsert_json(Some(text), "context_servers", &wanted, WRITE).unwrap();
        assert_eq!(status, Status::Added);
        let out = out.unwrap();
        assert!(
            out.starts_with("// Zed settings\n//\n// See https://zed.dev/docs\n{\n"),
            "{out}"
        );
        assert!(out.contains("\"vim_mode\": true, // mine\n"), "{out}");
        assert!(
            out.contains("\n    \"context_servers\": {\n        \"wrkflw\": {"),
            "{out}"
        );
        assert!(out.ends_with("}\n"), "{out}");

        // Read back, it is unchanged on a second run.
        let (status, again) = upsert_json(Some(&out), "context_servers", &wanted, WRITE).unwrap();
        assert_eq!((status, again), (Status::Unchanged, None));
    }

    #[test]
    fn force_on_a_commented_file_keeps_its_comments() {
        let text = "{\n  \"mcpServers\": {\n    // pinned build\n    \"wrkflw\": {\"command\": \"/old\", \"args\": [\"mcp\", \"serve\"], \"env\": {\"A\": \"1\"}},\n  },\n}\n";
        let force = Policy {
            dry_run: false,
            force: true,
        };
        let (status, out) = upsert_json(Some(text), "mcpServers", &entry(), force).unwrap();
        assert_eq!(status, Status::Replaced);
        let out = out.unwrap();
        assert!(out.contains("// pinned build"), "{out}");
        assert!(out.contains("\"command\": \"wrkflw\""), "{out}");
        assert!(out.contains("\"env\": {\"A\": \"1\"}"), "{out}");
        assert!(!out.contains("/old"), "{out}");
    }

    #[test]
    fn a_named_client_gets_its_config_created() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path().join("home"), root.path().join("config"));
        for client in [
            Client::Cursor,
            Client::Kiro,
            Client::Pi,
            Client::Zed,
            Client::Copilot,
        ] {
            // Not installed, so skipped unless named.
            assert!(install_one(client, &dirs, &launch(), WRITE, false).is_none());
            let outcome = install_one(client, &dirs, &launch(), WRITE, true).unwrap();
            assert_eq!(outcome.status, Status::Added, "{}", outcome.client.name());
            assert!(Path::new(&outcome.target).is_file(), "{}", outcome.target);
        }
        // A CLI client can't be set up without its CLI.
        let mut missing = dirs_in(root.path().join("home"), root.path().join("config"));
        missing.grok = root.path().join("no-grok");
        if which::which("grok").is_err() {
            let outcome = install_one(Client::Grok, &missing, &launch(), WRITE, true).unwrap();
            assert_eq!(outcome.status, Status::NotFound);
        }
    }

    #[test]
    fn kiro_is_detected_by_its_home_and_launched_by_absolute_path() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs_in(root.path().join("home"), root.path().join("config"));
        std::fs::create_dir_all(&dirs.kiro).unwrap();
        let outcome = install_one(Client::Kiro, &dirs, &launch(), WRITE, false).unwrap();
        assert_eq!(outcome.status, Status::Added);
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dirs.kiro.join("settings/mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            written,
            json!({"mcpServers": {"wrkflw": {"command": "/bin/wrkflw", "args": ["mcp", "serve"], "env": {}}}})
        );
    }

    #[test]
    fn new_clients_get_the_entry_each_one_documents() {
        let dirs = dirs_in("/h", "/c");
        let shape = |client| match method(client, &dirs, "wrkflw") {
            Method::Json {
                path, key, entry, ..
            } => (path, key, entry),
            Method::Cli { .. } => panic!("{} is a file", client.name()),
        };
        let (path, key, entry) = shape(Client::Copilot);
        assert_eq!(path, Path::new("/h/.copilot/mcp-config.json"));
        assert_eq!(key, "mcpServers");
        assert_eq!(
            entry,
            json!({"type": "local", "command": "wrkflw", "args": ["mcp", "serve"], "env": {}, "tools": ["*"]})
        );
        let (path, key, _) = shape(Client::Pi);
        assert_eq!(
            (path.as_path(), key),
            (Path::new("/h/.pi/agent/mcp.json"), "mcpServers")
        );
        let (path, key, entry) = shape(Client::Zed);
        assert_eq!(path, dirs.zed.join("settings.json"));
        assert_eq!(key, "context_servers");
        assert_eq!(
            entry,
            json!({"command": "wrkflw", "args": ["mcp", "serve"], "env": {}})
        );
        assert!(matches!(
            method(Client::Grok, &dirs, "wrkflw"),
            Method::Cli {
                program: "grok",
                ..
            }
        ));
    }

    #[test]
    fn agent_homes_follow_their_variables() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            Dirs::resolve("/home/u".into(), "/c".into(), move |name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            })
        };
        let dirs = vars(&[
            ("COPILOT_HOME", "/x/copilot"),
            ("GROK_HOME", "/x/grok"),
            ("PI_CODING_AGENT_DIR", "~/pi-agent"),
        ]);
        assert_eq!(dirs.copilot, Path::new("/x/copilot"));
        assert_eq!(dirs.grok, Path::new("/x/grok"));
        assert_eq!(dirs.pi, Path::new("/home/u/pi-agent"));

        let defaults = vars(&[]);
        assert_eq!(defaults.copilot, Path::new("/home/u/.copilot"));
        assert_eq!(defaults.grok, Path::new("/home/u/.grok"));
        assert_eq!(defaults.kiro, Path::new("/home/u/.kiro"));
        assert_eq!(defaults.pi, Path::new("/home/u/.pi/agent"));
    }

    #[cfg(unix)]
    #[test]
    fn kiro_home_follows_kiros_rule() {
        let home = Path::new("/home/u");
        let kiro = |value: Option<&str>| kiro_home(home, value.map(OsString::from));
        assert_eq!(kiro(None), Path::new("/home/u/.kiro"));
        assert_eq!(kiro(Some("/srv/kiro")), Path::new("/srv/kiro"));
        assert_eq!(
            kiro(Some("/home/u/kiro-home")),
            Path::new("/home/u/kiro-home")
        );
        for ignored in ["kiro", "/", "/home/u", "/home"] {
            assert_eq!(kiro(Some(ignored)), Path::new("/home/u/.kiro"), "{ignored}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn zed_settings_live_where_zed_reads_them() {
        let home = Path::new("/home/u");
        let config = Path::new("/c");
        let dir = zed_dir(home, config, Some("/xdg".into()));
        if cfg!(target_os = "macos") {
            assert_eq!(dir, Path::new("/home/u/.config/zed"));
        } else {
            assert_eq!(dir, Path::new("/xdg/zed"));
            assert_eq!(
                zed_dir(home, config, None),
                Path::new("/home/u/.config/zed")
            );
        }
    }

    #[test]
    fn grok_config_is_matched_on_its_mcp_server_table() {
        let text = "model = \"grok-4\"\n\n[mcp_servers.wrkflw]\ncommand = \"wrkflw\"\nargs = [\"mcp\", \"serve\"]\n";
        assert_eq!(toml_server_matches(text, "wrkflw"), Some(true));
        assert_eq!(toml_server_matches(text, "/other"), Some(false));
        let other = "[mcp_servers.atuin]\ncommand = \"atuin\"\nargs = [\"mcp\"]\n";
        assert_eq!(toml_server_matches(other, "wrkflw"), None);
        assert_eq!(toml_server_matches("not = [toml", "wrkflw"), None);
    }
}
