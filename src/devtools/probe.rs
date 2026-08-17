//! Recording and comparing simulation state, tick by tick.
//!
//! Two jobs that are really one:
//!
//! - **Following** a run: what was each entity doing on tick 37?
//! - **Trusting** a run: do two runs of the same inputs agree, and if not,
//!   exactly where do they first differ?
//!
//! A determinism bug reported as "the replay desynced" is nearly useless; the
//! same bug reported as "tick 41, entity 3, `pos_y` 552 vs 564" usually names
//! the system responsible. [`Probe::first_divergence`] is the whole point of
//! this module.
//!
//! Game-agnostic: what a snapshot *contains* is the host's business. It records
//! lines of text, compares them, and writes them out.

use std::fmt::Write as _;
use std::path::Path;

use bevy::prelude::*;

/// One tick's worth of recorded state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeFrame {
    pub tick: u64,
    /// One line per thing worth watching. Order must be deterministic, or
    /// every comparison is noise — sort by a stable key before recording.
    pub lines: Vec<String>,
}

/// Where two runs first disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub tick: u64,
    /// Index into the frame's lines, if both runs recorded that far.
    pub line: usize,
    pub left: Option<String>,
    pub right: Option<String>,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "tick {} line {}:\n  left:  {}\n  right: {}",
            self.tick,
            self.line,
            self.left.as_deref().unwrap_or("<missing>"),
            self.right.as_deref().unwrap_or("<missing>"),
        )
    }
}

/// Recorded frames for one run.
#[derive(Resource, Debug, Default, Clone)]
pub struct Probe {
    pub frames: Vec<ProbeFrame>,
    /// Stop recording past this many frames. 0 means unlimited.
    ///
    /// A probe left on during a long session otherwise grows without bound;
    /// the cap makes it safe to install unconditionally.
    pub limit: usize,
}

impl Probe {
    /// A probe that records at most `limit` frames.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            frames: Vec::new(),
            limit,
        }
    }

    /// Whether another frame would be recorded.
    pub fn accepting(&self) -> bool {
        self.limit == 0 || self.frames.len() < self.limit
    }

    /// Records one tick, if the limit allows.
    pub fn record(&mut self, tick: u64, lines: Vec<String>) {
        if self.accepting() {
            self.frames.push(ProbeFrame { tick, lines });
        }
    }

    /// The frame recorded for `tick`, if any.
    pub fn frame(&self, tick: u64) -> Option<&ProbeFrame> {
        self.frames.iter().find(|f| f.tick == tick)
    }

    /// The first point at which this run and `other` disagree.
    ///
    /// Compares frame by frame in recorded order, then line by line within a
    /// frame. A run that simply stops earlier than the other diverges at the
    /// first tick the shorter one lacks — a run ending early *is* a difference,
    /// and silently ignoring it is how a desync hides.
    pub fn first_divergence(&self, other: &Probe) -> Option<Divergence> {
        for (i, left) in self.frames.iter().enumerate() {
            let Some(right) = other.frames.get(i) else {
                return Some(Divergence {
                    tick: left.tick,
                    line: 0,
                    left: Some(format!("<{} lines>", left.lines.len())),
                    right: None,
                });
            };
            if left.tick != right.tick {
                return Some(Divergence {
                    tick: left.tick,
                    line: 0,
                    left: Some(format!("tick {}", left.tick)),
                    right: Some(format!("tick {}", right.tick)),
                });
            }
            let longest = left.lines.len().max(right.lines.len());
            for line in 0..longest {
                let l = left.lines.get(line);
                let r = right.lines.get(line);
                if l != r {
                    return Some(Divergence {
                        tick: left.tick,
                        line,
                        left: l.cloned(),
                        right: r.cloned(),
                    });
                }
            }
        }
        if other.frames.len() > self.frames.len() {
            let extra = &other.frames[self.frames.len()];
            return Some(Divergence {
                tick: extra.tick,
                line: 0,
                left: None,
                right: Some(format!("<{} lines>", extra.lines.len())),
            });
        }
        None
    }

    /// The whole recording as text, one tick per block.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for frame in &self.frames {
            let _ = writeln!(out, "-- tick {} --", frame.tick);
            for line in &frame.lines {
                let _ = writeln!(out, "{line}");
            }
        }
        out
    }

    /// Writes [`Probe::to_text`] to a file.
    pub fn write_to(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, self.to_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(frames: &[(u64, &[&str])]) -> Probe {
        let mut p = Probe::default();
        for (tick, lines) in frames {
            p.record(*tick, lines.iter().map(|s| s.to_string()).collect());
        }
        p
    }

    #[test]
    fn identical_runs_do_not_diverge() {
        let a = probe(&[(0, &["n0 x=100", "n1 x=200"]), (1, &["n0 x=108"])]);
        let b = a.clone();
        assert_eq!(a.first_divergence(&b), None);
    }

    #[test]
    fn a_divergence_names_the_tick_the_line_and_both_values() {
        let a = probe(&[(0, &["n0 x=100"]), (1, &["n0 x=108"])]);
        let b = probe(&[(0, &["n0 x=100"]), (1, &["n0 x=116"])]);

        let d = a.first_divergence(&b).expect("these disagree");
        assert_eq!(d.tick, 1);
        assert_eq!(d.line, 0);
        assert_eq!(d.left.as_deref(), Some("n0 x=108"));
        assert_eq!(d.right.as_deref(), Some("n0 x=116"));
    }

    #[test]
    fn it_reports_the_first_divergence_not_the_last() {
        let a = probe(&[(0, &["a"]), (1, &["b"]), (2, &["c"])]);
        let b = probe(&[(0, &["a"]), (1, &["X"]), (2, &["Y"])]);
        assert_eq!(a.first_divergence(&b).unwrap().tick, 1);
    }

    #[test]
    fn a_missing_line_within_a_tick_is_a_divergence() {
        // One run lost an entity: same tick, fewer lines.
        let a = probe(&[(0, &["n0", "n1"])]);
        let b = probe(&[(0, &["n0"])]);
        let d = a.first_divergence(&b).expect("an entity vanished");
        assert_eq!(d.line, 1);
        assert_eq!(d.left.as_deref(), Some("n1"));
        assert_eq!(d.right, None);
    }

    /// A run that stops early is a desync, not a shorter recording.
    #[test]
    fn a_run_ending_early_diverges_in_both_directions() {
        let long = probe(&[(0, &["a"]), (1, &["b"])]);
        let short = probe(&[(0, &["a"])]);

        let d = long
            .first_divergence(&short)
            .expect("the short run stopped");
        assert_eq!(d.tick, 1);
        assert_eq!(d.right, None);

        let d = short.first_divergence(&long).expect("still a divergence");
        assert_eq!(d.tick, 1);
        assert_eq!(d.left, None);
    }

    #[test]
    fn a_skipped_tick_is_caught_even_when_the_lines_match() {
        let a = probe(&[(0, &["a"]), (1, &["a"])]);
        let b = probe(&[(0, &["a"]), (2, &["a"])]);
        let d = a.first_divergence(&b).expect("tick 1 vs tick 2");
        assert_eq!(d.tick, 1);
    }

    #[test]
    fn the_limit_caps_growth_so_a_probe_is_safe_to_leave_on() {
        let mut p = Probe::with_limit(2);
        for tick in 0..10 {
            p.record(tick, vec!["x".to_string()]);
        }
        assert_eq!(p.frames.len(), 2);
        assert!(!p.accepting());
    }

    #[test]
    fn an_unlimited_probe_keeps_recording() {
        let mut p = Probe::default();
        for tick in 0..100 {
            p.record(tick, vec!["x".to_string()]);
        }
        assert_eq!(p.frames.len(), 100);
    }

    #[test]
    fn frames_are_addressable_by_tick() {
        let p = probe(&[(0, &["a"]), (7, &["b"])]);
        assert_eq!(p.frame(7).unwrap().lines, vec!["b".to_string()]);
        assert!(p.frame(3).is_none());
    }

    #[test]
    fn the_text_dump_is_readable_and_ordered() {
        let p = probe(&[(0, &["n0 x=100"]), (1, &["n0 x=108"])]);
        let text = p.to_text();
        assert_eq!(text, "-- tick 0 --\nn0 x=100\n-- tick 1 --\nn0 x=108\n");
    }

    #[test]
    fn a_divergence_prints_both_sides() {
        let d = Divergence {
            tick: 4,
            line: 2,
            left: Some("n0 x=1".into()),
            right: None,
        };
        let shown = d.to_string();
        assert!(shown.contains("tick 4"));
        assert!(shown.contains("n0 x=1"));
        assert!(shown.contains("<missing>"));
    }
}
