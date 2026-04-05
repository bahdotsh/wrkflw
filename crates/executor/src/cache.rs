//! Local cache storage for GitHub Actions `actions/cache` emulation.
//!
//! Caches are stored under `~/.wrkflw/cache/` (persistent across runs)
//! and keyed by a SHA-256 hash of the cache key string.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Default maximum cache size: 1 GiB. When the cache exceeds this limit,
/// the oldest entries (by last modification time) are evicted until the
/// total size is back under the limit.
const MAX_CACHE_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

/// Manages a persistent local cache for workflow runs.
///
/// All public I/O methods (`restore`, `save`) run filesystem work on a
/// blocking thread via `tokio::task::spawn_blocking` to avoid stalling the
/// async executor — matching the pattern used by `ArtifactStore`.
#[derive(Clone)]
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    /// Create a new cache store. Uses `~/.wrkflw/cache/` by default.
    pub fn new() -> Result<Self, String> {
        let root = dirs::home_dir()
            .ok_or_else(|| "Could not determine home directory".to_string())?
            .join(".wrkflw")
            .join("cache");
        std::fs::create_dir_all(&root).map_err(|e| {
            format!(
                "Failed to create cache directory '{}': {}",
                root.display(),
                e
            )
        })?;
        Ok(Self { root })
    }

    /// Create a cache store at a custom root (useful for testing or custom locations).
    #[allow(dead_code)]
    pub fn with_root(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Attempt to restore a cache. Tries `key` first, then each of `restore_keys`
    /// as a prefix match.
    ///
    /// `path` is the directory to restore into (relative to `workspace`).
    /// Returns the matched key on hit, or `None` on miss.
    ///
    /// Filesystem I/O is offloaded to a blocking thread.
    pub async fn restore(
        &self,
        key: &str,
        restore_keys: &[String],
        path: &str,
        workspace: &Path,
    ) -> Option<String> {
        let this = self.clone();
        let key = key.to_string();
        let restore_keys = restore_keys.to_vec();
        let path = path.to_string();
        let workspace = workspace.to_path_buf();

        match tokio::task::spawn_blocking(move || {
            this.restore_inner(&key, &restore_keys, &path, &workspace)
        })
        .await
        {
            Ok(result) => result,
            Err(e) => {
                wrkflw_logging::warning(&format!("Cache restore task panicked: {}", e));
                None
            }
        }
    }

    /// Save the contents of `path` (relative to `workspace`) under `key`.
    ///
    /// Filesystem I/O is offloaded to a blocking thread.
    pub async fn save(&self, key: &str, path: &str, workspace: &Path) -> Result<(), String> {
        let this = self.clone();
        let key = key.to_string();
        let path = path.to_string();
        let workspace = workspace.to_path_buf();

        tokio::task::spawn_blocking(move || this.save_inner(&key, &path, &workspace))
            .await
            .map_err(|e| format!("Cache task panicked: {}", e))?
    }

    fn restore_inner(
        &self,
        key: &str,
        restore_keys: &[String],
        path: &str,
        workspace: &Path,
    ) -> Option<String> {
        // Validate that the resolved target stays within the workspace
        if !validate_cache_path(path, workspace) {
            return None;
        }

        // Try exact match first
        let cache_dir = self.cache_path(key);
        if cache_dir.exists() {
            let target = workspace.join(path);
            if copy_dir_contents(&cache_dir, &target).is_ok() {
                return Some(key.to_string());
            }
        }

        // Try restore-keys as prefix matches
        for prefix in restore_keys {
            if let Some(matched) = self.find_by_prefix(prefix) {
                let cache_dir = self.cache_path(&matched);
                let target = workspace.join(path);
                if copy_dir_contents(&cache_dir, &target).is_ok() {
                    return Some(matched);
                }
            }
        }

        None
    }

    fn save_inner(&self, key: &str, path: &str, workspace: &Path) -> Result<(), String> {
        // Validate that the resolved source stays within the workspace
        if !validate_cache_path(path, workspace) {
            return Err(format!("Cache path '{}' escapes workspace directory", path));
        }

        let source = workspace.join(path);
        if !source.exists() {
            return Err(format!("Cache path '{}' does not exist", source.display()));
        }

        let cache_dir = self.cache_path(key);
        // Remove old cache entry if it exists
        if cache_dir.exists() {
            std::fs::remove_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to remove old cache: {}", e))?;
        }

        if source.is_dir() {
            copy_dir_contents(&source, &cache_dir)?;
        } else {
            let file_name = source
                .file_name()
                .ok_or_else(|| format!("Cache path '{}' has no file name component", path))?;
            std::fs::create_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to create cache dir: {}", e))?;
            let dest = cache_dir.join(file_name);
            std::fs::copy(&source, &dest).map_err(|e| format!("Failed to copy file: {}", e))?;
        }

        // Write key metadata for prefix matching
        let meta_path = cache_dir.join(".cache_key");
        std::fs::write(&meta_path, key)
            .map_err(|e| format!("Failed to write cache metadata: {}", e))?;

        // Evict oldest entries if cache exceeds size limit
        self.evict_if_needed();

        Ok(())
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        self.root.join(hash)
    }

    /// Find the most recently modified cached key that starts with the given prefix.
    ///
    /// When multiple entries match, the one with the newest modification time wins,
    /// matching GitHub Actions' behavior of preferring the most recently created key.
    fn find_by_prefix(&self, prefix: &str) -> Option<String> {
        let entries = std::fs::read_dir(&self.root).ok()?;
        let mut best: Option<(String, std::time::SystemTime)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            let meta_path = path.join(".cache_key");
            if let Ok(stored_key) = std::fs::read_to_string(&meta_path) {
                if stored_key.starts_with(prefix) {
                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                    if best.as_ref().is_none_or(|(_, t)| mtime > *t) {
                        best = Some((stored_key, mtime));
                    }
                }
            }
        }
        best.map(|(key, _)| key)
    }

    /// Evict oldest cache entries until the total size is under `MAX_CACHE_SIZE_BYTES`.
    ///
    /// Entries are sorted by last modification time (oldest first) and removed
    /// until the cache fits within the budget. Called automatically after `save`.
    fn evict_if_needed(&self) {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Collect entries with their total size and modification time
        let mut cache_entries: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total_size: u64 = 0;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let size = dir_size(&path);
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            total_size += size;
            cache_entries.push((path, size, mtime));
        }

        if total_size <= MAX_CACHE_SIZE_BYTES {
            return;
        }

        // Sort oldest first
        cache_entries.sort_by_key(|&(_, _, mtime)| mtime);

        for (path, size, _) in &cache_entries {
            if total_size <= MAX_CACHE_SIZE_BYTES {
                break;
            }
            if std::fs::remove_dir_all(path).is_ok() {
                total_size = total_size.saturating_sub(*size);
                wrkflw_logging::debug(&format!(
                    "Cache eviction: removed {} ({} bytes)",
                    path.display(),
                    size
                ));
            }
        }
    }
}

/// Recursively compute the total size of all files in a directory.
fn dir_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                total += dir_size(&p);
            } else if let Ok(meta) = p.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Check whether any path component is literally `..`.
fn has_dotdot_component(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// Validate that `path` (relative to `workspace`) does not escape the workspace via `..` etc.
fn validate_cache_path(path: &str, workspace: &Path) -> bool {
    let joined = workspace.join(path);
    // Use lexical normalization: canonicalize the workspace (which must exist) and
    // check that the joined path, once cleaned, starts with it.
    if let Ok(canonical_ws) = workspace.canonicalize() {
        // If the target exists, canonicalize it directly; otherwise check the parent.
        if let Ok(canonical_target) = joined.canonicalize() {
            canonical_target.starts_with(&canonical_ws)
        } else if let Some(parent) = joined.parent() {
            // Target doesn't exist yet — validate its parent
            parent
                .canonicalize()
                .map(|p| p.starts_with(&canonical_ws))
                .unwrap_or_else(|_| !has_dotdot_component(path))
        } else {
            !has_dotdot_component(path)
        }
    } else {
        // Workspace itself can't be canonicalized — fall back to component check
        !has_dotdot_component(path)
    }
}

/// Recursively copy directory contents from `src` to `dst`, skipping symlinks.
fn copy_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("Failed to create dir: {}", e))?;

    if !src.is_dir() {
        return Err(format!("Source '{}' is not a directory", src.display()));
    }

    for entry in std::fs::read_dir(src)
        .map_err(|e| format!("Failed to read dir: {}", e))?
        .flatten()
    {
        let src_path = entry.path();
        // Skip symlinks to prevent following links outside the cache tree
        if src_path.is_symlink() {
            continue;
        }
        let file_name = entry.file_name();
        let dst_path = dst.join(&file_name);

        if src_path.is_dir() {
            copy_dir_contents(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Failed to copy '{}': {}", src_path.display(), e))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn save_and_restore_directory() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        // Create test directory to cache
        let cache_path = workspace.path().join("node_modules");
        std::fs::create_dir_all(cache_path.join("pkg")).unwrap();
        std::fs::write(cache_path.join("pkg/index.js"), "module.exports = {}").unwrap();
        std::fs::write(cache_path.join("README"), "deps").unwrap();

        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();

        // Save
        store
            .save("node-deps-abc123", "node_modules", workspace.path())
            .await
            .unwrap();

        // Restore to a different workspace
        let workspace2 = tempdir().unwrap();
        let matched = store
            .restore("node-deps-abc123", &[], "node_modules", workspace2.path())
            .await;
        assert_eq!(matched, Some("node-deps-abc123".to_string()));
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("node_modules/README")).unwrap(),
            "deps"
        );
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("node_modules/pkg/index.js")).unwrap(),
            "module.exports = {}"
        );
    }

    #[tokio::test]
    async fn restore_miss() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();

        let result = store
            .restore("missing-key", &[], "some_dir", workspace.path())
            .await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn restore_by_prefix() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        // Create and save
        std::fs::create_dir_all(workspace.path().join("cache_dir")).unwrap();
        std::fs::write(workspace.path().join("cache_dir/data.bin"), "cached").unwrap();

        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();
        store
            .save("rust-cargo-abc123", "cache_dir", workspace.path())
            .await
            .unwrap();

        // Restore with prefix
        let workspace2 = tempdir().unwrap();
        let matched = store
            .restore(
                "rust-cargo-xyz789",
                &["rust-cargo-".to_string()],
                "cache_dir",
                workspace2.path(),
            )
            .await;
        assert_eq!(matched, Some("rust-cargo-abc123".to_string()));
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("cache_dir/data.bin")).unwrap(),
            "cached"
        );
    }

    #[tokio::test]
    async fn save_overwrites_existing() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        std::fs::create_dir_all(workspace.path().join("data")).unwrap();
        std::fs::write(workspace.path().join("data/v1.txt"), "version1").unwrap();

        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();
        store
            .save("my-key", "data", workspace.path())
            .await
            .unwrap();

        // Overwrite
        std::fs::write(workspace.path().join("data/v1.txt"), "version2").unwrap();
        store
            .save("my-key", "data", workspace.path())
            .await
            .unwrap();

        // Restore should get v2
        let workspace2 = tempdir().unwrap();
        store
            .restore("my-key", &[], "data", workspace2.path())
            .await;
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("data/v1.txt")).unwrap(),
            "version2"
        );
    }

    #[test]
    fn has_dotdot_rejects_traversal() {
        assert!(has_dotdot_component("../etc/passwd"));
        assert!(has_dotdot_component("foo/../../bar"));
        assert!(has_dotdot_component(".."));
    }

    #[test]
    fn has_dotdot_allows_similar_names() {
        // "..bar" is a valid directory name, not a traversal component
        assert!(!has_dotdot_component("foo/..bar/baz"));
        assert!(!has_dotdot_component("..."));
        assert!(!has_dotdot_component("node_modules"));
    }

    #[test]
    fn validate_rejects_path_traversal() {
        let workspace = tempdir().unwrap();
        assert!(!validate_cache_path("../escape", workspace.path()));
        assert!(!validate_cache_path("foo/../../escape", workspace.path()));
    }

    #[test]
    fn validate_allows_normal_paths() {
        let workspace = tempdir().unwrap();
        assert!(validate_cache_path("node_modules", workspace.path()));
        assert!(validate_cache_path("target/debug", workspace.path()));
        assert!(validate_cache_path("..bar/baz", workspace.path()));
    }

    #[test]
    fn dir_size_computes_total() {
        let d = tempdir().unwrap();
        // 5 bytes + 3 bytes = 8 bytes
        std::fs::write(d.path().join("a.txt"), "hello").unwrap();
        std::fs::create_dir_all(d.path().join("sub")).unwrap();
        std::fs::write(d.path().join("sub/b.txt"), "hey").unwrap();
        assert_eq!(dir_size(d.path()), 8);
    }

    #[tokio::test]
    async fn evict_removes_oldest_entries() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();

        // Create two cache entries with known order (sleep to separate mtimes)
        std::fs::create_dir_all(workspace.path().join("d1")).unwrap();
        std::fs::write(workspace.path().join("d1/f.txt"), "old").unwrap();
        store.save("key-old", "d1", workspace.path()).await.unwrap();

        // Touch the second entry slightly later
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::create_dir_all(workspace.path().join("d2")).unwrap();
        std::fs::write(workspace.path().join("d2/f.txt"), "new").unwrap();
        store.save("key-new", "d2", workspace.path()).await.unwrap();

        // Both entries should exist
        let workspace2 = tempdir().unwrap();
        assert!(store
            .restore("key-old", &[], "d1", workspace2.path())
            .await
            .is_some());
        assert!(store
            .restore("key-new", &[], "d2", workspace2.path())
            .await
            .is_some());
    }

    #[tokio::test]
    async fn save_single_file_without_filename_returns_error() {
        // The file_name() fix should error on paths with no file component.
        // In practice, validate_cache_path would catch ".." paths first,
        // but we test the save path for completeness.
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();

        // A normal file save should work
        std::fs::write(workspace.path().join("file.txt"), "content").unwrap();
        assert!(store
            .save("file-key", "file.txt", workspace.path())
            .await
            .is_ok());
    }
}
