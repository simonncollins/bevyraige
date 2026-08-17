//! Rendering a region of game state as text an agent can actually read.
//!
//! # Why text beats a screenshot
//!
//! A screenshot of a platforming level tells you almost nothing useful: whether
//! a gap is crossable, how deep a pit is, or where the ground under a character
//! actually starts are all measurements, and a picture makes you estimate them.
//! The same region as ASCII answers all three by counting characters.
//!
//! During the 13 Aug playtest every trigger placement came off an ASCII map and
//! none came off a screenshot. The obstacle that ruined the first attempt — a
//! floating platform acting as a ceiling — was invisible as a hazard on screen
//! and obvious in text.
//!
//! # What is generic here, and what is not
//!
//! Producing the characters is the game's job: only it knows what a pixel of
//! terrain, a tile, or a reachability score *is*. Everything around that is not,
//! and this is that everything — axis rulers, subsampling, the size cap, the
//! legend. Those were hand-written in throwaway Python four times during the
//! playtest, and got the column-to-coordinate arithmetic wrong once, which
//! produced a garbled map that cost a wasted read.
//!
//! A host supplies a sampler; [`AsciiView::render`] does the rest.
//!
//! # The cap is on output, not on region
//!
//! The obvious cap — refuse regions bigger than N — is the wrong one. It makes
//! the *whole board* unaskable, which is the single most useful view there is.
//! The cap here is on the characters produced, so a large region simply needs a
//! coarser [`step`](AsciiView::step): the whole 1600x720 level fits comfortably
//! at 8px a character. A request over the cap is refused with the step that
//! would work, rather than with a number the caller has to solve for.

use serde_json::Value;

/// Round `a / b` up, for positives.
///
/// `i32::div_ceil` is still unstable; both arguments here are validated positive
/// before they reach it. Rounding *up* rather than down matters: truncating
/// would silently drop the right-hand column and bottom row of a region, which
/// on a board view is the edge of the level.
fn div_ceil(a: i32, b: i32) -> i32 {
    (a + b - 1) / b
}

/// Most characters wide a rendered view may be.
pub const MAX_COLS: i32 = 220;
/// Most character rows a rendered view may be.
pub const MAX_ROWS: i32 = 140;

/// A rectangular region to draw, and how coarsely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsciiView {
    /// Top-left of the region, in the game's own coordinates.
    pub origin: (i32, i32),
    /// Size of the region, in the game's own coordinates.
    pub size: (i32, i32),
    /// Game units per character. 1 is every unit; 8 samples every eighth.
    pub step: i32,
    /// What the characters mean, printed under the grid.
    pub legend: &'static [(char, &'static str)],
}

impl AsciiView {
    /// Columns the rendered grid will have.
    pub fn cols(&self) -> i32 {
        div_ceil(self.size.0, self.step)
    }

    /// Rows the rendered grid will have.
    pub fn rows(&self) -> i32 {
        div_ceil(self.size.1, self.step)
    }

    /// Build from `{"x","y","width","height","step"}`.
    ///
    /// `step` defaults to 1. The error for an oversized request names the step
    /// that would fit, because "too big" without a remedy just costs another
    /// round trip to guess one.
    pub fn from_params(
        params: Option<&Value>,
        legend: &'static [(char, &'static str)],
    ) -> Result<Self, String> {
        let params = params.ok_or("expected {\"x\",\"y\",\"width\",\"height\"}")?;
        let int = |key: &str| -> Result<i32, String> {
            params
                .get(key)
                .and_then(Value::as_i64)
                .map(|v| v as i32)
                .ok_or_else(|| format!("expected an integer for {key:?}"))
        };
        let (width, height) = (int("width")?, int("height")?);
        if width <= 0 || height <= 0 {
            return Err("width and height must be positive".into());
        }
        let step = params
            .get("step")
            .and_then(Value::as_i64)
            .map(|v| v as i32)
            .unwrap_or(1);
        if step <= 0 {
            return Err("step must be positive".into());
        }

        let view = Self {
            origin: (int("x")?, int("y")?),
            size: (width, height),
            step,
            legend,
        };
        if view.cols() > MAX_COLS || view.rows() > MAX_ROWS {
            let needed = div_ceil(width, MAX_COLS).max(div_ceil(height, MAX_ROWS));
            return Err(format!(
                "{}x{} characters is past the {MAX_COLS}x{MAX_ROWS} cap; retry with \"step\": {needed}",
                view.cols(),
                view.rows()
            ));
        }
        Ok(view)
    }

    /// Draw the region, asking `sample` for the character at each point.
    ///
    /// The header states the coordinate mapping outright. The rulers underneath
    /// are a convenience for reading a column off by eye; the header is what
    /// makes a reading *exact*, and is the part whose absence caused an
    /// off-by-a-factor-of-two misread during the playtest.
    pub fn render(&self, mut sample: impl FnMut(i32, i32) -> char) -> String {
        let (cols, rows) = (self.cols(), self.rows());
        // Rows are labelled `y=NNNN `, so the rulers need the same indent.
        let gutter = " ".repeat(7);
        let mut out = format!(
            "x = {} + col*{}   y = {} + row*{}   ({}x{} chars, {} units each)\n",
            self.origin.0, self.step, self.origin.1, self.step, cols, rows, self.step
        );

        for (label, divisor) in [("100s", 100), ("10s", 10)] {
            out.push_str(&gutter);
            for c in 0..cols {
                let x = self.origin.0 + c * self.step;
                out.push(char::from_digit((x.unsigned_abs() / divisor) % 10, 10).unwrap_or('?'));
            }
            out.push_str(&format!("  <- x {label}\n"));
        }

        for r in 0..rows {
            let y = self.origin.1 + r * self.step;
            out.push_str(&format!("y={y:<5} "));
            for c in 0..cols {
                out.push(sample(self.origin.0 + c * self.step, y));
            }
            out.push('\n');
        }

        if !self.legend.is_empty() {
            let entries: Vec<String> = self
                .legend
                .iter()
                .map(|(ch, meaning)| format!("{ch} {meaning}"))
                .collect();
            out.push_str(&format!("legend: {}\n", entries.join("   ")));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEGEND: &[(char, &str)] = &[('#', "solid"), ('.', "air")];

    fn view(width: i32, height: i32, step: i32) -> AsciiView {
        AsciiView {
            origin: (100, 200),
            size: (width, height),
            step,
            legend: LEGEND,
        }
    }

    #[test]
    fn a_step_of_one_draws_every_unit() {
        let v = view(4, 3, 1);
        assert_eq!((v.cols(), v.rows()), (4, 3));
    }

    #[test]
    fn a_coarse_step_subsamples() {
        let v = view(1600, 720, 8);
        assert_eq!((v.cols(), v.rows()), (200, 90));
    }

    /// A region that does not divide evenly by the step still covers all of it —
    /// truncating would silently drop the right-hand edge of the board.
    #[test]
    fn a_region_that_does_not_divide_evenly_is_rounded_up() {
        let v = view(10, 10, 4);
        assert_eq!((v.cols(), v.rows()), (3, 3));
    }

    #[test]
    fn the_header_states_the_mapping_so_a_reading_can_be_exact() {
        let out = view(4, 2, 4).render(|_, _| '.');
        assert!(
            out.starts_with("x = 100 + col*4   y = 200 + row*4"),
            "{out}"
        );
    }

    /// Row labels step by the sample size, not by one — reading a row off as
    /// `y = origin + row` is the mistake the header exists to prevent.
    #[test]
    fn rows_are_labelled_with_their_coordinate_not_their_index() {
        // 15 units tall at 5 a character is three rows.
        let out = view(2, 15, 5).render(|_, _| '.');
        assert!(out.contains("y=200"), "{out}");
        assert!(out.contains("y=205"), "{out}");
        assert!(out.contains("y=210"), "{out}");
        assert!(!out.contains("y=201"), "rows are 5 apart, not 1: {out}");
    }

    #[test]
    fn the_sampler_is_asked_for_stepped_coordinates() {
        let mut asked = Vec::new();
        view(9, 1, 3).render(|x, y| {
            asked.push((x, y));
            '.'
        });
        assert_eq!(asked, vec![(100, 200), (103, 200), (106, 200)]);
    }

    #[test]
    fn the_legend_is_printed() {
        let out = view(2, 2, 1).render(|_, _| '#');
        assert!(out.contains("# solid"), "{out}");
        assert!(out.contains(". air"), "{out}");
    }

    // ── Params ────────────────────────────────────────────────────────────────

    #[test]
    fn params_default_to_a_step_of_one() {
        let v = AsciiView::from_params(Some(&json!({"x":1,"y":2,"width":3,"height":4})), LEGEND)
            .unwrap();
        assert_eq!(v.step, 1);
        assert_eq!(v.origin, (1, 2));
    }

    /// **The cap is on output, not on region.** A 400-a-side region cap makes
    /// the whole board unaskable, which is the most useful view there is.
    #[test]
    fn a_whole_board_is_askable_at_a_coarse_step() {
        let v = AsciiView::from_params(
            Some(&json!({"x":0,"y":0,"width":1600,"height":720,"step":8})),
            LEGEND,
        );
        assert!(v.is_ok(), "the whole level at 8px a char must be allowed");
    }

    /// …and an oversized request is refused with the step that would work,
    /// rather than a number the caller has to solve for.
    #[test]
    fn an_oversized_request_names_the_step_that_would_fit() {
        let err = AsciiView::from_params(
            Some(&json!({"x":0,"y":0,"width":1600,"height":720})),
            LEGEND,
        )
        .expect_err("1600 chars wide is past the cap");
        assert!(err.contains("\"step\": 8"), "{err}");

        // And that suggestion actually works.
        assert!(AsciiView::from_params(
            Some(&json!({"x":0,"y":0,"width":1600,"height":720,"step":8})),
            LEGEND,
        )
        .is_ok());
    }

    #[test]
    fn a_zero_or_negative_size_is_refused() {
        for bad in [
            json!({"x":0,"y":0,"width":0,"height":10}),
            json!({"x":0,"y":0,"width":10,"height":-5}),
        ] {
            assert!(AsciiView::from_params(Some(&bad), LEGEND).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_zero_step_is_refused_rather_than_dividing_by_it() {
        let bad = json!({"x":0,"y":0,"width":10,"height":10,"step":0});
        assert!(AsciiView::from_params(Some(&bad), LEGEND).is_err());
    }

    #[test]
    fn missing_params_are_refused() {
        assert!(AsciiView::from_params(None, LEGEND).is_err());
        assert!(AsciiView::from_params(Some(&json!({"x":1,"y":2})), LEGEND).is_err());
    }
}
