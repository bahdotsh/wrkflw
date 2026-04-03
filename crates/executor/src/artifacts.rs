//! Local artifact storage for GitHub Actions `actions/upload-artifact` and
//! `actions/download-artifact` emulation.
//!
//! Artifacts are stored as plain files under a per-workflow-run temporary
//! directory, preserving directory structure relative to the workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Recursively collect all files under `dir`.
fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(walk_files(&path));
            } else {
                files.push(path);
            }
        }
    }
    files
}

struct ArtifactMetadata {
    /// Path to the artifact directory on disk.
    path: PathBuf,
}

/// Manages artifact storage for a single workflow run.
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
        let artifact_dir = self.root.join(name);
        std::fs::create_dir_all(&artifact_dir)
            .map_err(|e| format!("Failed to create artifact directory: {}", e))?;

        let full_pattern = workspace.join(path_pattern).to_string_lossy().to_string();
        let entries: Vec<PathBuf> = glob::glob(&full_pattern)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", path_pattern, e))?
            .filter_map(|e| e.ok())
            .filter(|p| p.is_file())
            .collect();

        if entries.is_empty() {
            return Err(format!(
                "No files found matching pattern '{}' in {}",
                path_pattern,
                workspace.display()
            ));
        }

        let mut count = 0;
        for entry in &entries {
            let rel = entry.strip_prefix(workspace).unwrap_or(entry.as_path());
            let dest = artifact_dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
            std::fs::copy(entry, &dest)
                .map_err(|e| format!("Failed to copy '{}': {}", entry.display(), e))?;
            count += 1;
        }

        let mut idx = self.index.write().await;
        idx.insert(name.to_string(), ArtifactMetadata { path: artifact_dir });

        Ok(count)
    }

    /// Download a named artifact into `target_dir`.
    ///
    /// Returns the number of files downloaded.
    pub async fn download(&self, name: &str, target_dir: &Path) -> Result<usize, String> {
        let idx = self.index.read().await;
        let meta = idx
            .get(name)
            .ok_or_else(|| format!("Artifact '{}' not found", name))?;

        let artifact_dir = &meta.path;
        let mut count = 0;

        for file_path in walk_files(artifact_dir) {
            let rel = file_path.strip_prefix(artifact_dir).unwrap_or(&file_path);
            let dest = target_dir.join(rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
            std::fs::copy(&file_path, &dest)
                .map_err(|e| format!("Failed to copy '{}': {}", file_path.display(), e))?;
            count += 1;
        }

        Ok(count)
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
}
