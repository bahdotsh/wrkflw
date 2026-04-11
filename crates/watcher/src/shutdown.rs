//! Lightweight cancellation handle for [`crate::WorkflowWatcher::run`].
//!
//! This is intentionally **not** `tokio_util::sync::CancellationToken` —
//! pulling in `tokio-util` just for this one primitive inflates the
//! crate graph, and our needs are narrow: a cloneable handle that can
//! be `.trigger()`-ed once and awaited from any number of consumers.
//!
//! Backed by `tokio::sync::watch::channel::<bool>`. The watch
//! primitive is the correct fit because it stores the latest value
//! and subscribers can see it retroactively — subscribing *after*
//! `.trigger()` was called still yields a value of `true` on the
//! very next `wait_for` poll, so there is no "trigger before
//! subscribe" race to engineer around. An earlier `tokio::sync::Notify`
//! based implementation had exactly that race: `notify_waiters()`
//! only wakes currently-parked tasks, and a `wait()` that checked
//! `is_triggered()` and then parked in between a caller's
//! `trigger()` would miss the wake and park forever. That failure
//! mode is pinned by the
//! `wait_after_trigger_sees_triggered_state_immediately` test below
//! so a future refactor can't silently reintroduce it.
//!
//! If a caller needs the classic "never fires" handle (e.g. the CLI,
//! which relies on `process::exit` for termination),
//! [`ShutdownSignal::never`] returns a valid signal that is never
//! triggered. This keeps `run()`'s signature uniform instead of an
//! `Option<ShutdownSignal>` parameter that every call site would have
//! to branch on.

use std::sync::Arc;
use tokio::sync::watch;

/// Cooperative shutdown signal for the watcher loop.
///
/// Cloning a [`ShutdownSignal`] produces another handle over the same
/// underlying watch channel; triggering any handle fires all of them.
/// Once triggered, the signal stays triggered — there is no "reset".
#[derive(Clone, Debug)]
pub struct ShutdownSignal {
    tx: Arc<watch::Sender<bool>>,
}

impl ShutdownSignal {
    /// Build a fresh, un-triggered signal.
    pub fn new() -> Self {
        let (tx, _initial_rx) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    /// Build a signal that will **never** fire. Used by call sites that
    /// rely on `process::exit` for termination (the `wrkflw watch` CLI)
    /// and don't want to thread a real cancellation path through —
    /// keeps `run()` uniform across hosts instead of forcing an
    /// `Option<ShutdownSignal>` parameter. Memory-wise this is
    /// indistinguishable from a real signal that never has `.trigger()`
    /// called on it; the separate constructor exists so grep hits at
    /// call sites document the intent.
    pub fn never() -> Self {
        Self::new()
    }

    /// Request shutdown. Subsequent calls are no-ops.
    ///
    /// `send_replace` stores the new value unconditionally — even if
    /// there are no active receivers, the latest value is retained
    /// for any future `subscribe()` + `wait_for` pair. This is the
    /// property that closes the "trigger before wait" race: a caller
    /// that constructs a signal, immediately triggers it, then hands
    /// the handle to a task that eventually calls `.wait()` sees the
    /// `true` value on the very first poll.
    pub fn trigger(&self) {
        self.tx.send_replace(true);
    }

    /// Has the signal been triggered?
    ///
    /// Use for fast-path checks at the top of a loop iteration; the
    /// `.wait()` future is the right thing to `select!` on for the
    /// slow path.
    pub fn is_triggered(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolve when the signal fires.
    ///
    /// Returns immediately if the watch channel already holds `true`.
    /// Otherwise parks until the value transitions to `true`. Safe to
    /// call from multiple tasks concurrently — each call creates its
    /// own subscriber so waiters don't interfere with each other.
    pub async fn wait(&self) {
        let mut rx = self.tx.subscribe();
        // `wait_for` returns `Err` only if the sender is dropped.
        // Our `Arc<Sender>` keeps the sender alive for the lifetime
        // of every outstanding `ShutdownSignal` clone, so the only
        // way to reach the error arm is the singular corner case of
        // the last signal handle being dropped while a `.wait()`
        // future is in flight — which means the caller is tearing
        // down the world anyway, and the "wait" is effectively
        // satisfied by the fact that there is no one left to signal.
        let _ = rx.wait_for(|v| *v).await;
    }
}

impl Default for ShutdownSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn wait_returns_immediately_when_already_triggered() {
        let sig = ShutdownSignal::new();
        sig.trigger();
        // Should complete within microseconds, certainly under the
        // 500ms guard window.
        tokio::time::timeout(Duration::from_millis(500), sig.wait())
            .await
            .expect("wait() must return immediately when already triggered");
    }

    #[tokio::test]
    async fn wait_wakes_when_triggered_from_another_task() {
        let sig = ShutdownSignal::new();
        let sig_clone = sig.clone();
        let waiter = tokio::spawn(async move { sig_clone.wait().await });

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!waiter.is_finished(), "wait() must park until triggered");
        sig.trigger();

        tokio::time::timeout(Duration::from_millis(500), waiter)
            .await
            .expect("waiter must wake within 500ms of trigger")
            .expect("waiter task must not panic");
    }

    #[tokio::test]
    async fn multiple_waiters_all_wake_on_single_trigger() {
        // A future embedder with a supervisor + reporter both observing
        // cancellation must see it fire for every waiter. The watch
        // channel backs this naturally because each `subscribe()`
        // creates an independent receiver — `send_replace(true)`
        // reaches all of them.
        let sig = ShutdownSignal::new();
        let w1 = tokio::spawn({
            let s = sig.clone();
            async move { s.wait().await }
        });
        let w2 = tokio::spawn({
            let s = sig.clone();
            async move { s.wait().await }
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        sig.trigger();
        tokio::time::timeout(Duration::from_millis(500), async {
            let _ = tokio::join!(w1, w2);
        })
        .await
        .expect("both waiters must wake within 500ms");
    }

    #[tokio::test]
    async fn wait_after_trigger_sees_triggered_state_immediately() {
        // CRITICAL regression: the previous `tokio::sync::Notify`-backed
        // implementation had a trigger-before-wait race. `notify_waiters`
        // only wakes currently-parked tasks, so a caller that:
        //   1. constructed a ShutdownSignal
        //   2. called `.trigger()`
        //   3. spawned a task that called `.wait()`
        // would park forever at step 3 because the wake from step 2
        // landed before any waker was registered. The watch-channel
        // backing stores the latest value, so `wait_for(|v| *v)` on
        // a fresh subscription after trigger returns immediately.
        //
        // This test exercises the exact real-world flow that hit the
        // bug: a cross-thread clone (mirroring how the watcher runs
        // its loop on a dedicated OS thread with its own runtime)
        // observes a trigger that fired on the main task.
        let sig = ShutdownSignal::new();
        sig.trigger();

        // A waiter that subscribes AFTER the trigger must still see
        // the fired state. Test from the same runtime first to pin
        // the baseline.
        let sig_clone = sig.clone();
        tokio::time::timeout(
            Duration::from_millis(500),
            tokio::spawn(async move { sig_clone.wait().await }),
        )
        .await
        .expect("post-trigger wait must resolve within 500ms")
        .expect("waiter task must not panic");

        // And from a separate-runtime thread, matching the watcher's
        // actual execution model. Loud failure here = the watcher's
        // end-to-end shutdown test would flake; tighten the timeout
        // to catch it fast.
        let sig_for_thread = sig.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("rt");
            rt.block_on(sig_for_thread.wait());
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_millis(500))
            .expect("cross-runtime post-trigger wait must resolve within 500ms");
    }

    #[tokio::test]
    async fn is_triggered_reflects_current_state() {
        let sig = ShutdownSignal::new();
        assert!(!sig.is_triggered());
        sig.trigger();
        assert!(sig.is_triggered());
        // Re-triggering is a no-op, not an error.
        sig.trigger();
        assert!(sig.is_triggered());
    }

    #[tokio::test]
    async fn never_signal_does_not_fire_on_its_own() {
        let sig = ShutdownSignal::never();
        assert!(!sig.is_triggered());
        let result = tokio::time::timeout(Duration::from_millis(50), sig.wait()).await;
        assert!(
            result.is_err(),
            "a `never()` signal that isn't explicitly triggered must not wake"
        );
    }
}
