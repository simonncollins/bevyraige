//! Advancing the simulation a known number of ticks, and stopping.
//!
//! # Why an agent needs this and a player does not
//!
//! A player watches. An agent *inspects*, and inspection over a network takes
//! time the simulation does not stop for. Ask where the entities are, then ask
//! what the terrain under them looks like, and the two answers describe two
//! different worlds — with no way to tell from the answers themselves.
//!
//! That is not hypothetical. During the 13 Aug playtest a position read and a
//! terrain read taken a few seconds apart looked exactly like a physics bug —
//! nincompoops in state `Walking` standing on empty air. They were not. The sim
//! had run 3,700 ticks between the two calls, passed the stalemate cutoff,
//! force-bombed everyone, and the explosions had destroyed the ground they had
//! been standing on. Two correct readings, one wrong conclusion.
//!
//! Pausing first fixes the race but not the diagnosis: a paused world still
//! cannot say *which* tick it is showing, so a finding cannot be replayed.
//! Stepping fixes both. Advance exactly `n`, and every observation afterwards
//! carries a tick number and stands still while you take it.
//!
//! # Why `FixedUpdate` specifically
//!
//! Because that is what a headless conformance harness runs to execute a
//! scenario — see `docs/harness.md`. Stepping the game the same way the fixture
//! suite steps it means a bug found by an agent poking at a live match can be
//! turned into a scenario
//! that reproduces it, and the two will agree tick for tick. A more "faithful"
//! choice that diverged from the fixtures would be worth less.
//!
//! A game whose simulation lives in a different schedule calls [`step`] with its
//! own label from its own bridge verb.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

/// Most ticks one request may advance.
///
/// A full match is 1000 ticks, so this is ten of them. The cap exists because
/// stepping blocks the request until it finishes: a fat-fingered `1e9` would
/// hang the server with no way to interrupt it, and "the agent typed too many
/// zeroes" should cost an error message rather than a restart.
pub const MAX_STEP_TICKS: u32 = 10_000;

/// Run `schedule` exactly `ticks` times with the clock held still.
///
/// **Pauses `Time<Virtual>` and leaves it paused.** Stepping and a running
/// clock are two things driving the same simulation, and the point of stepping
/// is to be the only one. Resuming is the caller's business — in this project
/// that means the game's own speed control, which the next change to it applies.
///
/// Returns the number of ticks actually run, which is `ticks` unless the
/// schedule does not exist.
pub fn step(world: &mut World, ticks: u32, schedule: impl ScheduleLabel + Clone) -> u32 {
    if let Some(mut time) = world.get_resource_mut::<Time<Virtual>>() {
        time.pause();
    }
    let mut run = 0;
    for _ in 0..ticks {
        if world.try_run_schedule(schedule.clone()).is_err() {
            break;
        }
        run += 1;
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Ticks(u32);

    fn counting_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<Ticks>();
        app.add_systems(FixedUpdate, |mut t: ResMut<Ticks>| t.0 += 1);
        app
    }

    #[test]
    fn stepping_runs_exactly_the_ticks_asked_for() {
        let mut app = counting_app();
        let run = step(app.world_mut(), 7, FixedUpdate);
        assert_eq!(run, 7);
        assert_eq!(
            app.world().resource::<Ticks>().0,
            7,
            "seven, not six or eight"
        );
    }

    #[test]
    fn stepping_zero_ticks_does_nothing() {
        let mut app = counting_app();
        assert_eq!(step(app.world_mut(), 0, FixedUpdate), 0);
        assert_eq!(app.world().resource::<Ticks>().0, 0);
    }

    /// The whole point: after stepping, nothing moves until asked again. An
    /// `app.update()` with the clock paused must not sneak in extra ticks.
    #[test]
    fn the_clock_is_stopped_so_nothing_advances_between_observations() {
        let mut app = counting_app();
        step(app.world_mut(), 3, FixedUpdate);
        assert!(app.world().resource::<Time<Virtual>>().is_paused());

        for _ in 0..20 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<Ticks>().0,
            3,
            "twenty frames must not advance a stepped simulation"
        );
    }

    /// Two steps compose, so a caller can advance in stages and inspect between.
    #[test]
    fn steps_accumulate() {
        let mut app = counting_app();
        step(app.world_mut(), 4, FixedUpdate);
        step(app.world_mut(), 6, FixedUpdate);
        assert_eq!(app.world().resource::<Ticks>().0, 10);
    }

    /// A missing schedule stops rather than looping to the cap, and reports how
    /// far it got — silently returning the requested count would be a lie.
    #[test]
    fn a_missing_schedule_reports_zero_rather_than_pretending() {
        #[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
        struct Nonexistent;

        let mut app = counting_app();
        assert_eq!(step(app.world_mut(), 5, Nonexistent), 0);
        assert_eq!(app.world().resource::<Ticks>().0, 0);
    }
}
