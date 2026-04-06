//! GitHub Actions expression evaluator.
//!
//! Implements the expression language used inside `${{ }}` blocks in GitHub
//! Actions workflows. Supports context references (`inputs.*`, `env.*`,
//! `github.*`, `runner.*`, `matrix.*`, `steps.*.outputs.*`), operators
//! (`==`, `!=`, `&&`, `||`, `!`, comparisons), string/number/boolean literals,
//! and built-in functions (`contains`, `startsWith`, `endsWith`, `format`,
//! `success`, `failure`, `always`, `cancelled`).

use serde_yaml::Value;
use std::collections::HashMap;

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
}

impl ExprValue {
    /// GitHub Actions truthiness: `false`, `0`, `""`, and `null` are falsy.
    pub fn is_truthy(&self) -> bool {
        match self {
            ExprValue::Bool(b) => *b,
            ExprValue::Number(n) => *n != 0.0 && !n.is_nan(),
            ExprValue::String(s) => !s.is_empty(),
            ExprValue::Null => false,
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
    }
}

fn expr_cmp(a: &ExprValue, b: &ExprValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (ExprValue::Number(a), ExprValue::Number(b)) => a.partial_cmp(b),
        (ExprValue::String(a), ExprValue::String(b)) => {
            Some(a.to_lowercase().cmp(&b.to_lowercase()))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Built-in functions
// ---------------------------------------------------------------------------

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
}
