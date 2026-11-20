// runtime crate

pub mod container;
pub mod emulation;
pub mod live_output;
pub mod sandbox;
pub mod secure_emulation;

/// Env var naming, comma-separated, variables of this machine's own
/// environment that a step run on this machine (emulation, secure
/// emulation) must not inherit, such as the user's tokens kept from a
/// third-party action. The runtimes that run steps here read it and never
/// pass it on; a container runtime never sees it, as nothing sets it there.
pub const ENV_REMOVE: &str = "WRKFLW_ENV_REMOVE";

/// The names [`ENV_REMOVE`] in `env_vars` lists.
pub fn env_removals<'a>(env_vars: &[(&'a str, &'a str)]) -> Vec<&'a str> {
    env_vars
        .iter()
        .filter(|(key, _)| *key == ENV_REMOVE)
        .flat_map(|(_, names)| names.split(','))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod env_remove_tests {
    use super::*;

    #[test]
    fn removals_are_read_from_the_reserved_variable_only() {
        let env = [
            ("A", "1"),
            (ENV_REMOVE, "GH_TOKEN, AWS_SECRET_ACCESS_KEY,"),
            ("B", "GH_TOKEN"),
        ];
        assert_eq!(env_removals(&env), ["GH_TOKEN", "AWS_SECRET_ACCESS_KEY"]);
        assert!(env_removals(&[("A", "1")]).is_empty());
    }
}
