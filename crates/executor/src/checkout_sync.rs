//! What `actions/checkout` does locally: bring a job's workspace up to date
//! with the repository it runs from.
//!
//! Workspaces are kept between runs (see [`crate::workspace`]), so this
//! syncs rather than copies: unchanged files keep their timestamps, ignored
//! build output stays, and files the repository deleted go.

use crate::engine::ExecutionError;
use ignore::{gitignore::GitignoreBuilder, Match};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Create a gitignore matcher for the given directory
fn create_gitignore_matcher(
    dir: &Path,
) -> Result<Option<ignore::gitignore::Gitignore>, ExecutionError> {
    let mut builder = GitignoreBuilder::new(dir);

    // The repository's own ignore rules, then the ones git applies on top:
    // `.git/info/exclude` and the user's global excludes file. Hidden files
    // are synced by these rules in a git checkout, and editor state such as
    // `.idea` is often only ignored globally.
    let excludes = [
        Some(dir.join(".gitignore")),
        Some(dir.join(".git").join("info").join("exclude")),
        ignore::gitignore::gitconfig_excludes_path(),
    ];
    for path in excludes.into_iter().flatten() {
        if path.is_file() {
            builder.add(&path);
        }
    }

    // Add some common ignore patterns as fallback
    builder.add_line(None, "target/").map_err(|e| {
        ExecutionError::Execution(format!("Failed to add default ignore pattern: {}", e))
    })?;
    builder.add_line(None, ".git/").map_err(|e| {
        ExecutionError::Execution(format!("Failed to add default ignore pattern: {}", e))
    })?;

    match builder.build() {
        Ok(gitignore) => Ok(Some(gitignore)),
        Err(e) => {
            wrkflw_logging::warning(&format!("Failed to build gitignore matcher: {}", e));
            Ok(None)
        }
    }
}

/// Bring `to` up to date with the source files in `from`, as a checkout does.
///
/// Job workspaces are kept between runs, so this syncs rather than copies:
/// unchanged files are left alone with their timestamps, which keeps build
/// tools from rebuilding them; changed files are copied with the source's
/// timestamp; and source files deleted since the last run are removed. What
/// the repository ignores (build output, dependencies) is never touched.
pub(crate) fn copy_directory_contents(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    // Walk the source by its real path, so the workspace can be recognised
    // when it sits inside it.
    let from = from.canonicalize().unwrap_or_else(|_| from.to_path_buf());
    let from = from.as_path();
    // Syncing a directory onto itself would only risk deleting from it.
    if let Ok(to) = to.canonicalize() {
        if to == from {
            return Ok(());
        }
    }
    let rules = SyncRules::for_root(from, to)?;
    copy_directory_contents_with_gitignore(from, to, &rules, &[], false)?;
    // A checkout is a git repository, and steps run `git describe`, `git
    // diff` and the like. Without one they fail here and pass on GitHub, and
    // a test runner that stops at the first failure hides every real one.
    mirror_git_dir(from, &to.join(".git"))?;
    rules.record_tracked();
    Ok(())
}

/// Where the repository at `root` keeps its git data: the directory that
/// holds this checkout's own HEAD and index, and the one that holds the
/// objects and refs. They are the same for a plain clone.
///
/// A `.git` file says where the data is, as in a worktree (`git worktree
/// add`), which agents use to work on several branches at once, or in a
/// submodule. A worktree's own directory names the shared one in its
/// `commondir` file.
#[derive(Debug, PartialEq)]
struct GitDirs {
    own: PathBuf,
    common: PathBuf,
}

fn git_dirs(root: &Path) -> Option<GitDirs> {
    let dot_git = root.join(".git");
    let meta = dot_git.symlink_metadata().ok()?;
    if meta.is_dir() {
        return Some(GitDirs {
            own: dot_git.clone(),
            common: dot_git,
        });
    }
    if !meta.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let pointer = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let own = root.join(pointer);
    let own = own.canonicalize().ok().filter(|dir| dir.is_dir())?;
    let common = match std::fs::read_to_string(own.join("commondir")) {
        Ok(relative) => own.join(relative.trim()).canonicalize().ok()?,
        Err(_) => own.clone(),
    };
    Some(GitDirs { own, common })
}

/// Bring `to_git` up to date with the git data of the repository at `root`,
/// as a real directory a container can see, whether `root` has a `.git`
/// directory or a `.git` file pointing elsewhere.
fn mirror_git_dir(root: &Path, to_git: &Path) -> Result<(), ExecutionError> {
    let dot_git = root.join(".git");
    if dot_git.is_symlink() || !dot_git.exists() {
        return Ok(());
    }
    let Some(dirs) = git_dirs(root) else {
        wrkflw_logging::warning(&format!(
            "{} doesn't point at git data wrkflw can read, so the job's workspace is not \
             a git repository: steps that run git will fail here though they pass on GitHub",
            dot_git.display()
        ));
        return Ok(());
    };
    // Hooks stay out: a clone has none, and a step that commits must not
    // run the developer's. Objects are never changed once written, so they
    // are hard-linked where the filesystem allows instead of copied into
    // every job's workspace. Other worktrees' state stays out too.
    //
    // The config is written separately, without the settings that make git
    // run a host program (`core.hooksPath`, `core.fsmonitor`) or point
    // outside the workspace (`core.worktree`).
    mirror_directory(
        &dirs.common,
        to_git,
        false,
        &["hooks", "config", "worktrees"],
        &["objects"],
    )?;
    if dirs.own != dirs.common {
        // This worktree's HEAD, index and the like, over the main
        // checkout's. Only files: its directories (`logs`, and `refs` for
        // per-worktree refs) would replace the shared ones.
        const POINTERS: &[&str] = &["commondir", "gitdir", "locked"];
        let entries = std::fs::read_dir(&dirs.own).map_err(|e| {
            ExecutionError::Execution(format!("Failed to read {}: {e}", dirs.own.display()))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
            if is_file && !POINTERS.contains(&name.to_string_lossy().as_ref()) {
                sync_file(&path, &to_git.join(&name))?;
            }
        }
    }
    write_sanitized_git_config(&dirs.common.join("config"), &to_git.join("config"))
}

/// Settings of `.git/config` a checkout leaves out: each makes git run a
/// program from the developer's machine, or read a path only it has.
const UNSAFE_GIT_CONFIG_KEYS: &[&str] = &["hookspath", "fsmonitor", "worktree"];

/// Sections a checkout leaves out whole: `include` and `includeIf` pull in
/// another config file from the developer's machine, which may set any of
/// [`UNSAFE_GIT_CONFIG_KEYS`].
const UNSAFE_GIT_CONFIG_SECTIONS: &[&str] = &["include", "includeif"];

/// Copy the git config at `from` to `to`, as [`sanitized_git_config`] keeps it.
fn write_sanitized_git_config(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    let Ok(config) = std::fs::read_to_string(from) else {
        return Ok(());
    };
    // The mirror has removed `to` already, since it skips `config`; a step
    // can't have put anything there since. Removed again all the same, so
    // the write never goes through a link.
    if to.symlink_metadata().is_ok() {
        remove_entry(to).map_err(|e| {
            ExecutionError::Execution(format!("Failed to replace {}: {}", to.display(), e))
        })?;
    }
    std::fs::write(to, sanitized_git_config(&config))
        .map_err(|e| ExecutionError::Execution(format!("Failed to write git config: {}", e)))
}

/// `config` without [`UNSAFE_GIT_CONFIG_KEYS`] in any section, and without
/// any entry of [`UNSAFE_GIT_CONFIG_SECTIONS`]. Keys match in any case. A
/// value continued onto the next line with a backslash is dropped or kept
/// whole, and a section header that shares a line with a dropped entry
/// stays.
fn sanitized_git_config(config: &str) -> String {
    let mut out = String::with_capacity(config.len());
    let mut section = String::new();
    // Set while a value continues onto the next line: whether it is being
    // dropped, and whether the line ended inside quotes.
    let mut continued: Option<(bool, bool)> = None;
    for line in config.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some((dropping, in_quote)) = continued {
            continued = value_continues(body, in_quote).map(|q| (dropping, q));
            if !dropping {
                out.push_str(line);
            }
            continue;
        }
        let trimmed = body.trim_start();
        let (header, entry) = match trimmed.strip_prefix('[').and(header_end(trimmed)) {
            Some(end) => {
                section = section_name(&trimmed[..end]);
                (Some(&trimmed[..end]), trimmed[end..].trim_start())
            }
            None => (None, trimmed),
        };
        let key = entry
            .split(|c: char| c == '=' || c.is_whitespace())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let comment = key.is_empty() || key.starts_with(['#', ';']);
        let drop = !comment
            && (UNSAFE_GIT_CONFIG_KEYS.contains(&key.as_str())
                || UNSAFE_GIT_CONFIG_SECTIONS.contains(&section.as_str()));
        if !comment {
            continued = value_continues(entry, false).map(|q| (drop, q));
        }
        match (drop, header) {
            (false, _) => out.push_str(line),
            (true, Some(header)) => {
                out.push_str(header);
                out.push('\n');
            }
            (true, None) => {}
        }
    }
    out
}

/// The length of the section header `line` starts with, closing bracket
/// included. A quoted subsection may hold `]`.
fn header_end(line: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => in_quote = !in_quote,
            ']' if !in_quote => return Some(i + 1),
            _ => {}
        }
    }
    None
}

/// The lowercased name of the section `header` opens, without its
/// subsection: `includeif` for `[includeIf "gitdir:~/w/"]`.
fn section_name(header: &str) -> String {
    header
        .trim_start_matches('[')
        .split(|c: char| c.is_whitespace() || c == '"' || c == ']')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Whether `text`, an entry or a line continuing one, carries on to the
/// next line: it ends in a backslash that escapes nothing, outside a
/// comment. `Some` with whether that line starts inside quotes.
fn value_continues(text: &str, mut in_quote: bool) -> Option<bool> {
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if chars.next().is_none() {
                    return Some(in_quote);
                }
            }
            '"' => in_quote = !in_quote,
            '#' | ';' if !in_quote => return None,
            _ => {}
        }
    }
    None
}

/// Make `dir` a real directory. Whatever else is at that path is removed
/// first, a symlink above all: the workspace is writable by the job's
/// steps, and descending a link a step planted would let the sync write
/// and delete wherever it points.
fn ensure_real_dir(dir: &Path) -> std::io::Result<()> {
    match dir.symlink_metadata() {
        Ok(meta) if meta.is_dir() => return Ok(()),
        Ok(_) => remove_entry(dir)?,
        Err(_) => {}
    }
    std::fs::create_dir_all(dir)
}

/// Remove whatever is at `path` without following it: a directory with its
/// contents, and a file or symlink (to anything) on its own.
fn remove_entry(path: &Path) -> std::io::Result<()> {
    let meta = path.symlink_metadata()?;
    if meta.is_dir() {
        return std::fs::remove_dir_all(path);
    }
    match remove_file_even_read_only(path) {
        // Windows removes a symlink to a directory as a directory.
        #[cfg(windows)]
        Err(_) if meta.file_type().is_symlink() => std::fs::remove_dir(path),
        other => other,
    }
}

/// Make `to` an exact copy of `from`, copying only files that changed.
/// Files are hard-linked rather than copied when `link`. Of `from`'s own
/// entries, those named in `skip` are left out and those in `link_dirs`
/// are linked.
fn mirror_directory(
    from: &Path,
    to: &Path,
    link: bool,
    skip: &[&str],
    link_dirs: &[&str],
) -> Result<(), ExecutionError> {
    let io_err = |what: &str, e: std::io::Error| ExecutionError::Execution(format!("{what}: {e}"));
    ensure_real_dir(to).map_err(|e| io_err("Failed to create dir", e))?;

    let mut synced: HashSet<std::ffi::OsString> = HashSet::new();
    for entry in std::fs::read_dir(from).map_err(|e| io_err("Failed to read directory", e))? {
        let entry = entry.map_err(|e| io_err("Failed to read entry", e))?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_symlink() || skip.contains(&name_str.as_ref()) {
            continue;
        }
        let dest_path = to.join(&name);
        if path.is_dir() {
            let link = link || link_dirs.contains(&name_str.as_ref());
            mirror_directory(&path, &dest_path, link, &[], &[])?;
        } else if link {
            link_file(&path, &dest_path)?;
        } else {
            sync_file(&path, &dest_path)?;
        }
        synced.insert(name);
    }

    for entry in std::fs::read_dir(to)
        .map_err(|e| io_err("Failed to read directory", e))?
        .flatten()
    {
        if synced.contains(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        remove_entry(&path)
            .map_err(|e| io_err(&format!("Failed to remove {}", path.display()), e))?;
    }
    Ok(())
}

/// Which entries of a source tree a checkout brings into the workspace.
struct SyncRules {
    /// The root's rules: its `.gitignore`, `.git/info/exclude` and the
    /// global excludes file. Nested `.gitignore` files are read as the
    /// walk reaches them.
    gitignore: Option<ignore::gitignore::Gitignore>,
    /// Source directories never synced, because the workspace lives in
    /// them: run from `$HOME`, the cache directory is inside the source,
    /// and syncing it would copy the workspace into itself without end.
    never: Vec<PathBuf>,
    /// The source is a git repository. Its ignore rules then decide what is
    /// a source file, hidden or not, as they do for a clone: tracked
    /// dotfiles such as `.cargo/config.toml` belong in the checkout.
    git_checkout: bool,
    /// The canonical source root, which [`Tracked`] paths are relative to.
    root: PathBuf,
    /// What git tracks in the source. A tracked file is part of a checkout
    /// even where an ignore rule matches it. `None` outside a git
    /// repository, or where git can't be run.
    tracked: Option<Tracked>,
    /// What git tracked when the workspace was last synced. A hidden entry
    /// the repository has since deleted is only removed if it was tracked
    /// then, so tool state stays.
    previously_tracked: Option<Tracked>,
    /// Where the workspace's tracked set is recorded, and what to record
    /// there once this sync succeeds: `git ls-files` of the source.
    record: Option<(PathBuf, Option<String>)>,
}

/// Paths git tracks, relative to the repository root.
#[derive(Debug, Default)]
struct Tracked {
    files: HashSet<PathBuf>,
    /// Every directory holding a tracked file, at any depth.
    dirs: HashSet<PathBuf>,
}

impl Tracked {
    /// What `git ls-files -z` prints for the repository at `dir`. Only ever
    /// run on the source: the workspace's `.git` is the job's to change, and
    /// its config could make git run any program on this machine.
    fn listing(dir: &Path) -> Option<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["ls-files", "-z"])
            // A hook that runs wrkflw sets these for its own repository.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn parse(listing: &str) -> Self {
        let mut tracked = Tracked::default();
        for entry in listing.split('\0').filter(|e| !e.is_empty()) {
            let path: PathBuf = entry.split('/').collect();
            let mut parent = path.parent();
            while let Some(dir) = parent.filter(|d| !d.as_os_str().is_empty()) {
                if !tracked.dirs.insert(dir.to_path_buf()) {
                    break;
                }
                parent = dir.parent();
            }
            tracked.files.insert(path);
        }
        tracked
    }

    fn contains(&self, path: &Path, is_dir: bool) -> bool {
        self.files.contains(path) || (is_dir && self.dirs.contains(path))
    }
}

impl SyncRules {
    fn for_root(root: &Path, workspace: &Path) -> Result<Self, ExecutionError> {
        let mut never: Vec<PathBuf> = [
            Some(workspace.to_path_buf()),
            crate::workspace::workspaces_root(),
        ]
        .into_iter()
        .flatten()
        .map(|dir| dir.canonicalize().unwrap_or(dir))
        .filter(|dir| dir.starts_with(root))
        .collect();
        never.dedup();
        let git_checkout = root.join(".git").exists();
        let listing = git_checkout.then(|| Tracked::listing(root)).flatten();
        let tracked = listing.as_deref().map(Tracked::parse);
        // Read from wrkflw's own record beside the workspace, never from the
        // workspace's `.git`, which the job's steps can rewrite. Only worth
        // reading where the source's own index could be read: otherwise
        // nothing compares it with what is tracked now.
        let record_path = crate::workspace::tracked_record(workspace);
        let previously_tracked = match (&tracked, &record_path) {
            (Some(_), Some(path)) => std::fs::read_to_string(path)
                .ok()
                .map(|listing| Tracked::parse(&listing)),
            _ => None,
        };
        Ok(SyncRules {
            gitignore: create_gitignore_matcher(root)?,
            never,
            git_checkout,
            root: root.to_path_buf(),
            tracked,
            previously_tracked,
            record: record_path.map(|path| (path, listing)),
        })
    }

    /// Record what the source tracks for the next sync of this workspace.
    /// Without a listing the old record is removed, so a stale one is never
    /// read later. Failing to record only costs the next sync the removal of
    /// hidden files the repository deleted, so it is logged, not an error.
    fn record_tracked(&self) {
        let Some((path, listing)) = &self.record else {
            return;
        };
        let written = match listing {
            Some(listing) => {
                let partial = path.with_extension("tracked.partial");
                std::fs::write(&partial, listing).and_then(|()| std::fs::rename(&partial, path))
            }
            None => match std::fs::remove_file(path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        if let Err(e) = written {
            wrkflw_logging::warning(&format!(
                "Failed to record the files git tracks for {}: {}",
                path.display(),
                e
            ));
        }
    }

    fn relative<'a>(&self, source: &'a Path) -> Option<&'a Path> {
        source.strip_prefix(&self.root).ok()
    }

    fn is_tracked(&self, source: &Path, is_dir: bool) -> bool {
        match (&self.tracked, self.relative(source)) {
            (Some(tracked), Some(rel)) => tracked.contains(rel, is_dir),
            _ => false,
        }
    }

    fn was_tracked(&self, source: &Path, is_dir: bool) -> bool {
        match (&self.previously_tracked, self.relative(source)) {
            (Some(tracked), Some(rel)) => tracked.contains(rel, is_dir),
            _ => false,
        }
    }

    /// Whether `source`, an entry named `name` in the source tree, is
    /// synced. `source` must be a path under the root, so ignore rules
    /// anchored to the root still match in subdirectories.
    /// `nested` holds the `.gitignore` files of the directories between the
    /// root and `source`, outermost first. `only_tracked` is set inside an
    /// ignored directory the walk entered for the tracked files in it.
    fn syncs(
        &self,
        source: &Path,
        name: &str,
        is_dir: bool,
        nested: &[ignore::gitignore::Gitignore],
        only_tracked: bool,
    ) -> bool {
        // `.git` is mirrored separately, or not at all when it is a file.
        if name == ".git" || (is_dir && self.never.iter().any(|dir| dir == source)) {
            return false;
        }
        // Git never ignores what it tracks, so neither does a checkout.
        if self.is_tracked(source, is_dir) {
            return true;
        }
        if only_tracked || self.ignored(source, is_dir, nested) {
            return false;
        }
        self.git_checkout || !Self::hidden(name)
    }

    /// Whether git would ignore `source`. As in git, the `.gitignore`
    /// nearest to it has the last word, including to re-include with `!`.
    fn ignored(
        &self,
        source: &Path,
        is_dir: bool,
        nested: &[ignore::gitignore::Gitignore],
    ) -> bool {
        for gitignore in nested.iter().rev().chain(self.gitignore.as_ref()) {
            match gitignore.matched(source, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }

    /// Whether a workspace entry with no source counterpart may be deleted.
    /// What git tracked at the last sync always may: the repository deleted
    /// it. Other hidden entries never are: they may be tool state
    /// (`.pytest_cache`, `.venv`) a repository doesn't bother to ignore.
    fn may_delete(
        &self,
        source: &Path,
        name: &str,
        is_dir: bool,
        nested: &[ignore::gitignore::Gitignore],
        only_tracked: bool,
    ) -> bool {
        if self.was_tracked(source, is_dir) {
            return true;
        }
        !name.starts_with('.') && self.syncs(source, name, is_dir, nested, only_tracked)
    }

    /// The rules of `dir`'s own `.gitignore`, if it has one. The root's
    /// are already in [`SyncRules::gitignore`].
    fn nested_gitignore(dir: &Path) -> Option<ignore::gitignore::Gitignore> {
        let path = dir.join(".gitignore");
        if !path.is_file() {
            return None;
        }
        let mut builder = GitignoreBuilder::new(dir);
        if let Some(e) = builder.add(&path) {
            wrkflw_logging::warning(&format!("Ignoring unreadable {}: {}", path.display(), e));
            return None;
        }
        builder.build().ok()
    }

    /// Outside a git repository, most hidden files are editor and tool
    /// state rather than source.
    fn hidden(name: &str) -> bool {
        name.starts_with('.')
            && name != ".gitignore"
            && name != ".github"
            && name != ".gitea"
            && !name.starts_with(".env")
    }
}

fn copy_directory_contents_with_gitignore(
    from: &Path,
    to: &Path,
    rules: &SyncRules,
    nested: &[ignore::gitignore::Gitignore],
    only_tracked: bool,
) -> Result<(), ExecutionError> {
    let io_err = |what: &str, e: std::io::Error| ExecutionError::Execution(format!("{what}: {e}"));

    wrkflw_logging::debug(&format!(
        "Syncing directory contents from {} to {}",
        from.display(),
        to.display()
    ));

    let mut synced: HashSet<std::ffi::OsString> = HashSet::new();
    for entry in std::fs::read_dir(from).map_err(|e| io_err("Failed to read directory", e))? {
        let entry = entry.map_err(|e| io_err("Failed to read entry", e))?;
        let path = entry.path();
        let name = entry.file_name();
        let is_dir = path.is_dir();
        if path.is_symlink()
            || !rules.syncs(&path, &name.to_string_lossy(), is_dir, nested, only_tracked)
        {
            continue;
        }
        synced.insert(name.clone());
        let dest_path = to.join(&name);

        if is_dir {
            ensure_real_dir(&dest_path).map_err(|e| io_err("Failed to create dir", e))?;
            // An ignored directory is only entered for what git tracks in it.
            let inner_only_tracked = only_tracked || rules.ignored(&path, true, nested);
            match SyncRules::nested_gitignore(&path) {
                Some(own) => {
                    let mut inner = nested.to_vec();
                    inner.push(own);
                    copy_directory_contents_with_gitignore(
                        &path,
                        &dest_path,
                        rules,
                        &inner,
                        inner_only_tracked,
                    )?;
                }
                None => copy_directory_contents_with_gitignore(
                    &path,
                    &dest_path,
                    rules,
                    nested,
                    inner_only_tracked,
                )?,
            }
        } else {
            sync_file(&path, &dest_path)?;
        }
    }

    // Remove source files deleted since the last run. Only entries that
    // would have been synced are candidates, so ignored build output and
    // hidden tool state stay.
    let Ok(existing) = std::fs::read_dir(to) else {
        return Ok(());
    };
    for entry in existing.flatten() {
        let name = entry.file_name();
        if synced.contains(&name) {
            continue;
        }
        let path = entry.path();
        // Judge it by where it would be in the source, so ignore rules
        // anchored to the root apply.
        let source = from.join(&name);
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        // A symlink is removed only where a file would be; it is never
        // followed. Steps make links (`node_modules/.bin`), so one the
        // repository didn't have is judged by its name like any entry.
        let is_dir = meta.is_dir();
        if !rules.may_delete(
            &source,
            &name.to_string_lossy(),
            is_dir,
            nested,
            only_tracked,
        ) {
            continue;
        }
        remove_entry(&path).map_err(|e| {
            io_err(
                &format!(
                    "Failed to remove {} from the job's saved workspace (set {}=1 to start \
                     from an empty one)",
                    path.display(),
                    crate::run_options::FRESH_WORKSPACE_ENV
                ),
                e,
            )
        })?;
    }

    Ok(())
}

/// Hard-link `from` at `to`, or copy it where linking fails (another
/// filesystem, or one without links). Only for files nothing rewrites in
/// place, since the link shares the source's contents.
fn link_file(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (std::fs::metadata(from), std::fs::symlink_metadata(to)) {
            if a.dev() == b.dev() && a.ino() == b.ino() {
                return Ok(());
            }
        }
    }
    // Where links fail, an earlier run left a copy; keep it if it is current,
    // rather than copying the whole object store again on every run.
    if unchanged_copy(from, to) {
        return Ok(());
    }
    if to.symlink_metadata().is_ok() {
        remove_entry(to).map_err(|e| {
            ExecutionError::Execution(format!("Failed to replace {}: {}", to.display(), e))
        })?;
    }
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    sync_file(from, to)
}

/// Whether `to` is already a copy of `from` as [`sync_file`] makes one:
/// same size and modification time.
fn unchanged_copy(from: &Path, to: &Path) -> bool {
    // The destination is read without following links: a symlink a step
    // left there is never "already a copy", so it is replaced.
    match (std::fs::metadata(from), std::fs::symlink_metadata(to)) {
        (Ok(source), Ok(dest)) => {
            dest.is_file()
                && dest.len() == source.len()
                && dest.modified().ok() == source.modified().ok()
        }
        _ => false,
    }
}

/// Copy `from` to `to` unless `to` already matches it, keeping the source's
/// modification time so build tools see the file as unchanged. Read-only
/// files, such as generated code or git objects, are replaced and keep
/// their time too.
fn sync_file(from: &Path, to: &Path) -> Result<(), ExecutionError> {
    let source = std::fs::metadata(from)
        .map_err(|e| ExecutionError::Execution(format!("Failed to read file: {}", e)))?;
    if unchanged_copy(from, to) {
        return Ok(());
    }
    // `fs::copy` writes into an existing file, which a read-only one
    // refuses, and through a symlink, which could point anywhere; a new
    // file takes the source's permissions either way.
    if to.symlink_metadata().is_ok() {
        remove_entry(to).map_err(|e| {
            ExecutionError::Execution(format!("Failed to replace {}: {}", to.display(), e))
        })?;
    }
    std::fs::copy(from, to)
        .map_err(|e| ExecutionError::Execution(format!("Failed to copy file: {}", e)))?;
    if let Ok(modified) = source.modified() {
        if let Err(e) = set_modified(to, modified) {
            // Not fatal: the file is current, it is only copied again next run.
            wrkflw_logging::debug(&format!(
                "Could not keep the modification time of {}: {}",
                to.display(),
                e
            ));
        }
    }
    Ok(())
}

/// Set `path`'s modification time without needing write access to it: the
/// owner may change a read-only file's times.
fn set_modified(path: &Path, time: std::time::SystemTime) -> std::io::Result<()> {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_WRITE_ATTRIBUTES, which a read-only file still grants.
        std::fs::File::options().access_mode(0x100).open(path)?
    };
    #[cfg(not(windows))]
    let file = std::fs::File::open(path)?;
    file.set_modified(time)
}

/// Remove a file, clearing its read-only flag first where the platform
/// needs that (Windows refuses to delete read-only files).
fn remove_file_even_read_only(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut permissions = std::fs::symlink_metadata(path)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(path, permissions)?;
            std::fs::remove_file(path)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_sync_keeps_build_output_and_unchanged_timestamps() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(".gitignore"), "out/\n").unwrap();
        std::fs::create_dir_all(src.path().join("lib")).unwrap();
        std::fs::write(src.path().join("lib/keep.c"), "same").unwrap();
        std::fs::write(src.path().join("lib/edit.c"), "old").unwrap();
        std::fs::write(src.path().join("gone.c"), "deleted later").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        // The job builds, and leaves output the repository ignores.
        std::fs::create_dir_all(dst.path().join("out")).unwrap();
        std::fs::write(dst.path().join("out/app"), "binary").unwrap();
        let kept_mtime = std::fs::metadata(dst.path().join("lib/keep.c"))
            .unwrap()
            .modified()
            .unwrap();

        // Between runs: one file edited, one deleted.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(src.path().join("lib/edit.c"), "new!").unwrap();
        std::fs::remove_file(src.path().join("gone.c")).unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(dst.path().join("lib/edit.c")).unwrap(),
            "new!"
        );
        assert!(!dst.path().join("gone.c").exists());
        assert!(dst.path().join("out/app").exists());
        assert_eq!(
            std::fs::metadata(dst.path().join("lib/keep.c"))
                .unwrap()
                .modified()
                .unwrap(),
            kept_mtime
        );
        assert_eq!(
            std::fs::metadata(dst.path().join("lib/edit.c"))
                .unwrap()
                .modified()
                .unwrap(),
            std::fs::metadata(src.path().join("lib/edit.c"))
                .unwrap()
                .modified()
                .unwrap()
        );
    }

    #[test]
    fn workspace_sync_replaces_read_only_files_and_keeps_their_time() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let file = src.path().join("gen.pb.go");
        let read_only = |path: &Path, on: bool| {
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(on);
            std::fs::set_permissions(path, permissions).unwrap();
        };
        std::fs::write(&file, "v1").unwrap();
        read_only(&file, true);
        copy_directory_contents(src.path(), dst.path()).unwrap();
        let copy = dst.path().join("gen.pb.go");
        let mtime = |path: &Path| std::fs::metadata(path).unwrap().modified().unwrap();
        assert_eq!(mtime(&copy), mtime(&file), "the copy's time was not kept");

        // Regenerated between runs, still read-only.
        std::thread::sleep(std::time::Duration::from_millis(20));
        read_only(&file, false);
        std::fs::write(&file, "v2!").unwrap();
        read_only(&file, true);
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "v2!");
        assert_eq!(mtime(&copy), mtime(&file));

        // Deleted between runs: the read-only copy goes too.
        read_only(&file, false);
        std::fs::remove_file(&file).unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(!copy.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_file_that_fell_back_to_a_copy_is_not_copied_again() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("object");
        let to = dir.path().join("copy");
        std::fs::write(&from, "blob").unwrap();
        // What an earlier run leaves where links fail: a copy, not a link.
        sync_file(&from, &to).unwrap();
        let before = std::fs::metadata(&to).unwrap().ino();
        link_file(&from, &to).unwrap();
        let after = std::fs::metadata(&to).unwrap().ino();
        assert_eq!(after, before, "the current copy was replaced");
    }

    #[test]
    fn workspace_sync_mirrors_the_git_directory() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join(".git/objects/pack")).unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(src.path().join(".git/objects/pack/old.pack"), "x").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(dst.path().join(".git/objects/pack/old.pack").is_file());

        // `git gc` repacks between runs.
        std::fs::remove_file(src.path().join(".git/objects/pack/old.pack")).unwrap();
        std::fs::write(src.path().join(".git/objects/pack/new.pack"), "y").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(!dst.path().join(".git/objects/pack/old.pack").exists());
        assert!(dst.path().join(".git/objects/pack/new.pack").is_file());
        assert_eq!(
            std::fs::read_to_string(dst.path().join(".git/HEAD")).unwrap(),
            "ref: refs/heads/main"
        );
    }

    #[test]
    fn a_git_checkout_brings_tracked_dotfiles_and_leaves_tool_state() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join(".git")).unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::create_dir_all(src.path().join(".cargo")).unwrap();
        std::fs::write(src.path().join(".cargo/config.toml"), "[build]").unwrap();
        std::fs::write(src.path().join(".rustfmt.toml"), "").unwrap();
        std::fs::write(src.path().join(".gitignore"), ".secret-local\n").unwrap();
        std::fs::write(src.path().join(".secret-local"), "x").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join(".cargo/config.toml").is_file());
        assert!(dst.path().join(".rustfmt.toml").is_file());
        assert!(!dst.path().join(".secret-local").exists());

        // A tool leaves hidden state the repository doesn't ignore; the
        // next sync keeps it.
        std::fs::create_dir_all(dst.path().join(".pytest_cache")).unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(dst.path().join(".pytest_cache").is_dir());
    }

    #[test]
    fn workspace_sync_applies_anchored_ignores_in_subdirectories() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(".gitignore"), "/web/dist/\n").unwrap();
        std::fs::create_dir_all(src.path().join("web/dist")).unwrap();
        std::fs::write(src.path().join("web/index.ts"), "src").unwrap();
        std::fs::write(src.path().join("web/dist/host.js"), "host build").unwrap();
        // The job's own build output from its last run.
        std::fs::create_dir_all(dst.path().join("web/dist")).unwrap();
        std::fs::write(dst.path().join("web/dist/app.js"), "job build").unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join("web/index.ts").is_file());
        assert!(dst.path().join("web/dist/app.js").is_file());
        assert!(!dst.path().join("web/dist/host.js").exists());
    }

    #[test]
    fn workspace_sync_follows_nested_gitignores() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("web/dist")).unwrap();
        std::fs::write(
            src.path().join("web/.gitignore"),
            "dist/\n*.log\n!keep.log\n",
        )
        .unwrap();
        std::fs::write(src.path().join("web/index.ts"), "src").unwrap();
        std::fs::write(src.path().join("web/dist/host.js"), "host build").unwrap();
        std::fs::write(src.path().join("web/debug.log"), "noise").unwrap();
        std::fs::write(src.path().join("web/keep.log"), "tracked").unwrap();
        // The rules apply only below the directory that holds them.
        std::fs::write(src.path().join("top.log"), "tracked").unwrap();
        // The job's own build output from its last run.
        std::fs::create_dir_all(dst.path().join("web/dist")).unwrap();
        std::fs::write(dst.path().join("web/dist/app.js"), "job build").unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join("web/index.ts").is_file());
        assert!(dst.path().join("web/keep.log").is_file());
        assert!(dst.path().join("top.log").is_file());
        assert!(!dst.path().join("web/debug.log").exists());
        assert!(!dst.path().join("web/dist/host.js").exists());
        assert!(
            dst.path().join("web/dist/app.js").is_file(),
            "build output was deleted"
        );
    }

    #[test]
    fn workspace_sync_skips_a_workspace_inside_the_source() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("main.c"), "src").unwrap();
        let dst = src.path().join("cache/wrkflw/workspaces/job");
        std::fs::create_dir_all(&dst).unwrap();

        copy_directory_contents(src.path(), &dst).unwrap();

        assert!(dst.join("main.c").is_file());
        assert!(!dst.join("cache/wrkflw/workspaces/job").exists());
        // The workspace's parents are still synced, without the workspace.
        assert!(dst.join("cache/wrkflw/workspaces").is_dir());
    }

    #[test]
    fn git_mirror_leaves_out_hooks_and_links_objects() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for dir in [".git/hooks", ".git/objects/ab"] {
            std::fs::create_dir_all(src.path().join(dir)).unwrap();
        }
        std::fs::write(src.path().join(".git/hooks/pre-commit"), "exit 1").unwrap();
        std::fs::write(src.path().join(".git/objects/ab/cdef"), "blob").unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(!dst.path().join(".git/hooks").exists());
        assert!(dst.path().join(".git/HEAD").is_file());
        let object = dst.path().join(".git/objects/ab/cdef");
        assert_eq!(std::fs::read_to_string(&object).unwrap(), "blob");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let a = std::fs::metadata(src.path().join(".git/objects/ab/cdef")).unwrap();
            let b = std::fs::metadata(&object).unwrap();
            // Linked where the filesystem allows; tempdirs share one.
            assert_eq!(a.ino(), b.ino());
        }
        // A second sync leaves the link alone.
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(object.is_file());
    }

    #[test]
    fn workspace_copy_keeps_forge_dirs_and_skips_other_hidden_dirs() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        for dir in [".gitea/workflows", ".github/workflows", ".cache"] {
            std::fs::create_dir_all(src.path().join(dir)).unwrap();
            std::fs::write(src.path().join(dir).join("ci.yml"), "x").unwrap();
        }

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join(".gitea/workflows/ci.yml").is_file());
        assert!(dst.path().join(".github/workflows/ci.yml").is_file());
        assert!(!dst.path().join(".cache").exists());
    }

    /// Run git in `dir`, isolated from the user's config. `false` when git
    /// isn't installed, so tests that need a real index can skip.
    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn git_repo() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]).then_some(dir)
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_at_a_source_directory_is_replaced_not_followed() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("docs")).unwrap();
        std::fs::write(src.path().join("docs/guide.md"), "guide").unwrap();
        std::fs::write(outside.path().join("precious"), "keep me").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        // A step replaces the directory with a link to somewhere else.
        std::fs::remove_dir_all(dst.path().join("docs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dst.path().join("docs")).unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(outside.path().join("precious").is_file());
        assert!(!outside.path().join("guide.md").exists());
        let docs = dst.path().join("docs");
        assert!(!docs.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(
            std::fs::read_to_string(docs.join("guide.md")).unwrap(),
            "guide"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_at_git_is_replaced_not_followed() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join(".git")).unwrap();
        std::fs::write(src.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(outside.path().join("precious"), "keep me").unwrap();
        std::os::unix::fs::symlink(outside.path(), dst.path().join(".git")).unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(outside.path().join("precious").is_file());
        assert!(!outside.path().join("HEAD").exists());
        assert!(dst.path().join(".git/HEAD").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_at_a_source_file_is_replaced_not_written_through() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("main.c"), "source").unwrap();
        std::fs::write(outside.path().join("precious"), "keep me").unwrap();
        std::os::unix::fs::symlink(outside.path().join("precious"), dst.path().join("main.c"))
            .unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert_eq!(
            std::fs::read_to_string(outside.path().join("precious")).unwrap(),
            "keep me"
        );
        assert_eq!(
            std::fs::read_to_string(dst.path().join("main.c")).unwrap(),
            "source"
        );
    }

    #[test]
    fn a_tracked_file_an_ignore_rule_matches_is_still_checked_out() {
        let Some(src) = git_repo() else {
            return;
        };
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(".gitignore"), "*.pem\nfixtures/\n").unwrap();
        std::fs::create_dir_all(src.path().join("fixtures")).unwrap();
        std::fs::write(src.path().join("ca.pem"), "tracked cert").unwrap();
        std::fs::write(src.path().join("fixtures/golden.txt"), "tracked").unwrap();
        std::fs::write(src.path().join("fixtures/scratch.txt"), "untracked").unwrap();
        std::fs::write(src.path().join("local.pem"), "untracked cert").unwrap();
        assert!(git(
            src.path(),
            &["add", "-f", "ca.pem", "fixtures/golden.txt", ".gitignore"]
        ));

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join("ca.pem").is_file());
        assert!(dst.path().join("fixtures/golden.txt").is_file());
        assert!(!dst.path().join("fixtures/scratch.txt").exists());
        assert!(!dst.path().join("local.pem").exists());

        // Build output the job writes into the ignored directory stays.
        std::fs::write(dst.path().join("fixtures/out.bin"), "built").unwrap();
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(dst.path().join("fixtures/out.bin").is_file());
    }

    /// A workspace laid out as a kept one: a directory with its lock beside
    /// it, which is what makes checkout record what git tracks.
    fn kept_workspace() -> (tempfile::TempDir, PathBuf) {
        let parent = tempfile::tempdir().unwrap();
        let ws = parent.path().join("build-0123");
        std::fs::create_dir(&ws).unwrap();
        std::fs::write(ws.with_extension("lock"), "").unwrap();
        (parent, ws)
    }

    #[test]
    fn a_hidden_file_the_repository_deleted_is_removed() {
        let Some(src) = git_repo() else {
            return;
        };
        let (_parent, dst) = kept_workspace();
        std::fs::create_dir_all(src.path().join(".cargo")).unwrap();
        std::fs::write(src.path().join(".cargo/config.toml"), "[build]").unwrap();
        std::fs::write(src.path().join(".npmrc"), "x").unwrap();
        std::fs::write(src.path().join("lib.rs"), "").unwrap();
        assert!(git(src.path(), &["add", "."]));
        assert!(git(src.path(), &["commit", "-qm", "one"]));
        copy_directory_contents(src.path(), dst.as_path()).unwrap();
        assert!(dst.as_path().join(".cargo/config.toml").is_file());
        std::fs::create_dir_all(dst.as_path().join(".pytest_cache")).unwrap();

        assert!(git(src.path(), &["rm", "-rq", ".cargo", ".npmrc"]));
        assert!(git(src.path(), &["commit", "-qm", "two"]));
        copy_directory_contents(src.path(), dst.as_path()).unwrap();

        assert!(!dst.as_path().join(".cargo").exists());
        assert!(!dst.as_path().join(".npmrc").exists());
        // Tool state the repository never tracked stays.
        assert!(dst.as_path().join(".pytest_cache").is_dir());
        // What was tracked came from wrkflw's record, not the workspace.
        assert!(dst.with_extension("tracked").is_file());
    }

    #[test]
    fn without_a_record_no_hidden_file_is_removed() {
        let Some(src) = git_repo() else {
            return;
        };
        // A workspace that isn't kept has no record, so a hidden file is
        // never taken for one the repository deleted.
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(".npmrc"), "x").unwrap();
        assert!(git(src.path(), &["add", "."]));
        assert!(git(src.path(), &["commit", "-qm", "one"]));
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(git(src.path(), &["rm", "-q", ".npmrc"]));
        assert!(git(src.path(), &["commit", "-qm", "two"]));
        copy_directory_contents(src.path(), dst.path()).unwrap();
        assert!(dst.path().join(".npmrc").is_file());
        assert!(!dst.path().with_extension("tracked").exists());
    }

    /// The workspace's `.git` is the job's: a step can set `core.fsmonitor`
    /// there and turn on the index extension that makes git run it. The
    /// sync must never run git in the workspace, or that program would run
    /// on the host.
    #[cfg(unix)]
    #[test]
    fn checkout_never_runs_git_in_the_workspace() {
        use std::os::unix::fs::PermissionsExt;
        let Some(src) = git_repo() else {
            return;
        };
        let (parent, dst) = kept_workspace();
        std::fs::write(src.path().join("lib.rs"), "").unwrap();
        assert!(git(src.path(), &["add", "."]));
        assert!(git(src.path(), &["commit", "-qm", "one"]));
        copy_directory_contents(src.path(), &dst).unwrap();

        // What a step could do in the workspace it is given.
        let marker = parent.path().join("ran-on-host");
        let hook = dst.join("hook.sh");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = dst.join(".git/config");
        let mut planted = std::fs::read_to_string(&config).unwrap();
        planted.push_str(&format!("[core]\n\tfsmonitor = {}\n", hook.display()));
        std::fs::write(&config, planted).unwrap();
        // Planting it runs the hook once; only what the sync does counts.
        git(&dst, &["update-index", "--fsmonitor"]);
        let _ = std::fs::remove_file(&marker);

        copy_directory_contents(src.path(), &dst).unwrap();

        assert!(!marker.exists(), "the sync ran a program the job planted");
        let config = std::fs::read_to_string(dst.join(".git/config")).unwrap();
        assert!(!config.contains("fsmonitor"));
    }

    #[test]
    fn the_git_mirror_drops_settings_that_run_host_programs() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join(".git")).unwrap();
        std::fs::write(
            src.path().join(".git/config"),
            "[core]\n\tbare = false\n\thooksPath = .husky/_\n\tfsmonitor = /usr/bin/watch\n\
             \tworktree = /home/dev/repo\n[remote \"origin\"]\n\turl = git@example.com:r.git\n",
        )
        .unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        let config = std::fs::read_to_string(dst.path().join(".git/config")).unwrap();
        assert!(config.contains("bare = false"));
        assert!(config.contains("url = git@example.com:r.git"));
        assert!(!config.to_ascii_lowercase().contains("hookspath"));
        assert!(!config.contains("fsmonitor"));
        assert!(!config.contains("worktree"));
    }

    #[test]
    fn tracked_paths_are_parsed_with_every_parent_directory() {
        let tracked = Tracked::parse("a/b/c.txt\0d.txt\0");
        assert!(tracked.contains(Path::new("a/b/c.txt"), false));
        assert!(tracked.contains(Path::new("d.txt"), false));
        assert!(tracked.contains(Path::new("a"), true));
        assert!(tracked.contains(Path::new("a/b"), true));
        assert!(!tracked.contains(Path::new("a"), false));
        assert!(!tracked.contains(Path::new("e"), true));
    }

    #[test]
    fn the_git_config_filter_follows_sections_and_continued_values() {
        let config = "[core]\n\tbare = false\n\thooksPath = a\\\n b\n\tautocrlf = input\n\
                      [core] fsmonitor = x\n\
                      [include]\n\tpath = ~/.gitconfig-extra\n\
                      [includeIf \"gitdir:~/w/\"]\n\tpath = ~/.w\n\
                      [remote \"o]rigin\"]\n\turl = a#b \"x;y\" ; note \\\n\tfetch = +refs\n\
                      [alias]\n\tst = \"status \\\n--short\"\n\tworktree = nope\n\
                      [submodule \"m\"]\n\tpath = vendor/m\n";
        let kept = sanitized_git_config(config);
        assert_eq!(
            kept,
            "[core]\n\tbare = false\n\tautocrlf = input\n\
             [core]\n\
             [include]\n\
             [includeIf \"gitdir:~/w/\"]\n\
             [remote \"o]rigin\"]\n\turl = a#b \"x;y\" ; note \\\n\tfetch = +refs\n\
             [alias]\n\tst = \"status \\\n--short\"\n\
             [submodule \"m\"]\n\tpath = vendor/m\n"
        );
        // Comments and CRLF line ends pass through untouched.
        assert_eq!(
            sanitized_git_config("# hooksPath = x\r\n[core]\r\n\tHOOKSPATH=y\r\n"),
            "# hooksPath = x\r\n[core]\r\n"
        );
    }

    /// Run git in `dir` and return what it printed.
    fn git_out(dir: &Path, args: &[&str]) -> Option<String> {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    #[test]
    fn a_worktree_checkout_is_a_git_repository_on_its_branch() {
        let Some(repo) = git_repo() else { return };
        std::fs::write(repo.path().join("a.txt"), "a").unwrap();
        assert!(git(repo.path(), &["add", "."]));
        assert!(git(repo.path(), &["commit", "-q", "-m", "one"]));
        assert!(git(repo.path(), &["tag", "v1"]));
        let worktrees = tempfile::tempdir().unwrap();
        let tree = worktrees.path().join("feature");
        assert!(git(
            repo.path(),
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                tree.to_str().unwrap()
            ]
        ));
        std::fs::write(tree.join("b.txt"), "b").unwrap();
        assert!(git(&tree, &["add", "."]));
        assert!(git(&tree, &["commit", "-q", "-m", "two"]));
        assert!(tree.join(".git").is_file());
        let dst = tempfile::tempdir().unwrap();

        copy_directory_contents(&tree, dst.path()).unwrap();

        let git_dir = dst.path().join(".git");
        assert!(git_dir.is_dir());
        assert!(!git_dir.join("worktrees").exists());
        assert!(!git_dir.join("commondir").exists());
        assert_eq!(
            git_out(dst.path(), &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(),
            Some("feature")
        );
        assert_eq!(
            git_out(dst.path(), &["describe", "--tags"])
                .unwrap()
                .split('-')
                .next(),
            Some("v1")
        );
        assert_eq!(
            git_out(dst.path(), &["status", "--porcelain"]).as_deref(),
            Some("")
        );

        // A second sync keeps it a repository on the worktree's branch.
        copy_directory_contents(&tree, dst.path()).unwrap();
        assert_eq!(
            git_out(dst.path(), &["rev-parse", "--abbrev-ref", "HEAD"]).as_deref(),
            Some("feature")
        );
    }

    #[test]
    fn a_git_file_pointing_nowhere_leaves_no_repository() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join(".git"), "gitdir: /does/not/exist\n").unwrap();
        std::fs::write(src.path().join("a.txt"), "a").unwrap();
        let dst = tempfile::tempdir().unwrap();

        copy_directory_contents(src.path(), dst.path()).unwrap();

        assert!(dst.path().join("a.txt").is_file());
        assert!(!dst.path().join(".git").exists());
        assert_eq!(git_dirs(src.path()), None);
    }
}
