//! `wrkflw doctor`: what a run on this machine would use, and what would
//! make it slower or less faithful than it could be, before a run finds out.
//!
//! Every check is read-only: it pulls nothing, builds nothing and starts
//! no container.

use serde_json::{json, Value};
use std::path::Path;
use wrkflw_executor::runner_base::BaseImage;
use wrkflw_executor::vm_memory::Memory;
use wrkflw_ui::cli_style;

/// Below this much memory a VM fits too few jobs at once to run a matrix
/// in parallel, and suites that build in memory run out of it.
const LOW_VM_MEMORY: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
    fix: Option<String>,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Check {
            name,
            status: Status::Ok,
            detail: detail.into(),
            fix: None,
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Check {
            name,
            status: Status::Warn,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, fix: impl Into<String>) -> Self {
        Check {
            status: Status::Fail,
            ..Check::warn(name, detail, fix)
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "status": self.status.as_str(),
            "detail": self.detail,
            "fix": self.fix,
        })
    }
}

/// Which container engines answer, as `--runtime auto` probes them.
#[derive(Debug, Clone, Copy)]
struct Engines {
    docker: bool,
    podman: bool,
}

impl Engines {
    /// The CLI of the engine `auto` picks.
    fn picked(self) -> Option<&'static str> {
        if self.docker {
            Some("docker")
        } else if self.podman {
            Some("podman")
        } else {
            None
        }
    }
}

fn runtime_check(engines: Engines) -> Check {
    match (engines.docker, engines.podman) {
        (true, _) => Check::ok("runtime", "Docker answers: runs use it"),
        (false, true) => Check::warn(
            "runtime",
            "Docker isn't answering, so runs use Podman",
            "Start Docker if you meant to use it; a Podman machine usually has less memory, \
             and timings measured on Docker won't compare",
        ),
        (false, false) => Check::warn(
            "runtime",
            "Neither Docker nor Podman answers, so runs fall back to emulation, which runs \
             steps on this machine with no container",
            "Start Docker or Podman",
        ),
    }
}

fn memory_check(memory: Option<&Memory>) -> Option<Check> {
    let memory = memory?;
    let engine = if memory.cli == "podman" {
        "Podman"
    } else {
        "Docker"
    };
    let place = if memory.in_vm { "VM" } else { "host" };
    // In GiB, as the run's own "Docker VM 7.7 GB" line counts it.
    let size = format!("{:.1} GB", memory.bytes as f64 / (1u64 << 30) as f64);
    Some(if memory.in_vm && memory.bytes < LOW_VM_MEMORY {
        Check::warn(
            "memory",
            format!("The {engine} {place} has {size}: few jobs fit in it at once"),
            format!("Give the {engine} VM more memory (8 GB runs most matrices in parallel)"),
        )
    } else {
        Check::ok(
            "memory",
            format!("The {engine} {place} has {size} for jobs"),
        )
    })
}

fn runner_image_check(status: &BaseImage, present: Option<bool>) -> Check {
    match status {
        BaseImage::Used(image) => match present {
            Some(true) => Check::ok("runner_image", format!("{image} is here")),
            _ => Check::ok(
                "runner_image",
                format!("{image} is pulled on the first run, instead of built here"),
            ),
        },
        BaseImage::RecentlyUnavailable(image) => Check::warn(
            "runner_image",
            format!(
                "{image} couldn't be pulled lately, so job images are built here, which is slower"
            ),
            "It is tried again within a day. Check the network, or name another base with \
             WRKFLW_RUNNER_IMAGE",
        ),
        BaseImage::Disabled => Check::warn(
            "runner_image",
            "WRKFLW_PREBUILT_IMAGES=0: every new job image is built here from ubuntu:latest",
            "Unset WRKFLW_PREBUILT_IMAGES to start from the published runner base",
        ),
    }
}

fn token_check(source: Option<&str>) -> Check {
    match source {
        Some(source) => Check::ok(
            "github_token",
            format!(
                "Found in {source}: used by compare, for private actions and called \
                 workflows, and by jobs only with WRKFLW_JOB_GITHUB_TOKEN=1"
            ),
        ),
        None => Check::warn(
            "github_token",
            "No GitHub token: compare can't read runs, and actions or called workflows in \
             private repositories can't be cloned",
            "Set GITHUB_TOKEN, or sign in with `gh auth login`",
        ),
    }
}

fn git_check(version: Option<String>) -> Check {
    match version {
        Some(version) => Check::ok("git", version),
        None => Check::fail(
            "git",
            "git isn't installed: checkout, change detection and gate need it",
            "Install git",
        ),
    }
}

/// The workflow files in `dir`, by where they were found.
fn workflows_check(dir: &Path) -> Check {
    let count = |sub: &str| {
        std::fs::read_dir(dir.join(sub))
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| {
                        let path = e.path();
                        matches!(
                            path.extension().and_then(|x| x.to_str()),
                            Some("yml" | "yaml")
                        )
                    })
                    .count()
            })
            .unwrap_or(0)
    };
    let mut found = Vec::new();
    for sub in [".github/workflows", ".gitea/workflows"] {
        let n = count(sub);
        if n > 0 {
            found.push(format!("{n} in {sub}"));
        }
    }
    if dir.join(".gitlab-ci.yml").is_file() {
        found.push(".gitlab-ci.yml".to_string());
    }
    if found.is_empty() {
        Check::warn(
            "workflows",
            "No workflows here",
            "Run wrkflw from a repository's root, or pass a workflow's path",
        )
    } else {
        Check::ok("workflows", found.join(", "))
    }
}

fn macos_check(engines: Engines) -> Option<Check> {
    if !cfg!(target_os = "macos") || engines.picked().is_none() {
        return None;
    }
    Some(if wrkflw_executor::run_options::macos_on_host() {
        Check::ok(
            "macos_jobs",
            "macos-* jobs run on this Mac itself (--macos-on-host)",
        )
    } else {
        Check::ok(
            "macos_jobs",
            "macos-* jobs run as Linux and are reported unverified; --macos-on-host runs \
             them on this Mac",
        )
    })
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn gather() -> Vec<Check> {
    let probe = tokio::task::spawn_blocking(|| {
        let engines = Engines {
            docker: wrkflw_executor::docker::is_available(),
            podman: wrkflw_executor::podman::is_available(),
        };
        let memory = engines.picked().and_then(wrkflw_executor::vm_memory::probe);
        let base = wrkflw_executor::runner_base::status();
        let present = match (&base, engines.picked()) {
            (BaseImage::Used(image), Some(cli)) => Some(
                command_output(cli, &["image", "inspect", "--format", "{{.Id}}", image]).is_some(),
            ),
            _ => None,
        };
        let git = command_output("git", &["--version"]);
        (engines, memory, base, present, git)
    });
    let token = wrkflw_executor::run_options::github_token_source();
    let (probe, token) = tokio::join!(probe, token);
    let Ok((engines, memory, base, present, git)) = probe else {
        return vec![Check::fail(
            "runtime",
            "The checks themselves failed to run",
            "Run again with --debug",
        )];
    };
    let mut checks = vec![runtime_check(engines)];
    checks.extend(memory_check(memory.as_ref()));
    if engines.picked().is_some() {
        checks.push(runner_image_check(&base, present));
    }
    checks.extend(macos_check(engines));
    checks.push(token_check(token));
    checks.push(git_check(git));
    let cwd = std::env::current_dir().unwrap_or_default();
    checks.push(workflows_check(&cwd));
    checks
}

/// Run every check and print them. Returns whether none failed.
pub(crate) async fn run(json_out: bool) -> bool {
    let checks = gather().await;
    let ok = checks.iter().all(|c| c.status != Status::Fail);
    if json_out {
        wrkflw_runs::emit_line(&json!({
            "schema_version": wrkflw_runs::SCHEMA_VERSION,
            "type": "doctor",
            "ok": ok,
            "checks": checks.iter().map(Check::to_json).collect::<Vec<_>>(),
        }));
        return ok;
    }
    let width = checks.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for check in &checks {
        let name = format!("{:width$}", check.name);
        let line = format!("{name}  {}", check.detail);
        let line = match check.status {
            Status::Ok => cli_style::success(&line),
            Status::Warn => cli_style::warning(&line),
            Status::Fail => cli_style::error(&line),
        };
        println!("{line}");
        if let Some(fix) = &check.fix {
            println!("  {}  {}", " ".repeat(width), cli_style::dim(fix));
        }
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_docker_with_podman_up_is_a_warning_that_names_podman() {
        let check = runtime_check(Engines {
            docker: false,
            podman: true,
        });
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("Podman"), "{}", check.detail);
        let none = runtime_check(Engines {
            docker: false,
            podman: false,
        });
        assert!(none.detail.contains("emulation"), "{}", none.detail);
    }

    #[test]
    fn a_small_vm_is_a_warning_and_a_linux_host_never_is() {
        let vm = |bytes| Memory {
            cli: "docker".to_string(),
            bytes,
            in_vm: true,
        };
        assert_eq!(
            memory_check(Some(&vm(2_000_000_000))).unwrap().status,
            Status::Warn
        );
        assert_eq!(
            memory_check(Some(&vm(8_000_000_000))).unwrap().status,
            Status::Ok
        );
        let host = Memory {
            in_vm: false,
            ..vm(2_000_000_000)
        };
        assert_eq!(memory_check(Some(&host)).unwrap().status, Status::Ok);
        assert!(memory_check(None).is_none());
    }

    #[test]
    fn the_runner_image_says_whether_a_first_run_pulls_or_builds() {
        let used = BaseImage::Used("img:1".to_string());
        assert!(runner_image_check(&used, Some(false))
            .detail
            .contains("pulled"));
        assert!(runner_image_check(&used, Some(true))
            .detail
            .contains("is here"));
        let refused = BaseImage::RecentlyUnavailable("img:1".to_string());
        assert_eq!(runner_image_check(&refused, None).status, Status::Warn);
        assert_eq!(
            runner_image_check(&BaseImage::Disabled, None).status,
            Status::Warn
        );
    }

    #[test]
    fn a_token_is_named_by_where_it_was_found_never_shown() {
        let found = token_check(Some("GH_TOKEN"));
        assert_eq!(found.status, Status::Ok);
        assert!(found.detail.contains("GH_TOKEN"));
        assert_eq!(token_check(None).status, Status::Warn);
    }

    #[test]
    fn workflows_are_counted_where_wrkflw_looks_for_them() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(workflows_check(dir.path()).status, Status::Warn);
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "on: push").unwrap();
        std::fs::write(dir.path().join(".github/workflows/notes.md"), "").unwrap();
        std::fs::write(dir.path().join(".gitlab-ci.yml"), "").unwrap();
        let check = workflows_check(dir.path());
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.detail, "1 in .github/workflows, .gitlab-ci.yml");
    }

    #[test]
    fn only_a_missing_git_fails() {
        assert_eq!(git_check(None).status, Status::Fail);
        assert_eq!(
            git_check(Some("git version 2.50".into())).status,
            Status::Ok
        );
    }
}
