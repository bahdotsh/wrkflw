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
    copy_directory_contents_with_gitignore(from, to, &SyncRules::for_root(from, to)?, &[])?;
    // A checkout is a git repository, and steps run `git describe`, `git
    // diff` and the like. A `.git` file (a worktree or submodule) points at a
    // host path a container can't see, so only a directory is mirrored.
    let git_dir = from.join(".git");
    if git_dir.is_dir() && !git_dir.is_symlink() {
        // Hooks stay out: a clone has none, and a step that commits must
        // not run the developer's. Objects are never changed once written,
        // so they are hard-linked where the filesystem allows instead of
        // copied into every job's workspace.
        mirror_directory(&git_dir, &to.join(".git"), false, &["hooks"], &["objects"])?;
    }
    Ok(())
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
    if to.exists() && !to.is_dir() {
        remove_file_even_read_only(to).map_err(|e| io_err("Failed to replace file with dir", e))?;
    }
    std::fs::create_dir_all(to).map_err(|e| io_err("Failed to create dir", e))?;

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
        let removed = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            remove_file_even_read_only(&path)
        };
        removed.map_err(|e| io_err(&format!("Failed to remove {}", path.display()), e))?;
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
        Ok(SyncRules {
            gitignore: create_gitignore_matcher(root)?,
            never,
            git_checkout: root.join(".git").exists(),
        })
    }

    /// Whether `source`, an entry named `name` in the source tree, is
    /// synced. `source` must be a path under the root, so ignore rules
    /// anchored to the root still match in subdirectories.
    /// `nested` holds the `.gitignore` files of the directories between the
    /// root and `source`, outermost first.
    fn syncs(
        &self,
        source: &Path,
        name: &str,
        is_dir: bool,
        nested: &[ignore::gitignore::Gitignore],
    ) -> bool {
        // `.git` is mirrored separately, or not at all when it is a file.
        if name == ".git" || (is_dir && self.never.iter().any(|dir| dir == source)) {
            return false;
        }
        if self.ignored(source, is_dir, nested) {
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
    /// Hidden entries never are: they may be tool state (`.pytest_cache`,
    /// `.venv`) a repository doesn't bother to ignore.
    fn may_delete(
        &self,
        source: &Path,
        name: &str,
        is_dir: bool,
        nested: &[ignore::gitignore::Gitignore],
    ) -> bool {
        !name.starts_with('.') && self.syncs(source, name, is_dir, nested)
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
        if path.is_symlink() || !rules.syncs(&path, &name.to_string_lossy(), path.is_dir(), nested)
        {
            continue;
        }
        synced.insert(name.clone());
        let dest_path = to.join(&name);

        if path.is_dir() {
            if dest_path.exists() && !dest_path.is_dir() {
                remove_file_even_read_only(&dest_path)
                    .map_err(|e| io_err("Failed to replace file with dir", e))?;
            }
            std::fs::create_dir_all(&dest_path).map_err(|e| io_err("Failed to create dir", e))?;
            match SyncRules::nested_gitignore(&path) {
                Some(own) => {
                    let mut inner = nested.to_vec();
                    inner.push(own);
                    copy_directory_contents_with_gitignore(&path, &dest_path, rules, &inner)?;
                }
                None => copy_directory_contents_with_gitignore(&path, &dest_path, rules, nested)?,
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
        if path.is_symlink()
            || !rules.may_delete(&source, &name.to_string_lossy(), path.is_dir(), nested)
        {
            continue;
        }
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            remove_file_even_read_only(&path)
        };
        removed.map_err(|e| {
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
        if let (Ok(a), Ok(b)) = (std::fs::metadata(from), std::fs::metadata(to)) {
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
        remove_file_even_read_only(to).map_err(|e| {
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
    match (std::fs::metadata(from), std::fs::metadata(to)) {
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
    if to.is_dir() && !to.is_symlink() {
        std::fs::remove_dir_all(to).map_err(|e| {
            ExecutionError::Execution(format!("Failed to replace dir with file: {}", e))
        })?;
    }
    if unchanged_copy(from, to) {
        return Ok(());
    }
    // `fs::copy` writes into an existing file, which a read-only one
    // refuses; a new file takes the source's permissions either way.
    if to.symlink_metadata().is_ok() {
        remove_file_even_read_only(to).map_err(|e| {
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
}
