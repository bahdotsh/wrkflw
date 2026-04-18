//! GitHub Actions expression evaluator.
//!
//! Implements the expression language used inside `${{ }}` blocks in GitHub
//! Actions workflows. Supports context references (`inputs.*`, `env.*`,
//! `github.*`, `runner.*`, `matrix.*`, `steps.*.outputs.*`), operators
//! (`==`, `!=`, `&&`, `||`, `!`, comparisons), string/number/boolean literals,
//! and built-in functions (`contains`, `startsWith`, `endsWith`, `format`,
//! `success`, `failure`, `always`, `cancelled`).

use serde_yaml::Value;
use std::collections::{HashMap, HashSet};

// serde_json is used by toJSON() for robust string escaping.
use serde_json;

// ---------------------------------------------------------------------------
// Value type
// ---------------------------------------------------------------------------

/// Runtime value in the GitHub Actions expression language.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprValue {
    String(String),
    Number(f64),
    Bool(bool),
    Null,
    /// A key-value map, used for context objects like `env`, `github`, etc.
    Object(HashMap<String, ExprValue>),
}

impl ExprValue {
    /// GitHub Actions truthiness: `false`, `0`, `""`, and `null` are falsy.
    pub fn is_truthy(&self) -> bool {
        match self {
            ExprValue::Bool(b) => *b,
            ExprValue::Number(n) => *n != 0.0 && !n.is_nan(),
            ExprValue::String(s) => !s.is_empty(),
            ExprValue::Null => false,
            ExprValue::Object(_) => true,
        }
    }

    /// Coerce to string for substitution output.
    pub fn to_output_string(&self) -> String {
        match self {
            ExprValue::String(s) => s.clone(),
            ExprValue::Number(n) => {
                if n.is_finite() && *n == (*n as i64) as f64 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            ExprValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
            ExprValue::Null => String::new(),
            ExprValue::Object(map) => {
                // GHA coerces objects to their JSON representation in string contexts.
                let sorted: std::collections::BTreeMap<&String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k, expr_to_json(v))).collect();
                serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| "{}".to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Ident(String),
    StringLit(String),
    NumberLit(f64),
    True,
    False,
    Null,
    Dot,
    LParen,
    RParen,
    Comma,
    Eq,  // ==
    Ne,  // !=
    Lt,  // <
    Le,  // <=
    Gt,  // >
    Ge,  // >=
    And, // &&
    Or,  // ||
    Not, // !
    Eof,
}

struct Tokenizer<'a> {
    input: &'a str,
    pos: usize, // byte offset into `input`
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_whitespace(&mut self) {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn tokenize(&mut self) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.input.len() {
                tokens.push(Token::Eof);
                return Ok(tokens);
            }
            let bytes = self.input.as_bytes();
            let ch = bytes[self.pos] as char;
            match ch {
                '.' => {
                    tokens.push(Token::Dot);
                    self.pos += 1;
                }
                '(' => {
                    tokens.push(Token::LParen);
                    self.pos += 1;
                }
                ')' => {
                    tokens.push(Token::RParen);
                    self.pos += 1;
                }
                ',' => {
                    tokens.push(Token::Comma);
                    self.pos += 1;
                }
                '=' => {
                    if self.peek_next_byte() == Some(b'=') {
                        tokens.push(Token::Eq);
                        self.pos += 2;
                    } else {
                        return Err(format!("unexpected '=' at position {}", self.pos));
                    }
                }
                '!' => {
                    if self.peek_next_byte() == Some(b'=') {
                        tokens.push(Token::Ne);
                        self.pos += 2;
                    } else {
                        tokens.push(Token::Not);
                        self.pos += 1;
                    }
                }
                '<' => {
                    if self.peek_next_byte() == Some(b'=') {
                        tokens.push(Token::Le);
                        self.pos += 2;
                    } else {
                        tokens.push(Token::Lt);
                        self.pos += 1;
                    }
                }
                '>' => {
                    if self.peek_next_byte() == Some(b'=') {
                        tokens.push(Token::Ge);
                        self.pos += 2;
                    } else {
                        tokens.push(Token::Gt);
                        self.pos += 1;
                    }
                }
                '&' => {
                    if self.peek_next_byte() == Some(b'&') {
                        tokens.push(Token::And);
                        self.pos += 2;
                    } else {
                        return Err(format!("unexpected '&' at position {}", self.pos));
                    }
                }
                '|' => {
                    if self.peek_next_byte() == Some(b'|') {
                        tokens.push(Token::Or);
                        self.pos += 2;
                    } else {
                        return Err(format!("unexpected '|' at position {}", self.pos));
                    }
                }
                '\'' => {
                    tokens.push(self.read_string()?);
                }
                c if c.is_ascii_digit() => {
                    tokens.push(self.read_number()?);
                }
                c if c.is_ascii_alphabetic() || c == '_' => {
                    let ident = self.read_ident();
                    tokens.push(match ident.as_str() {
                        "true" => Token::True,
                        "false" => Token::False,
                        "null" => Token::Null,
                        _ => Token::Ident(ident),
                    });
                }
                _ => {
                    // Decode the actual char at this position for the error message
                    let actual_ch = self.input[self.pos..].chars().next().unwrap_or(ch);
                    return Err(format!(
                        "unexpected character '{}' at position {}",
                        actual_ch, self.pos
                    ));
                }
            }
        }
    }

    /// Peek at the next byte (used only for ASCII operator lookahead).
    fn peek_next_byte(&self) -> Option<u8> {
        let bytes = self.input.as_bytes();
        if self.pos + 1 < bytes.len() {
            Some(bytes[self.pos + 1])
        } else {
            None
        }
    }

    /// Read a single-quoted string literal, handling multi-byte UTF-8 correctly.
    fn read_string(&mut self) -> Result<Token, String> {
        self.pos += 1; // skip opening quote
        let mut s = String::new();
        while self.pos < self.input.len() {
            // Iterate chars from current position to handle multi-byte correctly
            let ch = self.input[self.pos..].chars().next().unwrap();
            if ch == '\'' {
                // Check for escaped quote ('')
                let next_pos = self.pos + 1;
                if next_pos < self.input.len() && self.input.as_bytes()[next_pos] == b'\'' {
                    s.push('\'');
                    self.pos += 2;
                } else {
                    self.pos += 1; // skip closing quote
                    return Ok(Token::StringLit(s));
                }
            } else {
                s.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err("unterminated string literal".to_string())
    }

    fn read_number(&mut self) -> Result<Token, String> {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len()
            && (bytes[self.pos].is_ascii_digit() || bytes[self.pos] == b'.')
        {
            self.pos += 1;
        }
        let s = &self.input[start..self.pos];
        let n: f64 = s
            .parse()
            .map_err(|e| format!("invalid number '{}': {}", s, e))?;
        Ok(Token::NumberLit(n))
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() {
            let ch = bytes[self.pos];
            if ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }
}

/// Returns `true` if this env-var key belongs to the user-defined `env:` context
/// rather than an internal variable injected by the executor/runner.
///
/// KNOWN LIMITATION: Because `env_context` is a single flat HashMap that mixes
/// user-declared env vars with runner-injected ones, we use a heuristic prefix
/// filter. This means a user-defined var like `env: { GITHUB_CUSTOM: "val" }`
/// or the commonly used `env: { GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }} }`
/// will be incorrectly excluded from `toJSON(env)` output. The proper fix is to
/// separate user env from runner env upstream in ExpressionContext, but that is
/// a larger refactor tracked separately.
///
/// Update this function when new internal prefixes are introduced.
pub(crate) fn is_user_env_var(key: &str) -> bool {
    !key.starts_with("GITHUB_")
        && !key.starts_with("RUNNER_")
        && !key.starts_with("INPUT_")
        && !key.starts_with("WRKFLW_")
        && key != "CI"
        && key != "MATRIX_CONTEXT" // inserted by add_matrix_context() in environment.rs
}

/// If `key` is a `GITHUB_*` env var that belongs on GHA's `github.*` expression
/// context, returns the stripped + lowercased suffix used as the object key
/// (`GITHUB_SHA` → `"sha"`). Returns `None` for non-`GITHUB_*` keys, the bare
/// `GITHUB_` prefix, and runner-internal env vars that real GHA does not expose
/// on the `github` context.
///
/// The excluded suffixes fall into two groups, both seeded by
/// `environment.rs::create_github_context`:
///   - File-path vars for the workflow-command protocol (`GITHUB_OUTPUT`,
///     `GITHUB_ENV`, `GITHUB_PATH`, `GITHUB_STEP_SUMMARY`) — these point at
///     local tempfiles; leaking them diverges from real GHA and leaks paths.
///   - CI-detection vars (`GITHUB_ACTIONS`) — documented as default runner
///     env, not as a `github.*` context property.
///
/// Update this function when new runner-internal `GITHUB_*` vars are seeded.
pub(crate) fn github_context_suffix(key: &str) -> Option<String> {
    let rest = key.strip_prefix("GITHUB_")?;
    if rest.is_empty() {
        return None;
    }
    let suffix = rest.to_ascii_lowercase();
    if matches!(
        suffix.as_str(),
        "output" | "env" | "path" | "step_summary" | "actions"
    ) {
        return None;
    }
    Some(suffix)
}

// ---------------------------------------------------------------------------
// Expression context
// ---------------------------------------------------------------------------

/// Provides variable resolution for expression evaluation.
pub struct ExpressionContext<'a> {
    pub env_context: &'a HashMap<String, String>,
    pub step_outputs: &'a HashMap<String, HashMap<String, String>>,
    pub matrix_combination: &'a Option<HashMap<String, Value>>,
    /// Step ID → (outcome, conclusion) where values are "success", "failure", or "skipped".
    /// `outcome` is the raw result before `continue-on-error`; `conclusion` is the effective result.
    pub step_statuses: &'a HashMap<String, (String, String)>,
    /// Current job status for `success()`/`failure()`/`cancelled()` builtins:
    /// "success", "failure", or "cancelled".
    pub job_status: &'a str,
    /// Pre-resolved secrets for `secrets.*` context.
    pub secrets_context: &'a HashMap<String, String>,
    /// Job outputs from upstream jobs: `job_name -> { output_key -> output_value }`.
    pub needs_context: &'a HashMap<String, HashMap<String, String>>,
    /// Job results from upstream jobs: `job_name -> "success" | "failure" | "skipped"`.
    pub needs_results: &'a HashMap<String, String>,
}

impl<'a> ExpressionContext<'a> {
    /// Resolve a dotted context reference like `inputs.toolchain` or
    /// `steps.build.outputs.version`.
    fn resolve(&self, parts: &[String]) -> ExprValue {
        if parts.is_empty() {
            return ExprValue::Null;
        }

        let root = parts[0].as_str();
        match root {
            "inputs" if parts.len() == 2 => {
                let env_key = format!("INPUT_{}", parts[1].to_uppercase().replace('-', "_"));
                self.env_context
                    .get(&env_key)
                    .map(|v| ExprValue::String(v.clone()))
                    .unwrap_or(ExprValue::Null)
            }
            "env" if parts.len() == 2 => self
                .env_context
                .get(&parts[1])
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            "github" if parts.len() >= 2 => {
                // Support nested github context like github.event.action,
                // github.event.pull_request.number, etc.
                // Map dotted path to GITHUB_ env var with underscores.
                //
                // LIMITATION: In real GitHub Actions, `github.event.*` is a deep
                // JSON object parsed from the webhook payload (`$GITHUB_EVENT_PATH`).
                // Here we approximate it via flat GITHUB_* environment variables,
                // which works for simple top-level properties (e.g. `github.event.action`,
                // `github.ref_name`) but will return Null for deeply-nested event
                // properties that don't have a corresponding env var.
                let env_key = format!("GITHUB_{}", parts[1..].join("_").to_uppercase());
                self.env_context
                    .get(&env_key)
                    .map(|v| ExprValue::String(v.clone()))
                    .unwrap_or(ExprValue::Null)
            }
            "runner" if parts.len() == 2 => {
                let env_key = format!("RUNNER_{}", parts[1].to_uppercase());
                self.env_context
                    .get(&env_key)
                    .map(|v| ExprValue::String(v.clone()))
                    .unwrap_or(ExprValue::Null)
            }
            "matrix" if parts.len() == 2 => {
                if let Some(matrix) = self.matrix_combination {
                    matrix
                        .get(&parts[1])
                        .map(yaml_value_to_expr)
                        .unwrap_or(ExprValue::Null)
                } else {
                    ExprValue::Null
                }
            }
            "steps" if parts.len() == 4 && parts[2] == "outputs" => self
                .step_outputs
                .get(&parts[1])
                .and_then(|m| m.get(&parts[3]))
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            "needs" if parts.len() == 4 && parts[2] == "outputs" => self
                .needs_context
                .get(&parts[1])
                .and_then(|m| m.get(&parts[3]))
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            "needs" if parts.len() == 3 && parts[2] == "result" => self
                .needs_results
                .get(&parts[1])
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            // jobs.* context — In real GitHub Actions, this is only available in
            // workflow_call output mapping contexts, not in step expressions. We alias
            // it to needs.* data here as a pragmatic approximation that covers the most
            // common use case (reusable workflow outputs). Note: jobs.*.result does not
            // exist in real GHA (only needs.*.result does), so we only support outputs.
            "jobs" if parts.len() == 4 && parts[2] == "outputs" => self
                .needs_context
                .get(&parts[1])
                .and_then(|m| m.get(&parts[3]))
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            "secrets" if parts.len() == 2 => self
                .secrets_context
                .get(&parts[1])
                .map(|v| ExprValue::String(v.clone()))
                .unwrap_or(ExprValue::Null),
            "steps" if parts.len() == 3 && parts[2] == "outcome" => self
                .step_statuses
                .get(&parts[1])
                .map(|(outcome, _)| ExprValue::String(outcome.clone()))
                .unwrap_or(ExprValue::Null),
            "steps" if parts.len() == 3 && parts[2] == "conclusion" => self
                .step_statuses
                .get(&parts[1])
                .map(|(_, conclusion)| ExprValue::String(conclusion.clone()))
                .unwrap_or(ExprValue::Null),
            // Bare context names — return the whole context as an Object so that
            // `toJSON(env)` (and similar) can serialise it.
            // TODO: support other bare contexts: matrix
            "steps" if parts.len() == 1 => {
                // Collect all step IDs from both outputs and statuses maps.
                let mut all_ids: HashSet<&String> = self.step_outputs.keys().collect();
                all_ids.extend(self.step_statuses.keys());

                let mut map = HashMap::new();
                for step_id in all_ids {
                    let mut step_obj = HashMap::new();

                    // outputs sub-object (empty if no outputs recorded)
                    let outputs_map: HashMap<String, ExprValue> = self
                        .step_outputs
                        .get(step_id)
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| (k.clone(), ExprValue::String(v.clone())))
                                .collect()
                        })
                        .unwrap_or_default();
                    step_obj.insert("outputs".to_string(), ExprValue::Object(outputs_map));

                    // outcome + conclusion (only present if the step has a status)
                    if let Some((outcome, conclusion)) = self.step_statuses.get(step_id) {
                        step_obj.insert("outcome".to_string(), ExprValue::String(outcome.clone()));
                        step_obj.insert(
                            "conclusion".to_string(),
                            ExprValue::String(conclusion.clone()),
                        );
                    }

                    map.insert(step_id.clone(), ExprValue::Object(step_obj));
                }
                ExprValue::Object(map)
            }
            "env" if parts.len() == 1 => {
                let map = self
                    .env_context
                    .iter()
                    .filter(|(k, _)| is_user_env_var(k))
                    .map(|(k, v)| (k.clone(), ExprValue::String(v.clone())))
                    .collect();
                ExprValue::Object(map)
            }
            "github" if parts.len() == 1 => {
                // Build a flat object from GITHUB_* env vars by stripping the
                // prefix and lowercasing the remainder, inverting the dotted-access
                // mapping (`github.sha` → `GITHUB_SHA`). Runner-internal keys that
                // aren't part of GHA's `github` context are filtered out inside
                // `github_context_suffix`. Does not include a nested `event`
                // sub-object — same documented limitation as the dotted-access arm
                // above.
                //
                // KNOWN LIMITATION: the inverse of `toJSON(env)`'s prefix heuristic
                // applies here — any user-defined env var starting with `GITHUB_`
                // (e.g. the common `env: { GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }} }`)
                // will appear in this object. In particular `GITHUB_TOKEN`, if set,
                // surfaces as `github.token` in plaintext; do not dump this object
                // to untrusted sinks without a masking layer.
                let map = self
                    .env_context
                    .iter()
                    .filter_map(|(k, v)| {
                        github_context_suffix(k).map(|key| (key, ExprValue::String(v.clone())))
                    })
                    .collect();
                ExprValue::Object(map)
            }
            "needs" if parts.len() == 1 => {
                // Collect all job IDs from both outputs and results maps.
                let mut all_ids: HashSet<&String> = self.needs_context.keys().collect();
                all_ids.extend(self.needs_results.keys());

                let mut map = HashMap::new();
                for job_id in all_ids {
                    let mut job_obj = HashMap::new();

                    // outputs sub-object (empty if no outputs recorded)
                    let outputs_map: HashMap<String, ExprValue> = self
                        .needs_context
                        .get(job_id)
                        .map(|m| {
                            m.iter()
                                .map(|(k, v)| (k.clone(), ExprValue::String(v.clone())))
                                .collect()
                        })
                        .unwrap_or_default();
                    job_obj.insert("outputs".to_string(), ExprValue::Object(outputs_map));

                    // result (only present if the job has a recorded result)
                    if let Some(result) = self.needs_results.get(job_id) {
                        job_obj.insert("result".to_string(), ExprValue::String(result.clone()));
                    }

                    map.insert(job_id.clone(), ExprValue::Object(job_obj));
                }
                ExprValue::Object(map)
            }
            "secrets" if parts.len() == 1 => {
                // Wrap `secrets_context` as an Object so `toJSON(secrets)` can
                // serialise it. Mirrors real GHA's `secrets` context shape
                // (flat `{ name: value }` map).
                //
                // Values are returned in plaintext by design — same policy as
                // `toJSON(github)` for `GITHUB_TOKEN`. Masking is a log-boundary
                // concern handled by `wrkflw_secrets::SecretMasker` when wired in
                // via `engine.rs`. Do not dump this object to untrusted sinks
                // without routing through the masker.
                let map = self
                    .secrets_context
                    .iter()
                    .map(|(k, v)| (k.clone(), ExprValue::String(v.clone())))
                    .collect();
                ExprValue::Object(map)
            }
            _ => ExprValue::Null,
        }
    }
}

fn yaml_value_to_expr(v: &Value) -> ExprValue {
    match v {
        Value::String(s) => ExprValue::String(s.clone()),
        Value::Number(n) => ExprValue::Number(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => ExprValue::Bool(*b),
        Value::Null => ExprValue::Null,
        _ => ExprValue::String(
            serde_yaml::to_string(v)
                .unwrap_or_else(|_| format!("{:?}", v))
                .trim()
                .to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Parser + Evaluator (recursive descent)
// ---------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        let tok = self.advance();
        if &tok == expected {
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", expected, tok))
        }
    }

    // Grammar: expr = or_expr
    fn parse_expr(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        self.parse_or(ctx)
    }

    // or_expr = and_expr ( '||' and_expr )*
    fn parse_or(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        let mut left = self.parse_and(ctx)?;
        while *self.peek() == Token::Or {
            self.advance();
            let right = self.parse_and(ctx)?;
            // GitHub Actions || returns the first truthy value, or the last value
            left = if left.is_truthy() { left } else { right };
        }
        Ok(left)
    }

    // and_expr = comparison ( '&&' comparison )*
    fn parse_and(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        let mut left = self.parse_comparison(ctx)?;
        while *self.peek() == Token::And {
            self.advance();
            let right = self.parse_comparison(ctx)?;
            // GitHub Actions && returns the first falsy value, or the last value
            left = if !left.is_truthy() { left } else { right };
        }
        Ok(left)
    }

    // comparison = unary ( ('==' | '!=' | '<' | '<=' | '>' | '>=') unary )?
    fn parse_comparison(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        let left = self.parse_unary(ctx)?;
        match self.peek().clone() {
            Token::Eq => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(expr_eq(&left, &right)))
            }
            Token::Ne => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(!expr_eq(&left, &right)))
            }
            Token::Lt => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(
                    expr_cmp(&left, &right) == Some(std::cmp::Ordering::Less),
                ))
            }
            Token::Le => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(matches!(
                    expr_cmp(&left, &right),
                    Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
                )))
            }
            Token::Gt => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(
                    expr_cmp(&left, &right) == Some(std::cmp::Ordering::Greater),
                ))
            }
            Token::Ge => {
                self.advance();
                let right = self.parse_unary(ctx)?;
                Ok(ExprValue::Bool(matches!(
                    expr_cmp(&left, &right),
                    Some(std::cmp::Ordering::Greater | std::cmp::Ordering::Equal)
                )))
            }
            _ => Ok(left),
        }
    }

    // unary = '!' unary | primary
    fn parse_unary(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        if *self.peek() == Token::Not {
            self.advance();
            let val = self.parse_unary(ctx)?;
            Ok(ExprValue::Bool(!val.is_truthy()))
        } else {
            self.parse_primary(ctx)
        }
    }

    // primary = literal | '(' expr ')' | ident_or_call
    fn parse_primary(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        match self.peek().clone() {
            Token::StringLit(s) => {
                self.advance();
                Ok(ExprValue::String(s))
            }
            Token::NumberLit(n) => {
                self.advance();
                Ok(ExprValue::Number(n))
            }
            Token::True => {
                self.advance();
                Ok(ExprValue::Bool(true))
            }
            Token::False => {
                self.advance();
                Ok(ExprValue::Bool(false))
            }
            Token::Null => {
                self.advance();
                Ok(ExprValue::Null)
            }
            Token::LParen => {
                self.advance();
                let val = self.parse_expr(ctx)?;
                self.expect(&Token::RParen)?;
                Ok(val)
            }
            Token::Ident(_) => self.parse_ident_or_call(ctx),
            Token::Not => self.parse_unary(ctx),
            other => Err(format!("unexpected token: {:?}", other)),
        }
    }

    // ident_or_call:
    //   ident '(' args ')' => function call
    //   ident ('.' ident)* => context reference
    fn parse_ident_or_call(&mut self, ctx: &ExpressionContext) -> Result<ExprValue, String> {
        let Token::Ident(name) = self.advance() else {
            return Err("expected identifier".to_string());
        };

        // Function call?
        if *self.peek() == Token::LParen {
            self.advance(); // consume '('
            let mut args = Vec::new();
            if *self.peek() != Token::RParen {
                args.push(self.parse_expr(ctx)?);
                while *self.peek() == Token::Comma {
                    self.advance();
                    args.push(self.parse_expr(ctx)?);
                }
            }
            self.expect(&Token::RParen)?;
            return call_builtin(&name, &args, ctx);
        }

        // Context reference: ident.ident.ident...
        let mut parts = vec![name];
        while *self.peek() == Token::Dot {
            self.advance(); // consume '.'
            match self.advance() {
                Token::Ident(part) => parts.push(part),
                other => return Err(format!("expected identifier after '.', got {:?}", other)),
            }
        }

        Ok(ctx.resolve(&parts))
    }
}

// ---------------------------------------------------------------------------
// Comparison helpers
// ---------------------------------------------------------------------------

fn expr_eq(a: &ExprValue, b: &ExprValue) -> bool {
    // GitHub Actions does loose type coercion for ==
    match (a, b) {
        (ExprValue::Null, ExprValue::Null) => true,
        (ExprValue::Null, _) | (_, ExprValue::Null) => false,
        (ExprValue::Bool(a), ExprValue::Bool(b)) => a == b,
        (ExprValue::Number(a), ExprValue::Number(b)) => (a - b).abs() < f64::EPSILON,
        (ExprValue::String(a), ExprValue::String(b)) => a.eq_ignore_ascii_case(b),
        // Coerce number to string for comparison
        (ExprValue::String(s), ExprValue::Number(n))
        | (ExprValue::Number(n), ExprValue::String(s)) => {
            if let Ok(parsed) = s.parse::<f64>() {
                (parsed - n).abs() < f64::EPSILON
            } else {
                false
            }
        }
        // Coerce bool to number: true=1, false=0
        (ExprValue::Bool(b), ExprValue::Number(n)) | (ExprValue::Number(n), ExprValue::Bool(b)) => {
            let bv = if *b { 1.0 } else { 0.0 };
            (bv - n).abs() < f64::EPSILON
        }
        (ExprValue::Bool(b), ExprValue::String(s)) | (ExprValue::String(s), ExprValue::Bool(b)) => {
            // GitHub Actions coerces strings to booleans for comparison:
            // "true" (case-insensitive) → true, everything else → false.
            // This means `false == "random"` is true (both coerce to false).
            let sv = s.eq_ignore_ascii_case("true");
            *b == sv
        }
        // Objects are not comparable via ==
        (ExprValue::Object(_), _) | (_, ExprValue::Object(_)) => false,
    }
}

fn expr_cmp(a: &ExprValue, b: &ExprValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (ExprValue::Number(a), ExprValue::Number(b)) => a.partial_cmp(b),
        (ExprValue::String(a), ExprValue::String(b)) => {
            Some(a.to_lowercase().cmp(&b.to_lowercase()))
        }
        // Objects are not orderable — comparisons like `env < env` yield None
        // (meaning the comparison expression will evaluate to false).
        (ExprValue::Object(_), _) | (_, ExprValue::Object(_)) => None,
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Built-in functions
// ---------------------------------------------------------------------------

/// Convert an `ExprValue` to a `serde_json::Value` for JSON serialisation.
fn expr_to_json(v: &ExprValue) -> serde_json::Value {
    match v {
        ExprValue::String(s) => serde_json::Value::String(s.clone()),
        ExprValue::Number(n) => serde_json::json!(n),
        ExprValue::Bool(b) => serde_json::Value::Bool(*b),
        ExprValue::Null => serde_json::Value::Null,
        ExprValue::Object(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), expr_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

fn call_builtin(
    name: &str,
    args: &[ExprValue],
    ctx: &ExpressionContext,
) -> Result<ExprValue, String> {
    match name {
        "contains" => {
            if args.len() != 2 {
                return Err("contains() requires 2 arguments".to_string());
            }
            let haystack = args[0].to_output_string().to_lowercase();
            let needle = args[1].to_output_string().to_lowercase();
            Ok(ExprValue::Bool(haystack.contains(&needle)))
        }
        "startsWith" | "startswith" => {
            if args.len() != 2 {
                return Err("startsWith() requires 2 arguments".to_string());
            }
            let s = args[0].to_output_string().to_lowercase();
            let prefix = args[1].to_output_string().to_lowercase();
            Ok(ExprValue::Bool(s.starts_with(&prefix)))
        }
        "endsWith" | "endswith" => {
            if args.len() != 2 {
                return Err("endsWith() requires 2 arguments".to_string());
            }
            let s = args[0].to_output_string().to_lowercase();
            let suffix = args[1].to_output_string().to_lowercase();
            Ok(ExprValue::Bool(s.ends_with(&suffix)))
        }
        "format" => {
            if args.is_empty() {
                return Err("format() requires at least 1 argument".to_string());
            }
            let fmt = args[0].to_output_string();
            // Single-pass replacement to prevent arg content from being consumed
            // by later placeholder substitutions (e.g. format('{0} {1}', '{1}', 'x')
            // should produce '{1} x', not 'x x').
            let mut result = String::with_capacity(fmt.len());
            let mut chars = fmt.char_indices().peekable();
            while let Some((i, ch)) = chars.next() {
                if ch == '{' {
                    // Look for {N} pattern
                    let rest = &fmt[i + 1..];
                    if let Some(close) = rest.find('}') {
                        let inner = &rest[..close];
                        if let Ok(idx) = inner.parse::<usize>() {
                            if idx + 1 < args.len() {
                                result.push_str(&args[idx + 1].to_output_string());
                                // Skip past the closing '}'
                                let skip_to = i + 1 + close + 1;
                                while chars.peek().is_some_and(|(ci, _)| *ci < skip_to) {
                                    chars.next();
                                }
                                continue;
                            }
                        }
                    }
                }
                result.push(ch);
            }
            Ok(ExprValue::String(result))
        }
        "join" => {
            if args.is_empty() || args.len() > 2 {
                return Err("join() requires 1 or 2 arguments".to_string());
            }
            let sep = if args.len() == 2 {
                args[1].to_output_string()
            } else {
                ",".to_string()
            };
            // Best-effort: just return the value as-is since we don't have arrays
            Ok(ExprValue::String(
                args[0].to_output_string().replace(',', &sep),
            ))
        }
        "toJSON" | "tojson" => {
            if args.len() != 1 {
                return Err("toJSON() requires 1 argument".to_string());
            }
            match &args[0] {
                ExprValue::String(s) => {
                    // Use serde_json for robust escaping (handles control chars, null bytes, etc.)
                    Ok(ExprValue::String(
                        serde_json::to_string(s).unwrap_or_else(|_| format!("\"{}\"", s)),
                    ))
                }
                ExprValue::Number(n) => Ok(ExprValue::String(format!("{}", n))),
                ExprValue::Bool(b) => Ok(ExprValue::String(format!("{}", b))),
                ExprValue::Null => Ok(ExprValue::String("null".to_string())),
                ExprValue::Object(map) => {
                    // Serialize as sorted, pretty-printed JSON (matches GHA behaviour).
                    // Use serde_json::Value so nested objects serialize correctly.
                    let sorted: std::collections::BTreeMap<&String, serde_json::Value> =
                        map.iter().map(|(k, v)| (k, expr_to_json(v))).collect();
                    Ok(ExprValue::String(
                        serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| "{}".to_string()),
                    ))
                }
            }
        }
        "fromJSON" | "fromjson" => {
            if args.len() != 1 {
                return Err("fromJSON() requires 1 argument".to_string());
            }
            let s = args[0].to_output_string();
            // Basic parsing
            match s.as_str() {
                "null" => Ok(ExprValue::Null),
                "true" => Ok(ExprValue::Bool(true)),
                "false" => Ok(ExprValue::Bool(false)),
                _ => {
                    if let Ok(n) = s.parse::<f64>() {
                        Ok(ExprValue::Number(n))
                    } else {
                        // Strip one layer of quotes if present
                        let stripped = s
                            .strip_prefix('"')
                            .and_then(|s| s.strip_suffix('"'))
                            .unwrap_or(&s);
                        Ok(ExprValue::String(stripped.to_string()))
                    }
                }
            }
        }
        // Status functions — consult job_status from context
        "success" => Ok(ExprValue::Bool(ctx.job_status == "success")),
        "failure" => Ok(ExprValue::Bool(ctx.job_status == "failure")),
        "always" => Ok(ExprValue::Bool(true)),
        "cancelled" => Ok(ExprValue::Bool(ctx.job_status == "cancelled")),
        _ => {
            // Unknown function — return null rather than erroring
            Ok(ExprValue::Null)
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Evaluate a GitHub Actions expression string and return the result.
///
/// The expression should be the content inside `${{ ... }}` (without the
/// delimiters). Returns `Err` on parse/evaluation errors.
pub fn evaluate(expr: &str, ctx: &ExpressionContext) -> Result<ExprValue, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(ExprValue::Null);
    }
    let mut tokenizer = Tokenizer::new(trimmed);
    let tokens = tokenizer.tokenize()?;
    let mut parser = Parser::new(tokens);
    let result = parser.parse_expr(ctx)?;
    // Ensure we consumed all tokens
    if *parser.peek() != Token::Eof {
        return Err(format!(
            "unexpected token after expression: {:?}",
            parser.peek()
        ));
    }
    Ok(result)
}

/// Evaluate a GitHub Actions expression and return it as a boolean.
///
/// Used for `if:` conditions. Strips `${{ }}` wrappers if present.
pub fn evaluate_as_bool(expr: &str, ctx: &ExpressionContext) -> Result<bool, String> {
    let trimmed = expr.trim();
    // Strip ${{ }} if present
    let inner = if trimmed.starts_with("${{") && trimmed.ends_with("}}") {
        &trimmed[3..trimmed.len() - 2]
    } else {
        trimmed
    };
    let val = evaluate(inner, ctx)?;
    Ok(val.is_truthy())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    lazy_static::lazy_static! {
        static ref EMPTY_ENV: HashMap<String, String> = HashMap::new();
        static ref EMPTY_STEPS: HashMap<String, HashMap<String, String>> = HashMap::new();
        static ref EMPTY_MATRIX: Option<HashMap<String, Value>> = None;
        static ref EMPTY_STATUSES: HashMap<String, (String, String)> = HashMap::new();
        static ref EMPTY_SECRETS: HashMap<String, String> = HashMap::new();
        static ref EMPTY_NEEDS: HashMap<String, HashMap<String, String>> = HashMap::new();
        static ref EMPTY_NEEDS_RESULTS: HashMap<String, String> = HashMap::new();
    }

    fn empty_ctx() -> ExpressionContext<'static> {
        ExpressionContext {
            env_context: &EMPTY_ENV,
            step_outputs: &EMPTY_STEPS,
            matrix_combination: &EMPTY_MATRIX,
            step_statuses: &EMPTY_STATUSES,
            job_status: "success",
            secrets_context: &EMPTY_SECRETS,
            needs_context: &EMPTY_NEEDS,
            needs_results: &EMPTY_NEEDS_RESULTS,
        }
    }

    /// Build an `ExpressionContext` from the fields that vary across tests;
    /// all other fields default to empty/success.
    fn make_ctx<'a>(
        env: &'a HashMap<String, String>,
        steps: &'a HashMap<String, HashMap<String, String>>,
        matrix: &'a Option<HashMap<String, Value>>,
    ) -> ExpressionContext<'a> {
        ExpressionContext {
            env_context: env,
            step_outputs: steps,
            matrix_combination: matrix,
            step_statuses: &EMPTY_STATUSES,
            job_status: "success",
            secrets_context: &EMPTY_SECRETS,
            needs_context: &EMPTY_NEEDS,
            needs_results: &EMPTY_NEEDS_RESULTS,
        }
    }

    // -- Literals --

    #[test]
    fn eval_string_literal() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("'hello'", &ctx).unwrap(),
            ExprValue::String("hello".to_string())
        );
    }

    #[test]
    fn eval_empty_string_literal() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("''", &ctx).unwrap(),
            ExprValue::String(String::new())
        );
    }

    #[test]
    fn eval_number_literal() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("42", &ctx).unwrap(), ExprValue::Number(42.0));
    }

    #[test]
    fn eval_bool_literals() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("true", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(evaluate("false", &ctx).unwrap(), ExprValue::Bool(false));
    }

    #[test]
    fn eval_null_literal() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("null", &ctx).unwrap(), ExprValue::Null);
    }

    // -- Truthiness --

    #[test]
    fn truthiness() {
        assert!(ExprValue::Bool(true).is_truthy());
        assert!(!ExprValue::Bool(false).is_truthy());
        assert!(ExprValue::Number(1.0).is_truthy());
        assert!(!ExprValue::Number(0.0).is_truthy());
        assert!(ExprValue::String("hello".to_string()).is_truthy());
        assert!(!ExprValue::String(String::new()).is_truthy());
        assert!(!ExprValue::Null.is_truthy());
    }

    // -- Operators --

    #[test]
    fn eval_equality() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("'nightly' == 'nightly'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("'nightly' == 'stable'", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
        assert_eq!(
            evaluate("'nightly' != 'stable'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
    }

    #[test]
    fn eval_bool_string_coercion() {
        let ctx = empty_ctx();
        // GitHub Actions coerces strings to booleans: "true" → true, everything else → false.
        // So false == "random" is true because "random" coerces to false.
        assert_eq!(
            evaluate("false == 'random'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("true == 'true'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("true == 'TRUE'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("true == 'false'", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
        assert_eq!(
            evaluate("false == 'false'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
    }

    #[test]
    fn eval_case_insensitive_equality() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("'Nightly' == 'nightly'", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
    }

    #[test]
    fn eval_number_comparison() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("1 < 2", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(evaluate("2 >= 2", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(evaluate("3 <= 2", &ctx).unwrap(), ExprValue::Bool(false));
    }

    #[test]
    fn eval_and_operator() {
        let ctx = empty_ctx();
        // && returns first falsy or last value
        assert_eq!(
            evaluate("true && 'hello'", &ctx).unwrap(),
            ExprValue::String("hello".to_string())
        );
        assert_eq!(
            evaluate("false && 'hello'", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
        assert_eq!(
            evaluate("'' && 'hello'", &ctx).unwrap(),
            ExprValue::String(String::new())
        );
    }

    #[test]
    fn eval_or_operator() {
        let ctx = empty_ctx();
        // || returns first truthy or last value
        assert_eq!(
            evaluate("'hi' || 'bye'", &ctx).unwrap(),
            ExprValue::String("hi".to_string())
        );
        assert_eq!(
            evaluate("'' || 'fallback'", &ctx).unwrap(),
            ExprValue::String("fallback".to_string())
        );
        assert_eq!(
            evaluate("false || ''", &ctx).unwrap(),
            ExprValue::String(String::new())
        );
    }

    #[test]
    fn eval_not_operator() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("!true", &ctx).unwrap(), ExprValue::Bool(false));
        assert_eq!(evaluate("!false", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(evaluate("!''", &ctx).unwrap(), ExprValue::Bool(true));
    }

    #[test]
    fn eval_parentheses() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("(true || false) && false", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
    }

    // -- Context resolution --

    #[test]
    fn eval_inputs_context() {
        let mut env = HashMap::new();
        env.insert("INPUT_TOOLCHAIN".to_string(), "nightly".to_string());
        let empty_steps = HashMap::new();
        let ctx = make_ctx(&env, &empty_steps, &None);

        assert_eq!(
            evaluate("inputs.toolchain", &ctx).unwrap(),
            ExprValue::String("nightly".to_string())
        );
    }

    #[test]
    fn eval_env_context() {
        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello".to_string());
        let empty_steps = HashMap::new();
        let ctx = make_ctx(&env, &empty_steps, &None);

        assert_eq!(
            evaluate("env.MY_VAR", &ctx).unwrap(),
            ExprValue::String("hello".to_string())
        );
    }

    #[test]
    fn eval_github_context() {
        let mut env = HashMap::new();
        env.insert("GITHUB_REPOSITORY".to_string(), "owner/repo".to_string());
        let empty_steps = HashMap::new();
        let ctx = make_ctx(&env, &empty_steps, &None);

        assert_eq!(
            evaluate("github.repository", &ctx).unwrap(),
            ExprValue::String("owner/repo".to_string())
        );
    }

    #[test]
    fn eval_steps_outputs() {
        let mut steps = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("version".to_string(), "1.2.3".to_string());
        steps.insert("build".to_string(), build_out);
        let empty_env = HashMap::new();
        let ctx = make_ctx(&empty_env, &steps, &None);

        assert_eq!(
            evaluate("steps.build.outputs.version", &ctx).unwrap(),
            ExprValue::String("1.2.3".to_string())
        );
    }

    #[test]
    fn eval_matrix_context() {
        let mut matrix = HashMap::new();
        matrix.insert("os".to_string(), Value::String("ubuntu".to_string()));
        let empty_env = HashMap::new();
        let empty_steps = HashMap::new();
        let matrix = Some(matrix);
        let ctx = make_ctx(&empty_env, &empty_steps, &matrix);

        assert_eq!(
            evaluate("matrix.os", &ctx).unwrap(),
            ExprValue::String("ubuntu".to_string())
        );
    }

    #[test]
    fn eval_missing_context_returns_null() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("inputs.nonexistent", &ctx).unwrap(),
            ExprValue::Null
        );
    }

    // -- Complex expressions (the dtolnay/rust-toolchain pattern) --

    #[test]
    fn eval_rust_toolchain_pattern() {
        // ${{ steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || '' }}
        let mut env = HashMap::new();
        env.insert("INPUT_COMPONENTS".to_string(), "rustfmt".to_string());

        let mut steps = HashMap::new();
        let mut parse_out = HashMap::new();
        parse_out.insert("toolchain".to_string(), "nightly".to_string());
        steps.insert("parse".to_string(), parse_out);

        let ctx = make_ctx(&env, &steps, &None);

        let result = evaluate(
            "steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || ''",
            &ctx,
        )
        .unwrap();
        assert_eq!(result, ExprValue::String(" --allow-downgrade".to_string()));
    }

    #[test]
    fn eval_rust_toolchain_pattern_not_nightly() {
        let mut env = HashMap::new();
        env.insert("INPUT_COMPONENTS".to_string(), "rustfmt".to_string());

        let mut steps = HashMap::new();
        let mut parse_out = HashMap::new();
        parse_out.insert("toolchain".to_string(), "stable".to_string());
        steps.insert("parse".to_string(), parse_out);

        let ctx = make_ctx(&env, &steps, &None);

        let result = evaluate(
            "steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || ''",
            &ctx,
        )
        .unwrap();
        // 'stable' != 'nightly' → false, && short-circuits, || returns ''
        assert_eq!(result, ExprValue::String(String::new()));
    }

    #[test]
    fn eval_rust_toolchain_pattern_no_components() {
        let env = HashMap::new(); // no INPUT_COMPONENTS

        let mut steps = HashMap::new();
        let mut parse_out = HashMap::new();
        parse_out.insert("toolchain".to_string(), "nightly".to_string());
        steps.insert("parse".to_string(), parse_out);

        let ctx = make_ctx(&env, &steps, &None);

        let result = evaluate(
            "steps.parse.outputs.toolchain == 'nightly' && inputs.components && ' --allow-downgrade' || ''",
            &ctx,
        )
        .unwrap();
        // toolchain == nightly → true, inputs.components → null (falsy), && returns null, || returns ''
        assert_eq!(result, ExprValue::String(String::new()));
    }

    // -- Built-in functions --

    #[test]
    fn eval_contains() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("contains('Hello World', 'hello')", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("contains('Hello', 'xyz')", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
    }

    #[test]
    fn eval_starts_with() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("startsWith('refs/heads/main', 'refs/heads')", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
        assert_eq!(
            evaluate("startsWith('refs/tags/v1', 'refs/heads')", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
    }

    #[test]
    fn eval_ends_with() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("endsWith('hello.txt', '.txt')", &ctx).unwrap(),
            ExprValue::Bool(true)
        );
    }

    #[test]
    fn eval_format_function() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("format('Hello {0}, you are {1}', 'world', 'great')", &ctx).unwrap(),
            ExprValue::String("Hello world, you are great".to_string())
        );
    }

    #[test]
    fn eval_format_non_ascii() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("format('{0} → {1}', 'a', 'b')", &ctx).unwrap(),
            ExprValue::String("a → b".to_string())
        );
    }

    #[test]
    fn eval_format_out_of_bounds_placeholder_preserved() {
        let ctx = empty_ctx();
        // {5} references a non-existent arg — should be left as literal "{5}"
        assert_eq!(
            evaluate("format('{0} {5}', 'hi')", &ctx).unwrap(),
            ExprValue::String("hi {5}".to_string())
        );
    }

    #[test]
    fn eval_format_arg_containing_placeholder_not_reinterpreted() {
        let ctx = empty_ctx();
        // format('{0} {1}', '{1}', 'x') should produce '{1} x', not 'x x'
        assert_eq!(
            evaluate("format('{0} {1}', '{1}', 'x')", &ctx).unwrap(),
            ExprValue::String("{1} x".to_string())
        );
    }

    #[test]
    fn eval_status_functions() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("success()", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(evaluate("failure()", &ctx).unwrap(), ExprValue::Bool(false));
        assert_eq!(evaluate("always()", &ctx).unwrap(), ExprValue::Bool(true));
        assert_eq!(
            evaluate("cancelled()", &ctx).unwrap(),
            ExprValue::Bool(false)
        );
    }

    // -- evaluate_as_bool --

    #[test]
    fn eval_as_bool_strips_delimiters() {
        let ctx = empty_ctx();
        assert!(evaluate_as_bool("${{ true }}", &ctx).unwrap());
        assert!(!evaluate_as_bool("${{ false }}", &ctx).unwrap());
    }

    #[test]
    fn eval_as_bool_bare_expression() {
        let ctx = empty_ctx();
        assert!(evaluate_as_bool("true", &ctx).unwrap());
        assert!(!evaluate_as_bool("false", &ctx).unwrap());
    }

    #[test]
    fn eval_as_bool_condition_with_context() {
        let mut env = HashMap::new();
        env.insert("GITHUB_REF".to_string(), "refs/tags/v1.0.0".to_string());
        let empty_steps = HashMap::new();
        let ctx = make_ctx(&env, &empty_steps, &None);

        assert!(evaluate_as_bool("startsWith(github.ref, 'refs/tags/')", &ctx).unwrap());
    }

    // -- Output string formatting --

    #[test]
    fn output_string_formatting() {
        assert_eq!(ExprValue::String("hi".to_string()).to_output_string(), "hi");
        assert_eq!(ExprValue::Number(42.0).to_output_string(), "42");
        assert_eq!(ExprValue::Number(3.15).to_output_string(), "3.15");
        assert_eq!(ExprValue::Bool(true).to_output_string(), "true");
        assert_eq!(ExprValue::Null.to_output_string(), "");
    }

    // -- Error cases --

    #[test]
    fn eval_unterminated_string_errors() {
        let ctx = empty_ctx();
        assert!(evaluate("'unterminated", &ctx).is_err());
    }

    #[test]
    fn eval_unexpected_token_errors() {
        let ctx = empty_ctx();
        assert!(evaluate("&&", &ctx).is_err());
    }

    #[test]
    fn eval_empty_expression() {
        let ctx = empty_ctx();
        assert_eq!(evaluate("", &ctx).unwrap(), ExprValue::Null);
    }

    #[test]
    fn unknown_step_id_returns_null() {
        let ctx = empty_ctx();
        assert_eq!(
            evaluate("steps.nonexistent.outcome", &ctx).unwrap(),
            ExprValue::Null
        );
        assert_eq!(
            evaluate("steps.nonexistent.conclusion", &ctx).unwrap(),
            ExprValue::Null
        );
    }

    #[test]
    fn tojson_escapes_control_characters() {
        let ctx = empty_ctx();
        // Tab, newline, carriage return
        let result = evaluate("toJSON('line1\tindented\nline2\rend')", &ctx).unwrap();
        let s = result.to_output_string();
        assert!(s.contains("\\t"), "should escape tab: {}", s);
        assert!(s.contains("\\n"), "should escape newline: {}", s);
        assert!(s.contains("\\r"), "should escape carriage return: {}", s);
    }

    #[test]
    fn tojson_escapes_quotes_and_backslash() {
        let ctx = empty_ctx();
        let result = evaluate(r#"toJSON('say "hello\world"')"#, &ctx).unwrap();
        let s = result.to_output_string();
        assert!(s.contains(r#"\""#), "should escape quotes: {}", s);
        assert!(s.contains(r"\\"), "should escape backslash: {}", s);
    }

    #[test]
    fn tojson_handles_null_bytes() {
        let ctx = empty_ctx();
        // Null byte in string — serde_json encodes as \u0000
        let result = evaluate("toJSON('before\x00after')", &ctx).unwrap();
        let s = result.to_output_string();
        assert!(!s.contains('\0'), "should not contain raw null: {}", s);
    }

    #[test]
    fn tojson_env_returns_object() {
        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello".to_string());
        env.insert("OTHER".to_string(), "world".to_string());
        // System vars should be excluded from the env object
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());
        env.insert("INPUT_NAME".to_string(), "test".to_string());
        env.insert("CI".to_string(), "true".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("MY_VAR").unwrap(), "hello");
        assert_eq!(obj.get("OTHER").unwrap(), "world");
        assert!(
            obj.get("GITHUB_SHA").is_none(),
            "should exclude GITHUB_ vars"
        );
        assert!(
            obj.get("RUNNER_OS").is_none(),
            "should exclude RUNNER_ vars"
        );
        assert!(
            obj.get("INPUT_NAME").is_none(),
            "should exclude INPUT_ vars"
        );
        assert!(obj.get("CI").is_none(), "should exclude CI");
    }

    #[test]
    fn tojson_env_sorted_keys() {
        let mut env = HashMap::new();
        env.insert("ZEBRA".to_string(), "z".to_string());
        env.insert("APPLE".to_string(), "a".to_string());
        env.insert("MANGO".to_string(), "m".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        // Keys should appear in alphabetical order
        let apple_pos = s.find("APPLE").unwrap();
        let mango_pos = s.find("MANGO").unwrap();
        let zebra_pos = s.find("ZEBRA").unwrap();
        assert!(apple_pos < mango_pos, "APPLE should come before MANGO");
        assert!(mango_pos < zebra_pos, "MANGO should come before ZEBRA");
    }

    #[test]
    fn bare_env_is_truthy() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        // `env` alone should be truthy (it's an object)
        let result = evaluate("env", &ctx).unwrap();
        assert!(result.is_truthy());
    }

    #[test]
    fn bare_env_to_output_string() {
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("env", &ctx).unwrap();
        // GHA coerces objects to their JSON representation in string contexts.
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("FOO").unwrap(), "bar");
    }

    #[test]
    fn tojson_env_empty_when_only_internal_vars() {
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());
        env.insert("CI".to_string(), "true".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(
            obj.is_empty(),
            "should be empty when all vars are internal: {}",
            s
        );
    }

    #[test]
    fn tojson_env_excludes_user_var_with_internal_prefix() {
        // KNOWN LIMITATION: user-defined vars that happen to match internal
        // prefixes (GITHUB_*, RUNNER_*, etc.) are incorrectly filtered out.
        let mut env = HashMap::new();
        env.insert("GITHUB_CUSTOM".to_string(), "user-val".to_string());
        env.insert("MY_VAR".to_string(), "hello".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("MY_VAR").unwrap(), "hello");
        // GITHUB_CUSTOM is excluded despite being user-defined — this is the
        // known limitation of the heuristic prefix filter.
        assert!(
            obj.get("GITHUB_CUSTOM").is_none(),
            "user var with GITHUB_ prefix is incorrectly excluded (known limitation)"
        );
    }

    #[test]
    fn tojson_env_empty_context() {
        let env = HashMap::new();
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.is_empty(), "should be empty with no env vars: {}", s);
    }

    #[test]
    fn fromjson_tojson_env_produces_parseable_json() {
        // Note: fromJSON currently returns an ExprValue::String containing
        // the raw JSON text, not an ExprValue::Object. This test verifies
        // that the string output is valid, parseable JSON with expected keys.
        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello".to_string());
        env.insert("OTHER".to_string(), "world".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("fromJSON(toJSON(env))", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("MY_VAR").unwrap(), "hello");
        assert_eq!(obj.get("OTHER").unwrap(), "world");
    }

    #[test]
    fn tojson_env_special_characters_in_values() {
        let mut env = HashMap::new();
        env.insert("QUOTED".to_string(), "he said \"hi\"".to_string());
        env.insert("NEWLINE".to_string(), "line1\nline2".to_string());
        env.insert("UNICODE".to_string(), "\u{1F600}".to_string());
        env.insert("BACKSLASH".to_string(), "path\\to\\file".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(env)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("should be valid JSON despite special chars");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("QUOTED").unwrap(), "he said \"hi\"");
        assert_eq!(obj.get("NEWLINE").unwrap(), "line1\nline2");
        assert_eq!(obj.get("UNICODE").unwrap(), "\u{1F600}");
        assert_eq!(obj.get("BACKSLASH").unwrap(), "path\\to\\file");
    }

    // -- toJSON(github) tests --

    #[test]
    fn tojson_github_returns_object() {
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        env.insert("GITHUB_REF".to_string(), "refs/heads/main".to_string());
        env.insert("GITHUB_REPOSITORY".to_string(), "owner/repo".to_string());
        // Unrelated vars should NOT appear in github object
        env.insert("MY_VAR".to_string(), "hello".to_string());
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());
        env.insert("INPUT_NAME".to_string(), "test".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("sha").unwrap(), "abc123");
        assert_eq!(obj.get("ref").unwrap(), "refs/heads/main");
        assert_eq!(obj.get("repository").unwrap(), "owner/repo");
        assert!(
            obj.get("MY_VAR").is_none(),
            "should exclude non-GITHUB vars"
        );
        assert!(
            obj.get("RUNNER_OS").is_none(),
            "should exclude RUNNER_ vars"
        );
        assert!(
            obj.get("INPUT_NAME").is_none(),
            "should exclude INPUT_ vars"
        );
    }

    #[test]
    fn tojson_github_empty_context() {
        let env = HashMap::new();
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.is_empty(), "should be empty with no env vars: {}", s);
    }

    #[test]
    fn tojson_github_no_github_prefix() {
        let mut env = HashMap::new();
        env.insert("MY_VAR".to_string(), "hello".to_string());
        env.insert("CI".to_string(), "true".to_string());
        env.insert("RUNNER_OS".to_string(), "Linux".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(
            obj.is_empty(),
            "should be empty when no GITHUB_* vars exist: {}",
            s
        );
    }

    #[test]
    fn tojson_github_sorted_keys() {
        let mut env = HashMap::new();
        env.insert("GITHUB_ZEBRA".to_string(), "z".to_string());
        env.insert("GITHUB_APPLE".to_string(), "a".to_string());
        env.insert("GITHUB_MANGO".to_string(), "m".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        // Keys appear stripped + lowercased, in alphabetical order
        let apple_pos = s.find("apple").unwrap();
        let mango_pos = s.find("mango").unwrap();
        let zebra_pos = s.find("zebra").unwrap();
        assert!(apple_pos < mango_pos, "apple should come before mango");
        assert!(mango_pos < zebra_pos, "mango should come before zebra");
    }

    #[test]
    fn bare_github_is_truthy() {
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        // `github` alone should be truthy (it's an object)
        let result = evaluate("github", &ctx).unwrap();
        assert!(result.is_truthy());
    }

    #[test]
    fn tojson_github_preserves_dotted_access() {
        // Regression guard: adding the bare-github arm must not shadow the
        // existing dotted-access arm (`github.sha` → GITHUB_SHA).
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);

        // Dotted access still works.
        let dotted = evaluate("github.sha", &ctx).unwrap();
        assert_eq!(dotted.to_output_string(), "abc123");

        // Bare access returns the full object.
        let bare = evaluate("toJSON(github)", &ctx).unwrap();
        let s = bare.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        assert_eq!(parsed.get("sha").unwrap(), "abc123");
    }

    #[test]
    fn tojson_github_special_characters_in_values() {
        let mut env = HashMap::new();
        env.insert(
            "GITHUB_EVENT_HEAD_COMMIT_MESSAGE".to_string(),
            "he said \"hi\"\nnew line".to_string(),
        );
        env.insert("GITHUB_WORKSPACE".to_string(), "C:\\Users\\dev".to_string());
        env.insert("GITHUB_ACTOR".to_string(), "\u{1F600}".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("should be valid JSON despite special chars");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(
            obj.get("event_head_commit_message").unwrap(),
            "he said \"hi\"\nnew line"
        );
        assert_eq!(obj.get("workspace").unwrap(), "C:\\Users\\dev");
        assert_eq!(obj.get("actor").unwrap(), "\u{1F600}");
    }

    #[test]
    fn fromjson_tojson_github_produces_parseable_json() {
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        env.insert("GITHUB_REF".to_string(), "refs/heads/main".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("fromJSON(toJSON(github))", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("sha").unwrap(), "abc123");
        assert_eq!(obj.get("ref").unwrap(), "refs/heads/main");
    }

    #[test]
    fn tojson_github_includes_token_in_plaintext() {
        // Documents current behavior: GITHUB_TOKEN (when present) surfaces as
        // `github.token`. No masking layer exists yet — callers must not dump
        // this object to untrusted sinks. Pin the behavior so any future change
        // (exclude, redact, route through a masker) is a deliberate decision.
        let mut env = HashMap::new();
        env.insert("GITHUB_TOKEN".to_string(), "ghs_secret".to_string());
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("token").unwrap(), "ghs_secret");
        assert_eq!(obj.get("sha").unwrap(), "abc123");
    }

    #[test]
    fn tojson_github_ignores_prefix_only_key() {
        // The bare prefix `GITHUB_` (no suffix) would strip to an empty string
        // and emit `{"": "..."}` — nonsense output. It should be filtered out.
        let mut env = HashMap::new();
        env.insert("GITHUB_".to_string(), "weird".to_string());
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.get("").is_none(), "empty-key entry must not appear");
        assert_eq!(obj.get("sha").unwrap(), "abc123");
        assert_eq!(obj.len(), 1);
    }

    #[test]
    fn tojson_github_excludes_runner_internal_keys() {
        // environment.rs seeds two classes of runner-internal GITHUB_* vars that
        // aren't part of real GHA's `github` context:
        //   - workflow-command-protocol tempfile paths (GITHUB_OUTPUT / GITHUB_ENV
        //     / GITHUB_PATH / GITHUB_STEP_SUMMARY) — dropping these also avoids
        //     leaking local tempfile paths.
        //   - CI-detection (GITHUB_ACTIONS) — documented as default runner env,
        //     not as a `github.*` context property.
        // `toJSON(github)` must drop all of them.
        let mut env = HashMap::new();
        env.insert("GITHUB_OUTPUT".to_string(), "/tmp/out".to_string());
        env.insert("GITHUB_ENV".to_string(), "/tmp/env".to_string());
        env.insert("GITHUB_PATH".to_string(), "/tmp/path".to_string());
        env.insert(
            "GITHUB_STEP_SUMMARY".to_string(),
            "/tmp/summary".to_string(),
        );
        env.insert("GITHUB_ACTIONS".to_string(), "true".to_string());
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.get("output").is_none(), "should exclude GITHUB_OUTPUT");
        assert!(obj.get("env").is_none(), "should exclude GITHUB_ENV");
        assert!(obj.get("path").is_none(), "should exclude GITHUB_PATH");
        assert!(
            obj.get("step_summary").is_none(),
            "should exclude GITHUB_STEP_SUMMARY"
        );
        assert!(
            obj.get("actions").is_none(),
            "should exclude GITHUB_ACTIONS (not a github-context property)"
        );
        assert_eq!(obj.get("sha").unwrap(), "abc123");
    }

    #[test]
    fn tojson_github_includes_user_defined_github_prefixed_vars() {
        // Documents the prefix-heuristic's inverse limitation: a user-defined
        // `env: { GITHUB_FOO: bar }` contaminates the github object as
        // `github.foo`. Pin this so any future switch to a curated allowlist
        // is a deliberate change, not a silent behaviour flip.
        let mut env = HashMap::new();
        env.insert("GITHUB_SHA".to_string(), "abc123".to_string());
        env.insert("GITHUB_FOO".to_string(), "bar".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        let result = evaluate("toJSON(github)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("foo").unwrap(), "bar");
        assert_eq!(obj.get("sha").unwrap(), "abc123");
    }

    // -- toJSON(steps) tests --

    /// Helper to build an ExpressionContext with step data.
    fn make_steps_ctx<'a>(
        step_outputs: &'a HashMap<String, HashMap<String, String>>,
        step_statuses: &'a HashMap<String, (String, String)>,
    ) -> ExpressionContext<'a> {
        ExpressionContext {
            env_context: &EMPTY_ENV,
            step_outputs,
            matrix_combination: &EMPTY_MATRIX,
            step_statuses,
            job_status: "success",
            secrets_context: &EMPTY_SECRETS,
            needs_context: &EMPTY_NEEDS,
            needs_results: &EMPTY_NEEDS_RESULTS,
        }
    }

    #[test]
    fn tojson_steps_returns_nested_object() {
        let mut outputs = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("artifact".to_string(), "app.zip".to_string());
        build_out.insert("version".to_string(), "1.2.3".to_string());
        outputs.insert("build".to_string(), build_out);

        let mut test_out = HashMap::new();
        test_out.insert("passed".to_string(), "true".to_string());
        outputs.insert("test".to_string(), test_out);

        let mut statuses = HashMap::new();
        statuses.insert(
            "build".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        statuses.insert(
            "test".to_string(),
            ("failure".to_string(), "failure".to_string()),
        );

        let ctx = make_steps_ctx(&outputs, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");

        // Check build step
        let build = obj.get("build").unwrap().as_object().unwrap();
        assert_eq!(build.get("outcome").unwrap(), "success");
        assert_eq!(build.get("conclusion").unwrap(), "success");
        let build_outputs = build.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(build_outputs.get("artifact").unwrap(), "app.zip");
        assert_eq!(build_outputs.get("version").unwrap(), "1.2.3");

        // Check test step
        let test = obj.get("test").unwrap().as_object().unwrap();
        assert_eq!(test.get("outcome").unwrap(), "failure");
        assert_eq!(test.get("conclusion").unwrap(), "failure");
        let test_outputs = test.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(test_outputs.get("passed").unwrap(), "true");
    }

    #[test]
    fn tojson_steps_empty_context() {
        let outputs = HashMap::new();
        let statuses = HashMap::new();
        let ctx = make_steps_ctx(&outputs, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.is_empty(), "should be empty with no steps: {}", s);
    }

    #[test]
    fn tojson_steps_sorted_keys() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "zebra".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        statuses.insert(
            "alpha".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        statuses.insert(
            "middle".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        let ctx = make_steps_ctx(&EMPTY_STEPS, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let alpha_pos = s.find("alpha").unwrap();
        let middle_pos = s.find("middle").unwrap();
        let zebra_pos = s.find("zebra").unwrap();
        assert!(alpha_pos < middle_pos, "alpha should come before middle");
        assert!(middle_pos < zebra_pos, "middle should come before zebra");
    }

    #[test]
    fn tojson_steps_status_without_outputs() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "checkout".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        let ctx = make_steps_ctx(&EMPTY_STEPS, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let checkout = parsed.get("checkout").unwrap().as_object().unwrap();
        assert_eq!(checkout.get("outcome").unwrap(), "success");
        assert_eq!(checkout.get("conclusion").unwrap(), "success");
        let outputs = checkout.get("outputs").unwrap().as_object().unwrap();
        assert!(outputs.is_empty(), "outputs should be empty: {:?}", outputs);
    }

    #[test]
    fn tojson_steps_outputs_without_status() {
        // Edge case: step has outputs but no recorded status yet.
        let mut outputs = HashMap::new();
        let mut step_out = HashMap::new();
        step_out.insert("result".to_string(), "42".to_string());
        outputs.insert("compute".to_string(), step_out);
        let statuses = HashMap::new();
        let ctx = make_steps_ctx(&outputs, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let compute = parsed.get("compute").unwrap().as_object().unwrap();
        // No outcome/conclusion fields when status is absent
        assert!(compute.get("outcome").is_none(), "should have no outcome");
        assert!(
            compute.get("conclusion").is_none(),
            "should have no conclusion"
        );
        let out = compute.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(out.get("result").unwrap(), "42");
    }

    #[test]
    fn bare_steps_is_truthy() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "build".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        let ctx = make_steps_ctx(&EMPTY_STEPS, &statuses);
        let result = evaluate("steps", &ctx).unwrap();
        assert!(result.is_truthy());
    }

    #[test]
    fn tojson_steps_special_characters_in_outputs() {
        let mut outputs = HashMap::new();
        let mut step_out = HashMap::new();
        step_out.insert("msg".to_string(), "he said \"hi\"".to_string());
        step_out.insert("path".to_string(), "C:\\Users\\dev".to_string());
        step_out.insert("emoji".to_string(), "\u{1F680}".to_string());
        outputs.insert("deploy".to_string(), step_out);
        let mut statuses = HashMap::new();
        statuses.insert(
            "deploy".to_string(),
            ("success".to_string(), "success".to_string()),
        );
        let ctx = make_steps_ctx(&outputs, &statuses);
        let result = evaluate("toJSON(steps)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("should be valid JSON despite special chars");
        let deploy_outputs = parsed
            .get("deploy")
            .unwrap()
            .get("outputs")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(deploy_outputs.get("msg").unwrap(), "he said \"hi\"");
        assert_eq!(deploy_outputs.get("path").unwrap(), "C:\\Users\\dev");
        assert_eq!(deploy_outputs.get("emoji").unwrap(), "\u{1F680}");
    }

    // -- toJSON(needs) tests --

    /// Helper to build an ExpressionContext with needs data.
    fn make_needs_ctx<'a>(
        needs_context: &'a HashMap<String, HashMap<String, String>>,
        needs_results: &'a HashMap<String, String>,
    ) -> ExpressionContext<'a> {
        ExpressionContext {
            env_context: &EMPTY_ENV,
            step_outputs: &EMPTY_STEPS,
            matrix_combination: &EMPTY_MATRIX,
            step_statuses: &EMPTY_STATUSES,
            job_status: "success",
            secrets_context: &EMPTY_SECRETS,
            needs_context,
            needs_results,
        }
    }

    #[test]
    fn tojson_needs_returns_nested_object() {
        let mut needs_ctx = HashMap::new();
        let mut build_out = HashMap::new();
        build_out.insert("artifact".to_string(), "app.zip".to_string());
        build_out.insert("version".to_string(), "1.2.3".to_string());
        needs_ctx.insert("build".to_string(), build_out);

        let mut test_out = HashMap::new();
        test_out.insert("passed".to_string(), "true".to_string());
        needs_ctx.insert("test".to_string(), test_out);

        let mut needs_res = HashMap::new();
        needs_res.insert("build".to_string(), "success".to_string());
        needs_res.insert("test".to_string(), "failure".to_string());

        let ctx = make_needs_ctx(&needs_ctx, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");

        // Check build job
        let build = obj.get("build").unwrap().as_object().unwrap();
        assert_eq!(build.get("result").unwrap(), "success");
        let build_outputs = build.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(build_outputs.get("artifact").unwrap(), "app.zip");
        assert_eq!(build_outputs.get("version").unwrap(), "1.2.3");

        // Check test job
        let test = obj.get("test").unwrap().as_object().unwrap();
        assert_eq!(test.get("result").unwrap(), "failure");
        let test_outputs = test.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(test_outputs.get("passed").unwrap(), "true");
    }

    #[test]
    fn tojson_needs_empty_context() {
        let needs_ctx = HashMap::new();
        let needs_res = HashMap::new();
        let ctx = make_needs_ctx(&needs_ctx, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.is_empty(), "should be empty with no needs: {}", s);
    }

    #[test]
    fn tojson_needs_sorted_keys() {
        let mut needs_res = HashMap::new();
        needs_res.insert("zebra".to_string(), "success".to_string());
        needs_res.insert("alpha".to_string(), "success".to_string());
        needs_res.insert("middle".to_string(), "success".to_string());
        let ctx = make_needs_ctx(&EMPTY_NEEDS, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let alpha_pos = s.find("alpha").unwrap();
        let middle_pos = s.find("middle").unwrap();
        let zebra_pos = s.find("zebra").unwrap();
        assert!(alpha_pos < middle_pos, "alpha should come before middle");
        assert!(middle_pos < zebra_pos, "middle should come before zebra");
    }

    #[test]
    fn tojson_needs_result_without_outputs() {
        let mut needs_res = HashMap::new();
        needs_res.insert("lint".to_string(), "success".to_string());
        let ctx = make_needs_ctx(&EMPTY_NEEDS, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let lint = parsed.get("lint").unwrap().as_object().unwrap();
        assert_eq!(lint.get("result").unwrap(), "success");
        let outputs = lint.get("outputs").unwrap().as_object().unwrap();
        assert!(outputs.is_empty(), "outputs should be empty: {:?}", outputs);
    }

    #[test]
    fn tojson_needs_outputs_without_result() {
        // Edge case: needs entry has outputs but no recorded result yet.
        let mut needs_ctx = HashMap::new();
        let mut job_out = HashMap::new();
        job_out.insert("value".to_string(), "42".to_string());
        needs_ctx.insert("compute".to_string(), job_out);
        let needs_res = HashMap::new();
        let ctx = make_needs_ctx(&needs_ctx, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let compute = parsed.get("compute").unwrap().as_object().unwrap();
        assert!(compute.get("result").is_none(), "should have no result");
        let out = compute.get("outputs").unwrap().as_object().unwrap();
        assert_eq!(out.get("value").unwrap(), "42");
    }

    #[test]
    fn bare_needs_is_truthy() {
        let mut needs_res = HashMap::new();
        needs_res.insert("build".to_string(), "success".to_string());
        let ctx = make_needs_ctx(&EMPTY_NEEDS, &needs_res);
        let result = evaluate("needs", &ctx).unwrap();
        assert!(result.is_truthy());
    }

    #[test]
    fn tojson_needs_special_characters_in_outputs() {
        let mut needs_ctx = HashMap::new();
        let mut job_out = HashMap::new();
        job_out.insert("msg".to_string(), "he said \"hi\"".to_string());
        job_out.insert("path".to_string(), "C:\\Users\\dev".to_string());
        job_out.insert("emoji".to_string(), "\u{1F680}".to_string());
        needs_ctx.insert("deploy".to_string(), job_out);
        let mut needs_res = HashMap::new();
        needs_res.insert("deploy".to_string(), "success".to_string());
        let ctx = make_needs_ctx(&needs_ctx, &needs_res);
        let result = evaluate("toJSON(needs)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("should be valid JSON despite special chars");
        let deploy_outputs = parsed
            .get("deploy")
            .unwrap()
            .get("outputs")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(deploy_outputs.get("msg").unwrap(), "he said \"hi\"");
        assert_eq!(deploy_outputs.get("path").unwrap(), "C:\\Users\\dev");
        assert_eq!(deploy_outputs.get("emoji").unwrap(), "\u{1F680}");
    }

    // -- toJSON(secrets) tests --

    /// Helper to build an ExpressionContext with secrets data.
    fn make_secrets_ctx(secrets: &HashMap<String, String>) -> ExpressionContext<'_> {
        ExpressionContext {
            env_context: &EMPTY_ENV,
            step_outputs: &EMPTY_STEPS,
            matrix_combination: &EMPTY_MATRIX,
            step_statuses: &EMPTY_STATUSES,
            job_status: "success",
            secrets_context: secrets,
            needs_context: &EMPTY_NEEDS,
            needs_results: &EMPTY_NEEDS_RESULTS,
        }
    }

    #[test]
    fn tojson_secrets_returns_object() {
        let mut secrets = HashMap::new();
        secrets.insert("NPM_TOKEN".to_string(), "abc".to_string());
        secrets.insert("DEPLOY_KEY".to_string(), "xyz".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("toJSON(secrets)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("NPM_TOKEN").unwrap(), "abc");
        assert_eq!(obj.get("DEPLOY_KEY").unwrap(), "xyz");
        assert_eq!(obj.len(), 2);
    }

    #[test]
    fn tojson_secrets_empty_context() {
        let secrets = HashMap::new();
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("toJSON(secrets)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert!(obj.is_empty(), "should be empty with no secrets: {}", s);
    }

    #[test]
    fn tojson_secrets_sorted_keys() {
        let mut secrets = HashMap::new();
        secrets.insert("ZEBRA".to_string(), "z".to_string());
        secrets.insert("APPLE".to_string(), "a".to_string());
        secrets.insert("MANGO".to_string(), "m".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("toJSON(secrets)", &ctx).unwrap();
        let s = result.to_output_string();
        let apple_pos = s.find("APPLE").unwrap();
        let mango_pos = s.find("MANGO").unwrap();
        let zebra_pos = s.find("ZEBRA").unwrap();
        assert!(apple_pos < mango_pos, "APPLE should come before MANGO");
        assert!(mango_pos < zebra_pos, "MANGO should come before ZEBRA");
    }

    #[test]
    fn tojson_secrets_preserves_special_characters() {
        let mut secrets = HashMap::new();
        secrets.insert("QUOTE".to_string(), "he said \"hi\"".to_string());
        secrets.insert("BACKSLASH".to_string(), "path\\to\\key".to_string());
        secrets.insert(
            "NEWLINE".to_string(),
            "-----BEGIN-----\nBODY\n-----END-----".to_string(),
        );
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("toJSON(secrets)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value =
            serde_json::from_str(&s).expect("should be valid JSON despite special chars");
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.get("QUOTE").unwrap(), "he said \"hi\"");
        assert_eq!(obj.get("BACKSLASH").unwrap(), "path\\to\\key");
        assert_eq!(
            obj.get("NEWLINE").unwrap(),
            "-----BEGIN-----\nBODY\n-----END-----"
        );
    }

    #[test]
    fn fromjson_tojson_secrets_produces_parseable_json() {
        // `fromJSON` currently returns the raw JSON text as `ExprValue::String`
        // (same pattern as `fromjson_tojson_env_produces_parseable_json`). The
        // round-trip must preserve exact values so pipe-through-an-action use
        // cases work and so any future switch to value-masking here is a
        // deliberate decision.
        let mut secrets = HashMap::new();
        secrets.insert("NPM_TOKEN".to_string(), "npm_ABC123".to_string());
        secrets.insert("DEPLOY_KEY".to_string(), "deploy_XYZ".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("fromJSON(toJSON(secrets))", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().expect("should be a JSON object");
        assert_eq!(obj.get("NPM_TOKEN").unwrap(), "npm_ABC123");
        assert_eq!(obj.get("DEPLOY_KEY").unwrap(), "deploy_XYZ");
    }

    #[test]
    fn bare_secrets_is_truthy() {
        let mut secrets = HashMap::new();
        secrets.insert("NPM_TOKEN".to_string(), "abc".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("secrets", &ctx).unwrap();
        assert!(result.is_truthy());
    }

    #[test]
    fn bare_secrets_does_not_shadow_dotted_access() {
        // Regression guard: the bare-`secrets` arm must not shadow the existing
        // `secrets.NAME` dotted-access arm.
        let mut secrets = HashMap::new();
        secrets.insert("NPM_TOKEN".to_string(), "abc".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("secrets.NPM_TOKEN", &ctx).unwrap();
        assert_eq!(result, ExprValue::String("abc".to_string()));
    }

    #[test]
    fn tojson_secrets_returns_values_in_plaintext() {
        // Documents current behavior: secret values surface in plaintext inside
        // `toJSON(secrets)`. This matches real GHA; masking lives at the log
        // boundary via `wrkflw_secrets::SecretMasker`, not in the evaluator.
        // Pin the behavior so any future change (exclude, redact, route through
        // a masker at this layer) is a deliberate decision.
        let mut secrets = HashMap::new();
        secrets.insert("GITHUB_TOKEN".to_string(), "ghs_supersecret".to_string());
        let ctx = make_secrets_ctx(&secrets);
        let result = evaluate("toJSON(secrets)", &ctx).unwrap();
        let s = result.to_output_string();
        let parsed: serde_json::Value = serde_json::from_str(&s).expect("should be valid JSON");
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.get("GITHUB_TOKEN").unwrap(), "ghs_supersecret");
    }

    #[test]
    fn object_cmp_returns_none() {
        // Object comparisons via <, >, <=, >= should all evaluate to false
        // because expr_cmp returns None for Object values.
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let ctx = make_ctx(&env, &EMPTY_STEPS, &EMPTY_MATRIX);
        // These should all evaluate to false (Object is not orderable)
        let result = evaluate("env < env", &ctx).unwrap();
        assert!(!result.is_truthy(), "env < env should be false");
        let result = evaluate("env > env", &ctx).unwrap();
        assert!(!result.is_truthy(), "env > env should be false");
        let result = evaluate("env <= env", &ctx).unwrap();
        assert!(!result.is_truthy(), "env <= env should be false");
        let result = evaluate("env >= env", &ctx).unwrap();
        assert!(!result.is_truthy(), "env >= env should be false");
    }
}
