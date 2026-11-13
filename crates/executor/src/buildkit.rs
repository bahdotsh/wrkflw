//! Building a job's runtime image with BuildKit, through the `docker` CLI,
//! where that builds into the engine wrkflw talks to.

use bollard::Docker;
use std::path::Path;
use wrkflw_runtime::container::ContainerError;

/// Chooses the builder for a job's runtime image: `classic` keeps to the
/// engine's own builder.
const IMAGE_BUILDER_ENV: &str = "WRKFLW_IMAGE_BUILDER";

fn buildkit_wanted(setting: Option<&str>) -> bool {
    setting.map(str::trim) != Some("classic")
}

/// Set once BuildKit built an image this engine couldn't find: the `docker`
/// CLI uses another engine, and BuildKit isn't used again in this process.
static BUILDKIT_IMAGES_UNSEEN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether a job's runtime image is built with BuildKit: unless it was
/// asked not to be, built an image elsewhere before, or can't build into
/// this engine.
pub(crate) async fn builds_here(docker: &Docker) -> bool {
    buildkit_wanted(std::env::var(IMAGE_BUILDER_ENV).ok().as_deref())
        && !BUILDKIT_IMAGES_UNSEEN.load(std::sync::atomic::Ordering::Relaxed)
        && buildkit_available(docker).await
}

/// Whether the `docker` CLI builds with BuildKit into this engine, asked
/// once a process.
async fn buildkit_available(docker: &Docker) -> bool {
    static AVAILABLE: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
    *AVAILABLE.get_or_init(|| buildkit_builds_here(docker)).await
}

/// Whether the `docker` CLI's builder builds into this engine's own images.
///
/// Its driver must be `docker`: another, such as `docker-container`, keeps
/// its own cache and takes a base image from the registry rather than the
/// one this engine has, which the image's tag names. And the CLI must talk
/// to this engine, or each image would be built in the other and then again
/// here. Where the engines can't be told apart, BuildKit is used, and the
/// check after the first build still catches an image built elsewhere.
async fn buildkit_builds_here(docker: &Docker) -> bool {
    let Some(inspect) = docker_cli(&["buildx", "inspect"]).await else {
        return false;
    };
    let driver = buildx_driver(&inspect);
    if driver != Some("docker") {
        wrkflw_logging::debug(&format!(
            "The `docker` CLI's builder uses the {} driver, not docker, so job images are \
             built by the engine's own builder",
            driver.unwrap_or("unknown")
        ));
        return false;
    }
    let cli_engine = docker_cli(&["info", "--format", "{{.ID}}"]).await;
    let api_engine = docker.info().await.ok().and_then(|info| info.id);
    if same_engine(cli_engine.as_deref(), api_engine.as_deref()) == Some(false) {
        warn_buildkit_elsewhere("The `docker` CLI talks to another engine than wrkflw does");
        return false;
    }
    true
}

/// Say once why BuildKit isn't used, and what to do about it.
fn warn_buildkit_elsewhere(why: &str) {
    wrkflw_logging::warning(&format!(
        "{}, as when it uses another context; job images are built by the engine's own \
         builder, which is slower. Point the CLI at the same engine, or set {}=classic to \
         skip BuildKit",
        why, IMAGE_BUILDER_ENV
    ));
}

/// What `docker <args>` printed, when it ran in time and succeeded.
async fn docker_cli(args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("docker")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The driver `docker buildx inspect` names for the builder in use.
fn buildx_driver(inspect: &str) -> Option<&str> {
    inspect
        .lines()
        .find_map(|line| line.strip_prefix("Driver:"))
        .map(str::trim)
        .filter(|driver| !driver.is_empty())
}

/// Whether the CLI's engine and the API's are one, by their IDs. `None`
/// when either didn't give one.
fn same_engine(cli: Option<&str>, api: Option<&str>) -> Option<bool> {
    let cli = cli.map(str::trim).filter(|id| !id.is_empty())?;
    let api = api.map(str::trim).filter(|id| !id.is_empty())?;
    Some(cli == api)
}

/// What is left to do for an image after BuildKit was asked to build it.
#[derive(Debug)]
pub(crate) enum Next {
    /// The engine has the image.
    Built,
    /// BuildKit didn't give the image; the engine's own builder may.
    EngineBuilds,
    /// The build failed, and the engine's own builder would fail the same.
    Failed(ContainerError),
}

/// Build `tag` with BuildKit, and say what is left to do for it.
/// `image_exists` asks the engine for the image, once BuildKit says it
/// built it.
pub(crate) async fn build(
    dockerfile: &Path,
    tag: &str,
    context_dir: &Path,
    image_exists: impl std::future::Future<Output = Result<bool, ContainerError>>,
) -> Next {
    match build_with_buildkit(dockerfile, tag, context_dir).await {
        Ok(()) => after_build(tag, image_exists.await, &BUILDKIT_IMAGES_UNSEEN),
        Err(e) => after_failure(tag, e),
    }
}

/// What to do once BuildKit built `tag`, by whether the engine has it.
fn after_build(
    tag: &str,
    engine_has_it: Result<bool, ContainerError>,
    unseen: &std::sync::atomic::AtomicBool,
) -> Next {
    match engine_has_it {
        Ok(true) => Next::Built,
        // The `docker` CLI talks to another engine than this one, though
        // the check before the first build couldn't tell. Every BuildKit
        // build would be done twice, so this one and the rest go to the
        // engine's own builder.
        Ok(false) => {
            if !unseen.swap(true, std::sync::atomic::Ordering::Relaxed) {
                warn_buildkit_elsewhere(&format!(
                    "BuildKit built {} where this engine can't see it",
                    tag
                ));
            }
            Next::EngineBuilds
        }
        // The engine didn't answer; it says nothing of where the image
        // went. The caller looks for it again before it takes this for a
        // failed build.
        Err(e) => Next::Failed(ContainerError::ImageBuild(format!(
            "BuildKit built {}, but the engine couldn't be asked for it: {}",
            tag, e
        ))),
    }
}

/// What to do once BuildKit failed to build `tag`. A build whose own
/// instruction failed is reported as it is: the engine's builder would
/// fail the same way, and take as long. Any other failure may be
/// BuildKit's alone, and the engine's builder reports as it always did.
fn after_failure(tag: &str, error: BuildKitError) -> Next {
    match error {
        BuildKitError::StepFailed(e) => Next::Failed(ContainerError::ImageBuild(e)),
        BuildKitError::NotBuilt(e) => {
            wrkflw_logging::debug(&format!(
                "BuildKit couldn't build {}, so the engine's own builder is tried: {}",
                tag, e
            ));
            Next::EngineBuilds
        }
    }
}

/// Build `tag` with BuildKit through the `docker` CLI.
///
/// The engine's own builder, which the API reaches, saves a layer after
/// every instruction: on a runtime image that took about as long as the
/// installs themselves, and ten seconds for an image that only adds an
/// interpreter to layers it already had. BuildKit saves once, at the end,
/// and builds that share a step run it once between them.
async fn build_with_buildkit(
    dockerfile: &Path,
    tag: &str,
    context_dir: &Path,
) -> Result<(), BuildKitError> {
    let output = tokio::process::Command::new("docker")
        .args([
            "buildx",
            "build",
            "--progress=plain",
            "--provenance=false",
            "--load",
            "-t",
            tag,
            "-f",
        ])
        .arg(dockerfile)
        .arg(context_dir)
        .stdin(std::process::Stdio::null())
        // The build is given up with its future, as on a timeout.
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| {
            BuildKitError::NotBuilt(format!("`docker buildx build` didn't start: {}", e))
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = stderr
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let report = |what: &str| {
        format!(
            "`docker buildx build` failed ({}):\n{}",
            output.status, what
        )
    };
    if an_instruction_failed(&stderr) {
        // What the failed instruction printed last, where the cause is,
        // rather than the end of the output, which is the Dockerfile.
        let what = failed_instruction_output(&stderr)
            .unwrap_or_else(|| lines[lines.len().saturating_sub(20)..].join("\n"));
        Err(BuildKitError::StepFailed(report(&what)))
    } else {
        Err(BuildKitError::NotBuilt(report(
            &lines[lines.len().saturating_sub(20)..].join("\n"),
        )))
    }
}

/// The last lines the failed instruction printed, then its error, from
/// BuildKit's plain progress, where each of a step's lines starts with the
/// step's number: `#6 0.195 E: ...`, then `#6 ERROR: process ...`.
fn failed_instruction_output(stderr: &str) -> Option<String> {
    let (step, error) = stderr.lines().find_map(|line| {
        let (step, rest) = line.split_once(' ')?;
        let error = rest.strip_prefix("ERROR: ")?;
        (step.starts_with('#') && an_instruction_failed(line)).then_some((step, error))
    })?;
    let printed: Vec<&str> = stderr
        .lines()
        .filter_map(|line| {
            let (number, rest) = line.split_once(' ')?;
            let (time, text) = rest.split_once(' ').unwrap_or((rest, ""));
            (number == step && time.parse::<f64>().is_ok()).then_some(text)
        })
        .collect();
    let mut report = printed[printed.len().saturating_sub(20)..].join("\n");
    if !report.is_empty() {
        report.push('\n');
    }
    report.push_str(error);
    Some(report)
}

/// Why BuildKit didn't give a job's runtime image.
#[derive(Debug)]
enum BuildKitError {
    /// An instruction of the Dockerfile failed, such as an install that
    /// exited non-zero. The engine's own builder would fail the same way.
    StepFailed(String),
    /// BuildKit itself couldn't build, as when the CLI has no builder or
    /// can't reach one; the engine's own builder may well.
    NotBuilt(String),
}

/// Whether BuildKit's output says an instruction of the Dockerfile failed,
/// rather than BuildKit itself: `process "/bin/sh -c ..." did not complete
/// successfully`, or `executor failed running [...]` from older releases.
/// Only an error line counts, not what the instruction itself printed.
fn an_instruction_failed(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        line.to_ascii_lowercase().contains("error")
            && ((line.contains("process \"") && line.contains("did not complete successfully"))
                || line.contains("executor failed running ["))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed instruction is the build's own failure, which the engine's
    /// builder would repeat; anything else may be BuildKit's alone.
    #[test]
    fn a_failed_instruction_is_told_from_buildkit_failing() {
        for failed in [
            "#7 ERROR: process \"/bin/sh -c apt-get install -y wget\" did not complete successfully: exit code: 100",
            "ERROR: failed to build: failed to solve: process \"/bin/sh -c false\" did not complete successfully: exit code: 1",
            "error: failed to solve: executor failed running [/bin/sh -c false]: exit code: 1",
        ] {
            assert!(an_instruction_failed(failed), "{failed}");
        }
        for not_the_build in [
            "ERROR: no builder \"wrkflw\" found",
            "ERROR: failed to solve: ubuntu:latest: failed to resolve source metadata",
            "error: unknown flag: --provenance",
            // What an instruction printed is not an error line of BuildKit's.
            "#7 0.312 process \"x\" did not complete successfully",
        ] {
            assert!(!an_instruction_failed(not_the_build), "{not_the_build}");
        }
    }

    /// The cause is in what the instruction printed last; the end of
    /// BuildKit's output is the Dockerfile, which says nothing of it.
    #[test]
    fn a_failed_instruction_is_reported_by_what_it_printed() {
        let stderr = "#5 [2/3] RUN echo one\n\
                      #5 0.170 one\n\
                      #5 DONE 0.2s\n\
                      \n\
                      #6 [3/3] RUN apt-get install -y wget\n\
                      #6 0.195 Reading package lists...\n\
                      #6 0.201 E: Unable to locate package wget\n\
                      #6 ERROR: process \"/bin/sh -c apt-get install -y wget\" did not complete successfully: exit code: 100\n\
                      ------\n\
                      Dockerfile:3\n\
                      \x20  3 | >>> RUN apt-get install -y wget\n\
                      ERROR: failed to build: failed to solve: process \"/bin/sh -c apt-get install -y wget\" did not complete successfully: exit code: 100\n";
        assert_eq!(
            failed_instruction_output(stderr).unwrap(),
            "Reading package lists...\n\
             E: Unable to locate package wget\n\
             process \"/bin/sh -c apt-get install -y wget\" did not complete successfully: exit code: 100"
        );
        // Without step numbers there is nothing to pick out.
        assert_eq!(
            failed_instruction_output(
                "error: failed to solve: executor failed running [/bin/sh -c false]: exit code: 1"
            ),
            None
        );
    }

    /// Only a builder of the `docker` driver builds into the engine's own
    /// images, from the base image the engine has.
    #[test]
    fn the_builders_driver_is_read_from_buildx_inspect() {
        let inspect = "Name:          desktop-linux\n\
                       Driver:        docker\n\
                       Last Activity: 2026-11-13 09:12:38 +0000 UTC\n\
                       \n\
                       Nodes:\n\
                       Name:             desktop-linux\n";
        assert_eq!(buildx_driver(inspect), Some("docker"));
        assert_eq!(
            buildx_driver("Name: x\nDriver: docker-container\n"),
            Some("docker-container")
        );
        assert_eq!(buildx_driver("Name: x\nDriver:\n"), None);
        assert_eq!(buildx_driver(""), None);
    }

    /// The CLI and the API reach one engine when they give the same ID; an
    /// ID missing on either side tells nothing.
    #[test]
    fn engines_are_told_apart_by_their_ids() {
        let id = "d154b3c8-fd5c-48fb-b028-ee89de535bd5";
        assert_eq!(same_engine(Some(&format!("{id}\n")), Some(id)), Some(true));
        assert_eq!(same_engine(Some("other"), Some(id)), Some(false));
        assert_eq!(same_engine(None, Some(id)), None);
        assert_eq!(same_engine(Some(id), Some("")), None);
    }

    #[test]
    fn the_engines_builder_can_be_asked_for() {
        assert!(buildkit_wanted(None));
        assert!(buildkit_wanted(Some("buildkit")));
        assert!(!buildkit_wanted(Some(" classic ")));
    }

    /// BuildKit said it built the image. The engine has it, or it went to
    /// another engine, where every later build would go too, so BuildKit
    /// is given up on for the process and the user is told once.
    #[test]
    fn an_image_built_where_the_engine_cant_see_it_ends_buildkit() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let unseen = AtomicBool::new(false);
        assert!(matches!(after_build("t", Ok(true), &unseen), Next::Built));
        assert!(!unseen.load(Ordering::Relaxed));
        assert!(matches!(
            after_build("t", Ok(false), &unseen),
            Next::EngineBuilds
        ));
        assert!(unseen.load(Ordering::Relaxed));
        // An engine that didn't answer says nothing of where the image is.
        let unseen = AtomicBool::new(false);
        let unanswered = Err(ContainerError::ContainerExecution("no answer".to_string()));
        match after_build("t", unanswered, &unseen) {
            Next::Failed(e) => assert!(e.to_string().contains("no answer"), "{e}"),
            other => panic!("{other:?}"),
        }
        assert!(!unseen.load(Ordering::Relaxed));
    }

    /// A failed instruction fails the build; BuildKit failing leaves the
    /// image to the engine's own builder.
    #[test]
    fn only_a_failed_instruction_fails_the_build() {
        match after_failure("t", BuildKitError::StepFailed("exit code: 100".to_string())) {
            Next::Failed(e) => assert!(e.to_string().contains("exit code: 100"), "{e}"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            after_failure("t", BuildKitError::NotBuilt("no builder".to_string())),
            Next::EngineBuilds
        ));
    }
}
