//! Bevy tooling that is not about any particular game.
//!
//! Extracted from a real project — an async multiplayer autobattler — after
//! every piece here had earned its place by fixing something. Nothing in this
//! crate imports a game type, and the two modules are independent: take one, or
//! both.
//!
//! | | |
//! |---|---|
//! | [`devtools`] | Play the game with no window, over BRP. Capture frames, boot into a state, diff two runs. |
//! | [`platform`] | The handful of things that differ between a desktop build and a web one. |
//!
//! Beyond the code, `docs/` carries the parts that are not code:
//!
//! - **`docs/web-deployment.md`** — how to ship a Bevy game to the web, and the
//!   six ways it fails first. Every one of them cost a CI run to find.
//! - **`docs/harness.md`** — the conformance-harness pattern, which is a shape
//!   rather than a library: its types name your game's states, so it cannot be
//!   handed over as code.
//! - **`template/`** — files to copy into a new project: the web build script,
//!   the HTML shell, a CI workflow.
//!
//! # Start here
//!
//! ```no_run
//! use bevy::prelude::*;
//! use beveraige::devtools::DevToolsPlugin;
//! use std::str::FromStr;
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
//!     .add_plugins(DefaultPlugins)
//!     .init_state::<AppState>()
//!     .add_plugins(DevToolsPlugin::<AppState>::new("MYGAME"))
//!     .run();
//! # }
//! ```
//!
//! Everything is driven by environment variables and is **inert without them**,
//! so this can ship in a release build for the cost of a branch at startup:
//!
//! ```text
//! MYGAME_HEADLESS=1280x720      # no window at all; drive it over BRP
//! MYGAME_START_STATE=Playing    # boot past the menus
//! MYGAME_CAPTURE=/tmp/shot.png  # write a frame and exit
//! ```
//!
//! # The one thing to read before using the agent
//!
//! **A game window takes the focus of whatever machine it opens on.** That is
//! what confines agent-driven testing to a machine nobody is using, and on
//! macOS it cannot be fixed with a window at all — winit calls
//! `activateIgnoringOtherApps` unconditionally and Bevy exposes no hook. So
//! headless mode removes the window and renders offscreen on the real GPU.
//!
//! The consequence worth knowing up front: **Bevy's `ui_focus_system` only
//! hit-tests cameras rendering to a window**, and headless retargets every
//! camera to an image. `Interaction` is therefore never set, however good your
//! coordinates are — so a button worth driving in a test needs a `Name`, and
//! [`devtools::agent`] presses it by name. See that module for the full contract
//! a host project has to meet.

pub mod devtools;
pub mod platform;
