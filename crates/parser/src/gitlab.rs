use crate::gitlab_config;
use crate::schema::{SchemaType, SchemaValidator};
use crate::workflow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use thiserror::Error;
use wrkflw_matrix::MatrixConfig;
use wrkflw_models::gitlab::{AllowFailure, Except, Image, Job, Only, Parallel, Pipeline, Rule};
use wrkflw_models::ValidationResult;

#[derive(Error, Debug)]
pub enum GitlabParserError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("YAML parsing error: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Invalid pipeline structure: {0}")]
    InvalidStructure(String),

    #[error("Schema validation error: {0}")]
    SchemaValidationError(String),
}

/// Parse a GitLab CI/CD pipeline file
///
/// Local includes, `spec:inputs`, `extends`, `!reference` tags and hidden
/// (`.`-prefixed) templates are resolved first, so the returned pipeline
/// contains only the jobs GitLab would actually create.
pub fn parse_pipeline(pipeline_path: &Path) -> Result<Pipeline, GitlabParserError> {
    // Assemble the full configuration from the pipeline file and its includes
    let config =
        gitlab_config::load_pipeline(pipeline_path).map_err(GitlabParserError::InvalidStructure)?;

    // Validate against schema
    let validator = SchemaValidator::new().map_err(GitlabParserError::SchemaValidationError)?;
    let config_json = serde_json::to_value(&config)
        .map_err(|e| GitlabParserError::InvalidStructure(e.to_string()))?;

    validator
        .validate_json(&config_json, SchemaType::GitLab)
        .map_err(GitlabParserError::SchemaValidationError)?;

    // Parse the resolved configuration
    let pipeline: Pipeline = serde_yaml::from_value(config)?;

    check_parallel(&pipeline).map_err(GitlabParserError::InvalidStructure)?;

    // Return the parsed pipeline
    Ok(pipeline)
}

/// Validate the basic structure of a GitLab CI/CD pipeline
pub fn validate_pipeline_structure(pipeline: &Pipeline) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Check for at least one job
    if pipeline.jobs.is_empty() {
        result.add_issue("Pipeline must contain at least one job".to_string());
    }

    // Check for script in jobs
    for (job_name, job) in &pipeline.jobs {
        // Skip template jobs
        if let Some(true) = job.template {
            continue;
        }

        // Check for script or extends
        if job.script.is_none() && job.extends.is_none() && job.trigger.is_none() {
            result.add_issue(format!(
                "Job '{}' must have a script or trigger section or extend another job",
                job_name
            ));
        }
    }

    // Check that referenced stages are defined
    if let Some(stages) = &pipeline.stages {
        for (job_name, job) in &pipeline.jobs {
            if let Some(stage) = &job.stage {
                if !stages.contains(stage) {
                    result.add_issue(format!(
                        "Job '{}' references undefined stage '{}'",
                        job_name, stage
                    ));
                }
            }
        }
    }

    // Check that job dependencies exist
    for (job_name, job) in &pipeline.jobs {
        if let Some(dependencies) = &job.dependencies {
            for dependency in dependencies {
                if !pipeline.jobs.contains_key(dependency) {
                    result.add_issue(format!(
                        "Job '{}' depends on undefined job '{}'",
                        job_name, dependency
                    ));
                }
            }
        }
    }

    // Check that job extensions exist
    for (job_name, job) in &pipeline.jobs {
        if let Some(extends) = &job.extends {
            for extend in extends {
                if !pipeline.jobs.contains_key(extend) {
                    result.add_issue(format!(
                        "Job '{}' extends undefined job '{}'",
                        job_name, extend
                    ));
                }
            }
        }
    }

    result
}

/// Runs a job's scripts in bash when its image has it and in `sh` otherwise,
/// as GitLab does. An internal value for `defaults.run.shell`: `execute_step`
/// must match it before its custom-shell fallback, and it never reaches the
/// GitHub workflow schema, whose shell enum doesn't list it.
pub const SHELL_BASH_OR_SH: &str = "wrkflw-bash-or-sh";

/// When a job runs relative to the outcome of the stages before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobWhen {
    OnSuccess,
    OnFailure,
    Always,
    /// Excluded from the pipeline whatever the earlier stages did.
    Skip(SkipReason),
}

/// Why a job is excluded from the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// `when: never`.
    Never,
    /// `when: manual`: nobody can press play on a local run.
    Manual,
    /// A manual job that later stages wait for, which here never happens.
    BlockingManual,
    /// The job has `rules:` and none of them matched.
    NoRuleMatched,
    /// `only:` doesn't select this run, or `except:` excludes it.
    RefFilter,
    /// A `rules:if` or `only:`/`except:` condition couldn't be evaluated.
    InvalidCondition,
    /// A `rules:if` reads a `parallel:matrix` variable. Rules are decided once
    /// per job, so no single answer holds for every instance.
    MatrixCondition,
    /// A `when:` value that isn't one of GitLab's.
    UnknownWhen,
}

/// What a job's rules decided for this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JobDecision {
    pub when: JobWhen,
    /// The matched rule's `allow_failure`, which overrides the job's own.
    pub allow_failure: Option<bool>,
}

impl JobWhen {
    fn parse(when: Option<&str>) -> Self {
        match when {
            None | Some("on_success") | Some("delayed") => JobWhen::OnSuccess,
            Some("on_failure") => JobWhen::OnFailure,
            Some("always") => JobWhen::Always,
            Some("never") => JobWhen::Skip(SkipReason::Never),
            Some("manual") => JobWhen::Skip(SkipReason::Manual),
            Some(_) => JobWhen::Skip(SkipReason::UnknownWhen),
        }
    }

    /// Whether later stages wait for this job, as they do for a manual job
    /// that isn't allowed to fail.
    pub fn blocks_later_stages(self) -> bool {
        self == JobWhen::Skip(SkipReason::BlockingManual)
    }

    /// Why this job doesn't run given the outcome of the earlier stages, or
    /// `None` if it does.
    pub fn skip_reason(self, earlier_failure: bool) -> Option<&'static str> {
        match self {
            JobWhen::Always => None,
            JobWhen::OnSuccess if earlier_failure => Some("an earlier stage failed"),
            JobWhen::OnSuccess => None,
            JobWhen::OnFailure if earlier_failure => None,
            JobWhen::OnFailure => Some("when: on_failure and no earlier stage failed"),
            JobWhen::Skip(SkipReason::Never) => Some("when: never"),
            JobWhen::Skip(SkipReason::Manual) => {
                Some("when: manual jobs can't be started on a local run")
            }
            JobWhen::Skip(SkipReason::BlockingManual) => {
                Some("when: manual jobs can't be started on a local run, so later stages wait")
            }
            JobWhen::Skip(SkipReason::MatrixCondition) => {
                Some("a rule reads a parallel:matrix variable, which isn't evaluated")
            }
            JobWhen::Skip(SkipReason::NoRuleMatched) => Some("no rule matched"),
            JobWhen::Skip(SkipReason::RefFilter) => Some("only/except excludes this run"),
            JobWhen::Skip(SkipReason::InvalidCondition) => {
                Some("a condition couldn't be evaluated")
            }
            JobWhen::Skip(SkipReason::UnknownWhen) => Some("unknown when value"),
        }
    }
}

/// Decide when a job runs. The first matching rule wins and a job whose rules
/// all miss is excluded, as in GitLab. A job without `rules:` goes through its
/// `only:`/`except:`. A condition that can't be evaluated excludes the job,
/// because guessing could run a guarded deploy. `variables` are the job's CI
/// variables.
pub fn job_decision(job_name: &str, job: &Job, variables: &HashMap<String, String>) -> JobDecision {
    let excluded = |reason| JobDecision {
        when: JobWhen::Skip(reason),
        allow_failure: None,
    };
    let owner = format!("Job '{}'", job_name);
    if job.needs.is_some() {
        wrkflw_logging::warning(&format!(
            "{}: needs isn't evaluated, so the job follows stage order",
            owner
        ));
    }
    if let Some(variable) = matrix_variable_in_rules(job) {
        wrkflw_logging::warning(&format!(
            "{} is excluded: its rules read parallel:matrix variable '{}', and rules aren't \
             evaluated per instance. Name it with --job to run every instance",
            owner, variable
        ));
        return excluded(SkipReason::MatrixCondition);
    }
    let job_level = job.when.as_deref();
    let (when, rule_allow_failure) = match &job.rules {
        Some(rules) => match first_matching_rule(&owner, rules, variables) {
            Ok(Some(rule)) => (
                rule.when.as_deref().or(job_level),
                rule.allow_failure.as_ref().and_then(allow_failure_flag),
            ),
            Ok(None) => return excluded(SkipReason::NoRuleMatched),
            Err(reason) => return excluded(reason),
        },
        None => {
            if let Some(reason) = ref_filter_exclusion(&owner, job, variables) {
                return excluded(reason);
            }
            (job_level, None)
        }
    };
    if when == Some("delayed") {
        wrkflw_logging::warning(&format!(
            "{}: when: delayed runs without waiting for start_in",
            owner
        ));
    }
    // As on GitLab, only a job-level `when: manual` defaults to being allowed
    // to fail, so a manual rule blocks unless something says otherwise.
    let allowed = rule_allow_failure
        .or(job.allow_failure.as_ref().and_then(allow_failure_flag))
        .unwrap_or(job_level == Some("manual"));
    let when = match JobWhen::parse(when) {
        JobWhen::Skip(SkipReason::Manual) if !allowed => JobWhen::Skip(SkipReason::BlockingManual),
        when => when,
    };
    JobDecision {
        when,
        allow_failure: rule_allow_failure,
    }
}

fn allow_failure_flag(allow_failure: &AllowFailure) -> Option<bool> {
    match allow_failure {
        AllowFailure::Bool(allowed) => Some(*allowed),
        AllowFailure::ExitCodes { .. } => None,
    }
}

/// A `parallel:matrix` variable that one of the job's `rules:if` reads.
fn matrix_variable_in_rules(job: &Job) -> Option<String> {
    let Some(Parallel::Matrix { matrix }) = &job.parallel else {
        return None;
    };
    job.rules
        .iter()
        .flatten()
        .filter_map(|rule| rule.if_.as_deref())
        .flat_map(|condition| tokenize_rule_condition(condition).unwrap_or_default())
        .find_map(|token| match token {
            RuleToken::Variable(name) if matrix.iter().any(|entry| entry.contains_key(&name)) => {
                Some(name)
            }
            _ => None,
        })
}

/// Why `workflow:rules` doesn't create this pipeline, or `None` if it does.
pub fn pipeline_exclusion(
    pipeline: &Pipeline,
    variables: &HashMap<String, String>,
) -> Option<SkipReason> {
    let rules = &pipeline.workflow.as_ref()?.rules;
    if rules.is_empty() {
        return None;
    }
    match first_matching_rule("The pipeline", rules, variables) {
        Ok(Some(rule)) => (rule.when.as_deref() == Some("never")).then_some(SkipReason::Never),
        Ok(None) => Some(SkipReason::NoRuleMatched),
        Err(reason) => Some(reason),
    }
}

/// The first rule that matches. `owner` names what the rules belong to in
/// warnings, and a condition that can't be evaluated stops the search.
fn first_matching_rule<'a>(
    owner: &str,
    rules: &'a [Rule],
    variables: &HashMap<String, String>,
) -> Result<Option<&'a Rule>, SkipReason> {
    for rule in rules {
        let matched = match rule.if_.as_deref() {
            Some(condition) => evaluate_or_warn(owner, "rules:if", condition, variables)?,
            None => true,
        };
        if matched {
            warn_about_ignored_clauses(owner, rule);
            return Ok(Some(rule));
        }
    }
    Ok(None)
}

fn evaluate_or_warn(
    owner: &str,
    keyword: &str,
    condition: &str,
    variables: &HashMap<String, String>,
) -> Result<bool, SkipReason> {
    evaluate_rule_condition(condition, variables).map_err(|error| {
        wrkflw_logging::warning(&format!(
            "{} is excluded: can't evaluate {} '{}': {}",
            owner, keyword, condition, error
        ));
        SkipReason::InvalidCondition
    })
}

/// The parts of an `only:` or `except:`.
struct RefFilter<'a> {
    refs: Vec<&'a str>,
    variables: &'a [String],
    has_changes: bool,
}

impl<'a> RefFilter<'a> {
    fn new(
        ref_lists: [&'a Option<Vec<String>>; 3],
        variables: &'a Option<Vec<String>>,
        has_changes: bool,
    ) -> Self {
        RefFilter {
            refs: ref_lists
                .into_iter()
                .flatten()
                .flatten()
                .map(String::as_str)
                .collect(),
            variables: variables.as_deref().unwrap_or_default(),
            has_changes,
        }
    }

    fn from_only(only: &'a Only) -> Self {
        match only {
            Only::Refs(refs) => Self::from_refs(refs),
            Only::Complex {
                refs,
                branches,
                tags,
                variables,
                changes,
            } => Self::new([refs, branches, tags], variables, changes.is_some()),
        }
    }

    fn from_except(except: &'a Except) -> Self {
        match except {
            Except::Refs(refs) => Self::from_refs(refs),
            Except::Complex {
                refs,
                branches,
                tags,
                variables,
                changes,
            } => Self::new([refs, branches, tags], variables, changes.is_some()),
        }
    }

    fn from_refs(refs: &'a [String]) -> Self {
        RefFilter {
            refs: refs.iter().map(String::as_str).collect(),
            variables: &[],
            has_changes: false,
        }
    }

    fn is_evaluated(&self) -> bool {
        !self.refs.is_empty() || !self.variables.is_empty()
    }

    /// Whether this run matches. A key matches when any of its conditions
    /// does. `only:` needs every key it sets to match, while one matching key
    /// is enough for `except:`.
    fn matches(
        &self,
        owner: &str,
        vars: &HashMap<String, String>,
        every_key: bool,
    ) -> Result<bool, SkipReason> {
        let mut refs_match = false;
        for pattern in &self.refs {
            refs_match |= ref_pattern_matches(pattern, vars).map_err(|error| {
                wrkflw_logging::warning(&format!(
                    "{} is excluded: can't evaluate ref pattern '{}': {}",
                    owner, pattern, error
                ));
                SkipReason::InvalidCondition
            })?;
        }
        let mut variables_match = false;
        for condition in self.variables {
            variables_match |= evaluate_or_warn(owner, "only/except", condition, vars)?;
        }
        Ok(if every_key {
            (refs_match || self.refs.is_empty()) && (variables_match || self.variables.is_empty())
        } else {
            refs_match || variables_match
        })
    }
}

/// Whether a run matches one `only:`/`except:` ref: a keyword, a `/regex/`, or
/// an exact ref name. A tag pipeline has no branch, as on GitLab.
fn ref_pattern_matches(pattern: &str, vars: &HashMap<String, String>) -> Result<bool, String> {
    let value = |name: &str| vars.get(name).map(String::as_str).filter(|v| !v.is_empty());
    let tag = value("CI_COMMIT_TAG");
    let branch = value("CI_COMMIT_BRANCH").filter(|_| tag.is_none());
    let source = |expected: &str| value("CI_PIPELINE_SOURCE") == Some(expected);
    Ok(match pattern {
        "branches" => branch.is_some(),
        "tags" => tag.is_some(),
        "api" | "chat" | "external" | "web" => source(pattern),
        "external_pull_requests" => source("external_pull_request_event"),
        "merge_requests" => source("merge_request_event"),
        "pipelines" => source("pipeline"),
        "pushes" => source("push"),
        "schedules" => source("schedule"),
        "triggers" => source("trigger"),
        _ => {
            let current = tag.or(branch);
            if pattern.starts_with('/') {
                let regex = compile_rule_regex(pattern)?;
                current.is_some_and(|name| regex.is_match(name))
            } else {
                current == Some(pattern)
            }
        }
    })
}

/// Why a job's `only:`/`except:` excludes it, or `None` if it doesn't.
/// `changes` isn't evaluated: alone, it neither selects nor excludes a run.
fn ref_filter_exclusion(
    owner: &str,
    job: &Job,
    variables: &HashMap<String, String>,
) -> Option<SkipReason> {
    let check = |filter: &RefFilter, keyword: &str, exclude_on_match: bool| {
        if filter.has_changes {
            wrkflw_logging::warning(&format!("{}: {}:changes isn't evaluated", owner, keyword));
        }
        if !filter.is_evaluated() {
            return None;
        }
        match filter.matches(owner, variables, !exclude_on_match) {
            Ok(matched) => (matched == exclude_on_match).then_some(SkipReason::RefFilter),
            Err(reason) => Some(reason),
        }
    };
    job.only
        .as_ref()
        .and_then(|only| check(&RefFilter::from_only(only), "only", false))
        .or_else(|| {
            let except = job.except.as_ref()?;
            check(&RefFilter::from_except(except), "except", true)
        })
}

fn warn_about_ignored_clauses(owner: &str, rule: &Rule) {
    if rule.variables.is_some() {
        wrkflw_logging::warning(&format!(
            "{}: rules:variables isn't applied, so the job keeps its other values",
            owner
        ));
    }
    for (clause, present) in [
        ("changes", rule.changes.is_some()),
        ("exists", rule.exists.is_some()),
    ] {
        if present {
            wrkflw_logging::warning(&format!(
                "{}: rules:{} isn't evaluated, so this rule applies as if it passed",
                owner, clause
            ));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum RuleToken {
    OpenParen,
    CloseParen,
    And,
    Or,
    Equal,
    NotEqual,
    Match,
    NotMatch,
    Null,
    Variable(String),
    Text(String),
    Regex(String),
}

fn tokenize_rule_condition(condition: &str) -> Result<Vec<RuleToken>, String> {
    let chars: Vec<char> = condition.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let two: String = chars[index..chars.len().min(index + 2)].iter().collect();
        let operator = match two.as_str() {
            "&&" => Some(RuleToken::And),
            "||" => Some(RuleToken::Or),
            "==" => Some(RuleToken::Equal),
            "!=" => Some(RuleToken::NotEqual),
            "=~" => Some(RuleToken::Match),
            "!~" => Some(RuleToken::NotMatch),
            _ => None,
        };
        if let Some(operator) = operator {
            tokens.push(operator);
            index += 2;
            continue;
        }
        match chars[index] {
            ch if ch.is_whitespace() => index += 1,
            '(' => {
                tokens.push(RuleToken::OpenParen);
                index += 1;
            }
            ')' => {
                tokens.push(RuleToken::CloseParen);
                index += 1;
            }
            quote @ ('"' | '\'') => {
                let close = chars[index + 1..]
                    .iter()
                    .position(|ch| *ch == quote)
                    .ok_or("unterminated string")?;
                let end = index + 1 + close;
                tokens.push(RuleToken::Text(chars[index + 1..end].iter().collect()));
                index = end + 1;
            }
            '/' => {
                let mut end = index + 1;
                while end < chars.len() && chars[end] != '/' {
                    end += if chars[end] == '\\' { 2 } else { 1 };
                }
                if end >= chars.len() {
                    return Err("unterminated regular expression".to_string());
                }
                end += 1;
                while end < chars.len() && chars[end].is_ascii_alphabetic() {
                    end += 1;
                }
                tokens.push(RuleToken::Regex(chars[index..end].iter().collect()));
                index = end;
            }
            '$' => {
                let braced = chars.get(index + 1) == Some(&'{');
                let start = index + 1 + usize::from(braced);
                let mut end = start;
                while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                if end == start {
                    return Err("expected a variable name after '$'".to_string());
                }
                tokens.push(RuleToken::Variable(chars[start..end].iter().collect()));
                index = end;
                if braced {
                    if chars.get(index) != Some(&'}') {
                        return Err("unterminated '${'".to_string());
                    }
                    index += 1;
                }
            }
            _ if chars[index..].starts_with(&['n', 'u', 'l', 'l'])
                && !chars
                    .get(index + 4)
                    .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_') =>
            {
                tokens.push(RuleToken::Null);
                index += 4;
            }
            ch => return Err(format!("unexpected '{}'", ch)),
        }
    }
    Ok(tokens)
}

/// Compile the pattern of a `=~`: `/pattern/flags`, or a bare pattern.
fn compile_rule_regex(pattern: &str) -> Result<regex::Regex, String> {
    let (pattern, flags) = pattern
        .strip_prefix('/')
        .and_then(|rest| rest.rsplit_once('/'))
        .unwrap_or((pattern, ""));
    if let Some(flag) = flags.chars().find(|flag| !"ims".contains(*flag)) {
        return Err(format!("unsupported regular expression flag '{}'", flag));
    }
    let unescaped = pattern.replace("\\/", "/");
    let inline_flags = if flags.is_empty() {
        String::new()
    } else {
        format!("(?{})", flags)
    };
    regex::Regex::new(&format!("{}{}", inline_flags, unescaped)).map_err(|error| error.to_string())
}

/// Recursive-descent evaluator for `rules:if`, where `&&` binds tighter than
/// `||`. Both sides of an operator are always evaluated, so that a mistake is
/// reported however the rest of the condition turns out.
struct RuleEvaluator<'a> {
    tokens: Vec<RuleToken>,
    position: usize,
    variables: &'a HashMap<String, String>,
}

impl RuleEvaluator<'_> {
    fn peek(&self) -> Option<&RuleToken> {
        self.tokens.get(self.position)
    }

    fn accept(&mut self, token: &RuleToken) -> bool {
        let accepted = self.peek() == Some(token);
        self.position += usize::from(accepted);
        accepted
    }

    fn any_of(&mut self) -> Result<bool, String> {
        let mut result = self.all_of()?;
        while self.accept(&RuleToken::Or) {
            result |= self.all_of()?;
        }
        Ok(result)
    }

    fn all_of(&mut self) -> Result<bool, String> {
        let mut result = self.term()?;
        while self.accept(&RuleToken::And) {
            result &= self.term()?;
        }
        Ok(result)
    }

    fn term(&mut self) -> Result<bool, String> {
        if self.accept(&RuleToken::OpenParen) {
            let result = self.any_of()?;
            if !self.accept(&RuleToken::CloseParen) {
                return Err("missing ')'".to_string());
            }
            return Ok(result);
        }
        let left = self.operand()?;
        let negated = match self.peek() {
            Some(RuleToken::Equal | RuleToken::Match) => false,
            Some(RuleToken::NotEqual | RuleToken::NotMatch) => true,
            // A bare value is true when it's set and not empty.
            _ => return Ok(left.is_some_and(|value| !value.is_empty())),
        };
        let is_regex = matches!(self.peek(), Some(RuleToken::Match | RuleToken::NotMatch));
        self.position += 1;
        if is_regex {
            let regex = self
                .pattern()?
                .map(|p| compile_rule_regex(&p))
                .transpose()?;
            // An undefined left side matches as an empty string, as on GitLab.
            let matched = regex.is_some_and(|regex| regex.is_match(&left.unwrap_or_default()));
            return Ok(matched != negated);
        }
        Ok((left == self.operand()?) != negated)
    }

    /// A variable, quoted string or `null`, where `None` means null.
    fn operand(&mut self) -> Result<Option<String>, String> {
        let token = self.peek().cloned();
        self.position += 1;
        match token {
            Some(RuleToken::Null) => Ok(None),
            Some(RuleToken::Text(text)) => Ok(Some(text)),
            Some(RuleToken::Variable(name)) => Ok(self.variables.get(&name).cloned()),
            Some(token) => Err(format!("unexpected {:?}", token)),
            None => Err("the condition ends too early".to_string()),
        }
    }

    /// The right side of a `=~`, which may hold its pattern in a variable.
    fn pattern(&mut self) -> Result<Option<String>, String> {
        if let Some(RuleToken::Regex(pattern)) = self.peek().cloned() {
            self.position += 1;
            return Ok(Some(pattern));
        }
        self.operand()
    }
}

/// Evaluate a `rules:if` condition, or say why it can't be.
fn evaluate_rule_condition(
    condition: &str,
    variables: &HashMap<String, String>,
) -> Result<bool, String> {
    let mut evaluator = RuleEvaluator {
        tokens: tokenize_rule_condition(condition)?,
        position: 0,
        variables,
    };
    let result = evaluator.any_of()?;
    match evaluator.peek() {
        None => Ok(result),
        Some(token) => Err(format!("unexpected {:?}", token)),
    }
}

/// Expand `$NAME` and `${NAME}`, including variables that refer to others. An
/// undefined reference stays in place, so that the error it causes names it.
fn expand_variables(text: &str, variables: &HashMap<String, String>) -> String {
    /// More than enough for real pipelines, and stops a variable that refers
    /// to itself.
    const MAX_DEPTH: usize = 8;
    let reference = regex::Regex::new(r"\$(?:\{(\w+)\}|(\w+))").expect("valid regex");
    let mut expanded = text.to_string();
    for _ in 0..MAX_DEPTH {
        let next = reference
            .replace_all(&expanded, |captures: &regex::Captures| {
                let name = captures
                    .get(1)
                    .or(captures.get(2))
                    .map_or("", |m| m.as_str());
                variables
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| captures[0].to_string())
            })
            .into_owned();
        if next == expanded {
            break;
        }
        expanded = next;
    }
    expanded
}

/// A string-or-list key from the pipeline's `default:` section.
fn default_list(pipeline: &Pipeline, key: &str) -> Option<Vec<String>> {
    match pipeline.default.as_ref()?.get(key)? {
        serde_yaml::Value::String(line) => Some(vec![line.clone()]),
        serde_yaml::Value::Sequence(lines) => Some(
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_string))
                .collect(),
        ),
        _ => None,
    }
}

/// The image a job runs in: its own, then `default:`, then the top-level one,
/// with the pipeline's and the job's variables expanded. A `parallel:matrix`
/// variable becomes a matrix reference, which each instance fills in.
fn job_image(job_name: &str, pipeline: &Pipeline, job: &Job) -> Option<String> {
    let image = job
        .image
        .clone()
        .or_else(|| {
            let image = pipeline.default.as_ref()?.get("image")?;
            serde_yaml::from_value::<Image>(image.clone()).ok()
        })
        .or_else(|| pipeline.image.clone())?;
    let (image, entrypoint) = match image {
        Image::Simple(name) => (name, None),
        Image::Detailed { name, entrypoint } => (name, entrypoint),
    };
    if entrypoint.is_some() {
        wrkflw_logging::warning(&format!(
            "Job '{}': image entrypoint isn't applied, so '{}' keeps its own",
            job_name, image
        ));
    }

    let mut variables = pipeline.variables.clone().unwrap_or_default();
    variables.extend(job.variables.clone().unwrap_or_default());
    if let Some(Parallel::Matrix { matrix }) = &job.parallel {
        for name in matrix.iter().flat_map(HashMap::keys) {
            variables.insert(name.clone(), format!("${{{{ matrix.{} }}}}", name));
        }
    }
    let expanded = expand_variables(&image, &variables);
    if expanded.replace("${{", "").contains('$') {
        wrkflw_logging::warning(&format!(
            "Job '{}': image '{}' refers to an undefined variable. Set it with --variable",
            job_name, expanded
        ));
    }
    Some(expanded)
}

/// The most instances GitLab creates from one job's `parallel`
const MAX_PARALLEL_INSTANCES: usize = 200;

/// Reject `parallel:matrix` mistakes the schema can't express. The schema
/// already bounds `parallel: N` and the number of matrix entries, but not how
/// many instances those entries multiply out to, or what's inside them.
fn check_parallel(pipeline: &Pipeline) -> Result<(), String> {
    let mut issues = Vec::new();
    for (job_name, job) in &pipeline.jobs {
        if let Some(Parallel::Matrix { matrix }) = &job.parallel {
            issues.extend(
                matrix_issues(matrix)
                    .into_iter()
                    .map(|issue| format!("Job '{}': parallel:matrix {}", job_name, issue)),
            );
        }
    }
    if issues.is_empty() {
        return Ok(());
    }
    issues.sort();
    Err(issues.join("; "))
}

fn matrix_issues(matrix: &[HashMap<String, serde_yaml::Value>]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut total = 0usize;
    for entry in matrix {
        if entry.is_empty() {
            issues.push("has an entry with no variables".to_string());
        }
        let mut instances = 1usize;
        for (name, value) in entry {
            if !is_variable_name(name) {
                issues.push(format!("variable '{}' is not a valid name", name));
            }
            let values = match value {
                serde_yaml::Value::Sequence(values) => values.len(),
                _ => 1,
            };
            if values == 0 {
                issues.push(format!("variable '{}' has no values", name));
            }
            instances = instances.saturating_mul(values);
        }
        total = total.saturating_add(instances);
    }
    if total > MAX_PARALLEL_INSTANCES {
        issues.push(format!(
            "expands to more than {} instances",
            MAX_PARALLEL_INSTANCES
        ));
    }
    issues
}

fn is_variable_name(name: &str) -> bool {
    !name.starts_with(|c: char| c.is_ascii_digit())
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Run a job once per `parallel` instance through a matrix, exposing each
/// instance's variables as environment variables, as GitLab does. Every
/// instance also gets `CI_NODE_INDEX` and `CI_NODE_TOTAL`.
fn apply_parallel(job: &mut workflow::Job, parallel: &Parallel) {
    let mut combinations: Vec<HashMap<String, serde_yaml::Value>> = match parallel {
        Parallel::Count(total) => vec![HashMap::new(); *total],
        Parallel::Matrix { matrix } => matrix.iter().flat_map(expand_parallel_entry).collect(),
    };
    for (index, combination) in combinations.iter_mut().enumerate() {
        combination.insert(
            "CI_NODE_INDEX".to_string(),
            serde_yaml::Value::from(index as u64 + 1),
        );
    }
    job.env
        .insert("CI_NODE_TOTAL".to_string(), combinations.len().to_string());

    let variables: HashSet<&String> = combinations.iter().flat_map(HashMap::keys).collect();
    for variable in variables {
        job.env
            .insert(variable.clone(), format!("${{{{ matrix.{} }}}}", variable));
    }
    // GitLab runs every instance even when one fails.
    job.strategy = Some(workflow::Strategy {
        matrix: Some(MatrixConfig::from_combinations(combinations)),
        fail_fast: Some(false),
        max_parallel: None,
    });
}

/// The Cartesian product of one `parallel:matrix` entry's variables. Entries
/// are independent: GitLab creates each one's instances, even if they overlap.
/// An entry with nothing to expand has no instances; `check_parallel` reports it.
fn expand_parallel_entry(
    entry: &HashMap<String, serde_yaml::Value>,
) -> Vec<HashMap<String, serde_yaml::Value>> {
    // Sorted so that each instance's CI_NODE_INDEX is stable between runs
    let mut variables: Vec<_> = entry.iter().collect();
    variables.sort_by_key(|(name, _)| name.as_str());
    let matrix = MatrixConfig {
        parameters: variables
            .into_iter()
            .map(|(name, values)| (name.clone(), values.clone()))
            .collect(),
        ..MatrixConfig::default()
    };
    wrkflw_matrix::expand_matrix(&matrix)
        .map(|combinations| combinations.into_iter().map(|c| c.values).collect())
        .unwrap_or_default()
}

/// Convert a GitLab CI/CD pipeline to a format compatible with the workflow executor
/// A `continue-on-error` value that holds whatever the job or step did.
pub const CONTINUE_ON_ERROR_ALWAYS: &str = "true";

pub fn convert_to_workflow_format(pipeline: &Pipeline) -> workflow::WorkflowDefinition {
    // Create a new workflow with required fields
    let mut workflow = workflow::WorkflowDefinition {
        name: "Converted GitLab CI Pipeline".to_string(),
        on: vec!["push".to_string()], // Default trigger
        on_raw: serde_yaml::Value::String("push".to_string()),
        jobs: HashMap::new(),
        defaults: None,
        env: HashMap::new(),
    };

    // Convert each GitLab job to a GitHub Actions job
    for (job_name, gitlab_job) in &pipeline.jobs {
        // Skip template jobs
        if let Some(true) = gitlab_job.template {
            continue;
        }

        let image = job_image(job_name, pipeline, gitlab_job);

        // Create a new job
        let mut job = workflow::Job {
            runs_on: Some(vec!["ubuntu-latest".to_string()]), // Default runner
            needs: None,
            container: image.clone().map(|image| workflow::JobContainer {
                image,
                credentials: None,
                env: HashMap::new(),
                ports: None,
                volumes: None,
                options: None,
            }),
            steps: Vec::new(),
            env: HashMap::new(),
            strategy: None,
            services: HashMap::new(),
            if_condition: None,
            outputs: None,
            permissions: None,
            uses: None,
            with: None,
            secrets: None,
            timeout_minutes: None,
            defaults: image.is_some().then(|| workflow::Defaults {
                run: Some(workflow::DefaultsRun {
                    shell: Some(SHELL_BASH_OR_SH.to_string()),
                    working_directory: None,
                }),
            }),
            continue_on_error: matches!(gitlab_job.allow_failure, Some(AllowFailure::Bool(true)))
                .then(|| CONTINUE_ON_ERROR_ALWAYS.to_string()),
        };

        // Add job-specific environment variables
        if let Some(variables) = &gitlab_job.variables {
            job.env.extend(variables.clone());
        }

        if let Some(parallel) = &gitlab_job.parallel {
            apply_parallel(&mut job, parallel);
        }

        // Add global variables if they exist
        if let Some(variables) = &pipeline.variables {
            // Only add if not already defined at job level
            for (key, value) in variables {
                job.env.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }

        // Convert before_script to steps if it exists
        let before_script = gitlab_job
            .before_script
            .clone()
            .or_else(|| default_list(pipeline, "before_script"))
            .or_else(|| pipeline.before_script.clone());
        if let Some(before_script) = &before_script {
            for (i, cmd) in before_script.iter().enumerate() {
                job.steps.push(workflow::Step::with_run(
                    format!("Before script {}", i + 1),
                    cmd.clone(),
                ));
            }
        }

        // Convert main script to steps
        if let Some(script) = &gitlab_job.script {
            for (i, cmd) in script.iter().enumerate() {
                job.steps.push(workflow::Step::with_run(
                    format!("Run script line {}", i + 1),
                    cmd.clone(),
                ));
            }
        }

        // Convert after_script to steps if it exists
        let after_script = gitlab_job
            .after_script
            .clone()
            .or_else(|| default_list(pipeline, "after_script"))
            .or_else(|| pipeline.after_script.clone());
        if let Some(after_script) = &after_script {
            for (i, cmd) in after_script.iter().enumerate() {
                let mut step =
                    workflow::Step::with_run(format!("After script {}", i + 1), cmd.clone());
                step.continue_on_error = Some(CONTINUE_ON_ERROR_ALWAYS.to_string()); // After script should continue even if previous steps fail
                job.steps.push(step);
            }
        }

        // Add services if they exist
        if let Some(services) = &gitlab_job.services {
            for (i, service) in services.iter().enumerate() {
                let service_name = format!("service-{}", i);
                let service_image = match service {
                    wrkflw_models::gitlab::Service::Simple(name) => name.clone(),
                    wrkflw_models::gitlab::Service::Detailed { name, .. } => name.clone(),
                };

                let service = workflow::Service {
                    image: service_image,
                    ports: None,
                    env: HashMap::new(),
                    volumes: None,
                    options: None,
                };

                job.services.insert(service_name, service);
            }
        }

        // Add the job to the workflow
        workflow.jobs.insert(job_name.clone(), job);
    }

    workflow
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    // use std::path::PathBuf; // unused
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_pipeline() {
        // Create a temporary file with a simple GitLab CI/CD pipeline
        let file = NamedTempFile::new().unwrap();
        let content = r#"
stages:
  - build
  - test

build_job:
  stage: build
  script:
    - echo "Building..."
    - make build

test_job:
  stage: test
  script:
    - echo "Testing..."
    - make test
"#;
        fs::write(&file, content).unwrap();

        // Parse the pipeline
        let pipeline = parse_pipeline(file.path()).unwrap();

        // Validate basic structure
        assert_eq!(pipeline.stages.as_ref().unwrap().len(), 2);
        assert_eq!(pipeline.jobs.len(), 2);

        // Check job contents
        let build_job = pipeline.jobs.get("build_job").unwrap();
        assert_eq!(build_job.stage.as_ref().unwrap(), "build");
        assert_eq!(build_job.script.as_ref().unwrap().len(), 2);

        let test_job = pipeline.jobs.get("test_job").unwrap();
        assert_eq!(test_job.stage.as_ref().unwrap(), "test");
        assert_eq!(test_job.script.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn allow_failure_maps_to_continue_on_error_only_when_unconditional() {
        let file = NamedTempFile::new().unwrap();
        fs::write(
            file.path(),
            r#"
stages:
  - test

allowed:
  stage: test
  script:
    - make test
  allow_failure: true

disallowed:
  stage: test
  script:
    - make test
  allow_failure: false

by_exit_code:
  stage: test
  script:
    - make test
  allow_failure:
    exit_codes: [47]

unset:
  stage: test
  script:
    - make test
"#,
        )
        .unwrap();

        let pipeline = parse_pipeline(file.path()).unwrap();
        let workflow = convert_to_workflow_format(&pipeline);
        let value = |job: &str| workflow.jobs[job].continue_on_error.as_deref();

        assert_eq!(value("allowed"), Some("true"));
        assert_eq!(value("disallowed"), None);
        assert_eq!(value("by_exit_code"), None);
        assert_eq!(value("unset"), None);
    }

    #[test]
    fn test_parse_pipeline_with_templates_and_includes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".gitlab")).unwrap();
        fs::write(
            dir.path().join(".gitlab/templates.yml"),
            r#"
.build_template:
  stage: build
  script:
    - make build
  allow_failure:
    exit_codes: [47]
"#,
        )
        .unwrap();
        let path = dir.path().join(".gitlab-ci.yml");
        fs::write(
            &path,
            r#"
include:
  - local: .gitlab/templates.yml

stages:
  - build
  - deploy

build_job:
  extends: .build_template

downstream:
  stage: deploy
  trigger:
    project: group/downstream
"#,
        )
        .unwrap();

        let pipeline = parse_pipeline(&path).unwrap();

        let mut names: Vec<_> = pipeline.jobs.keys().cloned().collect();
        names.sort();
        assert_eq!(names, ["build_job", "downstream"]);

        let build_job = &pipeline.jobs["build_job"];
        assert_eq!(build_job.stage.as_deref(), Some("build"));
        assert_eq!(build_job.script.as_ref().unwrap(), &["make build"]);
        assert!(build_job.extends.is_none());

        let result = validate_pipeline_structure(&pipeline);
        assert!(result.is_valid, "{:?}", result.issues);
    }

    fn convert(yaml: &str) -> workflow::WorkflowDefinition {
        convert_to_workflow_format(&serde_yaml::from_str(yaml).unwrap())
    }

    fn variables(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn when_of(job_yaml: &str, vars: &[(&str, &str)]) -> JobWhen {
        let job: Job = serde_yaml::from_str(job_yaml).unwrap();
        job_decision("job", &job, &variables(vars)).when
    }

    #[test]
    fn job_when_reads_the_job_level_key() {
        assert_eq!(when_of("script: [a]", &[]), JobWhen::OnSuccess);
        assert_eq!(
            when_of("script: [a]\nwhen: manual", &[]),
            JobWhen::Skip(SkipReason::Manual)
        );
        assert_eq!(
            when_of("script: [a]\nwhen: never", &[]),
            JobWhen::Skip(SkipReason::Never)
        );
        assert_eq!(when_of("script: [a]\nwhen: always", &[]), JobWhen::Always);
        assert_eq!(
            when_of("script: [a]\nwhen: on_failure", &[]),
            JobWhen::OnFailure
        );
    }

    #[test]
    fn first_matching_rule_decides() {
        let job = r#"
script: [a]
rules:
  - if: '$V == "top"'
    when: never
  - when: always
"#;
        assert_eq!(
            when_of(job, &[("V", "top")]),
            JobWhen::Skip(SkipReason::Never)
        );
        assert_eq!(when_of(job, &[("V", "other")]), JobWhen::Always);
    }

    #[test]
    fn a_job_whose_rules_all_miss_is_excluded() {
        let job = "script: [a]\nwhen: always\nrules:\n  - if: '$NOPE == \"1\"'\n";
        assert_eq!(when_of(job, &[]), JobWhen::Skip(SkipReason::NoRuleMatched));
        assert_eq!(when_of(job, &[("NOPE", "1")]), JobWhen::Always);
    }

    #[test]
    fn rule_conditions_cover_the_common_operators() {
        let vars = variables(&[
            ("A", "main"),
            ("B", "x"),
            ("EMPTY", ""),
            ("QUOTED", "a==b"),
            ("PATH_LIKE", "release/1.0"),
            ("PATTERN", "/^MAIN$/i"),
            ("BARE_PATTERN", "^ma"),
        ]);
        for (condition, expected) in [
            ("$A", true),
            ("$MISSING", false),
            ("$EMPTY", false),
            ("$A == 'main'", true),
            ("$A != \"main\"", false),
            ("$A == $B", false),
            ("$MISSING == null", true),
            ("$A != null", true),
            ("$A =~ /^ma/", true),
            ("$A =~ /^MA/i", true),
            ("$A !~ /^ma/", false),
            ("$A =~ /(main|dev)/", true),
            ("$A == 'main' && $B == 'x'", true),
            ("$A == 'nope' || $B == 'x'", true),
            ("$A == 'nope' && $B == 'x'", false),
            ("$A == 'a||b' || $B == 'nope'", false),
            ("${A} == 'main'", true),
            ("($A == 'nope')", false),
            ("($A == 'nope' || $B == 'x') && $A == 'main'", true),
            ("$A == 'main' && ($B == 'nope' || $B == 'y')", false),
            ("$A == 'main' || $B == 'x' && $B == 'nope'", true),
            ("$A=='main'", true),
            ("$QUOTED != \"a==b\"", false),
            ("$QUOTED == \"a==b\"", true),
            ("$PATH_LIKE =~ /^release\\/.*/", true),
            ("$A =~ /(main)&&(dev)/", false),
            ("$A =~ $PATTERN", true),
            ("$A !~ $PATTERN", false),
            ("$A =~ $BARE_PATTERN", true),
            ("$A =~ $MISSING", false),
            ("$MISSING =~ /^$/", true),
            ("$MISSING !~ /^$/", false),
            ("$MISSING !~ /x/", true),
        ] {
            assert_eq!(
                evaluate_rule_condition(condition, &vars),
                Ok(expected),
                "{}",
                condition
            );
        }
    }

    #[test]
    fn a_condition_that_cannot_be_evaluated_excludes_the_job() {
        for condition in [
            "($A == 'x'",
            "$A ==",
            "$A == 'x' extra",
            "$A =~ /(/",
            "'unterminated",
            "&& $A",
        ] {
            assert!(
                evaluate_rule_condition(condition, &HashMap::new()).is_err(),
                "{}",
                condition
            );
        }
        // Even when the broken rule would have been `when: never`.
        let job = "script: [a]\nrules:\n  - if: '($A'\n    when: never\n  - when: always\n";
        assert_eq!(
            when_of(job, &[]),
            JobWhen::Skip(SkipReason::InvalidCondition)
        );
    }

    #[test]
    fn rule_clauses_that_arent_evaluated_are_kept_on_the_rule() {
        let job: Job =
            serde_yaml::from_str("script: [a]\nrules:\n  - changes: [docs/*]\n").unwrap();
        let rules = job.rules.as_ref().unwrap();
        assert!(rules[0].changes.is_some());
        assert_eq!(
            job_decision("job", &job, &HashMap::new()).when,
            JobWhen::OnSuccess
        );
    }

    #[test]
    fn skip_reason_follows_the_outcome_of_earlier_stages() {
        for (when, runs_after_success, runs_after_failure) in [
            (JobWhen::OnSuccess, true, false),
            (JobWhen::OnFailure, false, true),
            (JobWhen::Always, true, true),
            (JobWhen::Skip(SkipReason::Manual), false, false),
        ] {
            assert_eq!(when.skip_reason(false).is_none(), runs_after_success);
            assert_eq!(when.skip_reason(true).is_none(), runs_after_failure);
        }
    }

    #[test]
    fn image_names_expand_variables() {
        let workflow = convert(
            r#"
variables:
  VERSION: "1.76"
  IMAGE: rust:${VERSION}
  CUSTOM: global
plain:
  image: rust:${VERSION}
  script: [echo]
own:
  image: $CUSTOM:1
  variables:
    CUSTOM: job
  script: [echo]
nested:
  image: $IMAGE
  script: [echo]
undefined:
  image: rust:$NOPE
  script: [echo]
"#,
        );
        let image = |job: &str| workflow.jobs[job].container.as_ref().unwrap().image.clone();
        assert_eq!(image("plain"), "rust:1.76");
        assert_eq!(image("own"), "job:1");
        assert_eq!(image("nested"), "rust:1.76");
        assert_eq!(image("undefined"), "rust:$NOPE");
    }

    #[test]
    fn a_variable_that_refers_to_itself_stops_expanding() {
        let vars = variables(&[("A", "x$A")]);
        assert!(expand_variables("$A", &vars).starts_with("xxx"));
    }

    #[test]
    fn image_and_scripts_fall_back_to_defaults() {
        let workflow = convert(
            r#"
image: top:1
default:
  image: dflt:1
  before_script: [echo before]
  after_script: echo after
inherit:
  script: [echo]
own:
  image: own:1
  before_script: [echo mine]
  script: [echo]
"#,
        );
        let inherit = &workflow.jobs["inherit"];
        assert_eq!(inherit.container.as_ref().unwrap().image, "dflt:1");
        let steps: Vec<_> = inherit
            .steps
            .iter()
            .filter_map(|s| s.run.as_deref())
            .collect();
        assert_eq!(steps, ["echo before", "echo", "echo after"]);

        let own = &workflow.jobs["own"];
        assert_eq!(own.container.as_ref().unwrap().image, "own:1");
        assert_eq!(own.steps[0].run.as_deref(), Some("echo mine"));
        assert_eq!(
            own.defaults
                .as_ref()
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .shell
                .as_deref(),
            Some(SHELL_BASH_OR_SH)
        );
    }

    #[test]
    fn only_and_except_decide_when_there_are_no_rules() {
        let branch = [("CI_COMMIT_BRANCH", "dev"), ("CI_PIPELINE_SOURCE", "push")];
        let tag = [
            ("CI_COMMIT_BRANCH", "dev"),
            ("CI_COMMIT_TAG", "v1.2"),
            ("CI_PIPELINE_SOURCE", "push"),
        ];
        let excluded = JobWhen::Skip(SkipReason::RefFilter);
        for (filter, vars, expected) in [
            ("only: [main]", &branch[..], excluded),
            ("only: [dev]", &branch, JobWhen::OnSuccess),
            ("only: [branches]", &branch, JobWhen::OnSuccess),
            ("only: [tags]", &branch, excluded),
            ("only: [tags]", &tag, JobWhen::OnSuccess),
            ("only: [branches]", &tag, excluded),
            ("only: [pushes]", &branch, JobWhen::OnSuccess),
            ("only: [merge_requests]", &branch, excluded),
            ("only: ['/^de/']", &branch, JobWhen::OnSuccess),
            ("only: ['/^v/']", &tag, JobWhen::OnSuccess),
            ("except: [dev]", &branch, excluded),
            ("except: [main]", &branch, JobWhen::OnSuccess),
            ("except: [tags]", &tag, excluded),
            ("only: {refs: [dev], variables: ['$X']}", &branch, excluded),
            (
                "only: {refs: [main], variables: ['$CI_COMMIT_BRANCH']}",
                &branch,
                excluded,
            ),
            (
                "only: {variables: ['$CI_COMMIT_BRANCH']}",
                &branch,
                JobWhen::OnSuccess,
            ),
            ("only: {changes: [docs/*]}", &branch, JobWhen::OnSuccess),
            ("except: {changes: [docs/*]}", &branch, JobWhen::OnSuccess),
            (
                "except: {refs: [dev], variables: ['$X']}",
                &branch,
                excluded,
            ),
            (
                "except: {refs: [main], variables: ['$CI_COMMIT_BRANCH']}",
                &branch,
                excluded,
            ),
            (
                "except: {refs: [main], variables: ['$X']}",
                &branch,
                JobWhen::OnSuccess,
            ),
            (
                "only: ['/(/']",
                &branch,
                JobWhen::Skip(SkipReason::InvalidCondition),
            ),
        ] {
            assert_eq!(
                when_of(&format!("script: [a]\n{}", filter), vars),
                expected,
                "{}",
                filter
            );
        }
    }

    #[test]
    fn manual_jobs_block_unless_allowed_to_fail() {
        let blocking = JobWhen::Skip(SkipReason::BlockingManual);
        let optional = JobWhen::Skip(SkipReason::Manual);
        for (job, expected) in [
            ("when: manual", optional),
            ("when: manual\nallow_failure: false", blocking),
            ("rules:\n  - when: manual", blocking),
            ("allow_failure: true\nrules:\n  - when: manual", optional),
            (
                "rules:\n  - when: manual\n    allow_failure: true",
                optional,
            ),
            (
                "allow_failure: true\nrules:\n  - when: manual\n    allow_failure: false",
                blocking,
            ),
        ] {
            assert_eq!(
                when_of(&format!("script: [a]\n{}", job), &[]),
                expected,
                "{}",
                job
            );
        }
        assert!(blocking.blocks_later_stages());
        assert!(!optional.blocks_later_stages());
    }

    #[test]
    fn the_matched_rule_supplies_allow_failure() {
        let decide = |vars: &[(&str, &str)]| {
            let job: Job = serde_yaml::from_str(
                "script: [a]\nrules:\n  - if: $V\n    allow_failure: true\n  - when: on_success\n",
            )
            .unwrap();
            job_decision("job", &job, &variables(vars)).allow_failure
        };
        assert_eq!(decide(&[("V", "1")]), Some(true));
        assert_eq!(decide(&[]), None);
    }

    #[test]
    fn rules_reading_a_matrix_variable_exclude_the_job() {
        let matrix = "script: [a]\nparallel:\n  matrix:\n    - CLOUD: [aws, gcp]\n";
        assert_eq!(
            when_of(
                &format!(
                    "{}rules:\n  - if: $CLOUD == \"gcp\"\n    when: never\n  - when: on_success\n",
                    matrix
                ),
                &[]
            ),
            JobWhen::Skip(SkipReason::MatrixCondition)
        );
        assert_eq!(
            when_of(
                &format!("{}rules:\n  - if: $OTHER\n", matrix),
                &[("OTHER", "1")]
            ),
            JobWhen::OnSuccess
        );
    }

    #[test]
    fn matrix_variables_in_an_image_become_matrix_references() {
        let workflow = convert(
            r#"
job:
  image: ruby:$RUBY
  parallel:
    matrix:
      - RUBY: ["3.2", "3.3"]
  script: [echo]
"#,
        );
        assert_eq!(
            workflow.jobs["job"].container.as_ref().unwrap().image,
            "ruby:${{ matrix.RUBY }}"
        );
    }

    #[test]
    fn rules_take_precedence_over_only() {
        let job = "script: [a]\nonly: [main]\nrules:\n  - when: always\n";
        assert_eq!(
            when_of(job, &[("CI_COMMIT_BRANCH", "dev")]),
            JobWhen::Always
        );
    }

    #[test]
    fn delayed_runs_like_on_success() {
        assert_eq!(
            when_of("script: [a]\nwhen: delayed\nstart_in: 1 hour", &[]),
            JobWhen::OnSuccess
        );
    }

    #[test]
    fn workflow_rules_decide_whether_the_pipeline_exists() {
        let pipeline = |rules: &str| -> Pipeline {
            serde_yaml::from_str(&format!(
                "workflow:\n  rules:\n{}\njob:\n  script: [a]\n",
                rules
            ))
            .unwrap()
        };
        let vars = variables(&[("CI_PIPELINE_SOURCE", "push")]);
        let none: Pipeline = serde_yaml::from_str("job:\n  script: [a]\n").unwrap();
        assert_eq!(pipeline_exclusion(&none, &vars), None);
        for (rules, expected) in [
            ("    - if: $CI_PIPELINE_SOURCE == \"push\"", None),
            (
                "    - if: $CI_PIPELINE_SOURCE == \"web\"",
                Some(SkipReason::NoRuleMatched),
            ),
            (
                "    - if: $CI_PIPELINE_SOURCE\n      when: never\n    - when: always",
                Some(SkipReason::Never),
            ),
            ("    - if: '($X'", Some(SkipReason::InvalidCondition)),
        ] {
            assert_eq!(
                pipeline_exclusion(&pipeline(rules), &vars),
                expected,
                "{}",
                rules
            );
        }
    }

    #[test]
    fn rule_conditions_handle_null_prefixes_and_regex_flags() {
        let vars = variables(&[("A", "line\nnext"), ("B", "x")]);
        assert_eq!(
            evaluate_rule_condition("$A =~ /line.next/s", &vars),
            Ok(true)
        );
        assert_eq!(
            evaluate_rule_condition("$A =~ /LINE.NEXT/is", &vars),
            Ok(true)
        );
        assert_eq!(evaluate_rule_condition("$A =~ /^next$/m", &vars), Ok(true));
        assert_eq!(evaluate_rule_condition("$B == null", &vars), Ok(false));
        assert!(evaluate_rule_condition("$A =~ /x/g", &vars).is_err());
        assert!(evaluate_rule_condition("$A == nullable", &vars).is_err());
    }

    #[test]
    fn detailed_images_use_their_name_at_every_level() {
        let workflow = convert(
            r#"
default:
  image:
    name: dflt:1
    entrypoint: [""]
inherit:
  script: [echo]
own:
  image:
    name: own:1
    entrypoint: [""]
  script: [echo]
"#,
        );
        let image = |job: &str| workflow.jobs[job].container.as_ref().unwrap().image.clone();
        assert_eq!(image("inherit"), "dflt:1");
        assert_eq!(image("own"), "own:1");
    }

    #[test]
    fn jobs_without_any_image_keep_the_default_runner() {
        let workflow = convert("job:\n  script: [echo]\n");
        assert!(workflow.jobs["job"].container.is_none());
        assert!(workflow.jobs["job"].defaults.is_none());
    }

    /// Each instance's variables, sorted so that tests can compare them
    fn instances(workflow: &workflow::WorkflowDefinition) -> Vec<Vec<(String, String)>> {
        let matrix = workflow.jobs["job"].matrix_config().unwrap();
        let mut instances: Vec<Vec<(String, String)>> = wrkflw_matrix::expand_matrix(matrix)
            .unwrap()
            .into_iter()
            .map(|combination| {
                let mut values: Vec<_> = combination
                    .values
                    .into_iter()
                    .filter(|(name, _)| name != "CI_NODE_INDEX")
                    .map(|(name, value)| {
                        let value = serde_yaml::to_string(&value).unwrap();
                        (name, value.trim().to_string())
                    })
                    .collect();
                values.sort();
                values
            })
            .collect();
        instances.sort();
        instances
    }

    fn pairs(values: &[(&str, &str)]) -> Vec<(String, String)> {
        values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    fn parse(yaml: &str) -> Result<Pipeline, GitlabParserError> {
        let file = NamedTempFile::new().unwrap();
        fs::write(file.path(), yaml).unwrap();
        parse_pipeline(file.path())
    }

    fn parse_matrix(matrix: &str) -> Result<Pipeline, GitlabParserError> {
        parse(&format!(
            "job:\n  script: [echo]\n  parallel:\n    matrix:\n{}",
            matrix
        ))
    }

    #[test]
    fn parallel_matrix_expands_each_entry() {
        let workflow = convert(
            r#"
stages: [test]
job:
  stage: test
  script: [echo]
  parallel:
    matrix:
      - A: [x, y]
        B: "1"
      - C: z
"#,
        );
        let job = &workflow.jobs["job"];
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("A", "x"), ("B", "'1'")]),
                pairs(&[("A", "y"), ("B", "'1'")]),
                pairs(&[("C", "z")]),
            ]
        );
        assert_eq!(job.env["A"], "${{ matrix.A }}");
        assert_eq!(job.env["C"], "${{ matrix.C }}");
        assert!(!job.fail_fast());
    }

    #[test]
    fn parallel_matrix_keeps_overlapping_entries_apart() {
        let workflow = convert(
            r#"
job:
  script: [echo]
  parallel:
    matrix:
      - OS: [linux, windows]
      - OS: linux
        ARCH: arm64
"#,
        );
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("ARCH", "arm64"), ("OS", "linux")]),
                pairs(&[("OS", "linux")]),
                pairs(&[("OS", "windows")]),
            ]
        );
    }

    #[test]
    fn parallel_matrix_keeps_an_entry_that_another_extends() {
        let workflow = convert(
            r#"
job:
  script: [echo]
  parallel:
    matrix:
      - A: x
      - A: x
        B: [1, 2]
"#,
        );
        assert_eq!(
            instances(&workflow),
            vec![
                pairs(&[("A", "x")]),
                pairs(&[("A", "x"), ("B", "1")]),
                pairs(&[("A", "x"), ("B", "2")]),
            ]
        );
    }

    #[test]
    fn parallel_matrix_keeps_number_values() {
        let workflow =
            convert("job:\n  script: [echo]\n  parallel:\n    matrix:\n      - RETRIES: [1, 2]\n");
        assert_eq!(
            instances(&workflow),
            vec![pairs(&[("RETRIES", "1")]), pairs(&[("RETRIES", "2")])]
        );
    }

    #[test]
    fn parallel_matrix_overrides_a_job_variable_of_the_same_name() {
        let workflow = convert(
            r#"
variables:
  GLOBAL: global
  OS: global
job:
  variables:
    OS: job
  script: [echo]
  parallel:
    matrix:
      - OS: [linux]
"#,
        );
        let job = &workflow.jobs["job"];
        assert_eq!(job.env["OS"], "${{ matrix.OS }}");
        assert_eq!(job.env["GLOBAL"], "global");
    }

    fn node_indexes(workflow: &workflow::WorkflowDefinition) -> Vec<u64> {
        let matrix = workflow.jobs["job"].matrix_config().unwrap();
        let mut indexes: Vec<u64> = wrkflw_matrix::expand_matrix(matrix)
            .unwrap()
            .iter()
            .map(|combination| combination.values["CI_NODE_INDEX"].as_u64().unwrap())
            .collect();
        indexes.sort();
        indexes
    }

    #[test]
    fn parallel_count_sets_node_variables() {
        let workflow = convert("job:\n  script: [echo]\n  parallel: 3\n");
        let job = &workflow.jobs["job"];
        assert_eq!(node_indexes(&workflow), vec![1, 2, 3]);
        assert_eq!(job.env["CI_NODE_TOTAL"], "3");
        assert_eq!(job.env["CI_NODE_INDEX"], "${{ matrix.CI_NODE_INDEX }}");
    }

    #[test]
    fn parallel_matrix_sets_node_variables() {
        let workflow = convert(
            "job:\n  script: [echo]\n  parallel:\n    matrix:\n      - A: [x, y]\n      - B: z\n",
        );
        let job = &workflow.jobs["job"];
        assert_eq!(node_indexes(&workflow), vec![1, 2, 3]);
        assert_eq!(job.env["CI_NODE_TOTAL"], "3");
    }

    #[test]
    fn parallel_count_is_bounded_when_parsing() {
        for count in ["1", "200"] {
            let yaml = format!("job:\n  script: [echo]\n  parallel: {}\n", count);
            assert!(parse(&yaml).is_ok(), "parallel: {}", count);
        }
        for count in ["0", "201", "\"3\""] {
            let yaml = format!("job:\n  script: [echo]\n  parallel: {}\n", count);
            assert!(
                matches!(
                    parse(&yaml),
                    Err(GitlabParserError::SchemaValidationError(_))
                ),
                "parallel: {}",
                count
            );
        }
    }

    #[test]
    fn parallel_matrix_is_limited_to_200_instances() {
        let values = |count: usize| (0..count).map(|v| format!("v{}", v)).collect::<Vec<_>>();
        let entry = |count: usize| format!("      - A: [{}]\n", values(count).join(", "));

        assert!(parse_matrix(&entry(200)).is_ok());
        let error = parse_matrix(&format!("{}{}", entry(200), entry(1)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Job 'job'"), "{}", error);
        assert!(error.contains("more than 200"), "{}", error);
    }

    #[test]
    fn parallel_matrix_rejects_entries_that_run_nothing() {
        let error = parse_matrix("      - {}\n").unwrap_err().to_string();
        assert!(error.contains("no variables"), "{}", error);
        let error = parse_matrix("      - A: []\n").unwrap_err().to_string();
        assert!(error.contains("'A' has no values"), "{}", error);
    }

    #[test]
    fn parallel_matrix_rejects_invalid_variable_names() {
        let error = parse_matrix("      - \"NOT VALID\": x\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("'NOT VALID'"), "{}", error);
    }
}
