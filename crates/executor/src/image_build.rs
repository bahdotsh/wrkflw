//! Building the images jobs run in: the layers images on one base share,
//! the builds started ahead of a job's turn, and what a run remembers of a
//! build that failed. The Dockerfile itself, and which image a job gets,
//! are the engine's.

use futures::future;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use wrkflw_runtime::container::{ContainerError, ContainerRuntime, COMBINED_IMAGE_PREFIX};

use crate::engine::fnv1a_hash;

/// One lock per image of shared layers, by its tag: the first build of
/// those layers goes alone. See `ensure_shared_layers`.
static SHARED_LAYER_BUILDS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

tokio::task_local! {
    /// The combined images whose build failed in this run, by tag, with
    /// the error. A job's image is built ahead while it waits, and again
    /// at its turn if that didn't produce it: a build that fails on its
    /// own would fail the same way, and take as long, so the job is given
    /// the first failure instead. The next run tries again.
    static FAILED_IMAGE_BUILDS: Mutex<HashMap<String, String>>;
}

/// Run `future` as one run, with no image build failed yet.
pub(crate) async fn with_run_image_builds<F: std::future::Future>(future: F) -> F::Output {
    FAILED_IMAGE_BUILDS
        .scope(Mutex::new(HashMap::new()), future)
        .await
}

/// Why `tag` failed to build earlier in this run, if it did.
pub(crate) fn failed_image_build(tag: &str) -> Option<String> {
    FAILED_IMAGE_BUILDS
        .try_with(|failed| failed.lock().unwrap().get(tag).cloned())
        .ok()
        .flatten()
}

/// Remember for the rest of the run that `tag` failed to build, unless
/// the same build [may work on another try](may_build_on_another_try).
pub(crate) fn remember_failed_image_build(tag: &str, error: &ContainerError) {
    if may_build_on_another_try(error) {
        return;
    }
    let _ = FAILED_IMAGE_BUILDS.try_with(|failed| {
        failed
            .lock()
            .unwrap()
            .insert(tag.to_string(), error.to_string())
    });
}

/// Whether `error` says nothing of the build itself, so the same build may
/// well work when it is tried again: the connection to the engine was
/// lost, or the build ran out of time. A build ahead shares the engine
/// with the jobs running and with other builds, and one that ran out of
/// time there is built again by its job, at its turn.
fn may_build_on_another_try(error: &ContainerError) -> bool {
    matches!(error, ContainerError::ImageBuildTimeout(_)) || is_transport_error(&error.to_string())
}

/// Build the layers a combined image shares with every image on the same
/// base, as an image of their own, unless the engine has it already.
///
/// Images that start with the same packages build them once: the first
/// build of those layers goes alone, and the builds waiting on it go on to
/// their own runtimes as soon as it is over, built or failed. Started
/// together, five BuildKit builds took 67 s to give the first image where
/// one alone took 43 s. Only the shared layers are waited for, not another
/// job's runtimes, and an engine that has the shared image from an earlier
/// run waits for nothing.
///
/// Only where a build [reuses the layers](ContainerRuntime::reuses_build_layers)
/// an earlier one left: no step runs in the shared image, so a runtime
/// that ships every image it builds to a registry, as Kubernetes does,
/// would build and ship it on every run for nothing.
pub(crate) async fn ensure_shared_layers(
    dockerfile: &str,
    runtime: &dyn ContainerRuntime,
) -> Result<(), String> {
    if !runtime.reuses_build_layers() {
        return Ok(());
    }
    let shared = shared_layers(dockerfile);
    if shared.len() == dockerfile.len() {
        return Ok(());
    }
    let tag = shared_layers_tag(shared);
    if runtime.image_exists(&tag).await.unwrap_or(false) {
        return Ok(());
    }
    let lock = {
        let mut locks = SHARED_LAYER_BUILDS.lock().unwrap();
        locks
            .entry(tag.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _first = lock.lock().await;
    // Whoever held the lock before built these layers, or failed to: the
    // same build would fail the same way.
    if let Some(error) = failed_image_build(&tag) {
        return Err(error);
    }
    if runtime.image_exists(&tag).await.unwrap_or(false) {
        return Ok(());
    }
    let temp_dir =
        tempfile::tempdir().map_err(|e| format!("Failed to create temp directory: {}", e))?;
    let dockerfile_path = temp_dir.path().join("Dockerfile");
    std::fs::write(&dockerfile_path, shared)
        .map_err(|e| format!("Failed to write Dockerfile: {}", e))?;
    build_image_checked(runtime, &dockerfile_path, &tag, temp_dir.path())
        .await
        .map_err(|e| {
            remember_failed_image_build(&tag, &e);
            e.to_string()
        })
}

/// The tag of the image of `shared` layers, by their text: the same layers
/// always get the same tag.
pub(crate) fn shared_layers_tag(shared: &str) -> String {
    format!(
        "{}shared-{:x}",
        COMBINED_IMAGE_PREFIX,
        fnv1a_hash(shared.as_bytes())
    )
}

/// The start of a combined image's Dockerfile, through its first `RUN`:
/// the packages every image on the same base installs.
pub(crate) fn shared_layers(dockerfile: &str) -> &str {
    let Some(run) = dockerfile.find("\nRUN ") else {
        return dockerfile;
    };
    let mut end = run + 1;
    for line in dockerfile[end..].split_inclusive('\n') {
        end += line.len();
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }
    &dockerfile[..end]
}

/// Builds images for jobs that haven't reached their turn while `work`
/// runs, and stops when `work` is over. A job waits for room in the VM
/// before its steps run, and used to start building its image only then:
/// the build is done by the time its turn comes instead.
pub(crate) async fn with_images_built_ahead<T>(
    ahead: impl std::future::Future<Output = ()>,
    work: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(ahead);
    tokio::pin!(work);
    match future::select(work, ahead).await {
        future::Either::Left((out, _)) => out,
        future::Either::Right(((), work)) => work.await,
    }
}

/// How many images are built ahead at once, across every job of every run
/// in this process: a workflow of twenty jobs would otherwise start twenty
/// builds, and an engine that busy is slow to start the steps of the jobs
/// running. A job whose turn has come builds its own whatever this is.
pub(crate) const IMAGES_BUILT_AHEAD: usize = 4;

pub(crate) static AHEAD_BUILDS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(IMAGES_BUILT_AHEAD);

/// The share of [`AHEAD_BUILDS`] one build ahead takes. A job's image used
/// to be built only once the job had its place in the VM's memory pool; a
/// build ahead has none. Where the run limits its jobs, memory is short and
/// the room the VM keeps for itself, which builds are counted in, holds one
/// build: a build ahead then takes every permit and goes alone.
pub(crate) fn ahead_build_permits(jobs_limited: bool) -> u32 {
    if jobs_limited {
        IMAGES_BUILT_AHEAD as u32
    } else {
        1
    }
}

/// Whether a build error is the connection to the engine failing rather
/// than the build: the stream of build output ended early or was cut. The
/// same build may well work on a second try.
///
/// Only the error's first line counts. What follows it is the build's own
/// output, where a failing `RUN` may well print "connection reset", and
/// that build would fail the same way again.
pub(crate) fn is_transport_error(message: &str) -> bool {
    let message = message
        .lines()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    [
        "bytes remaining on stream",
        "connection reset",
        "connection closed",
        "broken pipe",
        "unexpected eof",
        "error reading a body",
        "incomplete message",
    ]
    .iter()
    .any(|sign| message.contains(sign))
}

/// Build `tag`, telling a build that failed from one whose report of
/// success was lost. A tag is only applied when a build succeeds, and
/// callers build only a tag that doesn't exist yet, so finding it after an
/// error means it built. An error in the connection to the engine is tried
/// once more; any other is the build's own and is returned.
pub(crate) async fn build_image_checked(
    runtime: &dyn ContainerRuntime,
    dockerfile: &Path,
    tag: &str,
    context_dir: &Path,
) -> Result<(), wrkflw_runtime::container::ContainerError> {
    let first = match runtime.build_image(dockerfile, tag, context_dir).await {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if runtime.image_exists(tag).await.unwrap_or(false) {
        wrkflw_logging::warning(&format!(
            "Building {} reported an error, but the image was built, so it is used: {}",
            tag, first
        ));
        return Ok(());
    }
    if !is_transport_error(&first.to_string()) {
        return Err(first);
    }
    wrkflw_logging::warning(&format!(
        "Building {} lost its connection to the container engine ({}); trying once more",
        tag, first
    ));
    runtime.build_image(dockerfile, tag, context_dir).await
}
