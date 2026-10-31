//! Streaming mounts between the host and a pod as tar archives.
//!
//! The way back deserves the care it gets here: it deletes host files based
//! on what a workflow left in a pod.

use super::manifest::Mount;
use super::{SYNC_CONTAINER, WAIT_CONTAINER};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use wrkflw_runtime::container::ContainerError;

/// How much of a sync's stderr is kept for its error message.
const MAX_STDERR_BYTES: usize = 64 * 1024;

/// Read `child`'s stderr to the end on its own thread, keeping the first
/// [`MAX_STDERR_BYTES`].
///
/// Whoever is busy with the child's stdin or stdout cannot also read stderr.
/// A child that fills that pipe would block with us blocked on it in turn,
/// until `--k8s-timeout` ended the step.
pub(super) fn drain_stderr(child: &mut std::process::Child) -> std::thread::JoinHandle<String> {
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let Some(mut stderr) = stderr else {
            return String::new();
        };
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            let room = MAX_STDERR_BYTES - kept.len();
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        String::from_utf8_lossy(&kept).trim().to_string()
    })
}

/// Blocking: tar `mount.host` and pipe it into `tar -x` in the wait container.
pub(super) fn copy_dir_into_pod(
    kubectl: &OsStr,
    kubectl_args: &[String],
    pod: &str,
    mount: &Mount,
) -> Result<(), ContainerError> {
    let dest = mount.container.to_string_lossy().to_string();
    let mut command = std::process::Command::new(kubectl);
    command.args(kubectl_args).args([
        "exec",
        "-i",
        pod,
        "-c",
        WAIT_CONTAINER,
        "--",
        "tar",
        "-C",
        &dest,
        "-xof",
        "-",
    ]);
    stream_dir_into(command, mount)?;
    wrkflw_logging::debug(&format!(
        "Kubernetes: synced {} -> {}:{}",
        mount.host.display(),
        pod,
        dest
    ));
    Ok(())
}

/// Blocking: write `mount.host` as a tar archive to `command`'s stdin.
pub(super) fn stream_dir_into(
    mut command: std::process::Command,
    mount: &Mount,
) -> Result<(), ContainerError> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to spawn kubectl exec: {}", e))
        })?;
    let stderr = drain_stderr(&mut child);

    let stdin = child.stdin.take().ok_or_else(|| {
        ContainerError::ContainerExecution("kubectl exec stdin was not captured".to_string())
    })?;
    let mut builder = tar::Builder::new(stdin);
    // Keep mtimes and modes so tools comparing timestamps behave as they would
    // on a bind mount. The host's uid means nothing in the pod, so `tar -xo`
    // discards it. Symlinks are shipped as symlinks, not followed.
    builder.mode(tar::HeaderMode::Complete);
    builder.follow_symlinks(false);
    let archived = builder
        .append_dir_all(".", &mount.host)
        .and_then(|_| builder.into_inner())
        .map(drop);

    let status = child.wait().map_err(|e| {
        ContainerError::ContainerExecution(format!("kubectl exec did not complete: {}", e))
    })?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(ContainerError::ContainerExecution(format!(
            "syncing {} into the pod failed: {}",
            mount.host.display(),
            stderr
        )));
    }
    archived.map_err(|e| {
        ContainerError::ContainerExecution(format!(
            "failed to archive {}: {}",
            mount.host.display(),
            e
        ))
    })
}

/// Write the tar archive in `reader` onto `host`, returning every path it held.
///
/// The archive comes from a workflow-controlled pod, so nothing here follows a
/// symlink or leaves `host`. A clean return does not prove the archive is
/// whole: tar has no length header, so a stream that stops between two entries
/// reads as a normal end of file. Only the sender's exit status can tell.
pub(super) fn unpack_archive<R: io::Read>(reader: R, host: &Path) -> io::Result<HashSet<PathBuf>> {
    let mut archive = tar::Archive::new(reader);
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut directories = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let Some(rel) = safe_relative_path(&entry.path()?) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let is_dir = entry.header().entry_type().is_dir();
        clear_type_conflict(host, &rel, is_dir)?;
        // Not every tar writes an entry for each parent, and whatever is
        // missing from `seen` is deleted by the prune.
        seen.extend(
            rel.ancestors()
                .skip(1)
                .filter(|parent| !parent.as_os_str().is_empty())
                .map(Path::to_path_buf),
        );
        seen.insert(rel);
        if is_dir {
            directories.push(entry);
        } else {
            entry.unpack_in(host)?;
        }
    }

    // Deepest first and after the files, so a read-only directory's mode is
    // only applied once its contents are in place.
    directories.sort_by_key(|dir| {
        std::cmp::Reverse(dir.path().map(|p| p.components().count()).unwrap_or(0))
    });
    for mut dir in directories {
        dir.unpack_in(host)?;
    }
    Ok(seen)
}

/// Remove what the pod no longer holds, as if its directory had been bind
/// mounted, so a step's `rm` or `mv` is seen by the next step.
///
/// `keep` lists paths in `tar --exclude` form that belong to nested mounts;
/// they are absent from the archive on purpose and must survive. Call this
/// only once the whole archive is known to have arrived.
pub(super) fn prune_to_archive(
    host: &Path,
    seen: &HashSet<PathBuf>,
    keep: &[String],
) -> io::Result<()> {
    let keep: Vec<PathBuf> = keep
        .iter()
        .filter_map(|p| safe_relative_path(Path::new(p)))
        .collect();
    prune_unlisted(host, Path::new(""), seen, &keep)
}

/// `path` without `.` components, or `None` if it could leave its root.
pub(super) fn safe_relative_path(path: &Path) -> Option<PathBuf> {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => clean.push(part),
            _ => return None,
        }
    }
    Some(clean)
}

/// Remove what is at `host/rel` when it is the wrong kind for the incoming
/// entry, because a step replaced a directory with a file or the reverse.
/// Does nothing if any parent is a symlink, which could point outside `host`.
pub(super) fn clear_type_conflict(
    host: &Path,
    rel: &Path,
    incoming_is_dir: bool,
) -> io::Result<()> {
    let mut current = host.to_path_buf();
    let parents = rel.parent().into_iter().flat_map(Path::components);
    for part in parents {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Ok(()),
            Ok(_) => {}
            Err(_) => return Ok(()),
        }
    }
    let target = host.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&target) else {
        return Ok(());
    };
    if meta.is_dir() && !incoming_is_dir {
        std::fs::remove_dir_all(&target)
    } else if !meta.is_dir() && incoming_is_dir {
        std::fs::remove_file(&target)
    } else {
        Ok(())
    }
}

/// Delete everything under `host/rel` that is neither in `seen` nor kept.
pub(super) fn prune_unlisted(
    host: &Path,
    rel: &Path,
    seen: &HashSet<PathBuf>,
    keep: &[PathBuf],
) -> io::Result<()> {
    for entry in std::fs::read_dir(host.join(rel))? {
        let entry = entry?;
        let child = rel.join(entry.file_name());
        if keep.iter().any(|k| child.starts_with(k)) {
            continue;
        }
        // `file_type` does not follow symlinks, so a link is removed itself
        // and never descended into.
        let file_type = entry.file_type()?;
        let removed = if seen.contains(&child) {
            if file_type.is_dir() {
                prune_unlisted(host, &child, seen, keep)?;
            }
            continue;
        } else if file_type.is_dir() {
            std::fs::remove_dir_all(entry.path())
        } else if file_type.is_file() || file_type.is_symlink() {
            std::fs::remove_file(entry.path())
        } else {
            // tar cannot carry a socket, so one that appeared on the host
            // during the step says nothing about what the step deleted.
            continue;
        };
        // A leftover file is not worth failing a step that already succeeded.
        if let Err(e) = removed {
            wrkflw_logging::warning(&format!(
                "Kubernetes: could not remove {}: {}",
                entry.path().display(),
                e
            ));
        }
    }
    Ok(())
}

/// Blocking: run `tar -c` in the sync container and unpack it onto `mount.host`.
pub(super) fn copy_dir_out_of_pod(
    kubectl: &OsStr,
    kubectl_args: &[String],
    pod: &str,
    mount: &Mount,
    excludes: &[String],
) -> Result<(), ContainerError> {
    std::fs::create_dir_all(&mount.host).map_err(|e| {
        ContainerError::ContainerExecution(format!(
            "failed to create {}: {}",
            mount.host.display(),
            e
        ))
    })?;
    let src = mount.container.to_string_lossy().to_string();
    let mut tar_args: Vec<String> = vec!["tar".into(), "-C".into(), src.clone()];
    for ex in excludes {
        tar_args.push(format!("--exclude={}", ex));
    }
    tar_args.extend(["-cf".into(), "-".into(), ".".into()]);

    let mut command = std::process::Command::new(kubectl);
    command
        .args(kubectl_args)
        .args(["exec", pod, "-c", SYNC_CONTAINER, "--"])
        .args(&tar_args);
    mirror_from(command, &src, &mount.host, excludes)?;
    wrkflw_logging::debug(&format!(
        "Kubernetes: synced {}:{} -> {}",
        pod,
        src,
        mount.host.display()
    ));
    Ok(())
}

/// Blocking: unpack the tar archive `command` prints onto `host`, then prune.
/// `src` only names the source in errors.
pub(super) fn mirror_from(
    mut command: std::process::Command,
    src: &str,
    host: &Path,
    excludes: &[String],
) -> Result<(), ContainerError> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            ContainerError::ContainerStart(format!("Failed to spawn kubectl exec: {}", e))
        })?;
    let stderr = drain_stderr(&mut child);

    let stdout = child.stdout.take().ok_or_else(|| {
        ContainerError::ContainerExecution("kubectl exec stdout was not captured".to_string())
    })?;
    let unpacked = unpack_archive(stdout, host);

    let status = child.wait().map_err(|e| {
        ContainerError::ContainerExecution(format!("kubectl exec did not complete: {}", e))
    })?;
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(ContainerError::ContainerExecution(format!(
            "syncing {} back from the pod failed: {}",
            src, stderr
        )));
    }
    let unpack_error = |e: io::Error| {
        ContainerError::ContainerExecution(format!(
            "failed to unpack {} into {}: {}",
            src,
            host.display(),
            e
        ))
    };
    let seen = unpacked.map_err(unpack_error)?;
    prune_to_archive(host, &seen, excludes).map_err(unpack_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What [`mirror_from`] does once the stream is known to be complete.
    fn unpack_mirroring<R: io::Read>(reader: R, host: &Path, keep: &[String]) -> io::Result<()> {
        let seen = unpack_archive(reader, host)?;
        prune_to_archive(host, &seen, keep)
    }

    /// Tar builder for the mirroring tests.
    fn archive(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        build(&mut b);
        b.into_inner().unwrap()
    }

    fn add_file(b: &mut tar::Builder<Vec<u8>>, path: &str, body: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, path, body).unwrap();
    }

    fn add_dir(b: &mut tar::Builder<Vec<u8>>, path: &str, mode: u32) {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_size(0);
        h.set_mode(mode);
        h.set_cksum();
        b.append_data(&mut h, path, io::empty()).unwrap();
    }

    #[test]
    fn mirroring_writes_updates_and_removes_deleted_entries() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("gone/deep")).unwrap();
        std::fs::write(host.path().join("gone/deep/f"), "x").unwrap();
        std::fs::write(host.path().join("deleted.txt"), "x").unwrap();
        std::fs::write(host.path().join("kept.txt"), "old").unwrap();

        let tar = archive(|b| {
            add_dir(b, "./", 0o755);
            add_file(b, "./kept.txt", b"new");
            add_dir(b, "./fresh/", 0o755);
            add_file(b, "./fresh/f.txt", b"made in pod");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        let read = |p: &str| std::fs::read_to_string(host.path().join(p)).unwrap();
        assert_eq!(read("kept.txt"), "new");
        assert_eq!(read("fresh/f.txt"), "made in pod");
        assert!(!host.path().join("deleted.txt").exists());
        assert!(!host.path().join("gone").exists());
    }

    /// `tar` is free to leave parents implicit, and `--k8s-helper-image` lets
    /// users pick the tar.
    #[test]
    fn mirroring_keeps_directories_the_archive_only_implies() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("a/b")).unwrap();
        std::fs::write(host.path().join("a/b/stale.txt"), "x").unwrap();

        let tar = archive(|b| add_file(b, "./a/b/f.txt", b"from the pod"));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        assert_eq!(
            std::fs::read_to_string(host.path().join("a/b/f.txt")).unwrap(),
            "from the pod"
        );
        assert!(!host.path().join("a/b/stale.txt").exists());
    }

    #[test]
    fn mirroring_keeps_nested_mounts_out_of_the_prune() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("sub/deep")).unwrap();
        std::fs::write(host.path().join("sub/deep/nested.txt"), "x").unwrap();
        std::fs::write(host.path().join("sub/other.txt"), "x").unwrap();

        let tar = archive(|b| add_dir(b, "./sub/", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &["./sub/deep".to_string()]).unwrap();

        assert!(host.path().join("sub/deep/nested.txt").exists());
        assert!(!host.path().join("sub/other.txt").exists());
    }

    #[test]
    fn mirroring_replaces_entries_that_changed_type() {
        let host = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(host.path().join("was-dir/child")).unwrap();
        std::fs::write(host.path().join("was-file"), "x").unwrap();

        let tar = archive(|b| {
            add_file(b, "./was-dir", b"now a file");
            add_dir(b, "./was-file/", 0o755);
            add_file(b, "./was-file/child", b"now a dir");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();

        assert!(host.path().join("was-dir").is_file());
        assert!(host.path().join("was-file/child").is_file());
    }

    #[test]
    fn mirroring_applies_read_only_directory_modes_after_their_files() {
        let host = tempfile::tempdir().unwrap();
        let tar = archive(|b| {
            add_dir(b, "./locked/", 0o555);
            add_file(b, "./locked/f.txt", b"inside");
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(host.path().join("locked/f.txt").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = host.path().join("locked");
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o555);
            // Let tempdir clean up.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn mirroring_never_follows_symlinks_out_of_the_host_directory() {
        let host = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("precious.txt"), "x").unwrap();
        std::fs::create_dir(outside.path().join("target")).unwrap();
        std::os::unix::fs::symlink(outside.path(), host.path().join("link")).unwrap();

        // `link` stays in the archive, so it is not pruned, and an entry
        // beneath it must not reach the directory it points at.
        let tar = archive(|b| {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, "./link", outside.path()).unwrap();
            add_file(b, "./link/target", b"replaced");
        });
        let _ = unpack_mirroring(tar.as_slice(), host.path(), &[]);
        assert!(outside.path().join("target").is_dir());
        assert!(outside.path().join("precious.txt").exists());

        // A pruned symlink goes away without touching its target.
        let tar = archive(|b| add_dir(b, "./", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(!host.path().join("link").exists());
        assert!(outside.path().join("precious.txt").exists());
    }

    #[test]
    fn mirroring_does_not_delete_on_a_broken_stream() {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("existing.txt"), "x").unwrap();
        let mut tar = archive(|b| add_file(b, "./big.bin", &[7u8; 4096]));
        // Cut the stream off inside the file body.
        tar.truncate(1024);
        assert!(unpack_mirroring(tar.as_slice(), host.path(), &[]).is_err());
        assert!(host.path().join("existing.txt").exists());
    }

    /// `sh -c script` with the archive in `$1`, standing in for `kubectl exec`.
    #[cfg(unix)]
    fn sender(script: &str, tar: &Path) -> std::process::Command {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script, "sh"]).arg(tar);
        command
    }

    #[cfg(unix)]
    fn write_tar(dir: &Path, build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> PathBuf {
        let path = dir.join("stream.tar");
        std::fs::write(&path, archive(build)).unwrap();
        path
    }

    /// A connection lost between two entries is a clean EOF to the tar reader,
    /// and everything not yet received would otherwise look deleted.
    #[cfg(unix)]
    #[test]
    fn a_failed_exec_never_prunes_even_if_the_stream_looks_complete() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("not-yet-sent.txt"), "precious").unwrap();

        let tar = write_tar(work.path(), |b| add_file(b, "./first.txt", b"x"));
        let err = mirror_from(
            sender("head -c 1024 \"$1\"; echo lost >&2; exit 1", &tar),
            "/github/workspace",
            host.path(),
            &[],
        )
        .expect_err("a failed exec must fail the sync");

        assert!(err.to_string().contains("lost"), "{}", err);
        assert!(host.path().join("not-yet-sent.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_successful_exec_mirrors_and_prunes() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("deleted.txt"), "x").unwrap();

        let tar = write_tar(work.path(), |b| add_file(b, "./kept.txt", b"x"));
        mirror_from(sender("cat \"$1\"", &tar), "src", host.path(), &[]).unwrap();

        assert!(host.path().join("kept.txt").exists());
        assert!(!host.path().join("deleted.txt").exists());
    }

    /// More stderr than a pipe holds, before any stdout. Unread, this hangs.
    #[cfg(unix)]
    #[test]
    fn a_noisy_stderr_does_not_stall_the_sync_out() {
        let work = tempfile::tempdir().unwrap();
        let host = tempfile::tempdir().unwrap();
        let tar = write_tar(work.path(), |b| add_file(b, "./f.txt", b"x"));
        mirror_from(
            sender("head -c 1000000 /dev/zero >&2; cat \"$1\"", &tar),
            "src",
            host.path(),
            &[],
        )
        .unwrap();
        assert!(host.path().join("f.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_noisy_stderr_does_not_stall_the_sync_in() {
        let host = tempfile::tempdir().unwrap();
        std::fs::write(host.path().join("f.txt"), "x").unwrap();
        let mut command = std::process::Command::new("sh");
        command.args([
            "-c",
            "head -c 1000000 /dev/zero >&2; cat >/dev/null; echo refused >&2; exit 2",
        ]);
        let mount = Mount {
            host: host.path().to_path_buf(),
            container: PathBuf::from("/data"),
        };
        let err = stream_dir_into(command, &mount).expect_err("must report the failure");
        // Only the first bytes of a flood are kept, and it was all NULs.
        assert!(err.to_string().contains("into the pod failed"), "{}", err);
    }

    #[cfg(unix)]
    #[test]
    fn pruning_leaves_sockets_alone() {
        let host = tempfile::tempdir().unwrap();
        let socket = host.path().join("s");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let tar = archive(|b| add_dir(b, "./", 0o755));
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert!(socket.exists());
    }

    #[test]
    fn mirroring_unpacks_hard_links_inside_the_host_directory() {
        let host = tempfile::tempdir().unwrap();
        let tar = archive(|b| {
            add_file(b, "./original", b"same");
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Link);
            h.set_size(0);
            h.set_mode(0o644);
            b.append_link(&mut h, "./linked", "./original").unwrap();
        });
        unpack_mirroring(tar.as_slice(), host.path(), &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(host.path().join("linked")).unwrap(),
            "same"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_pruned_does_not_fail_the_step() {
        use std::os::unix::fs::PermissionsExt;
        let host = tempfile::tempdir().unwrap();
        let locked = host.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::write(locked.join("stuck.txt"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let tar = archive(|b| add_dir(b, "./locked/", 0o555));
        let result = unpack_mirroring(tar.as_slice(), host.path(), &[]);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        result.unwrap();
    }

    #[test]
    fn safe_relative_path_rejects_escapes() {
        assert_eq!(
            safe_relative_path(Path::new("./a/./b")),
            Some(PathBuf::from("a/b"))
        );
        assert_eq!(safe_relative_path(Path::new("./")), Some(PathBuf::new()));
        assert_eq!(safe_relative_path(Path::new("a/../../b")), None);
        assert_eq!(safe_relative_path(Path::new("/etc/passwd")), None);
    }
}
