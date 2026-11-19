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
/// On Linux a bind mount keeps this machine's owners, and the job's
/// workspace, its `$GITHUB_ENV` files and the `--dep-cache` directories are
/// all bind-mounted, holding files owned by whoever runs wrkflw: a container
/// user with another uid could neither change them nor leave files this
/// user can remove. So the user takes that uid, also when the workspace is
/// in a volume, whose files wrkflw gives to this same uid (see
/// `vm_workspace`). Elsewhere, on macOS and Windows, the engine's VM maps a
/// bind mount's owner to whoever reads it, and the user gets
/// [`HOSTED_UID`], as on GitHub.
pub(crate) fn uid() -> u32 {
    uid_for(host_uid())
}

/// The uids Debian and Ubuntu give people's accounts. Below them are root
/// and system accounts, and above them `nobody`: the image's user with the
/// uid is removed to make way for the runner user, so only a uid of this
/// range is taken.
const USER_UIDS: std::ops::Range<u32> = 1000..60000;

fn uid_for(host_uid: Option<u32>) -> u32 {
    match host_uid {
        // Root's files are root's, and a system account's are its own: the
        // user gets GitHub's uid, can't write them, and the session finds
        // that out and runs the steps as root.
        Some(uid) if USER_UIDS.contains(&uid) => uid,
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

/// Whether this run's steps may run as the user, as far as can be told
/// before any job's container is up: see [`ruled_out`]. Settled once per
/// run by [`settle_for_run`]; before that, or outside a run, from what
/// needs no engine.
///
/// The reuse key reads it, so a run that can't have the user files its
/// passes as root's, and can reuse them, rather than storing none.
pub(crate) fn for_run() -> bool {
    crate::acceleration::settled_runner_user()
        .unwrap_or_else(|| ruled_out(allowed(), host_uid(), false))
        .is_none()
}

/// Settle [`for_run`] for this run, asking the engine behind `cli` whether
/// it is rootless. Says nothing: see [`warn_if_ruled_out`].
pub(crate) async fn settle_for_run(cli: Option<&'static str>) {
    crate::acceleration::settle_runner_user(|| async move {
        let rootless = match cli {
            Some(cli) if allowed() => engine_is_rootless(cli).await,
            _ => false,
        };
        ruled_out(allowed(), host_uid(), rootless)
    })
    .await
}

/// Say, once per run, why its steps run as root, if the run settled that
/// they must for a reason it didn't ask for. Called for a job that would
/// have had the user, so a run whose jobs all have a `container:` or an
/// image mapped with `-P` hears nothing of it.
pub(crate) fn warn_if_ruled_out() {
    let Some(Some(Ruled::Because(reason))) = crate::acceleration::settled_runner_user() else {
        return;
    };
    if crate::acceleration::first_root_steps_warning() {
        wrkflw_logging::warning(&format!(
            "Steps run as root in this run, not as the {NAME} user GitHub's run as: \
             {reason}. Set {STEPS_AS_ROOT_ENV}=1 to say so and keep this warning away"
        ));
    }
}

/// Why a run's steps can't run as the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ruled {
    /// The run asked for root with [`STEPS_AS_ROOT_ENV`].
    Asked,
    /// The user couldn't write the job's files, for this reason.
    Because(&'static str),
}

/// Why steps can't run as the user in a run that `allowed` it, on a
/// machine whose uid is `host_uid` (on Linux), with an engine that is
/// `rootless` or not; `None` when they may.
///
/// Each of these leaves the job's bind-mounted files owned by someone the
/// user isn't inside the container: root's or a system account's on Linux,
/// where the user then gets GitHub's uid (see [`uid`]), and this machine's
/// user's under a rootless engine, which shows them as root's. Found here,
/// before any job, the run's keys say its steps run as root, and their
/// passes are stored and reused as such; found only once a job's container
/// is up, as for files an earlier run left that can't be handed over, the
/// job's pass isn't stored at all.
fn ruled_out(allowed: bool, host_uid: Option<u32>, rootless: bool) -> Option<Ruled> {
    if !allowed {
        return Some(Ruled::Asked);
    }
    if host_uid.is_some_and(|uid| !USER_UIDS.contains(&uid)) {
        return Some(Ruled::Because(
            "wrkflw runs as root or a system account, whose files that user can't write",
        ));
    }
    if rootless {
        return Some(Ruled::Because(
            "the container engine is rootless, where this machine's files are root's \
             inside a container",
        ));
    }
    None
}

/// How long the engine may take to say whether it is rootless.
const ENGINE_INFO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Whether the engine behind `cli` runs rootless, with root in a container
/// being this machine's user. `false` when it can't tell: each job's
/// container then finds out for itself.
///
/// A rootless Podman whose containers keep the user's ids (`keep-id`)
/// doesn't count: its containers show this machine's files as the user's
/// own, which the runner user, with that uid, can write.
async fn engine_is_rootless(cli: &str) -> bool {
    let podman = cli.ends_with("podman");
    engine_reports_rootless(cli, podman).await && !(podman && podman_keeps_ids())
}

async fn engine_reports_rootless(cli: &str, podman: bool) -> bool {
    // Podman says so in its host's security; Docker among its security
    // options, as `name=rootless`.
    let format = if podman {
        "{{.Host.Security.Rootless}}"
    } else {
        "{{json .SecurityOptions}}"
    };
    let output = tokio::process::Command::new(cli)
        .args(["info", "--format", format])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(ENGINE_INFO_TIMEOUT, output).await {
        Ok(Ok(output)) if output.status.success() => {
            reports_rootless(&String::from_utf8_lossy(&output.stdout))
        }
        _ => false,
    }
}

/// Whether Podman's containers get the user's own ids by default: from
/// `PODMAN_USERNS`, else the first containers.conf that sets `userns`, in
/// the order Podman reads them. A drop-in directory isn't read, and one that
/// sets it is missed: the run then keeps steps as root, as before there was
/// a runner user.
fn podman_keeps_ids() -> bool {
    if let Ok(userns) = std::env::var("PODMAN_USERNS") {
        return keeps_ids(&userns);
    }
    let user_conf = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .map(|dir| dir.join("containers/containers.conf"));
    std::env::var_os("CONTAINERS_CONF")
        .map(std::path::PathBuf::from)
        .into_iter()
        .chain(user_conf)
        .chain(
            [
                "/etc/containers/containers.conf",
                "/usr/share/containers/containers.conf",
            ]
            .map(std::path::PathBuf::from),
        )
        .find_map(|path| conf_userns(&std::fs::read_to_string(path).ok()?))
        .is_some_and(|userns| keeps_ids(&userns))
}

/// The `userns` that containers.conf text sets for containers, if any.
fn conf_userns(conf: &str) -> Option<String> {
    let mut in_containers = false;
    for line in conf.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_containers = line == "[containers]";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if in_containers && key.trim() == "userns" {
            return Some(
                value
                    .trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string(),
            );
        }
    }
    None
}

/// Whether a Podman `userns` mode maps the user's uid to itself.
fn keeps_ids(userns: &str) -> bool {
    userns.trim().starts_with("keep-id")
}

/// Whether `info`, Podman's or Docker's answer to the format
/// [`engine_is_rootless`] asks for, says the engine is rootless.
fn reports_rootless(info: &str) -> bool {
    let info = info.trim();
    info == "true" || info.contains("name=rootless")
}

/// Shell that creates the user in an image being built, with `uid`, a home
/// directory, `sudo` without a password, and the directories a step
/// installs into owned by it. Run as root, after `sudo` is installed.
///
/// A user the base image has under that uid, as `ubuntu:latest` has
/// `ubuntu` under 1000, is removed first: two names for one uid would make
/// `whoami` answer with the other. The user's group has the same number as
/// its uid, as the workspace volume's files are given both (see
/// `vm_workspace`), even where the base image has a group with that number.
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
         (getent group {NAME} >/dev/null || groupadd --non-unique --gid {uid} {NAME}) && \
         (id -u {NAME} >/dev/null 2>&1 || \
          useradd --no-log-init --create-home --home-dir {HOME} --uid {uid} --gid {NAME} --shell /bin/bash {NAME}) && \
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

    /// The user takes the uid of whoever owns the job's bind-mounted
    /// files on Linux, workspace in a volume or not, and GitHub's own where
    /// the VM maps the owner.
    #[test]
    fn the_user_takes_the_uid_that_owns_the_jobs_files() {
        // Linux: this user's files.
        assert_eq!(uid_for(Some(1000)), 1000);
        assert_eq!(uid_for(Some(1001)), HOSTED_UID);
        // Root's files can't be another user's: the hosted uid, and the
        // session falls back to root.
        assert_eq!(uid_for(Some(0)), HOSTED_UID);
        // Nor is a system account's uid taken, or `nobody`'s, whose
        // accounts the image has and would lose.
        assert_eq!(uid_for(Some(42)), HOSTED_UID);
        assert_eq!(uid_for(Some(999)), HOSTED_UID);
        assert_eq!(uid_for(Some(65534)), HOSTED_UID);
        assert_eq!(uid_for(Some(59999)), 59999);
        // macOS and Windows: the VM maps the owner.
        assert_eq!(uid_for(None), HOSTED_UID);
    }

    /// A run that asked for root, or whose job files the user couldn't
    /// write, keys its passes as root's before any job; one that may have
    /// the user doesn't.
    #[test]
    fn a_run_rules_out_the_user_when_it_couldnt_write_the_jobs_files() {
        assert_eq!(ruled_out(true, Some(1000), false), None);
        assert_eq!(ruled_out(true, None, false), None);
        assert_eq!(ruled_out(false, Some(1000), false), Some(Ruled::Asked));
        for (uid, rootless) in [(Some(0), false), (Some(999), false), (Some(1000), true)] {
            assert!(
                matches!(ruled_out(true, uid, rootless), Some(Ruled::Because(_))),
                "{uid:?} {rootless}"
            );
        }
        // Asked for, it isn't warned of.
        assert_eq!(ruled_out(false, Some(0), true), Some(Ruled::Asked));
    }

    /// A rootless Podman that keeps the user's ids lets the runner user
    /// write the job's files, as its containers say in containers.conf.
    #[test]
    fn podman_keeping_the_users_ids_is_read_from_containers_conf() {
        let conf = "[engine]\nuserns = \"auto\"\n\n[containers]\n# a comment\nuserns = \"keep-id:uid=1000\"\n";
        assert_eq!(conf_userns(conf).as_deref(), Some("keep-id:uid=1000"));
        assert!(keeps_ids(&conf_userns(conf).unwrap()));
        assert_eq!(
            conf_userns("[containers]\nuserns='host'\n").as_deref(),
            Some("host")
        );
        assert!(!keeps_ids("host"));
        assert!(!keeps_ids(""));
        // Set in another table, it says nothing of containers.
        assert_eq!(conf_userns("[engine]\nuserns = \"keep-id\"\n"), None);
        assert_eq!(conf_userns("[containers]\n#userns = \"keep-id\"\n"), None);
    }

    #[test]
    fn rootless_engines_say_so_in_their_info() {
        // Podman's `.Host.Security.Rootless`.
        assert!(reports_rootless("true\n"));
        assert!(!reports_rootless("false\n"));
        // Docker's `.SecurityOptions`.
        assert!(reports_rootless(
            r#"["name=seccomp,profile=builtin","name=rootless","name=cgroupns"]"#
        ));
        assert!(!reports_rootless(
            r#"["name=seccomp,profile=builtin","name=cgroupns"]"#
        ));
        assert!(!reports_rootless(""));
    }

    #[test]
    fn the_user_is_created_with_sudo_and_owns_what_steps_install_into() {
        let script = create_script(1000);
        assert!(script.contains("getent passwd 1000"), "{script}");
        assert!(script.contains("--uid 1000 --gid runner"), "{script}");
        // The group's number is the uid, which the volume's files get too.
        assert!(
            script.contains("groupadd --non-unique --gid 1000 runner"),
            "{script}"
        );
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
