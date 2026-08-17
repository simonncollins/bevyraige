//! A game-defined options payload, supplied from outside the game.
//!
//! Test and capture runs need to change settings the game does not otherwise
//! expose: mute the music, shrink the window, skip an intro. Which settings
//! those are is entirely game-specific, so **this module does not define a
//! schema** — the host declares a type, this loads it.
//!
//! ```rust
//! # use bevy::prelude::*;
//! # use serde::Deserialize;
//! # use beveraige::devtools::options::DevOptionsPlugin;
//! #[derive(Resource, Deserialize, Default, Clone, Debug)]
//! #[serde(default, deny_unknown_fields)]
//! struct MyOptions {
//!     music_volume: Option<f64>,
//!     window: Option<(f32, f32)>,
//! }
//!
//! # let mut app = App::new();
//! app.add_plugins(DevOptionsPlugin::<MyOptions>::new("MYGAME"));
//! ```
//!
//! ```bash
//! MYGAME_OPTIONS='{"music_volume": 0.0}'   # inline JSON
//! MYGAME_OPTIONS=/tmp/quiet.json           # or a path to a file
//! ```
//!
//! # Settings that must be applied before the app is built
//!
//! Window size is chosen when `WindowPlugin` is configured, which is before any
//! system runs — so a resource inserted by a plugin is already too late. Use
//! [`load_options`] directly in `main` for those, and the plugin for everything
//! a system can apply later. Both read the same variable, so one payload
//! configures both halves.
//!
//! # Why `deny_unknown_fields` is worth it
//!
//! A payload is usually typed by hand under time pressure. Without it,
//! `{"music_vol": 0}` is silently accepted, changes nothing, and the run looks
//! like the setting had no effect. Rejecting the payload names the typo.

use std::marker::PhantomData;

use bevy::prelude::*;
use serde::de::DeserializeOwned;

use super::env;

/// Loads `{PREFIX}_OPTIONS`, which is either inline JSON or a path to a file.
///
/// Returns `None` when unset. A payload that is set but unparseable logs an
/// error and yields `None`: a typo should be loud, but it should not stop the
/// game from starting, because these are dev conveniences rather than
/// requirements.
pub fn load_options<T: DeserializeOwned>(prefix: &str) -> Option<T> {
    let raw = env::var(prefix, "OPTIONS")?;
    let json = resolve_payload(&raw);

    match serde_json::from_str::<T>(&json) {
        Ok(options) => Some(options),
        Err(e) => {
            error!("{prefix}_OPTIONS could not be read: {e}");
            None
        }
    }
}

/// Treats the value as a file path when it does not look like JSON.
///
/// Guessing by first character rather than by trying the filesystem first: a
/// path that does not exist should report *that*, not a JSON parse error about
/// a filename.
pub fn resolve_payload(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        return trimmed.to_string();
    }
    match std::fs::read_to_string(trimmed) {
        Ok(contents) => contents,
        Err(e) => {
            error!("options file '{trimmed}' could not be read: {e}");
            // Fall through to the parser, which will report it as a payload
            // problem rather than silently doing nothing.
            String::new()
        }
    }
}

/// Inserts a game-defined options resource from `{PREFIX}_OPTIONS`.
///
/// Systems that apply the options should tolerate its absence, since an
/// ordinary run has no payload at all.
pub struct DevOptionsPlugin<T> {
    prefix: &'static str,
    _marker: PhantomData<T>,
}

impl<T> DevOptionsPlugin<T> {
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            _marker: PhantomData,
        }
    }
}

impl<T: Resource + DeserializeOwned + Clone + std::fmt::Debug> Plugin for DevOptionsPlugin<T> {
    fn build(&self, app: &mut App) {
        let Some(options) = load_options::<T>(self.prefix) else {
            return;
        };
        info!("Dev options: {options:?}");
        app.insert_resource(options);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Resource, Deserialize, Debug, Default, Clone, PartialEq)]
    #[serde(default, deny_unknown_fields)]
    struct TestOptions {
        music_volume: Option<f64>,
        window: Option<(f32, f32)>,
    }

    #[test]
    fn inline_json_is_used_as_is() {
        let payload = resolve_payload(r#"{"music_volume": 0.0}"#);
        let parsed: TestOptions = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed.music_volume, Some(0.0));
    }

    #[test]
    fn surrounding_whitespace_does_not_make_it_look_like_a_path() {
        let payload = resolve_payload("  {\"music_volume\": 0.5}  ");
        let parsed: TestOptions = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed.music_volume, Some(0.5));
    }

    #[test]
    fn a_path_is_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("options.json");
        std::fs::write(&path, r#"{"window": [640.0, 360.0]}"#).unwrap();

        let payload = resolve_payload(path.to_str().unwrap());
        let parsed: TestOptions = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed.window, Some((640.0, 360.0)));
    }

    #[test]
    fn omitted_settings_stay_unset_rather_than_taking_a_default_value() {
        // The distinction matters: `None` means "leave the game's own setting
        // alone", not "set it to zero".
        let parsed: TestOptions = serde_json::from_str(r#"{"music_volume": 0.0}"#).unwrap();
        assert_eq!(parsed.music_volume, Some(0.0));
        assert_eq!(parsed.window, None, "an absent key must not be assumed");
    }

    #[test]
    fn an_empty_payload_is_valid_and_changes_nothing() {
        let parsed: TestOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, TestOptions::default());
    }

    /// The reason for `deny_unknown_fields`: a typo must not look like a
    /// setting that had no effect.
    #[test]
    fn a_misspelled_key_is_rejected_rather_than_ignored() {
        let result = serde_json::from_str::<TestOptions>(r#"{"music_vol": 0.0}"#);
        assert!(
            result.is_err(),
            "a typo should be reported, not silently accepted"
        );
    }

    #[test]
    fn a_malformed_payload_is_an_error_not_a_panic() {
        assert!(serde_json::from_str::<TestOptions>("{not json").is_err());
    }
}
