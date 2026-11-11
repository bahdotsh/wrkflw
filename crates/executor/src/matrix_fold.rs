//! Folding matrix legs that would only repeat another leg.
//!
//! Locally every runner label maps to one Linux image, so legs of
//! `runs-on: ${{ matrix.os }}` would do identical work. A leg that differs
//! from an earlier one only in values `runs-on` reads, and runs in the same
//! image, reports that leg's result instead of running.
//! `run_options::every_matrix_leg` turns this off.

use crate::engine::{get_effective_runner_image, JobResult, JobStatus};
use serde_yaml::Value;
use std::collections::HashMap;
use wrkflw_matrix::MatrixCombination;
use wrkflw_parser::workflow::Job;

/// For each matrix leg, the earlier leg it would only repeat, if any.
///
/// Two legs repeat each other when they run in the same image and every
/// matrix value on which they differ is read by nothing but `runs-on`, and
/// at least one of them picks the runner. That is the common
/// `runs-on: ${{ matrix.os }}` case: locally every OS runs in the same Linux
/// image, so the legs would do identical work. A key that only labels a leg,
/// such as one the job's display name uses, changes nothing that runs. Legs
/// on the same runner are left alone even when nothing reads the key that
/// tells them apart, since a matrix like `attempt: [1, 2, 3]` exists to
/// repeat a job.
/// Any reference to the matrix that can't be pinned to one key counts as
/// reading all of them, so a leg is only folded away when that is safe. In
/// `runs-on` such a reference picks the runner from keys nobody can name,
/// so nothing is folded.
pub(crate) fn matrix_twins(
    job: &Job,
    combinations: &[MatrixCombination],
    platform_overrides: &HashMap<String, String>,
) -> Vec<Option<usize>> {
    let mut twins = vec![None; combinations.len()];
    if combinations.len() < 2 || crate::run_options::every_matrix_leg() {
        return twins;
    }
    let Some(body) = job_body_text(job) else {
        return twins;
    };
    let runs_on = job
        .runs_on
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let runner_keys = match matrix_reads(&runs_on) {
        Reads::Keys(keys) => keys,
        Reads::Any => return twins,
    };
    let images: Vec<String> = combinations
        .iter()
        .map(|c| get_effective_runner_image(job, platform_overrides, Some(&c.values)))
        .collect();
    let picks_runner = |key: &String| runner_keys.contains(key);
    for leg in 1..combinations.len() {
        // The legs this one would repeat, and whether each differs from it
        // in nothing but the runner.
        let candidates: Vec<(usize, bool)> = (0..leg)
            .filter(|&i| twins[i].is_none() && images[i] == images[leg])
            .filter_map(|i| {
                let differing = differing_keys(&combinations[i].values, &combinations[leg].values);
                (differing.iter().any(picks_runner) && !reads_matrix_keys(&body, &differing))
                    .then(|| (i, differing.iter().all(picks_runner)))
            })
            .collect();
        // A leg that differs only in the runner is this leg's exact twin.
        // One that also differs in an unread label stands in only when there
        // is none, so each leg of `os` x `attempt` reports its own attempt.
        twins[leg] = candidates
            .iter()
            .find(|(_, exact)| *exact)
            .or(candidates.first())
            .map(|(i, _)| *i);
    }
    twins
}

/// The job's text, minus the two places a matrix value may appear without
/// changing what the job does: `runs-on`, and the matrix itself. The job's
/// display name isn't parsed, so it is never here. Step names are, so a
/// step named after a matrix value keeps its leg, although the name changes
/// nothing that runs: the fold errs toward running a leg.
///
/// Every string in the job, and every key, one per line as written: not
/// the job serialised, where a newline in an `if: |` block is the letters
/// `\n` and would hide the `matrix` that starts the next line.
fn job_body_text(job: &Job) -> Option<String> {
    let mut value = serde_json::to_value(job).ok()?;
    let map = value.as_object_mut()?;
    map.remove("runs-on");
    map.remove("strategy");
    let mut text = String::new();
    collect_strings(&value, &mut text);
    Some(text)
}

fn collect_strings(value: &serde_json::Value, text: &mut String) {
    match value {
        serde_json::Value::String(s) => {
            text.push_str(s);
            text.push('\n');
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_strings(item, text);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                text.push_str(key);
                text.push('\n');
                collect_strings(item, text);
            }
        }
        // Numbers, booleans and null hold no expression.
        _ => {}
    }
}

/// Matrix keys whose values differ between two legs, lowercased.
fn differing_keys(a: &HashMap<String, Value>, b: &HashMap<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = a
        .keys()
        .chain(b.keys())
        .filter(|key| a.get(*key) != b.get(*key))
        .map(|key| key.to_lowercase())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// What a piece of workflow text reads from the matrix.
#[derive(Debug, PartialEq)]
enum Reads {
    /// These keys, lowercased, and no others.
    Keys(Vec<String>),
    /// Something that can't be pinned to keys: `matrix[...]`,
    /// `toJSON(matrix)`, `matrix.*`, a bare `matrix`, or the `strategy`
    /// context, whose `job-index` differs in every leg.
    Any,
}

/// The matrix keys `text` reads. Expressions are case-insensitive, and may
/// put spaces around the `.`.
fn matrix_reads(text: &str) -> Reads {
    let lower = text.to_lowercase();
    // `strategy.job-index` and `job-total` tell every leg apart.
    if mentions(&lower, "strategy").any(|rest| {
        let rest = rest.trim_start();
        rest.starts_with('.') || rest.starts_with('[')
    }) {
        return Reads::Any;
    }
    let mut keys = Vec::new();
    for rest in mentions(&lower, "matrix") {
        let Some(rest) = rest.trim_start().strip_prefix('.') else {
            return Reads::Any;
        };
        let key: String = rest
            .trim_start()
            .chars()
            .take_while(|&c| is_word(c))
            .collect();
        if key.is_empty() {
            // `matrix.*` or something unreadable.
            return Reads::Any;
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Reads::Keys(keys)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// What follows each mention of the context `name` in `lower` as a whole
/// word: not `my_matrix` or `matrix_os`.
fn mentions<'a>(lower: &'a str, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    lower.match_indices(name).filter_map(move |(pos, _)| {
        let before = lower[..pos].chars().next_back();
        let rest = &lower[pos + name.len()..];
        (!before.is_some_and(is_word) && !rest.chars().next().is_some_and(is_word)).then_some(rest)
    })
}

/// Whether `body` may read any of `keys` from the matrix, through an
/// expression or `MATRIX_<KEY>`/`MATRIX_CONTEXT`, the variables wrkflw
/// sets for each leg.
fn reads_matrix_keys(body: &str, keys: &[String]) -> bool {
    if body.contains("MATRIX_CONTEXT")
        || keys
            .iter()
            .any(|key| body.contains(&format!("MATRIX_{}", key.to_uppercase())))
    {
        return true;
    }
    match matrix_reads(body) {
        Reads::Any => true,
        Reads::Keys(read) => keys.iter().any(|key| read.contains(key)),
    }
}

/// The result of a leg that wasn't run because it would only repeat
/// `original`, reported under its own name.
pub(crate) fn repeated_leg(
    name: &str,
    original: &JobResult,
    image: &str,
    runner: Option<&crate::runner_fidelity::RunnerSubstitution>,
) -> JobResult {
    let why = format!(
        "since both run in {} and the job reads nothing else that differs. Pass \
         --every-matrix-leg to run it anyway.",
        image
    );
    let note = match original.status {
        JobStatus::Skipped => format!(
            "Not run: {} was skipped, and this leg would repeat it exactly, {}",
            original.name, why
        ),
        _ => format!(
            "Not run: it would repeat {} exactly, {}",
            original.name, why
        ),
    };
    crate::events::emit(|| crate::events::ExecutionEvent::JobStarted {
        job: name.to_string(),
        canonical_name: original.canonical_name.clone(),
    });
    crate::runner_fidelity::announce(name, runner);
    // This leg's result is the original's, and so is what that left unrun.
    for step in &original.steps {
        for uses in &step.actions_not_run {
            crate::events::emit(|| crate::events::ExecutionEvent::ActionNotRun {
                job: name.to_string(),
                step: step.name.clone(),
                uses: uses.clone(),
            });
        }
    }
    crate::events::emit(|| crate::events::ExecutionEvent::JobFinished {
        job: name.to_string(),
        status: original.status.clone(),
        failure_allowed: original.failure_allowed,
        duration_ms: 0,
        logs: note.clone(),
        repeated_from: Some(original.name.clone()),
        reused: None,
    });
    wrkflw_logging::info(&format!(
        "{} Job '{}': {}",
        wrkflw_logging::symbols::SKIPPED,
        name,
        note
    ));
    JobResult {
        name: name.to_string(),
        logs: format!("{}\n\n{}", note, original.logs),
        repeated_from: Some(original.name.clone()),
        ..(*original).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn job(yaml: &str) -> Job {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn legs(key: &str, values: &[&str]) -> Vec<MatrixCombination> {
        values
            .iter()
            .map(|v| {
                MatrixCombination::new(HashMap::from([(
                    key.to_string(),
                    Value::String(v.to_string()),
                )]))
            })
            .collect()
    }

    const OS_LEGS: &[&str] = &["ubuntu-latest", "macos-latest", "windows-latest"];

    #[test]
    fn legs_that_differ_only_in_runs_on_run_once() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps:\n  - run: make test\n");
        let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
        assert_eq!(twins, vec![None, Some(0), Some(0)]);
    }

    #[test]
    fn a_step_reading_the_key_keeps_every_leg() {
        for body in [
            "steps:\n  - run: echo ${{ matrix.os }}\n",
            "steps:\n  - run: echo ${{ Matrix.OS }}\n",
            "steps:\n  - if: matrix.os == 'windows-latest'\n    run: dir\n",
            "steps:\n  - run: echo $MATRIX_OS\n",
            "steps:\n  - run: echo '${{ toJSON(matrix) }}'\n",
            "steps:\n  - run: echo ${{ matrix['os'] }}\n",
            "env:\n  TARGET: ${{ matrix.os }}\nsteps:\n  - run: make\n",
            // What other jobs see of this one, and how runs of it queue.
            "outputs:\n  os: ${{ matrix.os }}\nsteps:\n  - run: make\n",
            "concurrency: build-${{ matrix.os }}\nsteps:\n  - run: make\n",
            // A read that starts a line of a block, or follows a tab.
            "steps:\n  - if: |\n      github.event_name == 'push' ||\n      matrix.os == 'windows-latest'\n    run: dir\n",
            "steps:\n  - run: |\n      echo ${{\n      matrix.os }}\n",
            "steps:\n  - run: \"echo\\tmatrix.os\"\n",
            "steps:\n  - run: echo ${{ matrix. os }}\n",
            "steps:\n  - run: echo ${{ matrix .os }}\n",
            // Every leg has its own index.
            "steps:\n  - run: ./test --shard ${{ strategy.job-index }}\n",
            "steps:\n  - run: echo ${{ strategy['job-total'] }}\n",
        ] {
            let job = job(&format!("runs-on: ${{{{ matrix.os }}}}\n{body}"));
            let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
            assert_eq!(twins, vec![None, None, None], "{body}");
        }
    }

    #[test]
    fn a_matrix_value_in_any_job_key_keeps_every_leg() {
        for body in [
            "if: matrix.os != 'macos-latest'\n",
            "container: node:${{ matrix.os }}\n",
            "services:\n  db:\n    image: postgres:${{ matrix.os }}\n",
            "defaults:\n  run:\n    shell: ${{ matrix.os }}\n",
            "continue-on-error: ${{ matrix.os == 'windows-latest' }}\n",
            "uses: ./.github/workflows/build.yml\nwith:\n  os: ${{ matrix.os }}\n",
            "uses: ./.github/workflows/build.yml\nsecrets:\n  OS: ${{ matrix.os }}\n",
        ] {
            let job = job(&format!(
                "runs-on: ${{{{ matrix.os }}}}\n{body}steps:\n  - run: make\n"
            ));
            let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
            assert_eq!(twins, vec![None, None, None], "{body}");
        }
    }

    /// The fold reads the job through its parsed fields, so a key the parser
    /// drops is one it can't see. A new field must be checked against the
    /// fold (and added to the test above) before it is added here.
    #[test]
    fn the_fold_sees_every_job_field() {
        let value = serde_json::to_value(job("steps: []\n")).unwrap();
        let mut fields: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            [
                "concurrency",
                "container",
                "continue-on-error",
                "defaults",
                "env",
                "if",
                "needs",
                "outputs",
                "permissions",
                "runs-on",
                "secrets",
                "services",
                "steps",
                "strategy",
                "timeout-minutes",
                "uses",
                "with",
            ]
        );
    }

    #[test]
    fn reading_a_key_that_is_the_same_in_both_legs_is_fine() {
        let job = job(
            "runs-on: ${{ matrix.os }}\nsteps:\n  - run: cargo +${{ matrix.toolchain }} test\n",
        );
        let combos: Vec<MatrixCombination> = OS_LEGS
            .iter()
            .map(|os| {
                MatrixCombination::new(HashMap::from([
                    ("os".to_string(), Value::String(os.to_string())),
                    ("toolchain".to_string(), Value::String("stable".to_string())),
                ]))
            })
            .collect();
        assert_eq!(
            matrix_twins(&job, &combos, &HashMap::new()),
            vec![None, Some(0), Some(0)]
        );
    }

    #[test]
    fn a_leg_reports_the_twin_that_differs_from_it_only_in_runner() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps:\n  - run: ./flaky\n");
        let leg = |os: &str, attempt: &str| {
            MatrixCombination::new(HashMap::from([
                ("os".to_string(), Value::String(os.to_string())),
                ("attempt".to_string(), Value::String(attempt.to_string())),
            ]))
        };
        let combos = vec![
            leg("ubuntu-latest", "1"),
            leg("ubuntu-latest", "2"),
            leg("windows-latest", "1"),
            leg("windows-latest", "2"),
        ];
        // Both attempts run; each Windows attempt repeats its own Ubuntu one,
        // not the first leg it could stand for.
        assert_eq!(
            matrix_twins(&job, &combos, &HashMap::new()),
            vec![None, None, Some(0), Some(1)]
        );
    }

    #[test]
    fn a_runner_picked_by_index_folds_nothing() {
        // `matrix['os']` can't be pinned to `os`, so `attempt` might pick
        // the runner too: each attempt must still run.
        let job = job("runs-on: ${{ matrix['os'] }}\nsteps:\n  - run: ./flaky\n");
        let leg = |os: &str, attempt: &str| {
            MatrixCombination::new(HashMap::from([
                ("os".to_string(), Value::String(os.to_string())),
                ("attempt".to_string(), Value::String(attempt.to_string())),
            ]))
        };
        let combos = vec![
            leg("ubuntu-latest", "1"),
            leg("ubuntu-latest", "2"),
            leg("windows-latest", "1"),
        ];
        assert_eq!(
            matrix_twins(&job, &combos, &HashMap::new()),
            vec![None, None, None]
        );
    }

    #[test]
    fn matrix_reads_name_their_keys() {
        assert_eq!(
            matrix_reads("${{ matrix.os || 'x' }} ${{ Matrix . Python }}"),
            Reads::Keys(vec!["os".to_string(), "python".to_string()])
        );
        assert_eq!(matrix_reads("my_matrix.os matrix_os"), Reads::Keys(vec![]));
        assert_eq!(matrix_reads("${{ matrix.* }}"), Reads::Any);
        assert_eq!(matrix_reads("${{ strategy.job-index }}"), Reads::Any);
        assert_eq!(matrix_reads("a strategy for us"), Reads::Keys(vec![]));
    }

    #[test]
    fn legs_in_different_images_all_run() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps:\n  - run: make test\n");
        let overrides = HashMap::from([("macos-latest".to_string(), "my/mac:1".to_string())]);
        let twins = matrix_twins(&job, &legs("os", OS_LEGS), &overrides);
        assert_eq!(twins, vec![None, None, Some(0)]);
    }

    #[test]
    fn a_container_image_from_the_matrix_keeps_every_leg() {
        let job = job("container: node:${{ matrix.node }}\nsteps:\n  - run: npm test\n");
        let twins = matrix_twins(&job, &legs("node", &["18", "20"]), &HashMap::new());
        assert_eq!(twins, vec![None, None]);
    }

    #[test]
    fn a_key_nothing_reads_still_repeats_the_job() {
        let job = job("runs-on: ubuntu-latest\nsteps:\n  - run: ./flaky-test\n");
        let twins = matrix_twins(&job, &legs("attempt", &["1", "2", "3"]), &HashMap::new());
        assert_eq!(twins, vec![None, None, None]);
    }

    /// pallets/click: two legs add an OS and a display name to a leg that
    /// already runs that Python on Linux.
    #[test]
    fn legs_that_differ_in_runner_and_display_name_run_once() {
        let job = job("name: ${{ matrix.name || matrix.python }}\n\
             runs-on: ${{ matrix.os || 'ubuntu-latest' }}\n\
             steps:\n  - run: tox -e py${{ matrix.python }}\n");
        let leg = |pairs: &[(&str, &str)]| {
            MatrixCombination::new(
                pairs
                    .iter()
                    .map(|(k, v)| (k.to_string(), Value::String(v.to_string())))
                    .collect(),
            )
        };
        let combos = vec![
            leg(&[("python", "3.15")]),
            leg(&[
                ("name", "Windows"),
                ("python", "3.15"),
                ("os", "windows-latest"),
            ]),
            leg(&[("name", "Mac"), ("python", "3.15"), ("os", "macos-latest")]),
            leg(&[("python", "3.14")]),
            leg(&[("name", "PyPy"), ("python", "pypy-3.11")]),
        ];
        assert_eq!(
            matrix_twins(&job, &combos, &HashMap::new()),
            vec![None, Some(0), Some(0), None, None]
        );
    }

    #[test]
    fn a_name_alone_never_folds_legs_on_the_same_runner() {
        let job = job(
            "name: try ${{ matrix.attempt }}\nruns-on: ubuntu-latest\nsteps:\n  - run: ./flaky\n",
        );
        let twins = matrix_twins(&job, &legs("attempt", &["1", "2"]), &HashMap::new());
        assert_eq!(twins, vec![None, None]);
    }

    #[tokio::test]
    async fn a_repeated_leg_says_which_leg_it_repeats() {
        #[derive(Default)]
        struct Finished(Mutex<Vec<Option<String>>>);
        impl crate::events::ExecutionObserver for Finished {
            fn on_event(&self, event: &crate::events::ExecutionEvent) {
                if let crate::events::ExecutionEvent::JobFinished { repeated_from, .. } = event {
                    self.0.lock().unwrap().push(repeated_from.clone());
                }
            }
        }
        let seen = Arc::new(Finished::default());
        let original = JobResult::skipped("build (os: a)", "build", "x");
        let hooks = crate::events::RunHooks {
            observers: vec![seen.clone()],
            from_step: None,
        };
        let twin = crate::events::with_run_hooks(hooks, async {
            repeated_leg("build (os: b)", &original, "ubuntu", None)
        })
        .await;
        assert_eq!(twin.name, "build (os: b)");
        assert_eq!(twin.status, original.status);
        assert!(
            twin.logs.starts_with("Not run: build (os: a) was skipped"),
            "{}",
            twin.logs
        );
        assert_eq!(
            *seen.0.lock().unwrap(),
            vec![Some("build (os: a)".to_string())]
        );
    }

    #[tokio::test]
    async fn a_repeated_leg_reports_the_actions_its_original_did_not_run() {
        #[derive(Default)]
        struct NotRun(Mutex<Vec<(String, String)>>);
        impl crate::events::ExecutionObserver for NotRun {
            fn on_event(&self, event: &crate::events::ExecutionEvent) {
                if let crate::events::ExecutionEvent::ActionNotRun { job, uses, .. } = event {
                    self.0.lock().unwrap().push((job.clone(), uses.clone()));
                }
            }
        }
        let seen = Arc::new(NotRun::default());
        let mut original = JobResult::skipped("test (os: a)", "test", "x");
        original.status = crate::engine::JobStatus::Success;
        original.steps = vec![crate::engine::StepResult {
            name: "Upload".to_string(),
            status: crate::engine::StepStatus::Success,
            output: String::new(),
            outcome: crate::engine::StepStatus::Success,
            conclusion: crate::engine::StepStatus::Success,
            exit_code: None,
            actions_not_run: vec!["codecov/codecov-action@v5".to_string()],
        }];
        let hooks = crate::events::RunHooks {
            observers: vec![seen.clone()],
            from_step: None,
        };
        crate::events::with_run_hooks(hooks, async {
            repeated_leg("test (os: b)", &original, "ubuntu", None)
        })
        .await;
        assert_eq!(
            *seen.0.lock().unwrap(),
            vec![(
                "test (os: b)".to_string(),
                "codecov/codecov-action@v5".to_string()
            )]
        );
    }

    #[test]
    fn the_word_matrix_elsewhere_counts_as_reading_it() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps:\n  - name: test matrix\n    run: make\n");
        let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
        assert_eq!(twins, vec![None, None, None]);
    }
}
