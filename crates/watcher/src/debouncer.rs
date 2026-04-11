use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// Default upper bound on pending paths collected between drains.
///
/// Without a cap, a filesystem churn burst (e.g. `cargo clean && cargo
/// build`, a `git checkout` of a large branch) can grow the HashSet to
/// tens of thousands of paths before the consumer runs. Each path is a
/// `PathBuf` allocation — unbounded growth is a silent memory hazard in
/// a long-running watch session. 8192 is enough to absorb a single
/// build's worth of events with plenty of headroom, and small enough
/// that the worst-case memory footprint is bounded at roughly a few
/// megabytes.
///
/// When the cap is hit, new events are dropped and an atomic counter
/// is incremented. The counter is surfaced to the consumer on every
/// drain so the reporter can tell the user "N events dropped this
/// cycle" — the classic silent-failure mode this PR has been
/// iteratively patching.
pub const DEFAULT_MAX_PENDING_EVENTS: usize = 8192;

/// Collects filesystem events over a configurable window,
/// deduplicates paths, and fires a single coalesced event.
pub struct Debouncer {
    duration: Duration,
    max_pending: usize,
    pending: Arc<Mutex<HashSet<PathBuf>>>,
    /// Monotonic counter of events dropped because `pending` hit
    /// `max_pending`. Never reset — consumers compute deltas on each
    /// drain, and the counter is only ever read/snapshotted from the
    /// watcher loop so relaxed ordering is fine.
    dropped: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Debouncer {
    pub fn new(duration: Duration) -> Self {
        Self::with_capacity(duration, DEFAULT_MAX_PENDING_EVENTS)
    }

    pub fn with_capacity(duration: Duration, max_pending: usize) -> Self {
        // A zero cap would make `add_event` a no-op; clamp to at
        // least 1 so the debouncer is always willing to accept one
        // event before dropping the rest.
        let max_pending = max_pending.max(1);
        Self {
            duration,
            max_pending,
            pending: Arc::new(Mutex::new(HashSet::new())),
            dropped: Arc::new(AtomicUsize::new(0)),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Return a clone of the Notify handle so the watcher loop can await it.
    pub fn notifier(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    /// Snapshot the cumulative dropped-event count. The watcher
    /// reports deltas by diffing successive snapshots so the cycle's
    /// summary reflects only this cycle's drops.
    pub fn dropped_count(&self) -> usize {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Add a path from a filesystem event.
    pub fn add_event(&self, path: PathBuf) {
        let mut pending = self.lock_or_recover();
        // Bound the set. The `contains` check avoids the common case
        // where a duplicate path would have been coalesced anyway —
        // dropping a duplicate wouldn't lose information but would
        // still bump the counter and confuse the user.
        if pending.len() >= self.max_pending && !pending.contains(&path) {
            drop(pending);
            self.dropped.fetch_add(1, Ordering::Relaxed);
            // Still notify: the consumer may not have been running,
            // and we want it to drain the (full) set sooner rather
            // than later. Missing this notify would let the pending
            // set stay saturated until the next fs event with a
            // non-duplicate path, compounding the drop rate.
            self.notify.notify_one();
            return;
        }
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
        //
        // Tuning rationale: with the default 500ms debounce, the previous
        // value of 10 rounds meant a sustained `cargo build` could delay
        // a drain by 5s — long enough for the user to wonder if the
        // watcher is hung. 3 rounds caps the worst case at ~1.5s while
        // still absorbing the brief flurries that follow most editor
        // saves (write → fsync → editor swap-rename).
        const MAX_SETTLE_ROUNDS: usize = 3;

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

    #[tokio::test]
    async fn max_pending_drops_excess_events_and_counts_them() {
        // Regression: without a cap the debouncer would grow
        // unbounded under a churn burst (e.g. `cargo build`, a big
        // `git checkout`). We must (a) refuse to grow past the cap,
        // (b) count the drops so the reporter can surface them, and
        // (c) still notify the consumer so a saturated queue drains
        // rather than parking.
        let debouncer = Debouncer::with_capacity(Duration::from_millis(10), 4);
        for i in 0..10 {
            debouncer.add_event(PathBuf::from(format!("f{}.rs", i)));
        }
        let drained = debouncer.drain().await;
        assert_eq!(
            drained.len(),
            4,
            "pending set must not exceed the configured cap"
        );
        assert_eq!(
            debouncer.dropped_count(),
            6,
            "every event past the cap must bump the dropped counter"
        );
    }

    #[tokio::test]
    async fn duplicate_events_at_cap_are_not_counted_as_drops() {
        // A duplicate would have been coalesced by the HashSet
        // anyway, so rejecting it when the set is full is not a
        // meaningful drop — the counter should stay at zero. Mixing
        // duplicates and genuine new paths in one burst must still
        // count only the genuine new paths against the cap.
        let debouncer = Debouncer::with_capacity(Duration::from_millis(10), 2);
        debouncer.add_event(PathBuf::from("a.rs"));
        debouncer.add_event(PathBuf::from("b.rs"));
        debouncer.add_event(PathBuf::from("a.rs")); // dup — coalesced, not dropped
        debouncer.add_event(PathBuf::from("c.rs")); // genuine overflow
        assert_eq!(debouncer.dropped_count(), 1);
        let drained = debouncer.drain().await;
        assert_eq!(drained.len(), 2);
    }

    #[tokio::test]
    async fn max_settle_rounds_prevents_livelock() {
        // Use a very short debounce window so the test completes quickly
        let debouncer = Arc::new(Debouncer::new(Duration::from_millis(5)));
        let debouncer_clone = debouncer.clone();

        // Continuously add events during drain to simulate sustained churn
        let feeder = tokio::spawn(async move {
            for i in 0..50 {
                debouncer_clone.add_event(PathBuf::from(format!("file_{}.rs", i)));
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        });

        // Seed at least one event so drain doesn't return empty immediately
        debouncer.add_event(PathBuf::from("seed.rs"));

        // drain() must return within a bounded time despite continuous events
        let result = tokio::time::timeout(Duration::from_secs(2), debouncer.drain()).await;
        assert!(result.is_ok(), "drain() should not livelock");
        let paths = result.unwrap();
        assert!(!paths.is_empty());

        feeder.abort();
    }
}
