//! Driving a Bevy game from outside it, without taking over the machine.
//!
//! **This module knows nothing about any game**, exactly like the rest of
//! [`devtools`](crate::devtools). It provides the four things an agent needs to
//! play a game it is developing, and nothing about what that game *is*:
//!
//! | Need | Piece |
//! |---|---|
//! | Run without stealing the desktop | [`headless`] — no window, no event loop, real GPU rendering |
//! | Press keys and buttons | [`input`] — synthetic input that cannot reach the OS |
//! | Press UI the engine will not | [`ui`] — hit-testing headless mode leaves undone |
//! | Observe without racing the sim | [`step`] — advance exactly N ticks, then stop |
//! | Read the board as text | [`ascii`] — rulers, subsampling and a legend around a game's own sampler |
//! | Be told what to do, live | [`control`] — the transport, over BRP |
//!
//! # The two problems this solves
//!
//! **Focus.** A game window takes the desktop, which makes agent-driven testing
//! something you can only run when nobody is using the machine. On macOS this
//! cannot be fixed with a window at all — see [`headless`] for the winit call
//! that makes it impossible — so this removes the window instead.
//!
//! **Input collision.** Synthetic input driven through OS APIs is *global*: it
//! lands on whatever is focused, which during development is somebody's editor.
//! Everything here is injected inside the process, so it cannot leave. Headless
//! mode closes the other direction too, because there is no OS input path to
//! collide with.
//!
//! # Writing the bridge for your game
//!
//! This module deliberately stops at the point where game knowledge begins. It
//! can press a button called `"To Battle"`; it has no idea what a battle is. The
//! game-specific half is a **bridge** — one module in your project that turns
//! agent requests into your game's own vocabulary. In this repository that is
//! a `harness::live` module; in yours it will be something else. The worked
//! example is in `docs/agent-bridge.md`.
//!
//! A bridge is worth writing when the generic verbs get clumsy. "Press the
//! Builder button, move the pointer to (412, 300), click" is three calls and a
//! coordinate; `place_trigger(Builder, 300, 500)` is one and survives a UI
//! redesign. Write bridge verbs for the things a *test* wants to say.
//!
//! ## What the host project must provide
//!
//! Beyond the contract in [`devtools`](crate::devtools) — one `States` enum,
//! gameplay reachable by setting it, a deterministic fixed-timestep simulation:
//!
//! 1. **Name the buttons worth pressing.** [`ui::PressTarget::Named`] matches on
//!    Bevy's [`Name`] component. A button with no `Name` can only be pressed by
//!    entity id or by pixel, and both of those are things a test should not have
//!    to know. Adding `Name::new("To Battle")` beside the button is the whole
//!    cost.
//!
//! 2. **Keep rendering independent of window size.** Headless renders at whatever
//!    frame size is asked for. A layout that only works at one resolution will
//!    disagree with what a player sees, and the screenshots will mislead you.
//!
//! 3. **Do not read input from anywhere but Bevy.** Anything reading an OS input
//!    API directly is invisible to [`input`] and cannot be driven.
//!
//! ## Registering bridge verbs
//!
//! [`control`] takes the generic methods; your bridge adds its own to the same
//! BRP plugin, so an agent talks to one endpoint:
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # use bevy::remote::{RemotePlugin, BrpResult};
//! # use bevyraige::devtools::agent;
//! # use serde_json::Value;
//! # let mut app = App::new();
//! # fn place_trigger(_: In<Option<Value>>) -> BrpResult { unimplemented!() }
//! app.add_plugins(
//!     agent::control::with_agent_methods(RemotePlugin::default())
//!         .with_method("game/place_trigger", place_trigger),
//! );
//! ```
//!
//! A bridge verb is an ordinary system taking `In<Option<Value>>` and returning
//! `BrpResult`. It has the whole `World`, so it can do anything a test needs —
//! and should return *what the agent needs to decide the next step*, not just
//! `Ok`. A verb that reports nothing forces a screenshot after every call.

pub mod ascii;
// BRP is native-only — its HTTP server needs real sockets, which is why
// `bevy_remote` is a per-target dependency. The rest of this module is portable.
#[cfg(not(target_arch = "wasm32"))]
pub mod control;
pub mod headless;
pub mod input;
pub mod step;
pub mod ui;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

pub use ascii::AsciiView;
pub use headless::{HeadlessConfig, HeadlessPlugin, HeadlessTarget};
pub use input::{AgentAction, AgentInput, AgentInputPlugin};
pub use step::{step, MAX_STEP_TICKS};
pub use ui::{AgentUi, AgentUiPlugin, PressTarget};

/// A screenshot the agent has asked for, waiting to be taken.
#[derive(Resource, Default)]
pub struct ScreenshotRequest(pub Option<String>);

/// Everything an agent needs, keyed off one environment-variable prefix.
///
/// Inert in a normal run: without `{PREFIX}_HEADLESS` the window is untouched,
/// and the input and UI layers do nothing until something queues an action.
/// Safe to install unconditionally.
pub struct AgentPlugin {
    pub prefix: &'static str,
}

impl AgentPlugin {
    pub fn new(prefix: &'static str) -> Self {
        Self { prefix }
    }
}

impl Plugin for AgentPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(HeadlessPlugin {
            config: HeadlessConfig::from_env(self.prefix),
        });
        app.add_plugins(AgentInputPlugin);
        app.add_plugins(AgentUiPlugin);
        app.init_resource::<ScreenshotRequest>();
        app.add_systems(Update, take_requested_screenshot);
    }
}

/// System: honour a queued screenshot request.
///
/// Reads the offscreen image in headless mode and the window otherwise, so the
/// same request works either way and a script does not care which mode it is in.
fn take_requested_screenshot(
    mut commands: Commands,
    mut request: ResMut<ScreenshotRequest>,
    target: Option<Res<HeadlessTarget>>,
) {
    let Some(path) = request.0.take() else {
        return;
    };
    let screenshot = match target {
        Some(target) => Screenshot::image(target.0.clone()),
        None => Screenshot::primary_window(),
    };
    commands.spawn(screenshot).observe(save_to_disk(path));
}
