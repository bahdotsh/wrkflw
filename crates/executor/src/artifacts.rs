//! Local artifact storage for GitHub Actions `actions/upload-artifact` and
//! `actions/download-artifact` emulation.
//!
//! Artifacts are stored as plain files under a per-workflow-run temporary
//! directory, preserving directory structure relative to the workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// Files within the workspace matching a glob or directory path, as
/// (original, canonical) pairs.
fn matching_files(
    workspace: &Path,
    pattern: &str,
    canonical_workspace: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let trimmed = pattern.trim_end_matches('/');
    let path = workspace.join(trimmed);
    let full_pattern = if path.is_dir() {
        path.join("**").join("*")
    } else {
        path
    };
    Ok(glob::glob(&full_pattern.to_string_lossy())
        .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?
        .filter_map(|e| e.ok())
        .filter(|p| p.is_file() && !p.is_symlink())
        .filter_map(|p| {
            p.canonicalize()
                .ok()
                .filter(|c| c.starts_with(canonical_workspace))
                .map(|c| (p, c))
        })
        .collect())
}

/// The directory a `path:` line starts from: its part before the first
/// wildcard, or the parent of a single file.
fn search_root(workspace: &Path, pattern: &str) -> PathBuf {
    let path = workspace.join(pattern.trim_end_matches('/'));
    let wildcard = path
        .components()
        .position(|c| c.as_os_str().to_string_lossy().contains(['*', '?', '[']));
    match wildcard {
        Some(index) => path.components().take(index).collect(),
        None if path.is_dir() => path,
        None => path.parent().map(Path::to_path_buf).unwrap_or(path),
    }
}

/// The deepest directory containing every path. `paths` must not be empty.
fn common_ancestor(paths: &[PathBuf]) -> PathBuf {
    let mut root = paths[0].clone();
    while !paths.iter().all(|path| path.starts_with(&root)) {
        if !root.pop() {
            break;
        }
    }
    root
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

    /// Upload files matching a glob pattern into a named artifact.
    ///
    /// Files are copied from `workspace` preserving their relative paths.
    /// Returns the number of files uploaded.
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
            let lines: Vec<&str> = pattern
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect();
            let includes: Vec<&str> = lines
                .iter()
                .copied()
                .filter(|line| !line.starts_with('!'))
                .collect();
            let excludes: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix('!')).collect();

            // Collect (original, canonical) pairs in one pass to avoid
            // double-canonicalize per file.
            let mut entries: Vec<(PathBuf, PathBuf)> = Vec::new();
            for include in &includes {
                entries.extend(matching_files(&ws, include, &canonical_workspace)?);
            }
            for exclude in &excludes {
                let excluded = matching_files(&ws, exclude, &canonical_workspace)?;
                entries.retain(|(_, canonical)| !excluded.iter().any(|(_, c)| c == canonical));
            }
            entries.sort_by(|a, b| a.1.cmp(&b.1));
            entries.dedup_by(|a, b| a.1 == b.1);

            if entries.is_empty() {
                return Ok(0);
            }

            std::fs::create_dir_all(&ad)
                .map_err(|e| format!("Failed to create artifact directory: {}", e))?;

            // Like upload-artifact, paths are relative to where the search
            // started, or to what the search paths share when there are several.
            let search_roots: Vec<PathBuf> = includes
                .iter()
                .filter_map(|include| search_root(&ws, include).canonicalize().ok())
                .collect();
            let root = common_ancestor(&search_roots);

            let mut count = 0;
            for (entry, canonical_entry) in &entries {
                let rel = canonical_entry.strip_prefix(&root).map_err(|_| {
                    format!(
                        "File '{}' is not within workspace '{}'",
                        entry.display(),
                        ws.display()
                    )
                })?;
                let dest = ad.join(rel);
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
        let count = store
            .upload("a", &outside.to_string_lossy(), workspace.path())
            .await
            .unwrap();
        assert_eq!(count, 0);
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
