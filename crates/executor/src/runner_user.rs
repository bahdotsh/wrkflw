//! The user a job's steps run as in the image wrkflw builds.
//!
//! GitHub's hosted runner runs steps as `runner`, a user with a home
//! directory and `sudo` without a password, not as root. The difference
//! shows: root may write a read-only file, so a test that expects a
//! permission error passes on GitHub and fails as root, and tools such as
//! `initdb` refuse to run as root at all.
//!
//! The image gets that user, and owns to it what a step installs into: its
//! home with rustup and cargo, the interpreters setup-python installs, and
//! npm's global directory. Steps then `exec` into the job's container as
//! that user, when it can write the job's workspace there; where it can't,
//! as under a rootless engine, they run as root, as they did before.
//!
//! An image mapped with `-P`, a job `container:` and a step in a container
//! of its own keep the image's user, which is the image's business: a
//! container job on GitHub runs as its image's user too.

/// The user's name, as on GitHub's hosted runners.
pub(crate) const NAME: &str = "runner";
/// Its home directory, as on GitHub's hosted runners.
pub(crate) const HOME: &str = "/home/runner";
/// Its uid and gid on GitHub's hosted runners, used wherever the
/// workspace's files don't dictate another.
pub(crate) const HOSTED_UID: u32 = 1001;
/// Env var that keeps steps running as root.
pub const STEPS_AS_ROOT_ENV: &str = "WRKFLW_STEPS_AS_ROOT";

/// The uid the user gets in images built on this machine.
///
/// A workspace that is this machine's directory, bind-mounted, holds files
/// owned by whoever runs wrkflw, and a container user with another uid
/// could neither change them nor leave files this user can remove. So the
/// user takes that uid. A workspace in a volume holds files wrkflw copies
/// in, which it gives to [`HOSTED_UID`], as it does where the engine's VM
/// maps a bind mount's owner to whoever reads it, on macOS and Windows.
pub(crate) fn uid() -> u32 {
    uid_for(host_uid(), crate::vm_workspace::volume_mode())
}

fn uid_for(host_uid: Option<u32>, workspace_in_volume: bool) -> u32 {
    match host_uid {
        // Root's files are root's: no other user can write them, and the
        // session finds that out and runs the steps as root.
        Some(uid) if uid != 0 && !workspace_in_volume => uid,
        _ => HOSTED_UID,
    }
}

/// This process's uid where a bind mount keeps the host's owners: on Linux.
fn host_uid() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata("/proc/self").ok().map(|meta| meta.uid())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Whether the run lets steps run as the user at all.
pub(crate) fn allowed() -> bool {
    !crate::run_options::env_flag(STEPS_AS_ROOT_ENV)
}

/// Shell that creates the user in an image being built, with `uid`, a home
/// directory, `sudo` without a password, and the directories a step
/// installs into owned by it. Run as root, after `sudo` is installed.
///
/// A user the base image has under that uid, as `ubuntu:latest` has
/// `ubuntu` under 1000, is removed first: two names for one uid would make
/// `whoami` answer with the other.
pub(crate) fn create_script(uid: u32) -> String {
    // What a step may install into without `sudo` on the hosted runner:
    // its home, and the shared directories under /usr/local and /opt.
    let home_dirs = [
        ".cargo",
        ".cache",
        ".config",
        ".local/bin",
        ".npm",
        "go/pkg",
    ]
    .map(|dir| format!("{HOME}/{dir}"))
    .join(" ");
    format!(
        "(other=\"$(getent passwd {uid} | cut -d: -f1)\"; \
          [ -z \"$other\" ] || [ \"$other\" = {NAME} ] || userdel -r \"$other\" 2>/dev/null || true) && \
         (id -u {NAME} >/dev/null 2>&1 || \
          useradd --no-log-init --create-home --home-dir {HOME} --uid {uid} --user-group --shell /bin/bash {NAME}) && \
         echo '{NAME} ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/{NAME} && \
         chmod 0440 /etc/sudoers.d/{NAME} && \
         mkdir -p {home_dirs} /opt/hostedtoolcache && \
         chown -R {NAME}:{NAME} {HOME} /opt/hostedtoolcache && \
         for dir in /opt /usr/local/bin /usr/local/lib /usr/local/share /usr/local/include; do \
           [ ! -d \"$dir\" ] || chown {NAME}:{NAME} \"$dir\"; done"
    )
}

/// Shell that gives the user what the runtime `language` just installed
/// and a step later writes into, or nothing. It goes in the same `RUN` as
/// the install: changing a file's owner in a later layer copies the file.
pub(crate) fn own_script(language: &str) -> Option<String> {
    // `-h`: a link is handed over itself, not the file it points to, which
    // may be another runtime's.
    let own = |paths: &str| format!("chown -hR {NAME}:{NAME} {paths}");
    match language {
        // rustup and cargo, which `rustup toolchain add`, `cargo install`
        // and every build's registry cache write into.
        "rust" => Some(own(&format!("{HOME}/.cargo {HOME}/.rustup"))),
        // `pip install` writes into the interpreter's own directory.
        "python" | "uv" => Some(format!("(! [ -d /opt/uv ] || {})", own("/opt/uv"))),
        // `npm install -g` writes beside npm itself. A Node.js from a
        // package has /usr for its prefix, which stays root's: its global
        // installs go under /usr/local instead, which is on PATH first.
        "node" => Some(format!(
            "(prefix=\"$(npm prefix -g 2>/dev/null)\"; \
              if [ \"$prefix\" = /usr/local ]; then {} 2>/dev/null || true; \
              else npm config --global set prefix /usr/local; fi)",
            own("/usr/local/lib/node_modules /usr/local/bin /usr/local/include /usr/local/share")
        )),
        // The engine's socket is the `docker` group's.
        "docker" => Some(format!(
            "(! getent group docker >/dev/null || usermod -aG docker {NAME})"
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user takes the uid of whoever owns a bind-mounted workspace's
    /// files, and GitHub's own everywhere else.
    #[test]
    fn the_user_takes_the_uid_that_owns_the_workspace() {
        // Linux, the directory bind-mounted: this user's files.
        assert_eq!(uid_for(Some(1000), false), 1000);
        assert_eq!(uid_for(Some(1001), false), HOSTED_UID);
        // Root's files can't be another user's: the hosted uid, and the
        // session falls back to root.
        assert_eq!(uid_for(Some(0), false), HOSTED_UID);
        // A volume holds what wrkflw copies in.
        assert_eq!(uid_for(Some(1000), true), HOSTED_UID);
        // macOS and Windows: the VM maps the owner.
        assert_eq!(uid_for(None, false), HOSTED_UID);
        assert_eq!(uid_for(None, true), HOSTED_UID);
    }

    #[test]
    fn the_user_is_created_with_sudo_and_owns_what_steps_install_into() {
        let script = create_script(1000);
        assert!(script.contains("getent passwd 1000"), "{script}");
        assert!(script.contains("--uid 1000 --user-group"), "{script}");
        // A large uid would otherwise grow a sparse lastlog in the layer.
        assert!(script.contains("useradd --no-log-init"), "{script}");
        assert!(script.contains("runner ALL=(ALL) NOPASSWD:ALL"), "{script}");
        assert!(
            script.contains("chown -R runner:runner /home/runner"),
            "{script}"
        );
        // A directory a cache is mounted under exists already, as the
        // user's: the engine would create it as root's.
        for dir in [
            "/home/runner/.cargo",
            "/home/runner/.cache",
            "/home/runner/go/pkg",
        ] {
            assert!(script.contains(dir), "{dir} missing: {script}");
        }

        assert!(own_script("rust").unwrap().contains("/home/runner/.rustup"));
        assert!(own_script("python").unwrap().contains("/opt/uv"));
        assert!(own_script("node")
            .unwrap()
            .contains("/usr/local/lib/node_modules"));
        assert!(own_script("docker")
            .unwrap()
            .contains("usermod -aG docker runner"));
        assert_eq!(own_script("go"), None);
        assert_eq!(own_script("java"), None);
    }
}
