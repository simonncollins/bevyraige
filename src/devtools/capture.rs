//! Capturing rendered frames from outside the game.
//!
//! Screenshotting a window through the OS is unreliable in exactly the
//! situations you most want it: the window may be on another desktop/Space,
//! behind another app, minimised, or blocked by the host's screen-recording
//! permission. Bevy can hand over the rendered frame directly, which sidesteps
//! all of that and works over SSH and in CI.
//!
//! Game-agnostic: nothing here refers to any project type.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use super::env;

/// Resolved capture settings.
#[derive(Resource, Debug, Clone)]
pub struct CaptureConfig {
    /// File to write. PNG is inferred from the extension by Bevy.
    pub path: String,
    /// Frame number to capture on.
    pub frame: u32,
    /// Whether to quit once the file is written.
    pub exit_after: bool,
}

/// Frame to capture on when unspecified.
///
/// A capture at frame 0 photographs an empty window: assets are still loading,
/// and anything fetched over the network has certainly not arrived. Three
/// seconds at 60fps is enough for most startups and cheap enough to wait for.
pub const DEFAULT_CAPTURE_FRAME: u32 = 180;

/// Frames to wait between requesting the screenshot and quitting.
///
/// The render app writes the file asynchronously; exiting immediately
/// truncates it or loses it entirely.
const EXIT_GRACE_FRAMES: u32 = 30;

impl CaptureConfig {
    /// Reads `{PREFIX}_CAPTURE`, `_CAPTURE_AT` and `_CAPTURE_EXIT`.
    pub fn from_env(prefix: &str) -> Option<Self> {
        Self::from_lookup(|name| env::var(prefix, name))
    }

    /// Builds the config from an arbitrary lookup.
    ///
    /// The environment is process-global, so a test that sets a variable races
    /// every other test in the binary. Taking the lookup as an argument keeps
    /// the parsing rules testable without that hazard.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let path = env::non_empty(get("CAPTURE"))?;
        Some(Self {
            path,
            frame: env::non_empty(get("CAPTURE_AT"))
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_CAPTURE_FRAME),
            exit_after: env::is_truthy(get("CAPTURE_EXIT").as_deref()),
        })
    }
}

/// Frames elapsed, and whether the capture has been requested.
#[derive(Resource, Debug, Default)]
pub struct CaptureState {
    pub frames: u32,
    pub fired: bool,
}

/// System: screenshots once the configured frame is reached, then optionally quits.
pub fn capture_when_ready(
    mut commands: Commands,
    config: Res<CaptureConfig>,
    mut state: ResMut<CaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    state.frames += 1;

    if !state.fired && state.frames >= config.frame {
        state.fired = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(config.path.clone()));
        info!("Capture: writing frame {} to {}", state.frames, config.path);
        return;
    }

    if state.fired && config.exit_after && state.frames >= config.frame + EXIT_GRACE_FRAMES {
        info!("Capture: written, exiting.");
        exit.write(AppExit::Success);
    }
}

/// Adds frame capture when `{PREFIX}_CAPTURE` names a file.
///
/// Inert otherwise, so a normal run pays nothing for this being installed.
pub struct CapturePlugin {
    pub prefix: &'static str,
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        let Some(config) = CaptureConfig::from_env(self.prefix) else {
            return;
        };
        info!(
            "Capture armed: frame {} -> {}{}",
            config.frame,
            config.path,
            if config.exit_after { ", then exit" } else { "" }
        );
        app.insert_resource(config);
        app.init_resource::<CaptureState>();
        app.add_systems(Update, capture_when_ready);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lookup over a fixed table, standing in for the environment.
    fn lookup(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn capture_is_off_unless_a_path_is_given() {
        assert!(CaptureConfig::from_lookup(lookup(&[])).is_none());
        assert!(
            CaptureConfig::from_lookup(lookup(&[("CAPTURE", "")])).is_none(),
            "an empty path is the same as unset"
        );
    }

    #[test]
    fn a_path_alone_is_enough_and_the_rest_defaults() {
        let c = CaptureConfig::from_lookup(lookup(&[("CAPTURE", "/tmp/x.png")])).expect("set");
        assert_eq!(c.path, "/tmp/x.png");
        assert_eq!(c.frame, DEFAULT_CAPTURE_FRAME);
        assert!(!c.exit_after, "a capture run keeps going by default");
    }

    #[test]
    fn the_frame_and_exit_flag_are_read() {
        let c = CaptureConfig::from_lookup(lookup(&[
            ("CAPTURE", "/tmp/x.png"),
            ("CAPTURE_AT", "500"),
            ("CAPTURE_EXIT", "1"),
        ]))
        .expect("set");
        assert_eq!(c.frame, 500);
        assert!(c.exit_after);
    }

    #[test]
    fn an_unparseable_frame_falls_back_to_the_default() {
        let c = CaptureConfig::from_lookup(lookup(&[
            ("CAPTURE", "/tmp/x.png"),
            ("CAPTURE_AT", "soon"),
        ]))
        .expect("set");
        assert_eq!(
            c.frame, DEFAULT_CAPTURE_FRAME,
            "a typo should not silently capture on frame 0"
        );
    }

    #[test]
    fn the_default_frame_leaves_time_for_loading() {
        // Capturing too early photographs a blank window.
        const { assert!(DEFAULT_CAPTURE_FRAME >= 120) }
    }
}
