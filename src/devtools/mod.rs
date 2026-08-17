//! Portable Bevy dev tooling: capture a frame, boot into a state, follow a run.
//!
//! **This module deliberately knows nothing about this game.** It imports no
//! project types and is intended to be lifted out into a shared crate. Keep it
//! that way: anything here that needs a game concept belongs on the host side
//! of the contract below.
//!
//! # Why it exists
//!
//! Three things are hard to do from outside a running game, and all three are
//! things you want constantly when working on one — especially with an agent or
//! a CI job driving:
//!
//! | Problem | Tool |
//! |---|---|
//! | "Show me what it looks like" | [`capture`] — takes the rendered frame from Bevy itself |
//! | "Get me to the gameplay" | [`boot`] — jumps past the menus into a named state |
//! | "Is this run the same as that one?" | [`probe`] — records state per tick and diffs two runs |
//! | "Turn the music down / shrink the window" | [`options`] — a **game-defined** settings payload |
//! | "Play the game without taking my desktop" | [`agent`] — no window, synthetic input, driven over BRP |
//!
//! Each is driven by an environment variable and is completely inert without
//! it, so this can be installed unconditionally in release builds and costs a
//! branch at startup.
//!
//! # Usage
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # use bevyraige::devtools::DevToolsPlugin;
//! # use std::str::FromStr;
//! #[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
//! enum AppState {
//!     #[default]
//!     Menu,
//!     Playing,
//! }
//!
//! // **Required.** `MYGAME_START_STATE=Playing` is a string, and this is what
//! // turns it into a state. Match on whatever spellings you want to accept.
//! impl FromStr for AppState {
//!     type Err = ();
//!     fn from_str(s: &str) -> Result<Self, ()> {
//!         match s.to_ascii_lowercase().as_str() {
//!             "menu" => Ok(AppState::Menu),
//!             "playing" | "play" => Ok(AppState::Playing),
//!             _ => Err(()),
//!         }
//!     }
//! }
//!
//! # fn main() {
//! App::new()
//!     .init_state::<AppState>()
//!     .add_plugins(DevToolsPlugin::<AppState>::new("MYGAME"))
//!     .run();
//! # }
//! ```
//!
//! ```text
//! MYGAME_CAPTURE=/tmp/shot.png     # write a frame here
//! MYGAME_CAPTURE_AT=500            # on this frame (default 180)
//! MYGAME_CAPTURE_EXIT=1            # quit once written
//! MYGAME_START_STATE=Practice      # boot straight into a state
//! MYGAME_HEADLESS=1280x720         # no window at all; drive it over BRP
//! ```
//!
//! # Agent-driven play
//!
//! [`agent`] is the newest and largest piece, and the one with the sharpest
//! reason to exist: **a game window takes the focus of whatever machine it opens
//! on**, which confines agent-driven testing to a machine nobody is using. On
//! macOS that cannot be fixed with a window at all — winit activates the
//! application unconditionally and Bevy exposes no hook — so `MYGAME_HEADLESS`
//! removes the window instead, and the game renders offscreen on the real GPU.
//! Input is injected inside the process, so it can neither escape onto the
//! desktop nor be corrupted by someone typing. See [`agent`] for the contract a
//! host project must meet, and for how to write the game-specific bridge that
//! turns agent requests into your own vocabulary.
//!
//! # What a host project must provide
//!
//! This is the opinionated part — the shape a Bevy project needs for this
//! tooling to work without modification:
//!
//! 1. **A single top-level `States` enum** covering every screen, implementing
//!    [`FromStr`](std::str::FromStr) so states can be named from outside. One
//!    enum, not several: a state graph split across plugins cannot be addressed
//!    by name from the command line.
//!
//! 2. **Gameplay reachable by setting that state alone.** If entering a match
//!    also requires a resource that only a menu button populates, no external
//!    tool can get there. Push that setup into an `OnEnter` handler instead —
//!    which is also what makes the state resumable, testable and replayable.
//!
//! 3. **A deterministic simulation on a fixed timestep**, separable from
//!    rendering, so [`probe`] can compare two runs meaningfully. If frame time
//!    leaks into simulation, two runs never agree and the diff is noise.
//!
//! 4. **A snapshot function** for [`probe`], producing one stable, sorted line
//!    per thing worth watching. Sorted matters: unordered ECS iteration makes
//!    every comparison a false positive.
//!
//! Points 2 and 3 are worth having regardless — this tooling mostly rewards
//! decisions a testable game wants anyway.

pub mod agent;
pub mod boot;
pub mod capture;
pub mod env;
pub mod options;
pub mod probe;

use std::marker::PhantomData;
use std::str::FromStr;

use bevy::prelude::*;
use bevy::state::state::FreelyMutableState;

pub use boot::{BootState, BootStatePlugin};
pub use capture::{CaptureConfig, CapturePlugin};
pub use options::{load_options, DevOptionsPlugin};
pub use probe::{Divergence, Probe, ProbeFrame};

/// All the dev tools, keyed off one environment-variable prefix.
///
/// Generic over the project's state enum so [`boot`] can name its variants.
pub struct DevToolsPlugin<S: FreelyMutableState + FromStr> {
    prefix: &'static str,
    _marker: PhantomData<S>,
}

impl<S: FreelyMutableState + FromStr> DevToolsPlugin<S> {
    /// Tools under `{prefix}_*` environment variables.
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            _marker: PhantomData,
        }
    }
}

impl<S: FreelyMutableState + FromStr + Clone> Plugin for DevToolsPlugin<S> {
    fn build(&self, app: &mut App) {
        app.add_plugins(CapturePlugin {
            prefix: self.prefix,
        });
        app.add_plugins(BootStatePlugin::<S>::new(self.prefix));
        app.add_plugins(agent::AgentPlugin::new(self.prefix));
    }
}
