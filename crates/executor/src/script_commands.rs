//! Which commands a `run:` script runs, as far as its text says.
//!
//! A job gets the runner's Rust when its steps run `cargo`, and a Docker
//! engine (or the warning that it has none) when they run `docker`. Both are
//! decided before any step runs, from the text alone. The script is split
//! as a shell splits it, over its whole text, so a quoted string or a
//! heredoc that spans lines stays one word, and only the word a shell would
//! run counts: `sudo -u root docker ps` runs `docker`, `echo docker` and
//! `command -v docker` don't.
//!
//! What a shell would only find out by running the script isn't seen: a
//! command run by `make`, by a script the step calls, through an alias or a
//! variable, as in `$DOCKER ps`. Such a job gets neither the tool nor the
//! warning that it is missing.

use wrkflw_parser::workflow::Step;

/// Whether a `run:` step of `steps` runs one of `commands`.
pub(crate) fn steps_run(steps: &[Step], commands: &[&str]) -> bool {
    steps
        .iter()
        .filter_map(|step| step.run.as_deref())
        .any(|script| script_runs(script, commands))
}

/// Whether `script` runs one of `commands`, by name or by path:
/// `FOO=1 cargo test`, `timeout 60 docker ps`, `$(rustc -V)`,
/// `bash -c "docker ps"`, or a line of a heredoc a shell reads.
fn script_runs(script: &str, commands: &[&str]) -> bool {
    simple_commands(script).iter().any(|command| {
        let words = words(command);
        let Some(at) = command_word(&words) else {
            return false;
        };
        let name = basename(&words[at]);
        if commands.contains(&name) {
            return true;
        }
        SHELLS.contains(&name)
            && shell_script(&words[at + 1..]).is_some_and(|inner| script_runs(inner, commands))
    })
}

/// Shells whose `-c` argument is a script of its own.
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "ksh"];

/// Shell words that come before a command in a compound one.
const SHELL_KEYWORDS: &[&str] = &[
    "then", "do", "else", "elif", "if", "while", "until", "!", "time",
];

/// A command that runs the command after it: its options that take a value
/// as the next word, and how many words it takes before that command.
struct Wrapper {
    name: &'static str,
    options_with_value: &'static [&'static str],
    operands: usize,
    /// Options with which it only looks the command up, as `command -v`.
    options_that_only_look: &'static [&'static str],
}

const WRAPPERS: &[Wrapper] = &[
    Wrapper {
        name: "sudo",
        options_with_value: &[
            "-u", "-g", "-C", "-D", "-h", "-p", "-r", "-t", "-U", "-R", "-T",
        ],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "env",
        options_with_value: &["-u", "-C", "-S"],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "timeout",
        options_with_value: &["-s", "-k"],
        // The duration.
        operands: 1,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "nice",
        options_with_value: &["-n"],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "stdbuf",
        options_with_value: &["-i", "-o", "-e"],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "xargs",
        options_with_value: &["-a", "-d", "-E", "-I", "-L", "-n", "-P", "-s"],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "exec",
        options_with_value: &["-a"],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "nohup",
        options_with_value: &[],
        operands: 0,
        options_that_only_look: &[],
    },
    Wrapper {
        name: "command",
        options_with_value: &[],
        operands: 0,
        options_that_only_look: &["-v", "-V"],
    },
];

/// `NAME=value`, as a shell takes ahead of a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// `path` without its directories, as `/usr/bin/docker` runs `docker`.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A heredoc whose body is still to come, on the lines after its `<<`.
struct Heredoc {
    delimiter: String,
    /// Its command is a shell, which runs the body as a script.
    is_script: bool,
}

/// What ended a piece of a script, which tells a `case` pattern from a
/// command.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ending {
    /// `)`, which ends a `case` pattern as well as a subshell.
    Close,
    /// `;;`, `;&` or `;;&`, which end a `case` item.
    CaseItem,
    /// Anything else.
    Other,
}

/// A piece of a script, split as [`simple_commands`] describes.
enum Piece {
    /// Text up to a split, and what ended it.
    Text(String, Ending),
    /// A command of a heredoc a shell reads, already split on its own.
    Inner(String),
}

/// The simple commands of `script`, as a shell would split them: at new
/// lines, `;`, `|`, `&`, parentheses, braces and backquotes outside quotes,
/// and at the `$(` and backquotes of a command substitution inside double
/// quotes. Single-quoted text is never split, a backslash escapes the
/// character after it and joins a line to the next, and a comment runs
/// from a `#` that starts a word to the end of its line. Arithmetic, as in
/// `$((1 << 4))`, is one word. A heredoc's body is left out, unless a shell
/// reads it, whose commands it then holds, and so are the patterns of a
/// `case`.
fn simple_commands(script: &str) -> Vec<String> {
    without_case_patterns(pieces(script))
}

/// Where a `case` is, in the pieces that follow it.
#[derive(Clone, Copy, PartialEq)]
enum Case {
    /// Before the `)` that ends an item's pattern.
    Pattern,
    /// In an item's commands, up to its `;;`.
    Commands,
}

/// The commands among `pieces`, without the `case`, `in` and patterns of a
/// `case`, which a shell matches and doesn't run: `docker)` in
/// `case "$x" in docker) ...` names no command.
fn without_case_patterns(pieces: Vec<Piece>) -> Vec<String> {
    let mut cases: Vec<Case> = Vec::new();
    let mut commands = Vec::new();
    for piece in pieces {
        let (text, ending) = match piece {
            Piece::Inner(text) => {
                commands.push(text);
                continue;
            }
            Piece::Text(text, ending) => (text, ending),
        };
        let words = words(&text);
        let first = words.first().map(String::as_str);
        if first == Some("esac") && !cases.is_empty() {
            cases.pop();
            continue;
        }
        if cases.last() == Some(&Case::Pattern) {
            if ending == Ending::Close {
                *cases.last_mut().expect("in a case") = Case::Commands;
            }
            continue;
        }
        if first == Some("case") {
            // `case WORD in`, and on the same line perhaps the first
            // pattern, up to its `)`.
            let at_pattern = ending == Ending::Close && words.len() > 3;
            cases.push(if at_pattern {
                Case::Commands
            } else {
                Case::Pattern
            });
            continue;
        }
        commands.push(text);
        if ending == Ending::CaseItem {
            if let Some(case) = cases.last_mut() {
                *case = Case::Pattern;
            }
        }
    }
    commands
}

/// `script` split into the pieces [`simple_commands`] reads.
fn pieces(script: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let (mut single, mut double) = (false, false);
    let mut word_start = true;
    // Inside `${...}`, whose braces and `#` aren't the shell's own.
    let mut parameter_depth = 0usize;
    // Inside `$((...))` or `((...))`: the parentheses still open.
    let mut arithmetic_depth = 0usize;
    let mut heredocs: Vec<Heredoc> = Vec::new();
    let mut chars = script.chars().peekable();
    let split = |pieces: &mut Vec<Piece>, current: &mut String, ending: Ending| {
        pieces.push(Piece::Text(std::mem::take(current), ending));
    };
    let opens_arithmetic = |chars: &std::iter::Peekable<std::str::Chars<'_>>| {
        let mut ahead = chars.clone();
        ahead.next() == Some('(') && ahead.next() == Some('(')
    };
    while let Some(c) = chars.next() {
        if single {
            single = c != '\'';
            current.push(c);
            continue;
        }
        if arithmetic_depth > 0 {
            match c {
                '(' => arithmetic_depth += 1,
                ')' => arithmetic_depth -= 1,
                _ => {}
            }
            current.push(c);
            word_start = false;
            continue;
        }
        if c == '\\' {
            match chars.next() {
                // A continued line is one line.
                Some('\n') => {}
                Some(next) => {
                    current.push(c);
                    current.push(next);
                }
                None => current.push(c),
            }
            word_start = false;
            continue;
        }
        if c == '$' && opens_arithmetic(&chars) {
            chars.next();
            chars.next();
            arithmetic_depth = 2;
            current.push_str("$((");
            word_start = false;
            continue;
        }
        if double {
            let splits = match c {
                '"' => {
                    double = false;
                    false
                }
                '`' => true,
                '$' if chars.peek() == Some(&'(') => {
                    chars.next();
                    true
                }
                _ => false,
            };
            if splits {
                split(&mut pieces, &mut current, Ending::Other);
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' => {
                single = true;
                current.push(c);
            }
            '"' => {
                double = true;
                current.push(c);
            }
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                parameter_depth += 1;
                current.push_str("${");
                word_start = false;
                continue;
            }
            '}' if parameter_depth > 0 => {
                parameter_depth -= 1;
                current.push(c);
            }
            '#' if word_start && parameter_depth == 0 => {
                while chars.peek().is_some_and(|&next| next != '\n') {
                    chars.next();
                }
            }
            // An arithmetic command, `(( i << 2 ))`, which runs nothing.
            '(' if word_start && chars.peek() == Some(&'(') => {
                chars.next();
                arithmetic_depth = 2;
                current.push_str("((");
                word_start = false;
                continue;
            }
            '<' if chars.peek() == Some(&'<') => {
                chars.next();
                if chars.peek() == Some(&'<') {
                    // A here-string: its word is an argument like any other.
                    chars.next();
                    current.push_str("<<<");
                } else {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                    }
                    let delimiter = heredoc_delimiter(&mut chars);
                    if !delimiter.is_empty() {
                        let before = words(&current);
                        let is_script = command_word(&before)
                            .is_some_and(|at| SHELLS.contains(&basename(&before[at])));
                        heredocs.push(Heredoc {
                            delimiter,
                            is_script,
                        });
                    }
                    current.push(' ');
                }
            }
            '\n' => {
                split(&mut pieces, &mut current, Ending::Other);
                for heredoc in std::mem::take(&mut heredocs) {
                    let body = heredoc_body(&mut chars, &heredoc.delimiter);
                    if heredoc.is_script {
                        pieces.extend(simple_commands(&body).into_iter().map(Piece::Inner));
                    }
                }
            }
            ';' => {
                // `;;`, `;&` and `;;&` end a `case` item.
                let mut ending = Ending::Other;
                if chars.peek() == Some(&';') {
                    chars.next();
                    ending = Ending::CaseItem;
                }
                if chars.peek() == Some(&'&') {
                    chars.next();
                    ending = Ending::CaseItem;
                }
                split(&mut pieces, &mut current, ending);
            }
            ')' => split(&mut pieces, &mut current, Ending::Close),
            '|' | '&' | '(' | '{' | '}' | '`' => {
                split(&mut pieces, &mut current, Ending::Other);
            }
            _ => current.push(c),
        }
        word_start = c.is_whitespace() || ";|&(){}`".contains(c);
    }
    split(&mut pieces, &mut current, Ending::Other);
    pieces
}

/// The word after a `<<`, without the quotes that keep its body from being
/// expanded.
fn heredoc_delimiter(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    while chars.peek().is_some_and(|&c| c == ' ' || c == '\t') {
        chars.next();
    }
    let mut delimiter = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() || ";|&()<>".contains(c) {
            break;
        }
        chars.next();
        if !matches!(c, '\'' | '"' | '\\') {
            delimiter.push(c);
        }
    }
    delimiter
}

/// The lines of a heredoc's body, up to and without the line that closes
/// it with `delimiter`, leading tabs allowed as `<<-` allows them.
fn heredoc_body(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, delimiter: &str) -> String {
    let mut body = String::new();
    loop {
        let mut line = String::new();
        let mut ended = false;
        for c in chars.by_ref() {
            if c == '\n' {
                ended = true;
                break;
            }
            line.push(c);
        }
        if line.trim_start_matches('\t').trim_end() == delimiter || (!ended && line.is_empty()) {
            return body;
        }
        body.push_str(&line);
        body.push('\n');
        if !ended {
            return body;
        }
    }
}

/// The words of a simple command, as a shell would pass them: split at
/// spaces outside quotes, and without the quotes.
fn words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let (mut single, mut double) = (false, false);
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !double => {
                single = !single;
                in_word = true;
            }
            '"' if !single => {
                double = !double;
                in_word = true;
            }
            '\\' if !single => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                in_word = true;
            }
            _ if c.is_whitespace() && !single && !double => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            _ => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    words
}

/// Which of `words` names what the command runs, past assignments,
/// keywords, options and wrappers such as `sudo -u root` or `timeout 60`.
/// `None` when it runs nothing, or only looks a command up, as
/// `command -v docker` does.
fn command_word(words: &[String]) -> Option<usize> {
    let mut wrapper: Option<&Wrapper> = None;
    let mut operands = 0;
    let mut at = 0;
    while at < words.len() {
        let word = words[at].as_str();
        at += 1;
        if word.is_empty() || is_assignment(word) || SHELL_KEYWORDS.contains(&word) {
            continue;
        }
        if word.starts_with('-') {
            if let Some(wrapper) = wrapper {
                if wrapper.options_that_only_look.contains(&word) {
                    return None;
                }
                if wrapper.options_with_value.contains(&word) {
                    at += 1;
                }
            }
            continue;
        }
        if let Some(found) = WRAPPERS.iter().find(|w| w.name == word) {
            wrapper = Some(found);
            operands = found.operands;
            continue;
        }
        if operands > 0 {
            operands -= 1;
            continue;
        }
        return Some(at - 1);
    }
    None
}

/// The script a shell is given with `-c`, from the words after its name:
/// the first operand after an option that holds `c`, as in `-c`, `-ec` or
/// `-lc`.
fn shell_script(args: &[String]) -> Option<&str> {
    let mut seen_c = false;
    for arg in args {
        if arg.starts_with('-') && !arg.starts_with("--") {
            seen_c |= arg.contains('c');
        } else if arg.starts_with("--") {
            continue;
        } else {
            return seen_c.then_some(arg.as_str());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_found_where_a_shell_would_run_it() {
        for script in [
            "docker run alpine true",
            "  sudo -E docker build .",
            "FOO=1 BAR=2 docker compose up -d",
            "echo start && docker ps",
            "id=$(docker create alpine)",
            "if ! docker info; then exit 1; fi",
            "/usr/bin/docker version",
            "make build\ndocker push example/image",
            "sudo -u root docker ps",
            "sudo -E -u root docker ps",
            "timeout 60 docker run alpine true",
            "timeout -s KILL 5m docker ps",
            "nice -n 10 docker build .",
            "env -u HOME docker info",
            "echo \"$(docker version)\"",
            "echo \"id: `docker create alpine`\"",
            "\"docker\" ps",
            "command docker ps",
            "bash -c \"docker ps\"",
            "sh -ec 'docker info'",
            "sudo bash -lc 'echo hi; docker ps'",
            "echo a \\\n  && docker ps",
            "bash <<EOF\ndocker ps\nEOF",
            "cat <<'EOF' > notes.txt\ncargo build\nEOF\ndocker ps",
            "echo \"one\ntwo\"\ndocker ps",
            "echo ok # done\ndocker ps",
            "n=${#items}; docker ps",
            "echo \"${HOME}\" && docker ps",
            "case \"$1\" in\n  build) docker build . ;;\n  *) echo no ;;\nesac",
            "case $x in a|b) echo a ;; push) docker push x ;; esac",
            "case $x in (a) echo a ;;\nesac\ndocker ps",
            "case $x in\n  a)\n    case $y in b) echo b ;; esac\n    docker ps\n    ;;\nesac",
            "echo $((1 << 4))\ndocker ps",
            "echo \"$(( (1 + 2) << 4 ))\"; docker ps",
            "(( n <<= 1 )) && docker ps",
            "(docker ps)",
            "id=$(docker create alpine) && echo $id",
        ] {
            assert!(script_runs(script, &["docker"]), "{script:?}");
        }
    }

    #[test]
    fn a_command_only_named_is_not_run() {
        for script in [
            "echo docker",
            "apt-get install -y docker",
            "# docker run alpine true",
            "echo ok # docker run alpine true",
            "./docker.sh",
            "docker-compose up",
            "DOCKER=1 make docker",
            "echo \"done; docker next\"",
            "echo 'a | docker b'",
            "echo \"(docker)\"",
            "timeout 60",
            "",
            "command -v docker",
            "if command -v docker >/dev/null; then echo yes; fi",
            "command -V docker",
            "cat <<EOF\ndocker ps\nEOF",
            "cat <<-'END'\n\tdocker ps\n\tEND\necho done",
            "echo \"first line\ndocker ps in a string\"",
            "echo 'a\ndocker b'",
            "bash -c 'echo docker'",
            "bash script.sh",
            "grep docker <<< \"$text\"",
            "case \"$1\" in\n  docker) echo yes ;;\nesac",
            "case $1 in docker|podman) echo engine ;; esac",
            "case $1 in (docker) echo yes ;;& *) echo any ;; esac",
            "case $1 in\n  a) echo a\n  ;;\n  docker)\n    echo d\nesac",
            "echo $((1 << 4))\necho docker",
        ] {
            assert!(!script_runs(script, &["docker"]), "{script:?}");
        }
    }

    #[test]
    fn the_lines_after_a_heredoc_are_read_again() {
        let script = "cat > Dockerfile <<EOF\nFROM alpine\nEOF\ncargo build";
        assert!(script_runs(script, &["cargo"]));
        assert!(!script_runs(script, &["FROM"]));
    }

    fn run_step(run: &str) -> Step {
        Step::with_run("step", run)
    }

    #[test]
    fn any_step_that_runs_it_counts() {
        let steps = [run_step("echo hello"), run_step("cargo test")];
        assert!(steps_run(&steps, &["cargo"]));
        assert!(!steps_run(&steps[..1], &["cargo"]));
    }
}
