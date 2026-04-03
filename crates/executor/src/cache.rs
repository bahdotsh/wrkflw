//! Local cache storage for GitHub Actions `actions/cache` emulation.
//!
//! Caches are stored under `~/.wrkflw/cache/` (persistent across runs)
//! and keyed by a SHA-256 hash of the cache key string.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Manages a persistent local cache for workflow runs.
pub struct CacheStore {
    root: PathBuf,
}

impl CacheStore {
    /// Create a new cache store. Uses `~/.wrkflw/cache/` by default.
    pub fn new() -> Option<Self> {
        let root = dirs::home_dir()?.join(".wrkflw").join("cache");
        std::fs::create_dir_all(&root).ok()?;
        Some(Self { root })
    }

    /// Create a cache store at a custom root (for testing).
    pub fn with_root(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Attempt to restore a cache. Tries `key` first, then each of `restore_keys`
    /// as a prefix match.
    ///
    /// `path` is the directory to restore into (relative to `workspace`).
    /// Returns the matched key on hit, or `None` on miss.
    pub fn restore(
        &self,
        key: &str,
        restore_keys: &[String],
        path: &str,
        workspace: &Path,
    ) -> Option<String> {
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

    /// Save the contents of `path` (relative to `workspace`) under `key`.
    pub fn save(&self, key: &str, path: &str, workspace: &Path) -> Result<(), String> {
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
            std::fs::create_dir_all(&cache_dir)
                .map_err(|e| format!("Failed to create cache dir: {}", e))?;
            let dest = cache_dir.join(source.file_name().unwrap_or_default());
            std::fs::copy(&source, &dest).map_err(|e| format!("Failed to copy file: {}", e))?;
        }

        // Write key metadata for prefix matching
        let meta_path = cache_dir.join(".cache_key");
        std::fs::write(&meta_path, key)
            .map_err(|e| format!("Failed to write cache metadata: {}", e))?;

        Ok(())
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        let hash = format!("{:x}", Sha256::digest(key.as_bytes()));
        self.root.join(hash)
    }

    /// Find a cached key that starts with the given prefix.
    fn find_by_prefix(&self, prefix: &str) -> Option<String> {
        let entries = std::fs::read_dir(&self.root).ok()?;
        for entry in entries.flatten() {
            let meta_path = entry.path().join(".cache_key");
            if let Ok(stored_key) = std::fs::read_to_string(&meta_path) {
                if stored_key.starts_with(prefix) {
                    return Some(stored_key);
                }
            }
        }
        None
    }
}

/// Recursively copy directory contents from `src` to `dst`.
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

    #[test]
    fn save_and_restore_directory() {
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
            .unwrap();

        // Restore to a different workspace
        let workspace2 = tempdir().unwrap();
        let matched = store.restore("node-deps-abc123", &[], "node_modules", workspace2.path());
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

    #[test]
    fn restore_miss() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();
        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();

        let result = store.restore("missing-key", &[], "some_dir", workspace.path());
        assert!(result.is_none());
    }

    #[test]
    fn restore_by_prefix() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        // Create and save
        std::fs::create_dir_all(workspace.path().join("cache_dir")).unwrap();
        std::fs::write(workspace.path().join("cache_dir/data.bin"), "cached").unwrap();

        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();
        store
            .save("rust-cargo-abc123", "cache_dir", workspace.path())
            .unwrap();

        // Restore with prefix
        let workspace2 = tempdir().unwrap();
        let matched = store.restore(
            "rust-cargo-xyz789",
            &["rust-cargo-".to_string()],
            "cache_dir",
            workspace2.path(),
        );
        assert_eq!(matched, Some("rust-cargo-abc123".to_string()));
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("cache_dir/data.bin")).unwrap(),
            "cached"
        );
    }

    #[test]
    fn save_overwrites_existing() {
        let cache_root = tempdir().unwrap();
        let workspace = tempdir().unwrap();

        std::fs::create_dir_all(workspace.path().join("data")).unwrap();
        std::fs::write(workspace.path().join("data/v1.txt"), "version1").unwrap();

        let store = CacheStore::with_root(cache_root.path().to_path_buf()).unwrap();
        store.save("my-key", "data", workspace.path()).unwrap();

        // Overwrite
        std::fs::write(workspace.path().join("data/v1.txt"), "version2").unwrap();
        store.save("my-key", "data", workspace.path()).unwrap();

        // Restore should get v2
        let workspace2 = tempdir().unwrap();
        store.restore("my-key", &[], "data", workspace2.path());
        assert_eq!(
            std::fs::read_to_string(workspace2.path().join("data/v1.txt")).unwrap(),
            "version2"
        );
    }
}
