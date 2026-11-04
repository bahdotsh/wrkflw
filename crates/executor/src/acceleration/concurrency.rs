//! Job-level `concurrency:` groups, enforced within one run.

use super::current;
use std::sync::Arc;
use tokio::sync::Semaphore;
use wrkflw_parser::workflow::Job;

/// A job's `concurrency:`: a group name, or a mapping with `group` and
/// `cancel-in-progress`.
struct Concurrency {
    group: String,
    /// Asked for, though nothing here can honour it.
    cancel_in_progress: bool,
}

fn concurrency_group(job: &Job) -> Option<Concurrency> {
    match job.concurrency.as_ref()? {
        serde_yaml::Value::String(group) => Some(Concurrency {
            group: group.clone(),
            cancel_in_progress: false,
        }),
        serde_yaml::Value::Mapping(map) => Some(Concurrency {
            group: map.get("group")?.as_str()?.to_string(),
            cancel_in_progress: map
                .get("cancel-in-progress")
                .is_some_and(|v| v.as_bool() != Some(false)),
        }),
        _ => None,
    }
}

/// Wait until no other job in this run holds `job`'s concurrency group.
///
/// Groups compare as written, so expressions aren't evaluated. Equal text
/// serializes, which is the safe direction. `cancel-in-progress` doesn't
/// apply: a local run has no earlier run of the same group to cancel.
pub(crate) async fn acquire_concurrency(
    job_name: &str,
    job: &Job,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    let Concurrency {
        group,
        cancel_in_progress,
    } = concurrency_group(job)?;
    let run = current()?;
    if cancel_in_progress {
        wrkflw_logging::warning(&format!(
            "Job '{}': cancel-in-progress has no effect locally, so it waits its turn in '{}'",
            job_name, group
        ));
    }
    let semaphore = {
        let mut groups = run.groups.lock().ok()?;
        groups
            .entry(group)
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone()
    };
    semaphore.acquire_owned().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acceleration::{with_run_state, Acceleration, RunState};
    use std::path::PathBuf;

    fn job(yaml: &str) -> Job {
        serde_yaml::from_str(yaml).unwrap()
    }

    /// The most jobs inside their critical section at once.
    async fn peak_overlap(first: &str, second: &str) -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let definitions = [job(first), job(second)];
        let state = RunState::new(Acceleration::default(), false, PathBuf::from(".")).unwrap();
        with_run_state(Arc::new(state), async {
            futures::future::join_all(definitions.iter().map(|definition| async {
                let _permit = acquire_concurrency("j", definition).await;
                let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                active.fetch_sub(1, Ordering::SeqCst);
            }))
            .await;
        })
        .await;
        peak.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_group_serializes_its_jobs_and_other_jobs_overlap() {
        let lock = "concurrency: lock\nsteps: []\n";
        let other = "concurrency: other\nsteps: []\n";
        let free = "steps: []\n";
        assert_eq!(peak_overlap(lock, lock).await, 1);
        assert_eq!(peak_overlap(lock, other).await, 2);
        assert_eq!(peak_overlap(free, free).await, 2);
    }

    #[test]
    fn concurrency_groups_come_from_a_string_or_a_mapping() {
        let group =
            |yaml: &str| concurrency_group(&job(yaml)).map(|c| (c.group, c.cancel_in_progress));
        assert_eq!(
            group("concurrency: db\nsteps: []\n"),
            Some(("db".to_string(), false))
        );
        assert_eq!(
            group("concurrency:\n  group: db\n  cancel-in-progress: true\nsteps: []\n"),
            Some(("db".to_string(), true))
        );
        assert_eq!(group("steps: []\n"), None);
    }
}
