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
            std::fs::create_dir_all(&ad)
                .map_err(|e| format!("Failed to create artifact directory: {}", e))?;

            let canonical_workspace = ws
                .canonicalize()
                .map_err(|e| format!("Failed to canonicalize workspace: {}", e))?;
            let full_pattern = ws.join(&pattern).to_string_lossy().to_string();
            // Collect (original, canonical) pairs in one pass to avoid
            // double-canonicalize per file.
            let entries: Vec<(PathBuf, PathBuf)> = glob::glob(&full_pattern)
                .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?
                .filter_map(|e| e.ok())
                .filter(|p| p.is_file() && !p.is_symlink())
                .filter_map(|p| {
                    p.canonicalize()
                        .ok()
                        .filter(|c| c.starts_with(&canonical_workspace))
                        .map(|c| (p, c))
                })
                .collect();

            if entries.is_empty() {
                return Err(format!(
                    "No files found matching pattern '{}' in {}",
                    pattern,
                    ws.display()
                ));
            }

            let mut count = 0;
            for (entry, canonical_entry) in &entries {
                let rel = canonical_entry
                    .strip_prefix(&canonical_workspace)
                    .map_err(|_| {
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
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No files found"));
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
