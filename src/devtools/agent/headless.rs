//! Running the game with no window at all.
//!
//! # Why there is no "don't steal focus" option
//!
//! On macOS there is no way to open a window without taking focus. winit calls
//! `NSApp.activateIgnoringOtherApps(true)` unconditionally from
//! `applicationDidFinishLaunching` — see `winit-0.30/src/platform_impl/macos/
//! app_state.rs` — and the only knob that suppresses it,
//! `EventLoopBuilderExtMacOS::with_activation_policy`, is set while *building the
//! event loop*, which `bevy_winit::WinitPlugin` does internally and exposes
//! nothing for. `Window { focused: false }` is not it either: that maps to
//! winit's `with_active(false)`, which governs whether the window becomes *key*,
//! not whether the application activates. Measured on macOS 15.7: the app still
//! became frontmost.
//!
//! (winit does skip forcing the activation policy when the binary is *bundled*,
//! deferring to `LSUIElement` in an `Info.plist`. A `cargo run` binary is not
//! bundled, and requiring a bundle to run tests is a worse trade than not having
//! a window.)
//!
//! So the tool does not try. **It removes the event loop instead.** Without
//! `WinitPlugin` the process never links an event loop, never touches AppKit, and
//! cannot take focus — because nothing about it is a GUI application any more.
//!
//! The `Window` *entity* stays, as inert data. It is where Bevy keeps the cursor
//! position, and game code reads it there; without one, every pointer-driven
//! path is unreachable. See [`without_window`].
//! Rendering is unaffected: wgpu does not need a surface, so the game draws to an
//! image at full quality on the real GPU, and [`Screenshot::image`] reads it back.
//!
//! It also solves the *other* half of driving a game from outside. With no window
//! there is no OS input path at all, so synthetic input cannot leak onto the
//! developer's desktop and their typing cannot leak into the run. See
//! [`super::input`].
//!
//! # Cost
//!
//! Bevy's `ui_focus_system` only hit-tests cameras whose render target is a
//! *window*, and every camera here is retargeted to an image — so `Interaction`
//! is still never set by Bevy, window entity or not. [`super::ui`] replaces it
//! with an equivalent hit-test over the same layout data, and `agent/press`
//! remains the way to work a button.

use bevy::app::{PluginGroupBuilder, ScheduleRunnerPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::ui::IsDefaultUiCamera;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use core::time::Duration;

use crate::devtools::env;

/// How big the offscreen frame is, and how fast the loop runs.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeadlessConfig {
    pub width: u32,
    pub height: u32,
    /// Wall-clock between updates. Zero runs as fast as the machine allows.
    pub frame_interval: Duration,
}

/// Frame size when the environment names none.
pub const DEFAULT_HEADLESS_SIZE: (u32, u32) = (1280, 720);

/// Default pacing: about 60Hz.
///
/// Not zero. A fixed-timestep simulation reads a wall clock, so a loop running
/// flat out fast-forwards the game — 10Hz ticks arriving hundreds of times a
/// second — and nothing an agent observes lines up with what a player would see.
/// Deliberate fast-forward is a separate control, not the default.
pub const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

impl HeadlessConfig {
    /// Reads `{PREFIX}_HEADLESS`: `1`, or `WIDTHxHEIGHT` to set the frame size.
    pub fn from_env(prefix: &str) -> Option<Self> {
        Self::from_value(env::var(prefix, "HEADLESS"))
    }

    /// Parses the raw value, so the rules are testable without the environment.
    pub fn from_value(raw: Option<String>) -> Option<Self> {
        let raw = env::non_empty(raw)?;
        if matches!(raw.to_ascii_lowercase().as_str(), "0" | "false" | "no") {
            return None;
        }
        let (width, height) = parse_size(&raw).unwrap_or(DEFAULT_HEADLESS_SIZE);
        Some(Self {
            width,
            height,
            frame_interval: DEFAULT_FRAME_INTERVAL,
        })
    }
}

/// `"1280x720"` → `(1280, 720)`. Anything else is `None`.
fn parse_size(raw: &str) -> Option<(u32, u32)> {
    let (w, h) = raw.split_once(['x', 'X'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// The image every camera is redirected to, and what a screenshot reads.
#[derive(Resource, Debug, Clone)]
pub struct HeadlessTarget(pub Handle<Image>);

/// Marks a camera already pointed at the offscreen image.
#[derive(Component)]
struct Retargeted;

/// Strip the **event loop** out of a plugin group, keeping the window as data.
///
/// Call this where the group is built — window configuration is read while
/// `DefaultPlugins` builds, long before any system runs, so it cannot be done
/// from a plugin. Passing `None` leaves the group exactly as it was, which is
/// what keeps this inert in a normal run.
///
/// `ExitCondition::DontExit` is load-bearing: the default is `OnAllClosed`, and
/// the window here is never opened in the first place.
///
/// # The `Window` entity stays; only winit goes
///
/// Removing `WinitPlugin` is the whole trick — no event loop, no AppKit, no
/// focus theft. The entity is a different matter, and dropping it too
/// (`primary_window: None`, which this used to do) cost more than it saved.
///
/// A `Window` is where Bevy keeps the **cursor position**, and game code reads
/// it there: `Window::cursor_position` is how a board turns a pointer into a
/// world coordinate. With no entity, `feed_agent_input`'s
/// `set_physical_cursor_position` had nothing to write to and every
/// pointer-driven path was silently unreachable — placing a trigger by clicking,
/// dragging one, the trash zone, selecting a role. Only the paths with a
/// bespoke BRP verb behind them (`game/place_trigger`) could be driven at all,
/// which is why every playtest so far has used one. `agent/pointer` and
/// `agent/click` appeared to work and did nothing.
///
/// Without winit the entity is inert data: nothing creates a surface for it,
/// because the renderer extracts windows by their `RawHandleWrapper` and only
/// winit adds one. It is a struct holding a size and a cursor.
///
/// The resolution matches the offscreen frame so that a screen coordinate means
/// the same thing to the game as it does to [`super::ui`]'s hit-test and to
/// `Screenshot::image`. `visible: false` is belt and braces — nothing is
/// listening — and states the intent for anyone who re-enables winit here.
pub fn without_window(
    group: PluginGroupBuilder,
    config: Option<HeadlessConfig>,
) -> PluginGroupBuilder {
    let Some(config) = config else {
        return group;
    };
    group.disable::<WinitPlugin>().set(WindowPlugin {
        primary_window: Some(Window {
            resolution: bevy::window::WindowResolution::new(config.width, config.height),
            visible: false,
            ..default()
        }),
        exit_condition: ExitCondition::DontExit,
        close_when_requested: false,
        ..default()
    })
}

/// Drives and renders a windowless app.
///
/// Add unconditionally; without a [`HeadlessConfig`] it does nothing. It must be
/// paired with [`without_window`] on the plugin group — this half cannot remove
/// the window on its own.
pub struct HeadlessPlugin {
    pub config: Option<HeadlessConfig>,
}

impl Plugin for HeadlessPlugin {
    fn build(&self, app: &mut App) {
        let Some(config) = self.config else {
            return;
        };
        app.insert_resource(config);
        // Nothing pumps the schedule once `WinitPlugin` is gone.
        app.add_plugins(ScheduleRunnerPlugin::run_loop(config.frame_interval));
        app.add_systems(PreStartup, create_target);
        // Every frame, not once: cameras are spawned by whichever screen is up,
        // so one that appears on entering a match still has to be caught.
        app.add_systems(PreUpdate, retarget_cameras);
    }
}

/// Make the image the game will be drawn into.
fn create_target(
    mut commands: Commands,
    config: Res<HeadlessConfig>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut image = Image::new_fill(
        Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // COPY_SRC is what makes it readable back out; without it a screenshot of
    // this image fails at the wgpu level rather than in anything we control.
    image.texture_descriptor.usage =
        TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING;
    image.sampler = ImageSampler::nearest();
    commands.insert_resource(HeadlessTarget(images.add(image)));
}

/// Point every camera at the offscreen image, and nominate one to carry the UI.
///
/// A camera's default target is the primary window, which does not exist here —
/// so a camera left alone renders nowhere and the frame comes back blank.
///
/// **`IsDefaultUiCamera` is the non-obvious half.** `bevy_ui` resolves which
/// camera a UI tree belongs to by looking for that marker and otherwise falling
/// back to *the camera rendering to the primary window*. With no window the
/// fallback finds nothing, so every node lays out — the coordinates are real, and
/// an inspector will happily report them — and then draws nowhere. The symptom is
/// a screenshot of the game world with the entire interface missing, which reads
/// like a render bug rather than a missing marker.
fn retarget_cameras(
    mut commands: Commands,
    target: Option<Res<HeadlessTarget>>,
    cameras: Query<(Entity, &Camera), Without<Retargeted>>,
    already_default: Query<(), With<IsDefaultUiCamera>>,
) {
    let Some(target) = target else {
        return;
    };
    if cameras.is_empty() {
        return;
    }

    // Lowest `order`, then lowest entity — a stable choice, because iteration
    // order is not, and the UI must not migrate between cameras frame to frame.
    let ui_camera = already_default.is_empty().then(|| {
        cameras
            .iter()
            .min_by_key(|(entity, camera)| (camera.order, *entity))
            .map(|(entity, _)| entity)
    });

    for (entity, _) in &cameras {
        commands
            .entity(entity)
            .insert((RenderTarget::Image(target.0.clone().into()), Retargeted));
        if ui_camera == Some(Some(entity)) {
            commands.entity(entity).insert(IsDefaultUiCamera);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_variable_leaves_the_game_windowed() {
        assert_eq!(HeadlessConfig::from_value(None), None);
        assert_eq!(HeadlessConfig::from_value(Some(String::new())), None);
    }

    /// `FOO=0` in a shell profile means off, the same as everywhere else in
    /// `devtools` — see `env::is_truthy`.
    #[test]
    fn the_usual_negatives_leave_the_game_windowed() {
        for off in ["0", "false", "no", "NO"] {
            assert_eq!(
                HeadlessConfig::from_value(Some(off.into())),
                None,
                "{off} should read as off"
            );
        }
    }

    #[test]
    fn a_bare_flag_uses_the_default_frame_size() {
        let c = HeadlessConfig::from_value(Some("1".into())).expect("on");
        assert_eq!((c.width, c.height), DEFAULT_HEADLESS_SIZE);
    }

    #[test]
    fn a_size_can_be_named() {
        let c = HeadlessConfig::from_value(Some("640x360".into())).expect("on");
        assert_eq!((c.width, c.height), (640, 360));
    }

    /// An unparseable size is still "on" — a typo should cost the default frame
    /// size, not silently put the window back and take the developer's focus.
    #[test]
    fn a_malformed_size_still_runs_headless() {
        let c = HeadlessConfig::from_value(Some("enormous".into())).expect("on");
        assert_eq!((c.width, c.height), DEFAULT_HEADLESS_SIZE);
    }

    /// The loop is paced rather than free-running: a fixed-timestep simulation
    /// reads a wall clock, so running flat out fast-forwards the game.
    #[test]
    fn the_default_loop_is_paced() {
        let c = HeadlessConfig::from_value(Some("1".into())).expect("on");
        assert!(c.frame_interval > Duration::ZERO);
        assert!(c.frame_interval < Duration::from_millis(100));
    }
}
