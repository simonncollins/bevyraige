//! The transport: BRP methods an agent calls to drive the game.
//!
//! Bevy already ships a JSON-RPC server ([`bevy_remote`]) that an agent can reach
//! over HTTP, and it already carries the generic inspection verbs — `bevy/query`,
//! `bevy/get`, `bevy/list`. Adding the driving verbs to the *same* endpoint means
//! one connection, one protocol, and no second server to keep alive.
//!
//! Every method here is game-agnostic. Game verbs go on the same plugin from the
//! host project's bridge — see [`super`].
//!
//! # Methods
//!
//! | Method | Params | Does |
//! |---|---|---|
//! | `agent/key` | `{"key":"KeyS","mode":"tap"\|"hold"\|"release"}` | keyboard |
//! | `agent/pointer` | `{"x":f32,"y":f32}` | move the synthetic pointer |
//! | `agent/click` | `{"button":"left"\|"right"\|"middle"}` | click where the pointer is |
//! | `agent/press` | `{"name":"To Battle"}` or `{"x":..,"y":..}` or `{"entity":N}` | press UI |
//! | `agent/ui` | `{"named_only":bool}` | list what is pressable, topmost first |
//! | `agent/screenshot` | `{"path":"/tmp/f.png"}` | write the current frame |
//! | `agent/exclusive` | `{"on":bool}` | drop real OS input while scripting |
//! | `agent/step` | `{"ticks":N}` | advance N fixed ticks, then hold still |
//!
//! Presses and keys take effect on the **next** frame, and a screenshot is
//! written a frame or two later still — the render runs behind the schedule. An
//! agent that presses and immediately screenshots photographs the old frame; see
//! `agent/screenshot`, which reports the frame it was queued on so a caller can
//! tell.

use bevy::prelude::*;
use bevy::remote::{BrpError, BrpResult, RemotePlugin};
use bevy::ui::UiStack;
use serde_json::{json, Value};

use super::input::{AgentAction, AgentInput};
use super::step::{step, MAX_STEP_TICKS};
use super::ui::{interactable_nodes, AgentUi, PressTarget, UiNodes};
use super::ScreenshotRequest;

/// Error code for a malformed or impossible request.
///
/// JSON-RPC reserves −32602 for "invalid params", which is what every failure
/// here is: the call was understood and could not be honoured as asked.
const INVALID_PARAMS: i16 = -32602;

fn bad(message: impl Into<String>) -> BrpError {
    BrpError {
        code: INVALID_PARAMS,
        message: message.into(),
        data: None,
    }
}

/// Add the generic agent methods to a [`RemotePlugin`].
///
/// Returns the plugin so a host can chain its own bridge verbs on:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use bevy::remote::RemotePlugin;
/// # use bevyraige::devtools::agent::control::with_agent_methods;
/// # let mut app = App::new();
/// app.add_plugins(with_agent_methods(RemotePlugin::default()));
/// ```
pub fn with_agent_methods(plugin: RemotePlugin) -> RemotePlugin {
    plugin
        .with_method("agent/key", key_method)
        .with_method("agent/pointer", pointer_method)
        .with_method("agent/click", click_method)
        .with_method("agent/press", press_method)
        .with_method("agent/ui", ui_method)
        .with_method("agent/screenshot", screenshot_method)
        .with_method("agent/exclusive", exclusive_method)
        .with_method("agent/step", step_method)
}

/// `{"ticks":30}` — advance the simulation exactly that far and stop.
///
/// An **exclusive** system, which is what lets it run a schedule: it needs
/// `&mut World`, and a handler with ordinary system parameters cannot have it.
/// That also makes it synchronous — the response is sent after the ticks have
/// run, so a caller that gets a reply knows the world is already there and
/// standing still.
fn step_method(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let ticks = params
        .as_ref()
        .and_then(|p| p.get("ticks"))
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if ticks > MAX_STEP_TICKS as u64 {
        return Err(bad(format!(
            "{ticks} ticks is past the {MAX_STEP_TICKS} cap; stepping blocks the request until it finishes"
        )));
    }
    let run = step(world, ticks as u32, FixedUpdate);
    Ok(json!({ "ticks_run": run, "clock": "paused" }))
}

/// `{"key":"KeyS","mode":"tap"}` — mode defaults to `tap`.
fn key_method(In(params): In<Option<Value>>, mut agent: ResMut<AgentInput>) -> BrpResult {
    let params = params.ok_or_else(|| bad("agent/key needs {\"key\": \"KeyS\"}"))?;
    let name = params
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("agent/key needs a \"key\""))?;
    let key = parse_key(name).ok_or_else(|| bad(format!("unknown key {name:?}")))?;

    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("tap");
    agent.push(match mode {
        "tap" => AgentAction::TapKey(key),
        "hold" => AgentAction::HoldKey(key),
        "release" => AgentAction::ReleaseKey(key),
        other => return Err(bad(format!("unknown mode {other:?}"))),
    });
    Ok(json!({ "key": name, "mode": mode }))
}

/// `{"x":120,"y":48}` — physical pixels from the top-left of the frame.
fn pointer_method(In(params): In<Option<Value>>, mut agent: ResMut<AgentInput>) -> BrpResult {
    let params = params.ok_or_else(|| bad("agent/pointer needs {\"x\":_, \"y\":_}"))?;
    let (x, y) = xy(&params)?;
    agent.push(AgentAction::MovePointer(Vec2::new(x, y)));
    Ok(json!({ "x": x, "y": y }))
}

/// `{"button":"left"}` — clicks wherever the pointer was last put.
/// Click at the pointer — on the game, or on whatever button is under it.
///
/// **One verb for both.** The mouse tap alone drives the game world, because
/// that reads `ButtonInput` and a cursor. It cannot work a button: Bevy's
/// `ui_focus_system` only hit-tests cameras rendering to a window, and headless
/// retargets every camera to an image, so `Interaction` is never set however
/// good the coordinates are. A click on a button therefore did nothing, silently
/// — a caller had to know which of two mechanisms applied to the pixel it was
/// aiming at, and got no complaint when it guessed wrong.
///
/// So a left click also queues a [`PressTarget::Pointer`], which presses a
/// button if one is there and shrugs if it is not.
///
/// The two cannot both fire. `apply_presses` runs in `PreUpdate` after
/// `UiSystems::Focus`, so the `Interaction` is set before `Update` — where a
/// host's board handler reads it and stands down, exactly as it does under a
/// real cursor. That is `_is_over_ui` in the original.
fn click_method(
    In(params): In<Option<Value>>,
    mut agent: ResMut<AgentInput>,
    mut ui: ResMut<AgentUi>,
) -> BrpResult {
    let name = params
        .as_ref()
        .and_then(|p| p.get("button"))
        .and_then(Value::as_str)
        .unwrap_or("left");
    let button = match name {
        "left" => MouseButton::Left,
        "right" => MouseButton::Right,
        "middle" => MouseButton::Middle,
        other => return Err(bad(format!("unknown button {other:?}"))),
    };
    let at = agent.pointer();
    agent.push(AgentAction::TapButton(button));
    // Left only: `Interaction` has no notion of which button pressed it, so a
    // right click driving a button would be inventing a gesture.
    if button == MouseButton::Left {
        if let Some(point) = at {
            ui.press(PressTarget::Pointer(point));
        }
    }
    Ok(json!({ "button": name, "at": at.map(|p| [p.x, p.y]) }))
}

/// `{"name":"To Battle"}`, `{"x":..,"y":..}` or `{"entity":N}`.
fn press_method(In(params): In<Option<Value>>, mut agent: ResMut<AgentUi>) -> BrpResult {
    let params = params.ok_or_else(|| bad("agent/press needs a name, a point or an entity"))?;
    let target = if let Some(name) = params.get("name").and_then(Value::as_str) {
        PressTarget::Named(name.to_owned())
    } else if let Some(bits) = params.get("entity").and_then(Value::as_u64) {
        PressTarget::Entity(Entity::from_bits(bits))
    } else {
        let (x, y) = xy(&params)?;
        PressTarget::Point(Vec2::new(x, y))
    };
    agent.press(target.clone());
    // The press itself happens next frame, so this cannot report whether it
    // landed — `agent/ui` is how a caller checks.
    Ok(json!({ "queued": format!("{target:?}") }))
}

/// What is pressable, topmost first.
fn ui_method(
    In(params): In<Option<Value>>,
    stack: Res<UiStack>,
    nodes: UiNodes,
    agent: Res<AgentUi>,
) -> BrpResult {
    let named_only = params
        .as_ref()
        .and_then(|p| p.get("named_only"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let probes: Vec<Value> = interactable_nodes(&stack, &nodes)
        .into_iter()
        .filter(|probe| !named_only || probe.name.is_some())
        .map(|probe| {
            json!({
                "entity": probe.entity.to_bits(),
                "name": probe.name,
                "rect": [probe.rect.min.x, probe.rect.min.y, probe.rect.max.x, probe.rect.max.y],
                "centre": [probe.rect.center().x, probe.rect.center().y],
                "interaction": format!("{:?}", probe.interaction),
                "visible": probe.visible,
            })
        })
        .collect();

    Ok(json!({ "nodes": probes, "last_press_error": agent.last_error() }))
}

/// `{"path":"/tmp/frame.png"}`.
fn screenshot_method(
    In(params): In<Option<Value>>,
    mut request: ResMut<ScreenshotRequest>,
    frames: Res<bevy::diagnostic::FrameCount>,
) -> BrpResult {
    let path = params
        .as_ref()
        .and_then(|p| p.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| bad("agent/screenshot needs a \"path\""))?;
    request.0 = Some(path.to_owned());
    // The frame number lets a caller tell a stale screenshot from a fresh one:
    // rendering runs behind the schedule, so the file appears a frame or two
    // after this returns.
    Ok(json!({ "path": path, "queued_at_frame": frames.0 }))
}

/// `{"on":true}` — drop real OS input while a script is running.
fn exclusive_method(In(params): In<Option<Value>>, mut agent: ResMut<AgentInput>) -> BrpResult {
    let on = params
        .as_ref()
        .and_then(|p| p.get("on"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    agent.exclusive = on;
    Ok(json!({ "exclusive": on }))
}

/// Pull `x` and `y` out of a params object.
fn xy(params: &Value) -> Result<(f32, f32), BrpError> {
    let get = |key: &str| {
        params
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| bad(format!("expected a number for {key:?}")))
    };
    Ok((get("x")? as f32, get("y")? as f32))
}

/// `"KeyS"` → [`KeyCode::KeyS`], via Bevy's own `Debug` spelling.
///
/// Matching on the debug name rather than a hand-written table means every key
/// Bevy knows is addressable and the list cannot drift out of date. It costs one
/// linear scan per call, on a path that runs at human speed.
fn parse_key(name: &str) -> Option<KeyCode> {
    ALL_KEYS
        .iter()
        .copied()
        .find(|key| format!("{key:?}").eq_ignore_ascii_case(name))
}

/// The keys addressable by name.
///
/// `KeyCode` is not enumerable, so this is the reachable set. It covers the
/// letters, the digits, the arrows and the modifiers — everything a game binds.
const ALL_KEYS: &[KeyCode] = &[
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
    KeyCode::Escape,
    KeyCode::Enter,
    KeyCode::Space,
    KeyCode::Tab,
    KeyCode::Backspace,
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::ShiftLeft,
    KeyCode::ControlLeft,
    KeyCode::AltLeft,
    KeyCode::SuperLeft,
    KeyCode::Minus,
    KeyCode::Equal,
    KeyCode::BracketLeft,
    KeyCode::BracketRight,
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_the_way_bevy_names_them() {
        assert_eq!(parse_key("KeyS"), Some(KeyCode::KeyS));
        assert_eq!(parse_key("Space"), Some(KeyCode::Space));
        assert_eq!(parse_key("ArrowLeft"), Some(KeyCode::ArrowLeft));
        assert_eq!(parse_key("Digit3"), Some(KeyCode::Digit3));
    }

    /// An agent writing `"keys"` instead of `"KeyS"` should be told, not
    /// silently ignored.
    #[test]
    fn an_unknown_key_is_not_silently_dropped() {
        assert_eq!(parse_key("Banana"), None);
    }

    #[test]
    fn key_names_are_case_insensitive() {
        assert_eq!(parse_key("keys"), Some(KeyCode::KeyS));
        assert_eq!(parse_key("SPACE"), Some(KeyCode::Space));
    }

    #[test]
    fn xy_rejects_a_missing_coordinate() {
        assert!(xy(&json!({ "x": 1.0 })).is_err());
        assert!(xy(&json!({ "x": 1.0, "y": 2.0 })).is_ok());
    }

    #[test]
    fn xy_accepts_integers_as_well_as_floats() {
        assert_eq!(xy(&json!({ "x": 12, "y": 30 })).unwrap(), (12.0, 30.0));
    }
}
