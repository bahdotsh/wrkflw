//! What a job's files are: a content id for every file checkout would hand
//! it, taken from git where git already knows it.

use super::{compile_globs, hex, matches_any, NEVER_INPUT};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Mutex;

/// `path -> content hash` for every tracked or untracked-but-not-ignored file.
pub(crate) type TreeHashes = BTreeMap<String, String>;

/// A hashed tree, with one digest of all of it for jobs that read every file.
/// Without it each such job's key would serialize every path in the
/// repository, which on a large one is megabytes per job.
pub(crate) struct Tree {
    pub(super) files: TreeHashes,
    pub(super) digest: String,
    /// The digest of the files a set of path patterns selects, by pattern
    /// text. Jobs often share their paths, and matching every file against
    /// every pattern is the costly part on a large repository.
    pub(super) selected: Mutex<HashMap<Vec<String>, String>>,
}

impl Tree {
    pub(crate) fn new(files: TreeHashes) -> Option<Self> {
        let digest = digest_of(&files)?;
        Some(Self {
            files,
            digest,
            selected: Mutex::new(HashMap::new()),
        })
    }

    /// One digest of the files `patterns` match.
    pub(super) fn digest_matching(&self, patterns: &[glob::Pattern]) -> Option<String> {
        let id: Vec<String> = patterns.iter().map(|p| p.as_str().to_string()).collect();
        if let Some(digest) = self.selected.lock().ok().and_then(|s| s.get(&id).cloned()) {
            return Some(digest);
        }
        let files: BTreeMap<&String, &String> = self
            .files
            .iter()
            .filter(|(path, _)| matches_any(path, patterns))
            .collect();
        let digest = digest_of(&files)?;
        if let Ok(mut selected) = self.selected.lock() {
            selected.insert(id, digest.clone());
        }
        Some(digest)
    }
}

fn digest_of(files: &impl serde::Serialize) -> Option<String> {
    let text = serde_json::to_string(files).ok()?;
    Some(hex(&Sha256::digest(text.as_bytes())))
}

/// Identify the files checkout would hand to a job. `None` outside a git work
/// tree, where there's no reliable list of source files.
///
/// Git already knows the blob of every clean tracked file, so only changed and
/// untracked files are read. Those get the blob id too, so committing a tree
/// that passed doesn't change its fingerprint.
///
/// A submodule with changes, or an untracked repository inside this one, is
/// hashed as a tree of its own. `None` if one of those can't be read either.
pub(crate) fn hash_tree(root: &Path) -> Option<TreeHashes> {
    hash_tree_at(root, 0)
}

/// How deep nested repositories are followed before giving up.
const MAX_NESTED_REPOS: usize = 8;

fn hash_tree_at(root: &Path, depth: usize) -> Option<TreeHashes> {
    if depth > MAX_NESTED_REPOS {
        return None;
    }
    let files = git_paths(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let dirty: HashSet<String> = git_paths(root, &["ls-files", "-z", "--modified"])?
        .into_iter()
        .collect();
    let index = index_blobs(root)?;
    let excluded = compile_globs(NEVER_INPUT);

    let mut hashes = TreeHashes::new();
    let mut unknown = Vec::new();
    for relative in files {
        if matches_any(&relative, &excluded) {
            continue;
        }
        match index.get(&relative).filter(|_| !dirty.contains(&relative)) {
            Some(blob) => {
                hashes.insert(relative, blob.clone());
            }
            None => unknown.push(relative),
        }
    }

    let (plain, odd): (Vec<String>, Vec<String>) = unknown
        .into_iter()
        .partition(|relative| is_plain_file(&root.join(relative)) && !relative.contains('\n'));
    let blobs = hash_objects(root, &plain);
    for (position, relative) in plain.into_iter().enumerate() {
        let path = root.join(&relative);
        let value = match blobs.as_ref().map(|ids| &ids[position]) {
            Some(id) => format!("{} {}", file_mode(&path, index.get(&relative)), id),
            None => hash_path(&path),
        };
        hashes.insert(relative, value);
    }
    for relative in odd {
        let path = root.join(&relative);
        let value = if is_real_dir(&path) {
            // A changed submodule or an untracked repository. Git lists it as
            // one path, so its contents have to be hashed from inside it.
            let nested = hash_tree_at(&path, depth + 1)?;
            let text = serde_json::to_string(&nested).ok()?;
            format!("repo {}", hex(&Sha256::digest(text.as_bytes())))
        } else {
            hash_path(&path)
        };
        hashes.insert(relative, value);
    }
    Some(hashes)
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn git_paths(root: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|b| *b == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect(),
    )
}

/// `path -> "mode blob"` for merged index entries. Unmerged paths are absent,
/// so they get read.
fn index_blobs(root: &Path) -> Option<HashMap<String, String>> {
    let mut blobs = HashMap::new();
    for entry in git_paths(root, &["ls-files", "-z", "--stage"])? {
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut fields = meta.split(' ');
        if let (Some(mode), Some(blob), Some("0")) = (fields.next(), fields.next(), fields.next()) {
            blobs.insert(path.to_string(), format!("{} {}", mode, blob));
        }
    }
    Some(blobs)
}

fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

/// The mode git would record for the file at `path`. `index_entry` is its
/// `"mode blob"` from the index, if it's tracked.
fn file_mode<'a>(path: &Path, index_entry: Option<&'a String>) -> &'a str {
    #[cfg(unix)]
    {
        let _ = index_entry;
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0) {
            return "100755";
        }
        "100644"
    }
    // No executable bit to read here, so a tracked file keeps the mode git
    // has for it, as git itself does with `core.fileMode` off. Otherwise an
    // edited script would never match its committed fingerprint.
    #[cfg(not(unix))]
    {
        let _ = path;
        index_entry
            .and_then(|entry| entry.split(' ').next())
            .filter(|mode| *mode == "100755")
            .unwrap_or("100644")
    }
}

/// The blob ids of `paths`, in order. `None` if git can't read one of them,
/// which leaves the caller hashing the content itself.
fn hash_objects(root: &Path, paths: &[String]) -> Option<Vec<String>> {
    if paths.is_empty() {
        return Some(Vec::new());
    }
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["hash-object", "--stdin-paths"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let list = paths.join("\n") + "\n";
    // On a thread, as git answers as it reads and a full pipe would block both.
    let writer = std::thread::spawn(move || stdin.write_all(list.as_bytes()));
    let output = child.wait_with_output().ok()?;
    writer.join().ok()?.ok()?;
    if !output.status.success() {
        return None;
    }
    let ids: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    (ids.len() == paths.len()).then_some(ids)
}

fn hash_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    match std::fs::symlink_metadata(path) {
        // A tracked file deleted in the work tree differs from its content.
        Err(_) => hasher.update(b"missing"),
        Ok(meta) if meta.file_type().is_symlink() => {
            hasher.update(b"link:");
            if let Ok(target) = std::fs::read_link(path) {
                hasher.update(target.to_string_lossy().as_bytes());
            }
        }
        Ok(meta) if meta.is_dir() => hasher.update(b"dir"),
        Ok(meta) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Whether a script is executable changes what a job does.
                hasher.update(if meta.permissions().mode() & 0o111 != 0 {
                    b"x"
                } else {
                    b"-"
                });
            }
            match std::fs::File::open(path) {
                Ok(mut file) => {
                    let mut buffer = [0u8; 64 * 1024];
                    loop {
                        match file.read(&mut buffer) {
                            Ok(0) => break,
                            Ok(n) => hasher.update(&buffer[..n]),
                            Err(_) => {
                                hasher.update(b"unreadable");
                                break;
                            }
                        }
                    }
                }
                Err(_) => hasher.update(b"unreadable"),
            }
        }
    }
    hex(&hasher.finalize())
}
