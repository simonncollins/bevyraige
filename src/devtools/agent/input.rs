//! Synthetic keyboard and pointer input that cannot reach the desktop.
//!
//! # The isolation guarantee
//!
//! Everything here writes Bevy's own input messages — the exact messages
//! `bevy_winit` would write — straight into the running `App`. Nothing calls an
//! OS input API, so a synthetic click cannot land on whatever window the
//! developer is actually using, and cannot be mistaken by anything outside this
//! process for a real event. In [headless mode](super::headless) the guarantee is
//! total in both directions: there is no OS input path at all, so their typing
//! cannot leak into the run either.
//!
//! With a window on screen the second direction is not free — a real click on the
//! game window is a real event. [`AgentInput::exclusive`] closes that: while it is
//! set, real input messages are dropped before the engine reads them, so a
//! scripted run cannot be corrupted by someone bumping the mouse.
//!
//! # Where it is injected
//!
//! `bevy_input` folds raw messages into `ButtonInput` in `PreUpdate`, in the
//! `InputSystems` set. Writing the messages *before* that set means the state
//! lands the same frame and by the same path as real input, so a game reading
//! either `ButtonInput` or the messages themselves sees it. Writing directly to
//! `ButtonInput` instead would work for the first kind of reader and silently
//! not for the second.

use std::collections::{HashSet, VecDeque};

use bevy::ecs::message::Messages;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow};

/// One thing to do to the game's input state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentAction {
    /// Press and release over two frames — one keystroke.
    TapKey(KeyCode),
    /// Press and hold until [`AgentAction::ReleaseKey`].
    HoldKey(KeyCode),
    ReleaseKey(KeyCode),
    /// Move the pointer, in physical pixels from the top-left of the frame.
    MovePointer(Vec2),
    /// Press and release over two frames — one click, wherever the pointer is.
    TapButton(MouseButton),
    HoldButton(MouseButton),
    ReleaseButton(MouseButton),
}

/// Pending synthetic input, and what is currently held down.
#[derive(Resource, Default)]
pub struct AgentInput {
    queue: VecDeque<AgentAction>,
    held_keys: HashSet<KeyCode>,
    held_buttons: HashSet<MouseButton>,
    /// Released at the start of the next frame — this is what makes a tap a tap.
    release_keys: Vec<KeyCode>,
    release_buttons: Vec<MouseButton>,
    pointer: Option<Vec2>,
    /// Drop real OS input before the engine sees it.
    ///
    /// Only meaningful with a window on screen; headless has no real input to
    /// drop. Off by default, because silently eating a developer's clicks on a
    /// window they can see is a surprising default.
    pub exclusive: bool,
}

impl AgentInput {
    /// Queue an action for the next frame.
    pub fn push(&mut self, action: AgentAction) {
        self.queue.push_back(action);
    }

    /// Where the synthetic pointer is, if it has been placed.
    pub fn pointer(&self) -> Option<Vec2> {
        self.pointer
    }

    /// Whether anything is still waiting to be applied.
    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.release_keys.is_empty() && self.release_buttons.is_empty()
    }
}

/// Feeds queued actions in as engine input messages.
pub struct AgentInputPlugin;

impl Plugin for AgentInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AgentInput>();
        // `CursorMoved` belongs to `WindowPlugin`, not `InputPlugin`, so an app
        // built without windows has the type but not the buffer — and a
        // `MessageWriter` for a message with no buffer fails system validation
        // at runtime rather than at compile time. Registered here only if
        // absent, because registering twice would add a second cleanup system
        // and halve how long every cursor message survives.
        if !app.world().contains_resource::<Messages<CursorMoved>>() {
            app.add_message::<CursorMoved>();
        }
        app.add_systems(
            PreUpdate,
            (suppress_real_input, feed_agent_input)
                .chain()
                .before(bevy::input::InputSystems),
        );
    }
}

/// Drop real input before `bevy_input` reads it.
///
/// Clearing the buffers rather than filtering them: a message is a broadcast,
/// and there is no way to remove one recipient's view of it. Everything written
/// after this — which is [`feed_agent_input`], chained next — survives.
fn suppress_real_input(
    agent: Res<AgentInput>,
    mut keys: ResMut<Messages<KeyboardInput>>,
    mut buttons: ResMut<Messages<MouseButtonInput>>,
    mut moved: ResMut<Messages<CursorMoved>>,
    mut wheel: ResMut<Messages<MouseWheel>>,
    mut motion: ResMut<Messages<MouseMotion>>,
) {
    if !agent.exclusive {
        return;
    }
    keys.clear();
    buttons.clear();
    moved.clear();
    wheel.clear();
    motion.clear();
}

#[allow(clippy::too_many_arguments)] // Four message streams, the window, and the queue.
fn feed_agent_input(
    mut agent: ResMut<AgentInput>,
    mut keys: MessageWriter<KeyboardInput>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut moved: MessageWriter<CursorMoved>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    // Headless has no window; the field is still required on the messages, and
    // nothing in `bevy_input` dereferences it.
    let window = primary.iter().next().unwrap_or(Entity::PLACEHOLDER);

    // Last frame's taps come up first, so a tap is exactly one frame down.
    for key in std::mem::take(&mut agent.release_keys) {
        agent.held_keys.remove(&key);
        keys.write(key_message(key, ButtonState::Released, window));
    }
    for button in std::mem::take(&mut agent.release_buttons) {
        agent.held_buttons.remove(&button);
        buttons.write(MouseButtonInput {
            button,
            state: ButtonState::Released,
            window,
        });
    }

    let mut pointer_moved = false;
    for action in std::mem::take(&mut agent.queue) {
        match action {
            AgentAction::TapKey(key) => {
                agent.held_keys.insert(key);
                agent.release_keys.push(key);
                keys.write(key_message(key, ButtonState::Pressed, window));
            }
            AgentAction::HoldKey(key) => {
                if agent.held_keys.insert(key) {
                    keys.write(key_message(key, ButtonState::Pressed, window));
                }
            }
            AgentAction::ReleaseKey(key) => {
                if agent.held_keys.remove(&key) {
                    keys.write(key_message(key, ButtonState::Released, window));
                }
            }
            AgentAction::MovePointer(position) => {
                agent.pointer = Some(position);
                pointer_moved = true;
            }
            AgentAction::TapButton(button) => {
                agent.held_buttons.insert(button);
                agent.release_buttons.push(button);
                buttons.write(MouseButtonInput {
                    button,
                    state: ButtonState::Pressed,
                    window,
                });
            }
            AgentAction::HoldButton(button) => {
                if agent.held_buttons.insert(button) {
                    buttons.write(MouseButtonInput {
                        button,
                        state: ButtonState::Pressed,
                        window,
                    });
                }
            }
            AgentAction::ReleaseButton(button) => {
                if agent.held_buttons.remove(&button) {
                    buttons.write(MouseButtonInput {
                        button,
                        state: ButtonState::Released,
                        window,
                    });
                }
            }
        }
    }

    // The window stores the cursor position and hit-testing reads it from there,
    // so moving the pointer means writing it back every frame — otherwise a real
    // `CursorLeft`, or the game's own reset, quietly takes it away mid-script.
    if let Some(position) = agent.pointer {
        if let Some(mut primary_window) = windows.iter_mut().next() {
            primary_window.set_physical_cursor_position(Some(position.as_dvec2()));
            if pointer_moved {
                moved.write(CursorMoved {
                    window,
                    position: position / primary_window.scale_factor(),
                    delta: None,
                });
            }
        }
    }
}

/// A keyboard message with the fields nothing here can know left unidentified.
///
/// `logical_key` and `text` are layout-dependent, and the point of driving by
/// `KeyCode` is to name a *physical* key regardless of layout. A game that reads
/// typed text wants a real text-entry path, not a synthesised keystroke.
fn key_message(key_code: KeyCode, state: ButtonState, window: Entity) -> KeyboardInput {
    KeyboardInput {
        key_code,
        logical_key: Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified),
        state,
        text: None,
        repeat: false,
        window,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(bevy::input::InputPlugin);
        app.add_plugins(AgentInputPlugin);
        app
    }

    /// The whole point: a queued key reaches `ButtonInput`, which is what a game
    /// actually reads.
    #[test]
    fn a_tapped_key_arrives_as_a_real_key_press() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::TapKey(KeyCode::KeyS));
        app.update();

        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            keys.just_pressed(KeyCode::KeyS),
            "pressed on the first frame"
        );
    }

    /// A tap is one frame down and then up — held forever would jam the game.
    #[test]
    fn a_tapped_key_comes_back_up_by_itself() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::TapKey(KeyCode::KeyS));
        app.update();
        app.update();

        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            keys.just_released(KeyCode::KeyS),
            "released on the next frame"
        );
        assert!(!keys.pressed(KeyCode::KeyS));
    }

    #[test]
    fn a_held_key_stays_down_until_released() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::HoldKey(KeyCode::KeyD));
        app.update();
        app.update();
        app.update();
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyD),
            "still down three frames later"
        );

        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::ReleaseKey(KeyCode::KeyD));
        app.update();
        assert!(!app
            .world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyD));
    }

    #[test]
    fn a_tapped_mouse_button_presses_then_releases() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::TapButton(MouseButton::Left));
        app.update();
        assert!(app
            .world()
            .resource::<ButtonInput<MouseButton>>()
            .just_pressed(MouseButton::Left));

        app.update();
        assert!(app
            .world()
            .resource::<ButtonInput<MouseButton>>()
            .just_released(MouseButton::Left));
    }

    /// Exclusive mode exists so someone bumping the mouse cannot corrupt a
    /// scripted run. Real input is dropped; ours still lands.
    #[test]
    fn exclusive_mode_drops_real_input_but_not_synthetic() {
        let mut app = input_app();
        app.world_mut().resource_mut::<AgentInput>().exclusive = true;

        // A real keypress, as `bevy_winit` would write it.
        app.world_mut()
            .resource_mut::<Messages<KeyboardInput>>()
            .write(key_message(
                KeyCode::KeyR,
                ButtonState::Pressed,
                Entity::PLACEHOLDER,
            ));
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::TapKey(KeyCode::KeyA));
        app.update();

        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            !keys.pressed(KeyCode::KeyR),
            "the real keypress was dropped"
        );
        assert!(keys.pressed(KeyCode::KeyA), "the synthetic one was not");
    }

    /// Off by default: silently eating input on a window the developer can see
    /// is a surprising thing to do without being asked.
    #[test]
    fn real_input_is_untouched_unless_exclusivity_is_asked_for() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<Messages<KeyboardInput>>()
            .write(key_message(
                KeyCode::KeyR,
                ButtonState::Pressed,
                Entity::PLACEHOLDER,
            ));
        app.update();
        assert!(app
            .world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyR));
    }

    #[test]
    fn the_pointer_remembers_where_it_was_put() {
        let mut app = input_app();
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::MovePointer(Vec2::new(120.0, 48.0)));
        app.update();
        assert_eq!(
            app.world().resource::<AgentInput>().pointer(),
            Some(Vec2::new(120.0, 48.0))
        );
    }

    #[test]
    fn an_empty_queue_is_idle() {
        let mut app = input_app();
        assert!(app.world().resource::<AgentInput>().is_idle());
        app.world_mut()
            .resource_mut::<AgentInput>()
            .push(AgentAction::TapKey(KeyCode::Space));
        assert!(!app.world().resource::<AgentInput>().is_idle());
        app.update();
        // The release is still outstanding.
        assert!(!app.world().resource::<AgentInput>().is_idle());
        app.update();
        assert!(app.world().resource::<AgentInput>().is_idle());
    }
}
