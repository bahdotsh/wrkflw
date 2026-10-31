use std::process::{Command, Stdio};

/// Force-removes a container when dropped while still armed.
///
/// A timed-out step drops the future driving its container, which would
/// otherwise leave the container running after wrkflw has moved on or exited.
/// Dropping can't await, so this goes through the CLI, and the removal carries
/// on by itself if wrkflw exits straight afterwards.
pub(crate) struct ContainerGuard {
    cli: &'static str,
    id: String,
    armed: bool,
}

impl ContainerGuard {
    /// `armed` is false when failed containers are being kept for debugging.
    pub(crate) fn new(cli: &'static str, id: &str, armed: bool) -> Self {
        Self {
            cli,
            id: id.to_string(),
            armed,
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
        wrkflw_logging::debug(&format!("Removing abandoned container {}", self.id));
        let spawned = Command::new(self.cli)
            .args(["rm", "-f", &self.id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Err(e) = spawned {
            wrkflw_logging::warning(&format!("Failed to remove container {}: {}", self.id, e));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

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
            if let Ok(calls) = fs::read_to_string(path) {
                return Some(calls);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }

    #[test]
    fn an_armed_guard_removes_the_container_when_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path());
        drop(ContainerGuard::new(cli, "abc", true));
        assert_eq!(wait_for(&log).as_deref(), Some("rm -f abc\n"));
    }

    #[test]
    fn a_disarmed_guard_leaves_the_container_alone() {
        let dir = tempfile::tempdir().unwrap();
        let (cli, log) = fake_cli(dir.path());
        let mut guard = ContainerGuard::new(cli, "abc", true);
        guard.disarm();
        drop(guard);
        drop(ContainerGuard::new(cli, "kept", false));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(!log.exists());
    }
}
