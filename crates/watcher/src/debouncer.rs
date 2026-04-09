use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// Collects filesystem events over a configurable window,
/// deduplicates paths, and fires a single coalesced event.
pub struct Debouncer {
    duration: Duration,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
    notify: Arc<Notify>,
}

impl Debouncer {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            pending: Arc::new(Mutex::new(HashSet::new())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Return a clone of the Notify handle so the watcher loop can await it.
    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    /// Add a path from a filesystem event.
    pub fn add_event(&self, path: PathBuf) {
        let mut pending = self.lock_or_recover();
        pending.insert(path);
        drop(pending);
        self.notify.notify_one();
    }

    /// Wait for the debounce window to settle, then drain all pending paths.
    ///
    /// Sleeps for the debounce duration, but only continues waiting while new
    /// events keep arriving. Returns as soon as a full debounce interval passes
    /// with no new events. A maximum of `MAX_SETTLE_ROUNDS` iterations prevents
    /// livelock under sustained filesystem churn (e.g. large builds).
    pub async fn drain(&self) -> Vec<PathBuf> {
        // Cap the number of settle rounds to prevent livelock when events
        // arrive faster than the debounce window.
        const MAX_SETTLE_ROUNDS: usize = 10;

        let mut rounds = 0;
        loop {
            let count_before = {
                let pending = self.lock_or_recover();
                pending.len()
            };

            if count_before == 0 {
                return Vec::new();
            }

            tokio::time::sleep(self.duration).await;
            rounds += 1;

            let mut pending = self.lock_or_recover();
            // Drain if no new events arrived during the sleep, or if we've
            // waited long enough to avoid starving the consumer.
            if pending.len() == count_before || rounds >= MAX_SETTLE_ROUNDS {
                return pending.drain().collect();
            }
        }
    }

    /// Check if there are any pending events without draining.
    pub fn has_pending(&self) -> bool {
        let pending = self.lock_or_recover();
        !pending.is_empty()
    }

    /// Lock the mutex, recovering from poison if necessary.
    fn lock_or_recover(&self) -> std::sync::MutexGuard<'_, HashSet<PathBuf>> {
        match self.pending.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
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

    #[tokio::test]
    async fn add_event_sends_notification() {
        let debouncer = Arc::new(Debouncer::new(Duration::from_millis(10)));
        let notifier = debouncer.notifier();

        // Spawn a task that waits for notification
        let handle = tokio::spawn(async move {
            notifier.notified().await;
            true
        });

        debouncer.add_event(PathBuf::from("test.rs"));
        let got_notified = handle.await.unwrap();
        assert!(got_notified);
    }
}
