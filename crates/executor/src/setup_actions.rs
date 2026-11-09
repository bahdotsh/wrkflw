//! The setup actions wrkflw runs itself, such as `actions/setup-python`:
//! which steps are setup actions, the runtime and version each asks for,
//! including from a `*-version-file`, and the shell that installs it into
//! the job's combined image. A setup action has no step of its own to run:
//! its runtime is built into the image before the job starts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wrkflw_parser::workflow::Step;

use crate::action_resolver::is_git_sha;

/// The cause reported for an action wrkflw has no implementation of.
pub(crate) const NO_IMPLEMENTATION: &str = "no implementation in wrkflw";

/// What the job's image was built with, by setup action language.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BuiltRuntimes {
    /// The `language` of each setup action whose runtime the image holds.
    pub(crate) installed: Vec<String>,
    /// Languages a setup step asked for with a version wrkflw can't
    /// install, such as an unknown alias, so none was built in.
    pub(crate) refused: Vec<String>,
}

impl BuiltRuntimes {
    fn has(&self, language: &str) -> bool {
        self.installed.iter().any(|l| l == language)
    }
}

/// The runtimes a job's setup steps ask for, and the languages whose
/// version couldn't be used.
pub(crate) struct SetupDetection {
    pub(crate) runtimes: Vec<SetupRuntime>,
    /// Languages asked for only with versions that were refused.
    pub(crate) refused: Vec<String>,
}

/// A runtime detected from a setup action step (e.g., `actions/setup-node@v3`).
#[derive(Clone)]
pub(crate) struct SetupRuntime {
    /// Language identifier (e.g., "node", "php", "python")
    pub(crate) language: String,
    /// Sanitized version string (e.g., "20", "8.2")
    pub(crate) version: String,
    /// Shell commands to install this runtime on an Ubuntu base image
    pub(crate) install_script: String,
}

/// Definition of a known setup action for runtime detection.
///
/// Used by both `detect_setup_runtimes` (to build combined images) and
/// `determine_action_image` (to select per-step images), keeping the two
/// in sync automatically.
pub(crate) struct SetupActionDef {
    /// Repository names that map to this runtime (exact match, no @version suffix).
    pub(crate) repos: &'static [&'static str],
    /// The `with:` key that specifies the version.
    pub(crate) with_key: &'static str,
    /// Default version when no `with:` key is provided.
    pub(crate) default_version: &'static str,
    /// Language identifier used in install scripts and image tags.
    pub(crate) language: &'static str,
    /// If true, fall back to the @ref from the `uses:` field when no `with:` key is set.
    /// Used by `dtolnay/rust-toolchain` which encodes the toolchain in the ref.
    pub(crate) version_from_ref: bool,
    /// The `with:` key naming a file in the repository that holds the
    /// version, such as `python-version-file`, read when `with_key` is absent.
    pub(crate) version_file_key: Option<&'static str>,
}

pub(crate) const SETUP_ACTIONS: &[SetupActionDef] = &[
    SetupActionDef {
        repos: &["actions/setup-node"],
        with_key: "node-version",
        // With no version setup-node keeps the runner's own Node.
        default_version: RUNNER_NODE_MAJOR,
        language: "node",
        version_from_ref: false,
        version_file_key: Some("node-version-file"),
    },
    SetupActionDef {
        repos: &["shivammathur/setup-php"],
        with_key: "php",
        default_version: "8.2",
        language: "php",
        version_from_ref: false,
        version_file_key: None,
    },
    SetupActionDef {
        repos: &["actions/setup-python"],
        with_key: "python-version",
        default_version: "3.11",
        language: "python",
        version_from_ref: false,
        version_file_key: Some("python-version-file"),
    },
    SetupActionDef {
        repos: &["actions/setup-go"],
        with_key: "go-version",
        default_version: "1.21",
        language: "go",
        version_from_ref: false,
        version_file_key: Some("go-version-file"),
    },
    SetupActionDef {
        repos: &["actions/setup-java"],
        with_key: "java-version",
        default_version: "17",
        language: "java",
        version_from_ref: false,
        version_file_key: None,
    },
    SetupActionDef {
        repos: &["actions/setup-dotnet"],
        with_key: "dotnet-version",
        default_version: "7.0",
        language: "dotnet",
        version_from_ref: false,
        version_file_key: None,
    },
    SetupActionDef {
        repos: &["actions-rs/toolchain", "dtolnay/rust-toolchain"],
        with_key: "toolchain",
        default_version: "stable",
        language: "rust",
        version_from_ref: true,
        version_file_key: None,
    },
    SetupActionDef {
        repos: &["astral-sh/setup-uv"],
        with_key: "version",
        default_version: "latest",
        language: "uv",
        version_from_ref: false,
        version_file_key: None,
    },
];

/// The Node.js major `actions/setup-node` installs for a semver `range`, as
/// the newest release that satisfies it: the highest major an upper bound
/// allows (a hyphen range's upper end included), the major of a caret,
/// tilde, `x` or bare version, or `current`
/// when the range has no upper bound. Of `||` alternatives the last, which
/// is the newest by convention, counts.
pub(crate) fn node_major_for_range(range: &str) -> Option<String> {
    let is_range_char = |c: char| {
        c.is_ascii_digit()
            || c == ' '
            || matches!(
                c,
                '.' | 'x' | 'X' | '*' | '^' | '~' | '<' | '>' | '=' | '|' | '-' | 'v'
            )
    };
    if !range.chars().all(is_range_char) {
        return None;
    }
    let alternative = range.rsplit("||").next()?.trim();
    let major_of = |text: &str| -> Option<u32> {
        let digits: String = text
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        digits.parse().ok()
    };
    // A hyphen range `18 - 20` includes its upper bound, whatever its minor.
    if let Some((_, upper)) = alternative.split_once(" - ") {
        return major_of(upper).map(|m| m.to_string());
    }
    let mut lower_only = false;
    for comparator in alternative.split_whitespace() {
        if let Some(rest) = comparator.strip_prefix("<=") {
            return major_of(rest).map(|m| m.to_string());
        }
        if let Some(rest) = comparator.strip_prefix('<') {
            // `<21` and `<21.0.0` exclude 21; `<21.2` allows 21.1.
            let major = major_of(rest)?;
            let rest_digits = rest.trim_start_matches('v');
            let exact_major = rest_digits
                .split('.')
                .skip(1)
                .all(|part| part.chars().all(|c| c == '0'));
            return Some(
                if exact_major {
                    major.saturating_sub(1)
                } else {
                    major
                }
                .to_string(),
            );
        }
        if comparator.starts_with('>') {
            lower_only = true;
        }
    }
    if lower_only {
        return Some("current".to_string());
    }
    if alternative == "*" || alternative.is_empty() {
        return Some("current".to_string());
    }
    major_of(alternative).map(|m| m.to_string())
}

/// The version a `*-version-file` input names, read from `file` under `root`.
///
/// `pyproject.toml` gives the `requires-python` specifier, `package.json` the
/// major [`node_major_for_range`] picks for `volta.node` or `engines.node`,
/// `go.mod` its `go` directive, and any other file
/// (`.python-version`, `.nvmrc`, `.node-version`, `.go-version`) its first
/// line that isn't blank or a comment, without a leading `v`. The hosted
/// setup actions read the same files. `None` when the file can't be read or
/// holds no version, or lies outside `root`: the workflow may come from a
/// repository `compare` checked out, and must not read the machine's files.
pub(crate) fn version_from_file(root: &Path, file: &str) -> Option<String> {
    let text = std::fs::read_to_string(contained_path(root, file)?).ok()?;
    let name = Path::new(file)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(file);
    let first_number = |line: &str| -> Option<String> {
        let start = line.find(|c: char| c.is_ascii_digit())?;
        let number: String = line[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        (!number.is_empty()).then_some(number.trim_end_matches('.').to_string())
    };
    let version = match name {
        // The whole specifier: uv resolves `>=3.10` to the newest release
        // that matches, as setup-python does, not to 3.10 itself.
        "pyproject.toml" => requires_python(&text),
        "package.json" => {
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            let range = json
                .get("volta")
                .and_then(|v| v.get("node"))
                .or_else(|| json.get("engines").and_then(|e| e.get("node")))?;
            node_major_for_range(range.as_str()?)
        }
        "go.mod" => text
            .lines()
            .find_map(|l| l.trim().strip_prefix("go "))
            .and_then(first_number),
        _ => text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.trim_start_matches('v').to_string()),
    }?;
    (!version.is_empty()).then_some(version)
}

/// Why a `uses:` step that reached the fallback did not run its action, or
/// `None` when it did its work some other way: a setup action whose runtime
/// was built into the job's image.
///
/// `built` says what the image was built with. A job `container:` is used
/// as is and a setup step inside a composite action isn't seen when the
/// image is built, so neither gets one, and a version wrkflw can't install
/// is left out. Emulation builds no image at all: the host's tools stand in.
pub(crate) fn unrun_action_cause(
    repository: &str,
    built: &BuiltRuntimes,
    container_runtime: bool,
) -> Option<&'static str> {
    let Some(def) = SETUP_ACTIONS.iter().find(|d| d.repos.contains(&repository)) else {
        return Some(NO_IMPLEMENTATION);
    };
    if !container_runtime {
        return Some("emulation installs no runtime, so the host's tools stand in for it");
    }
    if built.has(def.language) {
        return None;
    }
    if built.refused.iter().any(|l| l == def.language) {
        return Some(
            "the version it asks for isn't one wrkflw can install, so its runtime \
             was left out of the job's image",
        );
    }
    Some(
        "its runtime is not in the job's image, as a job container is used as is \
         and a setup step inside a composite action isn't built into it",
    )
}

/// `file` under `root`, if it stays there once links are followed.
fn contained_path(root: &Path, file: &str) -> Option<PathBuf> {
    let relative = Path::new(file);
    if relative.is_absolute()
        || relative.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        wrkflw_logging::warning(&format!(
            "Not reading version file {:?}: it must be a path inside the repository",
            file
        ));
        return None;
    }
    let root = root.canonicalize().ok()?;
    let path = root.join(relative).canonicalize().ok()?;
    if !path.starts_with(&root) {
        wrkflw_logging::warning(&format!(
            "Not reading version file {:?}: it links outside the repository",
            file
        ));
        return None;
    }
    Some(path)
}

/// The `requires-python` of `pyproject.toml`'s `[project]` table, without
/// quotes, spaces or a trailing comment.
pub(crate) fn requires_python(text: &str) -> Option<String> {
    let mut in_project = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_project = line == "[project]";
            continue;
        }
        let Some(rest) = line.strip_prefix("requires-python") else {
            continue;
        };
        if !in_project {
            continue;
        }
        // A longer key, such as `requires-python-extra`, isn't this one.
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let inner = &value[1..];
        let spec: String = inner[..inner.find(quote)?]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        return (!spec.is_empty()).then_some(spec);
    }
    None
}

/// Check that a version string contains only safe characters (alphanumeric,
/// dots, hyphens, underscores), and can't read as a command-line option or
/// a parent directory where it lands in a command or a URL.
pub(crate) fn is_safe_version(version: &str) -> bool {
    !version.is_empty()
        && !version.starts_with('-')
        && !version.contains("..")
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

/// Whether `spec` is a PEP 440 version specifier such as `>=3.10,<4` or
/// `~=3.11`, made only of characters that are safe inside single quotes.
pub(crate) fn is_safe_python_spec(spec: &str) -> bool {
    spec.starts_with(['<', '>', '=', '!', '~'])
        && spec.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '.' | '<' | '>' | '=' | '!' | '~' | ',' | '*')
        })
}

/// Scan job steps for known setup actions and return the runtimes they configure.
///
/// If the same language appears multiple times, only the last occurrence is kept
/// (matching GitHub Actions behavior where later setup steps override earlier ones).
#[cfg(test)]
pub(crate) fn detect_setup_runtimes(steps: &[Step]) -> Vec<SetupRuntime> {
    detect_setup(steps).runtimes
}

/// [`detect_setup_runtimes`], with the languages whose version was refused.
pub(crate) fn detect_setup(steps: &[Step]) -> SetupDetection {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    detect_setup_in(steps, &root)
}

/// [`detect_setup_runtimes`] with `*-version-file` inputs read under `root`.
#[cfg(test)]
pub(crate) fn detect_setup_runtimes_in(steps: &[Step], root: &Path) -> Vec<SetupRuntime> {
    detect_setup_in(steps, root).runtimes
}

/// [`detect_setup`] with `*-version-file` inputs read under `root`.
pub(crate) fn detect_setup_in(steps: &[Step], root: &Path) -> SetupDetection {
    let mut runtimes: Vec<SetupRuntime> = Vec::new();
    let mut refused: Vec<String> = Vec::new();

    for step in steps {
        let uses = match &step.uses {
            Some(u) => u,
            None => continue,
        };

        // Split "actions/setup-node@v3" into ("actions/setup-node", Some("v3"))
        let (repo, git_ref) = match uses.split_once('@') {
            Some((r, v)) => (r, Some(v)),
            None => (uses.as_str(), None),
        };

        let def = match SETUP_ACTIONS.iter().find(|d| d.repos.contains(&repo)) {
            Some(d) => d,
            None => continue,
        };

        let with = step.with.as_ref();
        // An empty input, as an unset workflow input leaves it, is no input:
        // the hosted actions then read the version file.
        let input = |key: &str| {
            with.and_then(|w| w.get(key))
                .filter(|value| !value.trim().is_empty())
        };
        let ver = input(def.with_key)
            .cloned()
            .or_else(|| {
                let file = input(def.version_file_key?)?;
                let found = version_from_file(root, file);
                if found.is_none() {
                    wrkflw_logging::warning(&format!(
                        "{}: no {} version found in {}; using {}",
                        uses, def.language, file, def.default_version
                    ));
                }
                found
            })
            .or_else(|| {
                // Some actions encode the version in the @ref (e.g., dtolnay/rust-toolchain@nightly).
                // Skip bare git SHAs — they pin the action version, not the toolchain.
                if def.version_from_ref {
                    git_ref.filter(|r| !is_git_sha(r)).map(|r| r.to_string())
                } else {
                    None
                }
            })
            .or_else(|| {
                // setup-python given no version reads `.python-version`.
                (def.language == "python")
                    .then(|| version_from_file(root, ".python-version"))
                    .flatten()
            })
            .unwrap_or_else(|| def.default_version.to_string());
        let ver = ver.trim().to_string();
        // setup-python takes `>= 3.10` with spaces; the check below doesn't.
        let ver = if def.language == "python" && ver.starts_with(['<', '>', '=', '!', '~']) {
            ver.chars().filter(|c| !c.is_whitespace()).collect()
        } else {
            ver
        };

        // Normalize trailing ".x" suffix (e.g., "16.x" -> "16") so it doesn't
        // leak into install scripts for languages that don't expect it.
        let ver = if ver.ends_with(".x") {
            ver[..ver.len() - 2].to_string()
        } else {
            ver
        };

        // setup-node takes an alias such as `lts/*`, and a semver range,
        // in `node-version` and its version files, and `v20` for 20.
        let ver = if def.language == "node" {
            let ver = match ver.strip_prefix('v') {
                Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
                _ => ver,
            };
            node_alias(&ver)
                .or_else(|| {
                    (!is_safe_version(&ver))
                        .then(|| node_major_for_range(&ver))
                        .flatten()
                })
                .unwrap_or(ver)
        } else {
            ver
        };
        let safe = (is_safe_version(&ver)
            || (def.language == "python" && is_safe_python_spec(&ver)))
            && (def.language != "go" || go_release_known(&ver));
        if !safe {
            wrkflw_logging::warning(&format!(
                "Ignoring {} with invalid version: {:?}",
                def.language, ver
            ));
            if !refused.iter().any(|l| l == def.language) {
                refused.push(def.language.to_string());
            }
            continue;
        }

        let rt = SetupRuntime {
            language: def.language.to_string(),
            version: ver.clone(),
            install_script: get_install_script(def.language, &ver),
        };

        // Deduplicate: later setup steps override earlier ones for the same language
        let existing_idx = runtimes.iter().position(|r| r.language == rt.language);
        if let Some(idx) = existing_idx {
            runtimes[idx] = rt;
        } else {
            runtimes.push(rt);
        }
    }

    // uv goes in before Python, whose install uses the uv already there, so
    // the release setup-uv asks for is the one the job gets.
    if let Some(uv) = runtimes.iter().position(|rt| rt.language == "uv") {
        if let Some(python) = runtimes.iter().position(|rt| rt.language == "python") {
            if python < uv {
                let uv_runtime = runtimes.remove(uv);
                runtimes.insert(python, uv_runtime);
            }
        }
    }

    // Every Rust toolchain step's components and targets go into the image,
    // so each step's own container has them, as GitHub's runner keeps them
    // for the rest of the job. Ones set by an expression aren't known yet.
    if let Some(rust) = runtimes.iter_mut().find(|rt| rt.language == "rust") {
        let repos = SETUP_ACTIONS
            .iter()
            .find(|def| def.language == "rust")
            .map(|def| def.repos)
            .unwrap_or_default();
        let mut extras: Vec<String> = Vec::new();
        for step in steps {
            let Some(uses) = &step.uses else { continue };
            let repo = uses.split_once('@').map_or(uses.as_str(), |(r, _)| r);
            if !repos.contains(&repo) {
                continue;
            }
            if let Some(with) = &step.with {
                for command in rust_toolchain_extras(with, static_input) {
                    if !extras.contains(&command) {
                        extras.push(command);
                    }
                }
            }
        }
        for command in extras {
            rust.install_script.push_str(" && ");
            rust.install_script.push_str(&command);
        }
    }

    // A language a later step set up anyway was installed after all.
    refused.retain(|language| !runtimes.iter().any(|rt| &rt.language == language));
    SetupDetection { runtimes, refused }
}

/// The nodesource channel for a `node-version` alias `actions/setup-node`
/// accepts: `lts/*` and `lts/<codename>` for an LTS line, and `node`,
/// `latest` or `current` for the newest release. `None` for anything else.
pub(crate) fn node_alias(version: &str) -> Option<String> {
    let version = version.trim();
    if matches!(version, "node" | "latest" | "current") {
        return Some("current".to_string());
    }
    let codename = version.strip_prefix("lts/")?;
    // Codenames name one LTS major each; `*` means the newest LTS, which
    // nodesource serves as `lts`. An unknown one is refused, as setup-node
    // refuses it, rather than read as the newest.
    let major = match codename.to_ascii_lowercase().as_str() {
        "argon" => "4",
        "boron" => "6",
        "carbon" => "8",
        "dubnium" => "10",
        "erbium" => "12",
        "fermium" => "14",
        "gallium" => "16",
        "hydrogen" => "18",
        "iron" => "20",
        "jod" => "22",
        "*" | "latest" => "lts",
        _ => return None,
    };
    Some(major.to_string())
}

/// Return shell commands that install a language runtime on an Ubuntu base image.
pub(crate) fn get_install_script(language: &str, version: &str) -> String {
    match language {
        // A full release, as `.nvmrc` often pins one, is that release, as
        // setup-node installs it. nodesource only has each major's newest.
        "node" if is_full_release(version) => {
            format!(
                "ARCH=$(dpkg --print-architecture || echo amd64) && \
                 case \"$ARCH\" in amd64) ARCH=x64 ;; armhf) ARCH=armv7l ;; esac && \
                 curl -fsSL https://nodejs.org/dist/v{v}/node-v{v}-linux-$ARCH.tar.gz | \
                 tar -xz -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE",
                v = version
            )
        }
        "node" => {
            // Strip .x suffix for nodesource URL (e.g., "16.x" -> "16")
            let major = version.split('.').next().unwrap_or(version);
            format!(
                "curl -fsSL https://deb.nodesource.com/setup_{}.x | bash - && apt-get install -y nodejs",
                major
            )
        }
        "php" => {
            format!(
                "apt-get install -y software-properties-common && \
                 add-apt-repository -y ppa:ondrej/php && apt-get update && \
                 apt-get install -y php{ver}-cli php{ver}-mbstring php{ver}-xml php{ver}-curl unzip && \
                 curl -sS https://getcomposer.org/installer | php -- --install-dir=/usr/local/bin --filename=composer",
                ver = version
            )
        }
        // uv installs any CPython, free-threaded or PyPy build the hosted
        // runner offers, from python-build-standalone, without a distro
        // package for it. The interpreter is linked where every image finds
        // it, and `pip` becomes a wrapper so `pip install` works as on the
        // hosted runner. uv marks its interpreters EXTERNALLY-MANAGED, so
        // PEP 668 makes pip refuse to install anything, and PyPy, which
        // ships without pip, can't even bootstrap it. setup-python's
        // interpreters carry no such marker, so it goes.
        "python" => {
            format!(
                "{install_uv} && \
                 uv python install '{req}' && \
                 p=\"$(uv python find '{req}')\" && \
                 ln -sf \"$p\" /usr/local/bin/python && ln -sf \"$p\" /usr/local/bin/python3 && \
                 \"$p\" -c '{drop_marker}' && \
                 (\"$p\" -m pip --version >/dev/null 2>&1 || \"$p\" -m ensurepip --upgrade) && \
                 printf '#!/bin/sh\\nexec /usr/local/bin/python -m pip \"$@\"\\n' > /usr/local/bin/pip && \
                 chmod +x /usr/local/bin/pip && ln -sf /usr/local/bin/pip /usr/local/bin/pip3",
                install_uv = uv_install_unless_present(),
                req = uv_python_request(version),
                drop_marker = DROP_EXTERNALLY_MANAGED,
            )
        }
        "uv" => uv_install_command(version),
        "go" => {
            format!(
                "ARCH=$(dpkg --print-architecture || echo amd64) && \
                 GO={} && test -n \"$GO\" && \
                 curl -fsSL https://go.dev/dl/${{GO}}.linux-${{ARCH}}.tar.gz | tar -C /usr/local -xz && \
                 ln -s /usr/local/go/bin/go /usr/bin/go",
                go_release(version)
            )
        }
        "java" => {
            format!(
                "apt-get install -y wget apt-transport-https gpg && \
                 wget -qO - https://packages.adoptium.net/artifactory/api/gpg/key/public | gpg --dearmor -o /usr/share/keyrings/adoptium.gpg && \
                 echo 'deb [signed-by=/usr/share/keyrings/adoptium.gpg] https://packages.adoptium.net/artifactory/deb $(cat /etc/os-release | grep UBUNTU_CODENAME | cut -d= -f2) main' > /etc/apt/sources.list.d/adoptium.list && \
                 apt-get update && apt-get install -y temurin-{}-jdk",
                version
            )
        }
        "dotnet" => {
            format!(
                "apt-get install -y wget && \
                 wget https://dot.net/v1/dotnet-install.sh -O /tmp/dotnet-install.sh && \
                 chmod +x /tmp/dotnet-install.sh && \
                 /tmp/dotnet-install.sh --channel {} --install-dir /usr/share/dotnet && \
                 ln -s /usr/share/dotnet/dotnet /usr/bin/dotnet",
                version
            )
        }
        // Images such as `rust:1` already have rustup, under a `CARGO_HOME`
        // other than `~/.cargo`, so use the one there and only install it
        // when it's missing. Its directory is linked into /usr/local/bin
        // unless it is already a system directory on PATH: linking all of
        // /usr/bin would be wrong, and linking /usr/local/bin into itself
        // fails and would stop the build.
        "rust" => {
            format!(
                "export PATH=\"${{CARGO_HOME:-$HOME/.cargo}}/bin:$PATH\" && \
                 (command -v rustup >/dev/null 2>&1 || \
                  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain {v}) && \
                 rustup default {v} && \
                 d=\"$(dirname \"$(command -v rustup)\")\" && \
                 case \"$d\" in /usr/local/bin|/usr/bin|/bin) ;; *) ln -sf \"$d\"/* /usr/local/bin/ ;; esac",
                v = version
            )
        }
        _ => String::new(),
    }
}

/// The shell word for the Go release `version` names, as `actions/setup-go`
/// resolves it. A full release such as `1.22.3` is used as is. A line such
/// as `1.22`, which is what a `go.mod` says, or `1` means its newest
/// release, and `stable` the newest of all. go.dev names releases with a
/// patch number, so those are looked up when the image is built.
fn go_release(version: &str) -> String {
    if let Some(pre) = go_prerelease(version) {
        return pre;
    }
    if is_full_release(version) {
        return format!("go{}", version);
    }
    // The two supported lines, newest first, are all go.dev lists without
    // `include=all`: `oldstable` is the second. Each release repeats its
    // version in every file it lists, so repeats are dropped first.
    if version == "oldstable" {
        return r#""$(curl -fsSL 'https://go.dev/dl/?mode=json' | grep -oE '"version": *"go[0-9.]+"' | uniq | sed -n 2p | grep -oE 'go[0-9.]+')""#.to_string();
    }
    // Releases are listed newest first; a line's release candidates, such
    // as `go1.22rc1`, don't match.
    let pattern = if version == "stable" {
        r#""go[0-9]+(\.[0-9]+)*""#.to_string()
    } else {
        format!(r#""go{}(\.[0-9]+)*""#, version.replace('.', r"\."))
    };
    format!(
        r#""$(curl -fsSL 'https://go.dev/dl/?mode=json&include=all' | grep -oE '{}' | head -n 1 | tr -d '"')""#,
        pattern
    )
}

/// Whether `version` is three dot-separated numbers, such as `20.11.0`.
fn is_full_release(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// go.dev's name for a Go pre-release setup-go accepts, such as
/// `1.22rc1`, `1.22.0-rc.1` or `1.23beta2`.
fn go_prerelease(version: &str) -> Option<String> {
    let (line, pre) = version
        .find(['r', 'b', '-'])
        .map(|at| version.split_at(at))?;
    let line = line
        .strip_suffix(".0")
        .filter(|l| l.contains('.'))
        .unwrap_or(line);
    let pre = pre.trim_start_matches('-');
    let (kind, number) = ["rc", "beta"]
        .into_iter()
        .find_map(|kind| Some((kind, pre.strip_prefix(kind)?.trim_start_matches('.'))))?;
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    (line.split('.').count() == 2 && line.split('.').all(numeric) && numeric(number))
        .then(|| format!("go{}{}{}", line, kind, number))
}

/// Whether [`go_release`] can name the release `version` asks for. Anything
/// else is refused before the image build, where it would fail the job.
fn go_release_known(version: &str) -> bool {
    matches!(version, "stable" | "oldstable")
        || go_prerelease(version).is_some()
        || (!version.is_empty()
            && version.split('.').count() <= 3
            && version
                .split('.')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
}

/// Where the image keeps the interpreters uv installs for setup-python:
/// outside any home directory, so every user and every `HOME` finds them.
pub(crate) const UV_PYTHON_DIR: &str = "/opt/uv/python";

/// Python run by the install script: deletes the PEP 668 marker of the
/// interpreter running it. Single-quoted in the shell, so it holds none.
pub(crate) const DROP_EXTERNALLY_MANAGED: &str = "import os, sysconfig; \
     f = os.path.join(sysconfig.get_path(\"stdlib\"), \"EXTERNALLY-MANAGED\"); \
     os.path.exists(f) and os.remove(f)";

/// The command that installs uv `version` (`latest` or a release such as
/// `0.9.30`) into `/usr/local/bin`, replacing one already there.
pub(crate) fn uv_install_command(version: &str) -> String {
    let url = if version == "latest" {
        "https://astral.sh/uv/install.sh".to_string()
    } else {
        format!("https://astral.sh/uv/{}/install.sh", version)
    };
    format!(
        "(curl -LsSf {} | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh)",
        url
    )
}

/// [`uv_install_command`] for `latest`, skipped when a uv is already on
/// `PATH`, such as one `astral-sh/setup-uv` installed at its own version.
pub(crate) fn uv_install_unless_present() -> String {
    format!(
        "(command -v uv >/dev/null 2>&1 || {})",
        uv_install_command("latest")
    )
}

/// `version`, as `actions/setup-python` takes it, in the form `uv python
/// install` takes: `pypy-3.11` becomes `pypy@3.11`, `3.14-dev` `3.14`,
/// `3.x` `3`, `3.14.0-alpha.1` `3.14.0a1`; `3.13t` and `3.12.4` stay as
/// they are.
pub(crate) fn uv_python_request(version: &str) -> String {
    let version = version.trim_end_matches(".x").trim_end_matches("-dev");
    // setup-python's semver pre-releases, in PEP 440's spelling.
    if let Some((release, pre)) = version.split_once('-') {
        for (word, short) in [("alpha", "a"), ("beta", "b"), ("rc", "rc")] {
            if let Some(number) = pre.strip_prefix(word).map(|n| n.trim_start_matches('.')) {
                if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) {
                    return format!("{}{}{}", release, short, number);
                }
            }
        }
    }
    for implementation in ["pypy", "graalpy"] {
        if let Some(rest) = version.strip_prefix(implementation) {
            let rest = rest.trim_start_matches('-');
            // `pypy-3.11-v7.3.17` names the PyPy release too; uv picks it.
            let rest = rest.split('-').next().unwrap_or(rest);
            return format!("{}@{}", implementation, rest);
        }
    }
    version.to_string()
}

/// The Node.js an Ubuntu hosted runner has before any setup step: the
/// current LTS major, unless the base image already has one.
pub(crate) const RUNNER_NODE_MAJOR: &str = "22";

pub(crate) fn runner_node_install() -> String {
    format!(
        "(command -v node >/dev/null 2>&1 || ({}))",
        get_install_script("node", RUNNER_NODE_MAJOR)
    )
}

/// The `rustup` commands that add what a Rust toolchain step's `components`
/// and `targets` (or `target`) inputs ask for, empty if it asks for none.
/// Names are split on commas and whitespace, as `dtolnay/rust-toolchain`
/// does. `resolve` gives an input's final value, or `None` when it can't be
/// known yet, which leaves that input out.
pub(crate) fn rust_toolchain_extras(
    with: &HashMap<String, String>,
    resolve: impl Fn(&str) -> Option<String>,
) -> Vec<String> {
    let names = |keys: &[&str]| -> Vec<String> {
        keys.iter()
            .filter_map(|key| with.get(*key))
            .filter_map(|value| resolve(value))
            .flat_map(|value| {
                value
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            // Names reach a shell, so anything but a plain rustup name is
            // refused, as is one rustup would read as an option.
            .filter(|name| {
                !name.starts_with('-')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            })
            .collect()
    };
    let components = names(&["components"]);
    let targets = names(&["targets", "target"]);
    let mut commands = Vec::new();
    if !components.is_empty() {
        commands.push(format!("rustup component add {}", components.join(" ")));
    }
    if !targets.is_empty() {
        commands.push(format!("rustup target add {}", targets.join(" ")));
    }
    commands
}

/// An input's value when it holds no expression, so it is known before the
/// job runs and can go into the runner image.
pub(crate) fn static_input(value: &str) -> Option<String> {
    (!value.contains("${{")).then(|| value.to_string())
}
