use serde::Serialize;

/// One structured validation finding, for `--format json` consumers.
///
/// `message` is the same text as the matching entry in
/// [`ValidationResult::issues`], so human and machine output never disagree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationIssue {
    /// Stable kebab-case identifier of the check, e.g. `unknown-trigger`.
    pub rule: String,
    pub message: String,
    /// Where in the document the issue is, e.g. `jobs.build.steps[0].uses`.
    pub path: Option<String>,
    /// 1-based line of `path`, when [`ValidationResult::locate_lines`] found it.
    pub line: Option<usize>,
    /// How to fix it, when the rule has a generic answer.
    pub hint: Option<String>,
    /// `path` split into keys and `[n]` indexes. Job names may contain dots,
    /// so the joined form cannot be split back reliably.
    #[serde(skip)]
    segments: Vec<String>,
}

/// The rule used by [`ValidationResult::add_issue`] callers that have not
/// been classified.
pub const UNCLASSIFIED_RULE: &str = "unclassified";

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub is_valid: bool,
    pub issues: Vec<String>,
    /// Structured form of `issues`, in the same order.
    pub details: Vec<ValidationIssue>,
    scope: Vec<String>,
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationResult {
    pub fn new() -> Self {
        ValidationResult {
            is_valid: true,
            issues: Vec::new(),
            details: Vec::new(),
            scope: Vec::new(),
        }
    }

    pub fn add_issue(&mut self, issue: String) {
        self.report(UNCLASSIFIED_RULE, issue);
    }

    /// Record an issue at the current scope under a stable rule id.
    pub fn report(&mut self, rule: &str, message: impl Into<String>) {
        let segments = self.scope.clone();
        self.push_issue(rule, message.into(), segments);
    }

    /// Record an issue at `key` below the current scope.
    pub fn report_at(&mut self, rule: &str, key: &str, message: impl Into<String>) {
        let mut segments = self.scope.clone();
        segments.push(key.to_string());
        self.push_issue(rule, message.into(), segments);
    }

    /// Enter a YAML key for everything reported until the matching
    /// [`pop_scope`](Self::pop_scope).
    pub fn push_scope(&mut self, key: impl Into<String>) {
        self.scope.push(key.into());
    }

    /// Enter a sequence index, rendered as `[index]`.
    pub fn push_index(&mut self, index: usize) {
        self.scope.push(format!("[{}]", index));
    }

    pub fn pop_scope(&mut self) {
        self.scope.pop();
    }

    /// Run `f` with `key` pushed. Early returns inside `f` stay balanced.
    pub fn scoped<R>(&mut self, key: impl Into<String>, f: impl FnOnce(&mut Self) -> R) -> R {
        self.push_scope(key);
        let out = f(self);
        self.pop_scope();
        out
    }

    /// Fill in `line` on every issue by finding its path in `source`.
    pub fn locate_lines(&mut self, source: &str) {
        for issue in &mut self.details {
            issue.line = yaml_line_for_path(source, &issue.segments);
        }
    }

    fn push_issue(&mut self, rule: &str, message: String, segments: Vec<String>) {
        self.is_valid = false;
        self.issues.push(message.clone());
        self.details.push(ValidationIssue {
            rule: rule.to_string(),
            message,
            path: render_path(&segments),
            line: None,
            hint: rule_hint(rule).map(str::to_string),
            segments,
        });
    }
}

fn render_path(segments: &[String]) -> Option<String> {
    let mut out = String::new();
    for segment in segments {
        if !segment.starts_with('[') && !out.is_empty() {
            out.push('.');
        }
        out.push_str(segment);
    }
    (!out.is_empty()).then_some(out)
}

/// Generic fix for a rule. Rules without one return `None`, which is better
/// than a hint that restates the message.
pub fn rule_hint(rule: &str) -> Option<&'static str> {
    Some(match rule {
        "missing-jobs" => "Add a top-level `jobs:` mapping with at least one job.",
        "missing-triggers" => "Add a top-level `on:` section, e.g. `on: [push]`.",
        "unknown-trigger" => "Use an event from the GitHub Actions events list, e.g. `push`.",
        "missing-runs-on" => {
            "Add `runs-on: ubuntu-latest`, or call a reusable workflow with `uses:`."
        }
        "missing-steps" => "Add a `steps:` list, or call a reusable workflow with `uses:`.",
        "step-missing-action" => "Give the step either a `run:` command or a `uses:` action.",
        "undefined-need" => "Point `needs:` at a job id defined in this workflow.",
        "undefined-stage" => "Add the stage to `stages:`, or change the job's `stage:`.",
        "undefined-dependency" => {
            "Point `needs:` or `dependencies:` at a job defined in this pipeline."
        }
        "undefined-extends" => "Define the job named by `extends:`, or remove the reference.",
        "empty-pipeline" => "Add at least one job with a `script:` section.",
        "env-not-mapping" => "Write `env:` as `KEY: value` pairs, or a single `${{ }}` expression.",
        "action-missing-version" => {
            "Pin the action to a tag or commit SHA, e.g. `actions/checkout@v4`."
        }
        "step-uses-and-run" => "Split it into two steps, one with `uses:` and one with `run:`.",
        "missing-cron" => "Add a `cron:` entry, e.g. `- cron: '0 3 * * 1'`.",
        "invalid-cron" => "Use five fields: minute hour day month day-of-week, e.g. `0 3 * * 1`.",
        "circular-needs" => "Remove a `needs:` entry so the jobs no longer form a loop.",
        "circular-extends" => "Remove an `extends:` entry so the jobs no longer form a loop.",
        "empty-artifact-paths" => {
            "List files under `artifacts:paths:`, or add `artifacts:reports:`."
        }
        _ => return None,
    })
}

/// Find the 1-based line of a YAML path in block-style `source`.
///
/// `serde_yaml` drops source marks, so this walks the text by indentation.
/// Flow-style YAML (`{a: b}`) and anchors return the deepest parent it can
/// reach, or `None` when not even the first key exists. A wrong line would
/// be worse than none, so anything ambiguous stops the walk.
pub fn yaml_line_for_path(source: &str, segments: &[String]) -> Option<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut start = 0usize;
    let mut end = lines.len();
    let mut parent_indent: Option<usize> = None;
    let mut found = None;

    for segment in segments {
        let hit = if let Some(index) = segment
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .and_then(|s| s.parse::<usize>().ok())
        {
            find_sequence_item(&lines, start, end, parent_indent, index)
        } else {
            find_key(&lines, start, end, parent_indent, segment)
        };
        let Some((line, indent)) = hit else {
            return found;
        };
        found = Some(line + 1);
        // Keys inside a sequence item sit deeper than its dash, on the same
        // line as the dash for the first key, so the search reopens there.
        let is_item = segment.starts_with('[');
        start = if is_item { line } else { line + 1 };
        end = block_end(&lines, line, indent, is_item, end);
        parent_indent = Some(indent);
    }
    found
}

/// Indent of a line, or `None` for blanks and comments.
fn content_indent(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    (!trimmed.is_empty() && !trimmed.starts_with('#')).then(|| line.len() - trimmed.len())
}

fn find_key(
    lines: &[&str],
    start: usize,
    end: usize,
    parent: Option<usize>,
    key: &str,
) -> Option<(usize, usize)> {
    let mut child_indent = None;
    for (offset, line) in lines[start..end].iter().enumerate() {
        let Some(mut indent) = content_indent(line) else {
            continue;
        };
        let mut text = line.trim_start();
        // First key of a sequence item: `- name: x`.
        while let Some(rest) = text.strip_prefix("- ") {
            let spaces = rest.len() - rest.trim_start().len();
            indent += 2 + spaces;
            text = rest.trim_start();
        }
        if parent.is_some_and(|p| indent <= p) {
            continue;
        }
        let expected = *child_indent.get_or_insert(indent);
        if indent != expected {
            continue;
        }
        let name = text.split(':').next().unwrap_or("").trim();
        let bare = name
            .trim_matches(|c| c == '"' || c == '\'')
            .trim_matches('\'');
        if bare == key && text[name.len()..].trim_start().starts_with(':') {
            return Some((start + offset, indent));
        }
    }
    None
}

fn find_sequence_item(
    lines: &[&str],
    start: usize,
    end: usize,
    parent: Option<usize>,
    index: usize,
) -> Option<(usize, usize)> {
    let mut dash_indent = None;
    let mut seen = 0;
    for (offset, line) in lines[start..end].iter().enumerate() {
        let Some(indent) = content_indent(line) else {
            continue;
        };
        if !line.trim_start().starts_with("- ") {
            continue;
        }
        if parent.is_some_and(|p| indent < p) {
            continue;
        }
        let expected = *dash_indent.get_or_insert(indent);
        if indent != expected {
            continue;
        }
        if seen == index {
            return Some((start + offset, indent));
        }
        seen += 1;
    }
    None
}

/// End of the block opened at `line`: the next line at or above its indent.
fn block_end(lines: &[&str], line: usize, indent: usize, is_item: bool, limit: usize) -> usize {
    for (offset, text) in lines[line + 1..limit].iter().enumerate() {
        let Some(next) = content_indent(text) else {
            continue;
        };
        // YAML allows a list at the same indent as its key, so a dash at the
        // key's indent still belongs to that key.
        let same_level_item = !is_item && next == indent && text.trim_start().starts_with("- ");
        if next <= indent && !same_level_item {
            return line + 1 + offset;
        }
    }
    limit
}

#[cfg(test)]
mod validation_tests {
    use super::*;

    const YAML: &str = "name: ci\non: [push]\n\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: one\n        run: echo 1\n      # comment\n      - uses: actions/checkout@v4\n  test-suite:\n    steps:\n    - run: echo 2\n    - name: x\n      run: echo 3\n";

    fn line(path: &[&str]) -> Option<usize> {
        let segments: Vec<String> = path.iter().map(|s| s.to_string()).collect();
        yaml_line_for_path(YAML, &segments)
    }

    #[test]
    fn locates_top_level_keys_and_nested_jobs() {
        assert_eq!(line(&["on"]), Some(2));
        assert_eq!(line(&["jobs", "test-suite"]), Some(12));
        assert_eq!(line(&["jobs", "build", "runs-on"]), Some(6));
    }

    #[test]
    fn locates_sequence_items_and_their_keys() {
        assert_eq!(line(&["jobs", "build", "steps", "[1]"]), Some(11));
        assert_eq!(line(&["jobs", "build", "steps", "[0]", "run"]), Some(9));
        assert_eq!(line(&["jobs", "build", "steps", "[1]", "uses"]), Some(11));
    }

    #[test]
    fn locates_sequences_at_the_same_indent_as_their_key() {
        assert_eq!(
            line(&["jobs", "test-suite", "steps", "[1]", "run"]),
            Some(16)
        );
    }

    #[test]
    fn stops_at_the_deepest_known_parent() {
        assert_eq!(line(&["jobs", "build", "nope"]), Some(5));
        assert_eq!(line(&["nope"]), None);
    }

    #[test]
    fn does_not_cross_into_a_sibling_job() {
        assert_eq!(line(&["jobs", "test-suite", "runs-on"]), Some(12));
    }

    #[test]
    fn report_uses_scope_and_keeps_issues_in_sync() {
        let mut result = ValidationResult::new();
        result.scoped("jobs", |r| {
            r.scoped("build", |r| {
                r.push_scope("steps");
                r.push_index(1);
                r.report_at("step-missing-action", "uses", "bad");
                r.pop_scope();
                r.pop_scope();
            })
        });
        result.add_issue("legacy".to_string());
        result.locate_lines(YAML);

        assert!(!result.is_valid);
        assert_eq!(result.issues, vec!["bad", "legacy"]);
        let issue = &result.details[0];
        assert_eq!(issue.path.as_deref(), Some("jobs.build.steps[1].uses"));
        assert_eq!(issue.line, Some(11));
        assert!(issue.hint.is_some());
        assert_eq!(result.details[1].rule, UNCLASSIFIED_RULE);
        assert_eq!(result.details[1].path, None);
    }
}

// GitLab pipeline models
pub mod gitlab {
    use serde::{Deserialize, Deserializer, Serialize};
    use std::collections::HashMap;

    fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrVec {
            String(String),
            Vec(Vec<String>),
        }
        let value = Option::<StringOrVec>::deserialize(deserializer)?;
        match value {
            Some(StringOrVec::String(s)) => Ok(Some(vec![s])),
            Some(StringOrVec::Vec(v)) => Ok(Some(v)),
            None => Ok(None),
        }
    }

    fn deserialize_variables<'de, D>(
        deserializer: D,
    ) -> Result<Option<HashMap<String, String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let opt: Option<HashMap<String, serde_yaml::Value>> = Option::deserialize(deserializer)?;
        Ok(opt.map(|m| {
            m.into_iter()
                .map(|(k, v)| {
                    let s = match v {
                        serde_yaml::Value::String(s) => s,
                        serde_yaml::Value::Number(n) => n.to_string(),
                        serde_yaml::Value::Bool(b) => b.to_string(),
                        serde_yaml::Value::Null => String::new(),
                        other => serde_yaml::to_string(&other)
                            .map(|s| s.trim().to_string())
                            .unwrap_or_default(),
                    };
                    (k, s)
                })
                .collect()
        }))
    }

    /// Represents a GitLab CI/CD pipeline configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Pipeline {
        /// Default image for all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image: Option<Image>,

        /// Global variables available to all jobs
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_variables"
        )]
        pub variables: Option<HashMap<String, String>>,

        /// Pipeline stages in execution order
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stages: Option<Vec<String>>,

        /// Default before_script for all jobs
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub before_script: Option<Vec<String>>,

        /// Default after_script for all jobs
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub after_script: Option<Vec<String>>,

        /// Default settings for all jobs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub default: Option<serde_yaml::Value>,

        /// Job definitions (name => job)
        #[serde(flatten)]
        pub jobs: HashMap<String, Job>,

        /// Workflow rules for the pipeline
        #[serde(skip_serializing_if = "Option::is_none")]
        pub workflow: Option<Workflow>,

        /// Includes for pipeline configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub include: Option<Vec<Include>>,
    }

    /// A job in a GitLab CI/CD pipeline
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Job {
        /// The stage this job belongs to
        #[serde(skip_serializing_if = "Option::is_none")]
        pub stage: Option<String>,

        /// Docker image to use for this job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub image: Option<Image>,

        /// Script commands to run
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub script: Option<Vec<String>>,

        /// Commands to run before the main script
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub before_script: Option<Vec<String>>,

        /// Commands to run after the main script
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub after_script: Option<Vec<String>>,

        /// When to run the job (on_success, on_failure, always, manual)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,

        /// Allow job failure
        #[serde(skip_serializing_if = "Option::is_none")]
        pub allow_failure: Option<AllowFailure>,

        /// Services to run alongside the job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub services: Option<Vec<Service>>,

        /// Tags to define which runners can execute this job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub tags: Option<Vec<String>>,

        /// Job-specific variables
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_variables"
        )]
        pub variables: Option<HashMap<String, String>>,

        /// Job dependencies
        #[serde(skip_serializing_if = "Option::is_none")]
        pub dependencies: Option<Vec<String>>,

        /// Artifacts to store after job execution
        #[serde(skip_serializing_if = "Option::is_none")]
        pub artifacts: Option<Artifacts>,

        /// Cache configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub cache: Option<Cache>,

        /// Rules for when this job should run
        #[serde(skip_serializing_if = "Option::is_none")]
        pub rules: Option<Vec<Rule>>,

        /// Only run on specified refs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub only: Option<Only>,

        /// Exclude specified refs
        #[serde(skip_serializing_if = "Option::is_none")]
        pub except: Option<Except>,

        /// Retry configuration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub retry: Option<Retry>,

        /// Timeout for the job in seconds
        #[serde(skip_serializing_if = "Option::is_none")]
        pub timeout: Option<String>,

        /// Run the job several times: a fixed count, or once per `parallel:matrix` combination
        #[serde(skip_serializing_if = "Option::is_none")]
        pub parallel: Option<Parallel>,

        /// Flag to indicate this is a template job
        #[serde(skip_serializing_if = "Option::is_none")]
        pub template: Option<bool>,

        /// List of jobs this job extends from
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_string_or_vec"
        )]
        pub extends: Option<Vec<String>>,

        /// Job needs (dependencies with more granular control)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub needs: Option<serde_yaml::Value>,

        /// Whether the job can be interrupted by a newer pipeline
        #[serde(skip_serializing_if = "Option::is_none")]
        pub interruptible: Option<bool>,

        /// Downstream pipeline to trigger instead of running a script
        #[serde(skip_serializing_if = "Option::is_none")]
        pub trigger: Option<serde_yaml::Value>,
    }

    /// Whether a job may fail without failing the pipeline
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum AllowFailure {
        /// Allow (or disallow) failure for any exit code
        Bool(bool),
        /// Allow failure only for specific exit codes
        ExitCodes {
            /// A single exit code or a list of exit codes
            exit_codes: ExitCodes,
        },
    }

    /// How many copies of a job to run
    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    #[serde(untagged)]
    pub enum Parallel {
        /// A fixed number of instances
        Count(usize),
        /// One instance per combination of variables
        Matrix {
            /// Each entry's variables are a value or a list of values
            matrix: Vec<HashMap<String, serde_yaml::Value>>,
        },
    }

    /// One or more process exit codes
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum ExitCodes {
        /// A single exit code
        Single(i32),
        /// A list of exit codes
        Multiple(Vec<i32>),
    }

    /// Docker image configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Image {
        /// Simple image name as string
        Simple(String),
        /// Detailed image configuration
        Detailed {
            /// Image name
            name: String,
            /// Entrypoint to override in the image
            #[serde(skip_serializing_if = "Option::is_none")]
            entrypoint: Option<Vec<String>>,
        },
    }

    /// Service container to run alongside a job
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Service {
        /// Simple service name as string
        Simple(String),
        /// Detailed service configuration
        Detailed {
            /// Service name/image
            name: String,
            /// Command to run in the service container
            #[serde(skip_serializing_if = "Option::is_none")]
            command: Option<Vec<String>>,
            /// Entrypoint to override in the image
            #[serde(skip_serializing_if = "Option::is_none")]
            entrypoint: Option<Vec<String>>,
        },
    }

    /// Artifacts configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Artifacts {
        /// Paths to include as artifacts
        #[serde(skip_serializing_if = "Option::is_none")]
        pub paths: Option<Vec<String>>,
        /// Artifact expiration duration
        #[serde(skip_serializing_if = "Option::is_none")]
        pub expire_in: Option<String>,
        /// When to upload artifacts (on_success, on_failure, always)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
        /// Reports configuration (e.g., junit, coverage_report)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reports: Option<serde_yaml::Value>,
    }

    /// Cache key configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum CacheKey {
        /// Simple string key
        Simple(String),
        /// Structured key with files and optional prefix
        Structured {
            /// Files to use for cache key generation
            files: Vec<String>,
            /// Optional prefix for the cache key
            #[serde(skip_serializing_if = "Option::is_none")]
            prefix: Option<String>,
        },
    }

    /// Cache configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Cache {
        /// Cache key
        #[serde(skip_serializing_if = "Option::is_none")]
        pub key: Option<CacheKey>,
        /// Paths to cache
        #[serde(skip_serializing_if = "Option::is_none")]
        pub paths: Option<Vec<String>>,
        /// When to save cache (on_success, on_failure, always)
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
        /// Cache policy
        #[serde(skip_serializing_if = "Option::is_none")]
        pub policy: Option<String>,
    }

    /// Rule for conditional job execution
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Rule {
        /// If condition expression
        #[serde(rename = "if", skip_serializing_if = "Option::is_none")]
        pub if_: Option<String>,
        /// When to run if condition is true
        #[serde(skip_serializing_if = "Option::is_none")]
        pub when: Option<String>,
        /// Variables to set if condition is true
        #[serde(skip_serializing_if = "Option::is_none")]
        pub variables: Option<HashMap<String, String>>,
        /// Overrides the job's `allow_failure` when this rule matches
        #[serde(skip_serializing_if = "Option::is_none")]
        pub allow_failure: Option<AllowFailure>,
        /// Files whose changes the rule depends on. Not evaluated.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub changes: Option<serde_yaml::Value>,
        /// Files whose presence the rule depends on. Not evaluated.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub exists: Option<serde_yaml::Value>,
    }

    /// Only/except configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Only {
        /// Simple list of refs
        Refs(Vec<String>),
        /// Detailed configuration
        Complex {
            /// Refs to include
            #[serde(skip_serializing_if = "Option::is_none")]
            refs: Option<Vec<String>>,
            /// Branch patterns to include
            #[serde(skip_serializing_if = "Option::is_none")]
            branches: Option<Vec<String>>,
            /// Tags to include
            #[serde(skip_serializing_if = "Option::is_none")]
            tags: Option<Vec<String>>,
            /// Pipeline types to include
            #[serde(skip_serializing_if = "Option::is_none")]
            variables: Option<Vec<String>>,
            /// Changes to files that trigger the job
            #[serde(skip_serializing_if = "Option::is_none")]
            changes: Option<Vec<String>>,
        },
    }

    /// Except configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Except {
        /// Simple list of refs
        Refs(Vec<String>),
        /// Detailed configuration
        Complex {
            /// Refs to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            refs: Option<Vec<String>>,
            /// Branch patterns to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            branches: Option<Vec<String>>,
            /// Tags to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            tags: Option<Vec<String>>,
            /// Pipeline types to exclude
            #[serde(skip_serializing_if = "Option::is_none")]
            variables: Option<Vec<String>>,
            /// Changes to files that don't trigger the job
            #[serde(skip_serializing_if = "Option::is_none")]
            changes: Option<Vec<String>>,
        },
    }

    /// Workflow configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    pub struct Workflow {
        /// Rules for when to run the pipeline
        pub rules: Vec<Rule>,
    }

    /// Retry configuration
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Retry {
        /// Simple max attempts
        MaxAttempts(u32),
        /// Detailed retry configuration
        Detailed {
            /// Maximum retry attempts
            max: u32,
            /// When to retry
            #[serde(skip_serializing_if = "Option::is_none")]
            when: Option<Vec<String>>,
        },
    }

    /// Include configuration for external pipeline files
    #[derive(Debug, Serialize, Deserialize, Clone)]
    #[serde(untagged)]
    pub enum Include {
        /// Simple string include
        Local(String),
        /// Detailed include configuration
        Detailed {
            /// Local file path
            #[serde(skip_serializing_if = "Option::is_none")]
            local: Option<String>,
            /// Remote file URL
            #[serde(skip_serializing_if = "Option::is_none")]
            remote: Option<String>,
            /// Include from project
            #[serde(skip_serializing_if = "Option::is_none")]
            project: Option<String>,
            /// Include specific file from project
            #[serde(skip_serializing_if = "Option::is_none")]
            file: Option<String>,
            /// Include template
            #[serde(skip_serializing_if = "Option::is_none")]
            template: Option<String>,
            /// Ref to use when including from project
            #[serde(skip_serializing_if = "Option::is_none")]
            ref_: Option<String>,
        },
    }
}
