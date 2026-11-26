//! JavaScript actions (`runs.using: node<N>`), run as GitHub's runner runs
//! them: `node <main>` in the job's container, from the action's own
//! directory, with its inputs as `INPUT_<NAME>`, its `pre` right before
//! `main` (GitHub runs it at the start of the job) and its `post` at the
//! end of the job, which reads what `main` saved with `$GITHUB_STATE` or
//! `::save-state` as `STATE_<name>`.
//!
//! The engine glue is in `engine.rs` (`execute_node_action` and the post
//! loops); this module holds what that needs and can be tested alone.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Env var that, set to `0`, leaves JavaScript actions unrun: the step
/// passes and the action is reported as not run, as before wrkflw ran them.
pub const JS_ACTIONS_ENV: &str = "WRKFLW_JS_ACTIONS";

/// Why a JavaScript action didn't run when `node` wasn't there to run it.
pub(crate) const NO_NODE: &str =
    "the job's image has no `node` to run this JavaScript action (use a runner image with Node, or -P)";

/// Why a JavaScript action didn't run when [`JS_ACTIONS_ENV`] is `0`.
pub(crate) const JS_ACTIONS_OFF: &str = "JavaScript actions are turned off (WRKFLW_JS_ACTIONS=0)";

/// Whether JavaScript actions run: on unless [`JS_ACTIONS_ENV`] turns them
/// off.
pub(crate) fn enabled() -> bool {
    crate::run_options::env_switch(JS_ACTIONS_ENV, true)
}

/// What `runs:` says about a JavaScript action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NodeRuns {
    /// The Node major version `runs.using` names: 20 for `node20`.
    pub node_major: u32,
    /// The script `main` names, relative to the action's directory.
    pub main: String,
    pub pre: Option<String>,
    /// `pre-if`, which defaults to `always()` on GitHub.
    pub pre_if: Option<String>,
    pub post: Option<String>,
    /// `post-if`, which defaults to `always()` on GitHub.
    pub post_if: Option<String>,
}

impl NodeRuns {
    /// The `runs:` of a JavaScript action's `action.yml`, or `None` when
    /// it isn't one or names no `main`.
    pub(crate) fn from_definition(definition: &serde_yaml::Value) -> Option<Self> {
        let runs = definition.get("runs")?;
        let using = runs.get("using")?.as_str()?;
        let node_major = using.strip_prefix("node")?.parse().ok()?;
        let text = |key: &str| {
            runs.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Some(Self {
            node_major,
            main: text("main")?,
            pre: text("pre"),
            pre_if: text("pre-if"),
            post: text("post"),
            post_if: text("post-if"),
        })
    }
}

/// The env var `core.getInput(name)` reads: upper case, spaces as `_`,
/// hyphens kept.
pub(crate) fn input_env_name(name: &str) -> String {
    format!("INPUT_{}", name.replace(' ', "_").to_uppercase())
}

/// The env var `core.getState(name)` reads in `post`.
pub(crate) fn state_env_name(name: &str) -> String {
    format!("STATE_{}", name)
}

/// The inputs a JavaScript action gets, by env var: each input the action
/// declares, from `with` or else its `default`, and anything else `with`
/// passes, as the runner passes it too. `evaluate` resolves `${{ }}` in a
/// default, such as `${{ github.token }}`; `with` arrives evaluated.
///
/// Input names are matched without regard to case, as on GitHub: `with:
/// Token:` gives the declared `token`, and each `INPUT_` variable is set
/// once.
pub(crate) fn input_env(
    definition: &serde_yaml::Value,
    with: Option<&HashMap<String, String>>,
    evaluate: impl Fn(&str) -> String,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // `with` by the variable each entry sets. Two keys that differ only in
    // case set the same one; the key that sorts last wins, so the choice
    // doesn't depend on the map's order.
    let mut given: Vec<(String, &String, &String)> = with
        .into_iter()
        .flatten()
        .map(|(name, value)| (input_env_name(name), name, value))
        .collect();
    given.sort();
    let given: HashMap<String, &String> = given
        .into_iter()
        .map(|(var, _, value)| (var, value))
        .collect();
    if let Some(inputs) = definition.get("inputs").and_then(|v| v.as_mapping()) {
        for (name, spec) in inputs {
            let Some(name) = name.as_str() else { continue };
            let var = input_env_name(name);
            if !seen.insert(var.clone()) {
                continue;
            }
            let value = match given.get(&var) {
                Some(given) => (*given).clone(),
                None => match spec.get("default") {
                    Some(serde_yaml::Value::String(s)) => evaluate(s),
                    Some(serde_yaml::Value::Number(n)) => n.to_string(),
                    Some(serde_yaml::Value::Bool(b)) => b.to_string(),
                    _ => String::new(),
                },
            };
            env.push((var, value));
        }
    }
    let mut extra: Vec<_> = given
        .into_iter()
        .filter(|(var, _)| !seen.contains(var))
        .collect();
    extra.sort();
    for (var, value) in extra {
        env.push((var, value.clone()));
    }
    env
}

/// `actions/github-script`'s token input, as its code reads it.
pub(crate) const GITHUB_SCRIPT_TOKEN_INPUT: &str = "INPUT_GITHUB-TOKEN";

/// What `actions/github-script` gets for its token when the job has none.
/// Not a token: the API refuses it, as it would refuse a bad one.
pub(crate) const STAND_IN_TOKEN: &str = "wrkflw-no-github-token";

/// Whether `actions/github-script` would stop before running its script.
/// It refuses an empty `github-token`, which defaults to the token every
/// GitHub job has and a local job lacks unless asked: actions such as
/// `codecov/codecov-action` call it with a script that never reaches the
/// API, and failed only for that. A stand-in lets the script run; one that
/// calls the API then fails there, as without a token it would anyway.
pub(crate) fn needs_stand_in_token(repository: &str, env: &HashMap<String, String>) -> bool {
    repository.eq_ignore_ascii_case("actions/github-script")
        && env
            .get(GITHUB_SCRIPT_TOKEN_INPUT)
            .is_none_or(|token| token.trim().is_empty())
}

/// The `bash` command that runs `script` from the action's directory, for
/// an action that asks for Node `major`. The script's path is quoted, so
/// whatever its name holds reaches `node` as written. When the image's
/// Node is older than the action asks for, the step warns first, as a
/// failure may then come from the older Node: a newer one is fine, as
/// GitHub's runner too runs older actions on a newer Node.
pub(crate) fn node_command(script: &str, major: u32) -> String {
    let script = script.trim_start_matches("./").replace('\'', r"'\''");
    format!(
        "command -v node >/dev/null 2>&1 || {{ echo '{NODE_MISSING}' >&2; exit 127; }}; \
         v=$(node -p 'process.versions.node.split(\".\")[0]' 2>/dev/null) && [ \"$v\" -lt {major} ] && \
         echo \"::warning::This action asks for node{major}, and the job's image has Node $v: \
         a failure may come from the older Node\"; \
         node \"$GITHUB_ACTION_PATH\"/'{script}'"
    )
}

/// What [`node_command`] prints when there is no `node` to run, so a
/// missing `node` is told from a script that fails, in any locale: bash's
/// own "command not found" is translated.
const NODE_MISSING: &str = "wrkflw: no node on PATH to run this JavaScript action";

/// Why a JavaScript action didn't run when the image had no `bash` to
/// start `node` with.
pub(crate) const NO_BASH: &str =
    "the job's image has no `bash` to start this JavaScript action's `node` with";

/// Why a phase that exited with `exit_code` and printed `output` couldn't
/// start: no `node`, or no `bash` to run it with. `None` when it ran.
pub(crate) fn missing_program(exit_code: Option<i32>, output: &str) -> Option<&'static str> {
    if !matches!(exit_code, Some(126 | 127)) {
        return None;
    }
    if output.contains("\"bash\": executable file not found")
        || output.contains("exec: bash: not found")
        || output.contains("bash: no such file or directory")
    {
        return Some(NO_BASH);
    }
    (exit_code == Some(127) && output.contains(NODE_MISSING)).then_some(NO_NODE)
}

/// What an action saved for its `post`: `$GITHUB_STATE`'s entries, then
/// `::save-state` lines in its output, a later one winning.
pub(crate) fn saved_state(state_file: &str, output: &str) -> Vec<(String, String)> {
    let mut state: Vec<(String, String)> =
        crate::github_env_files::parse_github_kv_file(state_file)
            .into_iter()
            .collect();
    state.sort();
    for command in crate::workflow_commands::parse_workflow_commands(output) {
        if let crate::workflow_commands::WorkflowCommand::SaveState { name, value } = command {
            state.retain(|(k, _)| *k != name);
            state.push((name, value));
        }
    }
    state
}

/// A JavaScript action's `post`, run at the end of the job.
#[derive(Clone)]
pub(crate) struct PostStep {
    /// The step's name, as GitHub shows it: `Post <step>`.
    pub name: String,
    /// The `bash` command that runs it.
    pub command: String,
    /// `post-if`, `always()` when the action gives none.
    pub condition: String,
    /// What the action's step had beyond the job's env when `main` ran
    /// (its `env:`, the action's inputs and paths) and the `STATE_*` that
    /// `main` saved. The post runs with these over the job's env as it is
    /// at the end of the job, as on GitHub.
    pub own_env: HashMap<String, String>,
    /// Where the action's repository is: see [`with_action_root`].
    pub action_root: Option<ActionRoot>,
    /// Holds a cloned action's directory, so it outlives the step that
    /// cloned it until its post has run. Held, never read.
    #[allow(dead_code)]
    pub keep_action: Option<Arc<tempfile::TempDir>>,
    /// Set when the action turned out not to run at all, so neither does
    /// its post.
    pub withdrawn: bool,
}

/// Where the repository of a JavaScript action is, so a container can see
/// all of it and a script can reach out of the action's own directory
/// (`main: ../dist/x.js`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionRoot {
    /// A remote action's clone, whose root a container mounts.
    Repository(PathBuf),
    /// The job's workspace, canonical, which holds a local action: a
    /// container sees it at its workspace path already, so nothing is
    /// mounted or copied, and a later step's changes there are seen.
    Workspace(PathBuf),
}

tokio::task_local! {
    /// Where the repository of the JavaScript action whose script is
    /// running is, set by wrkflw around that script alone.
    static ACTION_ROOT: Option<ActionRoot>;
}

/// Run `future`, one of a JavaScript action's scripts, so a container sees
/// the whole repository at `root`. Carried beside the step rather than in
/// its env, which the action's `env:` and `$GITHUB_ENV` can write: an
/// action naming its own root could have any host directory mounted.
pub(crate) fn with_action_root<F: std::future::Future>(
    root: Option<ActionRoot>,
    future: F,
) -> tokio::task::futures::TaskLocalFuture<Option<ActionRoot>, F> {
    ACTION_ROOT.scope(root, future)
}

/// [`with_action_root`] for synchronous code, in tests.
#[cfg(test)]
pub(crate) fn sync_with_action_root<T>(root: Option<ActionRoot>, f: impl FnOnce() -> T) -> T {
    ACTION_ROOT.sync_scope(root, f)
}

/// The root [`with_action_root`] set for the script running, if any.
pub(crate) fn action_root() -> Option<ActionRoot> {
    ACTION_ROOT.try_with(Clone::clone).ok().flatten()
}

/// What a job keeps of the actions its steps run: their posts, in the
/// order their actions ran, and the job's copy of each cloned action.
#[derive(Default)]
pub(crate) struct JobActions {
    posts: Mutex<Vec<PostStep>>,
    /// By `repo@ref`. As on GitHub's runner, which downloads an action
    /// once per job, the job's steps share one copy, and what a step
    /// writes beside the action (`npm ci` in its directory) the job's
    /// later steps see; other jobs and matrix legs have copies of their
    /// own.
    copies: Mutex<HashMap<String, Arc<tempfile::TempDir>>>,
}

/// The [`JobActions`] of a job, shared by its steps.
pub(crate) type JobScope = Arc<JobActions>;

tokio::task_local! {
    /// Set around each step of a job by its step loop.
    static JOB_ACTIONS: JobScope;
}

/// Run `future`, a step of a job, so what its actions leave for the job
/// (posts, copies) goes in `job`.
///
/// A plain `fn` returning the scope's future: an `async fn` would hold
/// `future` and the scope that wraps it, twice the size of a step.
pub(crate) fn within_job<F: std::future::Future>(
    job: &JobScope,
    future: F,
) -> tokio::task::futures::TaskLocalFuture<JobScope, F> {
    JOB_ACTIONS.scope(job.clone(), future)
}

/// The job's copy of the action `key` names, if a step of the job made one.
pub(crate) fn job_copy(key: &str) -> Option<Arc<tempfile::TempDir>> {
    JOB_ACTIONS
        .try_with(|job| {
            job.copies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(key)
                .cloned()
        })
        .ok()
        .flatten()
}

/// Keep `copy` as the job's copy of the action `key` names, and return the
/// job's copy: `copy`, or one kept first. Outside a job, `copy` itself.
pub(crate) fn keep_job_copy(key: &str, copy: Arc<tempfile::TempDir>) -> Arc<tempfile::TempDir> {
    JOB_ACTIONS
        .try_with(|job| {
            job.copies
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.to_string())
                .or_insert_with(|| copy.clone())
                .clone()
        })
        .unwrap_or(copy)
}

/// Queue `post` for the end of the job, and return where it is in the
/// queue for [`update_post`]. As on GitHub's runner, a post is queued
/// before its action's `main` runs, so it runs at the end of the job also
/// when `main` failed or ran out of time. Outside a step loop, as in a
/// test calling a step on its own, there is no end of job to run it at: it
/// is dropped with a warning.
pub(crate) fn queue_post(post: PostStep) -> Option<usize> {
    let queued = JOB_ACTIONS.try_with(|job| {
        let mut posts = job.posts.lock().unwrap_or_else(|e| e.into_inner());
        posts.push(post.clone());
        posts.len() - 1
    });
    if queued.is_err() {
        wrkflw_logging::warning(&format!(
            "'{}' has nowhere to run: it isn't part of a job",
            post.name
        ));
    }
    queued.ok()
}

/// Change the post [`queue_post`] put at `index`, once its `main` is done:
/// what `main` saved for it, or that it is withdrawn.
pub(crate) fn update_post(index: Option<usize>, change: impl FnOnce(&mut PostStep)) {
    let Some(index) = index else { return };
    let _ = JOB_ACTIONS.try_with(|job| {
        if let Some(post) = job
            .posts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(index)
        {
            change(post);
        }
    });
}

/// The posts queued so far and not withdrawn, last first, as GitHub runs
/// them.
pub(crate) fn take_posts(job: &JobScope) -> Vec<PostStep> {
    let mut taken = std::mem::take(&mut *job.posts.lock().unwrap_or_else(|e| e.into_inner()));
    taken.retain(|post| !post.withdrawn);
    taken.reverse();
    taken
}

/// The `run:` step that runs one of an action's scripts. It names the
/// workspace as its working directory, so a `defaults.run.working-directory`
/// doesn't move it: on GitHub those defaults are for `run:` steps, and an
/// action starts in `$GITHUB_WORKSPACE`.
pub(crate) fn script_step(name: String, command: String) -> wrkflw_parser::workflow::Step {
    let mut step = wrkflw_parser::workflow::Step::with_run(name, command);
    step.shell = Some("bash".to_string());
    step.working_directory = Some(".".to_string());
    step
}

/// A post as the `workflow::Step` that runs it.
pub(crate) fn post_as_step(post: &PostStep) -> wrkflw_parser::workflow::Step {
    let mut step = script_step(post.name.clone(), post.command.clone());
    step.if_condition = Some(post.condition.clone());
    step
}

/// Repositories of actions cloned this run, by `repo@ref`, so a matrix's
/// legs and repeated steps clone each once. Kept until the run ends.
/// Each key has its own cell, so a slow clone holds up only the steps
/// waiting for that same action.
#[derive(Default)]
pub(crate) struct ActionClones {
    clones: Mutex<HashMap<String, Arc<CloneCell>>>,
}

type CloneCell = tokio::sync::OnceCell<Arc<tempfile::TempDir>>;

impl ActionClones {
    /// The clone of `key`, made with `clone` into the directory it is
    /// given the first time it's asked for. A failed clone isn't kept, so
    /// the next step that asks tries again.
    pub(crate) async fn get_or_clone<F, Fut>(
        &self,
        key: &str,
        clone: F,
    ) -> Result<Arc<tempfile::TempDir>, String>
    where
        F: FnOnce(PathBuf) -> Fut,
        Fut: std::future::Future<Output = Result<(), String>>,
    {
        let cell = self
            .clones
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.to_string())
            .or_default()
            .clone();
        cell.get_or_try_init(|| async {
            let dir =
                tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
            clone(dir.path().join("action")).await?;
            Ok(Arc::new(dir))
        })
        .await
        .cloned()
    }
}

/// The directory of a cloned action, its repository's root or the
/// subdirectory `sub_path` names, which must stay inside it; and that
/// root. Both resolved, so the directory is a path inside the root.
pub(crate) fn action_dir_in(
    clone: &Path,
    sub_path: Option<&str>,
) -> Result<(PathBuf, PathBuf), String> {
    let root = clone.join("action");
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("Failed to resolve {}: {}", root.display(), e))?;
    let Some(sub) = sub_path.filter(|s| !s.is_empty()) else {
        return Ok((canonical_root.clone(), canonical_root));
    };
    let canonical = root
        .join(sub)
        .canonicalize()
        .map_err(|e| format!("The action's path '{}' doesn't exist: {}", sub, e))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!("The action's path '{}' leaves its repository", sub));
    }
    Ok((canonical, canonical_root))
}

/// A copy of the action cloned in `clone` for one job to run, so jobs
/// that run at once never share it: an action may write beside its own
/// files (`npm ci` in its directory), and under emulation or a mounted
/// directory two matrix legs would then write into one clone. The copy
/// leaves out `.git`, as GitHub's runner gets an action as a tarball.
pub(crate) fn private_copy(clone: &Path) -> Result<Arc<tempfile::TempDir>, String> {
    let dir = tempfile::tempdir().map_err(|e| format!("Failed to create temp dir: {}", e))?;
    let from = clone.join("action");
    let root = from
        .canonicalize()
        .map_err(|e| format!("Failed to resolve {}: {}", from.display(), e))?;
    copy_tree(&from, &dir.path().join("action"), &root, true)
        .map_err(|e| format!("Failed to copy the action for its step: {}", e))?;
    Ok(Arc::new(dir))
}

/// Copy the directory `from` to `to`, symlinks as symlinks, without the
/// top-level `.git` when `top`. `root` is the canonical directory being
/// copied, which a symlink copied as its target must stay inside.
fn copy_tree(from: &Path, to: &Path, root: &Path, top: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if top && name == ".git" {
            continue;
        }
        let (source, target) = (entry.path(), to.join(&name));
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)?;
            // Windows needs a privilege to make symlinks: a link to a file
            // in the repository is copied as that file, and any other is
            // left out. A link the repository holds to a file elsewhere on
            // this machine, such as a key, is never followed.
            #[cfg(not(unix))]
            if link_stays_in(&source, root) {
                std::fs::copy(&source, &target)?;
            }
            #[cfg(unix)]
            let _ = root;
        } else if kind.is_dir() {
            copy_tree(&source, &target, root, false)?;
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// Whether the symlink `link` points at a file inside `root`, which is
/// canonical.
#[cfg_attr(unix, allow(dead_code))]
fn link_stays_in(link: &Path, root: &Path) -> bool {
    link.canonicalize()
        .is_ok_and(|target| target.starts_with(root) && target.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_repository_is_not_followed() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("id_rsa"), "key").unwrap();
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path().canonicalize().unwrap();
        std::fs::write(root.join("index.js"), "main").unwrap();
        std::os::unix::fs::symlink(root.join("index.js"), root.join("inside")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("id_rsa"), root.join("outside")).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), root.join("dangling")).unwrap();
        assert!(link_stays_in(&root.join("inside"), &root));
        assert!(!link_stays_in(&root.join("outside"), &root));
        assert!(!link_stays_in(&root.join("dangling"), &root));
    }

    fn yaml(text: &str) -> serde_yaml::Value {
        serde_yaml::from_str(text).unwrap()
    }

    #[test]
    fn runs_names_the_node_version_and_its_scripts() {
        let runs = NodeRuns::from_definition(&yaml(
            "runs:\n  using: node20\n  main: dist/index.js\n  post: dist/post.js\n  post-if: success()\n",
        ))
        .unwrap();
        assert_eq!(runs.node_major, 20);
        assert_eq!(runs.main, "dist/index.js");
        assert_eq!(runs.post.as_deref(), Some("dist/post.js"));
        assert_eq!(runs.post_if.as_deref(), Some("success()"));
        assert_eq!(runs.pre, None);
    }

    #[test]
    fn a_composite_or_docker_action_has_no_node_runs() {
        assert!(NodeRuns::from_definition(&yaml("runs:\n  using: composite\n")).is_none());
        assert!(NodeRuns::from_definition(&yaml("runs:\n  using: docker\n  image: x\n")).is_none());
        assert!(NodeRuns::from_definition(&yaml("runs:\n  using: node20\n")).is_none());
    }

    #[test]
    fn inputs_keep_hyphens_and_take_defaults() {
        let def = yaml(
            "inputs:\n  my-input:\n    default: d\n  token:\n    default: ${{ github.token }}\n  count:\n    default: 3\n  given:\n    default: no\n",
        );
        let with = HashMap::from([
            ("given".to_string(), "yes".to_string()),
            ("extra value".to_string(), "e".to_string()),
        ]);
        let env = input_env(&def, Some(&with), |s| {
            s.replace("${{ github.token }}", "tok")
        });
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("INPUT_MY-INPUT"), Some("d"));
        assert_eq!(get("INPUT_TOKEN"), Some("tok"));
        assert_eq!(get("INPUT_COUNT"), Some("3"));
        assert_eq!(get("INPUT_GIVEN"), Some("yes"));
        assert_eq!(get("INPUT_EXTRA_VALUE"), Some("e"));
    }

    #[test]
    fn inputs_are_matched_without_regard_to_case() {
        let def = yaml("inputs:\n  token:\n    default: d\n");
        let with = HashMap::from([
            ("Token".to_string(), "given".to_string()),
            ("Extra".to_string(), "a".to_string()),
            ("EXTRA".to_string(), "b".to_string()),
        ]);
        let env = input_env(&def, Some(&with), str::to_string);
        assert_eq!(
            env,
            vec![
                ("INPUT_TOKEN".to_string(), "given".to_string()),
                ("INPUT_EXTRA".to_string(), "a".to_string()),
            ],
            "each variable once, the declared input from `with`"
        );
    }

    #[test]
    fn state_comes_from_the_file_and_save_state_lines() {
        let state = saved_state(
            "a=1\nb=2\n",
            "hello\n::save-state name=b::3\n::save-state name=c::4\n",
        );
        assert_eq!(
            state,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "3".to_string()),
                ("c".to_string(), "4".to_string())
            ]
        );
    }

    #[test]
    fn a_missing_node_or_bash_is_told_from_a_failing_script() {
        assert_eq!(missing_program(Some(127), NODE_MISSING), Some(NO_NODE));
        assert_eq!(missing_program(Some(1), NODE_MISSING), None);
        assert_eq!(
            missing_program(Some(127), "bash: node: Befehl nicht gefunden"),
            None,
            "only wrkflw's own probe says node is missing"
        );
        assert_eq!(
            missing_program(Some(127), "Error: Cannot find module"),
            None
        );
        assert_eq!(
            missing_program(
                Some(127),
                "OCI runtime exec failed: exec failed: unable to start container process: \
                 exec: \"bash\": executable file not found in $PATH"
            ),
            Some(NO_BASH)
        );
    }

    #[test]
    fn a_scripts_path_reaches_node_as_written() {
        assert!(node_command("./dist/index.js", 20)
            .ends_with(r#"node "$GITHUB_ACTION_PATH"/'dist/index.js'"#));
        assert!(node_command("../lib/$x \"a'b\".js", 20)
            .ends_with(r#"node "$GITHUB_ACTION_PATH"/'../lib/$x "a'\''b".js'"#));
    }

    /// Only a Node older than the action asks for warns, as GitHub's runner
    /// also runs older actions on a newer Node.
    #[test]
    fn an_older_node_than_the_action_asks_for_warns() {
        let command = node_command("index.js", 24);
        assert!(command.contains(r#"[ "$v" -lt 24 ]"#), "{command}");
        assert!(command.contains("::warning::This action asks for node24"));
        // The command runs in a Linux container or on a Unix host; Git
        // Bash on Windows can't take a Windows path in GITHUB_ACTION_PATH.
        if cfg!(windows)
            || std::process::Command::new("node")
                .arg("--version")
                .output()
                .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.js"), "console.log('ran')").unwrap();
        let run = |major: u32| {
            let out = std::process::Command::new("bash")
                .arg("-c")
                .arg(node_command("index.js", major))
                .env("GITHUB_ACTION_PATH", dir.path())
                .output()
                .unwrap();
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(run(1), "ran\n");
        let newer = run(999);
        assert!(
            newer.starts_with("::warning::This action asks for node999"),
            "{newer}"
        );
        assert!(newer.ends_with("ran\n"), "{newer}");

        // By its path: the child's PATH, which has no `node`, has no
        // `bash` either.
        let no_node = std::process::Command::new("/bin/bash")
            .arg("-c")
            .arg(node_command("index.js", 20))
            .env("GITHUB_ACTION_PATH", dir.path())
            .env("PATH", "/nonexistent")
            .env("LANG", "de_DE.UTF-8")
            .output()
            .unwrap();
        let printed = String::from_utf8_lossy(&no_node.stderr).into_owned();
        assert_eq!(
            missing_program(no_node.status.code(), &printed),
            Some(NO_NODE),
            "{printed}"
        );
    }

    #[test]
    fn a_withdrawn_post_does_not_run() {
        let posts: JobScope = Arc::default();
        let post = |name: &str| PostStep {
            name: name.to_string(),
            command: String::new(),
            condition: "always()".to_string(),
            own_env: HashMap::new(),
            action_root: None,
            keep_action: None,
            withdrawn: false,
        };
        let (kept, dropped) = tokio_test_scope(&posts, || {
            let kept = queue_post(post("kept"));
            let dropped = queue_post(post("dropped"));
            update_post(dropped, |p| p.withdrawn = true);
            update_post(kept, |p| {
                p.own_env.insert("STATE_x".to_string(), "1".to_string());
            });
            (kept, dropped)
        });
        assert_eq!((kept, dropped), (Some(0), Some(1)));
        let taken = take_posts(&posts);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].name, "kept");
        assert_eq!(
            taken[0].own_env.get("STATE_x").map(String::as_str),
            Some("1")
        );
    }

    /// Run `f` as a step of a job whose posts go on `posts`.
    fn tokio_test_scope<T>(posts: &JobScope, f: impl FnOnce() -> T) -> T {
        JOB_ACTIONS.sync_scope(posts.clone(), f)
    }

    #[test]
    fn posts_run_last_first() {
        let posts: JobScope = Arc::default();
        let post = |name: &str| PostStep {
            name: name.to_string(),
            command: String::new(),
            condition: "always()".to_string(),
            own_env: HashMap::new(),
            action_root: None,
            keep_action: None,
            withdrawn: false,
        };
        posts.posts.lock().unwrap().push(post("first"));
        posts.posts.lock().unwrap().push(post("second"));
        let names: Vec<_> = take_posts(&posts).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["second", "first"]);
        assert!(take_posts(&posts).is_empty());
    }

    #[tokio::test]
    async fn a_slow_clone_holds_up_only_its_own_action() {
        let clones = Arc::new(ActionClones::default());
        let (release, wait) = tokio::sync::oneshot::channel::<()>();
        let slow = {
            let clones = clones.clone();
            tokio::spawn(async move {
                clones
                    .get_or_clone("slow/r@v1", |dir| async move {
                        let _ = wait.await;
                        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())
                    })
                    .await
            })
        };
        tokio::task::yield_now().await;
        let fast = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            clones.get_or_clone("fast/r@v1", |dir| async move {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())
            }),
        )
        .await
        .expect("another action's clone waited on the slow one");
        assert!(fast.is_ok());
        release.send(()).unwrap();
        assert!(slow.await.unwrap().is_ok());
    }

    #[tokio::test]
    async fn a_failed_clone_is_tried_again() {
        let clones = ActionClones::default();
        let first = clones
            .get_or_clone("o/r@v1", |_| async { Err("offline".to_string()) })
            .await;
        assert!(first.is_err());
        let second = clones
            .get_or_clone("o/r@v1", |dir| async move {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())
            })
            .await;
        assert!(second.is_ok());
    }

    #[tokio::test]
    async fn an_action_is_cloned_once_per_key() {
        let clones = ActionClones::default();
        let count = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..3 {
            clones
                .get_or_clone("o/r@v1", |dir| {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async move { std::fs::create_dir_all(&dir).map_err(|e| e.to_string()) }
                })
                .await
                .unwrap();
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_sub_path_must_stay_in_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("action/sub")).unwrap();
        let root = dir.path().join("action").canonicalize().unwrap();
        assert_eq!(
            action_dir_in(dir.path(), Some("sub")).unwrap(),
            (root.join("sub"), root.clone())
        );
        assert!(action_dir_in(dir.path(), Some("../..")).is_err());
        assert_eq!(
            action_dir_in(dir.path(), None).unwrap(),
            (root.clone(), root)
        );
    }

    #[test]
    fn a_jobs_steps_share_one_copy_and_other_jobs_have_their_own() {
        let dir = |name: &str| {
            let d = tempfile::tempdir().unwrap();
            std::fs::write(d.path().join("from"), name).unwrap();
            Arc::new(d)
        };
        let job: JobScope = Arc::default();
        let (first, second, other_key) = tokio_test_scope(&job, || {
            assert!(job_copy("o/a@v1").is_none());
            let first = keep_job_copy("o/a@v1", dir("first"));
            let second = keep_job_copy("o/a@v1", dir("second"));
            assert!(Arc::ptr_eq(&job_copy("o/a@v1").unwrap(), &first));
            (first, second, keep_job_copy("o/b@v1", dir("b")))
        });
        assert!(Arc::ptr_eq(&first, &second), "the copy kept first wins");
        assert!(!Arc::ptr_eq(&first, &other_key));

        let other_job: JobScope = Arc::default();
        tokio_test_scope(&other_job, || assert!(job_copy("o/a@v1").is_none()));
        assert!(
            job_copy("o/a@v1").is_none(),
            "outside a job nothing is kept"
        );
    }

    #[test]
    fn each_job_gets_its_own_copy_without_git() {
        let clone = tempfile::tempdir().unwrap();
        let root = clone.path().join("action");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(root.join(".git/HEAD"), "ref").unwrap();
        std::fs::write(root.join("dist/index.js"), "main").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("dist/index.js", root.join("index.js")).unwrap();

        let a = private_copy(clone.path()).unwrap();
        let b = private_copy(clone.path()).unwrap();
        assert_ne!(a.path(), b.path());
        let copied = a.path().join("action");
        assert_eq!(
            std::fs::read_to_string(copied.join("dist/index.js")).unwrap(),
            "main"
        );
        assert!(!copied.join(".git").exists());
        #[cfg(unix)]
        assert!(copied.join("index.js").is_symlink());

        // What one job writes there, the other doesn't see.
        std::fs::write(copied.join("node_modules"), "x").unwrap();
        assert!(!b.path().join("action/node_modules").exists());
    }

    #[test]
    fn github_script_gets_a_stand_in_only_without_a_token() {
        let with = |token: &str| {
            HashMap::from([(GITHUB_SCRIPT_TOKEN_INPUT.to_string(), token.to_string())])
        };
        assert!(needs_stand_in_token("actions/github-script", &with("")));
        assert!(needs_stand_in_token(
            "Actions/GitHub-Script",
            &HashMap::new()
        ));
        assert!(!needs_stand_in_token(
            "actions/github-script",
            &with("ghs_real")
        ));
        assert!(!needs_stand_in_token("someone/other-action", &with("")));
    }
}
