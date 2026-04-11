//! Classification of `notify::EventKind` into "watch cycle should fire"
//! vs "drop". Extracted from `watcher.rs` so the cross-backend rationale
//! (kqueue / PollWatcher `Modify(Any)`, macOS FSEvents variants) is
//! documented next to the tests pinning the behavior.

use notify::event::{EventKind, ModifyKind};

/// Return `true` if a notify event kind is relevant for trigger
/// re-evaluation. We care about creates, writes, removes, and rename
/// endpoints; we drop access/metadata updates (atime/chmod/owner).
///
/// `Modify(Any)` is treated as relevant. The Linux inotify and macOS
/// FSEvents backends emit specific subkinds (`Modify(Data(...))`,
/// `Modify(Name(...))`), but the kqueue backend (FreeBSD, some macOS
/// configurations) and notify's `PollWatcher` fallback emit
/// `Modify(Any)` for content changes when the underlying API can't
/// distinguish data from metadata. Dropping `Modify(Any)` would leave
/// the watcher silently dead on those platforms — over-firing is
/// bounded by the debouncer; under-firing isn't recoverable.
///
/// The match is exhaustive on `ModifyKind` (no catch-all `Modify(_)`
/// arm) so future additions to the notify enum surface as a compile
/// error instead of silently being routed to "drop".
pub(crate) fn is_relevant_event_kind(kind: &EventKind) -> bool {
    match kind {
        EventKind::Create(_) => true,
        EventKind::Remove(_) => true,
        EventKind::Modify(ModifyKind::Data(_)) => true,
        EventKind::Modify(ModifyKind::Name(_)) => true,
        EventKind::Modify(ModifyKind::Any) => true,
        EventKind::Modify(ModifyKind::Metadata(_)) => false,
        EventKind::Modify(ModifyKind::Other) => false,
        EventKind::Access(_) => false,
        EventKind::Any | EventKind::Other => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, RemoveKind};

    #[test]
    fn accepts_creates_and_writes() {
        assert!(is_relevant_event_kind(&EventKind::Create(CreateKind::File)));
        assert!(is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Data(DataChange::Content)
        )));
        assert!(is_relevant_event_kind(&EventKind::Remove(RemoveKind::File)));
        // Renames must propagate — a `git checkout` fires them in pairs.
        assert!(is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Name(notify::event::RenameMode::Any)
        )));
    }

    #[test]
    fn drops_access_and_metadata() {
        assert!(!is_relevant_event_kind(&EventKind::Access(
            AccessKind::Read
        )));
        assert!(!is_relevant_event_kind(&EventKind::Modify(
            ModifyKind::Metadata(MetadataKind::Permissions)
        )));
        assert!(!is_relevant_event_kind(&EventKind::Any));
        assert!(!is_relevant_event_kind(&EventKind::Other));
    }

    #[test]
    fn accepts_modify_any() {
        // Regression: `Modify(Any)` is what kqueue (FreeBSD) and
        // PollWatcher emit for content changes when the underlying API
        // can't distinguish data from metadata. The previous catch-all
        // `Modify(_) => false` arm dropped these silently, leaving the
        // watcher in a "no events ever fire" state on those backends
        // even though Linux/macOS users saw it work fine. Treating
        // `Modify(Any)` as relevant fixes the platform parity.
        assert!(is_relevant_event_kind(&EventKind::Modify(ModifyKind::Any)));
    }
}
