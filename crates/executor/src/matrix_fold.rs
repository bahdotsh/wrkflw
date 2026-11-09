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
/// reading all of them, so a leg is only folded away when that is safe.
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
    let images: Vec<String> = combinations
        .iter()
        .map(|c| get_effective_runner_image(job, platform_overrides, Some(&c.values)))
        .collect();
    for leg in 1..combinations.len() {
        twins[leg] = (0..leg).filter(|&i| twins[i].is_none()).find(|&i| {
            let differing = differing_keys(&combinations[i].values, &combinations[leg].values);
            let picks_runner =
                |key: &String| reads_matrix_keys(&runs_on, std::slice::from_ref(key));
            images[i] == images[leg]
                && differing.iter().any(picks_runner)
                && !reads_matrix_keys(&body, &differing)
        });
    }
    twins
}

/// The job as text, minus the two places a matrix value may appear without
/// changing what the job does: `runs-on`, and the matrix itself. The job's
/// display name isn't parsed, so it is never here. Step names are, so a
/// step named after a matrix value keeps its leg, although the name changes
/// nothing that runs: the fold errs toward running a leg.
fn job_body_text(job: &Job) -> Option<String> {
    let mut value = serde_json::to_value(job).ok()?;
    let map = value.as_object_mut()?;
    map.remove("runs-on");
    map.remove("strategy");
    Some(value.to_string())
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

/// Whether `body` may read any of `keys` from the matrix. Expressions are
/// case-insensitive, and `MATRIX_<KEY>`/`MATRIX_CONTEXT` are the variables
/// wrkflw sets for each leg.
fn reads_matrix_keys(body: &str, keys: &[String]) -> bool {
    if body.contains("MATRIX_CONTEXT")
        || keys
            .iter()
            .any(|key| body.contains(&format!("MATRIX_{}", key.to_uppercase())))
    {
        return true;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let lower = body.to_lowercase();
    for (pos, _) in lower.match_indices("matrix") {
        if lower[..pos].chars().next_back().is_some_and(is_word) {
            continue;
        }
        let rest = &lower[pos + "matrix".len()..];
        if rest.chars().next().is_some_and(is_word) {
            continue;
        }
        // `matrix[...]`, `toJSON(matrix)` or a bare mention: can't tell
        // which key, so assume any.
        let Some(rest) = rest.strip_prefix('.') else {
            return true;
        };
        let key: String = rest.chars().take_while(|&c| is_word(c)).collect();
        if key == "*" || keys.contains(&key) {
            return true;
        }
    }
    false
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
        ] {
            let job = job(&format!("runs-on: ${{{{ matrix.os }}}}\n{body}"));
            let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
            assert_eq!(twins, vec![None, None, None], "{body}");
        }
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

    #[test]
    fn the_word_matrix_elsewhere_counts_as_reading_it() {
        let job = job("runs-on: ${{ matrix.os }}\nsteps:\n  - name: test matrix\n    run: make\n");
        let twins = matrix_twins(&job, &legs("os", OS_LEGS), &HashMap::new());
        assert_eq!(twins, vec![None, None, None]);
    }
}
