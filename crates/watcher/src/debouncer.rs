use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Collects filesystem events over a configurable window,
/// deduplicates paths, and fires a single coalesced event.
pub struct Debouncer {
    duration: Duration,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
}

impl Debouncer {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            pending: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Add a path from a filesystem event.
    pub fn add_event(&self, path: PathBuf) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(path);
        }
    }

    /// Wait for the debounce duration, then drain all pending paths.
    /// Returns empty vec if nothing was collected.
    pub async fn drain(&self) -> Vec<PathBuf> {
        tokio::time::sleep(self.duration).await;
        let mut pending = self.pending.lock().unwrap();
        let paths: Vec<PathBuf> = pending.drain().collect();
        paths
    }

    /// Check if there are any pending events without draining.
    pub fn has_pending(&self) -> bool {
        self.pending.lock().map(|p| !p.is_empty()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn debouncer_collects_and_deduplicates() {
        let debouncer = Debouncer::new(Duration::from_millis(10));
        debouncer.add_event(PathBuf::from("src/main.rs"));
        debouncer.add_event(PathBuf::from("src/lib.rs"));
        debouncer.add_event(PathBuf::from("src/main.rs")); // duplicate

        let paths = debouncer.drain().await;
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("src/main.rs")));
        assert!(paths.contains(&PathBuf::from("src/lib.rs")));
    }

    #[tokio::test]
    async fn debouncer_drains_empty_after_collect() {
        let debouncer = Debouncer::new(Duration::from_millis(10));
        debouncer.add_event(PathBuf::from("foo.rs"));

        let paths = debouncer.drain().await;
        assert_eq!(paths.len(), 1);

        let paths2 = debouncer.drain().await;
        assert!(paths2.is_empty());
    }
}
