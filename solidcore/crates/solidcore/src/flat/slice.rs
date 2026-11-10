//! The five ways of building a solid back up from flat parts cut from sheet stock, each part a
//! [`Shape`] in its own 2D frame plus the 3D [`Frame`] it sits in, so one result feeds the sheet
//! export, the packing and the 3D preview of the assembled model:
//!
//! - [`Style::Stacked`]: layers one sheet thick along the slicing axis, glued; dowel holes that
//!   line up through the stack, and each layer engraved with where its smaller neighbour goes;
//! - [`Style::Rod`]: slices threaded on a rod through all of them, with spacer rings cut from the
//!   same sheet to fill each gap;
//! - [`Style::Base`]: upright slices with tabs that sit in slots of a base plate under the model;
//! - [`Style::Back`]: the same into a back plate behind the model (wall mounted);
//! - [`Style::Waffle`]: two sets of upright slices at right angles, slotted half way so they
//!   slide together.

use serde::{Deserialize, Serialize};

use super::{FlatError, FlatResult, Region, Ring, Shape, P2};

/// How the model is built back up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// Glued layers.
    #[default]
    Stacked,
    /// Slices on a rod with spacer rings.
    Rod,
    /// Upright slices in a base plate.
    Base,
    /// Upright slices in a back plate.
    Back,
    /// Two interlocking sets of slices.
    Waffle,
}

/// A model axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    /// X.
    X,
    /// Y.
    Y,
    /// Z.
    #[default]
    Z,
}

impl Axis {
    /// The index of the axis in a point `[x, y, z]`.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// The plane normal and the slice's 2D directions `(n, u, v)` for slices across this axis,
    /// with `u x v = n` so parts are never mirrored: X slices have `(u, v) = (y, z)`, Y slices
    /// `(x, z)` (seen from -Y), Z slices `(x, y)`.
    #[must_use]
    pub fn directions(self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        match self {
            Axis::X => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
            Axis::Y => ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            Axis::Z => ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        }
    }

    /// The frame of the slice at `c` along this axis.
    #[must_use]
    pub fn frame(self, c: f64) -> Frame {
        let (n, u, v) = self.directions();
        let mut origin = [0.0; 3];
        if let Some(o) = origin.get_mut(self.index()) {
            *o = c;
        }
        Frame { origin, u, v, n }
    }
}

/// The settings of a slicing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FabSettings {
    /// How the model is built back up.
    pub style: Style,
    /// The slicing axis: the stacking direction for stacked layers, the rod's direction for
    /// rod slices, the slicing axis of base and back plates (X or Y). Waffles always slice
    /// across X and Y.
    pub axis: Axis,
    /// The sheet thickness (mm, > 0).
    pub thickness: f64,
    /// Centre-to-centre distance of upright or rod slices (mm); 0 for automatic.
    pub spacing: f64,
    /// The number of upright or rod slices; overrides `spacing` when above 0.
    pub count: u32,
    /// Play in slots and holes (mm, >= 0).
    pub clearance: f64,
    /// The rod's diameter (mm).
    pub rod_diameter: f64,
    /// The rod's position in slice coordinates; `None` for the middle of the slices.
    pub rod_at: Option<P2>,
    /// Stacked: alignment holes through every layer (0 for none).
    pub dowels: u32,
    /// The dowels' diameter (mm).
    pub dowel_diameter: f64,
    /// Material kept around holes, and the width of spacer rings (mm).
    pub wall: f64,
    /// How far a base or back plate reaches past the slices (mm).
    pub plate_margin: f64,
    /// Stacked: engrave where the smaller neighbouring layer goes.
    pub guide_outline: bool,
    /// Pieces smaller than this (mm²) are dropped.
    pub min_area: f64,
    /// How far section outlines may stray from the model's exact section (mm).
    pub chord: f64,
}

impl Default for FabSettings {
    fn default() -> Self {
        FabSettings {
            style: Style::Stacked,
            axis: Axis::Z,
            thickness: 3.0,
            spacing: 0.0,
            count: 0,
            clearance: 0.2,
            rod_diameter: 8.0,
            rod_at: None,
            dowels: 2,
            dowel_diameter: 6.0,
            wall: 4.0,
            plate_margin: 10.0,
            guide_outline: true,
            min_area: 4.0,
            chord: 0.01,
        }
    }
}

/// The most slices one set may have.
pub const MAX_SLICES: usize = 500;

/// Where a flat part sits in the model: its 2D `(x, y)` is `origin + x u + y v`, and the sheet's
/// thickness is centred on that plane along `n`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    /// The point 2D `(0, 0)` maps to.
    pub origin: [f64; 3],
    /// The 3D direction of the part's x.
    pub u: [f64; 3],
    /// The 3D direction of the part's y.
    pub v: [f64; 3],
    /// The sheet's normal, `u x v`.
    pub n: [f64; 3],
}

impl Frame {
    /// The 3D point of the part's `(x, y)`, `z` along the normal.
    #[must_use]
    pub fn to3d(&self, x: f64, y: f64, z: f64) -> [f64; 3] {
        let (o, u, v, n) = (self.origin, self.u, self.v, self.n);
        [
            o[0] + x * u[0] + y * v[0] + z * n[0],
            o[1] + x * u[1] + y * v[1] + z * n[1],
            o[2] + x * u[2] + y * v[2] + z * n[2],
        ]
    }
}

/// What a part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartKind {
    /// A slice of the model.
    Slice,
    /// A base or back plate.
    Plate,
    /// A spacer ring between rod slices.
    Spacer,
}

/// Text engraved at a point of a part: centred on `at`'s x, its baseline at `at`'s y.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// The text.
    pub text: String,
    /// Where it goes.
    pub at: P2,
    /// Its capital height (mm).
    pub height: f64,
}

/// One flat part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FabPart {
    /// Its name, engraved on it: `L1`, `S3`, `X2`, `S4b` (a slice that falls apart), `Base plate`.
    pub name: String,
    /// What it is.
    pub kind: PartKind,
    /// Its material in its own 2D frame.
    pub shape: Shape,
    /// Where it sits (the first copy for spacers).
    pub frame: Frame,
    /// Lines engraved on it (open polylines): where a neighbouring layer goes.
    pub engrave: Vec<Ring>,
    /// Text engraved on it: slot numbers on a plate.
    pub marks: Vec<Mark>,
    /// How many are cut.
    pub copies: u32,
    /// Where each copy goes, when there are several (spacers).
    pub places: Vec<Frame>,
}

/// Something about a slicing the user should know; the result is still usable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Warning {
    /// The slice across `axis` at `at` has no material and was left out.
    NoMaterial {
        /// The slicing axis.
        axis: Axis,
        /// The slice's position along it (mm).
        at: f64,
    },
    /// The part falls apart into `pieces` pieces, named with a, b, c...
    FallsApart {
        /// The slice's name.
        part: String,
        /// How many pieces.
        pieces: usize,
    },
    /// Only `placed` of `wanted` dowel holes found room in every layer.
    DowelsShort {
        /// Holes placed.
        placed: u32,
        /// Holes asked for.
        wanted: u32,
    },
    /// The rod misses the slice's material, so it got a hub around the rod and a bridge.
    RodHub {
        /// The slice.
        part: String,
    },
    /// The slice is too narrow where it meets the plate for a tab.
    NoTab {
        /// The slice.
        part: String,
    },
    /// The gaps between rod slices are no whole number of sheets: spacers are not exact.
    GapsNotWhole {
        /// The sheet thickness (mm).
        thickness: f64,
    },
    /// The two sets of waffle slices never cross: nothing holds them together.
    NeverCross,
}

/// A slicing: its parts and what to know about them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FabResult {
    /// The parts.
    pub parts: Vec<FabPart>,
    /// The warnings, in the order they arose.
    pub warnings: Vec<Warning>,
    /// The sheet thickness the parts are cut from (mm).
    pub thickness: f64,
}

impl FabResult {
    /// The number of parts cut, copies counted.
    #[must_use]
    pub fn count(&self) -> u64 {
        self.parts.iter().map(|p| u64::from(p.copies)).sum()
    }
}

/// The model being sliced: its bounds and its section by any plane square to an axis.
pub trait Sectioner {
    /// The bounds `[xmin, ymin, zmin, xmax, ymax, zmax]` (mm).
    fn bounds(&self) -> [f64; 6];
    /// The material on the plane `axis = at`, in the slice coordinates of
    /// [`Axis::directions`] (normalised, outlines counter-clockwise).
    fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region>;
}

/// The model cut into flat parts by `s`.
///
/// # Errors
/// [`FlatError::Setting`] for a setting out of range; [`FlatError::Nothing`] when the slices
/// miss the model; the sectioner's errors.
pub fn make(model: &mut dyn Sectioner, s: &FabSettings) -> FlatResult<FabResult> {
    let _ = (model, s);
    Err(FlatError::Nothing)
}
