//! Flat parts packed onto sheets for cutting.

use serde::{Deserialize, Serialize};

use super::{FlatError, FlatResult, Ring};

/// Where one part goes: turned `angle` degrees counter-clockwise about its own origin first,
/// then moved by `(dx, dy)`; the sheet's origin is its lower-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// The part's index in the list given.
    pub part: usize,
    /// The sheet, from 0.
    pub sheet: usize,
    /// 0, 90, 180 or 270.
    pub angle: u16,
    /// Moved along x after turning (mm).
    pub dx: f64,
    /// Moved along y after turning (mm).
    pub dy: f64,
}

/// One ring of a part as it lies on its sheet: turned, then moved.
#[must_use]
pub fn transform(ring: &[[f64; 2]], p: &Placement) -> Ring {
    let _ = p;
    ring.to_vec()
}

/// Every part placed once, on as few `sheet_w` x `sheet_h` sheets as it takes. Each part is its
/// outline (holes do not matter); parts end at least `gap` apart and at least `margin` from the
/// sheet's edges; `rotate` allows turns by 90°. One placement per part, in part order.
///
/// # Errors
/// [`FlatError::Setting`] for a sheet with no room inside its margin, a negative gap or a part
/// without outline; [`FlatError::DoesNotFit`] for a part no empty sheet takes;
/// [`FlatError::TooBig`] past the part budget.
pub fn nest(
    outlines: &[Ring],
    sheet_w: f64,
    sheet_h: f64,
    gap: f64,
    margin: f64,
    rotate: bool,
) -> FlatResult<Vec<Placement>> {
    let _ = (outlines, sheet_w, sheet_h, gap, margin, rotate);
    Err(FlatError::Setting("sheet"))
}
