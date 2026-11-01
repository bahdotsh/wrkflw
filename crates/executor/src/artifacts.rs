//! Local artifact storage for GitHub Actions `actions/upload-artifact` and
//! `actions/download-artifact` emulation.
//!
//! Artifacts are stored as plain files under a per-workflow-run temporary
//! directory, laid out relative to where the `path:` search started, as
//! `actions/upload-artifact` does.

use std::collections::{BTreeMap, HashMap};
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
    /// The line without trailing slashes, unless it is only slashes.
    text: &'a str,
    /// The line joined to the workspace. Absolute lines stay as written.
    path: PathBuf,
    /// Whether the line names an existing directory.
    is_dir: bool,
}

impl<'a> PathLine<'a> {
    fn parse(workspace: &Path, line: &'a str) -> Self {
        let trimmed = line.trim_end_matches('/');
        let text = if trimmed.is_empty() { line } else { trimmed };
        let path = workspace.join(text);
        let is_dir = path.is_dir();
        Self { text, path, is_dir }
    }
}

/// Files within the workspace matching a path line, as (normalized, original)
/// pairs. Containment is decided on the canonical path; the normalized path is
/// what names the file in the artifact, so a file reached through a symlinked
/// directory keeps the path it was found at.
fn matching_files(
    workspace: &Path,
    line: &PathLine<'_>,
    canonical_workspace: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let full_pattern = if line.is_dir {
        PathBuf::from(glob::Pattern::escape(&line.path.to_string_lossy()))
            .join("**")
            .join("*")
    } else {
        let glob_path = if Path::new(line.text).is_absolute() {
            PathBuf::from(line.text)
        } else {
            PathBuf::from(glob::Pattern::escape(&workspace.to_string_lossy())).join(line.text)
        };
        // The glob crate's trailing `**` matches directories only, whereas
        // `dist/**` in a workflow means everything under `dist`.
        if glob_path.file_name().is_some_and(|name| name == "**") {
            glob_path.join("*")
        } else {
            glob_path
        }
    };
    Ok(glob::glob(&full_pattern.to_string_lossy())
        .map_err(|e| format!("Invalid glob pattern '{}': {}", line.text, e))?
        .filter_map(|e| e.ok())
        .filter(|p| p.is_file() && !p.is_symlink())
        .filter(|p| {
            p.canonicalize()
                .is_ok_and(|c| c.starts_with(canonical_workspace))
        })
        .map(|p| (normalize(&p), p))
        .collect())
}

/// The directory a `path:` line starts from: the directory it names, its part
/// before the first wildcard, or the parent of a single file. Only the line
/// itself is searched for wildcards, as the workspace path may contain them.
fn search_root(workspace: &Path, line: &PathLine<'_>) -> PathBuf {
    if line.is_dir {
        return normalize(&line.path);
    }
    let text = Path::new(line.text);
    let wildcard = text
        .components()
        .position(|c| c.as_os_str().to_string_lossy().contains(['*', '?', '[']));
    let root = match wildcard {
        Some(index) => workspace.join(text.components().take(index).collect::<PathBuf>()),
        None => line
            .path
            .parent()
            .map_or_else(|| line.path.clone(), Path::to_path_buf),
    };
    normalize(&root)
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
    /// Path to the artifact directory on disk.
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
    /// Uploading replaces any artifact of the same name. Unlike upload-artifact,
    /// hidden files are included.
    ///
    /// Returns the number of files uploaded. When that is zero, no artifact is
    /// recorded; whether that is an error is the caller's decision.
    pub async fn upload(
        &self,
        name: &str,
        path_pattern: &str,
        workspace: &Path,
    ) -> Result<usize, String> {
        let safe_name = sanitize_artifact_name(name)?;
        let artifact_dir = self.root.join(&safe_name);
        let workspace = workspace.to_path_buf();
        let pattern = path_pattern.to_string();

        let ad = artifact_dir.clone();
        let ws = workspace.clone();
        let count = tokio::task::spawn_blocking(move || -> Result<usize, String> {
            let canonical_workspace = ws
                .canonicalize()
                .map_err(|e| format!("Failed to canonicalize workspace: {}", e))?;
            // Paths are compared lexically below, which needs every one of
            // them to be absolute.
            let ws = std::path::absolute(&ws)
                .map_err(|e| format!("Failed to resolve workspace path: {}", e))?;
            let normalized_workspace = normalize(&ws);

            // Keyed by normalized path: this removes files matched by several
            // lines, and a file stays selected under each path it is found at.
            let mut entries: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
            let mut search_roots: Vec<PathBuf> = Vec::new();
            for line in pattern
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                if let Some(exclude) = line.strip_prefix('!') {
                    let exclude = PathLine::parse(&ws, exclude);
                    for (normalized, _) in matching_files(&ws, &exclude, &canonical_workspace)? {
                        entries.remove(&normalized);
                    }
                } else {
                    let include = PathLine::parse(&ws, line);
                    let root = search_root(&ws, &include);
                    if !root.starts_with(&normalized_workspace)
                        && !root.starts_with(&canonical_workspace)
                    {
                        return Err(format!(
                            "Path '{}' is outside the workspace '{}'",
                            include.text,
                            ws.display()
                        ));
                    }
                    search_roots.push(root);
                    entries.extend(matching_files(&ws, &include, &canonical_workspace)?);
                }
            }

            if entries.is_empty() {
                return Ok(0);
            }

            // Like upload-artifact, paths are relative to where the search
            // started, or to what the search paths share when there are several.
            let root = common_ancestor(&search_roots).unwrap_or(normalized_workspace);

            // Resolve every destination first, so that a failure leaves any
            // earlier artifact of this name in place.
            let mut copies = Vec::with_capacity(entries.len());
            for (normalized, entry) in &entries {
                let rel = normalized.strip_prefix(&root).map_err(|_| {
                    format!(
                        "File '{}' is not under the artifact root '{}'",
                        entry.display(),
                        root.display()
                    )
                })?;
                // Joining anything else could name the source file itself,
                // which copying would truncate.
                if rel.as_os_str().is_empty() || rel.is_absolute() {
                    return Err(format!(
                        "File '{}' has no path relative to the artifact root '{}'",
                        entry.display(),
                        root.display()
                    ));
                }
                copies.push((entry, ad.join(rel)));
            }

            if ad.exists() {
                std::fs::remove_dir_all(&ad)
                    .map_err(|e| format!("Failed to replace artifact directory: {}", e))?;
            }
            std::fs::create_dir_all(&ad)
                .map_err(|e| format!("Failed to create artifact directory: {}", e))?;

            let mut count = 0;
            for (entry, dest) in copies {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create directory: {}", e))?;
                }
                std::fs::copy(entry, &dest)
                    .map_err(|e| format!("Failed to copy '{}': {}", entry.display(), e))?;
                count += 1;
            }
            Ok(count)
        })
        .await
        .map_err(|e| format!("Upload task panicked: {}", e))??;

        if count == 0 {
            return Ok(0);
        }

        let mut idx = self.index.write().await;
        idx.insert(
            safe_name.to_string(),
            ArtifactMetadata { path: artifact_dir },
        );

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
        drop(idx);

        tokio::task::spawn_blocking(move || -> Result<usize, String> {
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
        .map_err(|e| format!("Download task panicked: {}", e))?
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
            .upload("my-artifact", "**/*.txt", workspace.path())
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
            .upload("empty", "*.nonexistent", workspace.path())
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
        store.upload("a", path, workspace.path()).await.unwrap();
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
            .upload("a", &outside.to_string_lossy(), workspace.path())
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
            let result = store.upload("a", &path, workspace.path()).await;
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
        let count = store.upload("a", &path, workspace.path()).await.unwrap();
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
            .upload("a", "out/../out/\nout/", workspace.path())
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
        store.upload("a", "out/", workspace.path()).await.unwrap();
        std::fs::remove_file(workspace.path().join("out/b.txt")).unwrap();
        store.upload("a", "out/", workspace.path()).await.unwrap();
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
        store.upload("a", "a.txt", workspace.path()).await.unwrap();
        assert!(store
            .upload("a", "../nope", workspace.path())
            .await
            .is_err());
        assert_eq!(store.upload("a", "*.none", workspace.path()).await, Ok(0));
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
            .upload("a", &path.to_string_lossy(), workspace.path())
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
        let count = store.upload("a", "!*.log", workspace.path()).await.unwrap();
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
            store.upload("a", path, &workspace).await.unwrap();
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
        store.upload("a", "out/", workspace.path()).await.unwrap();
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
        let count = store.upload("a", "out/", workspace.path()).await.unwrap();
        assert_eq!(count, 1);
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
            .upload("../../escape", "*.txt", workspace.path())
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));

        let result = store.upload("foo/bar", "*.txt", workspace.path()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));

        let result = store.download("../escape", workspace.path()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid artifact name"));
    }
}
