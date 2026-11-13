//! Flat parts for sheet fabrication: a solid cut into parts to laser or CNC from sheet stock
//! (wood, acrylic), finished for the machine and packed onto sheets.
//!
//! Not upstream. The geometry of SolidCraft's "Slice for Laser / CNC":
//! - [`region`]: polygonal regions (outlines with holes, flattened to a chord tolerance) and the
//!   polygon booleans, offsets and clipping done on them;
//! - [`section`]: the section of a B-rep by a plane as a region, from the kernel's exact planar
//!   section curves;
//! - [`slice`]: the five ways of building a solid back up from flat parts (stacked layers,
//!   slices on a rod, on a base plate or a back plate, a waffle grid);
//! - [`finish`]: a part as a machine cuts it (laser kerf, router corner reliefs cut out of its
//!   material) and its engraved labels;
//! - `relief`: the circles of the router's corner reliefs in a part's rings, placed by the wedge
//!   computation of [`crate::dogbone`];
//! - [`font`]: single-stroke lettering for those labels;
//! - [`nest`]: parts packed onto sheets.
//!
//! Regions here are polygons, not the exact lines-arcs-Béziers regions of [`crate::prism`]: each
//! part of a sliced model takes many booleans and offsets (slots, tabs, dowel holes, kerf, label
//! room), some of them offsets that overlap themselves and need non-zero winding, and a polygon
//! engine (iOverlay, MIT OR Apache-2.0) does that work far more cheaply than the kernel's exact
//! profile offset
//! ([`create_offset_profile`](crate::primitive::api_primitives::create_offset_profile)) and exact
//! 2D boolean ([`boolean_2d`](crate::boolean::api_brep::boolean_2d)). Section outlines are made
//! from the exact section curves and stay within the caller's chord tolerance of the exact
//! section.
//!
//! Deviations: none (no upstream counterpart). No recursion.

pub mod finish;
pub mod font;
pub mod nest;
pub mod region;
mod relief;
pub mod section;
pub mod slice;

use serde::{Deserialize, Serialize};

/// A point in a flat part's own 2D frame (mm).
pub type P2 = [f64; 2];

/// A closed ring of points (the closing point not repeated) or an open polyline.
pub type Ring = Vec<P2>;

/// One connected piece of material: its outline (counter-clockwise) and its holes (clockwise).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// The outline, counter-clockwise.
    pub outer: Ring,
    /// The holes, clockwise.
    pub holes: Vec<Ring>,
}

impl Shape {
    /// The outline, then the holes.
    pub fn rings(&self) -> impl Iterator<Item = &Ring> {
        std::iter::once(&self.outer).chain(&self.holes)
    }
}

/// Material in a plane: disjoint shapes.
pub type Region = Vec<Shape>;

/// Why a flat-part computation gave no result.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum FlatError {
    /// A setting out of range or not finite: the setting's name and the range it must be in.
    #[error("`{name}` must be {limit}")]
    Setting {
        /// The setting's name.
        name: &'static str,
        /// The range the setting must be in, as words ("more than 0 and at most 1000 mm").
        limit: &'static str,
    },
    /// The slices miss the model: nothing to cut.
    #[error("nothing to cut: the slices miss the model")]
    Nothing,
    /// Part `part` (`width` x `height` mm, its index in the list given) fits on no sheet.
    #[error("part {part} ({width:.1} x {height:.1} mm) does not fit on the sheet")]
    DoesNotFit {
        /// Index of the part in the list given to the packer.
        part: usize,
        /// Its width (mm).
        width: f64,
        /// Its height (mm).
        height: f64,
    },
    /// More geometry than the budget allows (points, parts, slices).
    #[error("too much geometry: {0}")]
    TooBig(&'static str),
    /// The kernel failed to section the body by the plane `axis = at` (the position asked for,
    /// before any nudge off a vertex).
    #[error("the kernel cannot section the body at {axis} = {at:.3} mm: {error}")]
    Section {
        /// The axis the plane is square to.
        axis: slice::Axis,
        /// Where the plane crosses that axis (mm).
        at: f64,
        /// The kernel's error.
        error: crate::SolidError,
    },
    /// The section of the body by the plane `axis = at` does not close: an outline's ends stay
    /// `gap` mm apart, more than the section curves' ends may be and still join.
    #[error("the section of the body at {axis} = {at:.3} mm does not close: a {gap:.3} mm gap")]
    OpenSection {
        /// The axis the plane is square to.
        axis: slice::Axis,
        /// Where the plane crosses that axis (mm).
        at: f64,
        /// How far apart the open outline's ends are (mm).
        gap: f64,
    },
    /// The kernel failed reading the body (its vertices or its bounding box), or refused a
    /// part's corner reliefs ([`finish::relieve_corners`]).
    #[error(transparent)]
    Kernel(#[from] crate::SolidError),
}

/// The result type of this module.
pub type FlatResult<T> = Result<T, FlatError>;
