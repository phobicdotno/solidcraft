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
//!
//! Deviations: none (no upstream counterpart). No recursion.

use serde::{Deserialize, Serialize};

use super::finish::LABEL_MARGIN;
use super::region::{self, Join};
use super::section;
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

/// The axis's letter, as in the warnings ("X").
impl std::fmt::Display for Axis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        })
    }
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
    /// How far a base or back plate reaches past the slices (mm), at least; the plate grows to
    /// hold the slice numbers.
    pub plate_margin: f64,
    /// Stacked: engrave where the smaller neighbouring layer goes.
    pub guide_outline: bool,
    /// Pieces smaller than this (mm²) are dropped.
    pub min_area: f64,
    /// How far section outlines may stray from the model's exact section (mm).
    pub chord: f64,
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
    /// The gaps between rod slices are not a whole number of sheets: spacers are not exact.
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

/// The most dowel holes a stack may have.
pub const MAX_DOWELS: u32 = 64;

/// The most spacer rings a rod may carry.
pub const MAX_SPACERS: u64 = 100_000;

/// The most parts one slicing may give.
pub const MAX_PARTS: usize = 20_000;

/// The largest size (mm) of a length setting: ten metres.
const MAX_LENGTH: f64 = 10_000.0;

/// How close (mm) to a whole number of sheets a gap between rod slices must be for its
/// spacers to count as exact: half a millimetre, about what sheet stock varies by and the
/// play on the rod takes up; past it the stack comes out visibly short or long.
const GAP_SLACK: f64 = 0.5;

/// How far (mm) inside a layer's own edge a guide line may be engraved: a neighbour's outline
/// that runs along the edge is left out rather than cut into the edge itself.
const GUIDE_INSET: f64 = 0.3;

/// What (mm) the material round the rod may fall short of the hub's radius and still hold the
/// rod without a hub: rounding of the offset outline, not a real lack of material.
const HUB_SLACK: f64 = 0.01;

/// The steps across the larger side of the grid the dowel points are picked from (41 x 41
/// points): fine enough to spread a few dowels, coarse enough that the farthest-point pick
/// stays quick.
const DOWEL_GRID: u32 = 40;

/// The height of the band whose width makes a plated slice's foot, in sheets, or this share
/// of the slice's height, whichever is more: enough that a round bottom gives a foot wider
/// than the point it touches at, little enough to follow the slice's own bottom.
const FOOT_SHEETS: f64 = 3.0;
/// See [`FOOT_SHEETS`].
const FOOT_SHARE: f64 = 0.15;

/// A foot this many sheets wide carries two tabs, at its quarters: two tabs of
/// [`TAB_SHEETS`] then leave a sheet between them and half a sheet at each end.
const TWO_TABS_SHEETS: f64 = 6.0;

/// A tab is at least this many sheets wide, so it holds in its slot, or this share of the foot
/// when that is more, at most [`TAB_MAX`] mm, so a wide slice's slots stay short in the plate.
const TAB_SHEETS: f64 = 2.0;
/// See [`TAB_SHEETS`].
const TAB_SHARE: f64 = 0.2;
/// See [`TAB_SHEETS`].
const TAB_MAX: f64 = 40.0;

/// Stretches of material along a waffle line closer than this (mm, a micrometre) are one
/// stretch: pieces that touch meet with rounding gaps far below any cut.
const SPAN_MERGE: f64 = 1e-3;

/// Checks every setting: finite and in range.
///
/// # Errors
/// [`FlatError::Setting`] naming the first setting out of range: `thickness` (above 0, at
/// most 1000 mm), `spacing` (0 to 10 m), `count` (at most [`MAX_SLICES`]), `clearance`
/// (0 to the thickness), `rod_diameter` and `dowel_diameter` (above 0, at most 10 m), `rod_at`
/// (finite, within a kilometre), `dowels` (at most [`MAX_DOWELS`]), `wall` and
/// `plate_margin` (0 to 10 m), `min_area` (0 to 100 m²), `chord` (1e-4 to 10 mm), `axis`
/// for a base or back plate sliced across Z, and `wall` for rod slices when it is at most half
/// the clearance (no spacer ring).
pub fn validate(s: &FabSettings) -> FlatResult<()> {
    let within = |x: f64, lo: f64, hi: f64| x.is_finite() && x >= lo && x <= hi;
    let positive = |x: f64, hi: f64| x.is_finite() && x > 0.0 && x <= hi;
    let checks: [(bool, &'static str, &'static str); 12] = [
        (
            positive(s.thickness, 1000.0),
            "thickness",
            "more than 0 and at most 1000 mm",
        ),
        (
            within(s.spacing, 0.0, MAX_LENGTH),
            "spacing",
            "0 (automatic) to 10000 mm",
        ),
        (s.count as usize <= MAX_SLICES, "count", "0 to 500"),
        (
            within(s.clearance, 0.0, s.thickness),
            "clearance",
            "0 to the sheet thickness",
        ),
        (
            positive(s.rod_diameter, MAX_LENGTH),
            "rod_diameter",
            "more than 0 and at most 10000 mm",
        ),
        (
            s.rod_at
                .is_none_or(|p| p.iter().all(|c| c.abs() < region::KM)),
            "rod_at",
            "within 1000000 mm of the origin on every axis",
        ),
        (s.dowels <= MAX_DOWELS, "dowels", "0 to 64"),
        (
            positive(s.dowel_diameter, MAX_LENGTH),
            "dowel_diameter",
            "more than 0 and at most 10000 mm",
        ),
        (within(s.wall, 0.0, MAX_LENGTH), "wall", "0 to 10000 mm"),
        (
            within(s.plate_margin, 0.0, MAX_LENGTH),
            "plate_margin",
            "0 to 10000 mm",
        ),
        (
            within(s.min_area, 0.0, 1e8),
            "min_area",
            "0 to 100000000 mm²",
        ),
        (
            within(s.chord, section::CHORD_RANGE.0, section::CHORD_RANGE.1),
            "chord",
            section::CHORD_LIMIT,
        ),
    ];
    if let Some(&(_, name, limit)) = checks.iter().find(|(ok, ..)| !ok) {
        return Err(FlatError::Setting { name, limit });
    }
    if matches!(s.style, Style::Base | Style::Back) && s.axis == Axis::Z {
        return Err(FlatError::Setting {
            name: "axis",
            limit: "X or Y for a base or back plate",
        });
    }
    // a spacer ring is `wall` wide around a bore half the clearance wider than the rod: a
    // wall of at most that leaves no ring
    if s.style == Style::Rod && s.wall <= s.clearance / 2.0 {
        return Err(FlatError::Setting {
            name: "wall",
            limit: "more than half the clearance for slices on a rod",
        });
    }
    Ok(())
}

/// The centres of the slices along an axis the model spans from `lo` to `hi`: for stacked
/// layers one sheet apart, the nearest whole number of layers, at least one, centred on the
/// body; otherwise `count` evenly over the span, or `spacing` apart (automatic: four sheets or
/// an eighth of the span, whichever is more, rounded to whole sheets for rod slices so their
/// spacers fit exactly) centred on the span.
///
/// # Errors
/// [`FlatError::Setting`] for a span or thickness that is not finite, and for upright or rod
/// slices less than a sheet apart (`count` or `spacing`);
/// [`FlatError::TooBig`] for more than [`MAX_SLICES`] slices.
pub fn positions(lo: f64, hi: f64, s: &FabSettings, stacked: bool) -> FlatResult<Vec<f64>> {
    let (length, t) = (hi - lo, s.thickness);
    if !(lo.is_finite() && hi.is_finite() && length.is_finite() && length >= 0.0) {
        return Err(FlatError::Setting {
            name: "model bounds",
            limit: "finite",
        });
    }
    if !(t.is_finite() && t > 0.0) {
        return Err(FlatError::Setting {
            name: "thickness",
            limit: "more than 0",
        });
    }
    let too_many = FlatError::TooBig("more slices than allowed");
    if stacked {
        let n = (length / t).round_ties_even().max(1.0);
        if !n.is_finite() || n > MAX_SLICES as f64 {
            return Err(too_many);
        }
        let first = lo + (length - n * t) / 2.0;
        return Ok((0..n as usize)
            .map(|i| first + (i as f64 + 0.5) * t)
            .collect());
    }
    if s.count > 0 {
        if s.count as usize > MAX_SLICES {
            return Err(too_many);
        }
        let n = f64::from(s.count);
        if s.count > 1 && length / n < t {
            return Err(FlatError::Setting {
                name: "count",
                limit: "at most the body's length divided by the sheet thickness",
            });
        }
        return Ok((0..s.count)
            .map(|i| lo + length * (f64::from(i) + 0.5) / n)
            .collect());
    }
    if s.spacing > 0.0 && s.spacing < t {
        return Err(FlatError::Setting {
            name: "spacing",
            limit: "at least the sheet thickness",
        });
    }
    let step = if s.spacing > 0.0 {
        s.spacing
    } else if s.style == Style::Rod {
        // whole sheets, so the spacers between neighbouring slices come out exact; four
        // sheets at least, so the rounding never brings it under four
        let auto = (4.0 * t).max(length / 8.0);
        t * (auto / t).round()
    } else {
        (4.0 * t).max(length / 8.0)
    };
    let n = (((length - t) / step).floor() + 1.0).max(1.0);
    if !n.is_finite() || n > MAX_SLICES as f64 {
        return Err(too_many);
    }
    let first = (lo + hi) / 2.0 - (n - 1.0) / 2.0 * step;
    Ok((0..n as usize).map(|i| first + i as f64 * step).collect())
}

/// The model cut into flat parts by `s`.
///
/// # Errors
/// [`FlatError::Setting`] for a setting out of range (see [`validate`]);
/// [`FlatError::Nothing`] when the model's bounds are not finite or the slices miss it;
/// [`FlatError::TooBig`] past [`MAX_SLICES`], [`MAX_PARTS`], [`MAX_SPACERS`],
/// [`section::MAX_SECTION_SAMPLES`] or for a model over a kilometre; the sectioner's errors.
pub fn make(model: &mut dyn Sectioner, s: &FabSettings) -> FlatResult<FabResult> {
    validate(s)?;
    let b = model_bounds(&*model)?;
    let mut out = Out::default();
    match s.style {
        Style::Stacked => stacked(model, s, b, &mut out)?,
        Style::Rod => rod(model, s, b, &mut out)?,
        Style::Base => plated(model, s, b, false, &mut out)?,
        Style::Back => plated(model, s, b, true, &mut out)?,
        Style::Waffle => waffle(model, s, b, &mut out)?,
    }
    if out.parts.is_empty() {
        return Err(FlatError::Nothing);
    }
    Ok(FabResult {
        parts: out.parts,
        warnings: out.warnings,
        thickness: s.thickness,
    })
}

/// The model's bounds, checked: finite and not inside out (else nothing to cut), within a
/// kilometre of the origin.
fn model_bounds(model: &dyn Sectioner) -> FlatResult<[f64; 6]> {
    let b = model.bounds();
    let ordered = (0..3).all(|i| match (b.get(i), b.get(i + 3)) {
        (Some(lo), Some(hi)) => lo <= hi,
        _ => false,
    });
    if !(b.iter().all(|x| x.is_finite()) && ordered) {
        return Err(FlatError::Nothing);
    }
    if b.iter().any(|x| x.abs() >= region::KM) {
        return Err(FlatError::TooBig("the model reaches past a kilometre"));
    }
    Ok(b)
}

/// The span `(lo, hi)` of bounds `b` along `axis`.
fn extent(b: [f64; 6], axis: Axis) -> (f64, f64) {
    let i = axis.index();
    (
        b.get(i).copied().unwrap_or(0.0),
        b.get(i + 3).copied().unwrap_or(0.0),
    )
}

/// The parts and warnings gathered so far.
#[derive(Default)]
struct Out {
    parts: Vec<FabPart>,
    warnings: Vec<Warning>,
}

/// A slice that cuts material: its name, its position along the axis and its section.
struct Cut {
    name: String,
    at: f64,
    region: Region,
}

/// The slices at `cs` across `axis` that cut material, named `prefix1`, `prefix2`... in
/// order; those that miss the model get a [`Warning::NoMaterial`].
fn slices(
    model: &mut dyn Sectioner,
    axis: Axis,
    cs: &[f64],
    s: &FabSettings,
    out: &mut Out,
    prefix: &str,
) -> FlatResult<Vec<Cut>> {
    let mut cuts = Vec::new();
    for &c in cs {
        let raw = model.section(axis, c)?;
        if region::point_count(&raw) > section::MAX_SECTION_SAMPLES {
            return Err(FlatError::TooBig("points in a section"));
        }
        let reg = region::normalize(&raw);
        if region::pieces(&reg, s.min_area).is_empty() {
            out.warnings.push(Warning::NoMaterial { axis, at: c });
            continue;
        }
        cuts.push(Cut {
            name: format!("{prefix}{}", cuts.len() + 1),
            at: c,
            region: reg,
        });
    }
    Ok(cuts)
}

/// The letters naming piece `i` of a part that falls apart: a..z, then aa, ab...
fn piece_suffix(i: usize) -> String {
    let mut n = i;
    let mut letters = Vec::new();
    loop {
        letters.push(char::from(b'a' + (n % 26) as u8));
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    letters.iter().rev().collect()
}

/// Adds the pieces of `reg` of at least `min_area` as parts named `name`, or `name` with a,
/// b, c... (largest first) and a [`Warning::FallsApart`] when there are several.
fn add_pieces(
    out: &mut Out,
    name: &str,
    kind: PartKind,
    reg: &[Shape],
    frame: Frame,
    s: &FabSettings,
    engrave: &[Ring],
    marks: &[Mark],
) -> FlatResult<()> {
    let pieces = region::pieces(reg, s.min_area);
    if out.parts.len() + pieces.len() > MAX_PARTS {
        return Err(FlatError::TooBig("more parts than allowed"));
    }
    let several = pieces.len() > 1;
    for (i, shape) in pieces.iter().enumerate() {
        let nm = if several {
            format!("{name}{}", piece_suffix(i))
        } else {
            name.to_string()
        };
        out.parts.push(FabPart {
            name: nm,
            kind,
            shape: shape.clone(),
            frame,
            engrave: engrave.to_vec(),
            marks: marks.to_vec(),
            copies: 1,
            places: Vec::new(),
        });
    }
    if several {
        out.warnings.push(Warning::FallsApart {
            part: name.to_string(),
            pieces: pieces.len(),
        });
    }
    Ok(())
}

fn round_join(s: &FabSettings) -> Join {
    Join::Round { chord: s.chord }
}

// ---- stacked --------------------------------------------------------------------------------

/// Layers one sheet thick along `s.axis`, with dowel holes where every layer has room and the
/// outline of each smaller neighbour engraved as a placement guide.
fn stacked(
    model: &mut dyn Sectioner,
    s: &FabSettings,
    b: [f64; 6],
    out: &mut Out,
) -> FlatResult<()> {
    let (lo, hi) = extent(b, s.axis);
    let layers = slices(model, s.axis, &positions(lo, hi, s, true)?, s, out, "L")?;
    let holes = dowel_holes(&layers, s, out);
    let r = s.dowel_diameter / 2.0 + s.clearance / 2.0;
    let discs: Vec<Region> = holes.iter().map(|&p| region::disk(p, r, s.chord)).collect();
    let discs = region::union_all(&discs);
    let areas: Vec<f64> = layers.iter().map(|l| region::area(&l.region)).collect();
    for (i, layer) in layers.iter().enumerate() {
        let cut = if discs.is_empty() {
            layer.region.clone()
        } else {
            region::difference(&layer.region, &discs)
        };
        let mut guide = Vec::new();
        if s.guide_outline {
            let here = areas.get(i).copied().unwrap_or(0.0);
            let inside = region::offset(&cut, -GUIDE_INSET, round_join(s));
            for j in [i.wrapping_sub(1), i + 1] {
                if let (Some(n), Some(&a)) = (layers.get(j), areas.get(j)) {
                    if a < here {
                        guide.extend(region::clip_lines(&closed_lines(&n.region), &inside));
                    }
                }
            }
        }
        let frame = s.axis.frame(layer.at);
        add_pieces(
            out,
            &layer.name,
            PartKind::Slice,
            &cut,
            frame,
            s,
            &guide,
            &[],
        )?;
    }
    Ok(())
}

/// The region's rings as closed polylines (the first point repeated at the end).
fn closed_lines(r: &[Shape]) -> Vec<Ring> {
    r.iter()
        .flat_map(Shape::rings)
        .filter_map(|ring| {
            let first = *ring.first()?;
            let mut line = ring.clone();
            line.push(first);
            Some(line)
        })
        .collect()
}

/// The dowel hole centres: spread over the material every layer shares, kept a wall and the
/// dowel's radius from its edges; a [`Warning::DowelsShort`] when fewer fit than asked for.
fn dowel_holes(layers: &[Cut], s: &FabSettings, out: &mut Out) -> Vec<P2> {
    let Some((first, rest)) = layers.split_first() else {
        return Vec::new();
    };
    if s.dowels == 0 {
        return Vec::new();
    }
    let mut common = first.region.clone();
    for l in rest {
        common = region::intersection(&common, &l.region);
    }
    let safe = region::offset(&common, -(s.dowel_diameter / 2.0 + s.wall), round_join(s));
    let holes = spread_points(&safe, s.dowels as usize);
    if holes.len() < s.dowels as usize {
        out.warnings.push(Warning::DowelsShort {
            placed: holes.len() as u32,
            wanted: s.dowels,
        });
    }
    holes
}

/// Up to `n` points inside the region, as far apart as possible: the points of a 41 x 41 grid
/// ([`DOWEL_GRID`]) over its bounds that lie inside; for one point the one nearest their centroid, else the
/// one farthest from it, then each next the one farthest from those picked (ties go to the
/// first in grid order).
pub fn spread_points(r: &[Shape], n: usize) -> Vec<P2> {
    let Some([x0, y0, x1, y1]) = region::bounds(r) else {
        return Vec::new();
    };
    if n == 0 {
        return Vec::new();
    }
    let step = match (x1 - x0).max(y1 - y0) / f64::from(DOWEL_GRID) {
        st if st > 0.0 && st.is_finite() => st,
        _ => 1.0,
    };
    let mut cand: Vec<P2> = Vec::new();
    for i in 0..=DOWEL_GRID {
        let y = y0 + f64::from(i) * step;
        if y > y1 {
            break;
        }
        for j in 0..=DOWEL_GRID {
            let x = x0 + f64::from(j) * step;
            if x > x1 {
                break;
            }
            if region::contains(r, [x, y]) {
                cand.push([x, y]);
            }
        }
    }
    if cand.is_empty() {
        return Vec::new();
    }
    let m = cand.len() as f64;
    let c = [
        cand.iter().map(|p| p[0]).sum::<f64>() / m,
        cand.iter().map(|p| p[1]).sum::<f64>() / m,
    ];
    let d2 = |p: P2, q: P2| (p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2);
    let pick = |score: &dyn Fn(P2) -> f64| -> Option<(P2, f64)> {
        let mut best: Option<(P2, f64)> = None;
        for &p in &cand {
            let v = score(p);
            if best.is_none_or(|(_, b)| v > b) {
                best = Some((p, v));
            }
        }
        best
    };
    if n == 1 {
        return pick(&|p| -d2(p, c)).into_iter().map(|(p, _)| p).collect();
    }
    let mut picked: Vec<P2> = pick(&|p| d2(p, c)).into_iter().map(|(p, _)| p).collect();
    while picked.len() < n {
        let nearest = |p: P2| {
            picked
                .iter()
                .map(|&q| d2(p, q))
                .fold(f64::INFINITY, f64::min)
        };
        match pick(&nearest) {
            Some((p, v)) if v >= 1e-6 => picked.push(p),
            _ => break,
        }
    }
    picked
}

// ---- rod ------------------------------------------------------------------------------------

/// Slices across `s.axis` with a hole for a rod; a slice whose material misses the rod gets a
/// hub around it and bridges to its pieces. Spacer rings fill the gaps between slices.
fn rod(model: &mut dyn Sectioner, s: &FabSettings, b: [f64; 6], out: &mut Out) -> FlatResult<()> {
    let axis = s.axis;
    let (lo, hi) = extent(b, axis);
    let cuts = slices(model, axis, &positions(lo, hi, s, false)?, s, out, "S")?;
    if cuts.is_empty() {
        return Ok(());
    }
    let centre = s.rod_at.unwrap_or_else(|| {
        let outlines = cuts
            .iter()
            .flat_map(|c| c.region.iter().map(|sh| &sh.outer));
        let [x0, y0, x1, y1] = region::ring_bounds(outlines).unwrap_or([0.0; 4]);
        [(x0 + x1) / 2.0, (y0 + y1) / 2.0]
    });
    let bore = s.rod_diameter / 2.0 + s.clearance / 2.0;
    let hole = region::disk(centre, bore, s.chord);
    let hub_r = s.rod_diameter / 2.0 + s.wall;
    for cut in &cuts {
        let mut reg = cut.region.clone();
        let reach = hub_r - HUB_SLACK;
        let inner = if reach > 0.0 {
            region::offset(&reg, -reach, round_join(s))
        } else {
            reg.clone()
        };
        if !region::contains(&inner, centre) {
            let hub = region::disk(centre, hub_r, s.chord);
            let width = (2.0 * s.wall).max(s.rod_diameter);
            reg = region::union_all(&[reg.clone(), hub, bridges(&reg, centre, width)]);
            out.warnings.push(Warning::RodHub {
                part: cut.name.clone(),
            });
        }
        let frame = axis.frame(cut.at);
        let shape = region::difference(&reg, &hole);
        add_pieces(out, &cut.name, PartKind::Slice, &shape, frame, s, &[], &[])?;
    }
    spacers(&cuts, s, centre, bore, hub_r, out)
}

/// A straight strip `width` wide from `p` to the nearest outline point of every piece of the
/// region, reaching `width` into the piece for a solid join.
fn bridges(reg: &[Shape], p: P2, width: f64) -> Region {
    let mut rings: Vec<Ring> = Vec::new();
    for piece in region::pieces(reg, 0.0) {
        let mut best: Option<(f64, P2)> = None;
        for &q in &piece.outer {
            let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            if best.is_none_or(|(b, _)| d < b) {
                best = Some((d, q));
            }
        }
        let Some((_, q)) = best else { continue };
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let len = dx.hypot(dy);
        if len < 1e-6 {
            continue;
        }
        let (ex, ey) = (dx / len, dy / len);
        let far = [q[0] + ex * width, q[1] + ey * width];
        let (nx, ny) = (-ey * width / 2.0, ex * width / 2.0);
        rings.push(vec![
            [p[0] + nx, p[1] + ny],
            [far[0] + nx, far[1] + ny],
            [far[0] - nx, far[1] - ny],
            [p[0] - nx, p[1] - ny],
        ]);
    }
    region::from_rings(&rings, false)
}

/// The spacer rings (outer radius `hub_r`, bore `bore`) that fill each gap between rod
/// slices, a whole number of sheets each, and where each copy goes; a
/// [`Warning::GapsNotWhole`] when a gap is more than half a millimetre off a whole number of
/// sheets.
fn spacers(
    cuts: &[Cut],
    s: &FabSettings,
    centre: P2,
    bore: f64,
    hub_r: f64,
    out: &mut Out,
) -> FlatResult<()> {
    let t = s.thickness;
    let gaps: Vec<(f64, f64)> = cuts
        .windows(2)
        .filter_map(|w| match w {
            [a, b] => Some((a.at, b.at - a.at - t)),
            _ => None,
        })
        .collect();
    let sheets = |g: f64| (g / t).round_ties_even().max(0.0);
    let total: f64 = gaps.iter().map(|&(_, g)| sheets(g)).sum();
    if total > MAX_SPACERS as f64 {
        return Err(FlatError::TooBig("more spacer rings than allowed"));
    }
    let ring = region::pieces(
        &region::difference(
            &region::disk([0.0, 0.0], hub_r, s.chord),
            &region::disk([0.0, 0.0], bore, s.chord),
        ),
        0.0,
    );
    if let (true, Some(shape)) = (total > 0.0, ring.into_iter().next()) {
        let mut places = Vec::new();
        for &(a, g) in &gaps {
            let n = sheets(g) as u64;
            for r in 0..n {
                let f = s
                    .axis
                    .frame(a + t / 2.0 + (r as f64 + 0.5) * (g / n as f64));
                places.push(Frame {
                    origin: f.to3d(centre[0], centre[1], 0.0),
                    ..f
                });
            }
        }
        if let Some(&first) = places.first() {
            out.parts.push(FabPart {
                name: "Spacer".to_string(),
                kind: PartKind::Spacer,
                shape,
                frame: first,
                engrave: Vec::new(),
                marks: Vec::new(),
                copies: places.len() as u32,
                places,
            });
        }
    }
    if gaps
        .iter()
        .any(|&(_, g)| (g - (g / t).round_ties_even() * t).abs() > GAP_SLACK)
    {
        out.warnings.push(Warning::GapsNotWhole { thickness: t });
    }
    Ok(())
}

// ---- base and back plates -------------------------------------------------------------------

/// The map between a slice's coordinates and the plate's local `(p, q)`: `p` runs along the
/// plate, `q` is the height above it (`q < 0` is inside the plate). A base plate lies under
/// the model at `zmin`; a back plate stands behind it at the far end `u1` of the slices' u.
/// Both maps are rotations or translations, never mirrors.
#[derive(Clone, Copy)]
struct PlateMap {
    back: bool,
    u1: f64,
    v0: f64,
}

impl PlateMap {
    fn to_local(self, [x, y]: P2) -> P2 {
        if self.back {
            [y, self.u1 - x]
        } else {
            [x, y - self.v0]
        }
    }

    fn to_slice(self, [p, q]: P2) -> P2 {
        if self.back {
            [self.u1 - q, p]
        } else {
            [p, q + self.v0]
        }
    }
}

/// A slot in the plate: the slice's number, its position along the axis, and the tab's
/// stretch `(a, b)` along the plate.
struct Slot {
    slice: usize,
    at: f64,
    a: f64,
    b: f64,
}

/// Upright slices across X or Y with tabs in slots of a base plate under the model (or a back
/// plate behind it, `back`).
fn plated(
    model: &mut dyn Sectioner,
    s: &FabSettings,
    b: [f64; 6],
    back: bool,
    out: &mut Out,
) -> FlatResult<()> {
    let axis = s.axis;
    let (lo, hi) = extent(b, axis);
    let cuts = slices(model, axis, &positions(lo, hi, s, false)?, s, out, "S")?;
    let Some([u0, _, _, v1]) = region::ring_bounds(
        cuts.iter()
            .flat_map(|c| c.region.iter().map(|sh| &sh.outer)),
    ) else {
        return Ok(());
    };
    // the plate goes against the model itself, not just its outermost slice
    let v0 = extent(b, Axis::Z).0;
    let u1 = extent(b, if axis == Axis::X { Axis::Y } else { Axis::X }).1;
    let map = PlateMap { back, u1, v0 };
    let mut slots: Vec<Slot> = Vec::new();
    for (k, cut) in cuts.iter().enumerate() {
        let loc = region::map(&cut.region, |p| map.to_local(p), false);
        let (loc, tabs) = with_foot_and_tabs(&loc, s.thickness);
        slots.extend(tabs.iter().map(|&(a, b)| Slot {
            slice: k + 1,
            at: cut.at,
            a,
            b,
        }));
        let shape = region::map(&loc, |p| map.to_slice(p), false);
        add_pieces(
            out,
            &cut.name,
            PartKind::Slice,
            &shape,
            axis.frame(cut.at),
            s,
            &[],
            &[],
        )?;
    }
    let corners = [[u0, v0], [u1, v1], [u0, v1], [u1, v0]];
    let along: Vec<f64> = corners.iter().map(|&c| map.to_local(c)[0]).collect();
    let cs: Vec<f64> = cuts.iter().map(|c| c.at).collect();
    let (plate, marks) = plate_outline(&cs, &along, &slots, s);
    plate_part(plate, marks, axis, back, u1, v0, s, out)
}

/// The slice in plate coordinates with a foot down to the plate (as wide as the slice a
/// little way up: a round part touches the plate only at a point, and one floating above it
/// not at all) and the tabs that reach into the plate; also the tabs' stretches.
fn with_foot_and_tabs(loc: &[Shape], t: f64) -> (Region, Vec<(f64, f64)>) {
    let Some([p0, q0, p1, q1]) = region::bounds(loc) else {
        return (loc.to_vec(), Vec::new());
    };
    let band = (FOOT_SHEETS * t).max(FOOT_SHARE * (q1 - q0));
    let foot = region::intersection(loc, &region::rect(p0 - 1.0, q0 - 1.0, p1 + 1.0, q0 + band));
    let (mut fa, mut fb) = region::bounds(&foot).map_or((p0, p1), |f| (f[0], f[2]));
    if fb - fa < 2.0 * t {
        let mid = (fa + fb) / 2.0;
        fa = mid - t;
        fb = mid + t;
    }
    let loc = region::union(loc, &region::rect(fa, 0.0, fb, q0 + band / 2.0));
    // the foot is the one stretch the slice stands on, at least two sheets wide: one tab across
    // its middle half, or two from six sheets on
    let w = fb - fa;
    let tabs: Vec<(f64, f64)> = if w >= TWO_TABS_SHEETS * t {
        let tw = (TAB_SHEETS * t).max(TAB_SHARE * w).min(TAB_MAX);
        [0.25, 0.75]
            .iter()
            .map(|f| (fa + w * f - tw / 2.0, fa + w * f + tw / 2.0))
            .collect()
    } else {
        vec![(fa + w / 4.0, fb - w / 4.0)]
    };
    let rects: Vec<Region> = tabs
        .iter()
        .map(|&(a, b)| region::rect(a, -t, b, 0.01))
        .chain(std::iter::once(loc))
        .collect();
    (region::union_all(&rects), tabs)
}

/// How far (mm) below its lowest slot a slice's number stands on the plate (its baseline):
/// the number's top stays a millimetre clear of the slot.
const NUMBER_DROP: f64 = 4.0;

/// The capital height (mm) of the slice numbers on a plate: readable once cut, small enough to
/// sit under a slot.
const NUMBER_HEIGHT: f64 = 3.0;

/// The plate in its `(a, p)` coordinates (`a` along the slicing axis, `p` along the plate):
/// at least a margin past the slices, grown to hold the slice numbers, a slot (sheet plus
/// clearance wide) for every tab, and each slice's number engraved below its slots.
fn plate_outline(
    cs: &[f64],
    along: &[f64],
    slots: &[Slot],
    s: &FabSettings,
) -> (Region, Vec<Mark>) {
    let (t, cl, m) = (s.thickness, s.clearance, s.plate_margin);
    let lo = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut marks = Vec::new();
    for (k, &c) in cs.iter().enumerate() {
        let low = slots
            .iter()
            .filter(|sl| sl.slice == k + 1)
            .map(|sl| sl.a)
            .fold(f64::INFINITY, f64::min);
        if low.is_finite() {
            marks.push(Mark {
                text: (k + 1).to_string(),
                at: [c, low - NUMBER_DROP],
                height: NUMBER_HEIGHT,
            });
        }
    }
    // the margin is the least the plate reaches; it grows wherever a number, with the label
    // margin round it, would leave it, so every number is engraved on material
    let (mut a0, mut p0, mut a1) = (lo(cs) - t / 2.0 - m, lo(along) - m, hi(cs) + t / 2.0 + m);
    for mk in &marks {
        let lines = super::font::strokes(&mk.text, mk.height);
        if let Some([x0, y0, x1, _]) = region::ring_bounds(&lines) {
            let half = (x1 - x0) / 2.0 + LABEL_MARGIN;
            a0 = a0.min(mk.at[0] - half);
            a1 = a1.max(mk.at[0] + half);
            p0 = p0.min(mk.at[1] + y0 - LABEL_MARGIN);
        }
    }
    let plate = region::rect(a0, p0, a1, hi(along) + m);
    let w = t + cl;
    let cuts: Vec<Shape> = slots
        .iter()
        .flat_map(|sl| {
            region::rect(
                sl.at - w / 2.0,
                sl.a - cl / 2.0,
                sl.at + w / 2.0,
                sl.b + cl / 2.0,
            )
        })
        .collect();
    (region::difference(&plate, &cuts), marks)
}

/// The plate as a part, in its frame: a back plate stands at `u1 + t/2` square to the
/// slices' u, its `(a, p)` mapping straight to (along the axis, z); a base plate lies at
/// `zmin - t/2`, its `(a, p)` being `(x, y)` for X slices and `(y, x)` (mirrored back to
/// `(x, y)`) for Y slices.
fn plate_part(
    plate: Region,
    marks: Vec<Mark>,
    axis: Axis,
    back: bool,
    u1: f64,
    v0: f64,
    s: &FabSettings,
    out: &mut Out,
) -> FlatResult<()> {
    let t = s.thickness;
    let (x, y, z) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]);
    let (frame, shape, marks) = match (back, axis) {
        (true, Axis::Y) => (
            Frame {
                origin: [u1 + t / 2.0, 0.0, 0.0],
                u: y,
                v: z,
                n: x,
            },
            plate,
            marks,
        ),
        (true, _) => (
            Frame {
                origin: [0.0, u1 + t / 2.0, 0.0],
                u: x,
                v: z,
                n: [0.0, -1.0, 0.0],
            },
            plate,
            marks,
        ),
        (false, Axis::Y) => (
            Frame {
                origin: [0.0, 0.0, v0 - t / 2.0],
                u: x,
                v: y,
                n: z,
            },
            region::map(&plate, |[a, p]| [p, a], true),
            marks
                .into_iter()
                .map(|m| Mark {
                    at: [m.at[1], m.at[0]],
                    ..m
                })
                .collect(),
        ),
        (false, _) => (
            Frame {
                origin: [0.0, 0.0, v0 - t / 2.0],
                u: x,
                v: y,
                n: z,
            },
            plate,
            marks,
        ),
    };
    let name = if back { "Back plate" } else { "Base plate" };
    add_pieces(out, name, PartKind::Plate, &shape, frame, s, &[], &marks)
}

// ---- waffle ---------------------------------------------------------------------------------

/// The stretches of `v` where the region has material along the line at `u`, stretches less
/// than a micrometre apart merged.
fn spans(r: &[Shape], u: f64) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (a, b) in region::spans_at_x(r, u) {
        match out.last_mut() {
            Some(last) if a <= last.1 + SPAN_MERGE => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Two sets of upright slices, across X and across Y, slotted where they cross at one height per
/// crossing line, the middle of the longest stretch of material both have there: the X slice
/// keeps what lies below it and the Y slice what lies above, each slot running out of the part,
/// so the X slices stand first and the Y slices drop on from above. Where the line runs through
/// more material, the X slice is cut through above and the Y slice below; the pieces this frees
/// are reported.
fn waffle(
    model: &mut dyn Sectioner,
    s: &FabSettings,
    b: [f64; 6],
    out: &mut Out,
) -> FlatResult<()> {
    let (t, cl) = (s.thickness, s.clearance);
    let w = t + cl;
    let (x0, x1) = extent(b, Axis::X);
    let (y0, y1) = extent(b, Axis::Y);
    let xs = slices(model, Axis::X, &positions(x0, x1, s, false)?, s, out, "X")?;
    let ys = slices(model, Axis::Y, &positions(y0, y1, s, false)?, s, out, "Y")?;
    let (z0, z1) = extent(b, Axis::Z);
    let (lo, hi) = (z0 - 1.0, z1 + 1.0);
    let mut cut_x: Vec<Region> = vec![Vec::new(); xs.len()];
    let mut cut_y: Vec<Region> = vec![Vec::new(); ys.len()];
    for (xc, cx) in xs.iter().zip(cut_x.iter_mut()) {
        for (yc, cy) in ys.iter().zip(cut_y.iter_mut()) {
            // the X slice is (y, z), the Y slice (x, z): each along the line where they cross
            let a = spans(&xc.region, yc.at);
            let bb = spans(&yc.region, xc.at);
            // the longest stretch both slices have material on, the lowest of equal ones
            let mut shared: Option<(f64, f64)> = None;
            for &(p, q) in &a {
                for &(r, u) in &bb {
                    let (z0, z1) = (p.max(r), q.min(u));
                    if z1 > z0 && shared.is_none_or(|(s0, s1)| z1 - z0 > s1 - s0) {
                        shared = Some((z0, z1));
                    }
                }
            }
            let Some((z0, z1)) = shared else { continue };
            let zm = (z0 + z1) / 2.0;
            cx.extend(region::rect(
                yc.at - w / 2.0,
                zm - cl / 2.0,
                yc.at + w / 2.0,
                hi,
            ));
            cy.extend(region::rect(
                xc.at - w / 2.0,
                lo,
                xc.at + w / 2.0,
                zm + cl / 2.0,
            ));
        }
    }
    for (set, cuts, axis) in [(&xs, &cut_x, Axis::X), (&ys, &cut_y, Axis::Y)] {
        for (c, cut) in set.iter().zip(cuts) {
            let shape = region::difference(&c.region, cut);
            add_pieces(
                out,
                &c.name,
                PartKind::Slice,
                &shape,
                axis.frame(c.at),
                s,
                &[],
                &[],
            )?;
        }
    }
    if cut_x.iter().all(Vec::is_empty) {
        out.warnings.push(Warning::NeverCross);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::section::BrepSection;
    use crate::geom::vector3d::Point3;
    use crate::kernel::Kernel;
    use crate::primitive::api_primitives::create_sphere;
    use std::f64::consts::{PI, TAU};

    // ---- analytic models ----------------------------------------------------------------

    fn dot(p: [f64; 3], d: [f64; 3]) -> f64 {
        p[0] * d[0] + p[1] * d[1] + p[2] * d[2]
    }

    /// A point's slice coordinates on slices across `axis`.
    fn uv(axis: Axis, p: [f64; 3]) -> P2 {
        let (_, u, v) = axis.directions();
        [dot(p, u), dot(p, v)]
    }

    /// Axis-aligned boxes `(min, max)`; their sections are rectangles.
    struct Boxes(Vec<([f64; 3], [f64; 3])>);

    impl Sectioner for Boxes {
        fn bounds(&self) -> [f64; 6] {
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [-f64::INFINITY; 3]);
            for (a, b) in &self.0 {
                for (l, x) in lo.iter_mut().zip(a) {
                    *l = l.min(*x);
                }
                for (h, x) in hi.iter_mut().zip(b) {
                    *h = h.max(*x);
                }
            }
            [lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
        }

        fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region> {
            let i = axis.index();
            let rects: Vec<Region> = self
                .0
                .iter()
                .filter(|(lo, hi)| lo[i] < at && at < hi[i])
                .map(|(lo, hi)| {
                    let (a, b) = (uv(axis, *lo), uv(axis, *hi));
                    region::rect(a[0], a[1], b[0], b[1])
                })
                .collect();
            Ok(region::union_all(&rects))
        }
    }

    fn one_box(lo: [f64; 3], hi: [f64; 3]) -> Boxes {
        Boxes(vec![(lo, hi)])
    }

    /// A 100 x 60 x 40 box with a corner at the origin.
    fn test_box() -> Boxes {
        one_box([0.0; 3], [100.0, 60.0, 40.0])
    }

    /// A sphere; its sections are discs within a fine chord.
    struct Sphere {
        c: [f64; 3],
        r: f64,
    }

    impl Sectioner for Sphere {
        fn bounds(&self) -> [f64; 6] {
            let (c, r) = (self.c, self.r);
            [c[0] - r, c[1] - r, c[2] - r, c[0] + r, c[1] + r, c[2] + r]
        }

        fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region> {
            let d = at - self.c[axis.index()];
            if d.abs() >= self.r {
                return Ok(Vec::new());
            }
            Ok(region::disk(
                uv(axis, self.c),
                (self.r * self.r - d * d).sqrt(),
                0.005,
            ))
        }
    }

    /// A sphere of radius 50 standing on z = 0.
    fn test_sphere() -> Sphere {
        Sphere {
            c: [0.0, 0.0, 50.0],
            r: 50.0,
        }
    }

    /// A torus about the Z axis through `c`: annuli across Z, the two-lobed or one-lobed
    /// sections across X and Y, sampled from the tube's circle.
    struct Torus {
        c: [f64; 3],
        big: f64,
        small: f64,
    }

    impl Sectioner for Torus {
        fn bounds(&self) -> [f64; 6] {
            let (c, o, s) = (self.c, self.big + self.small, self.small);
            [c[0] - o, c[1] - o, c[2] - s, c[0] + o, c[1] + o, c[2] + s]
        }

        fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region> {
            let (c, big, small) = (self.c, self.big, self.small);
            let i = axis.index();
            let d = at - c[i];
            if axis == Axis::Z {
                if d.abs() >= small {
                    return Ok(Vec::new());
                }
                let h = (small * small - d * d).sqrt();
                let centre = [c[0], c[1]];
                return Ok(region::difference(
                    &region::disk(centre, big + h, 0.005),
                    &region::disk(centre, big - h, 0.005),
                ));
            }
            // across X the slice's u is y; across Y it is x; v is z either way
            let uc = if axis == Axis::X { c[1] } else { c[0] };
            let a = d.abs();
            if a >= big + small {
                return Ok(Vec::new());
            }
            let n = 720_u32;
            let at_phi = |phi: f64, sign: f64| {
                let rho = big + small * phi.cos();
                [
                    uc + sign * (rho * rho - a * a).max(0.0).sqrt(),
                    c[2] + small * phi.sin(),
                ]
            };
            let rings: Vec<Ring> = if a < big - small {
                [1.0, -1.0]
                    .iter()
                    .map(|&sg| {
                        (0..n)
                            .map(|k| at_phi(TAU * f64::from(k) / f64::from(n), sg))
                            .collect()
                    })
                    .collect()
            } else {
                let phi0 = ((a - big) / small).clamp(-1.0, 1.0).acos();
                let mut ring: Ring = (0..n)
                    .map(|k| at_phi(-phi0 + 2.0 * phi0 * f64::from(k) / f64::from(n), 1.0))
                    .collect();
                ring.extend(
                    (0..n).map(|k| at_phi(phi0 - 2.0 * phi0 * f64::from(k) / f64::from(n), -1.0)),
                );
                vec![ring]
            };
            Ok(region::from_rings(&rings, true))
        }
    }

    /// A torus of major radius 40 and minor radius 15 about z = 20.
    fn test_torus() -> Torus {
        Torus {
            c: [0.0, 0.0, 20.0],
            big: 40.0,
            small: 15.0,
        }
    }

    /// A model with no material at all, finite bounds.
    struct Empty;

    impl Sectioner for Empty {
        fn bounds(&self) -> [f64; 6] {
            [0.0, 0.0, 0.0, 10.0, 10.0, 10.0]
        }

        fn section(&mut self, _: Axis, _: f64) -> FlatResult<Region> {
            Ok(Vec::new())
        }
    }

    /// Fixed bounds, every section a 10 mm square (or `bad` points).
    struct Fixed {
        b: [f64; 6],
        bad: bool,
    }

    impl Sectioner for Fixed {
        fn bounds(&self) -> [f64; 6] {
            self.b
        }

        fn section(&mut self, _: Axis, _: f64) -> FlatResult<Region> {
            if self.bad {
                return Ok(vec![Shape {
                    outer: vec![[0.0, 0.0], [f64::NAN, 1.0], [1.0, f64::INFINITY]],
                    holes: Vec::new(),
                }]);
            }
            Ok(region::rect(0.0, 0.0, 10.0, 10.0))
        }
    }

    // ---- helpers ------------------------------------------------------------------------

    /// The settings these tests start from, for `style`.
    fn settings(style: Style) -> FabSettings {
        FabSettings {
            style,
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

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn of_kind(res: &FabResult, kind: PartKind) -> Vec<&FabPart> {
        res.parts.iter().filter(|p| p.kind == kind).collect()
    }

    fn size(p: &FabPart) -> (f64, f64) {
        let b = region::ring_bounds([&p.shape.outer]).unwrap();
        (b[2] - b[0], b[3] - b[1])
    }

    fn centroid(r: &Ring) -> P2 {
        let n = r.len() as f64;
        [
            r.iter().map(|p| p[0]).sum::<f64>() / n,
            r.iter().map(|p| p[1]).sum::<f64>() / n,
        ]
    }

    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Every part normalised (outline counter-clockwise, holes clockwise, material), and
    /// every frame right-handed: `u x v = n`.
    fn assert_sound(res: &FabResult) {
        for p in &res.parts {
            assert!(p.shape.outer.len() >= 3, "{}", p.name);
            assert!(region::ring_area(&p.shape.outer) > 0.0, "{}", p.name);
            assert!(
                p.shape.holes.iter().all(|h| region::ring_area(h) < 0.0),
                "{}",
                p.name
            );
            assert!(region::shape_area(&p.shape) > 0.0, "{}", p.name);
            assert!(p.copies >= 1);
            for f in std::iter::once(&p.frame).chain(&p.places) {
                let n = cross(f.u, f.v);
                for (a, b) in n.iter().zip(f.n) {
                    assert!(close(*a, b, 1e-12), "{}: {f:?}", p.name);
                }
            }
            if p.kind == PartKind::Spacer {
                assert_eq!(p.copies as usize, p.places.len());
            }
        }
    }

    // ---- styles -------------------------------------------------------------------------

    #[test]
    fn stacked_box_has_ten_layers_with_dowel_holes_lined_up() {
        let s = FabSettings {
            thickness: 4.0,
            dowels: 2,
            ..settings(Style::Stacked)
        };
        let res = make(&mut test_box(), &s).unwrap();
        assert_eq!(res.parts.len(), 10); // 40 mm / 4 mm
        assert_eq!(res.count(), 10);
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        for (i, p) in res.parts.iter().enumerate() {
            assert_eq!(p.name, format!("L{}", i + 1));
            assert_eq!(p.kind, PartKind::Slice);
            let (w, h) = size(p);
            assert!(close(w, 100.0, 1e-6) && close(h, 60.0, 1e-6), "{w} x {h}");
            assert_eq!(p.shape.holes.len(), 2);
            assert!(close(p.frame.origin[2], 2.0 + 4.0 * i as f64, 1e-12));
            assert_eq!(p.frame.n, [0.0, 0.0, 1.0]);
        }
        let centres = |p: &FabPart| -> Vec<P2> { p.shape.holes.iter().map(centroid).collect() };
        let first = centres(&res.parts[0]);
        for p in &res.parts {
            for (a, b) in centres(p).iter().zip(&first) {
                assert!(close(a[0], b[0], 1e-9) && close(a[1], b[1], 1e-9));
            }
        }
        assert_sound(&res);
    }

    #[test]
    fn stacked_sphere_guides_go_on_the_bigger_neighbour() {
        let s = FabSettings {
            thickness: 5.0,
            dowels: 0,
            ..settings(Style::Stacked)
        };
        let res = make(&mut test_sphere(), &s).unwrap();
        assert_eq!(res.parts.len(), 20);
        assert!(res.parts.iter().all(|p| p.shape.holes.is_empty()));
        // the end layers are the smallest: nothing smaller sits on them
        assert!(res.parts[0].engrave.is_empty() && res.parts[19].engrave.is_empty());
        assert!(!res.parts[9].engrave.is_empty());
        assert_eq!(
            res.parts.iter().filter(|p| !p.engrave.is_empty()).count(),
            18
        );
        for p in &res.parts {
            for line in &p.engrave {
                assert!(line.len() >= 2);
                for &q in line {
                    assert!(
                        region::contains(std::slice::from_ref(&p.shape), q),
                        "{}",
                        p.name
                    );
                }
            }
        }
        assert_sound(&res);
    }

    #[test]
    fn rod_spacers_fill_the_gaps() {
        let s = FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            spacing: 12.0,
            rod_diameter: 8.0,
            ..settings(Style::Rod)
        };
        let res = make(&mut test_box(), &s).unwrap();
        let slices = of_kind(&res, PartKind::Slice);
        assert_eq!(slices.len(), 9);
        for (i, p) in slices.iter().enumerate() {
            assert!(close(p.frame.origin[0], 2.0 + 12.0 * i as f64, 1e-12));
            assert_eq!(p.shape.holes.len(), 1); // the rod hole
        }
        // the rod goes through the middle of the slices, (y, z) = (30, 20)
        let hole = &slices[0].shape.holes[0];
        for q in hole {
            let r = (q[0] - 30.0).hypot(q[1] - 20.0);
            assert!(close(r, 4.1, 1e-3), "{r}"); // rod radius + half the clearance
        }
        let spacer = of_kind(&res, PartKind::Spacer)[0];
        assert_eq!(spacer.copies, 8 * 3); // 9 mm gaps = 3 sheets each
        let mut xs: Vec<f64> = spacer.places.iter().map(|f| f.origin[0]).collect();
        xs.sort_by(f64::total_cmp);
        assert_eq!(xs.len(), 24);
        for (x, want) in xs.iter().zip([5.0, 8.0, 11.0, 17.0]) {
            assert!(close(*x, want, 1e-9), "{xs:?}");
        }
        for f in &spacer.places {
            assert!(close(f.origin[1], 30.0, 1e-12) && close(f.origin[2], 20.0, 1e-12));
        }
        // the ring: outer radius 4 + 4, bore 4.1
        let ring_area = region::shape_area(&spacer.shape);
        // inscribed polygons within the 0.01 chord: off by at most the perimeters times it
        let slack = TAU * (8.0 + 4.1) * 0.01;
        assert!(
            close(ring_area, PI * (64.0 - 4.1 * 4.1), slack),
            "{ring_area}"
        );
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        assert_sound(&res);
    }

    #[test]
    fn rod_through_a_torus_gets_hubs_and_stays_one_piece() {
        let s = FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            ..settings(Style::Rod)
        };
        let res = make(&mut test_torus(), &s).unwrap();
        let names: Vec<&str> = of_kind(&res, PartKind::Slice)
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names.len(), 8);
        assert!(
            names
                .iter()
                .all(|n| n.ends_with(|c: char| c.is_ascii_digit())),
            "{names:?}"
        );
        let hubs: Vec<&Warning> = res
            .warnings
            .iter()
            .filter(|w| matches!(w, Warning::RodHub { .. }))
            .collect();
        // the four middle slices (|x| < 25) are two lobes around the rod
        assert_eq!(hubs.len(), 4, "{:?}", res.warnings);
        assert!(!res
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::FallsApart { .. })));
        assert_sound(&res);
    }

    #[test]
    fn plate_slots_match_the_tabs() {
        for style in [Style::Base, Style::Back] {
            for axis in [Axis::X, Axis::Y] {
                let s = FabSettings {
                    axis,
                    thickness: 3.0,
                    count: 5,
                    clearance: 0.2,
                    ..settings(style)
                };
                let res = make(&mut test_sphere(), &s).unwrap();
                let plates = of_kind(&res, PartKind::Plate);
                assert_eq!(plates.len(), 1);
                let plate = plates[0];
                assert_eq!(of_kind(&res, PartKind::Slice).len(), 5);
                assert_eq!(plate.shape.holes.len(), 10, "{style:?} {axis:?}"); // two tabs a slice
                for hole in &plate.shape.holes {
                    let b = region::ring_bounds([hole]).unwrap();
                    let narrow = (b[2] - b[0]).min(b[3] - b[1]);
                    assert!(close(narrow, 3.2, 1e-6), "{narrow}");
                }
                let texts: Vec<&str> = plate.marks.iter().map(|m| m.text.as_str()).collect();
                assert_eq!(texts, ["1", "2", "3", "4", "5"]);
                assert!(res.warnings.is_empty(), "{:?}", res.warnings);
                assert_sound(&res);
            }
        }
    }

    #[test]
    fn the_base_plate_lies_under_the_model_and_the_back_plate_behind_it() {
        let s = FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            count: 4,
            ..settings(Style::Base)
        };
        let res = make(&mut test_sphere(), &s).unwrap();
        let plate = of_kind(&res, PartKind::Plate)[0];
        assert_eq!(plate.name, "Base plate");
        assert!(close(plate.frame.origin[2], -1.5, 1e-12));
        assert_eq!(plate.frame.n, [0.0, 0.0, 1.0]);
        let back = make(
            &mut test_sphere(),
            &FabSettings {
                style: Style::Back,
                ..s.clone()
            },
        )
        .unwrap();
        let plate = of_kind(&back, PartKind::Plate)[0];
        assert_eq!(plate.name, "Back plate");
        assert_eq!(plate.frame.origin, [0.0, 51.5, 0.0]);
        assert_eq!(plate.frame.n, [0.0, -1.0, 0.0]);
        let back_y = make(
            &mut test_sphere(),
            &FabSettings {
                style: Style::Back,
                axis: Axis::Y,
                ..s
            },
        )
        .unwrap();
        let plate = of_kind(&back_y, PartKind::Plate)[0];
        assert_eq!(plate.frame.origin, [51.5, 0.0, 0.0]);
        assert_eq!(plate.frame.n, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn every_slice_reaches_into_its_plate() {
        // each tab goes one sheet below the plate's top face (q = 0, z = 0 for a base plate)
        let s = FabSettings {
            axis: Axis::Y,
            thickness: 3.0,
            count: 3,
            ..settings(Style::Base)
        };
        let res = make(&mut test_sphere(), &s).unwrap();
        for p in of_kind(&res, PartKind::Slice) {
            let b = region::ring_bounds([&p.shape.outer]).unwrap();
            assert!(close(b[1], -3.0, 1e-6), "{}: {b:?}", p.name);
        }
    }

    #[test]
    fn waffle_box_slots_meet_halfway() {
        let s = FabSettings {
            thickness: 3.0,
            count: 3,
            clearance: 0.2,
            ..settings(Style::Waffle)
        };
        let res = make(&mut test_box(), &s).unwrap();
        let xs: Vec<&FabPart> = res
            .parts
            .iter()
            .filter(|p| p.name.starts_with('X'))
            .collect();
        let ys: Vec<&FabPart> = res
            .parts
            .iter()
            .filter(|p| p.name.starts_with('Y'))
            .collect();
        assert_eq!((xs.len(), ys.len()), (3, 3));
        // an X slice is 60 x 40 with three slots from the top down to half height
        let slot = 3.0 * 3.2 * (20.0 + 0.1);
        for p in &xs {
            // (the polygon engine's grid puts slot corners within about 3e-8 mm)
            let a = region::shape_area(&p.shape);
            assert!(close(a, 60.0 * 40.0 - slot, 1e-4), "{}: {a}", p.name);
            let top = p.shape.outer.iter().map(|q| q[1]).fold(f64::MIN, f64::max);
            assert!(close(top, 40.0, 1e-6));
        }
        for p in &ys {
            assert!(close(
                region::shape_area(&p.shape),
                100.0 * 40.0 - slot,
                1e-4
            ));
        }
        assert!(res.warnings.is_empty());
        assert_sound(&res);
        line_split(&res);
    }

    #[test]
    fn waffle_sphere_has_no_slivers_and_no_warnings() {
        let s = FabSettings {
            thickness: 3.0,
            count: 6,
            ..settings(Style::Waffle)
        };
        let res = make(&mut test_sphere(), &s).unwrap();
        assert_eq!(res.parts.len(), 12);
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        assert_sound(&res);
        line_split(&res);
    }

    /// The lowest and highest material of `part` along its line at `u`, `None` with none.
    fn line(part: &FabPart, u: f64) -> Option<(f64, f64)> {
        let spans = region::spans_at_x(std::slice::from_ref(&part.shape), u);
        Some((
            spans.first()?.0,
            spans.iter().map(|s| s.1).fold(f64::MIN, f64::max),
        ))
    }

    /// The waffle's assembly rule: on every crossing line where both have material, an X part's
    /// material ends below where the Y part's begins, so the Y slices drop onto the standing X
    /// slices from above.
    fn line_split(res: &FabResult) {
        let set = |c: char| -> Vec<&FabPart> {
            res.parts.iter().filter(|p| p.name.starts_with(c)).collect()
        };
        for x in set('X') {
            for y in set('Y') {
                // an X part lies at x = its frame's origin, a Y part at y = its frame's
                let (Some((_, top)), Some((bottom, _))) =
                    (line(x, y.frame.origin[1]), line(y, x.frame.origin[0]))
                else {
                    continue;
                };
                assert!(top < bottom, "{} {}: {top} {bottom}", x.name, y.name);
            }
        }
    }

    #[test]
    fn waffle_thin_crossings_are_slotted() {
        // a 5 mm plate: every crossing shares less than 2 t, and still gets its half-laps
        let mut model = one_box([0.0; 3], [100.0, 100.0, 5.0]);
        let s = FabSettings {
            thickness: 3.0,
            count: 3,
            clearance: 0.2,
            ..settings(Style::Waffle)
        };
        let res = make(&mut model, &s).unwrap();
        let names: Vec<&str> = res.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["X1", "X2", "X3", "Y1", "Y2", "Y3"]);
        // the X slots from 2.4 up, the Y slots from 2.6 down, three in each part
        for p in &res.parts {
            let a = region::shape_area(&p.shape);
            assert!(close(a, 500.0 - 3.0 * 3.2 * 2.6, 1e-4), "{}: {a}", p.name);
        }
        for x in res.parts.iter().filter(|p| p.name.starts_with('X')) {
            for y in res.parts.iter().filter(|p| p.name.starts_with('Y')) {
                let (Some((_, top)), Some((bottom, _))) =
                    (line(x, y.frame.origin[1]), line(y, x.frame.origin[0]))
                else {
                    panic!("{} {}", x.name, y.name)
                };
                assert!(
                    close(top, 2.4, 1e-6) && close(bottom, 2.6, 1e-6),
                    "{top} {bottom}"
                );
            }
        }
        assert!(res.warnings.is_empty(), "{:?}", res.warnings);
        assert_sound(&res);
        line_split(&res);
    }

    #[test]
    fn waffle_crossing_two_layers_cuts_one_slice_through() {
        // a bookshelf: a wall at x 0..10 and two shelves; slices at 2, 14, 26 and 38 both ways
        let mut model = Boxes(vec![
            ([0.0; 3], [10.0, 40.0, 50.0]),
            ([0.0; 3], [40.0, 40.0, 20.0]),
            ([0.0, 0.0, 30.0], [40.0, 40.0, 50.0]),
        ]);
        let s = FabSettings {
            thickness: 3.0,
            clearance: 0.2,
            ..settings(Style::Waffle)
        };
        let res = make(&mut model, &s).unwrap();
        let names: Vec<&str> = res.parts.iter().map(|p| p.name.as_str()).collect();
        let mut want = vec!["X1".to_string()];
        for i in 2..=4 {
            want.extend("abcdef".chars().map(|c| format!("X{i}{c}")));
        }
        want.extend((1..=4).map(|i| format!("Y{i}")));
        assert_eq!(names, want);
        // through the wall, one stretch (0, 50) cut at 25; through the shelves, two equal ones
        // cut at the lower's middle, 10: the upper shelf of each X slice is cut through
        let area = |name: &str| {
            let p = res.parts.iter().find(|p| p.name == name).unwrap();
            region::shape_area(&p.shape)
        };
        assert!(close(area("X1"), 2000.0 - 4.0 * 3.2 * 25.1, 1e-4));
        for i in 2..=4 {
            assert!(close(
                area(&format!("X{i}a")),
                800.0 - 4.0 * 3.2 * 10.1,
                1e-4
            ));
        }
        for i in 1..=4 {
            let y = 1700.0 - 3.2 * 25.1 - 3.0 * 3.2 * 10.1;
            assert!(close(area(&format!("Y{i}")), y, 1e-4));
        }
        let apart: Vec<Warning> = (2..=4)
            .map(|i| Warning::FallsApart {
                part: format!("X{i}"),
                pieces: 6,
            })
            .collect();
        assert_eq!(res.warnings, apart);
        assert_sound(&res);
        line_split(&res);
    }

    #[test]
    fn bad_settings_are_refused() {
        let s = FabSettings {
            thickness: 0.0,
            ..settings(Style::Stacked)
        };
        assert!(matches!(
            make(&mut test_box(), &s),
            Err(FlatError::Setting {
                name: "thickness",
                ..
            })
        ));
        for style in [Style::Base, Style::Back] {
            let s = FabSettings {
                axis: Axis::Z,
                ..settings(style)
            };
            assert!(matches!(
                make(&mut test_box(), &s),
                Err(FlatError::Setting { name: "axis", .. })
            ));
        }
        // a waffle ignores the axis
        let s = FabSettings {
            axis: Axis::Z,
            count: 2,
            ..settings(Style::Waffle)
        };
        assert!(make(&mut test_box(), &s).is_ok());
    }

    // ---- more cases ---------------------------------------------------------------------

    #[test]
    fn every_style_along_every_axis_it_can_use() {
        let cases: [(Style, &[Axis]); 5] = [
            (Style::Stacked, &[Axis::X, Axis::Y, Axis::Z]),
            (Style::Rod, &[Axis::X, Axis::Y, Axis::Z]),
            (Style::Base, &[Axis::X, Axis::Y]),
            (Style::Back, &[Axis::X, Axis::Y]),
            (Style::Waffle, &[Axis::X, Axis::Y, Axis::Z]),
        ];
        for (style, axes) in cases {
            for &axis in axes {
                let s = FabSettings {
                    axis,
                    thickness: 4.0,
                    count: 4,
                    ..settings(style)
                };
                for model in [
                    &mut test_sphere() as &mut dyn Sectioner,
                    &mut test_box(),
                    &mut test_torus(),
                ] {
                    let res = make(model, &s).unwrap();
                    assert_sound(&res);
                    let slices = of_kind(&res, PartKind::Slice);
                    assert!(!slices.is_empty());
                    if style != Style::Waffle {
                        let n = axis.directions().0;
                        assert!(slices.iter().all(|p| p.frame.n == n), "{style:?} {axis:?}");
                    }
                    assert!(close(res.thickness, 4.0, 0.0));
                }
            }
        }
    }

    #[test]
    fn stacked_layers_follow_the_axis() {
        // a 100 x 60 x 40 box stacked along X: 100 / 4 = 25 layers of 60 x 40
        let s = FabSettings {
            axis: Axis::X,
            thickness: 4.0,
            dowels: 1,
            ..settings(Style::Stacked)
        };
        let res = make(&mut test_box(), &s).unwrap();
        assert_eq!(res.parts.len(), 25);
        for p in &res.parts {
            let (w, h) = size(p);
            assert!(close(w, 60.0, 1e-6) && close(h, 40.0, 1e-6));
            assert_eq!(p.shape.holes.len(), 1);
            assert_eq!(p.frame.n, [1.0, 0.0, 0.0]);
        }
        // one dowel: in the middle of the shared material
        let c = centroid(&res.parts[0].shape.holes[0]);
        assert!(close(c[0], 30.0, 1.5) && close(c[1], 20.0, 1.5), "{c:?}"); // a grid step
    }

    #[test]
    fn hostile_settings_never_panic() {
        type Set = fn(&mut FabSettings, f64);
        let values = [
            f64::NAN,
            f64::INFINITY,
            -f64::INFINITY,
            -1.0,
            0.0,
            1e-300,
            1e300,
            5e3,
        ];
        let fields: [(&str, Set); 10] = [
            ("thickness", |s, x| s.thickness = x),
            ("spacing", |s, x| s.spacing = x),
            ("clearance", |s, x| s.clearance = x),
            ("rod_diameter", |s, x| s.rod_diameter = x),
            ("dowel_diameter", |s, x| s.dowel_diameter = x),
            ("wall", |s, x| s.wall = x),
            ("plate_margin", |s, x| s.plate_margin = x),
            ("min_area", |s, x| s.min_area = x),
            ("chord", |s, x| s.chord = x),
            ("rod_at", |s, x| s.rod_at = Some([x, 1.0])),
        ];
        let styles = [
            Style::Stacked,
            Style::Rod,
            Style::Base,
            Style::Back,
            Style::Waffle,
        ];
        for style in styles {
            let base = FabSettings {
                axis: Axis::X,
                ..settings(style)
            };
            for (name, set) in fields {
                for x in values {
                    let mut s = base.clone();
                    set(&mut s, x);
                    let model = &mut one_box([0.0; 3], [20.0, 10.0, 8.0]);
                    match make(model, &s) {
                        Ok(res) => assert_sound(&res),
                        Err(e) => {
                            if x.is_nan() || x == -f64::INFINITY || (x == -1.0 && name != "rod_at")
                            {
                                assert!(
                                    matches!(e, FlatError::Setting { name: n, .. } if n == name),
                                    "{name} = {x}"
                                );
                            }
                        }
                    }
                }
            }
            for count in [0, 1, MAX_SLICES as u32, MAX_SLICES as u32 + 1, u32::MAX] {
                let s = FabSettings {
                    count,
                    ..base.clone()
                };
                let r = make(&mut one_box([0.0; 3], [20.0, 10.0, 8.0]), &s);
                if count as usize > MAX_SLICES {
                    assert!(matches!(r, Err(FlatError::Setting { name: "count", .. })));
                }
            }
            for dowels in [0, MAX_DOWELS, MAX_DOWELS + 1, u32::MAX] {
                let s = FabSettings {
                    dowels,
                    ..base.clone()
                };
                let r = make(&mut one_box([0.0; 3], [20.0, 10.0, 8.0]), &s);
                if dowels > MAX_DOWELS {
                    assert!(matches!(r, Err(FlatError::Setting { name: "dowels", .. })));
                } else {
                    assert_sound(&r.unwrap());
                }
            }
        }
    }

    #[test]
    fn too_many_slices_are_refused_not_attempted() {
        let s = FabSettings {
            thickness: 0.001,
            clearance: 0.0,
            ..settings(Style::Stacked)
        };
        assert!(matches!(
            make(&mut test_box(), &s),
            Err(FlatError::TooBig(_))
        ));
        // a sheet apart over 2 m: 666 rod slices
        let s = FabSettings {
            axis: Axis::X,
            spacing: 3.0,
            ..settings(Style::Rod)
        };
        assert!(matches!(
            make(&mut one_box([0.0; 3], [2000.0, 60.0, 40.0]), &s),
            Err(FlatError::TooBig(_))
        ));
        // closer than a sheet is a setting out of range before it is too many
        let s = FabSettings {
            spacing: 1e-6,
            ..settings(Style::Rod)
        };
        assert!(matches!(
            make(&mut test_box(), &s),
            Err(FlatError::Setting {
                name: "spacing",
                ..
            })
        ));
    }

    #[test]
    fn a_model_with_nothing_to_cut() {
        for style in [
            Style::Stacked,
            Style::Rod,
            Style::Base,
            Style::Back,
            Style::Waffle,
        ] {
            let s = FabSettings {
                axis: Axis::X,
                ..settings(style)
            };
            assert_eq!(make(&mut Empty, &s), Err(FlatError::Nothing));
            for b in [
                [f64::NAN; 6],
                [0.0, 0.0, 0.0, f64::INFINITY, 1.0, 1.0],
                [5.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::INFINITY,
                    -f64::INFINITY,
                    -f64::INFINITY,
                    -f64::INFINITY,
                ],
            ] {
                assert_eq!(
                    make(&mut Fixed { b, bad: false }, &s),
                    Err(FlatError::Nothing)
                );
            }
            let huge = [0.0, 0.0, 0.0, 1e7, 1.0, 1.0];
            assert!(matches!(
                make(
                    &mut Fixed {
                        b: huge,
                        bad: false
                    },
                    &s
                ),
                Err(FlatError::TooBig(_))
            ));
            // sections of points that are not finite are no material
            let b = [0.0, 0.0, 0.0, 10.0, 10.0, 10.0];
            assert_eq!(
                make(&mut Fixed { b, bad: true }, &s),
                Err(FlatError::Nothing)
            );
        }
    }

    #[test]
    fn a_model_in_two_pieces_names_them_a_and_b() {
        let mut model = Boxes(vec![
            ([0.0; 3], [20.0, 20.0, 12.0]),
            ([40.0, 0.0, 0.0], [60.0, 20.0, 12.0]),
        ]);
        let s = FabSettings {
            thickness: 3.0,
            dowels: 0,
            ..settings(Style::Stacked)
        };
        let res = make(&mut model, &s).unwrap();
        let names: Vec<&str> = res.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["L1a", "L1b", "L2a", "L2b", "L3a", "L3b", "L4a", "L4b"]
        );
        let want: Vec<Warning> = (1..=4)
            .map(|i| Warning::FallsApart {
                part: format!("L{i}"),
                pieces: 2,
            })
            .collect();
        assert_eq!(res.warnings, want);
        assert_sound(&res);
    }

    #[test]
    fn empty_slices_are_left_out_with_a_warning() {
        let mut model = Boxes(vec![
            ([0.0; 3], [20.0, 20.0, 6.0]),
            ([0.0, 0.0, 12.0], [20.0, 20.0, 18.0]),
        ]);
        let res = make(&mut model, &settings(Style::Stacked)).unwrap();
        let names: Vec<&str> = res.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["L1", "L2", "L3", "L4"]);
        assert_eq!(
            res.warnings,
            [
                Warning::NoMaterial {
                    axis: Axis::Z,
                    at: 7.5
                },
                Warning::NoMaterial {
                    axis: Axis::Z,
                    at: 10.5
                },
            ]
        );
        assert!(res.parts.iter().all(|p| p.shape.holes.len() == 2));
    }

    #[test]
    fn dowels_that_find_no_room_are_reported() {
        // a 10 mm square leaves no room for a 6 mm dowel with 4 mm of wall
        let res = make(
            &mut one_box([0.0; 3], [10.0, 10.0, 9.0]),
            &settings(Style::Stacked),
        )
        .unwrap();
        assert_eq!(
            res.warnings,
            [Warning::DowelsShort {
                placed: 0,
                wanted: 2
            }]
        );
        assert!(res.parts.iter().all(|p| p.shape.holes.is_empty()));
    }

    #[test]
    fn waffle_sets_that_never_cross_are_reported() {
        // an X-long bar and a Y-long bar that never share a crossing line
        let mut model = Boxes(vec![
            ([0.0; 3], [100.0, 2.0, 10.0]),
            ([0.0, 50.0, 0.0], [2.0, 100.0, 10.0]),
        ]);
        let s = FabSettings {
            count: 3,
            ..settings(Style::Waffle)
        };
        let res = make(&mut model, &s).unwrap();
        let names: Vec<&str> = res.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["X1", "X2", "X3", "Y1"]);
        assert_eq!(res.warnings.last(), Some(&Warning::NeverCross));
        assert_eq!(
            res.warnings
                .iter()
                .filter(|w| matches!(w, Warning::NoMaterial { axis: Axis::Y, .. }))
                .count(),
            2
        );
    }

    #[test]
    fn count_and_spacing_place_the_slices() {
        let s = |count, spacing| FabSettings {
            thickness: 3.0,
            count,
            spacing,
            ..settings(Style::Stacked)
        };
        assert_eq!(
            positions(0.0, 100.0, &s(4, 12.0), false).unwrap(),
            [12.5, 37.5, 62.5, 87.5]
        );
        let spaced = positions(0.0, 100.0, &s(0, 15.0), false).unwrap();
        assert_eq!(spaced, [5.0, 20.0, 35.0, 50.0, 65.0, 80.0, 95.0]);
        // automatic: the larger of four sheets and an eighth of the span, 12.5 here
        let auto = positions(0.0, 100.0, &s(0, 0.0), false).unwrap();
        assert_eq!(auto.len(), 8);
        assert!(close(auto[0], 6.25, 1e-12) && close(auto[7], 93.75, 1e-12));
        // stacked: one sheet apart, the nearest whole number of layers centred on the body
        let st = positions(0.0, 40.0, &s(0, 0.0), true).unwrap();
        assert_eq!(st.len(), 13); // 40 / 3 rounds to 13
        assert!(
            close(st[0], 2.0, 1e-12) && close(st[12], 38.0, 1e-12),
            "{st:?}"
        );
        // a body thinner than half a sheet still gets one layer, at its middle
        assert_eq!(positions(0.0, 1.0, &s(0, 0.0), true).unwrap(), [0.5]);
        assert_eq!(positions(0.0, 1.0, &s(0, 0.0), false).unwrap(), [0.5]);
        assert!(positions(f64::NAN, 1.0, &s(0, 0.0), false).is_err());
        assert!(positions(1.0, 0.0, &s(0, 0.0), false).is_err());
    }

    #[test]
    fn automatic_rod_spacing_is_whole_sheets() {
        let s = FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            ..settings(Style::Rod)
        };
        // 200 mm: an eighth is 25, rounded to 24 (eight sheets), so the gaps are 7 rings each
        let at = positions(0.0, 200.0, &s, false).unwrap();
        assert!(
            at.windows(2).all(|w| close(w[1] - w[0], 24.0, 1e-9)),
            "{at:?}"
        );
        let res = make(&mut one_box([0.0; 3], [200.0, 60.0, 40.0]), &s).unwrap();
        assert!(
            !res.warnings
                .iter()
                .any(|w| matches!(w, Warning::GapsNotWhole { .. })),
            "{:?}",
            res.warnings
        );
        // 100 mm: 12.5 rounds to 12
        let at = positions(0.0, 100.0, &s, false).unwrap();
        assert!(close(at[1] - at[0], 12.0, 1e-9), "{at:?}");
    }

    #[test]
    fn upright_slices_are_at_least_a_sheet_apart() {
        let s = |count, spacing| FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            count,
            spacing,
            ..settings(Style::Base)
        };
        // 100 mm at 3 mm: 40 slices would be 2.5 apart
        assert!(matches!(
            positions(0.0, 100.0, &s(40, 0.0), false),
            Err(FlatError::Setting { name: "count", .. })
        ));
        assert!(matches!(
            positions(0.0, 100.0, &s(0, 2.0), false),
            Err(FlatError::Setting {
                name: "spacing",
                ..
            })
        ));
        // 33 slices are 3.03 apart
        assert_eq!(positions(0.0, 100.0, &s(33, 0.0), false).unwrap().len(), 33);
    }

    #[test]
    fn a_rod_wall_must_leave_a_spacer_ring() {
        let s = |wall| FabSettings {
            rod_diameter: 8.0,
            clearance: 0.2,
            wall,
            ..settings(Style::Rod)
        };
        assert!(matches!(
            make(&mut test_box(), &s(0.1)),
            Err(FlatError::Setting { name: "wall", .. })
        ));
        let res = make(&mut test_box(), &s(0.2)).unwrap();
        assert!(!of_kind(&res, PartKind::Spacer).is_empty());
    }

    #[test]
    fn slice_numbers_lie_on_the_plate() {
        let s = FabSettings {
            axis: Axis::X,
            count: 3,
            plate_margin: 0.0,
            ..settings(Style::Base)
        };
        let res = make(&mut test_box(), &s).unwrap();
        let plate = of_kind(&res, PartKind::Plate)[0];
        assert!(!plate.marks.is_empty());
        for m in &plate.marks {
            let lines = crate::flat::font::strokes(&m.text, m.height);
            let [x0, _, x1, _] = region::ring_bounds(&lines).unwrap();
            let dx = m.at[0] - (x0 + x1) / 2.0;
            for p in lines.iter().flatten() {
                let q = [p[0] + dx, p[1] + m.at[1]];
                assert!(
                    region::contains(std::slice::from_ref(&plate.shape), q),
                    "{} at {q:?}",
                    m.text
                );
            }
        }
    }

    #[test]
    fn whole_sheet_gaps_give_exact_spacers() {
        let rod = |count, spacing| FabSettings {
            axis: Axis::X,
            thickness: 3.0,
            count,
            spacing,
            ..settings(Style::Rod)
        };
        let gaps = |r: &FabResult| {
            r.warnings
                .iter()
                .any(|w| matches!(w, Warning::GapsNotWhole { .. }))
        };
        // spacing 15 = five sheets: 12 mm gaps of four rings each
        let res = make(&mut test_box(), &rod(0, 15.0)).unwrap();
        assert!(!gaps(&res));
        assert_eq!(of_kind(&res, PartKind::Spacer)[0].copies, 6 * 4);
        // spacing 10: 7 mm gaps, two rings a millimetre short
        let res = make(&mut test_box(), &rod(0, 10.0)).unwrap();
        assert!(gaps(&res));
        assert_eq!(res.warnings, [Warning::GapsNotWhole { thickness: 3.0 }]);
        // four slices by count: 22 mm gaps, 7 rings and a millimetre over
        let res = make(&mut test_box(), &rod(4, 0.0)).unwrap();
        assert_eq!(of_kind(&res, PartKind::Slice).len(), 4);
        assert!(gaps(&res));
        assert_eq!(of_kind(&res, PartKind::Spacer)[0].copies, 3 * 7);
        // one slice: no gaps, no spacers
        let res = make(&mut test_box(), &rod(1, 0.0)).unwrap();
        assert!(of_kind(&res, PartKind::Spacer).is_empty() && res.warnings.is_empty());
    }

    #[test]
    fn a_rod_where_asked() {
        let s = FabSettings {
            axis: Axis::Z,
            count: 2,
            rod_at: Some([10.0, 15.0]),
            ..settings(Style::Rod)
        };
        let res = make(&mut test_box(), &s).unwrap();
        for p in of_kind(&res, PartKind::Slice) {
            let c = centroid(&p.shape.holes[0]);
            assert!(close(c[0], 10.0, 1e-6) && close(c[1], 15.0, 1e-6), "{c:?}");
        }
        let spacer = of_kind(&res, PartKind::Spacer)[0];
        assert!(spacer
            .places
            .iter()
            .all(|f| f.origin[0] == 10.0 && f.origin[1] == 15.0));
        // a rod outside the model gets a hub and a bridge on every slice
        let s = FabSettings {
            rod_at: Some([-30.0, 30.0]),
            ..s
        };
        let res = make(&mut test_box(), &s).unwrap();
        assert_eq!(
            res.warnings
                .iter()
                .filter(|w| matches!(w, Warning::RodHub { .. }))
                .count(),
            2
        );
        assert_eq!(of_kind(&res, PartKind::Slice).len(), 2);
        assert_sound(&res);
    }

    #[test]
    fn piece_letters_run_past_z() {
        let got: Vec<String> = [0, 1, 25, 26, 27, 51, 52, 701, 702]
            .iter()
            .map(|&i| piece_suffix(i))
            .collect();
        assert_eq!(got, ["a", "b", "z", "aa", "ab", "az", "ba", "zz", "aaa"]);
    }

    #[test]
    fn spread_points_pick_far_apart_points_inside() {
        let sq = region::rect(0.0, 0.0, 40.0, 40.0);
        // the grid point nearest the centroid of the grid points inside (boundary points may
        // count either way, so within a grid step of the middle)
        let one = spread_points(&sq, 1);
        assert_eq!(one.len(), 1);
        assert!((one[0][0] - 20.0).hypot(one[0][1] - 20.0) <= 1.5, "{one:?}");
        let two = spread_points(&sq, 2);
        assert_eq!(two.len(), 2);
        let d = (two[0][0] - two[1][0]).hypot(two[0][1] - two[1][1]);
        assert!(d > 50.0, "{two:?}");
        assert!(spread_points(&sq, 0).is_empty());
        assert!(spread_points(&[], 3).is_empty());
        // a tiny region has one grid point: no second point apart from it
        let dot = region::rect(0.0, 0.0, 1e-9, 1e-9);
        assert!(spread_points(&dot, 3).len() <= 1);
    }

    #[test]
    fn the_kernel_sphere_slices_in_every_style() {
        let mut k = Kernel::default();
        let mut b = create_sphere(&mut k, Point3::new(0.0, 0.0, 30.0), 30.0).unwrap();
        let mut model = BrepSection::new(&k, &mut b, 0.01).unwrap();
        let cases = [
            (Style::Stacked, Axis::Z, 0),
            (Style::Rod, Axis::X, 0),
            (Style::Base, Axis::X, 3),
            (Style::Back, Axis::Y, 3),
            (Style::Waffle, Axis::Z, 3),
        ];
        for (style, axis, count) in cases {
            let s = FabSettings {
                style,
                axis,
                count,
                thickness: 6.0,
                dowels: 0,
                ..settings(Style::Stacked)
            };
            let res = make(&mut model, &s).unwrap();
            assert_sound(&res);
            assert!(
                !res.warnings
                    .iter()
                    .any(|w| matches!(w, Warning::FallsApart { .. } | Warning::NoMaterial { .. })),
                "{style:?}: {:?}",
                res.warnings
            );
            if style == Style::Stacked {
                // 60 / 6 = 10 layers; the fifth at z = 27 is a disc of radius² 900 - 9
                assert_eq!(res.parts.len(), 10);
                let a = region::shape_area(&res.parts[4].shape);
                let want = PI * 891.0;
                assert!(
                    a <= want + 1e-6 && a >= want - TAU * 891f64.sqrt() * 0.01 - 1e-6,
                    "{a}"
                );
            }
        }
    }
}
