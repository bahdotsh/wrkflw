//! The runner base image published with each release: the packages, tools,
//! Node.js, GitHub CLI and runner user every built-in job image starts
//! with, so a first run pulls them instead of installing them.
//!
//! The image is named by a hash of its Dockerfile, which
//! [`crate::engine::runner_base_dockerfile`] writes, so a wrkflw asks only
//! for an image built from the same text it would build itself. The
//! release workflow builds `docker/runner.Dockerfile` and pushes it as
//! the tag in `docker/runner-image.txt`; a test keeps both current.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where the runner base images are published.
pub const REPOSITORY: &str = "ghcr.io/bahdotsh/wrkflw-runner";

/// Env var that, set to `0`, builds every job image here, as before.
pub const PREBUILT_IMAGES_ENV: &str = "WRKFLW_PREBUILT_IMAGES";

/// Env var naming another runner base image to start from, such as one
/// built locally from `docker/runner.Dockerfile` for a test.
pub const RUNNER_IMAGE_ENV: &str = "WRKFLW_RUNNER_IMAGE";

/// How long a runner base image the registry refused isn't tried again.
pub(crate) const REFUSED_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a runner base image that couldn't be reached (no network, a
/// firewall in front of the registry) isn't tried again: long enough that
/// a machine offline for the day doesn't wait on the pull every run, short
/// enough that it soon has the image once it's back.
pub(crate) const UNREACHABLE_FOR: Duration = Duration::from_secs(60 * 60);

/// The published name of the runner base image built from `dockerfile`.
pub fn published_image(dockerfile: &str) -> String {
    format!(
        "{}:{:016x}",
        REPOSITORY,
        crate::engine::fnv1a_hash(dockerfile.as_bytes())
    )
}

/// The runner base image to start job images from, or `None` when
/// [`PREBUILT_IMAGES_ENV`] turns it off or it couldn't be pulled lately.
pub(crate) fn image() -> Option<String> {
    // Unit tests build with mock runtimes and mustn't pull, or leave the
    // unavailable marker in the user's cache.
    if cfg!(test) || !enabled() {
        return None;
    }
    let image = std::env::var(RUNNER_IMAGE_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| published_image(&crate::engine::runner_base_dockerfile()));
    (!recently_unavailable(&image, SystemTime::now())).then_some(image)
}

fn enabled() -> bool {
    crate::run_options::env_switch(PREBUILT_IMAGES_ENV, true)
}

fn marker_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("wrkflw").join("runner-base-unavailable"))
}

/// Whether a failed pull of the base image `image` says the registry has
/// no such image for us (not published yet, or still private), rather than
/// that the network or the machine failed, which says nothing about
/// tomorrow.
pub(crate) fn pull_refused(image: &str, error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    // A missing tag is "<image>: not found" or "manifest unknown"; a
    // "not found" naming anything else is a missing `docker`, shell or file.
    let missing_tag = format!("{}: not found", image.to_ascii_lowercase());
    // A registry's "denied" is always about access to the image; a bare
    // "denied" would also match the engine's own "permission denied", such
    // as on its socket, which says nothing about the image.
    error.contains(&missing_tag)
        || [
            "manifest unknown",
            "pull access denied",
            "requested access to the resource is denied",
            "error from registry: denied",
            "unauthorized",
            "403 forbidden",
        ]
        .iter()
        .any(|marker| error.contains(marker))
}

/// Remember that `image` couldn't be pulled, so runs within `period` build
/// their images here without trying it first.
pub(crate) fn mark_unavailable(image: &str, period: Duration) {
    let Some(path) = marker_path() else { return };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, format!("{}\n{}\n{}\n", image, now, period.as_secs()));
}

fn recently_unavailable(image: &str, now: SystemTime) -> bool {
    let Some(text) = marker_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return false;
    };
    unavailable_in(&text, image, now)
}

/// Whether the marker `text` says `image` couldn't be pulled within the
/// period it names (a marker from before periods were named: a day) of
/// `now`.
fn unavailable_in(text: &str, image: &str, now: SystemTime) -> bool {
    let mut lines = text.lines();
    if lines.next() != Some(image) {
        return false;
    }
    let Some(when) = lines.next().and_then(|l| l.trim().parse::<u64>().ok()) else {
        return false;
    };
    let period = lines
        .next()
        .and_then(|l| l.trim().parse::<u64>().ok())
        .unwrap_or(REFUSED_FOR.as_secs());
    let now = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    now.saturating_sub(when) < period
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_dockerfile_and_tag_are_current() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dockerfile = crate::engine::runner_base_dockerfile();
        // A Windows checkout may still turn line endings into CRLF.
        let committed = std::fs::read_to_string(root.join("docker/runner.Dockerfile"))
            .unwrap()
            .replace("\r\n", "\n");
        let tag = std::fs::read_to_string(root.join("docker/runner-image.txt")).unwrap();
        assert!(
            committed == dockerfile && tag.trim() == published_image(&dockerfile),
            "docker/runner.Dockerfile or docker/runner-image.txt is out of date: run \
             `cargo run -p wrkflw-executor --example runner_dockerfile` to rewrite them"
        );
    }

    #[test]
    fn the_image_names_its_dockerfile() {
        let a = published_image("FROM a\n");
        assert!(a.starts_with("ghcr.io/bahdotsh/wrkflw-runner:"));
        assert_ne!(a, published_image("FROM b\n"));
    }

    #[test]
    fn only_a_registry_refusal_is_remembered() {
        let image = "ghcr.io/bahdotsh/wrkflw-runner:abc";
        let refused = |error: &str| pull_refused(image, error);
        assert!(refused(
            "ERROR: failed to solve: ghcr.io/bahdotsh/wrkflw-runner:abc: not found"
        ));
        assert!(refused(
            "failed to authorize: failed to fetch anonymous token: 403 Forbidden"
        ));
        assert!(refused("manifest unknown"));
        assert!(!refused("dial tcp: lookup ghcr.io: no such host"));
        assert!(!refused("no space left on device"));
        assert!(!refused("docker not found in PATH"));
        assert!(!refused("sh: 1: docker: not found"));
        assert!(refused(
            "pull access denied for ghcr.io/bahdotsh/wrkflw-runner, repository does not exist"
        ));
        assert!(refused("error from registry: denied\ndenied"));
        assert!(!refused(
            "permission denied while trying to connect to the Docker daemon socket at \
             unix:///var/run/docker.sock"
        ));
        assert!(!refused(
            "open /root/.docker/config.json: permission denied"
        ));
    }

    #[test]
    fn an_unavailable_image_is_skipped_for_a_day() {
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        let marker = "img:1\n1000\n";
        assert!(unavailable_in(marker, "img:1", at(1000 + 60)));
        assert!(!unavailable_in(marker, "img:1", at(1000 + 25 * 3600)));
        assert!(!unavailable_in(marker, "img:2", at(1000 + 60)));
        assert!(!unavailable_in("garbage", "img:1", at(1000)));

        let unreachable = "img:1\n1000\n3600\n";
        assert!(unavailable_in(unreachable, "img:1", at(1000 + 60)));
        assert!(!unavailable_in(unreachable, "img:1", at(1000 + 3601)));
    }
}
