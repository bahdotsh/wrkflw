use crate::error::TriggerFilterError;
use crate::model::{EventFilter, GlobPattern, WorkflowTriggerConfig};
use std::path::PathBuf;
use wrkflw_parser::workflow::WorkflowDefinition;

/// Parse the `on_raw` YAML value from a WorkflowDefinition into structured trigger config.
///
/// Glob patterns in `branches`, `tags`, `paths`, and their `*-ignore` counterparts
/// are compiled here so that invalid patterns surface as a `ParseError` instead of
/// silently never matching at evaluation time.
pub fn parse_trigger_config(
    workflow: &WorkflowDefinition,
    workflow_path: PathBuf,
) -> Result<WorkflowTriggerConfig, TriggerFilterError> {
    let events = parse_events(&workflow.on_raw)?;
    Ok(WorkflowTriggerConfig {
        workflow_path,
        workflow_name: workflow.name.clone(),
        events,
    })
}

fn parse_events(on_raw: &serde_yaml::Value) -> Result<Vec<EventFilter>, TriggerFilterError> {
    match on_raw {
        // on: push
        serde_yaml::Value::String(event) => Ok(vec![EventFilter {
            event_name: event.clone(),
            ..Default::default()
        }]),

        // on: [push, pull_request]
        serde_yaml::Value::Sequence(events) => {
            // Reject non-string entries the same way `extract_string_list`
            // does for `paths`/`branches`/etc. Previously this path silently
            // dropped anything that wasn't a string, so a typo like
            // `on: [push, {pull_request: {branches: [main]}}]` collapsed to
            // `on: [push]` and the pull_request filter was never applied —
            // exactly the "silently lying about which workflows would run"
            // mode the rest of this crate is built to prevent.
            let mut filters = Vec::with_capacity(events.len());
            for (idx, event) in events.iter().enumerate() {
                let name = event.as_str().ok_or_else(|| {
                    TriggerFilterError::ParseError(format!(
                        "on[{}]: expected a string event name, got {}",
                        idx,
                        yaml_kind(event),
                    ))
                })?;
                filters.push(EventFilter {
                    event_name: name.to_string(),
                    ..Default::default()
                });
            }
            Ok(filters)
        }

        // on: { push: { branches: [main], paths: [src/**] } }
        serde_yaml::Value::Mapping(map) => {
            let mut filters = Vec::new();
            for (key, value) in map {
                let event_name = key
                    .as_str()
                    .ok_or_else(|| {
                        TriggerFilterError::ParseError("Event name must be a string".to_string())
                    })?
                    .to_string();

                let filter = parse_event_config(&event_name, value)?;
                filters.push(filter);
            }
            Ok(filters)
        }

        _ => Err(TriggerFilterError::ParseError(
            "'on' section has invalid format".to_string(),
        )),
    }
}

fn parse_event_config(
    event_name: &str,
    value: &serde_yaml::Value,
) -> Result<EventFilter, TriggerFilterError> {
    // null or empty config means no filters
    if value.is_null() || value == &serde_yaml::Value::Mapping(serde_yaml::Mapping::new()) {
        return Ok(EventFilter {
            event_name: event_name.to_string(),
            ..Default::default()
        });
    }

    // A non-null value for an event MUST be a mapping of filter keys
    // (branches/tags/paths/types/...). Previously this path silently
    // returned a default (unfiltered) EventFilter for anything else, so
    // a typo like `on: { push: "main" }` collapsed to "push with no
    // filters" — exactly the silent-drop mode the rest of this parser
    // guards against via `extract_string_list`. Surface the typo with
    // the offending yaml kind so the user can fix it.
    let map = value.as_mapping().ok_or_else(|| {
        TriggerFilterError::ParseError(format!(
            "{}: event config must be a mapping of filter keys (branches, tags, paths, types, ...) or null, got {}",
            event_name,
            yaml_kind(value),
        ))
    })?;

    // GitHub Actions supports inline `!`-prefixed exclusion patterns in
    // `branches`, `tags`, and `paths`. `resolve_include_and_ignore` splits
    // them into the include / exclude lists at parse time so the evaluator
    // sees them the same way as the explicit `*-ignore` forms. Mixing
    // `!`-patterns with the dedicated `*-ignore` key is rejected by GitHub
    // Actions, so we enforce the same rule to keep semantics predictable.
    // GitHub Actions also rejects combining a populated `branches:` (or
    // `tags:`/`paths:`) with the matching `*-ignore` key — that
    // mutual-exclusion check lives in `resolve_include_and_ignore` too.
    let (branches, branches_ignore) =
        resolve_include_and_ignore(map, "branches", "branches-ignore", event_name)?;
    let (tags, tags_ignore) = resolve_include_and_ignore(map, "tags", "tags-ignore", event_name)?;
    let (paths, paths_ignore) =
        resolve_include_and_ignore(map, "paths", "paths-ignore", event_name)?;

    Ok(EventFilter {
        event_name: event_name.to_string(),
        branches,
        branches_ignore,
        tags,
        tags_ignore,
        paths,
        paths_ignore,
        types: extract_string_list(map, "types", event_name)?,
    })
}

/// Resolve an `(include, ignore)` list pair for one of the three filter
/// axes — `branches`/`branches-ignore`, `tags`/`tags-ignore`,
/// `paths`/`paths-ignore`.
///
/// Handles GitHub Actions' inline `!`-negation semantics (which
/// `extract_glob_list` splits into `(includes, inline_excludes)`), and
/// enforces both of GHA's mutual-exclusion rules:
///
/// 1. Inline `!`-patterns inside `<include>:` cannot be combined with a
///    separate `<include>-ignore:` key.
/// 2. A populated `<include>:` cannot be combined with a populated
///    `<include>-ignore:` key — GHA rejects this at upload time, so
///    accepting it locally would let users iterate against semantics that
///    will fail in production.
fn resolve_include_and_ignore(
    map: &serde_yaml::Mapping,
    include_key: &str,
    ignore_key: &str,
    event_name: &str,
) -> Result<(Vec<GlobPattern>, Vec<GlobPattern>), TriggerFilterError> {
    let (includes, inline_ignore) = extract_glob_list(map, include_key, event_name)?;
    let explicit_ignore = extract_glob_list(map, ignore_key, event_name)?.0;
    if !inline_ignore.is_empty() && !explicit_ignore.is_empty() {
        return Err(TriggerFilterError::ParseError(format!(
            "{}: cannot mix inline `!`-patterns in `{}:` with a separate `{}:` key",
            event_name, include_key, ignore_key
        )));
    }
    if !includes.is_empty() && !explicit_ignore.is_empty() {
        return Err(TriggerFilterError::ParseError(format!(
            "{}: cannot use both `{}:` and `{}:` on the same event \
             (GitHub Actions rejects this combination — pick one)",
            event_name, include_key, ignore_key
        )));
    }
    let ignore = if explicit_ignore.is_empty() {
        inline_ignore
    } else {
        explicit_ignore
    };
    Ok((includes, ignore))
}

/// Extract a `String` or `Vec<String>` from a YAML mapping, surfacing
/// any non-string entries as a `ParseError`.
///
/// We deliberately do NOT silently drop non-string entries the way
/// `filter_map(... .as_str())` would. A typo like
/// `paths: [{src: foo}]` (a forgotten quoted glob containing `:`) used
/// to yield an empty list, which then matched everything — exactly the
/// "silently lying about which workflows would run" failure mode the
/// rest of this crate is built to prevent. Surface the typo with a
/// location that names the event and key so the user can find it fast.
fn extract_string_list(
    map: &serde_yaml::Mapping,
    key: &str,
    event_name: &str,
) -> Result<Vec<String>, TriggerFilterError> {
    let value = match map.get(serde_yaml::Value::String(key.to_string())) {
        Some(v) => v,
        None => return Ok(Vec::new()),
    };

    match value {
        serde_yaml::Value::String(s) => Ok(vec![s.clone()]),
        serde_yaml::Value::Sequence(seq) => {
            let mut out = Vec::with_capacity(seq.len());
            for (idx, item) in seq.iter().enumerate() {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(TriggerFilterError::ParseError(format!(
                            "{}.{}[{}]: expected a string, got {}",
                            event_name,
                            key,
                            idx,
                            yaml_kind(item),
                        )));
                    }
                }
            }
            Ok(out)
        }
        // null is treated as "absent" — GHA accepts e.g. `branches:` with
        // no value and the user clearly intended an empty list.
        serde_yaml::Value::Null => Ok(Vec::new()),
        other => Err(TriggerFilterError::ParseError(format!(
            "{}.{}: expected a string or list of strings, got {}",
            event_name,
            key,
            yaml_kind(other),
        ))),
    }
}

/// Human-readable name for a YAML node, used in `ParseError` messages so
/// the diagnostic doesn't dump a `Debug`-formatted blob at the user.
fn yaml_kind(v: &serde_yaml::Value) -> &'static str {
    match v {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "bool",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "sequence",
        serde_yaml::Value::Mapping(_) => "mapping",
        serde_yaml::Value::Tagged(_) => "tagged value",
    }
}

/// Extract a list of strings from a YAML mapping and compile each as a glob pattern.
///
/// Returns `(includes, inline_excludes)` — entries prefixed with `!` are treated
/// as inline negations (GitHub Actions semantics) and routed into the second
/// tuple field with the `!` stripped. Callers are responsible for merging the
/// inline excludes into the corresponding `*-ignore` list.
///
/// Compilation failures are surfaced as `TriggerFilterError::ParseError` so that
/// a typo like `paths: [src/**.rs]` is reported to the user instead of silently
/// causing the workflow to never trigger.
fn extract_glob_list(
    map: &serde_yaml::Mapping,
    key: &str,
    event_name: &str,
) -> Result<(Vec<GlobPattern>, Vec<GlobPattern>), TriggerFilterError> {
    let raw = extract_string_list(map, key, event_name)?;
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for source in raw {
        // `!!literal` is GHA's escape for a literal leading `!`: two `!`
        // prefixes collapse into one and the result is an include pattern
        // matching a ref/path that literally starts with `!`. Three or
        // more bangs are undocumented in GHA; we treat `!!!foo` as a
        // literal include of `!!foo` (the `!!` escape consumes only the
        // first two bangs). The behavior is pinned by
        // `triple_bang_treated_as_literal_double_bang` so a future
        // refactor cannot silently flip it.
        let (is_exclude, pattern_str) = if let Some(rest) = source.strip_prefix("!!") {
            (false, format!("!{}", rest))
        } else if let Some(rest) = source.strip_prefix('!') {
            (true, rest.to_string())
        } else {
            (false, source.clone())
        };

        let compiled = GlobPattern::new(&pattern_str).map_err(|e| {
            TriggerFilterError::ParseError(format!(
                "Invalid glob pattern '{}' under '{}.{}': {}",
                source, event_name, key, e
            ))
        })?;

        if is_exclude {
            excludes.push(compiled);
        } else {
            includes.push(compiled);
        }
    }
    Ok((includes, excludes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_on_raw(yaml: &str) -> serde_yaml::Value {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn parse_string_trigger() {
        let raw = make_on_raw("push");
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "push");
        assert!(events[0].paths.is_empty());
    }

    #[test]
    fn parse_sequence_trigger() {
        let raw = make_on_raw("[push, pull_request]");
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_name, "push");
        assert_eq!(events[1].event_name, "pull_request");
    }

    #[test]
    fn parse_mapping_with_paths() {
        let raw = make_on_raw(
            r#"
push:
  branches: [main, release/**]
  paths:
    - 'src/**'
    - 'Cargo.toml'
pull_request:
  paths-ignore:
    - 'docs/**'
    - '*.md'
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);

        let push = events.iter().find(|e| e.event_name == "push").unwrap();
        let branch_sources: Vec<&str> = push.branches.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(branch_sources, vec!["main", "release/**"]);
        let path_sources: Vec<&str> = push.paths.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(path_sources, vec!["src/**", "Cargo.toml"]);

        let pr = events
            .iter()
            .find(|e| e.event_name == "pull_request")
            .unwrap();
        let pi: Vec<&str> = pr.paths_ignore.iter().map(|g| g.source.as_str()).collect();
        assert_eq!(pi, vec!["docs/**", "*.md"]);
    }

    #[test]
    fn parse_mapping_with_null_config() {
        let raw = make_on_raw(
            r#"
workflow_dispatch:
push:
  branches: [main]
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 2);
        let wd = events
            .iter()
            .find(|e| e.event_name == "workflow_dispatch")
            .unwrap();
        assert!(wd.paths.is_empty());
        assert!(wd.branches.is_empty());
    }

    #[test]
    fn parse_mapping_with_tags() {
        // Note: this test originally combined `tags:` + `tags-ignore:` on
        // the same event, which GitHub Actions rejects at upload time.
        // The parser now enforces the same rule, so we use the inline
        // `!`-negation form (which IS valid GHA syntax) to exercise the
        // include-and-exclude code path. Inline negation is split into
        // `tags_ignore` by `extract_glob_list`, so the assertion shape
        // matches the original.
        let raw = make_on_raw(
            r#"
push:
  tags:
    - 'v*'
    - '!v*-rc*'
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events[0].tags[0].source, "v*");
        assert_eq!(events[0].tags_ignore[0].source, "v*-rc*");
    }

    #[test]
    fn parse_mapping_with_types() {
        let raw = make_on_raw(
            r#"
pull_request:
  types:
    - opened
    - synchronize
    - reopened
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_name, "pull_request");
        assert_eq!(events[0].types, vec!["opened", "synchronize", "reopened"]);
    }

    #[test]
    fn parse_single_string_type() {
        let raw = make_on_raw(
            r#"
issues:
  types: opened
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert_eq!(events[0].types, vec!["opened"]);
    }

    #[test]
    fn invalid_glob_pattern_surfaces_as_parse_error() {
        // Unclosed bracket is an invalid glob — should fail at parse time, not silently
        let raw = make_on_raw(
            r#"
push:
  paths:
    - '[unclosed'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Invalid glob"), "got: {}", msg);
        assert!(msg.contains("[unclosed"), "got: {}", msg);
        assert!(msg.contains("push.paths"), "got: {}", msg);
    }

    #[test]
    fn inline_negation_routes_into_ignore_list() {
        // GitHub Actions semantics: `!pattern` inside `branches:` is an
        // inline exclusion equivalent to adding to `branches-ignore:`.
        let raw = make_on_raw(
            r#"
push:
  branches:
    - 'release/*'
    - '!release/old'
    - '!release/abandoned'
"#,
        );
        let events = parse_events(&raw).unwrap();
        let push = &events[0];
        let inc: Vec<&str> = push.branches.iter().map(|g| g.source.as_str()).collect();
        let exc: Vec<&str> = push
            .branches_ignore
            .iter()
            .map(|g| g.source.as_str())
            .collect();
        assert_eq!(inc, vec!["release/*"]);
        assert_eq!(exc, vec!["release/old", "release/abandoned"]);
    }

    #[test]
    fn inline_negation_on_paths_and_tags() {
        let raw = make_on_raw(
            r#"
push:
  paths:
    - 'src/**'
    - '!src/generated/**'
  tags:
    - 'v*'
    - '!v*-rc*'
"#,
        );
        let events = parse_events(&raw).unwrap();
        let push = &events[0];
        assert_eq!(
            push.paths
                .iter()
                .map(|g| g.source.as_str())
                .collect::<Vec<_>>(),
            vec!["src/**"]
        );
        assert_eq!(
            push.paths_ignore
                .iter()
                .map(|g| g.source.as_str())
                .collect::<Vec<_>>(),
            vec!["src/generated/**"]
        );
        assert_eq!(
            push.tags
                .iter()
                .map(|g| g.source.as_str())
                .collect::<Vec<_>>(),
            vec!["v*"]
        );
        assert_eq!(
            push.tags_ignore
                .iter()
                .map(|g| g.source.as_str())
                .collect::<Vec<_>>(),
            vec!["v*-rc*"]
        );
    }

    #[test]
    fn triple_bang_treated_as_literal_double_bang() {
        // GHA's spec only documents `!!foo` (literal `!foo`); `!!!foo`
        // is undocumented. Our parser greedily consumes the first two
        // bangs as the escape, so `!!!foo` becomes a literal include
        // matching `!!foo`. This test pins that behavior — flipping it
        // (e.g. to "exclude `!foo`") would be a semantic change that
        // must be done deliberately, not as a side effect of refactoring
        // `extract_glob_list`.
        let raw = make_on_raw(
            r#"
push:
  branches:
    - '!!!triple'
"#,
        );
        let events = parse_events(&raw).unwrap();
        let push = &events[0];
        assert_eq!(push.branches.len(), 1);
        assert_eq!(push.branches[0].source, "!!triple");
        assert!(push.branches_ignore.is_empty());
    }

    #[test]
    fn double_bang_escapes_literal_leading_bang() {
        // `!!foo` means a pattern whose literal first character is `!`.
        let raw = make_on_raw(
            r#"
push:
  branches:
    - '!!weird-branch'
"#,
        );
        let events = parse_events(&raw).unwrap();
        let push = &events[0];
        assert_eq!(push.branches.len(), 1);
        assert_eq!(push.branches[0].source, "!weird-branch");
        assert!(push.branches_ignore.is_empty());
    }

    #[test]
    fn inline_negation_rejected_when_mixed_with_ignore_key() {
        // GHA disallows mixing inline `!` with the dedicated ignore key.
        let raw = make_on_raw(
            r#"
push:
  branches:
    - 'main'
    - '!release/old'
  branches-ignore:
    - 'legacy/*'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot mix"), "got: {}", msg);
        assert!(msg.contains("branches"), "got: {}", msg);
    }

    #[test]
    fn invalid_branches_glob_surfaces_as_parse_error() {
        let raw = make_on_raw(
            r#"
push:
  branches:
    - 'main'
    - '[bad'
"#,
        );
        assert!(parse_events(&raw).is_err());
    }

    #[test]
    fn paths_and_paths_ignore_combo_is_rejected() {
        // Regression: GHA rejects `paths:` + `paths-ignore:` on the same
        // event at upload time. Previously this parser silently accepted
        // both and ran them sequentially, so users could iterate locally
        // against semantics that would later fail in production.
        let raw = make_on_raw(
            r#"
push:
  paths:
    - 'src/**'
  paths-ignore:
    - 'docs/**'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("cannot use both"), "got: {}", msg);
        assert!(
            msg.contains("paths") && msg.contains("paths-ignore"),
            "got: {}",
            msg
        );
    }

    #[test]
    fn branches_and_branches_ignore_combo_is_rejected() {
        let raw = make_on_raw(
            r#"
push:
  branches:
    - 'main'
  branches-ignore:
    - 'release/*'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        assert!(err.to_string().contains("cannot use both"), "got: {}", err);
    }

    #[test]
    fn tags_and_tags_ignore_combo_is_rejected() {
        let raw = make_on_raw(
            r#"
push:
  tags:
    - 'v*'
  tags-ignore:
    - 'v*-rc*'
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        assert!(err.to_string().contains("cannot use both"), "got: {}", err);
    }

    #[test]
    fn non_string_list_entry_surfaces_as_parse_error() {
        // Regression: a typo like `paths: [{src: foo}]` (forgotten quotes
        // around a glob containing `:`) used to silently yield an empty
        // list, which then matched everything — the worst possible
        // failure mode for a "would this trigger?" filter. Surface the
        // typo as a `ParseError` with the offending location instead.
        let raw = make_on_raw(
            r#"
push:
  paths:
    - 'src/**'
    - { not: a-string }
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expected a string"), "got: {}", msg);
        assert!(msg.contains("push.paths"), "got: {}", msg);
        assert!(msg.contains("[1]"), "got: {}", msg);
    }

    #[test]
    fn non_string_types_entry_surfaces_as_parse_error() {
        // The same surfacing must work for `types:` (which goes through
        // the same `extract_string_list` helper).
        let raw = make_on_raw(
            r#"
pull_request:
  types:
    - opened
    - 42
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expected a string"), "got: {}", msg);
        assert!(msg.contains("pull_request.types"), "got: {}", msg);
    }

    #[test]
    fn non_string_entry_in_sequence_form_surfaces_as_parse_error() {
        // Regression: the `on: [push, pull_request]` sequence path used to
        // silently drop non-string entries via `if let Some(name) = event.as_str()`.
        // A typo like `on: [push, {pull_request: ...}]` would collapse to
        // `on: [push]` and the misplaced pull_request filter would never
        // apply — the same failure mode the rest of the parser guards against
        // via `extract_string_list`. Now it must be a `ParseError` that names
        // the offending index and kind.
        let raw = make_on_raw(
            r#"
- push
- pull_request: { branches: [main] }
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expected a string"), "got: {}", msg);
        assert!(msg.contains("on[1]"), "got: {}", msg);
        assert!(msg.contains("mapping"), "got: {}", msg);
    }

    #[test]
    fn non_mapping_event_value_surfaces_as_parse_error() {
        // Regression: `on: { push: "main" }` (value is a string instead
        // of a `{ branches: [main] }` mapping) used to be silently
        // accepted as "push with default filters", which matched every
        // push regardless of the user's intent. Now it must be a
        // ParseError naming the event and the offending yaml kind so
        // the user can locate the typo.
        let raw = make_on_raw(
            r#"
push: "main"
"#,
        );
        let err = parse_events(&raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("must be a mapping"), "got: {}", msg);
        assert!(msg.contains("push"), "got: {}", msg);
        assert!(msg.contains("string"), "got: {}", msg);
    }

    #[test]
    fn null_value_for_list_key_is_treated_as_empty() {
        // `branches:` with no value (null) should be accepted and treated
        // as an empty list, matching GHA's behavior. This is the negative
        // case for the "non-string entry" rejection above.
        let raw = make_on_raw(
            r#"
push:
  branches:
"#,
        );
        let events = parse_events(&raw).unwrap();
        assert!(events[0].branches.is_empty());
    }
}
