//! The job workspace as a container volume, on hosts whose filesystem is not
//! Linux's.
//!
//! Docker Desktop and Podman machine share a macOS directory with their Linux
//! VM through a file share that does not behave like a Linux filesystem: it
//! refuses names that are not UTF-8 and folds case. A test that relies on
//! either fails locally and passes on GitHub. So on such hosts the workspace
//! lives in a named volume inside the VM, and the host copy is kept in step
//! with it around every container step: what changed on the host goes in
//! before the step, and what the step changed comes back after it. Host-side
//! work, such as checkout, caches, artifacts and `hashFiles`, keeps using the
//! host copy unchanged.
//!
//! Both directions compare listings (type, size, whole-second mtime, mode, link
//! target), so a step that changes little costs little. An entry the host
//! cannot hold, such as a name that is not UTF-8, stays in the volume and is
//! never deleted from it for being absent on the host.
//!
//! `WRKFLW_WORKSPACE_MOUNT=bind` turns this off; `=volume` turns it on on
//! Linux too.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};

use sha2::{Digest, Sha256};
use wrkflw_runtime::container::ContainerRuntime;

use crate::kubernetes::sync::{clear_type_conflict, safe_relative_path};

/// Chooses how the workspace reaches step containers.
pub(crate) const MOUNT_ENV: &str = "WRKFLW_WORKSPACE_MOUNT";
/// The image whose `find` and `tar` (GNU) do the volume side of a sync.
const HELPER_IMAGE_ENV: &str = "WRKFLW_WORKSPACE_HELPER_IMAGE";
const DEFAULT_HELPER_IMAGE: &str = "ubuntu:latest";
/// Labels every workspace volume with the host directory it mirrors.
const LABEL: &str = "wrkflw.workspace";
const VOLUME_PREFIX: &str = "wrkflw-ws-";
/// Where the helper container mounts the volume.
const ROOT: &str = "/ws";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    File { size: u64, mtime: i64 },
    Dir,
    Link(PathBuf),
}

impl Kind {
    fn same_type(&self, other: &Kind) -> bool {
        matches!(
            (self, other),
            (Kind::File { .. }, Kind::File { .. })
                | (Kind::Dir, Kind::Dir)
                | (Kind::Link(_), Kind::Link(_))
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    /// Permission bits, `None` where the host has none to compare.
    mode: Option<u32>,
}

impl Entry {
    /// True when copying one over the other would change nothing.
    fn matches(&self, other: &Entry) -> bool {
        let modes_agree = match (self.mode, other.mode) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        };
        // A link's own mode means nothing.
        self.kind == other.kind && (matches!(self.kind, Kind::Link(_)) || modes_agree)
    }
}

type Listing = HashMap<PathBuf, Entry>;

/// True when steps should get a volume instead of the host directory.
fn wanted(host_is_linux: bool, setting: Option<&str>) -> bool {
    match setting.map(str::trim) {
        Some("bind") => false,
        Some("volume") => true,
        _ => !host_is_linux,
    }
}

/// The volume that holds the workspace at `host`.
fn volume_name(host: &Path) -> String {
    let digest = Sha256::digest(host.to_string_lossy().as_bytes());
    let hex: String = digest[..8].iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}{}", VOLUME_PREFIX, hex)
}

struct State {
    /// The helper container, once started.
    helper: Option<String>,
    /// The volume as of the end of the last sync; `None` before the first.
    volume: Option<Listing>,
    /// Volume entries the host could not hold.
    kept: HashSet<PathBuf>,
    /// Set when the volume couldn't be set up before any step used it: the
    /// job then mounts the host directory as before.
    unusable: bool,
}

pub(crate) struct VmWorkspace {
    cli: &'static str,
    host: PathBuf,
    volume: String,
    volume_path: PathBuf,
    state: Mutex<State>,
}

static WORKSPACES: LazyLock<Mutex<HashMap<PathBuf, Arc<VmWorkspace>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Before a container step: the volume for the job workspace at `host`, with
/// the host's changes copied in, or `None` when the step should mount `host`.
///
/// An `Err` means the volume and the host copy may now disagree, so the step
/// must not run.
pub(crate) async fn before_step(
    runtime: &dyn ContainerRuntime,
    host: &Path,
) -> Result<Option<Arc<VmWorkspace>>, String> {
    let Some(cli) = runtime.session_cli() else {
        return Ok(None);
    };
    let setting = std::env::var(MOUNT_ENV).ok();
    if !wanted(cfg!(target_os = "linux"), setting.as_deref()) || !host.is_dir() {
        return Ok(None);
    }
    let workspace = {
        let mut all = WORKSPACES.lock().unwrap_or_else(|e| e.into_inner());
        all.entry(host.to_path_buf())
            .or_insert_with(|| {
                let volume = volume_name(host);
                Arc::new(VmWorkspace {
                    cli,
                    host: host.to_path_buf(),
                    volume_path: PathBuf::from(&volume),
                    volume,
                    state: Mutex::new(State {
                        helper: None,
                        volume: None,
                        kept: HashSet::new(),
                        unusable: false,
                    }),
                })
            })
            .clone()
    };
    let synced = workspace.clone();
    let used = tokio::task::spawn_blocking(move || synced.sync_in())
        .await
        .map_err(|e| format!("the workspace sync stopped: {}", e))??;
    Ok(used.then_some(workspace))
}

impl VmWorkspace {
    /// Point the workspace mount in `volumes` at the volume.
    pub(crate) fn mount_in<'a>(&'a self, volumes: &mut [(&'a Path, &'a Path)]) {
        for (host, _) in volumes.iter_mut() {
            if *host == self.host.as_path() {
                *host = self.volume_path.as_path();
            }
        }
    }

    /// After the step: copy what it changed back to the host.
    pub(crate) async fn after_step(self: &Arc<Self>) -> Result<(), String> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.sync_out())
            .await
            .map_err(|e| format!("the workspace sync stopped: {}", e))?
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `Ok(false)` when the volume can't be used and steps mount the host.
    fn sync_in(&self) -> Result<bool, String> {
        let mut state = self.lock();
        if state.unusable {
            return Ok(false);
        }
        let first = state.volume.is_none();
        let result = self.sync_in_locked(&mut state);
        match result {
            Err(e) if first => {
                wrkflw_logging::warning(&format!(
                    "Steps see the workspace through the {} file share, because a volume for \
                     it couldn't be set up: {}. Linux-only file names and case-sensitive paths \
                     may then behave differently than on GitHub.",
                    self.cli, e
                ));
                state.unusable = true;
                if let Some(helper) = state.helper.take() {
                    remove_container(self.cli, &helper);
                }
                Ok(false)
            }
            Err(e) => Err(format!(
                "copying the workspace into the {} volume failed: {}",
                self.cli, e
            )),
            Ok(()) => Ok(true),
        }
    }

    fn sync_in_locked(&self, state: &mut State) -> io::Result<()> {
        let helper = self.helper(state)?;
        let volume = match state.volume.take() {
            Some(listing) => listing,
            None => self.list_volume(&helper)?,
        };
        let host = walk(&self.host)?;
        let plan = plan_sync_in(&host, &volume, &state.kept, |rel| {
            std::fs::symlink_metadata(self.host.join(rel)).is_ok()
        });
        if !plan.delete.is_empty() {
            exec(
                self.cli,
                &helper,
                &["sh", "-c", &format!("cd {} && xargs -0 -r rm -rf --", ROOT)],
                Some(nul_list(&plan.delete)),
            )?;
        }
        if !plan.send.is_empty() {
            self.send(&helper, &plan.send)?;
        }
        let mut volume = volume;
        for gone in &plan.delete {
            volume.retain(|path, _| !path.starts_with(gone));
        }
        for path in &plan.send {
            if let Some(entry) = host.get(path) {
                volume.insert(path.clone(), entry.clone());
            }
        }
        state.volume = Some(volume);
        Ok(())
    }

    fn sync_out(&self) -> Result<(), String> {
        let mut state = self.lock();
        self.sync_out_locked(&mut state).map_err(|e| {
            // The next step must list the volume again.
            state.volume = None;
            format!(
                "copying the workspace back from the {} volume failed: {}",
                self.cli, e
            )
        })
    }

    fn sync_out_locked(&self, state: &mut State) -> io::Result<()> {
        let helper = self.helper(state)?;
        let volume = self.list_volume(&helper)?;
        let host = walk(&self.host)?;
        let plan = plan_sync_out(&host, &volume, &state.kept);
        for path in &plan.delete {
            let target = self.host.join(path);
            let removed = match std::fs::symlink_metadata(&target) {
                Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&target),
                Ok(_) => std::fs::remove_file(&target),
                Err(_) => Ok(()),
            };
            if let Err(e) = removed {
                wrkflw_logging::warning(&format!(
                    "Could not remove {} from the workspace: {}",
                    target.display(),
                    e
                ));
            }
        }
        if !plan.fetch.is_empty() {
            let refused = self.fetch(&helper, &plan.fetch, &volume)?;
            if !refused.is_empty() {
                let mut names: Vec<String> = refused
                    .iter()
                    .take(3)
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if refused.len() > 3 {
                    names.push(format!("and {} more", refused.len() - 3));
                }
                wrkflw_logging::info(&format!(
                    "{} workspace entr{} can't exist on this machine's filesystem, so {} only \
                     in the container volume: {}",
                    refused.len(),
                    if refused.len() == 1 { "y" } else { "ies" },
                    if refused.len() == 1 {
                        "it stays"
                    } else {
                        "they stay"
                    },
                    names.join(", ")
                ));
                state.kept.extend(refused);
            }
        }
        state.kept.retain(|path| volume.contains_key(path));
        state.volume = Some(volume);
        Ok(())
    }

    /// The helper container, started on first use.
    fn helper(&self, state: &mut State) -> io::Result<String> {
        if let Some(id) = &state.helper {
            return Ok(id.clone());
        }
        let label = format!("{}={}", LABEL, self.host.display());
        let create = Command::new(self.cli)
            .args(["volume", "create", "--label", &label, &self.volume])
            .stdin(Stdio::null())
            .output()?;
        if !create.status.success() {
            let stderr = String::from_utf8_lossy(&create.stderr);
            // Podman refuses to create a volume that exists; Docker doesn't.
            if !stderr.contains("already exists") {
                return Err(io::Error::other(format!(
                    "{} volume create failed: {}",
                    self.cli,
                    stderr.trim()
                )));
            }
        }
        let image = std::env::var(HELPER_IMAGE_ENV)
            .ok()
            .filter(|image| !image.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_HELPER_IMAGE.to_string());
        let name = format!("wrkflw-ws-sync-{}", uuid::Uuid::new_v4());
        let mount = format!("{}:{}", self.volume, ROOT);
        let started = Command::new(self.cli)
            .args([
                "run",
                "-d",
                "--name",
                &name,
                "-v",
                &mount,
                "--entrypoint",
                "sh",
                &image,
                "-c",
                crate::job_session::KEEP_ALIVE,
            ])
            .stdin(Stdio::null())
            .output()?;
        if !started.status.success() {
            return Err(io::Error::other(format!(
                "the sync container ({}) didn't start: {}",
                image,
                String::from_utf8_lossy(&started.stderr).trim()
            )));
        }
        let id = String::from_utf8_lossy(&started.stdout).trim().to_string();
        track(self.cli, &id);
        state.helper = Some(id.clone());
        Ok(id)
    }

    fn list_volume(&self, helper: &str) -> io::Result<Listing> {
        let out = exec_allowing(
            self.cli,
            helper,
            &[
                "find",
                ROOT,
                "-mindepth",
                "1",
                "-printf",
                "%P\\0%y\\0%s\\0%T@\\0%m\\0%l\\0",
            ],
            None,
            // find exits 1 when an entry vanishes while it walks.
            1,
        )?;
        Ok(parse_listing(&out))
    }

    /// Stream `paths` from the host into the volume.
    fn send(&self, helper: &str, paths: &[PathBuf]) -> io::Result<()> {
        let mut child = Command::new(self.cli)
            .args([
                "exec",
                "-i",
                helper,
                "tar",
                "-C",
                ROOT,
                "--no-same-owner",
                "-xf",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = drain(child.stderr.take());
        let stdin = child.stdin.take().expect("piped stdin");
        let written = (|| {
            let mut builder = tar::Builder::new(io::BufWriter::new(stdin));
            builder.follow_symlinks(false);
            for path in paths {
                builder.append_path_with_name(self.host.join(path), path)?;
            }
            builder.into_inner()?.flush()
        })();
        let status = child.wait()?;
        let stderr = stderr.join().unwrap_or_default();
        written?;
        if !status.success() {
            return Err(io::Error::other(format!("tar failed: {}", stderr.trim())));
        }
        Ok(())
    }

    /// Stream `paths` from the volume onto the host. Returns the entries the
    /// host refused.
    fn fetch(&self, helper: &str, paths: &[PathBuf], volume: &Listing) -> io::Result<Vec<PathBuf>> {
        let mut child = Command::new(self.cli)
            .args([
                "exec",
                "-i",
                helper,
                "tar",
                "-C",
                ROOT,
                "--null",
                "--no-recursion",
                "-T",
                "-",
                "-cf",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = drain(child.stderr.take());
        let mut stdin = child.stdin.take().expect("piped stdin");
        let names = nul_list(paths);
        // Written alongside the read: tar starts writing before it has read
        // every name, and both pipes are bounded.
        let feeder = std::thread::spawn(move || stdin.write_all(&names));
        let stdout = child.stdout.take().expect("piped stdout");
        let unpacked = unpack(stdout, &self.host, volume);
        let _ = feeder.join();
        let status = child.wait()?;
        let stderr = stderr.join().unwrap_or_default();
        let refused = unpacked?;
        // GNU tar exits 1 when a file changed while it was read, as one a
        // background process still writes may.
        if !status.success() && status.code() != Some(1) {
            return Err(io::Error::other(format!("tar failed: {}", stderr.trim())));
        }
        Ok(refused)
    }
}

/// What a copy into the volume does.
#[derive(Debug, Default, PartialEq)]
struct SyncIn {
    /// Volume entries to remove first: gone from the host, or of a new type.
    delete: Vec<PathBuf>,
    /// Host entries to copy, parents first.
    send: Vec<PathBuf>,
}

fn plan_sync_in(
    host: &Listing,
    volume: &Listing,
    kept: &HashSet<PathBuf>,
    on_host: impl Fn(&Path) -> bool,
) -> SyncIn {
    let mut delete: Vec<PathBuf> = volume
        .iter()
        .filter(|(path, entry)| match host.get(*path) {
            Some(theirs) => !theirs.kind.same_type(&entry.kind),
            // Absent from the listing but present on disk means the host
            // folds case and holds this name under another spelling.
            None => !kept.contains(*path) && !on_host(path),
        })
        .map(|(path, _)| path.clone())
        .collect();
    delete.sort();
    let mut outermost: Vec<PathBuf> = Vec::new();
    for path in delete {
        if !outermost.iter().any(|done| path.starts_with(done)) {
            outermost.push(path);
        }
    }
    let mut send: Vec<PathBuf> = host
        .iter()
        .filter(|(path, entry)| {
            let replaced = outermost.iter().any(|gone| path.starts_with(gone));
            replaced
                || volume
                    .get(*path)
                    .is_none_or(|theirs| !entry.matches(theirs))
        })
        .map(|(path, _)| path.clone())
        .collect();
    send.sort();
    SyncIn {
        delete: outermost,
        send,
    }
}

/// What a copy back to the host does.
#[derive(Debug, Default, PartialEq)]
struct SyncOut {
    /// Host entries the step removed.
    delete: Vec<PathBuf>,
    /// Volume entries to copy, parents first.
    fetch: Vec<PathBuf>,
}

fn plan_sync_out(host: &Listing, volume: &Listing, kept: &HashSet<PathBuf>) -> SyncOut {
    let mut delete: Vec<PathBuf> = host
        .keys()
        .filter(|path| !volume.contains_key(*path))
        .cloned()
        .collect();
    delete.sort();
    let mut outermost: Vec<PathBuf> = Vec::new();
    for path in delete {
        if !outermost.iter().any(|done| path.starts_with(done)) {
            outermost.push(path);
        }
    }
    let mut fetch: Vec<PathBuf> = volume
        .iter()
        .filter(|(path, entry)| {
            !kept.contains(*path) && host.get(*path).is_none_or(|ours| !ours.matches(entry))
        })
        .map(|(path, _)| path.clone())
        .collect();
    fetch.sort();
    SyncOut {
        delete: outermost,
        fetch,
    }
}

/// Parse `find -printf '%P\0%y\0%s\0%T@\0%m\0%l\0'`.
fn parse_listing(out: &[u8]) -> Listing {
    let mut listing = Listing::new();
    let mut fields = out.split(|b| *b == 0);
    while let (Some(path), Some(kind), Some(size), Some(mtime), Some(mode), Some(target)) = (
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
        fields.next(),
    ) {
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let kind = match kind {
            b"f" => Kind::File {
                size: text(size).parse().unwrap_or(0),
                mtime: text(mtime)
                    .split('.')
                    .next()
                    .and_then(|secs| secs.parse().ok())
                    .unwrap_or(0),
            },
            b"d" => Kind::Dir,
            b"l" => Kind::Link(path_from_bytes(target)),
            // Sockets, pipes and devices: tar can't carry them across.
            _ => continue,
        };
        let mode = u32::from_str_radix(&text(mode), 8).ok().map(|m| m & 0o777);
        let Some(path) = safe_relative_path(&path_from_bytes(path)) else {
            continue;
        };
        if path.as_os_str().is_empty() {
            continue;
        }
        listing.insert(path, Entry { kind, mode });
    }
    listing
}

#[cfg(unix)]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().replace('\\', "/").into_bytes()
}

fn nul_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths {
        out.extend(path_bytes(path));
        out.push(0);
    }
    out
}

fn host_entry(meta: &std::fs::Metadata, path: &Path) -> Option<Entry> {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        Kind::Link(std::fs::read_link(path).ok()?)
    } else if file_type.is_dir() {
        Kind::Dir
    } else if file_type.is_file() {
        Kind::File {
            size: meta.len(),
            mtime: mtime_secs(meta),
        }
    } else {
        return None;
    };
    Some(Entry {
        kind,
        mode: host_mode(meta),
    })
}

#[cfg(unix)]
fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.mtime()
}

#[cfg(not(unix))]
fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(unix)]
fn host_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn host_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Everything under `root`, without following links.
fn walk(root: &Path) -> io::Result<Listing> {
    let mut listing = Listing::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(rel) = pending.pop() {
        for entry in std::fs::read_dir(root.join(&rel))? {
            let entry = entry?;
            let child = rel.join(entry.file_name());
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let Some(found) = host_entry(&meta, &path) else {
                continue;
            };
            if found.kind == Kind::Dir {
                pending.push(child.clone());
            }
            listing.insert(child, found);
        }
    }
    Ok(listing)
}

/// Write the archive onto `host`, skipping entries it refuses. Nothing here
/// follows a symlink or leaves `host`: the volume is the workflow's to fill.
fn unpack<R: Read>(reader: R, host: &Path, volume: &Listing) -> io::Result<Vec<PathBuf>> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_mtime(true);
    let mut refused = Vec::new();
    let mut directories = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(rel) = safe_relative_path(&entry.path()?) else {
            continue;
        };
        if rel.as_os_str().is_empty() || refused.iter().any(|r| rel.starts_with(r)) {
            continue;
        }
        let is_dir = entry.header().entry_type().is_dir();
        let written = clear_type_conflict(host, &rel, is_dir).and_then(|()| {
            if is_dir {
                std::fs::create_dir_all(host.join(&rel))
            } else {
                entry.unpack_in(host).map(|_| ())
            }
        });
        match written {
            Ok(()) if is_dir => directories.push((rel, entry.header().mode().ok())),
            Ok(()) => {}
            Err(e) => {
                wrkflw_logging::debug(&format!(
                    "Workspace entry {} stays in the volume: {}",
                    rel.to_string_lossy(),
                    e
                ));
                refused.push(rel);
            }
        }
    }
    // Deepest first and after the files, so a read-only directory's mode is
    // only applied once its contents are in place.
    directories.sort_by_key(|(rel, _)| std::cmp::Reverse(rel.components().count()));
    for (rel, mode) in directories {
        set_dir_mode(&host.join(&rel), mode, volume.get(&rel));
    }
    Ok(refused)
}

#[cfg(unix)]
fn set_dir_mode(dir: &Path, mode: Option<u32>, _entry: Option<&Entry>) {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode & 0o777));
    }
}

#[cfg(not(unix))]
fn set_dir_mode(_dir: &Path, _mode: Option<u32>, _entry: Option<&Entry>) {}

fn drain(stderr: Option<std::process::ChildStderr>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    })
}

fn exec(cli: &str, helper: &str, cmd: &[&str], stdin: Option<Vec<u8>>) -> io::Result<Vec<u8>> {
    exec_allowing(cli, helper, cmd, stdin, 0)
}

/// Run `cmd` in the helper; exit codes up to `allowed` count as success.
fn exec_allowing(
    cli: &str,
    helper: &str,
    cmd: &[&str],
    stdin: Option<Vec<u8>>,
    allowed: i32,
) -> io::Result<Vec<u8>> {
    let mut args = vec!["exec"];
    if stdin.is_some() {
        args.push("-i");
    }
    args.push(helper);
    args.extend_from_slice(cmd);
    let mut child = Command::new(cli)
        .args(&args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let feeder = match (stdin, child.stdin.take()) {
        (Some(bytes), Some(mut pipe)) => Some(std::thread::spawn(move || pipe.write_all(&bytes))),
        _ => None,
    };
    let output = child.wait_with_output()?;
    if let Some(feeder) = feeder {
        let _ = feeder.join();
    }
    match output.status.code() {
        Some(code) if code <= allowed => Ok(output.stdout),
        _ => Err(io::Error::other(format!(
            "`{}` failed: {}",
            cmd.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn track(cli: &str, id: &str) {
    if cli == "podman" {
        crate::podman::track_container(id);
    } else {
        crate::docker::track_container(id);
    }
}

fn remove_container(cli: &str, id: &str) {
    let _ = Command::new(cli)
        .args(["rm", "-f", id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if cli == "podman" {
        crate::podman::untrack_container(id);
    } else {
        crate::docker::untrack_container(id);
    }
}

/// The job that used the workspace at `host` is over: stop its helper. A
/// workspace that isn't kept takes its volume with it, once the job's own
/// container lets go of it.
pub(crate) fn release(host: &Path, kept: bool) {
    let workspace = WORKSPACES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(host);
    let Some(workspace) = workspace else {
        return;
    };
    let helper = workspace.lock().helper.take();
    if let Some(helper) = helper {
        remove_container(workspace.cli, &helper);
    }
    if kept {
        return;
    }
    let (cli, volume) = (workspace.cli, workspace.volume.clone());
    // The job container is removed on a thread of its own, so the volume
    // may still be in use for a moment. `wrkflw clean` gets any it misses.
    std::thread::spawn(move || {
        for _ in 0..40 {
            let removed = Command::new(cli)
                .args(["volume", "rm", "-f", &volume])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            if removed {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
}

/// Remove workspace volumes whose host directory is gone. Returns how many.
pub(crate) fn remove_orphaned_volumes() -> usize {
    let mut removed = 0;
    for cli in ["docker", "podman"] {
        let Ok(listed) = Command::new(cli)
            .args([
                "volume",
                "ls",
                "--filter",
                &format!("label={}", LABEL),
                "--format",
                &format!("{{{{.Name}}}}\t{{{{index .Labels \"{}\"}}}}", LABEL),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            continue;
        };
        if !listed.status.success() {
            continue;
        }
        for line in String::from_utf8_lossy(&listed.stdout).lines() {
            let Some((name, host)) = line.split_once('\t') else {
                continue;
            };
            if !name.starts_with(VOLUME_PREFIX) || Path::new(host).is_dir() {
                continue;
            }
            let gone = Command::new(cli)
                .args(["volume", "rm", name])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            removed += usize::from(gone);
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(size: u64, mtime: i64) -> Entry {
        Entry {
            kind: Kind::File { size, mtime },
            mode: Some(0o644),
        }
    }

    fn dir() -> Entry {
        Entry {
            kind: Kind::Dir,
            mode: Some(0o755),
        }
    }

    fn listing(entries: &[(&str, Entry)]) -> Listing {
        entries
            .iter()
            .map(|(path, entry)| (PathBuf::from(path), entry.clone()))
            .collect()
    }

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_volume_is_used_off_linux_unless_bind_is_asked_for() {
        assert!(wanted(false, None));
        assert!(!wanted(true, None));
        assert!(!wanted(false, Some("bind")));
        assert!(wanted(true, Some("volume")));
        assert!(wanted(false, Some("anything else")));
    }

    #[test]
    fn each_workspace_has_its_own_volume() {
        let a = volume_name(Path::new("/w/a"));
        assert_eq!(a, volume_name(Path::new("/w/a")));
        assert_ne!(a, volume_name(Path::new("/w/b")));
        assert!(a.starts_with(VOLUME_PREFIX));
    }

    #[test]
    fn a_listing_keeps_names_that_are_not_utf8_and_skips_sockets() {
        let mut out = Vec::new();
        for field in [
            &b"src"[..],
            b"d",
            b"4096",
            b"1700000000.5",
            b"755",
            b"",
            b"src/\xff",
            b"f",
            b"3",
            b"1700000001.9999",
            b"644",
            b"",
            b"link",
            b"l",
            b"3",
            b"1700000002.0",
            b"777",
            b"src",
            b"sock",
            b"s",
            b"0",
            b"1700000003.0",
            b"755",
            b"",
        ] {
            out.extend_from_slice(field);
            out.push(0);
        }
        let parsed = parse_listing(&out);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[Path::new("src")], dir());
        assert_eq!(
            parsed[&path_from_bytes(b"src/\xff")],
            file(3, 1_700_000_001)
        );
        assert_eq!(
            parsed[Path::new("link")].kind,
            Kind::Link(PathBuf::from("src"))
        );
    }

    #[test]
    fn copying_in_sends_what_changed_and_removes_what_the_host_dropped() {
        let host = listing(&[
            ("src", dir()),
            ("src/same", file(1, 10)),
            ("src/edited", file(2, 20)),
            ("src/new", file(3, 30)),
        ]);
        let volume = listing(&[
            ("src", dir()),
            ("src/same", file(1, 10)),
            ("src/edited", file(2, 19)),
            ("old", dir()),
            ("old/inner", file(1, 1)),
        ]);
        let plan = plan_sync_in(&host, &volume, &HashSet::new(), |_| false);
        assert_eq!(plan.delete, paths(&["old"]));
        assert_eq!(plan.send, paths(&["src/edited", "src/new"]));
    }

    #[test]
    fn copying_in_never_deletes_what_the_host_could_not_hold() {
        let host = listing(&[("src", dir())]);
        let volume = listing(&[
            ("src", dir()),
            ("src/unrepresentable", file(1, 1)),
            ("src/Readme", file(1, 1)),
        ]);
        let kept: HashSet<PathBuf> = paths(&["src/unrepresentable"]).into_iter().collect();
        // The host folds case: it holds `src/Readme` as `src/README`.
        let plan = plan_sync_in(&host, &volume, &kept, |p| p == Path::new("src/Readme"));
        assert_eq!(plan, SyncIn::default());
    }

    #[test]
    fn copying_in_replaces_an_entry_that_changed_type() {
        let host = listing(&[("x", file(1, 1))]);
        let volume = listing(&[("x", dir()), ("x/inner", file(1, 1))]);
        let plan = plan_sync_in(&host, &volume, &HashSet::new(), |_| false);
        assert_eq!(plan.delete, paths(&["x"]));
        assert_eq!(plan.send, paths(&["x"]));
    }

    #[test]
    fn copying_out_fetches_changes_and_removes_what_the_step_deleted() {
        let host = listing(&[
            ("a", file(1, 1)),
            ("gone", dir()),
            ("gone/inner", file(1, 1)),
            ("chmod", file(1, 1)),
        ]);
        let mut executable = file(1, 1);
        executable.mode = Some(0o755);
        let volume = listing(&[
            ("a", file(1, 1)),
            ("chmod", executable),
            ("made", file(5, 5)),
            ("refused", file(1, 1)),
        ]);
        let kept: HashSet<PathBuf> = paths(&["refused"]).into_iter().collect();
        let plan = plan_sync_out(&host, &volume, &kept);
        assert_eq!(plan.delete, paths(&["gone"]));
        assert_eq!(plan.fetch, paths(&["chmod", "made"]));
    }

    #[test]
    fn unpacking_skips_entries_the_host_refuses_and_writes_the_rest() {
        let host = tempfile::tempdir().unwrap();
        // A file where the archive wants a directory's parent: refused.
        std::fs::write(host.path().join("blocker"), "x").unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o644);
        header.set_mtime(1_700_000_000);
        header.set_entry_type(tar::EntryType::Regular);
        builder
            .append_data(&mut header.clone(), "blocker/child", &b"hi"[..])
            .unwrap();
        builder
            .append_data(&mut header, "fine", &b"ok"[..])
            .unwrap();
        let archive = builder.into_inner().unwrap();
        let refused = unpack(&archive[..], host.path(), &Listing::new()).unwrap();
        assert_eq!(refused, paths(&["blocker/child"]));
        assert_eq!(std::fs::read(host.path().join("fine")).unwrap(), b"ok");
        let meta = std::fs::metadata(host.path().join("fine")).unwrap();
        assert_eq!(
            host_entry(&meta, &host.path().join("fine")).unwrap(),
            file(2, 1_700_000_000)
        );
    }

    fn docker_ready() -> bool {
        std::env::var("WRKFLW_TEST_SKIP_DOCKER").is_err()
            && Command::new("docker")
                .args(["info"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    }

    /// A step in the volume can do what a Linux filesystem allows and the
    /// host's may not, and its other changes come back to the host.
    #[test]
    fn a_step_in_the_volume_round_trips_through_the_host() {
        if !docker_ready() {
            eprintln!("skipping: Docker is not available");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("ws");
        std::fs::create_dir_all(host.join("src")).unwrap();
        std::fs::write(host.join("src/keep.txt"), "kept").unwrap();
        std::fs::write(host.join("src/remove.txt"), "bye").unwrap();
        let workspace = Arc::new(VmWorkspace {
            cli: "docker",
            host: host.clone(),
            volume: volume_name(&host),
            volume_path: PathBuf::from(volume_name(&host)),
            state: Mutex::new(State {
                helper: None,
                volume: None,
                kept: HashSet::new(),
                unusable: false,
            }),
        });
        WORKSPACES
            .lock()
            .unwrap()
            .insert(host.clone(), workspace.clone());
        assert!(workspace.sync_in().unwrap());
        let helper = workspace.lock().helper.clone().unwrap();
        let step = "cd /ws && test \"$(cat src/keep.txt)\" = kept \
                    && rm src/remove.txt && echo new > src/new.txt \
                    && touch \"$(printf 'src/bad\\377')\" && touch src/Case src/case \
                    && ln -s keep.txt src/link";
        exec(workspace.cli, &helper, &["sh", "-c", step], None).unwrap();
        workspace.sync_out().unwrap();

        assert!(!host.join("src/remove.txt").exists());
        assert_eq!(
            std::fs::read_to_string(host.join("src/new.txt")).unwrap(),
            "new\n"
        );
        assert_eq!(
            std::fs::read_link(host.join("src/link")).unwrap(),
            PathBuf::from("keep.txt")
        );
        // A host edit goes back in, and nothing the host lacks is lost.
        std::fs::write(host.join("src/keep.txt"), "edited").unwrap();
        assert!(workspace.sync_in().unwrap());
        let check = "cd /ws && test \"$(cat src/keep.txt)\" = edited \
                     && test -e \"$(printf 'src/bad\\377')\" && test -e src/Case && test -e src/case";
        exec(workspace.cli, &helper, &["sh", "-c", check], None).unwrap();

        release(&host, false);
        drop(dir);
    }
}
