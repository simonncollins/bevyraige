//! Booting straight into a chosen state.
//!
//! Reaching gameplay in most games means clicking through several menus, which
//! a capture run, a CI check or an agent cannot do. This jumps to a named state
//! on the first update.
//!
//! Generic over the project's state enum: the host implements [`FromStr`] for
//! it and nothing here needs to know the variants.

use std::marker::PhantomData;
use std::str::FromStr;

use bevy::prelude::*;
use bevy::state::state::FreelyMutableState;

use super::env;

/// The state to boot into.
#[derive(Resource, Debug)]
pub struct BootState<S: States>(pub S);

/// System: jumps to the configured state once, on the first update.
///
/// Deliberately in `Update` rather than `Startup`: every `OnEnter` handler and
/// the state machine itself must be in place first, or the jump fires into a
/// world that is not ready to receive it.
pub fn apply_boot_state<S: FreelyMutableState + Clone>(
    mut next: ResMut<NextState<S>>,
    boot: Res<BootState<S>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    info!("Booting straight into {:?}.", boot.0);
    next.set(boot.0.clone());
}

/// Adds a boot-state jump when `{PREFIX}_START_STATE` names a parseable state.
///
/// An unrecognised name warns and is ignored rather than failing the run, so a
/// stale value in a shell profile costs a log line rather than a crash.
pub struct BootStatePlugin<S: FreelyMutableState + FromStr> {
    pub prefix: &'static str,
    pub _marker: PhantomData<S>,
}

impl<S: FreelyMutableState + FromStr> BootStatePlugin<S> {
    pub fn new(prefix: &'static str) -> Self {
        Self {
            prefix,
            _marker: PhantomData,
        }
    }
}

impl<S: FreelyMutableState + FromStr + Clone> Plugin for BootStatePlugin<S> {
    fn build(&self, app: &mut App) {
        let Some(raw) = env::var(self.prefix, "START_STATE") else {
            return;
        };
        match S::from_str(&raw) {
            Ok(state) => {
                app.insert_resource(BootState(state));
                app.add_systems(Update, apply_boot_state::<S>);
            }
            Err(_) => warn!(
                "{}_START_STATE: '{}' is not a known state; staying where we are.",
                self.prefix, raw
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
    enum TestState {
        #[default]
        Menu,
        Playing,
    }

    impl FromStr for TestState {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            match s.to_ascii_lowercase().as_str() {
                "menu" => Ok(TestState::Menu),
                "playing" => Ok(TestState::Playing),
                _ => Err(()),
            }
        }
    }

    fn boot_app(state: TestState) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
        app.init_state::<TestState>();
        app.insert_resource(BootState(state));
        app.add_systems(Update, apply_boot_state::<TestState>);
        app
    }

    #[test]
    fn the_first_update_jumps_to_the_configured_state() {
        let mut app = boot_app(TestState::Playing);
        assert_eq!(
            *app.world().resource::<State<TestState>>().get(),
            TestState::Menu
        );

        app.update();
        app.world_mut().run_schedule(StateTransition);

        assert_eq!(
            *app.world().resource::<State<TestState>>().get(),
            TestState::Playing
        );
    }

    #[test]
    fn it_only_fires_once_so_the_game_can_leave_that_state_again() {
        let mut app = boot_app(TestState::Playing);
        app.update();
        app.world_mut().run_schedule(StateTransition);

        // The game moves on of its own accord.
        app.world_mut()
            .resource_mut::<NextState<TestState>>()
            .set(TestState::Menu);
        app.world_mut().run_schedule(StateTransition);

        app.update();
        app.world_mut().run_schedule(StateTransition);
        assert_eq!(
            *app.world().resource::<State<TestState>>().get(),
            TestState::Menu,
            "the boot jump must not drag the game back every frame"
        );
    }

    #[test]
    fn parsing_is_the_hosts_business_and_bad_names_are_rejected() {
        assert_eq!(TestState::from_str("playing"), Ok(TestState::Playing));
        assert_eq!(TestState::from_str("PLAYING"), Ok(TestState::Playing));
        assert!(TestState::from_str("nonsense").is_err());
    }
}
