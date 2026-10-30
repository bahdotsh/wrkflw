//! Assembly of a GitLab CI configuration into a single resolved document.
//!
//! GitLab composes a pipeline from several pieces before any job runs:
//! `spec:inputs` headers with `$[[ inputs.* ]]` interpolation, `include:`d
//! files, `extends:` inheritance, `!reference` tags and hidden (`.`-prefixed)
//! template keys. This module performs those steps so the rest of wrkflw sees
//! a flat configuration that contains only real jobs.

use regex::Regex;
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// GitLab's default limit on the number of included files per pipeline.
const MAX_INCLUDES: usize = 150;
/// GitLab allows up to 11 levels of `extends` inheritance.
const MAX_EXTENDS_DEPTH: usize = 11;
/// GitLab allows up to 10 levels of nested `!reference` tags.
const MAX_REFERENCE_DEPTH: usize = 10;

const SCRIPT_KEYS: [&str; 3] = ["before_script", "script", "after_script"];

/// Top-level keywords that are never job definitions.
const GLOBAL_KEYWORDS: [&str; 11] = [
    "after_script",
    "before_script",
    "cache",
    "default",
    "image",
    "include",
    "services",
    "spec",
    "stages",
    "variables",
    "workflow",
];

/// Load the pipeline at `path` and return the fully resolved configuration.
///
/// Local includes are resolved relative to the directory containing `path`,
/// which is the repository root for a conventional `.gitlab-ci.yml`.
pub fn load_pipeline(path: &Path) -> Result<Value, String> {
    let root = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let root = root
        .canonicalize()
        .map_err(|e| format!("failed to resolve '{}': {}", root.display(), e))?;

    let mut loader = Loader {
        root,
        stack: Vec::new(),
        included: 0,
    };
    let mut config = loader.load_file(path, None)?;

    resolve_extends(&mut config)?;
    resolve_references(&mut config)?;
    finalize(&mut config);
    Ok(config)
}

struct Loader {
    root: PathBuf,
    /// Files currently being loaded, used to detect include cycles.
    stack: Vec<PathBuf>,
    included: usize,
}

impl Loader {
    /// Load one configuration file, interpolate its inputs and merge in
    /// everything it includes. Keys defined in the file itself take
    /// precedence over keys from its includes.
    fn load_file(&mut self, path: &Path, inputs: Option<&Mapping>) -> Result<Value, String> {
        let canonical = path
            .canonicalize()
            .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
        if self.stack.contains(&canonical) {
            return Err(format!("include cycle detected at '{}'", path.display()));
        }

        let content = fs::read_to_string(&canonical)
            .map_err(|e| format!("failed to read '{}': {}", path.display(), e))?;
        let in_file = |e: String| format!("{}: {}", path.display(), e);

        let (spec, mut body) = split_documents(&content).map_err(in_file)?;
        match spec {
            Some(spec) => {
                let values = resolve_inputs(&spec, inputs).map_err(in_file)?;
                interpolate(&mut body, &values).map_err(in_file)?;
            }
            None if inputs.is_some_and(|i| !i.is_empty()) => {
                return Err(in_file(
                    "inputs were provided but the file has no `spec:inputs` header".to_string(),
                ));
            }
            None => {}
        }

        let mut body = match body {
            Value::Mapping(map) => map,
            Value::Null => Mapping::new(),
            _ => return Err(in_file("configuration must be a mapping".to_string())),
        };

        let mut merged = Value::Mapping(Mapping::new());
        if let Some(include) = body.remove("include") {
            self.stack.push(canonical);
            for entry in include_entries(include) {
                for (file, inputs) in self.resolve_include(&entry).map_err(in_file)? {
                    self.included += 1;
                    if self.included > MAX_INCLUDES {
                        return Err(format!(
                            "too many included files (limit is {})",
                            MAX_INCLUDES
                        ));
                    }
                    let included = self.load_file(&file, inputs.as_ref())?;
                    deep_merge(&mut merged, included);
                }
            }
            self.stack.pop();
        }
        deep_merge(&mut merged, Value::Mapping(body));
        Ok(merged)
    }

    /// Expand one `include:` entry into the local files it refers to,
    /// together with the inputs passed to them. Includes that would need
    /// network access are skipped with a warning.
    fn resolve_include(&self, entry: &Value) -> Result<Vec<(PathBuf, Option<Mapping>)>, String> {
        match entry {
            Value::String(s) if s.starts_with("http://") || s.starts_with("https://") => {
                Ok(skip_include("remote", s))
            }
            Value::String(s) => Ok(self
                .local_files(s)?
                .into_iter()
                .map(|file| (file, None))
                .collect()),
            Value::Mapping(map) => {
                let inputs = match map.get("inputs") {
                    None | Some(Value::Null) => None,
                    Some(Value::Mapping(inputs)) => Some(inputs.clone()),
                    Some(_) => return Err("`include:inputs` must be a mapping".to_string()),
                };
                if let Some(local) = map.get("local") {
                    let local = local.as_str().ok_or("`include:local` must be a string")?;
                    return Ok(self
                        .local_files(local)?
                        .into_iter()
                        .map(|file| (file, inputs.clone()))
                        .collect());
                }
                for kind in ["remote", "project", "template", "component"] {
                    if map.contains_key(kind) {
                        let target = map.get(kind).map(display_value).unwrap_or_default();
                        return Ok(skip_include(kind, &target));
                    }
                }
                Err("include entry must specify one of: local, remote, project, template, component"
                    .to_string())
            }
            _ => Err("include entries must be strings or mappings".to_string()),
        }
    }

    /// Resolve a local include path, which may contain glob wildcards, to the
    /// files it names. Paths are relative to the repository root.
    fn local_files(&self, pattern: &str) -> Result<Vec<PathBuf>, String> {
        let relative = pattern.trim_start_matches('/');
        let full = self.root.join(relative);

        let files = if relative.contains('*') {
            let full = full.to_string_lossy();
            let mut files = glob::glob(&full)
                .map_err(|e| format!("invalid include pattern '{}': {}", pattern, e))?
                .filter_map(Result::ok)
                .filter(|path| path.is_file())
                .collect::<Vec<_>>();
            files.sort();
            if files.is_empty() {
                return Err(format!("included pattern '{}' matched no files", pattern));
            }
            files
        } else {
            if !full.is_file() {
                return Err(format!("included file '{}' does not exist", pattern));
            }
            vec![full]
        };

        for file in &files {
            let canonical = file
                .canonicalize()
                .map_err(|e| format!("failed to resolve '{}': {}", file.display(), e))?;
            if !canonical.starts_with(&self.root) {
                return Err(format!(
                    "included file '{}' is outside the repository",
                    pattern
                ));
            }
        }
        Ok(files)
    }
}

fn skip_include(kind: &str, target: &str) -> Vec<(PathBuf, Option<Mapping>)> {
    wrkflw_logging::warning(&format!(
        "Skipping `include:{}` '{}': only local includes can be resolved when running a pipeline locally",
        kind, target
    ));
    Vec::new()
}

/// `include` accepts a single entry or a list of entries.
fn include_entries(include: Value) -> Vec<Value> {
    match include {
        Value::Sequence(entries) => entries,
        Value::Null => Vec::new(),
        entry => vec![entry],
    }
}

/// Split a file into its optional `spec` header and its configuration body.
fn split_documents(content: &str) -> Result<(Option<Value>, Value), String> {
    let mut documents = Vec::new();
    for document in serde_yaml::Deserializer::from_str(content) {
        let mut value = Value::deserialize(document).map_err(|e| e.to_string())?;
        value.apply_merge().map_err(|e| e.to_string())?;
        documents.push(value);
    }

    let is_header = |value: &Value| {
        value
            .as_mapping()
            .is_some_and(|map| map.len() == 1 && map.contains_key("spec"))
    };

    match documents.len() {
        0 => Ok((None, Value::Null)),
        1 => {
            let document = documents.remove(0);
            if document.get("spec").is_some() {
                return Err(
                    "`spec` must be in its own YAML document, separated from the configuration by `---`"
                        .to_string(),
                );
            }
            Ok((None, document))
        }
        2 if is_header(&documents[0]) => {
            let body = documents.remove(1);
            let header = documents.remove(0);
            Ok((header.get("spec").cloned(), body))
        }
        2 => Err(
            "a file with two YAML documents must start with a `spec` header document".to_string(),
        ),
        _ => Err(
            "a file may contain at most two YAML documents (a `spec` header and the configuration)"
                .to_string(),
        ),
    }
}

/// Work out the value of every input declared in `spec`, from the provided
/// inputs or the declared defaults.
fn resolve_inputs(
    spec: &Value,
    provided: Option<&Mapping>,
) -> Result<HashMap<String, Value>, String> {
    let declared = match spec.get("inputs") {
        None | Some(Value::Null) => Mapping::new(),
        Some(Value::Mapping(inputs)) => inputs.clone(),
        Some(_) => return Err("`spec:inputs` must be a mapping".to_string()),
    };

    if let Some(provided) = provided {
        for name in provided.keys() {
            if !declared.contains_key(name) {
                return Err(format!("unknown input '{}'", key_name(name)));
            }
        }
    }

    let mut values = HashMap::new();
    for (name, definition) in &declared {
        let name = key_name(name);
        let value = provided
            .and_then(|p| p.get(name.as_str()))
            .or_else(|| definition.get("default"))
            .cloned()
            .ok_or_else(|| format!("input '{}' is required but was not provided", name))?;

        let expected = definition
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("string");
        let type_matches = match expected {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "boolean" => value.is_bool(),
            "array" => value.is_sequence(),
            other => return Err(format!("input '{}' has unknown type '{}'", name, other)),
        };
        if !type_matches {
            return Err(format!("input '{}' must be of type {}", name, expected));
        }

        if let Some(Value::Sequence(options)) = definition.get("options") {
            if !options.contains(&value) {
                return Err(format!(
                    "input '{}' has value {} which is not one of the allowed options",
                    name,
                    display_value(&value)
                ));
            }
        }

        if let (Some(pattern), Some(text)) = (
            definition.get("regex").and_then(Value::as_str),
            value.as_str(),
        ) {
            let pattern = pattern.trim_matches('/');
            let regex = Regex::new(pattern)
                .map_err(|e| format!("input '{}' has an invalid regex: {}", name, e))?;
            if !regex.is_match(text) {
                return Err(format!(
                    "input '{}' does not match the required regex /{}/",
                    name, pattern
                ));
            }
        }

        values.insert(name, value);
    }
    Ok(values)
}

fn interpolation_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"\$\[\[\s*(.*?)\s*\]\]").expect("valid regex"))
}

/// Replace `$[[ inputs.NAME ]]` placeholders in every key and string value.
fn interpolate(value: &mut Value, inputs: &HashMap<String, Value>) -> Result<(), String> {
    match value {
        Value::String(text) => {
            if let Some(replacement) = interpolate_string(text, inputs)? {
                *value = replacement;
            }
        }
        Value::Sequence(items) => {
            for item in items {
                interpolate(item, inputs)?;
            }
        }
        Value::Mapping(map) => {
            let mut interpolated = Mapping::new();
            for (mut key, mut item) in std::mem::take(map) {
                interpolate(&mut key, inputs)?;
                interpolate(&mut item, inputs)?;
                interpolated.insert(key, item);
            }
            *map = interpolated;
        }
        Value::Tagged(tagged) => interpolate(&mut tagged.value, inputs)?,
        _ => {}
    }
    Ok(())
}

/// Interpolate one string. A string that consists of a single placeholder
/// takes the input's value with its original type; otherwise placeholders
/// are replaced with text.
fn interpolate_string(
    text: &str,
    inputs: &HashMap<String, Value>,
) -> Result<Option<Value>, String> {
    let pattern = interpolation_pattern();
    if !pattern.is_match(text) {
        return Ok(None);
    }

    if let Some(captures) = pattern.captures(text) {
        let whole = captures.get(0).expect("match exists");
        if whole.start() == 0 && whole.end() == text.len() {
            return evaluate_expression(&captures[1], inputs).map(Some);
        }
    }

    let mut result = String::with_capacity(text.len());
    let mut last = 0;
    for captures in pattern.captures_iter(text) {
        let whole = captures.get(0).expect("match exists");
        result.push_str(&text[last..whole.start()]);
        result.push_str(&display_value(&evaluate_expression(&captures[1], inputs)?));
        last = whole.end();
    }
    result.push_str(&text[last..]);
    Ok(Some(Value::String(result)))
}

/// Evaluate `inputs.NAME`, optionally followed by `| function` calls.
fn evaluate_expression(expression: &str, inputs: &HashMap<String, Value>) -> Result<Value, String> {
    let mut parts = expression.split('|').map(str::trim);
    let accessor = parts.next().unwrap_or_default();
    let name = accessor.strip_prefix("inputs.").ok_or_else(|| {
        format!(
            "unsupported interpolation '$[[ {} ]]'; only inputs can be interpolated",
            expression
        )
    })?;
    let mut value = inputs
        .get(name)
        .cloned()
        .ok_or_else(|| format!("unknown input '{}' in '$[[ {} ]]'", name, expression))?;

    for function in parts {
        let text = display_value(&value);
        value = Value::String(apply_function(function, text)?);
    }
    Ok(value)
}

fn apply_function(function: &str, text: String) -> Result<String, String> {
    if function == "expand_vars" {
        // Variables are expanded by the shell when the job runs.
        return Ok(text);
    }
    if function == "posix_escape" {
        return Ok(posix_escape(&text));
    }
    if let Some(args) = function
        .strip_prefix("truncate(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let numbers = args
            .split(',')
            .map(|arg| arg.trim().parse::<usize>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| format!("invalid arguments to '{}'", function))?;
        if let [offset, length] = numbers[..] {
            return Ok(text.chars().skip(offset).take(length).collect());
        }
        return Err(format!("'{}' takes two arguments", function));
    }
    Err(format!("unsupported interpolation function '{}'", function))
}

/// Escape a string so a POSIX shell treats it as a single literal word.
fn posix_escape(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => escaped.push_str("'\n'"),
            c if c.is_ascii_alphanumeric() || "_-.,:+/@".contains(c) => escaped.push(c),
            c => {
                escaped.push('\\');
                escaped.push(c);
            }
        }
    }
    escaped
}

/// Render a value the way it appears when interpolated into a string.
fn display_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn key_name(key: &Value) -> String {
    match key {
        Value::String(s) => s.clone(),
        other => display_value(other),
    }
}

/// Merge `overlay` into `base`: mappings are merged key by key, anything else
/// in `overlay` replaces what is in `base`. This is how GitLab combines both
/// included files and `extends` parents.
fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Mapping(base), Value::Mapping(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn is_job_key(name: &str) -> bool {
    !GLOBAL_KEYWORDS.contains(&name)
}

/// Apply `extends` to every job and hidden template.
fn resolve_extends(config: &mut Value) -> Result<(), String> {
    let Value::Mapping(map) = config else {
        return Ok(());
    };

    let names = map
        .iter()
        .filter(|(_, value)| value.get("extends").is_some())
        .filter_map(|(key, _)| key.as_str())
        .filter(|name| is_job_key(name))
        .map(String::from)
        .collect::<Vec<_>>();

    let mut resolved = HashMap::new();
    for name in &names {
        resolve_job(name, map, &mut resolved, &mut Vec::new())?;
    }
    for (name, job) in resolved {
        map.insert(Value::String(name), job);
    }
    Ok(())
}

fn resolve_job(
    name: &str,
    map: &Mapping,
    resolved: &mut HashMap<String, Value>,
    chain: &mut Vec<String>,
) -> Result<Value, String> {
    if let Some(job) = resolved.get(name) {
        return Ok(job.clone());
    }
    if chain.iter().any(|n| n == name) {
        chain.push(name.to_string());
        return Err(format!("circular `extends`: {}", chain.join(" -> ")));
    }
    if chain.len() >= MAX_EXTENDS_DEPTH {
        return Err(format!(
            "job '{}' exceeds the maximum `extends` depth of {}",
            chain[0], MAX_EXTENDS_DEPTH
        ));
    }

    let job = map
        .get(name)
        .ok_or_else(|| format!("job '{}' is not defined", name))?;
    let parents = match job.get("extends") {
        None => return Ok(job.clone()),
        Some(Value::String(parent)) => vec![parent.clone()],
        Some(Value::Sequence(parents)) => parents
            .iter()
            .map(|p| {
                p.as_str()
                    .map(String::from)
                    .ok_or_else(|| format!("job '{}' has a non-string `extends` entry", name))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(format!(
                "job '{}' has an invalid `extends`; expected a job name or a list of job names",
                name
            ))
        }
    };

    chain.push(name.to_string());
    let mut merged = Value::Mapping(Mapping::new());
    for parent in &parents {
        if !map.contains_key(parent.as_str()) {
            return Err(format!("job '{}' extends undefined job '{}'", name, parent));
        }
        let parent_job = resolve_job(parent, map, resolved, chain)?;
        deep_merge(&mut merged, parent_job);
    }
    chain.pop();

    let mut own = job.clone();
    if let Value::Mapping(own) = &mut own {
        own.remove("extends");
    }
    deep_merge(&mut merged, own);

    resolved.insert(name.to_string(), merged.clone());
    Ok(merged)
}

/// Replace every `!reference [key, ...]` tag with the value it points to.
fn resolve_references(config: &mut Value) -> Result<(), String> {
    let snapshot = config.clone();
    resolve_references_in(config, &snapshot, 0)
}

fn resolve_references_in(value: &mut Value, root: &Value, depth: usize) -> Result<(), String> {
    match value {
        Value::Tagged(tagged) if tagged.tag == "reference" => {
            let path = tagged
                .value
                .as_sequence()
                .map(|seq| seq.iter().map(Value::as_str).collect::<Option<Vec<_>>>())
                .unwrap_or_default()
                .filter(|path| !path.is_empty())
                .ok_or("`!reference` must be a non-empty list of keys")?;
            if depth >= MAX_REFERENCE_DEPTH {
                return Err(format!(
                    "`!reference [{}]` is nested more than {} levels deep",
                    path.join(", "),
                    MAX_REFERENCE_DEPTH
                ));
            }

            let mut target = root;
            for key in &path {
                target = target.get(key).ok_or_else(|| {
                    format!("`!reference [{}]` points to a missing key", path.join(", "))
                })?;
            }
            let mut target = target.clone();
            resolve_references_in(&mut target, root, depth + 1)?;
            *value = target;
        }
        Value::Tagged(tagged) => resolve_references_in(&mut tagged.value, root, depth)?,
        Value::Sequence(items) => {
            for item in items {
                resolve_references_in(item, root, depth)?;
            }
        }
        Value::Mapping(map) => {
            for (_, item) in map.iter_mut() {
                resolve_references_in(item, root, depth)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Drop hidden templates and flatten nested script lists, leaving only what
/// the pipeline model describes.
fn finalize(config: &mut Value) {
    let Value::Mapping(map) = config else {
        return;
    };
    map.retain(|key, _| !key.as_str().is_some_and(|name| name.starts_with('.')));

    flatten_scripts(map);
    for (key, value) in map.iter_mut() {
        let is_job = key.as_str().is_some_and(is_job_key);
        if is_job || key.as_str() == Some("default") {
            if let Value::Mapping(job) = value {
                flatten_scripts(job);
            }
        }
    }
}

/// GitLab flattens nested arrays in scripts, which is what YAML anchors and
/// `!reference` tags inside a script list produce.
fn flatten_scripts(map: &mut Mapping) {
    for key in SCRIPT_KEYS {
        if let Some(Value::Sequence(lines)) = map.get_mut(key) {
            let mut flat = Vec::with_capacity(lines.len());
            flatten_into(std::mem::take(lines), &mut flat);
            *lines = flat;
        }
    }
}

fn flatten_into(items: Vec<Value>, out: &mut Vec<Value>) {
    for item in items {
        match item {
            Value::Sequence(nested) => flatten_into(nested, out),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }

    fn load(content: &str) -> Result<Value, String> {
        let dir = TempDir::new().unwrap();
        load_pipeline(&write(&dir, ".gitlab-ci.yml", content))
    }

    fn script(config: &Value, job: &str) -> Vec<String> {
        config[job]["script"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn hidden_keys_are_templates_not_jobs() {
        let config = load(
            r#"
.base:
  image: alpine
  variables:
    A: from-base
    B: from-base
.extra:
  variables:
    B: from-extra
  script: [echo extra]
job:
  extends: [.base, .extra]
  variables:
    C: from-job
"#,
        )
        .unwrap();

        let map = config.as_mapping().unwrap();
        assert_eq!(map.len(), 1, "only the real job should remain");
        assert_eq!(config["job"]["image"], "alpine");
        assert_eq!(config["job"]["variables"]["A"], "from-base");
        assert_eq!(config["job"]["variables"]["B"], "from-extra");
        assert_eq!(config["job"]["variables"]["C"], "from-job");
        assert_eq!(script(&config, "job"), ["echo extra"]);
        assert!(config["job"].get("extends").is_none());
    }

    #[test]
    fn extends_is_resolved_transitively_and_job_keys_win() {
        let config = load(
            r#"
.a:
  script: [echo a]
  stage: build
.b:
  extends: .a
  stage: test
job:
  extends: .b
  script: [echo job]
"#,
        )
        .unwrap();
        assert_eq!(config["job"]["stage"], "test");
        assert_eq!(script(&config, "job"), ["echo job"]);
    }

    #[test]
    fn extends_errors_are_reported() {
        let err = load("job:\n  extends: .missing\n  script: [x]\n").unwrap_err();
        assert!(err.contains("extends undefined job '.missing'"), "{}", err);

        let err =
            load(".a:\n  extends: .b\n.b:\n  extends: .a\njob:\n  extends: .a\n").unwrap_err();
        assert!(err.contains("circular `extends`"), "{}", err);
    }

    #[test]
    fn local_includes_are_merged_with_main_file_taking_precedence() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "ci/a.yml",
            "variables:\n  SHARED: from-a\n  ONLY_A: a\n.tmpl:\n  script: [echo tmpl]\n",
        );
        write(&dir, "ci/b.yml", "job_b:\n  extends: .tmpl\n");
        let main = write(
            &dir,
            ".gitlab-ci.yml",
            r#"
include:
  - local: /ci/a.yml
  - ci/b.yml
variables:
  SHARED: from-main
job:
  extends: .tmpl
"#,
        );

        let config = load_pipeline(&main).unwrap();
        assert_eq!(config["variables"]["SHARED"], "from-main");
        assert_eq!(config["variables"]["ONLY_A"], "a");
        assert_eq!(script(&config, "job"), ["echo tmpl"]);
        assert_eq!(script(&config, "job_b"), ["echo tmpl"]);
        assert!(config.get("include").is_none());
    }

    #[test]
    fn local_includes_support_wildcards_and_single_entries() {
        let dir = TempDir::new().unwrap();
        write(&dir, "ci/one.yml", "one:\n  script: [echo 1]\n");
        write(&dir, "ci/two.yml", "two:\n  script: [echo 2]\n");
        let main = write(&dir, ".gitlab-ci.yml", "include: 'ci/*.yml'\n");

        let config = load_pipeline(&main).unwrap();
        assert_eq!(script(&config, "one"), ["echo 1"]);
        assert_eq!(script(&config, "two"), ["echo 2"]);
    }

    #[test]
    fn non_local_includes_are_skipped() {
        let config = load(
            r#"
include:
  - https://example.com/ci.yml
  - project: group/project
    file: ci.yml
  - template: Security/SAST.gitlab-ci.yml
job:
  script: [echo job]
"#,
        )
        .unwrap();
        assert_eq!(script(&config, "job"), ["echo job"]);
    }

    #[test]
    fn include_errors_are_reported() {
        let err = load("include: missing.yml\n").unwrap_err();
        assert!(err.contains("does not exist"), "{}", err);

        let dir = TempDir::new().unwrap();
        write(&dir, "a.yml", "include: b.yml\n");
        write(&dir, "b.yml", "include: a.yml\n");
        let main = write(&dir, ".gitlab-ci.yml", "include: a.yml\n");
        let err = load_pipeline(&main).unwrap_err();
        assert!(err.contains("include cycle"), "{}", err);
    }

    #[test]
    fn includes_cannot_escape_the_repository() {
        let outer = TempDir::new().unwrap();
        write(&outer, "secret.yml", "job:\n  script: [x]\n");
        let main = write(&outer, "repo/.gitlab-ci.yml", "include: ../secret.yml\n");
        let err = load_pipeline(&main).unwrap_err();
        assert!(err.contains("outside the repository"), "{}", err);
    }

    #[test]
    fn spec_inputs_use_defaults_and_values_from_include() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "ci/os.yml",
            r#"
spec:
  inputs:
    tag:
    count:
      type: number
      default: 1
    flag:
      type: boolean
      default: false
---
"build-$[[ inputs.tag ]]":
  tags:
    - $[[ inputs.tag ]]
  parallel: $[[ inputs.count ]]
  interruptible: $[[ inputs.flag ]]
  script:
    - echo "$[[ inputs.tag | truncate(0,3) ]] $[[ inputs.tag | posix_escape ]]"
"#,
        );
        let main = write(
            &dir,
            ".gitlab-ci.yml",
            r#"
spec:
  inputs:
    project-tag:
      default: "my tag"
      options: ["my tag", other]
---
include:
  - local: ci/os.yml
    inputs:
      tag: $[[ inputs.project-tag ]]
      count: 3
"#,
        );

        let config = load_pipeline(&main).unwrap();
        let job = &config["build-my tag"];
        assert_eq!(job["tags"][0], "my tag");
        assert_eq!(job["parallel"], 3, "whole-string inputs keep their type");
        assert_eq!(job["interruptible"], false);
        assert_eq!(script(&config, "build-my tag"), [r#"echo "my  my\ tag""#]);
    }

    #[test]
    fn spec_input_errors_are_reported() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "t.yml",
            "spec:\n  inputs:\n    env:\n      options: [dev, prod]\n---\njob:\n  script: [\"echo $[[ inputs.env ]]\"]\n",
        );
        let cases = [
            ("include: t.yml\n", "input 'env' is required"),
            (
                "include:\n  - local: t.yml\n    inputs: {env: qa}\n",
                "not one of the allowed options",
            ),
            (
                "include:\n  - local: t.yml\n    inputs: {env: dev, typo: x}\n",
                "unknown input 'typo'",
            ),
        ];
        for (main, expected) in cases {
            let main = write(&dir, ".gitlab-ci.yml", main);
            let err = load_pipeline(&main).unwrap_err();
            assert!(err.contains(expected), "{}: {}", expected, err);
        }

        let err = load("spec:\n  inputs: {}\njob:\n  script: [x]\n").unwrap_err();
        assert!(err.contains("own YAML document"), "{}", err);
    }

    #[test]
    fn references_and_anchors_are_flattened_into_scripts() {
        let config = load(
            r#"
.setup:
  script:
    - echo setup
.anchors:
  lines: &lines
    - echo anchored
job:
  before_script:
    - !reference [.setup, script]
  script:
    - *lines
    - echo own
"#,
        )
        .unwrap();
        assert_eq!(config["job"]["before_script"][0], "echo setup");
        assert_eq!(script(&config, "job"), ["echo anchored", "echo own"]);

        let err = load("job:\n  script:\n    - !reference [.nope, script]\n").unwrap_err();
        assert!(err.contains("points to a missing key"), "{}", err);
    }
}
