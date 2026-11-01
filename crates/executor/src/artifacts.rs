//! Local artifact storage for GitHub Actions `actions/upload-artifact` and
//! `actions/download-artifact` emulation.
//!
//! Artifacts are stored as plain files under a per-workflow-run temporary
//! directory, laid out relative to where the `path:` search started, as
//! `actions/upload-artifact` does.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Sanitize an artifact name to prevent path traversal.
///
/// Rejects names containing path separators or `..` components and strips
/// null bytes. Returns an error if the name is invalid.
fn sanitize_artifact_name(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("Artifact name cannot be empty".to_string());
    }
    // Reject names containing null bytes outright rather than silently stripping
    // (stripping could create collisions, e.g. "foo\0bar" → "foobar").
    if name.contains('\0') {
        return Err(format!(
            "Invalid artifact name '{}': contains null bytes",
            name
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.starts_with('.') {
        return Err(format!(
            "Invalid artifact name '{}': must not contain path separators, '..', or start with '.'",
            name
        ));
    }
    Ok(name.to_string())
}

/// Resolve `.` and `..` lexically, so that paths from the same `path:` line
/// compare equal however they were spelled.
fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::ParentDir) | None => normalized.push(".."),
                _ => {}
            },
            other => normalized.push(other),
        }
    }
    normalized
}

/// One glob or path from a `path:` input, resolved against the workspace.
struct PathLine<'a> {
    /// The line without trailing slashes, unless it is only slashes. An absolute
    /// line inside the canonical workspace is made relative to it, so that every
    /// path compared later shares one spelling of the workspace.
    text: Cow<'a, str>,
    /// The line joined to the workspace. Absolute lines stay as written.
    path: PathBuf,
    /// Whether the line names an existing directory.
    is_dir: bool,
}

impl<'a> PathLine<'a> {
    fn parse(workspace: &Path, canonical_workspace: &Path, line: &'a str) -> Self {
        let trimmed = line.trim_end_matches('/');
        let text = if trimmed.is_empty() { line } else { trimmed };
        let text = match normalize(Path::new(text)).strip_prefix(canonical_workspace) {
            Ok(rest) if rest.as_os_str().is_empty() => Cow::Borrowed("."),
            Ok(rest) => Cow::Owned(rest.to_string_lossy().into_owned()),
            Err(_) => Cow::Borrowed(text),
        };
        let path = workspace.join(text.as_ref());
        let is_dir = path.is_dir();
        Self { text, path, is_dir }
    }

    /// The components before the first wildcard, and the rest as a pattern.
    /// Only the line itself is searched, as the workspace path may contain
    /// wildcard characters.
    fn split_at_wildcard(&self) -> (PathBuf, Option<String>) {
        let components: Vec<_> = Path::new(self.text.as_ref()).components().collect();
        let wildcard = components
            .iter()
            .position(|c| c.as_os_str().to_string_lossy().contains(['*', '?', '[']));
        match wildcard {
            Some(index) if !self.is_dir => {
                let pattern: Vec<_> = components[index..]
                    .iter()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect();
                (
                    components[..index].iter().collect(),
                    Some(pattern.join("/")),
                )
            }
            _ => (components.iter().collect(), None),
        }
    }
}

/// Whether a file or directory is hidden, by the rule of upload-artifact.
fn is_hidden(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// The directory a `path:` line starts from: the directory it names, its part
/// before the first wildcard, or the parent of a single file.
fn search_root(workspace: &Path, line: &PathLine<'_>) -> PathBuf {
    let root = match line.split_at_wildcard() {
        (base, Some(_)) => workspace.join(base),
        (_, None) if line.is_dir => line.path.clone(),
        (_, None) => line
            .path
            .parent()
            .map_or_else(|| line.path.clone(), Path::to_path_buf),
    };
    normalize(&root)
}

/// A recursive search below one directory. Symlinked directories are followed
/// as upload-artifact follows them, unless they leave the workspace or lead
/// back to a directory the search is already inside.
struct Search<'a> {
    pattern: Option<&'a glob::Pattern>,
    /// How deep the pattern can match. `None` when it holds a `**`.
    max_depth: Option<usize>,
    include_hidden: bool,
    canonical_workspace: &'a Path,
}

impl Search<'_> {
    fn visit(&self, dir: &Path, relative: &Path, chain: &mut Vec<PathBuf>, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !self.include_hidden && is_hidden(&name) {
                continue;
            }
            let path = dir.join(&name);
            let relative = relative.join(&name);
            if path.is_dir() {
                self.descend(&path, &relative, chain, out);
            } else if path.is_file() && !path.is_symlink() && self.matches(&relative) {
                out.push(path);
            }
        }
    }

    fn descend(
        &self,
        dir: &Path,
        relative: &Path,
        chain: &mut Vec<PathBuf>,
        out: &mut Vec<PathBuf>,
    ) {
        if self
            .max_depth
            .is_some_and(|depth| relative.components().count() >= depth)
        {
            return;
        }
        let Ok(canonical) = dir.canonicalize() else {
            return;
        };
        if !canonical.starts_with(self.canonical_workspace) || chain.contains(&canonical) {
            return;
        }
        chain.push(canonical);
        self.visit(dir, relative, chain, out);
        chain.pop();
    }

    fn matches(&self, relative: &Path) -> bool {
        let options = glob::MatchOptions {
            require_literal_separator: true,
            ..glob::MatchOptions::new()
        };
        self.pattern
            .is_none_or(|pattern| pattern.matches_path_with(relative, options))
    }
}

/// Files within the workspace matching a path line, under the normalized path
/// that names them in the artifact. A file reached through a symlinked directory
/// keeps the path it was found at.
///
/// Unless `include_hidden` is set, hidden files and directories are skipped,
/// including the line's own file or search directory. The workspace itself is
/// exempt, as its name is not the user's choice: temporary directories start
/// with a dot.
fn matching_files(
    workspace: &Path,
    canonical_workspace: &Path,
    line: &PathLine<'_>,
    include_hidden: bool,
) -> Result<Vec<PathBuf>, String> {
    let hidden = |path: &Path| !include_hidden && path.file_name().is_some_and(is_hidden);
    let (_, wildcards) = line.split_at_wildcard();
    if !line.is_dir && wildcards.is_none() {
        let file = normalize(&line.path);
        let selected = !hidden(&file)
            && file.is_file()
            && !file.is_symlink()
            && file
                .canonicalize()
                .is_ok_and(|c| c.starts_with(canonical_workspace));
        return Ok(selected.then_some(file).into_iter().collect());
    }

    let pattern = wildcards
        .as_deref()
        .map(glob::Pattern::new)
        .transpose()
        .map_err(|e| format!("Invalid glob pattern '{}': {}", line.text, e))?;

    let root = search_root(workspace, line);
    let normalized_workspace = normalize(workspace);
    if !root.starts_with(&normalized_workspace) || (root != normalized_workspace && hidden(&root)) {
        return Ok(Vec::new());
    }
    let canonical_root = match root.canonicalize() {
        Ok(canonical) if canonical.starts_with(canonical_workspace) => canonical,
        _ => return Ok(Vec::new()),
    };
    let max_depth = wildcards
        .as_deref()
        .filter(|wildcards| !wildcards.contains("**"))
        .map(|wildcards| wildcards.split('/').count());

    let search = Search {
        pattern: pattern.as_ref(),
        max_depth,
        include_hidden,
        canonical_workspace,
    };
    let mut files = Vec::new();
    search.visit(&root, Path::new(""), &mut vec![canonical_root], &mut files);
    Ok(files)
}

/// The deepest directory containing every path, or `None` if there are none.
fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut root = paths.first()?.clone();
    while !paths.iter().all(|path| path.starts_with(&root)) {
        if !root.pop() {
            break;
        }
    }
    Some(root)
}

/// The files selected by a `path:` input and the directory they are relative to.
struct Selection {
    files: BTreeSet<PathBuf>,
    root: PathBuf,
}

/// Apply the lines of a `path:` input in order. See [`ArtifactStore::upload`].
fn select_files(
    workspace: &Path,
    path_pattern: &str,
    include_hidden: bool,
) -> Result<Selection, String> {
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize workspace: {}", e))?;
    // Paths are compared lexically below, which needs every one of them to be
    // absolute.
    let workspace = std::path::absolute(workspace)
        .map_err(|e| format!("Failed to resolve workspace path: {}", e))?;
    let normalized_workspace = normalize(&workspace);

    let mut files = BTreeSet::new();
    let mut search_roots = Vec::new();
    for line in path_pattern
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if let Some(exclude) = line.strip_prefix('!') {
            let exclude = exclude.trim_start();
            if exclude.is_empty() {
                return Err("Path line '!' has nothing to exclude".to_string());
            }
            let exclude = PathLine::parse(&workspace, &canonical_workspace, exclude);
            // Everything selected so far stays excludable, hidden or not.
            for file in matching_files(&workspace, &canonical_workspace, &exclude, true)? {
                files.remove(&file);
            }
        } else {
            let include = PathLine::parse(&workspace, &canonical_workspace, line);
            let root = search_root(&workspace, &include);
            if !root.starts_with(&normalized_workspace) {
                return Err(format!(
                    "Path '{}' is outside the workspace '{}'",
                    include.text,
                    workspace.display()
                ));
            }
            search_roots.push(root);
            files.extend(matching_files(
                &workspace,
                &canonical_workspace,
                &include,
                include_hidden,
            )?);
        }
    }

    // Like upload-artifact, paths are relative to where the search started, or
    // to what the search paths share when there are several.
    let root = common_ancestor(&search_roots).unwrap_or(normalized_workspace);
    Ok(Selection { files, root })
}

/// Copy the selected files into the empty directory `target`.
fn copy_selection(selection: &Selection, target: &Path) -> Result<usize, String> {
    for file in &selection.files {
        let rel = file.strip_prefix(&selection.root).map_err(|_| {
            format!(
                "File '{}' is not under the artifact root '{}'",
                file.display(),
                selection.root.display()
            )
        })?;
        // Joining anything else could name the source file itself, which
        // copying would truncate.
        if rel.as_os_str().is_empty() || rel.is_absolute() {
            return Err(format!(
                "File '{}' has no path relative to the artifact root '{}'",
                file.display(),
                selection.root.display()
            ));
        }
        let dest = target.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }
        std::fs::copy(file, &dest)
            .map_err(|e| format!("Failed to copy '{}': {}", file.display(), e))?;
    }
    Ok(selection.files.len())
}

/// Recursively collect all regular files under `dir`, skipping symlinks.
fn walk_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory '{}': {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip symlinks to prevent following links outside the artifact tree
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            files.extend(walk_files(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

struct ArtifactMetadata {
    /// Path to the artifact directory on disk, unique to each upload.
    path: PathBuf,
}

/// Manages artifact storage for a single workflow run.
#[derive(Clone)]
pub struct ArtifactStore {
    root: PathBuf,
    index: Arc<RwLock<HashMap<String, ArtifactMetadata>>>,
}

impl ArtifactStore {
    /// Create a new artifact store under `run_dir/artifacts/`.
    pub fn new(run_dir: &Path) -> std::io::Result<Self> {
        let root = run_dir.join("artifacts");
        std::fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            index: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Upload the files selected by a `path:` input into a named artifact.
    ///
    /// `path_pattern` holds one glob or path per line. Lines apply in order, so
    /// a later line overrides an earlier one: lines starting with `!` exclude
    /// files, and lines starting with `#` are comments. Paths in the artifact
    /// are relative to the search root of the lines. A line outside the
    /// workspace is an error, as its files could not be uploaded.
    ///
    /// Like upload-artifact, hidden files are skipped unless `include_hidden` is
    /// set, and so are hidden search directories such as `.config/`.
    ///
    /// Uploading replaces any artifact of the same name, all at once: the files
    /// are copied aside first, so a failed upload keeps the earlier artifact,
    /// and of several concurrent uploads exactly one is kept, whole.
    ///
    /// Returns the number of files uploaded. When that is zero, no artifact is
    /// recorded; whether that is an error is the caller's decision.
    pub async fn upload(
        &self,
        name: &str,
        path_pattern: &str,
        workspace: &Path,
        include_hidden: bool,
    ) -> Result<usize, String> {
        let safe_name = sanitize_artifact_name(name)?;
        let store_root = self.root.clone();
        let workspace = workspace.to_path_buf();
        let pattern = path_pattern.to_string();

        let staged = tokio::task::spawn_blocking(move || {
            let selection = select_files(&workspace, &pattern, include_hidden)?;
            if selection.files.is_empty() {
                return Ok(None);
            }
            // Artifact names cannot start with a dot, so this cannot collide.
            let stage = tempfile::Builder::new()
                .prefix(".upload-")
                .tempdir_in(&store_root)
                .map_err(|e| format!("Failed to create artifact directory: {}", e))?;
            let count = copy_selection(&selection, stage.path())?;
            wrkflw_logging::info(&format!(
                "  Artifact paths are relative to {}",
                selection.root.display()
            ));
            Ok::<_, String>(Some((stage.keep(), count)))
        })
        .await
        .map_err(|e| format!("Upload task panicked: {}", e))??;

        let Some((path, count)) = staged else {
            return Ok(0);
        };

        // Downloads hold the read lock, so none is reading what is removed here.
        let mut idx = self.index.write().await;
        if let Some(replaced) = idx.insert(safe_name, ArtifactMetadata { path }) {
            if let Err(e) = tokio::fs::remove_dir_all(&replaced.path).await {
                wrkflw_logging::warning(&format!(
                    "Failed to remove replaced artifact '{}': {}",
                    replaced.path.display(),
                    e
                ));
            }
        }

        Ok(count)
    }

    /// Download a named artifact into `target_dir`.
    ///
    /// Returns the number of files downloaded.
    pub async fn download(&self, name: &str, target_dir: &Path) -> Result<usize, String> {
        let safe_name = sanitize_artifact_name(name)?;
        let idx = self.index.read().await;
        let meta = idx
            .get(&safe_name)
            .ok_or_else(|| format!("Artifact '{}' not found", name))?;

        let artifact_dir = meta.path.clone();
        let target = target_dir.to_path_buf();

        // The read lock is held throughout, so that an upload replacing this
        // artifact waits rather than removing files being read.
        let downloaded = tokio::task::spawn_blocking(move || -> Result<usize, String> {
            let mut count = 0;
            for file_path in walk_files(&artifact_dir)? {
                let rel = file_path.strip_prefix(&artifact_dir).map_err(|_| {
                    format!(
                        "Artifact file '{}' is outside artifact directory '{}'",
                        file_path.display(),
                        artifact_dir.display()
                    )
                })?;
                let dest = target.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create directory: {}", e))?;
                }
                std::fs::copy(&file_path, &dest)
                    .map_err(|e| format!("Failed to copy '{}': {}", file_path.display(), e))?;
                count += 1;
            }
            Ok(count)
        })
        .await
        .map_err(|e| format!("Download task panicked: {}", e))?;
        drop(idx);
        downloaded
    }

    /// List all available artifact names.
    pub async fn list(&self) -> Vec<String> {
        let idx = self.index.read().await;
        idx.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn upload_and_download() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        // Create test files
        std::fs::write(workspace.path().join("file1.txt"), "hello").unwrap();
        std::fs::create_dir_all(workspace.path().join("sub")).unwrap();
        std::fs::write(workspace.path().join("sub/file2.txt"), "world").unwrap();

        let store = ArtifactStore::new(run_dir.path()).unwrap();

        // Upload
        let count = store
            .upload("my-artifact", "**/*.txt", workspace.path(), false)
            .await
            .unwrap();
        assert_eq!(count, 2);

        // List
        let names = store.list().await;
        assert_eq!(names, vec!["my-artifact"]);

        // Download to a different directory
        let download_dir = tempdir().unwrap();
        let count = store
            .download("my-artifact", download_dir.path())
            .await
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            std::fs::read_to_string(download_dir.path().join("file1.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(download_dir.path().join("sub/file2.txt")).unwrap(),
            "world"
        );
    }

    #[tokio::test]
    async fn download_missing_artifact() {
        let run_dir = tempdir().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let dl_dir = tempdir().unwrap();
        let result = store.download("nonexistent", dl_dir.path()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[tokio::test]
    async fn upload_no_matching_files() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let result = store
            .upload("empty", "*.nonexistent", workspace.path(), false)
            .await;
        assert_eq!(result.unwrap(), 0);
        assert!(store.list().await.is_empty());
    }

    /// Upload `path`, then download it, returning the downloaded directory.
    async fn round_trip(files: &[&str], path: &str) -> tempfile::TempDir {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        for file in files {
            let file = workspace.path().join(file);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "x").unwrap();
        }
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        store
            .upload("a", path, workspace.path(), false)
            .await
            .unwrap();
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        target
    }

    #[tokio::test]
    async fn directory_paths_upload_their_contents_relative_to_the_directory() {
        for path in ["out", "out/"] {
            let target = round_trip(&["out/a.txt", "out/sub/b.txt", "other.txt"], path).await;
            assert!(target.path().join("a.txt").is_file(), "{}", path);
            assert!(target.path().join("sub/b.txt").is_file(), "{}", path);
            assert!(!target.path().join("other.txt").exists(), "{}", path);
        }
    }

    #[tokio::test]
    async fn single_file_lands_at_the_artifact_root() {
        let target = round_trip(&["out/deep/a.txt"], "out/deep/a.txt").await;
        assert!(target.path().join("a.txt").is_file());
    }

    #[tokio::test]
    async fn wildcards_keep_the_hierarchy_after_the_first_wildcard() {
        let target = round_trip(&["out/a/b/c.txt"], "out/**/*.txt").await;
        assert!(target.path().join("a/b/c.txt").is_file());

        let target = round_trip(&["out/c.txt"], "out/*").await;
        assert!(target.path().join("c.txt").is_file());
    }

    #[tokio::test]
    async fn multiple_lines_keep_paths_below_their_common_parent() {
        let target = round_trip(
            &["dist/a/1.txt", "dist/b/2.txt", "dist/b/skip.log"],
            "dist/a/\n\n  dist/b/*\n!dist/b/*.log\n",
        )
        .await;
        assert!(target.path().join("a/1.txt").is_file());
        assert!(target.path().join("b/2.txt").is_file());
        assert!(!target.path().join("b/skip.log").exists());
    }

    #[tokio::test]
    async fn paths_outside_the_workspace_are_not_collected() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let outside = run_dir.path().join("secret.txt");
        std::fs::write(&outside, "x").unwrap();
        let result = store
            .upload("a", &outside.to_string_lossy(), workspace.path(), false)
            .await;
        assert!(result.unwrap_err().contains("outside the workspace"));
    }

    #[tokio::test]
    async fn a_line_outside_the_workspace_fails_rather_than_moving_the_root() {
        for outside in ["../nope/", "/nope", "/", "../*.txt"] {
            let run_dir = tempdir().unwrap();
            let workspace = tempdir().unwrap();
            std::fs::create_dir_all(workspace.path().join("out")).unwrap();
            std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
            let store = ArtifactStore::new(run_dir.path()).unwrap();
            let path = format!("out/\n{}", outside);
            let result = store.upload("a", &path, workspace.path(), false).await;
            assert!(
                result.unwrap_err().contains("outside the workspace"),
                "{}",
                outside
            );
            assert!(store.list().await.is_empty(), "{}", outside);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_relative_workspace_never_overwrites_its_source_files() {
        let run_dir = tempdir().unwrap();
        let workspace = tempfile::tempdir_in(".").unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "payload").unwrap();
        std::fs::write(workspace.path().join("b.txt"), "x").unwrap();
        let absolute = workspace.path().join("out").canonicalize().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let path = format!("b.txt\n{}", absolute.display());
        let count = store
            .upload("a", &path, workspace.path(), false)
            .await
            .unwrap();
        assert_eq!(count, 2);
        let source = std::fs::read_to_string(workspace.path().join("out/a.txt")).unwrap();
        assert_eq!(source, "payload");
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        assert!(target.path().join("b.txt").is_file());
        assert!(target.path().join("out/a.txt").is_file());
    }

    #[tokio::test]
    async fn a_later_line_overrides_an_earlier_exclusion() {
        let target = round_trip(&["out/a.log", "out/b.log"], "out/\n!out/*.log\nout/a.log").await;
        assert!(target.path().join("a.log").is_file());
        assert!(!target.path().join("b.log").exists());
    }

    #[tokio::test]
    async fn an_exclusion_only_applies_to_earlier_lines() {
        let target = round_trip(&["out/a.log"], "!out/*.log\nout/").await;
        assert!(target.path().join("a.log").is_file());
    }

    #[tokio::test]
    async fn a_file_matched_under_two_spellings_is_uploaded_once() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let count = store
            .upload("a", "out/../out/\nout/", workspace.path(), false)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn uploading_again_replaces_the_artifact() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        std::fs::write(workspace.path().join("out/b.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        store
            .upload("a", "out/", workspace.path(), false)
            .await
            .unwrap();
        std::fs::remove_file(workspace.path().join("out/b.txt")).unwrap();
        store
            .upload("a", "out/", workspace.path(), false)
            .await
            .unwrap();
        let target = tempdir().unwrap();
        assert_eq!(store.download("a", target.path()).await.unwrap(), 1);
        assert!(!target.path().join("b.txt").exists());
    }

    #[tokio::test]
    async fn a_failed_upload_keeps_the_earlier_artifact() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::write(workspace.path().join("a.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        store
            .upload("a", "a.txt", workspace.path(), false)
            .await
            .unwrap();
        assert!(store
            .upload("a", "../nope", workspace.path(), false)
            .await
            .is_err());
        assert_eq!(
            store.upload("a", "*.none", workspace.path(), false).await,
            Ok(0)
        );
        let target = tempdir().unwrap();
        assert_eq!(store.download("a", target.path()).await.unwrap(), 1);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_trailing_backslash_names_a_directory() {
        let target = round_trip(&["out/a.txt"], "out\\").await;
        assert!(target.path().join("a.txt").is_file());
    }

    #[tokio::test]
    async fn trailing_double_star_uploads_everything_below_it() {
        let target = round_trip(&["out/a.txt", "out/sub/b.txt", "other.txt"], "out/**").await;
        assert!(target.path().join("a.txt").is_file());
        assert!(target.path().join("sub/b.txt").is_file());
        assert!(!target.path().join("other.txt").exists());
    }

    #[tokio::test]
    async fn trailing_double_star_works_in_exclusions() {
        let target = round_trip(&["out/a.txt", "out/cache/b.txt"], "out/\n!out/cache/**").await;
        assert!(target.path().join("a.txt").is_file());
        assert!(!target.path().join("cache/b.txt").exists());
    }

    #[tokio::test]
    async fn comment_lines_are_ignored() {
        let target = round_trip(&["out/a.txt", "other/b.txt"], "# other/\nout/").await;
        assert!(target.path().join("a.txt").is_file());
        assert!(!target.path().join("b.txt").exists());
    }

    #[tokio::test]
    async fn dot_paths_are_relative_to_the_workspace() {
        for path in [".", "./out/..", "./*"] {
            let target = round_trip(&["a.txt", "out/b.txt"], path).await;
            assert!(target.path().join("a.txt").is_file(), "{}", path);
        }
    }

    #[tokio::test]
    async fn absolute_paths_inside_the_workspace_are_uploaded() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let path = workspace.path().join("out");
        store
            .upload("a", &path.to_string_lossy(), workspace.path(), false)
            .await
            .unwrap();
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        assert!(target.path().join("a.txt").is_file());
    }

    #[tokio::test]
    async fn a_line_matching_nothing_still_counts_towards_the_root() {
        let target = round_trip(&["dist/a/1.txt"], "dist/a/\ndist/missing/").await;
        assert!(target.path().join("a/1.txt").is_file());
    }

    #[tokio::test]
    async fn only_exclusions_select_nothing() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::write(workspace.path().join("a.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let count = store
            .upload("a", "!*.log", workspace.path(), false)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn glob_characters_in_the_workspace_path_are_literal() {
        let run_dir = tempdir().unwrap();
        let base = tempdir().unwrap();
        let workspace = base.path().join("proj[1]");
        std::fs::create_dir_all(workspace.join("out/sub")).unwrap();
        std::fs::write(workspace.join("out/sub/a.txt"), "x").unwrap();
        std::fs::write(workspace.join("b.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        for (path, expected) in [
            ("out/", "sub/a.txt"),
            ("out/*/*.txt", "sub/a.txt"),
            ("b.txt", "b.txt"),
        ] {
            store.upload("a", path, &workspace, false).await.unwrap();
            let target = tempdir().unwrap();
            store.download("a", target.path()).await.unwrap();
            assert!(target.path().join(expected).is_file(), "{}", path);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_directories_keep_the_path_they_were_found_at() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::create_dir_all(workspace.path().join("shared")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        std::fs::write(workspace.path().join("shared/s.txt"), "x").unwrap();
        std::os::unix::fs::symlink(
            workspace.path().join("shared"),
            workspace.path().join("out/link"),
        )
        .unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        store
            .upload("a", "out/", workspace.path(), false)
            .await
            .unwrap();
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        assert!(target.path().join("a.txt").is_file());
        assert!(target.path().join("link/s.txt").is_file());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinked_directories_leaving_the_workspace_are_not_collected() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join("out/link")).unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let count = store
            .upload("a", "out/", workspace.path(), false)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_cycles_do_not_duplicate_files() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out/sub")).unwrap();
        std::fs::write(workspace.path().join("out/a.txt"), "x").unwrap();
        std::os::unix::fs::symlink("..", workspace.path().join("out/sub/loop")).unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        for path in ["out/", "out/**", "out/**/*.txt"] {
            let count = store.upload("a", path, workspace.path(), false).await;
            assert_eq!(count, Ok(1), "{}", path);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_between_directories_terminate() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let root = workspace.path();
        std::fs::create_dir_all(root.join("out/x")).unwrap();
        std::fs::create_dir_all(root.join("out/y")).unwrap();
        std::fs::write(root.join("out/x/a.txt"), "x").unwrap();
        // Would be 2^n paths for a search that only stops at ELOOP.
        for link in ["l1", "l2", "l3", "l4"] {
            std::os::unix::fs::symlink(".", root.join("out").join(link)).unwrap();
            std::os::unix::fs::symlink("../y", root.join("out/x").join(link)).unwrap();
            std::os::unix::fs::symlink("../x", root.join("out/y").join(link)).unwrap();
        }
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let count = store.upload("a", "out/", root, false).await.unwrap();
        assert!(count >= 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn two_links_to_one_directory_both_upload() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let root = workspace.path();
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        std::fs::write(root.join("shared/s.txt"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("shared"), root.join("out/one")).unwrap();
        std::os::unix::fs::symlink(root.join("shared"), root.join("out/two")).unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        assert_eq!(store.upload("a", "out/", root, false).await, Ok(2));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_canonical_line_shares_the_root_of_a_symlinked_workspace() {
        let run_dir = tempdir().unwrap();
        let base = tempdir().unwrap();
        let real = base.path().join("real");
        std::fs::create_dir_all(real.join("out")).unwrap();
        std::fs::write(real.join("out/a.txt"), "x").unwrap();
        std::fs::write(real.join("b.txt"), "x").unwrap();
        let link = base.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let canonical = real.canonicalize().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let path = format!(
            "b.txt\n{}/out\n{}/out/a.txt",
            canonical.display(),
            canonical.display()
        );
        assert_eq!(store.upload("a", &path, &link, false).await, Ok(2));
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        assert!(target.path().join("b.txt").is_file());
        assert!(target.path().join("out/a.txt").is_file());
    }

    #[tokio::test]
    async fn hidden_files_are_skipped_by_default() {
        let files = [
            ".git/config",
            ".env",
            "b.txt",
            "out/.secret",
            "out/.cache/c.txt",
        ];
        for path in [".", "./*", "**/*", "**"] {
            let target = round_trip(&files, path).await;
            assert!(target.path().join("b.txt").is_file(), "{}", path);
            assert!(!target.path().join(".git").exists(), "{}", path);
            assert!(!target.path().join(".env").exists(), "{}", path);
            assert!(!target.path().join("out").exists(), "{}", path);
        }
    }

    #[tokio::test]
    async fn hidden_search_paths_are_skipped_by_default() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join(".config")).unwrap();
        std::fs::write(workspace.path().join(".config/a.txt"), "x").unwrap();
        std::fs::write(workspace.path().join(".env"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        for path in [".config/", ".config/*", ".env"] {
            let count = store.upload("a", path, workspace.path(), false).await;
            assert_eq!(count, Ok(0), "{}", path);
        }
        // Only the name of the file itself counts for a single file.
        let count = store
            .upload("a", ".config/a.txt", workspace.path(), false)
            .await;
        assert_eq!(count, Ok(1));
    }

    #[tokio::test]
    async fn include_hidden_uploads_hidden_files_and_exclusions_still_apply() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("out/.cache")).unwrap();
        std::fs::write(workspace.path().join("out/.env"), "x").unwrap();
        std::fs::write(workspace.path().join("out/.cache/c.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let count = store
            .upload("a", "out/\n!out/.cache/", workspace.path(), true)
            .await;
        assert_eq!(count, Ok(1));
        let target = tempdir().unwrap();
        store.download("a", target.path()).await.unwrap();
        assert!(target.path().join(".env").is_file());
        assert!(!target.path().join(".cache").exists());
    }

    #[tokio::test]
    async fn an_exclusion_with_nothing_to_exclude_is_an_error() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::write(workspace.path().join("a.txt"), "x").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        for path in ["*.txt\n!", "*.txt\n!   "] {
            let result = store.upload("a", path, workspace.path(), false).await;
            assert!(result.unwrap_err().contains("nothing to exclude"));
        }
    }

    #[tokio::test]
    async fn exclusions_outside_the_workspace_match_nothing() {
        let target = round_trip(&["a.txt"], "*.txt\n!../*").await;
        assert!(target.path().join("a.txt").is_file());
    }

    #[tokio::test]
    async fn an_invalid_pattern_is_an_error() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let result = store.upload("a", "out/***", workspace.path(), false).await;
        assert!(result.unwrap_err().contains("Invalid glob pattern"));
    }

    #[tokio::test]
    async fn a_star_does_not_cross_directories() {
        let target = round_trip(&["out/a.txt", "out/sub/b.txt"], "out/*.txt").await;
        assert!(target.path().join("a.txt").is_file());
        assert!(!target.path().join("sub").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_uploads_of_one_name_keep_one_whole_upload() {
        let run_dir = tempdir().unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        let mut tasks = Vec::new();
        let mut workspaces = Vec::new();
        for leg in 0..8 {
            let workspace = tempdir().unwrap();
            for file in 0..32 {
                let name = format!("{}-{}.txt", leg, file);
                std::fs::write(workspace.path().join(name), "x").unwrap();
            }
            let store = store.clone();
            let path = workspace.path().to_path_buf();
            workspaces.push(workspace);
            tasks.push(tokio::spawn(async move {
                store.upload("shared", "*.txt", &path, false).await
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), Ok(32));
        }

        let target = tempdir().unwrap();
        assert_eq!(store.download("shared", target.path()).await, Ok(32));
        let legs: BTreeSet<String> = std::fs::read_dir(target.path())
            .unwrap()
            .map(|entry| {
                let name = entry.unwrap().file_name().to_string_lossy().into_owned();
                name.split('-').next().unwrap().to_string()
            })
            .collect();
        assert_eq!(legs.len(), 1, "{:?}", legs);

        // Replaced uploads leave nothing behind.
        let stored = std::fs::read_dir(run_dir.path().join("artifacts")).unwrap();
        assert_eq!(stored.count(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_copy_keeps_the_earlier_artifact() {
        use std::os::unix::fs::PermissionsExt;
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let unreadable = workspace.path().join("b.txt");
        std::fs::write(workspace.path().join("a.txt"), "first").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();
        store
            .upload("a", "a.txt", workspace.path(), false)
            .await
            .unwrap();

        std::fs::write(&unreadable, "x").unwrap();
        std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&unreadable).is_ok() {
            return; // Running as root, permissions cannot make the copy fail.
        }
        std::fs::write(workspace.path().join("a.txt"), "second").unwrap();
        let result = store.upload("a", "*.txt", workspace.path(), false).await;
        assert!(result.unwrap_err().contains("Failed to copy"));

        let target = tempdir().unwrap();
        assert_eq!(store.download("a", target.path()).await, Ok(1));
        let kept = std::fs::read_to_string(target.path().join("a.txt")).unwrap();
        assert_eq!(kept, "first");
        let stored = std::fs::read_dir(run_dir.path().join("artifacts")).unwrap();
        assert_eq!(stored.count(), 1);
    }

    #[test]
    fn common_ancestor_of_nothing_is_none() {
        assert_eq!(common_ancestor(&[]), None);
    }

    #[test]
    fn normalize_resolves_dot_components() {
        assert_eq!(normalize(Path::new("/a/./b/../c")), Path::new("/a/c"));
        assert_eq!(normalize(Path::new("./a")), Path::new("a"));
        assert_eq!(normalize(Path::new("../a")), Path::new("../a"));
        assert_eq!(normalize(Path::new("/../a")), Path::new("/a"));
    }

    #[tokio::test]
    async fn rejects_path_traversal_in_artifact_name() {
        let run_dir = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        std::fs::write(workspace.path().join("f.txt"), "data").unwrap();
        let store = ArtifactStore::new(run_dir.path()).unwrap();

        let result = store
            .upload("../../escape", "*.txt", workspace.path(), false)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));

        let result = store
            .upload("foo/bar", "*.txt", workspace.path(), false)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));

        let result = store.download("../escape", workspace.path()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));
    }
}
