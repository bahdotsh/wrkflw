//! What a step sees inside a real container, through the real binary. These
//! need Docker and are skipped when it doesn't answer, so they run where CI
//! has it and on machines that do.
#![cfg(unix)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// Stands in for the user's git config, so a global hook can't change what
/// these tests see.
const NO_GIT_CONFIG: &str = "/dev/null";

/// The PATH of the Alpine image the fixtures run in.
const ALPINE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

fn docker_answers() -> bool {
    Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// A throwaway repository holding `files`, committed.
fn repo(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    for args in [
        &["init", "-q"][..],
        &["add", "."],
        &["commit", "-qm", "init"],
    ] {
        let status = Command::new("git")
            .current_dir(dir.path())
            .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
    dir
}

fn run(root: &Path, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wrkflw"))
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", NO_GIT_CONFIG)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("WRKFLW_NO_HISTORY", "1")
        .args(["run", ".github/workflows/ci.yml", "--runtime", "docker"])
        .args(flags)
        .output()
        .unwrap()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A `$GITHUB_PATH` entry goes in front of the image's PATH, never the
/// host's, which would hide the image's tools from every later step.
#[test]
fn a_github_path_entry_extends_the_containers_own_path() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let workflow = format!(
        r#"
name: ci
on: push
jobs:
  path:
    runs-on: ubuntu-latest
    container: alpine:3.20
    defaults:
      run:
        shell: sh
    steps:
      - run: echo /opt/extra >> "$GITHUB_PATH"
      - run: test "$PATH" = "/opt/extra:{}"
"#,
        ALPINE_PATH
    );
    let dir = repo(&[(".github/workflows/ci.yml", &workflow)]);
    assert_ok(&run(dir.path(), &[]));
    assert_ok(&run(dir.path(), &["--step-containers"]));
}

/// A composite action's steps run in the job container, so what they leave
/// outside the workspace, such as an installed tool, is there for the steps
/// after it, as on GitHub.
#[test]
fn a_composite_actions_changes_stay_in_the_job_container() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let action = r#"
name: install
runs:
  using: composite
  steps:
    - shell: sh
      run: |
        test -f "$GITHUB_ACTION_PATH/tool.sh"
        cp "$GITHUB_ACTION_PATH/tool.sh" /usr/local/bin/mytool
        chmod +x /usr/local/bin/mytool
"#;
    let workflow = r#"
name: ci
on: push
jobs:
  tool:
    runs-on: ubuntu-latest
    container: alpine:3.20
    steps:
      - uses: actions/checkout@v4
      - uses: ./.github/actions/install
      - shell: sh
        run: test "$(mytool)" = tool-ok
"#;
    let dir = repo(&[
        (".github/workflows/ci.yml", workflow),
        (".github/actions/install/action.yml", action),
        (
            ".github/actions/install/tool.sh",
            "#!/bin/sh\necho tool-ok\n",
        ),
    ]);
    assert_ok(&run(dir.path(), &[]));
}

/// What a step installs outside the workspace is there for the next step,
/// as on GitHub, where a job's steps share a machine. With a container per
/// step it is gone, which is what `--step-containers` asks for.
#[test]
fn what_a_step_installs_is_there_for_the_next() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let workflow = r#"
name: ci
on: push
jobs:
  tool:
    runs-on: ubuntu-latest
    container: alpine:3.20
    steps:
      - shell: sh
        run: |
          printf '#!/bin/sh\necho tool-ok\n' > /usr/local/bin/mytool
          chmod +x /usr/local/bin/mytool
          echo kept > "$HOME/kept"
      - shell: sh
        run: |
          test "$(mytool)" = tool-ok
          test "$(cat "$HOME/kept")" = kept
"#;
    let dir = repo(&[(".github/workflows/ci.yml", workflow)]);
    assert_ok(&run(dir.path(), &[]));
    // Still accepted, now that it is the default.
    assert_ok(&run(dir.path(), &["--job-container"]));
    let apart = run(dir.path(), &["--step-containers"]);
    assert!(
        !apart.status.success(),
        "a step in a container of its own found the last step's files:\n{}",
        String::from_utf8_lossy(&apart.stdout)
    );
}
