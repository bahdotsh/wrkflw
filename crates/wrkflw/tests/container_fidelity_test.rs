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

/// One wrkflw run at a time: on a cold runner, two processes building the
/// same job image at once both ran into the 10-minute build limit.
static ONE_RUN_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn run(root: &Path, flags: &[&str]) -> Output {
    let _turn = ONE_RUN_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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

/// What a composite action's steps write to `$GITHUB_PATH` and
/// `$GITHUB_ENV` applies to the rest of the job, as it does on GitHub:
/// installers such as `taiki-e/install-action` depend on it.
#[test]
fn a_composite_actions_path_and_env_reach_the_job() {
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
        mkdir -p /opt/tool/bin
        printf '#!/bin/sh\necho tool-ok\n' > /opt/tool/bin/mytool
        chmod +x /opt/tool/bin/mytool
        echo /opt/tool/bin >> "$GITHUB_PATH"
    - shell: sh
      run: |
        echo TOOL_FLAVOUR=plain >> "$GITHUB_ENV"
        printf 'TOOL_NOTES<<EOF\nline one\nline two\nEOF\n' >> "$GITHUB_ENV"
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
        run: |
          test "$(mytool)" = tool-ok
          test "$TOOL_FLAVOUR" = plain
          test "$(printf '%s' "$TOOL_NOTES" | wc -l)" -eq 1
"#;
    let dir = repo(&[
        (".github/workflows/ci.yml", workflow),
        (".github/actions/install/action.yml", action),
    ]);
    assert_ok(&run(dir.path(), &[]));
}

/// A composite action that fails still hands on what its earlier steps
/// wrote to `$GITHUB_PATH`, as on GitHub, where each of its steps' writes
/// apply to the job as soon as that step ends.
#[test]
fn a_failed_composite_actions_path_still_reaches_the_job() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let action = r#"
name: install then fail
runs:
  using: composite
  steps:
    - shell: sh
      run: |
        mkdir -p /opt/tool/bin
        printf '#!/bin/sh\necho tool-ok\n' > /opt/tool/bin/mytool
        chmod +x /opt/tool/bin/mytool
        echo /opt/tool/bin >> "$GITHUB_PATH"
    - shell: sh
      run: exit 1
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
        continue-on-error: true
      - shell: sh
        run: test "$(mytool)" = tool-ok
"#;
    let dir = repo(&[
        (".github/workflows/ci.yml", workflow),
        (".github/actions/install/action.yml", action),
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

/// With `--docker-in-jobs` a step's `docker` reaches an engine inside the
/// job's container, so `-v $(pwd):/wrk` names the job's workspace, not a
/// path on this machine. Without it the run says why those steps fail.
#[test]
fn a_job_that_runs_docker_gets_an_engine_of_its_own_when_asked() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let workflow = r#"
name: ci
on: push
jobs:
  engine:
    runs-on: ubuntu-latest
    steps:
      - run: echo marker-ok > marker.txt
      - run: test "$(docker run --rm -v "$(pwd):/wrk" alpine:3.20 cat /wrk/marker.txt)" = marker-ok
"#;
    let dir = repo(&[(".github/workflows/ci.yml", workflow)]);
    let with_engine = run(dir.path(), &["--docker-in-jobs"]);
    assert_ok(&with_engine);
    let said = String::from_utf8_lossy(&with_engine.stdout).to_string()
        + &String::from_utf8_lossy(&with_engine.stderr);
    assert!(
        said.contains("privileged"),
        "no word of a privileged container:\n{said}"
    );

    let without = run(dir.path(), &[]);
    assert!(
        !without.status.success(),
        "a step ran `docker` with no engine"
    );
    let said = String::from_utf8_lossy(&without.stdout).to_string()
        + &String::from_utf8_lossy(&without.stderr);
    assert!(
        said.contains("--docker-in-jobs"),
        "no word of the flag:\n{said}"
    );
}

/// A JavaScript action runs with the job image's node, as the runner user,
/// in the job's container: what it writes to `$GITHUB_ENV` reaches the next
/// step, and its post reads what it saved with `$GITHUB_STATE`.
#[test]
fn a_javascript_action_runs_in_the_job_container_with_its_post() {
    if !docker_answers() {
        eprintln!("skipped: Docker isn't available");
        return;
    }
    let action = r#"
name: js
inputs:
  file-name:
    default: made-by-js
runs:
  using: node20
  main: main.js
  post: post.js
"#;
    let main = r#"const fs = require("fs");
fs.writeFileSync(process.env.GITHUB_WORKSPACE + "/" + process.env["INPUT_FILE-NAME"], require("os").userInfo().username);
fs.appendFileSync(process.env.GITHUB_ENV, "FROM_JS=yes\n");
fs.appendFileSync(process.env.GITHUB_STATE, "saved=from-main\n");
"#;
    let post = r#"if (process.env.STATE_saved !== "from-main") { console.error("no state"); process.exit(1); }
console.log("post ok");
"#;
    let workflow = r#"
name: ci
on: push
jobs:
  js:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: ./.github/actions/js
      - run: |
          test "$FROM_JS" = yes
          test "$(cat made-by-js)" = runner
"#;
    let dir = repo(&[
        (".github/workflows/ci.yml", workflow),
        (".github/actions/js/action.yml", action),
        (".github/actions/js/main.js", main),
        (".github/actions/js/post.js", post),
    ]);
    let output = run(dir.path(), &[]);
    assert_ok(&output);
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("Post "), "{text}");
}
