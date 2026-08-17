//! The handful of things that differ between a desktop build and a web one.
//!
//! Small on purpose. Almost all of this game is target-agnostic — the
//! simulation, the roles, the UI, the ELO calculator, the parsing — and the
//! places that genuinely are not deserve to be visible in one file rather than
//! scattered as `#[cfg]` through the modules that happen to need them.
//!
//! # Why not `std::time`
//!
//! `wasm32-unknown-unknown` has no clock. `Instant::now()` and
//! `SystemTime::now()` both **panic** there — they do not return an error, and
//! nothing about the call site suggests they might. The port had three
//! open-coded copies of `SystemTime::now().duration_since(UNIX_EPOCH)`, one per
//! module that needed a timestamp, and every one of them was a panic waiting
//! for the web build.
//!
//! [`Instant`] is Bevy's, which is `web_time::Instant` on the web and
//! `std::time::Instant` everywhere else. [`now_unix`] is this file's, because
//! nothing provides a portable wall clock.

/// A monotonic instant that works on the web.
///
/// Re-exported rather than used directly so the reason travels with it:
/// `std::time::Instant::now()` panics on `wasm32-unknown-unknown`, and a
/// session-expiry check is not somewhere to discover that.
pub use bevy::platform::time::Instant;

/// Seconds since the Unix epoch.
///
/// Returns 0 rather than failing. Every caller is stamping a record — a
/// recording's `recordedAt`, a matchmaking ticket — where a zero is a visibly
/// wrong timestamp and a panic is a lost game.
#[cfg(not(target_arch = "wasm32"))]
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds since the Unix epoch, from the browser.
///
/// `Date.now()` is milliseconds and a `f64`; the seconds it divides down to fit
/// an `i64` for another quarter of a billion years.
#[cfg(target_arch = "wasm32")]
pub fn now_unix() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// Milliseconds since the Unix epoch.
///
/// Matchmaking tickets are ordered by this, and two tickets a second apart is
/// not enough resolution to order a queue.
pub fn now_unix_millis() -> i64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sanity, not precision: a clock that returns zero has failed, and one
    /// that returns a 1970 date has failed differently.
    #[test]
    fn the_wall_clock_is_this_century() {
        // 2020-01-01, comfortably in the past and comfortably not zero.
        const Y2020: i64 = 1_577_836_800;
        assert!(now_unix() > Y2020, "got {}", now_unix());
        assert!(now_unix_millis() > Y2020 * 1000);
    }

    #[test]
    fn milliseconds_and_seconds_agree() {
        let millis = now_unix_millis();
        let secs = now_unix();
        assert!(
            (millis / 1000 - secs).abs() <= 1,
            "{millis}ms and {secs}s describe different times"
        );
    }

    #[test]
    fn the_monotonic_clock_moves_forward() {
        let a = Instant::now();
        let b = Instant::now();
        assert!(b >= a);
    }
}
