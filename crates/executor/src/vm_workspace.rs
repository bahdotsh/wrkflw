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
//! cannot hold, such as a name that is not UTF-8, stays in the volume with
//! everything inside it, and is never deleted from it for being absent on
//! the host. A host directory that can't be read is left alone both ways.
//!
//! An entry the host refuses for a reason that may pass, such as a directory
//! a step made read-only or a full disk, is not kept: it is left alone by the
//! next copy in and tried again by the next copy out. An entry the volume's
//! listing couldn't reach is left alone both ways, like an unreadable host
//! directory. Kept entries are remembered between runs of a kept workspace.
//!
//! Two limits follow from the listings. A file rewritten on the host in the
//! same second as a step wrote it, at the same size and mode, looks
//! unchanged, as it does to rsync's quick check. And on a host that folds
//! case, of two names that differ only in case the host holds the first one
//! copied back; the other stays in the volume only, so host-side work, such
//! as artifact uploads, doesn't see it.
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
    /// Volume entries the host refused for a reason that may pass. The next
    /// copy in leaves them alone, and the next copy out tries them again.
    pending: HashSet<PathBuf>,
    /// Volume entries the last listing couldn't reach. Neither direction
    /// touches them, as for a host directory that can't be read.
    unlisted: HashSet<PathBuf>,
    /// Set when the volume couldn't be set up before any step used it: the
    /// job then mounts the host directory as before.
    unusable: bool,
    /// Set once a step has seen the volume. From then on the volume holds
    /// what steps wrote, so a failed sync fails the step instead of falling
    /// back to the host copy, which lacks it.
    used: bool,
}

impl State {
    fn new() -> Self {
        State {
            helper: None,
            volume: None,
            kept: HashSet::new(),
            pending: HashSet::new(),
            unlisted: HashSet::new(),
            unusable: false,
            used: false,
        }
    }
}

/// Whether `path` is a kept entry or inside one. A directory the host
/// refused keeps everything in it in the volume too.
fn is_kept(kept: &HashSet<PathBuf>, path: &Path) -> bool {
    path.ancestors().any(|ancestor| kept.contains(ancestor))
}

/// `listing` without the entries in `set` and everything inside them.
fn outside(listing: &Listing, set: &HashSet<PathBuf>) -> Listing {
    if set.is_empty() {
        return listing.clone();
    }
    listing
        .iter()
        .filter(|(path, _)| !is_kept(set, path))
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect()
}

/// Whether `path` is one of `sorted`, a sorted list where no entry is
/// inside another, or inside one.
fn under_any(sorted: &[PathBuf], path: &Path) -> bool {
    // Paths order component by component, so the only candidate is the
    // greatest entry not after `path`.
    let at = sorted.partition_point(|p| p.as_path() <= path);
    at > 0 && path.starts_with(&sorted[at - 1])
}

/// The outermost of `sorted` paths: those not inside another one.
fn outermost(sorted: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for path in sorted {
        // A path's descendants sort right after it, so only the last kept
        // path can contain this one.
        if !out.last().is_some_and(|last| path.starts_with(last)) {
            out.push(path);
        }
    }
    out
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
                let mut state = State::new();
                state.kept = load_kept(kept_file(&volume).as_deref());
                Arc::new(VmWorkspace {
                    cli,
                    host: host.to_path_buf(),
                    volume_path: PathBuf::from(&volume),
                    volume,
                    state: Mutex::new(state),
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
        let result = self.sync_in_locked(&mut state);
        match result {
            Err(e) if !state.used => {
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
            Ok(()) => {
                state.used = true;
                Ok(true)
            }
        }
    }

    fn sync_in_locked(&self, state: &mut State) -> io::Result<()> {
        let helper = self.helper(state)?;
        let volume = match state.volume.take() {
            Some(listing) => listing,
            None => {
                let (listing, unlisted) = self.list_volume(&helper)?;
                state.unlisted = unlisted;
                listing
            }
        };
        let (host, unreadable) = walk(&self.host)?;
        // What the host refused for now, and what the listing couldn't
        // reach, may hold a step's writes the host doesn't have yet.
        let mut hidden = state.pending.clone();
        hidden.extend(state.unlisted.iter().cloned());
        let plan = plan_sync_in(
            &outside(&host, &hidden),
            &outside(&without_unreadable(&volume, &unreadable), &hidden),
            &state.kept,
            |rel| std::fs::symlink_metadata(self.host.join(rel)).is_ok(),
        );
        if !plan.delete.is_empty() {
            exec(
                self.cli,
                &helper,
                &["sh", "-c", &format!("cd {} && xargs -0 -r rm -rf --", ROOT)],
                Some(nul_list(&plan.delete)),
            )?;
        }
        if !plan.send.is_empty() {
            self.send(&helper, &plan.send, &volume)?;
        }
        let mut volume = volume;
        for gone in &plan.delete {
            volume.retain(|path, _| !path.starts_with(gone));
        }
        for path in &plan.send {
            if let Some(entry) = host.get(path) {
                let mut sent = entry.clone();
                // A host without permission bits sent the mode
                // `append_host_entry` chose, which the volume now has.
                if sent.mode.is_none() && matches!(sent.kind, Kind::File { .. }) {
                    sent.mode = Some(volume.get(path).and_then(|e| e.mode).unwrap_or(0o755));
                }
                volume.insert(path.clone(), sent);
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
        let (volume, unlisted) = self.list_volume(&helper)?;
        state.unlisted = unlisted;
        let (host, unreadable) = walk(&self.host)?;
        // An entry the listing couldn't reach is not one the step deleted.
        let plan = plan_sync_out(
            &outside(&host, &state.unlisted),
            &outside(&without_unreadable(&volume, &unreadable), &state.unlisted),
            &state.kept,
        );
        for path in &plan.delete {
            // Left on the host, the next sync would copy it back into the
            // volume as if the step had never removed it.
            remove_host_entry(&self.host.join(path))?;
        }
        let kept_before = state.kept.clone();
        let mut pending = HashSet::new();
        if !plan.fetch.is_empty() {
            let Unpacked { refused, failed } = self.fetch(&helper, &plan.fetch, &volume)?;
            if !failed.is_empty() {
                let mut names: Vec<String> = failed
                    .iter()
                    .take(3)
                    .map(|(path, e)| format!("{} ({})", path.to_string_lossy(), e))
                    .collect();
                if failed.len() > 3 {
                    names.push(format!("and {} more", failed.len() - 3));
                }
                wrkflw_logging::warning(&format!(
                    "{} workspace entr{} couldn't be copied back to this machine, so host-side \
                     work such as artifact uploads won't see {} until a later step's copy \
                     succeeds: {}",
                    failed.len(),
                    if failed.len() == 1 { "y" } else { "ies" },
                    if failed.len() == 1 { "it" } else { "them" },
                    names.join(", ")
                ));
                pending.extend(failed.into_iter().map(|(path, _)| path));
            }
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
        state.pending = pending;
        state.kept.retain(|path| volume.contains_key(path));
        if state.kept != kept_before {
            save_kept(kept_file(&self.volume).as_deref(), &state.kept);
        }
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

    /// The volume's entries, and those `find` couldn't reach.
    fn list_volume(&self, helper: &str) -> io::Result<(Listing, HashSet<PathBuf>)> {
        let cmd = [
            "find",
            ROOT,
            "-mindepth",
            "1",
            "-printf",
            "%P\\0%y\\0%s\\0%T@\\0%m\\0%l\\0",
        ];
        let out = exec_output(self.cli, helper, &cmd, None)?;
        let unlisted = match out.status.code() {
            Some(0) => HashSet::new(),
            // find exits 1 for any entry it couldn't list, not only one
            // that vanished as it walked. Each says which.
            Some(1) => unlisted_entries(&out.stderr).map_err(io::Error::other)?,
            _ => return Err(command_failed(&cmd, &out.stderr)),
        };
        if !unlisted.is_empty() {
            let mut names: Vec<String> = unlisted
                .iter()
                .take(3)
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            names.sort();
            wrkflw_logging::warning(&format!(
                "Part of the workspace volume couldn't be listed, so it isn't synced either way: \
                 {}",
                names.join(", ")
            ));
        }
        Ok((parse_listing(&out.stdout), unlisted))
    }

    /// Stream `paths` from the host into the volume.
    fn send(&self, helper: &str, paths: &[PathBuf], volume: &Listing) -> io::Result<()> {
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
                append_host_entry(&mut builder, &self.host.join(path), path, volume.get(path))?;
            }
            builder.into_inner()?.flush()
        })();
        let status = child.wait()?;
        let stderr = stderr.join().unwrap_or_default();
        // When tar dies first, writing fails with a broken pipe, which says
        // nothing; tar's own message says why.
        if !status.success() {
            return Err(io::Error::other(format!("tar failed: {}", stderr.trim())));
        }
        written
    }

    /// Stream `paths` from the volume onto the host. Returns the entries the
    /// host refused.
    fn fetch(&self, helper: &str, paths: &[PathBuf], volume: &Listing) -> io::Result<Unpacked> {
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
        let unpacked = unpacked?;
        // GNU tar exits 1 when a file changed while it was read, as one a
        // background process still writes may.
        if !status.success() && status.code() != Some(1) {
            return Err(io::Error::other(format!("tar failed: {}", stderr.trim())));
        }
        Ok(unpacked)
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
    // Absent from the listing but present on disk means the host folds case
    // and holds this name, or a directory above it, under another spelling.
    // What is inside such a directory is the volume's, not deleted.
    let mut spelled_otherwise: HashMap<PathBuf, bool> = HashMap::new();
    let mut held_elsewhere = |path: &Path| {
        path.ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .any(|ancestor| {
                !host.contains_key(ancestor)
                    && *spelled_otherwise
                        .entry(ancestor.to_path_buf())
                        .or_insert_with(|| on_host(ancestor))
            })
    };
    let mut delete: Vec<PathBuf> = Vec::new();
    for (path, entry) in volume {
        let gone = match host.get(path) {
            Some(theirs) => !theirs.kind.same_type(&entry.kind),
            None => !is_kept(kept, path) && !held_elsewhere(path),
        };
        if gone {
            delete.push(path.clone());
        }
    }
    delete.sort();
    let outermost = outermost(delete);
    let mut send: Vec<PathBuf> = host
        .iter()
        .filter(|(path, entry)| {
            let replaced = under_any(&outermost, path);
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
    let outermost = outermost(delete);
    let mut fetch: Vec<PathBuf> = volume
        .iter()
        .filter(|(path, entry)| {
            !is_kept(kept, path) && host.get(*path).is_none_or(|ours| !ours.matches(entry))
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
        Kind::Link(link_target(std::fs::read_link(path).ok()?))
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

/// A link target as `find -printf %l` prints it in the volume.
fn link_target(target: PathBuf) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(target.to_string_lossy().replace('\\', "/"))
    } else {
        target
    }
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

/// Everything under `root`, without following links, and the directories
/// below it that couldn't be read. Those are listed but not their contents.
fn walk(root: &Path) -> io::Result<(Listing, Vec<PathBuf>)> {
    let mut listing = Listing::new();
    let mut unreadable = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(rel) = pending.pop() {
        let entries = match std::fs::read_dir(root.join(&rel)) {
            Ok(entries) => entries,
            // The workspace itself must be readable; a directory in it a
            // step made unreadable is left alone rather than failing the job.
            Err(e) if !rel.as_os_str().is_empty() => {
                wrkflw_logging::debug(&format!(
                    "Workspace directory {} can't be read, so it isn't synced: {}",
                    rel.to_string_lossy(),
                    e
                ));
                unreadable.push(rel);
                continue;
            }
            Err(e) => return Err(e),
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
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
    Ok((listing, unreadable))
}

/// `volume` without what is inside a host directory that couldn't be read.
/// The host's side of those is unknown, so neither direction touches them.
fn without_unreadable(volume: &Listing, unreadable: &[PathBuf]) -> Listing {
    if unreadable.is_empty() {
        return volume.clone();
    }
    volume
        .iter()
        .filter(|(path, _)| {
            !unreadable
                .iter()
                .any(|dir| path.starts_with(dir) && path.as_path() != dir.as_path())
        })
        .map(|(path, entry)| (path.clone(), entry.clone()))
        .collect()
}

/// Remove `target` from the host, making read-only directories in it
/// writable first if that is what stops it.
fn remove_host_entry(target: &Path) -> io::Result<()> {
    let remove = || match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    };
    match remove() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            make_writable(target);
            remove().map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("could not remove {}: {}", target.display(), e),
                )
            })
        }
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!("could not remove {}: {}", target.display(), e),
        )),
    }
}

/// Let the owner change `path` and, for a directory, everything in it.
/// Links are not followed.
fn make_writable(path: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    let mut permissions = meta.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let extra = if meta.is_dir() { 0o700 } else { 0o200 };
        permissions.set_mode(permissions.mode() | extra);
    }
    #[cfg(not(unix))]
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    let _ = std::fs::set_permissions(path, permissions);
    if meta.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                make_writable(&entry.path());
            }
        }
    }
}

/// Add the host entry at `full` to `builder` as `rel`. A host without
/// permission bits gives a file the mode it has in the volume, or 0755 for
/// a new one, as the file share it replaces showed every file executable.
fn append_host_entry<W: Write>(
    builder: &mut tar::Builder<W>,
    full: &Path,
    rel: &Path,
    _in_volume: Option<&Entry>,
) -> io::Result<()> {
    #[cfg(not(unix))]
    {
        let meta = std::fs::symlink_metadata(full)?;
        if meta.is_file() {
            let mut header = tar::Header::new_gnu();
            header.set_metadata(&meta);
            header.set_mode(_in_volume.and_then(|e| e.mode).unwrap_or(0o755));
            return builder.append_data(&mut header, rel, std::fs::File::open(full)?);
        }
    }
    builder.append_path_with_name(full, rel)
}

/// What copying entries back to the host did not write.
#[derive(Debug, Default)]
struct Unpacked {
    /// Entries the host can't hold, such as a name that isn't valid there.
    refused: Vec<PathBuf>,
    /// Entries the host refused for a reason that may pass, with the error.
    failed: Vec<(PathBuf, io::Error)>,
}

/// True when `e` says the host can't hold the entry at all, rather than
/// that it couldn't write it this time.
fn cannot_hold(e: &io::Error) -> bool {
    use io::ErrorKind::{InvalidFilename, InvalidInput, IsADirectory, NotADirectory};
    matches!(
        e.kind(),
        InvalidFilename | InvalidInput | IsADirectory | NotADirectory
    ) || os_error(e).is_some_and(|code| Some(code) == illegal_name_code())
}

/// The OS error for a name the filesystem doesn't accept: `EILSEQ`, which
/// macOS gives for a name that isn't UTF-8, or Windows' invalid name.
fn illegal_name_code() -> Option<i32> {
    if cfg!(any(
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )) {
        Some(92)
    } else if cfg!(any(target_os = "linux", target_os = "android")) {
        Some(84)
    } else if cfg!(windows) {
        Some(123)
    } else {
        None
    }
}

/// The OS error code behind `e`, through the errors tar wraps it in.
fn os_error(e: &io::Error) -> Option<i32> {
    let mut current = e;
    loop {
        if let Some(code) = current.raw_os_error() {
            return Some(code);
        }
        let mut source: Option<&(dyn std::error::Error + 'static)> = Some(current.get_ref()?);
        let mut next = None;
        while let Some(err) = source {
            if let Some(io) = err.downcast_ref::<io::Error>() {
                next = Some(io);
                break;
            }
            source = err.source();
        }
        current = next?;
    }
}

/// True when the host already holds `rel` under another spelling, as a
/// filesystem that folds case does. `names` caches directory listings.
fn spelled_otherwise_on_host(
    host: &Path,
    rel: &Path,
    names: &mut HashMap<PathBuf, Option<HashSet<std::ffi::OsString>>>,
) -> bool {
    let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) else {
        return false;
    };
    if std::fs::symlink_metadata(host.join(rel)).is_err() {
        return false;
    }
    let listed = names.entry(parent.to_path_buf()).or_insert_with(|| {
        std::fs::read_dir(host.join(parent)).ok().map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.file_name()))
                .collect()
        })
    });
    listed.as_ref().is_some_and(|listed| !listed.contains(name))
}

/// Write the archive onto `host`, skipping entries it refuses. Nothing here
/// follows a symlink or leaves `host`: the volume is the workflow's to fill.
fn unpack<R: Read>(reader: R, host: &Path, volume: &Listing) -> io::Result<Unpacked> {
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_mtime(true);
    let mut unpacked = Unpacked::default();
    let mut directories = Vec::new();
    let mut names = HashMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(rel) = safe_relative_path(&entry.path()?) else {
            continue;
        };
        let skipped = unpacked.refused.iter().any(|r| rel.starts_with(r))
            || unpacked.failed.iter().any(|(r, _)| rel.starts_with(r));
        if rel.as_os_str().is_empty() || skipped {
            continue;
        }
        let is_dir = entry.header().entry_type().is_dir();
        let written = if let Err(e) = refuse_link_parents(host, &rel) {
            Err((e, true))
        } else if spelled_otherwise_on_host(host, &rel, &mut names) {
            let e = io::Error::other("the host holds this name in another case");
            Err((e, true))
        } else {
            clear_type_conflict(host, &rel, is_dir)
                .and_then(|()| {
                    if is_dir {
                        std::fs::create_dir_all(host.join(&rel))
                    } else {
                        entry.unpack_in(host).map(|_| ())
                    }
                })
                .map_err(|e| {
                    let permanent = cannot_hold(&e);
                    (e, permanent)
                })
        };
        match written {
            Ok(()) => {
                if let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) {
                    if let Some(Some(listed)) = names.get_mut(parent) {
                        listed.insert(name.to_os_string());
                    }
                }
                if is_dir {
                    directories.push((rel, entry.header().mode().ok()));
                }
            }
            Err((e, true)) => {
                wrkflw_logging::debug(&format!(
                    "Workspace entry {} stays in the volume: {}",
                    rel.to_string_lossy(),
                    e
                ));
                unpacked.refused.push(rel);
            }
            Err((e, false)) => unpacked.failed.push((rel, e)),
        }
    }
    // Deepest first and after the files, so a read-only directory's mode is
    // only applied once its contents are in place.
    directories.sort_by_key(|(rel, _)| std::cmp::Reverse(rel.components().count()));
    for (rel, mode) in directories {
        set_dir_mode(&host.join(&rel), mode, volume.get(&rel));
    }
    Ok(unpacked)
}

/// The entries a failed `find` names, unless it only names entries that
/// vanished as it walked. An error naming nothing in the volume, or naming
/// it in a form that can't be read back, means the listing can't be
/// trusted, and that is an error.
fn unlisted_entries(stderr: &[u8]) -> Result<HashSet<PathBuf>, String> {
    let text = String::from_utf8_lossy(stderr);
    let prefix = format!("{}/", ROOT);
    let mut unlisted = HashSet::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let untrusted = || format!("the workspace volume's listing is incomplete: {}", line);
        let rest = line.strip_prefix("find: ").ok_or_else(untrusted)?;
        // GNU find quotes names with '…' in the C locale, ‘…’ in others.
        let (body, close) = if let Some(body) = rest.strip_prefix('\'') {
            (body, "': ")
        } else if let Some(body) = rest.strip_prefix('\u{2018}') {
            (body, "\u{2019}: ")
        } else {
            return Err(untrusted());
        };
        let end = body.rfind(close).ok_or_else(untrusted)?;
        let (path, reason) = (&body[..end], &body[end + close.len()..]);
        if reason.trim() == "No such file or directory" {
            continue;
        }
        // find escapes what it can't print, so such a name isn't the
        // entry's; nor is a name holding a quote, which it would have
        // quoted differently.
        let rel = path
            .strip_prefix(&prefix)
            .filter(|rel| !rel.is_empty())
            .filter(|rel| !rel.contains(['\\', '\'', '\u{FFFD}']))
            .ok_or_else(untrusted)?;
        unlisted.insert(PathBuf::from(rel));
    }
    Ok(unlisted)
}

/// Where the entries kept in `volume` are remembered: a kept workspace's
/// volume outlives the run, and without this its next run would delete
/// them for being absent on the host.
fn kept_file(volume: &str) -> Option<PathBuf> {
    Some(
        dirs::cache_dir()?
            .join("wrkflw")
            .join("workspace-kept")
            .join(volume),
    )
}

fn load_kept(file: Option<&Path>) -> HashSet<PathBuf> {
    let Some(bytes) = file.and_then(|file| std::fs::read(file).ok()) else {
        return HashSet::new();
    };
    bytes
        .split(|b| *b == 0)
        .filter(|path| !path.is_empty())
        .map(path_from_bytes)
        .filter_map(|path| safe_relative_path(&path))
        .filter(|path| !path.as_os_str().is_empty())
        .collect()
}

fn save_kept(file: Option<&Path>, kept: &HashSet<PathBuf>) {
    let Some(file) = file else {
        return;
    };
    if kept.is_empty() {
        let _ = std::fs::remove_file(file);
        return;
    }
    let mut sorted: Vec<PathBuf> = kept.iter().cloned().collect();
    sorted.sort();
    let saved = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(file, nul_list(&sorted)));
    if let Err(e) = saved {
        wrkflw_logging::debug(&format!(
            "Could not remember the workspace entries kept in the volume: {}",
            e
        ));
    }
}

/// Refuse to write `rel` when a directory above it on the host is a link,
/// so nothing the volume holds lands outside the workspace, or is a file,
/// which no later step makes room for. The OS error for a file parent isn't
/// the same everywhere (Windows says the path wasn't found), so it is
/// decided here rather than from the failed write.
fn refuse_link_parents(host: &Path, rel: &Path) -> io::Result<()> {
    let mut dir = host.to_path_buf();
    let parents = rel.parent().into_iter().flat_map(|p| p.components());
    for component in parents {
        dir.push(component);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(io::Error::other(format!(
                    "{} is a link on the host",
                    dir.display()
                )))
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(io::Error::other(format!(
                    "{} is a file on the host",
                    dir.display()
                )))
            }
            Ok(_) => {}
            // Missing parents are created as directories.
            Err(_) => return Ok(()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn set_dir_mode(dir: &Path, mode: Option<u32>, _entry: Option<&Entry>) {
    use std::os::unix::fs::PermissionsExt;
    let is_dir = std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir());
    if let (Some(mode), true) = (mode, is_dir) {
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

/// Run `cmd` in the helper; it must succeed.
fn exec(cli: &str, helper: &str, cmd: &[&str], stdin: Option<Vec<u8>>) -> io::Result<Vec<u8>> {
    let output = exec_output(cli, helper, cmd, stdin)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(command_failed(cmd, &output.stderr))
    }
}

fn command_failed(cmd: &[&str], stderr: &[u8]) -> io::Error {
    io::Error::other(format!(
        "`{}` failed: {}",
        cmd.join(" "),
        String::from_utf8_lossy(stderr).trim()
    ))
}

/// Run `cmd` in the helper and return what it did, whatever its exit code.
fn exec_output(
    cli: &str,
    helper: &str,
    cmd: &[&str],
    stdin: Option<Vec<u8>>,
) -> io::Result<std::process::Output> {
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
    Ok(output)
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
    let (cli, volume) = (workspace.cli, workspace.volume.clone());
    // This runs as a job ends, on the async runtime, so everything that
    // blocks goes to a thread: the engine calls, and the workspace's lock,
    // which a sync of a cancelled step may still hold. The job container is
    // removed on a thread of its own too, so the volume may still be in use
    // for a while. The run waits for this before it ends; `wrkflw clean`
    // gets any it misses.
    let removal = std::thread::spawn(move || {
        let helper = workspace.lock().helper.take();
        if let Some(helper) = helper {
            remove_container(cli, &helper);
        }
        if kept {
            return;
        }
        save_kept(kept_file(&volume).as_deref(), &HashSet::new());
        for _ in 0..VOLUME_REMOVAL_TRIES {
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
    PENDING_REMOVALS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(removal);
}

/// Tries to remove a fresh workspace's volume, 250 ms apart: the job's own
/// container, and its services, are removed alongside, which can take a
/// Podman machine a while.
const VOLUME_REMOVAL_TRIES: usize = 120;

static PENDING_REMOVALS: Mutex<Vec<std::thread::JoinHandle<()>>> = Mutex::new(Vec::new());

/// Remove the volume and helper of the workspace at `host` now, as an
/// interrupted run exits.
pub(crate) fn remove_now(host: &Path) {
    let workspace = WORKSPACES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(host);
    let Some(workspace) = workspace else {
        return;
    };
    // A sync may hold the lock; the helper is tracked and goes with the
    // rest of the run's containers anyway.
    if let Ok(mut state) = workspace.state.try_lock() {
        if let Some(helper) = state.helper.take() {
            remove_container(workspace.cli, &helper);
        }
    }
    let _ = Command::new(workspace.cli)
        .args(["volume", "rm", "-f", &workspace.volume])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    save_kept(kept_file(&workspace.volume).as_deref(), &HashSet::new());
}

/// Wait for the volumes released so far to be removed, so a run that ends
/// right after its last job doesn't leave that job's volume behind.
pub(crate) async fn wait_for_released_volumes() {
    let pending = std::mem::take(&mut *PENDING_REMOVALS.lock().unwrap_or_else(|e| e.into_inner()));
    if pending.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        for removal in pending {
            let _ = removal.join();
        }
    })
    .await;
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
            if gone {
                save_kept(kept_file(name).as_deref(), &HashSet::new());
            }
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
    fn what_is_inside_a_directory_the_host_refused_stays_in_the_volume() {
        // The host couldn't hold `bad`, so it never saw what is in it.
        let host = listing(&[("src", dir())]);
        let volume = listing(&[
            ("src", dir()),
            ("src/bad", dir()),
            ("src/bad/out.txt", file(1, 10)),
            ("src/bad/deep", dir()),
            ("src/bad/deep/more", file(1, 10)),
        ]);
        let kept = HashSet::from([PathBuf::from("src/bad")]);
        let plan = plan_sync_in(&host, &volume, &kept, |_| false);
        assert_eq!(plan, SyncIn::default());
        // Nor is any of it fetched again, to be refused again.
        let plan = plan_sync_out(&host, &volume, &kept);
        assert_eq!(plan, SyncOut::default());
    }

    #[test]
    fn copying_in_keeps_what_is_inside_a_directory_the_host_spells_otherwise() {
        // A step wrote `Foo/x` and a file `foo`; the host, folding case,
        // holds `foo` and so has neither `Foo` nor `Foo/x`.
        let host = listing(&[("foo", file(1, 1))]);
        let volume = listing(&[("Foo", dir()), ("Foo/x", file(1, 1)), ("foo", file(1, 1))]);
        let kept = HashSet::new();
        // `Foo` folds onto `foo`; `Foo/x` is `foo/x`, below a file.
        let plan = plan_sync_in(&host, &volume, &kept, |p| p == Path::new("Foo"));
        assert_eq!(plan, SyncIn::default());
        // A directory the host deleted still goes, with what was in it.
        let host = listing(&[]);
        let volume = listing(&[("gone", dir()), ("gone/x", file(1, 1))]);
        let plan = plan_sync_in(&host, &volume, &kept, |_| false);
        assert_eq!(plan.delete, paths(&["gone"]));
    }

    #[test]
    fn find_errors_name_what_the_listing_missed_or_make_it_untrusted() {
        let vanished = b"find: '/ws/tmp/x': No such file or directory\n";
        assert!(unlisted_entries(vanished).unwrap().is_empty());
        let denied =
            "find: '/ws/a/b': Permission denied\nfind: \u{2018}/ws/c\u{2019}: Input/output error\n";
        assert_eq!(
            unlisted_entries(denied.as_bytes()).unwrap(),
            paths(&["a/b", "c"]).into_iter().collect::<HashSet<_>>()
        );
        // Anything that doesn't name an entry the way it is spelled.
        for untrusted in [
            &b"find: '/ws': Permission denied"[..],
            b"find: '/ws/caf\\303\\251': Permission denied",
            b"find: memory exhausted",
            b"find: '/elsewhere': Permission denied",
        ] {
            assert!(
                unlisted_entries(untrusted).is_err(),
                "{}",
                String::from_utf8_lossy(untrusted)
            );
        }
    }

    #[test]
    fn what_the_volume_listing_missed_is_not_deleted_from_the_host() {
        let host = listing(&[("a", dir()), ("a/b", dir()), ("a/b/out", file(1, 1))]);
        // find listed `a` but couldn't read `a/b`.
        let volume = listing(&[("a", dir())]);
        let unlisted: HashSet<PathBuf> = paths(&["a/b"]).into_iter().collect();
        let plan = plan_sync_out(
            &outside(&host, &unlisted),
            &outside(&volume, &unlisted),
            &HashSet::new(),
        );
        assert_eq!(plan, SyncOut::default());
        // Without that, the host would lose it.
        let plan = plan_sync_out(&host, &volume, &HashSet::new());
        assert_eq!(plan.delete, paths(&["a/b"]));
    }

    #[test]
    fn only_a_name_the_host_cant_hold_is_kept_for_good() {
        let illegal = illegal_name_code().map(io::Error::from_raw_os_error);
        if let Some(illegal) = illegal {
            assert!(cannot_hold(&illegal));
            // As tar reports it, wrapped in its own error.
            let wrapped = io::Error::other(illegal);
            assert!(cannot_hold(&wrapped));
        }
        assert!(cannot_hold(&io::Error::from(io::ErrorKind::NotADirectory)));
        for passing in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::StorageFull,
            io::ErrorKind::ResourceBusy,
        ] {
            assert!(!cannot_hold(&io::Error::from(passing)), "{:?}", passing);
        }
    }

    #[test]
    fn kept_entries_are_remembered_between_runs() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("wrkflw-ws-x");
        let mut kept: HashSet<PathBuf> = paths(&["src/bad", "Readme"]).into_iter().collect();
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            kept.insert(PathBuf::from(std::ffi::OsStr::from_bytes(b"src/n\xff")));
        }
        save_kept(Some(&file), &kept);
        assert_eq!(load_kept(Some(&file)), kept);
        // Nothing kept leaves nothing behind.
        save_kept(Some(&file), &HashSet::new());
        assert!(!file.exists());
        assert!(load_kept(Some(&file)).is_empty());
        assert!(load_kept(None).is_empty());
    }

    #[test]
    fn nested_paths_collapse_to_the_outermost() {
        let sorted = paths(&["a", "a/b", "a/b/c", "ab", "b/c", "b/c/d"]);
        let out = outermost(sorted);
        assert_eq!(out, paths(&["a", "ab", "b/c"]));
        assert!(under_any(&out, Path::new("a/x")));
        assert!(under_any(&out, Path::new("ab")));
        assert!(under_any(&out, Path::new("b/c/e")));
        assert!(!under_any(&out, Path::new("b")));
        assert!(!under_any(&out, Path::new("abc")));
        assert!(!under_any(&out, Path::new("0")));
    }

    #[test]
    fn a_directory_the_host_cant_read_is_left_alone_both_ways() {
        let volume = listing(&[
            ("locked", dir()),
            ("locked/inside", file(1, 10)),
            ("open", file(1, 10)),
        ]);
        let visible = without_unreadable(&volume, &paths(&["locked"]));
        assert_eq!(
            visible,
            listing(&[("locked", dir()), ("open", file(1, 10))])
        );
    }

    #[cfg(unix)]
    #[test]
    fn walking_skips_unreadable_directories_and_removal_gets_past_read_only_ones() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let locked = root.path().join("locked");
        std::fs::create_dir_all(locked.join("sub")).unwrap();
        std::fs::write(locked.join("sub/file"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&locked).is_ok();
        let (listing, unreadable) = walk(root.path()).unwrap();
        assert!(listing.contains_key(Path::new("locked")));
        if !readable {
            // Root reads anything, so this only holds for other users.
            assert_eq!(unreadable, paths(&["locked"]));
        }

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        std::fs::set_permissions(locked.join("sub"), std::fs::Permissions::from_mode(0o555))
            .unwrap();
        remove_host_entry(&locked).unwrap();
        assert!(!locked.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unpacking_never_writes_through_a_linked_directory() {
        let host = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), host.path().join("link")).unwrap();
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        builder
            .append_data(&mut header, "link/escaped", &b"hi"[..])
            .unwrap();
        let archive = builder.into_inner().unwrap();
        let refused = unpack(&archive[..], host.path(), &Listing::new()).unwrap();
        assert_eq!(refused.refused, paths(&["link/escaped"]));
        assert!(!outside.path().join("escaped").exists());
    }

    /// A workspace whose container engine always fails.
    fn broken_workspace(used: bool) -> VmWorkspace {
        let dir = std::env::temp_dir();
        let mut state = State::new();
        state.used = used;
        VmWorkspace {
            cli: "wrkflw-test-no-such-engine",
            host: dir.clone(),
            volume: volume_name(&dir),
            volume_path: PathBuf::from(volume_name(&dir)),
            state: Mutex::new(state),
        }
    }

    #[test]
    fn a_volume_falls_back_to_the_host_only_before_a_step_used_it() {
        // Before any step: the job mounts the host directory instead.
        let fresh = broken_workspace(false);
        assert!(!fresh.sync_in().unwrap());
        assert!(fresh.lock().unusable);
        // After one: the volume holds what steps wrote, so the step fails,
        // even when a failed copy out forgot the volume's listing.
        let used = broken_workspace(true);
        assert!(used.sync_in().is_err());
        assert!(!used.lock().unusable);
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
        assert_eq!(refused.refused, paths(&["blocker/child"]));
        assert!(refused.failed.is_empty());
        assert_eq!(std::fs::read(host.path().join("fine")).unwrap(), b"ok");
        let meta = std::fs::metadata(host.path().join("fine")).unwrap();
        let found = host_entry(&meta, &host.path().join("fine")).unwrap();
        assert_eq!(
            found.kind,
            Kind::File {
                size: 2,
                mtime: 1_700_000_000
            }
        );
        // A host without permission bits has none to report.
        assert_eq!(found.mode, host_mode(&meta));
        #[cfg(unix)]
        assert_eq!(found.mode, Some(0o644));
    }

    fn one_entry_archive(entries: &[(&str, bool)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, is_dir) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mtime(1_700_000_000);
            if *is_dir {
                header.set_size(0);
                header.set_mode(0o755);
                header.set_entry_type(tar::EntryType::Directory);
                builder.append_data(&mut header, name, &b""[..]).unwrap();
            } else {
                header.set_size(2);
                header.set_mode(0o644);
                header.set_entry_type(tar::EntryType::Regular);
                builder.append_data(&mut header, name, &b"hi"[..]).unwrap();
            }
        }
        builder.into_inner().unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn a_write_the_host_refuses_for_now_is_tried_again_not_kept() {
        use std::os::unix::fs::PermissionsExt;
        let host = tempfile::tempdir().unwrap();
        let locked = host.path().join("out");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anyway, so this only holds for other users.
        let writable = std::fs::write(locked.join("probe"), "x").is_ok();
        let archive = one_entry_archive(&[("out/new", false), ("fine", false)]);
        let unpacked = unpack(&archive[..], host.path(), &Listing::new()).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(unpacked.refused.is_empty());
        if !writable {
            assert_eq!(unpacked.failed.len(), 1);
            assert_eq!(unpacked.failed[0].0, PathBuf::from("out/new"));
        }
        assert!(host.path().join("fine").exists());
    }

    #[test]
    fn a_name_the_host_holds_in_another_case_stays_in_the_volume() {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("foo"), "x").unwrap();
        let folds = host.path().join("FOO").exists();
        let archive = one_entry_archive(&[("Foo", true), ("Foo/x", false)]);
        let unpacked = unpack(&archive[..], host.path(), &Listing::new()).unwrap();
        if folds {
            assert_eq!(unpacked.refused, paths(&["Foo"]));
            // The host's file is untouched.
            assert_eq!(std::fs::read(host.path().join("foo")).unwrap(), b"x");
        } else {
            assert!(unpacked.refused.is_empty());
            assert!(host.path().join("Foo/x").exists());
        }
        assert!(unpacked.failed.is_empty());
    }

    /// A Docker daemon that runs Linux containers, as the helper needs. A
    /// Windows daemon answers `docker info` but can't start it.
    fn docker_ready() -> bool {
        std::env::var("WRKFLW_TEST_SKIP_DOCKER").is_err()
            && Command::new("docker")
                .args(["info", "--format", "{{.OSType}}"])
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|out| {
                    out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "linux"
                })
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
            state: Mutex::new(State::new()),
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
