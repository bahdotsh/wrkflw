use std::process::{Command, Stdio};

/// Cleans up a container when dropped while still armed.
///
/// A timed-out step drops the future driving its container, which would
/// otherwise leave the container running after wrkflw has moved on or exited.
/// Dropping can't await, so this goes through the CLI, and the cleanup carries
/// on by itself if wrkflw exits straight afterwards. Docker therefore needs the
/// `docker` CLI on `PATH`, even though it otherwise talks to the daemon through
/// bollard. Kubernetes has the equivalent `PodGuard`.
pub(crate) struct ContainerGuard {
    cli: &'static str,
    id: String,
    armed: bool,
    /// Kill the container but keep it for debugging, rather than removing it.
    preserve: bool,
    /// Drops the container from its runtime's exit sweep.
    untrack: fn(&str),
}

impl ContainerGuard {
    pub(crate) fn new(cli: &'static str, id: &str, preserve: bool, untrack: fn(&str)) -> Self {
        Self {
            cli,
            id: id.to_string(),
            armed: true,
            preserve,
            untrack,
        }
    }

    /// The run finished normally, so its usual cleanup applies.
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        (self.untrack)(&self.id);
        let args: &[&str] = if self.preserve {
            wrkflw_logging::info(&format!(
                "Step cancelled, killing container {} and keeping it for debugging. \
                 Use '{} logs {}' to inspect it.",
                self.id, self.cli, self.id
            ));
            &["kill"]
        } else {
            wrkflw_logging::debug(&format!("Removing abandoned container {}", self.id));
            &["rm", "-f"]
        };
        let spawned = Command::new(self.cli)
            .args(args)
            .arg(&self.id)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            // Only reaps it. If wrkflw exits first, the cleanup still finishes.
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => {
                wrkflw_logging::warning(&format!("Failed to clean up container {}: {}", self.id, e))
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    static UNTRACKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn record_untrack(id: &str) {
        UNTRACKED.lock().unwrap().push(id.to_string());
    }

    fn was_untracked(id: &str) -> bool {
        UNTRACKED.lock().unwrap().iter().any(|u| u == id)
    }

    /// A fake CLI that records its arguments, so nothing real is removed.
    fn fake_cli(dir: &std::path::Path) -> (&'static str, std::path::PathBuf) {
        let log = dir.join("calls");
        let cli = dir.join("fake-cli");
        fs::write(
            &cli,
            format!("#!/bin/sh\necho \"$@\" >> {}\n", log.display()),
        )
        .unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o755)).unwrap();
        (Box::leak(cli.display().to_string().into_boxed_str()), log)
    }

    fn wait_for(path: &std::path::Path) -> Option<String> {
        for _ in 0..100 {
            // The shell creates the file before it writes to it.
            match fs::read_to_string(path) {
                Ok(calls) if !calls.is_empty() => return Some(calls),
                _ => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn an_armed_guard_removes_the_container_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path());
        drop(ContainerGuard::new(cli, "removed", false, record_untrack));
        assert_eq!(wait_for(&log).as_deref(), Some("rm -f removed\n"));
        assert!(was_untracked("removed"));
    }

    #[test]
    fn a_preserving_guard_kills_the_container_without_removing_it() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path());
        drop(ContainerGuard::new(cli, "preserved", true, record_untrack));
        assert_eq!(wait_for(&log).as_deref(), Some("kill preserved\n"));
        assert!(was_untracked("preserved"));
    }

    #[test]
    fn a_disarmed_guard_leaves_the_container_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path());
        let mut guard = ContainerGuard::new(cli, "disarmed", false, record_untrack);
        guard.disarm();
        drop(guard);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!log.exists());
        assert!(!was_untracked("disarmed"));
    }

    #[test]
    fn a_missing_cli_still_untracks_the_container() {
        let missing = "/nonexistent/wrkflw-fake-cli";
        drop(ContainerGuard::new(
            missing,
            "no-cli",
            false,
            record_untrack,
        ));
        assert!(was_untracked("no-cli"));
    }
}
