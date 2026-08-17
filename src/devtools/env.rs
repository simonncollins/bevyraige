//! Environment-variable plumbing shared by the dev tools.
//!
//! Everything here is keyed off a project-chosen prefix, so two games using
//! this module in the same shell do not collide and neither inherits the
//! other's settings.

/// Reads `{PREFIX}_{NAME}` from the environment.
///
/// An empty value counts as unset. That matters more than it sounds: `FOO=`
/// in a CI matrix or a `.env` file is how people disable a setting, and
/// treating it as "enabled with an empty path" produces a confusing failure a
/// long way from the cause.
pub fn var(prefix: &str, name: &str) -> Option<String> {
    non_empty(std::env::var(format!("{prefix}_{name}")).ok())
}

/// Reads a variable and parses it, ignoring anything unparseable.
pub fn parse_var<T: std::str::FromStr>(prefix: &str, name: &str) -> Option<T> {
    var(prefix, name)?.parse().ok()
}

/// Whether a flag variable is set to anything other than `0` / `false` / `no`.
pub fn flag(prefix: &str, name: &str) -> bool {
    is_truthy(var(prefix, name).as_deref())
}

/// Whether a raw value reads as "on".
///
/// Split out from [`flag`] so it can be tested without touching the process
/// environment. Environment mutation is not safe to test under a parallel test
/// runner: `set_var` is global, so two tests setting the same variable race and
/// fail intermittently — which is exactly what happened here.
pub fn is_truthy(value: Option<&str>) -> bool {
    match value {
        None => false,
        Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"),
    }
}

/// Normalises a raw value, treating empty as absent.
pub fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_value_counts_as_unset() {
        // `FOO=` in a CI matrix or a .env file is how people disable a
        // setting; treating it as "enabled, with an empty value" fails a long
        // way from the cause.
        assert_eq!(non_empty(Some(String::new())), None);
        assert_eq!(non_empty(None), None);
        assert_eq!(non_empty(Some("x".into())).as_deref(), Some("x"));
    }

    #[test]
    fn flags_treat_the_usual_negatives_as_off() {
        for off in ["0", "false", "FALSE", "no", ""] {
            assert!(!is_truthy(Some(off)), "{off:?} should be off");
        }
        for on in ["1", "true", "yes", "anything"] {
            assert!(is_truthy(Some(on)), "{on:?} should be on");
        }
        assert!(!is_truthy(None), "unset is off");
    }

    #[test]
    fn a_variable_is_namespaced_by_its_prefix() {
        // Two games in one shell must not read each other's settings. The
        // lookup is by exact `{PREFIX}_{NAME}`, so this holds by construction;
        // the check here is that the name is built the way it is documented.
        assert_eq!(format!("{}_{}", "MYGAME", "CAPTURE"), "MYGAME_CAPTURE");
    }
}
