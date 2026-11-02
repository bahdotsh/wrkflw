//! Checks on `${{ }}` expressions that need the whole job in view: whether
//! they are closed, and whether the steps, needs and matrix keys they read
//! exist. GitHub rejects the first at push time, and the rest quietly
//! evaluate to an empty string.

use serde_yaml::{Mapping, Value};
use std::collections::HashSet;
use wrkflw_models::ValidationResult;

/// What references can resolve to, lowercased because contexts are
/// case-insensitive. `None` means unknown, so references go unchecked: a
/// matrix from `fromJSON` only has keys at run time.
#[derive(Clone, Default)]
struct Scope {
    steps: Option<HashSet<String>>,
    needs: Option<HashSet<String>>,
    matrix: Option<HashSet<String>>,
}

/// Check every expression in `workflow`.
pub fn validate_expressions(workflow: &Value, result: &mut ValidationResult) {
    let Some(sections) = workflow.as_mapping() else {
        return;
    };
    for (key, value) in sections {
        let Some(section) = key.as_str() else {
            continue;
        };
        result.scoped(section, |r| match value.as_mapping() {
            Some(jobs) if section == "jobs" => {
                for (id, job) in jobs {
                    if let (Some(id), Some(job)) = (id.as_str(), job.as_mapping()) {
                        r.scoped(id, |r| check_job(job, r));
                    }
                }
            }
            _ => walk(value, &Scope::default(), r),
        });
    }
}

fn check_job(job: &Mapping, result: &mut ValidationResult) {
    let steps = job.get("steps").and_then(Value::as_sequence);
    let ids = |steps: &[Value]| -> HashSet<String> {
        steps
            .iter()
            .filter_map(|step| step.get("id")?.as_str())
            .map(str::to_lowercase)
            .collect()
    };
    let mut scope = Scope {
        steps: Some(ids(steps.map_or(&[], Vec::as_slice))),
        needs: Some(needs_of(job)),
        matrix: matrix_keys(job),
    };
    for (key, value) in job {
        let Some(key) = key.as_str() else {
            continue;
        };
        match key {
            // The matrix itself is defined here, so nothing in it can be
            // checked against it.
            "strategy" => result.scoped(key, |r| walk(value, &Scope::default(), r)),
            "steps" => {}
            "if" => result.scoped(key, |r| check_condition(value, &scope, r)),
            _ => result.scoped(key, |r| walk(value, &scope, r)),
        }
    }
    let Some(steps) = steps else {
        return;
    };
    result.scoped("steps", |r| {
        for (index, step) in steps.iter().enumerate() {
            // A step can only read the steps before it.
            scope.steps = Some(ids(&steps[..index]));
            r.push_index(index);
            walk(step, &scope, r);
            r.pop_scope();
        }
    });
}

fn needs_of(job: &Mapping) -> HashSet<String> {
    match job.get("needs") {
        Some(Value::String(need)) => HashSet::from([need.to_lowercase()]),
        Some(Value::Sequence(needs)) => needs
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_lowercase)
            .collect(),
        _ => HashSet::new(),
    }
}

/// The keys of the job's matrix, or `None` if they aren't known until it runs.
fn matrix_keys(job: &Mapping) -> Option<HashSet<String>> {
    let strategy = match job.get("strategy") {
        None => return Some(HashSet::new()),
        Some(strategy) => strategy.as_mapping()?,
    };
    let matrix = match strategy.get("matrix") {
        None => return Some(HashSet::new()),
        Some(matrix) => matrix.as_mapping()?,
    };
    let mut keys = HashSet::new();
    for (key, value) in matrix {
        match key.as_str()? {
            "exclude" => {}
            "include" => {
                for entry in value.as_sequence()? {
                    let entry = entry.as_mapping()?;
                    keys.extend(
                        entry
                            .keys()
                            .filter_map(Value::as_str)
                            .map(str::to_lowercase),
                    );
                }
            }
            axis => {
                keys.insert(axis.to_lowercase());
            }
        }
    }
    Some(keys)
}

fn walk(value: &Value, scope: &Scope, result: &mut ValidationResult) {
    match value {
        Value::String(text) => check_text(text, scope, result),
        Value::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                result.push_index(index);
                walk(item, scope, result);
                result.pop_scope();
            }
        }
        Value::Mapping(entries) => {
            for (key, value) in entries {
                let Some(key) = key.as_str() else {
                    continue;
                };
                result.scoped(key, |r| {
                    if key == "if" {
                        check_condition(value, scope, r);
                    } else {
                        walk(value, scope, r);
                    }
                });
            }
        }
        _ => {}
    }
}

/// `if:` is an expression even without `${{ }}`.
fn check_condition(value: &Value, scope: &Scope, result: &mut ValidationResult) {
    match value.as_str() {
        Some(text) if !text.contains("${{") => check_references(text, scope, result),
        _ => walk(value, scope, result),
    }
}

fn check_text(text: &str, scope: &Scope, result: &mut ValidationResult) {
    let (expressions, closed) = expressions_in(text);
    if !closed {
        result.report(
            "unterminated-expression",
            format!("Expression is missing its closing `}}}}`: {}", text.trim()),
        );
    }
    for expression in expressions {
        check_references(expression, scope, result);
    }
}

fn check_references(expression: &str, scope: &Scope, result: &mut ValidationResult) {
    let code = without_strings(expression).to_lowercase();
    let mut report =
        |known: &Option<HashSet<String>>, context, rule, message: &dyn Fn(&str) -> String| {
            let Some(known) = known else {
                return;
            };
            for name in references(&code, context) {
                if !known.contains(&name) {
                    result.report(rule, message(&name));
                }
            }
        };
    report(&scope.steps, "steps", "undefined-step-reference", &|name| {
        format!("Reference to step '{name}', but no earlier step in this job has that id")
    });
    report(&scope.needs, "needs", "undefined-need-reference", &|name| {
        format!("Reference to needs.{name}, but this job does not list '{name}' in `needs:`")
    });
    report(&scope.matrix, "matrix", "undefined-matrix-key", &|name| {
        format!("Reference to matrix.{name}, but this job's matrix has no such key")
    });
}

/// The bodies of the `${{ }}` in `text`, and whether the last one was closed.
fn expressions_in(text: &str) -> (Vec<&str>, bool) {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        rest = &rest[start + 3..];
        let Some(end) = closing_braces(rest) else {
            return (found, false);
        };
        found.push(&rest[..end]);
        rest = &rest[end + 2..];
    }
    (found, true)
}

/// Where `}}` ends the expression. Inside a string literal it doesn't.
fn closing_braces(text: &str) -> Option<usize> {
    let mut in_string = false;
    for (index, c) in text.char_indices() {
        if c == '\'' {
            in_string = !in_string;
        } else if !in_string && text[index..].starts_with("}}") {
            return Some(index);
        }
    }
    None
}

/// `expression` with string literals blanked, so that `'steps.foo'` is not a
/// reference. `''` is an escaped quote, which toggling handles.
fn without_strings(expression: &str) -> String {
    let mut in_string = false;
    expression
        .chars()
        .filter(|c| {
            if *c == '\'' {
                in_string = !in_string;
            }
            !in_string && *c != '\''
        })
        .collect()
}

/// Names read through `context.` in lowercased `code`, without duplicates.
/// `github.steps.x` isn't the `steps` context, and `steps.*` names nothing.
fn references(code: &str, context: &str) -> Vec<String> {
    let is_name = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let prefix = format!("{context}.");
    let mut names = Vec::new();
    for (start, _) in code.match_indices(&prefix) {
        let before = code[..start].chars().next_back();
        if before.is_some_and(|c| is_name(c) || c == '.') {
            continue;
        }
        let name: String = code[start + prefix.len()..]
            .chars()
            .take_while(|c| is_name(*c))
            .collect();
        if !name.is_empty() && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(workflow: &str) -> Vec<String> {
        let workflow: Value = serde_yaml::from_str(workflow).unwrap();
        let mut result = ValidationResult::new();
        validate_expressions(&workflow, &mut result);
        result.details.into_iter().map(|d| d.rule).collect()
    }

    fn job(body: &str) -> String {
        format!("jobs:\n  a:\n    runs-on: ubuntu-latest\n{body}")
    }

    fn run_step(script: &str) -> String {
        job(&format!("    steps:\n      - run: {script}\n"))
    }

    #[test]
    fn an_unclosed_expression_is_reported_anywhere_in_the_file() {
        assert_eq!(
            rules("env:\n  A: ${{ github.sha\n"),
            ["unterminated-expression"]
        );
        assert_eq!(
            rules(&run_step("echo ${{ github.sha")),
            ["unterminated-expression"]
        );
        assert!(rules(&run_step("echo ${{ format('}}') }}")).is_empty());
        assert!(rules(&run_step("echo $${HOME} }}")).is_empty());
    }

    #[test]
    fn a_step_reference_needs_an_earlier_step_with_that_id() {
        let later = job(
            "    steps:\n      - run: echo ${{ steps.b.outputs.x }}\n      - id: b\n        run: echo\n",
        );
        assert_eq!(rules(&later), ["undefined-step-reference"]);
        let earlier = job(
            "    steps:\n      - id: B\n        run: echo\n      - if: steps.b.outcome == 'success'\n        run: echo ${{ steps.b.outputs.x }}\n",
        );
        assert!(rules(&earlier).is_empty());
    }

    #[test]
    fn job_outputs_can_read_any_step() {
        let outputs = job(
            "    outputs:\n      x: ${{ steps.b.outputs.x }}\n    steps:\n      - id: b\n        run: echo\n",
        );
        assert!(rules(&outputs).is_empty());
    }

    #[test]
    fn a_bare_condition_is_checked() {
        let condition =
            job("    if: steps.nope.outcome == 'success'\n    steps:\n      - run: echo\n");
        assert_eq!(rules(&condition), ["undefined-step-reference"]);
    }

    #[test]
    fn a_needs_reference_must_be_in_needs() {
        assert_eq!(
            rules(&run_step("echo ${{ needs.build.outputs.y }}")),
            ["undefined-need-reference"]
        );
        let listed =
            job("    needs: build\n    steps:\n      - run: echo ${{ needs.build.result }}\n");
        assert!(rules(&listed).is_empty());
    }

    #[test]
    fn a_matrix_reference_needs_a_key() {
        assert_eq!(
            rules(&run_step("echo ${{ matrix.os }}")),
            ["undefined-matrix-key"]
        );
        let axes = job(
            "    strategy:\n      matrix:\n        os: [a]\n        include:\n          - extra: 1\n    steps:\n      - run: echo ${{ matrix.os }} ${{ matrix.extra }}\n",
        );
        assert!(rules(&axes).is_empty());
        let wrong = axes.replace("matrix.extra", "matrix.nope");
        assert_eq!(rules(&wrong), ["undefined-matrix-key"]);
    }

    #[test]
    fn a_matrix_known_only_at_run_time_is_not_checked() {
        for matrix in [
            "matrix: ${{ fromJSON(needs.x.outputs.m) }}",
            "matrix:\n        include: ${{ fromJSON(needs.x.outputs.m) }}",
        ] {
            let dynamic = job(&format!(
                "    needs: x\n    strategy:\n      {matrix}\n    steps:\n      - run: echo ${{{{ matrix.anything }}}}\n"
            ));
            assert!(rules(&dynamic).is_empty(), "{matrix}");
        }
    }

    #[test]
    fn strings_and_other_contexts_are_not_references() {
        assert!(rules(&run_step("echo ${{ format('steps.x {0}', github.sha) }}")).is_empty());
        assert!(rules(&run_step(
            "echo ${{ github.steps.x }} ${{ toJSON(steps.*.outcome) }}"
        ))
        .is_empty());
    }
}
