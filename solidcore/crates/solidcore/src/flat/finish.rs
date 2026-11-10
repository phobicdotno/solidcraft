//! A flat part as a machine cuts it, and its engraving; the parts packed onto sheets.
//!
//! A laser burns away a kerf, so a part comes out smaller by half the kerf on every edge and
//! slotted parts fall apart: [`kerf`] grows the outline and shrinks the holes to make up for
//! it. A router bit is round, so it leaves a fillet in every inside corner and a mating part
//! does not seat in a slot: [`relieve_corners`] cuts a relief into each of those corners.
//! Labels are single-stroke text ([`super::font`]) placed where they fit inside the part.

use serde::{Deserialize, Serialize};

use super::region::{self, Join};
use super::slice::{FabResult, PartKind};
use super::{FlatError, FlatResult, Region, Ring, Shape, P2};

/// The machine that cuts the parts.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Machine {
    /// A laser cutter burning a kerf of `kerf` mm.
    Laser {
        /// The width of the line the beam burns (mm).
        kerf: f64,
    },
    /// A CNC router with a bit of `tool_diameter` mm.
    Router {
        /// The bit's diameter (mm).
        tool_diameter: f64,
    },
}

/// The sheets and the machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SheetSettings {
    /// Sheet width (mm).
    pub width: f64,
    /// Sheet height (mm).
    pub height: f64,
    /// The least distance between parts (mm).
    pub gap: f64,
    /// The least distance from the sheet's edge (mm).
    pub margin: f64,
    /// The machine.
    pub machine: Machine,
    /// Engrave each part's name on it.
    pub labels: bool,
    /// The capital height of the names (mm).
    pub label_height: f64,
}

impl Default for SheetSettings {
    fn default() -> Self {
        SheetSettings {
            width: 600.0,
            height: 400.0,
            gap: 3.0,
            margin: 5.0,
            machine: Machine::Laser { kerf: 0.15 },
            labels: true,
            label_height: 5.0,
        }
    }
}

/// A part ready to cut: its outline and holes as the machine follows them, and its engraving.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finished {
    /// The part's name.
    pub name: String,
    /// What it is.
    pub kind: PartKind,
    /// What is cut.
    pub shape: Shape,
    /// What is engraved (open polylines).
    pub engrave: Vec<Ring>,
}

/// One sheet of parts.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sheet {
    /// Closed rings to cut, in sheet coordinates (origin at the sheet's lower-left corner).
    pub cuts: Vec<Ring>,
    /// Open polylines to engrave.
    pub engrave: Vec<Ring>,
    /// The names of the parts on it, in the order placed.
    pub parts: Vec<String>,
}

/// The material of a shape whatever the direction of its rings (even-odd: a ring inside
/// another is a hole), normalised.
fn material(shape: &Shape) -> Region {
    let rings: Vec<Ring> = shape.rings().cloned().collect();
    region::from_rings(&rings, true)
}

/// The largest piece of a region, or an empty shape.
fn largest(r: Region) -> Shape {
    r.into_iter()
        .map(|s| (region::shape_area(&s), s))
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
        .unwrap_or_default()
}

/// The shape grown by half of a laser `width` (mm): the outline out and the holes in, corners
/// kept sharp, so the part is true size once the beam has burnt its line. A negative width
/// shrinks it. The largest piece is kept if the offset splits it; a shape shrunk to nothing is
/// empty. A width of 0, not finite or beyond [`MAX_KERF`] mm either way leaves the shape as
/// given. Rings may come in either direction; the result has its outline counter-clockwise and
/// its holes clockwise.
#[must_use]
pub fn kerf(shape: &Shape, width: f64) -> Shape {
    if width == 0.0 || !width.is_finite() || width.abs() > MAX_KERF {
        return shape.clone();
    }
    largest(region::offset(&material(shape), width / 2.0, Join::Miter))
}

/// The largest kerf width [`kerf`] applies (mm): no beam is a metre wide, and offsets that far
/// leave the coordinate range the polygon engine resolves well.
pub const MAX_KERF: f64 = 1000.0;

/// The largest void angle of a corner a router relief is cut into (radians): 135°, so square
/// slots and sharper corners get one and gently bent outlines (a sliced curve) do not.
pub const RELIEF_MAX_ANGLE: f64 = 0.75 * std::f64::consts::PI;

/// How far a relief's arc may stray from its circle (mm).
pub const RELIEF_CHORD: f64 = 0.005;

/// The shape with a dogbone relief cut into every inside corner a router bit of
/// `tool_diameter` mm cannot reach: a circle of the bit's radius through the corner, its centre
/// on the corner's bisector ([`crate::dogbone`]), for corners whose void angle is at most
/// [`RELIEF_MAX_ANGLE`]. Each ring gets its arcs spliced in; where reliefs at both ends of a short
/// edge overlap, their circles are cut out of the material instead. The largest piece is kept.
/// A diameter that is not positive and finite leaves the shape as given.
#[must_use]
pub fn relieve_corners(shape: &Shape, tool_diameter: f64) -> Shape {
    let options = crate::dogbone::Options {
        radius: tool_diameter / 2.0,
        style: crate::dogbone::Style::Normal,
        max_angle: RELIEF_MAX_ANGLE,
    };
    let Ok(options) = options.checked() else {
        return shape.clone();
    };
    let norm = largest(material(shape));
    if norm.outer.len() < 3 {
        return shape.clone();
    }
    let spliced: Option<Vec<Ring>> = norm
        .rings()
        .map(|r| crate::dogbone::relieved_ring(r, &options, RELIEF_CHORD).ok())
        .collect();
    if let Some(rings) = spliced {
        return largest(region::from_rings(&rings, true));
    }
    // an edge too short for the reliefs at both its ends: cut every relief circle out instead,
    // each a polygon round the circle so the corner itself is cut
    let mut circles: Region = Vec::new();
    for ring in norm.rings() {
        let Ok(corners) = crate::dogbone::ring_reliefs(ring, &options) else {
            return norm.clone();
        };
        for c in corners {
            if let crate::dogbone::Corner2::Relief(r) = c {
                circles.extend(region::disk(r.centre, r.radius + RELIEF_CHORD, RELIEF_CHORD));
            }
        }
    }
    largest(region::difference(&[norm], &circles))
}

/// The bounds `[xmin, ymin, xmax, ymax]` of polylines, `None` without points.
fn line_bounds(lines: &[Ring]) -> Option<[f64; 4]> {
    region::ring_bounds(lines)
}

/// `text` as single-stroke lines with capitals `height` mm tall, the lower-left of its box at
/// `at` and turned `angle` radians counter-clockwise about it. Nothing for an empty text, a
/// height that is not positive, or a point or angle that is not finite.
#[must_use]
pub fn label(text: &str, at: [f64; 2], height: f64, angle: f64) -> Vec<Ring> {
    if !(at[0].is_finite() && at[1].is_finite() && angle.is_finite()) {
        return Vec::new();
    }
    let lines = super::font::strokes(text, height);
    let Some([x0, y0, _, _]) = line_bounds(&lines) else {
        return Vec::new();
    };
    let (s, c) = angle.sin_cos();
    lines
        .into_iter()
        .map(|l| {
            l.into_iter()
                .map(|[px, py]| {
                    let (u, v) = (px - x0, py - y0);
                    [at[0] + u * c - v * s, at[1] + u * s + v * c]
                })
                .collect()
        })
        .collect()
}

/// The most candidate centres [`label_inside`] tries per height and direction.
const MAX_CANDIDATES: u64 = 6000;

/// The most candidates, deepest first, whose label box is tested against the region.
const MAX_TRIES: usize = 2000;

/// The distance from `p` to the nearest edge of the region's rings.
fn depth(r: &[Shape], p: P2) -> f64 {
    let mut best = f64::INFINITY;
    for ring in r.iter().flat_map(Shape::rings) {
        let n = ring.len();
        for (i, a) in ring.iter().enumerate() {
            let Some(b) = ring.get((i + 1) % n) else {
                continue;
            };
            let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
            let ll = (ex * ex + ey * ey).max(1e-24);
            let t = (((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / ll).clamp(0.0, 1.0);
            let (dx, dy) = (p[0] - a[0] - t * ex, p[1] - a[1] - t * ey);
            best = best.min(dx.hypot(dy));
        }
    }
    best
}

/// The grid of candidate centres: `n` points from `lo` by `step` while at most `hi`.
fn grid_count(lo: f64, hi: f64, step: f64) -> u64 {
    let n = ((hi - lo) / step).floor() + 1.0;
    if n.is_finite() && n >= 1.0 {
        // bounds are within a kilometre and the step at least 0.25 mm: far below u64
        n.min(1e9) as u64
    } else {
        0
    }
}

/// `text` engraved where it fits inside the shape's material, at least `margin` mm from every
/// edge and hole: horizontal or upright, its height stepped down towards [`MIN_LABEL`] mm when
/// needed; nothing when even that does not fit.
///
/// Candidate centres lie on a grid over the shrunk material (finer for small labels; an evenly
/// strided subset of at most a few thousand on a big part, so the search is quick and the
/// result deterministic) and are tried deepest first, by their distance to the boundary, so the
/// label sits well inside. A negative or NaN margin counts as 0; one that leaves no room
/// (infinite included) gives nothing.
#[must_use]
pub fn label_inside(shape: &Shape, text: &str, height: f64, margin: f64) -> Vec<Ring> {
    if text.trim().is_empty() || !(height.is_finite() && height > 0.0) {
        return Vec::new();
    }
    let mut room = material(shape);
    if margin > 0.0 {
        // a margin of half the smaller side or more leaves no room (and keeps the offset sane)
        let [bx0, by0, bx1, by1] = region::bounds(&room).unwrap_or_default();
        if margin >= (bx1 - bx0).min(by1 - by0) / 2.0 {
            return Vec::new();
        }
        room = region::offset(&room, -margin, Join::Miter);
    }
    let Some([x0, y0, x1, y1]) = region::bounds(&room) else {
        return Vec::new();
    };
    let (rw, rh) = (x1 - x0, y1 - y0);

    let mut heights = Vec::new();
    let mut h = height;
    while h > MIN_LABEL + 1e-9 {
        heights.push(h);
        h *= 0.85;
    }
    heights.push(height.min(MIN_LABEL));

    // the lettering scales linearly, so lay it out once per direction at height 1
    let unit = [
        label(text, [0.0, 0.0], 1.0, 0.0),
        label(text, [0.0, 0.0], 1.0, std::f64::consts::FRAC_PI_2),
    ];
    for h in heights {
        for u in &unit {
            let Some([ux0, uy0, ux1, uy1]) = line_bounds(u) else {
                return Vec::new();
            };
            let (w, t) = ((ux1 - ux0) * h, (uy1 - uy0) * h);
            if !(w <= rw && t <= rh) {
                continue;
            }
            if let Some([cx, cy]) = place_box(&room, [x0, y0, x1, y1], w, t) {
                let (dx, dy) = (cx - w / 2.0 - ux0 * h, cy - t / 2.0 - uy0 * h);
                return u
                    .iter()
                    .map(|l| l.iter().map(|p| [p[0] * h + dx, p[1] * h + dy]).collect())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// The centre of a `w` x `t` box lying inside `room` (whose bounds are `b`), deepest candidate
/// first; `None` when no candidate fits.
fn place_box(room: &[Shape], b: [f64; 4], w: f64, t: f64) -> Option<P2> {
    let [x0, y0, x1, y1] = b;
    let step = (w.min(t).min(5.0) / 2.0)
        .max((x1 - x0).min(y1 - y0) / 200.0)
        .max(0.25);
    let (gx0, gy0) = (x0 + w / 2.0, y0 + t / 2.0);
    let nx = grid_count(gx0, x1 - w / 2.0 + 1e-9, step);
    let ny = grid_count(gy0, y1 - t / 2.0 + 1e-9, step);
    let total = nx.checked_mul(ny)?;
    if total == 0 {
        return None;
    }
    let take = total.min(MAX_CANDIDATES);
    let mut cands: Vec<(f64, P2)> = (0..take)
        .map(|k| {
            // evenly strided over the row-major grid when there are too many
            let j = if total > MAX_CANDIDATES {
                k * total / MAX_CANDIDATES
            } else {
                k
            };
            [
                gx0 + (j % nx) as f64 * step,
                gy0 + (j / nx) as f64 * step,
            ]
        })
        .filter(|&p| region::contains(room, p))
        .map(|p| (depth(room, p), p))
        .collect();
    cands.sort_by(|a, b| b.0.total_cmp(&a.0));
    let need = w.min(t) / 2.0;
    for (d, [cx, cy]) in cands.into_iter().take(MAX_TRIES) {
        if d < need - 1e-6 {
            // the deeper candidates are used up
            break;
        }
        let bx = region::rect(cx - w / 2.0, cy - t / 2.0, cx + w / 2.0, cy + t / 2.0);
        // what of the box sticks out of the room: under 0.0001 mm² is rounding
        if region::area(&region::difference(&bx, room)) < 1e-4 {
            return Some([cx, cy]);
        }
    }
    None
}

/// The smallest label height (mm) still engraved.
pub const MIN_LABEL: f64 = 2.0;

/// The room (mm) a part's name keeps from its edges and holes.
pub const LABEL_MARGIN: f64 = 1.0;

/// The part as the machine cuts it, with its engraving: router reliefs, the slot numbers and
/// guide lines, its name (not on spacers), then the laser kerf last so the name sits on the true
/// size. Each [`super::slice::Mark`] is centred on its point's x with its baseline on its y.
/// Settings are taken as given: a kerf or tool that is not positive does nothing, a label
/// height that is not positive engraves no name.
#[must_use]
pub fn finish(part: &super::slice::FabPart, ss: &SheetSettings) -> Finished {
    let mut shape = part.shape.clone();
    if let Machine::Router { tool_diameter } = ss.machine {
        if tool_diameter > 0.0 {
            shape = relieve_corners(&shape, tool_diameter);
        }
    }
    let mut engrave = part.engrave.clone();
    for m in &part.marks {
        if !(m.at[0].is_finite() && m.at[1].is_finite()) {
            continue;
        }
        let lines = super::font::strokes(&m.text, m.height);
        if let Some([lx0, _, lx1, _]) = line_bounds(&lines) {
            let dx = m.at[0] - (lx0 + lx1) / 2.0;
            engrave.extend(
                lines
                    .into_iter()
                    .map(|l| l.into_iter().map(|p| [p[0] + dx, p[1] + m.at[1]]).collect()),
            );
        }
    }
    if ss.labels && part.kind != PartKind::Spacer {
        engrave.extend(label_inside(&shape, &part.name, ss.label_height, LABEL_MARGIN));
    }
    if let Machine::Laser { kerf: k } = ss.machine {
        if k > 0.0 {
            shape = kerf(&shape, k);
        }
    }
    Finished {
        name: part.name.clone(),
        kind: part.kind,
        shape,
        engrave,
    }
}

/// The most parts (copies counted) one layout packs.
pub const MAX_PIECES: u64 = 10_000;

/// A setting is finite and within `lo..=hi`.
fn within(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

/// The sheet settings checked: the first one out of range, by name.
fn check(ss: &SheetSettings) -> FlatResult<()> {
    let fail = |name| Err(FlatError::Setting(name));
    if !(within(ss.width, 0.0, 10_000.0) && ss.width > 0.0) {
        return fail("width");
    }
    if !(within(ss.height, 0.0, 10_000.0) && ss.height > 0.0) {
        return fail("height");
    }
    if !within(ss.gap, 0.0, 10_000.0) {
        return fail("gap");
    }
    if !within(ss.margin, 0.0, 10_000.0)
        || ss.width - 2.0 * ss.margin <= 0.0
        || ss.height - 2.0 * ss.margin <= 0.0
    {
        return fail("margin");
    }
    match ss.machine {
        Machine::Laser { kerf } if !within(kerf, 0.0, 5.0) => return fail("kerf"),
        Machine::Router { tool_diameter } if !within(tool_diameter, 0.0, 50.0) => {
            return fail("tool_diameter")
        }
        _ => {}
    }
    if ss.labels && !(within(ss.label_height, 0.0, 10_000.0) && ss.label_height > 0.0) {
        return fail("label_height");
    }
    Ok(())
}

/// Every part (every copy) finished and packed onto as few sheets as it takes, turning parts by
/// 90° where that packs better. Each sheet holds its parts' cut rings (outline and holes) and
/// engraving in sheet coordinates, and their names in the order placed.
///
/// # Errors
/// [`FlatError::Setting`] for sheet settings out of range (naming the field: `width`,
/// `height`, `gap`, `margin`, `kerf`, `tool_diameter`, `label_height`);
/// [`FlatError::TooBig`] past [`MAX_PIECES`] copies; [`FlatError::DoesNotFit`] when a part fits
/// on no sheet, its `part` being the index of the part in `res.parts` (not of the copy);
/// the packer's other errors.
pub fn layout(res: &FabResult, ss: &SheetSettings) -> FlatResult<Vec<Sheet>> {
    check(ss)?;
    if res.count() > MAX_PIECES {
        return Err(FlatError::TooBig("parts"));
    }
    // every copy, with the index of the part it is a copy of
    let mut pieces: Vec<(usize, Finished)> = Vec::new();
    for (i, part) in res.parts.iter().enumerate() {
        let f = finish(part, ss);
        for _ in 0..part.copies {
            pieces.push((i, f.clone()));
        }
    }
    let outlines: Vec<Ring> = pieces.iter().map(|(_, f)| f.shape.outer.clone()).collect();
    let places = super::nest::nest(&outlines, ss.width, ss.height, ss.gap, ss.margin, true)
        .map_err(|e| match e {
            FlatError::DoesNotFit {
                part,
                width,
                height,
            } => FlatError::DoesNotFit {
                part: pieces.get(part).map_or(part, |p| p.0),
                width,
                height,
            },
            other => other,
        })?;
    let mut sheets: Vec<Sheet> = Vec::new();
    for pl in &places {
        let Some((_, f)) = pieces.get(pl.part) else {
            continue;
        };
        if pl.sheet >= pieces.len() {
            continue;
        }
        while sheets.len() <= pl.sheet {
            sheets.push(Sheet::default());
        }
        let Some(sh) = sheets.get_mut(pl.sheet) else {
            continue;
        };
        sh.cuts
            .extend(f.shape.rings().map(|r| super::nest::transform(r, pl)));
        sh.engrave
            .extend(f.engrave.iter().map(|l| super::nest::transform(l, pl)));
        sh.parts.push(f.name.clone());
    }
    Ok(sheets)
}

#[cfg(test)]
mod tests {
    use super::super::region::{area, contains, ring_area, shape_area};
    use super::super::slice::{Axis, FabPart, Mark};
    use super::*;

    fn sq(x0: f64, y0: f64, x1: f64, y1: f64, ccw: bool) -> Ring {
        let mut r = vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        if !ccw {
            r.reverse();
        }
        r
    }

    fn solid(outer: Ring) -> Shape {
        Shape {
            outer,
            holes: Vec::new(),
        }
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn points(lines: &[Ring]) -> Vec<P2> {
        lines.iter().flatten().copied().collect()
    }

    /// Plate 100 x 60 with a 6 x 30 slot cut in from the top edge.
    fn slot_plate() -> Shape {
        solid(vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 60.0],
            [53.0, 60.0],
            [53.0, 30.0],
            [47.0, 30.0],
            [47.0, 60.0],
            [0.0, 60.0],
        ])
    }

    fn ring_plate() -> Shape {
        Shape {
            outer: sq(0.0, 0.0, 60.0, 60.0, true),
            holes: vec![sq(10.0, 10.0, 50.0, 50.0, false)],
        }
    }

    /// Every engraved point lies in the shape's material, at least `margin` from its boundary.
    fn assert_inside(shape: &Shape, lines: &[Ring], margin: f64) {
        let m = material(shape);
        for p in points(lines) {
            assert!(contains(&m, p), "{p:?} outside");
            assert!(depth(&m, p) >= margin - 1e-6, "{p:?} too close to an edge");
        }
    }

    fn part(name: &str, kind: PartKind, shape: Shape) -> FabPart {
        FabPart {
            name: name.to_string(),
            kind,
            shape,
            frame: Axis::Z.frame(0.0),
            engrave: Vec::new(),
            marks: Vec::new(),
            copies: 1,
            places: Vec::new(),
        }
    }

    fn laser(k: f64, labels: bool) -> SheetSettings {
        SheetSettings {
            machine: Machine::Laser { kerf: k },
            labels,
            ..SheetSettings::default()
        }
    }

    // --- kerf ---

    #[test]
    fn kerf_grows_a_square_and_keeps_its_corners() {
        let g = kerf(&solid(sq(0.0, 0.0, 50.0, 50.0, true)), 0.2);
        assert!(close(shape_area(&g), 50.2 * 50.2, 1e-3));
        assert_eq!(g.outer.len(), 4);
        assert!(g.holes.is_empty());
        let b = region::ring_bounds([&g.outer]).unwrap();
        for (v, e) in b.iter().zip([-0.1, -0.1, 50.1, 50.1]) {
            assert!(close(*v, e, 1e-5));
        }
    }

    #[test]
    fn kerf_shrinks_holes() {
        let holed = Shape {
            outer: sq(0.0, 0.0, 50.0, 50.0, true),
            holes: vec![sq(20.0, 20.0, 30.0, 30.0, false)],
        };
        let g = kerf(&holed, 0.2);
        assert_eq!(g.holes.len(), 1);
        assert!(ring_area(&g.outer) > 0.0 && ring_area(&g.holes[0]) < 0.0);
        assert!(close(-ring_area(&g.holes[0]), 9.8 * 9.8, 1e-3));
        assert!(close(shape_area(&g), 50.2 * 50.2 - 9.8 * 9.8, 1e-3));
    }

    #[test]
    fn kerf_of_zero_or_not_finite_leaves_the_shape() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        for w in [0.0, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(kerf(&s, w), s);
        }
    }

    #[test]
    fn kerf_takes_rings_in_either_direction() {
        let cw = solid(sq(0.0, 0.0, 10.0, 10.0, false));
        let g = kerf(&cw, 1.0);
        assert!(close(shape_area(&g), 121.0, 1e-3));
        assert!(ring_area(&g.outer) > 0.0);
        // a counter-clockwise hole is still a hole
        let holed = Shape {
            outer: sq(0.0, 0.0, 50.0, 50.0, false),
            holes: vec![sq(20.0, 20.0, 30.0, 30.0, true)],
        };
        let g = kerf(&holed, 0.2);
        assert_eq!(g.holes.len(), 1);
        assert!(close(shape_area(&g), 50.2 * 50.2 - 9.8 * 9.8, 1e-3));
    }

    #[test]
    fn kerf_beyond_a_metre_leaves_the_shape() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        assert_eq!(kerf(&s, 1e300), s);
        assert_eq!(kerf(&s, -1e300), s);
        assert!(close(shape_area(&kerf(&s, MAX_KERF)), 1010.0 * 1010.0, 1e-3));
    }

    #[test]
    fn negative_kerf_shrinks_and_can_shrink_to_nothing() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        assert!(close(shape_area(&kerf(&s, -2.0)), 64.0, 1e-3));
        assert_eq!(kerf(&s, -20.0), Shape::default());
        assert_eq!(kerf(&Shape::default(), 1.0), Shape::default());
    }

    #[test]
    fn kerf_keeps_the_largest_piece_when_it_splits() {
        // a 20 mm square joined to a 10 mm square by a 0.4 mm neck: shrinking by 0.5 cuts it
        let dumbbell = solid(vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 9.8],
            [30.0, 9.8],
            [30.0, 5.0],
            [40.0, 5.0],
            [40.0, 15.0],
            [30.0, 15.0],
            [30.0, 10.2],
            [20.0, 10.2],
            [20.0, 20.0],
            [0.0, 20.0],
        ]);
        let g = kerf(&dumbbell, -1.0);
        assert!(close(shape_area(&g), 19.0 * 19.0, 1e-3));
        let b = region::ring_bounds([&g.outer]).unwrap();
        assert!(close(b[2], 19.5, 1e-5));
    }

    #[test]
    fn reliefs_go_into_the_slot_bottom_corners_only() {
        // the slot is 6 wide and 30 deep: its two bottom corners (47, 30) and (53, 30) are the
        // only inside corners; a 4 mm bit's circle of radius 2 passes through each corner with
        // its centre on the bisector, 2 mm from it, into the empty side
        let s = slot_plate();
        let out = relieve_corners(&s, 4.0);
        assert!(out.holes.is_empty());
        let r = 2.0;
        let d = r / std::f64::consts::SQRT_2;
        for (corner, centre) in [([53.0, 30.0], [53.0 + d, 30.0 - d]), ([47.0, 30.0], [47.0 - d, 30.0 - d])] {
            // just past the corner, inside the circle: cut away
            let inside = [corner[0] + (centre[0] - corner[0]) * 0.5, corner[1] + (centre[1] - corner[1]) * 0.5];
            assert!(region::contains(&material(&s), inside));
            assert!(!region::contains(&material(&out), inside), "{corner:?}");
        }
        // the outside corners and the walls away from the corners keep their material
        for p in [[1.0, 1.0], [99.0, 59.0], [50.0, 28.0], [55.0, 50.0], [45.0, 50.0]] {
            assert!(region::contains(&material(&out), p), "{p:?}");
        }
        // what goes is two lenses, each less than a half circle
        let removed = shape_area(&s) - shape_area(&out);
        assert!(removed > 0.5 && removed < std::f64::consts::PI * r * r, "{removed}");
        // the closed form of the dogbone primitive for a square corner
        let opts = crate::dogbone::Options { radius: r, style: crate::dogbone::Style::Normal, max_angle: RELIEF_MAX_ANGLE };
        if let Some(each) = crate::dogbone::removed_area(std::f64::consts::FRAC_PI_2, &opts) {
            assert!(close(removed, 2.0 * each, 0.02), "{removed} vs 2 x {each}");
        }
    }

    #[test]
    fn reliefs_go_into_every_corner_of_a_square_hole() {
        let plate = Shape {
            outer: sq(0.0, 0.0, 60.0, 60.0, true),
            holes: vec![sq(20.0, 20.0, 40.0, 40.0, false)],
        };
        let out = relieve_corners(&plate, 4.0);
        assert_eq!(out.holes.len(), 1);
        // each hole corner gets a bite past it, diagonally out of the hole
        for c in [[20.0, 20.0], [40.0, 20.0], [40.0, 40.0], [20.0, 40.0]] {
            let out_dir = [if c[0] < 30.0 { -0.5 } else { 0.5 }, if c[1] < 30.0 { -0.5 } else { 0.5 }];
            let p = [c[0] + out_dir[0], c[1] + out_dir[1]];
            assert!(!region::contains(&material(&out), p), "{c:?}");
        }
        // the outline's corners are outside corners: untouched
        assert!(close(region::ring_area(&out.outer), 3600.0, 1e-9));
    }

    #[test]
    fn a_slot_narrower_than_two_bits_is_relieved_by_a_boolean() {
        // a 3 mm slot with a 4 mm bit: the two corner circles overlap on the slot's bottom edge
        let s = solid(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 10.0], [11.5, 10.0], [11.5, 5.0], [8.5, 5.0], [8.5, 10.0], [0.0, 10.0]]);
        let out = relieve_corners(&s, 4.0);
        assert!(shape_area(&out) < shape_area(&s));
        assert!(!region::contains(&material(&out), [8.6, 4.9]));
        assert!(!region::contains(&material(&out), [11.4, 4.9]));
        assert!(region::contains(&material(&out), [1.0, 1.0]));
    }

    #[test]
    fn no_bit_or_a_bad_bit_leaves_the_shape() {
        let s = slot_plate();
        for d in [0.0, -4.0, f64::NAN, f64::INFINITY] {
            assert_eq!(relieve_corners(&s, d), s);
        }
        // an outline with no inside corner gets nothing
        let sq_shape = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        assert!(close(shape_area(&relieve_corners(&sq_shape, 3.0)), 100.0, 1e-9));
    }

    // --- label ---

    #[test]
    fn label_box_starts_at_its_corner() {
        let s = label("A12", [10.0, 20.0], 5.0, 0.0);
        let b = line_bounds(&s).unwrap();
        assert!(close(b[0], 10.0, 1e-12) && close(b[1], 20.0, 1e-12));
        assert!(close(b[3], 25.0, 1e-12));
        // three characters: two advances and one glyph width
        assert!(close(b[2], 10.0 + (2.0 * 0.8 + 0.6) * 5.0, 1e-12));
    }

    #[test]
    fn label_turns_upright_about_its_corner() {
        let s = label("A12", [10.0, 20.0], 5.0, std::f64::consts::FRAC_PI_2);
        let b = line_bounds(&s).unwrap();
        assert!(close(b[2], 10.0, 1e-9) && close(b[1], 20.0, 1e-9));
        assert!(close(b[2] - b[0], 5.0, 1e-9));
        assert!(close(b[3] - b[1], 11.0, 1e-9));
    }

    #[test]
    fn label_of_nothing_or_hostile_input_is_empty() {
        assert!(label("", [0.0, 0.0], 5.0, 0.0).is_empty());
        assert!(label("   ", [0.0, 0.0], 5.0, 0.0).is_empty());
        assert!(label("A", [f64::NAN, 0.0], 5.0, 0.0).is_empty());
        assert!(label("A", [0.0, f64::INFINITY], 5.0, 0.0).is_empty());
        assert!(label("A", [0.0, 0.0], 5.0, f64::NAN).is_empty());
        assert!(label("A", [0.0, 0.0], -5.0, 0.0).is_empty());
    }

    // --- label_inside ---

    #[test]
    fn label_inside_avoids_the_hole_of_a_frame() {
        let p = ring_plate();
        let s = label_inside(&p, "P7", 5.0, 1.0);
        assert!(!s.is_empty());
        assert_inside(&p, &s, 1.0);
        for [x, y] in points(&s) {
            assert!(!(9.0 < x && x < 51.0 && 9.0 < y && y < 51.0), "{x} {y} in the hole");
        }
    }

    #[test]
    fn label_inside_avoids_a_slot() {
        let p = slot_plate();
        let s = label_inside(&p, "PART 12", 6.0, 1.0);
        assert!(!s.is_empty());
        assert_inside(&p, &s, 1.0);
        for [x, y] in points(&s) {
            assert!(!(46.0 < x && x < 54.0 && y > 29.0));
        }
        // it had room for the full height
        let b = line_bounds(&s).unwrap();
        assert!(close(b[3] - b[1], 6.0, 1e-9));
    }

    #[test]
    fn label_inside_shrinks_to_fit_a_strip() {
        let strip = solid(sq(0.0, 0.0, 80.0, 6.0, true));
        let s = label_inside(&strip, "AB", 20.0, 1.0);
        assert!(!s.is_empty());
        assert_inside(&strip, &s, 1.0);
        let b = line_bounds(&s).unwrap();
        // 20 x 0.85^10 = 3.94 mm is the first step under the 4 mm of room
        assert!(close(b[3] - b[1], 20.0 * 0.85f64.powi(10), 1e-9));
    }

    #[test]
    fn label_inside_gives_up_when_nothing_fits() {
        assert!(label_inside(&solid(sq(0.0, 0.0, 3.0, 3.0, true)), "AB", 5.0, 1.0).is_empty());
    }

    #[test]
    fn label_inside_turns_upright_in_a_tall_strip() {
        // 4 mm of room across: "ABCD" is 3 heights long, so it only fits upright
        let strip = solid(sq(0.0, 0.0, 6.0, 80.0, true));
        let s = label_inside(&strip, "ABCD", 5.0, 1.0);
        assert!(!s.is_empty());
        assert_inside(&strip, &s, 1.0);
        let b = line_bounds(&s).unwrap();
        let h = 5.0 * 0.85 * 0.85;
        assert!(close(b[2] - b[0], h, 1e-9));
        assert!(close(b[3] - b[1], 3.0 * h, 1e-9));
    }

    #[test]
    fn label_inside_below_the_smallest_height_keeps_it() {
        let s = label_inside(&solid(sq(0.0, 0.0, 50.0, 50.0, true)), "X", 1.5, 1.0);
        let b = line_bounds(&s).unwrap();
        assert!(close(b[3] - b[1], 1.5, 1e-9));
    }

    #[test]
    fn label_inside_hostile_input_is_empty_or_inside() {
        let p = solid(sq(0.0, 0.0, 40.0, 20.0, true));
        for h in [0.0, -3.0, f64::NAN, f64::INFINITY] {
            assert!(label_inside(&p, "A1", h, 1.0).is_empty());
        }
        assert!(label_inside(&p, "", 5.0, 1.0).is_empty());
        assert!(label_inside(&p, "  ", 5.0, 1.0).is_empty());
        assert!(label_inside(&p, "\u{263a}", 5.0, 1.0).is_empty());
        assert!(label_inside(&Shape::default(), "A1", 5.0, 1.0).is_empty());
        assert!(label_inside(&p, "A1", 5.0, 1e9).is_empty());
        assert!(label_inside(&p, "A1", 5.0, f64::INFINITY).is_empty());
        assert!(label_inside(&p, "A1", 5.0, 10.0).is_empty());
        // a margin that is not positive counts as none
        for m in [f64::NAN, -5.0, 0.0] {
            let s = label_inside(&p, "A1", 5.0, m);
            assert!(!s.is_empty());
            assert_inside(&p, &s, 0.0);
        }
        // a huge height steps down until it fits
        let s = label_inside(&p, "A1", 1e300, 1.0);
        assert!(!s.is_empty());
        assert_inside(&p, &s, 1.0);
        // a non-finite point in the shape
        let bad = solid(vec![[0.0, 0.0], [f64::NAN, 0.0], [10.0, 10.0]]);
        assert!(label_inside(&bad, "A", 2.0, 0.0).is_empty());
    }

    #[test]
    fn label_inside_on_a_long_part_is_quick_and_deterministic() {
        // 40 000 x 2 candidate centres: an evenly strided subset is searched
        let strip = solid(sq(0.0, 0.0, 100_000.0, 10.0, true));
        let a = label_inside(&strip, "X1", 5.0, 1.0);
        assert!(!a.is_empty());
        assert_inside(&strip, &a, 1.0);
        assert_eq!(a, label_inside(&strip, "X1", 5.0, 1.0));
    }

    // --- finish ---

    #[test]
    fn laser_grows_a_part_by_half_the_kerf_each_side() {
        let p = part("L1", PartKind::Slice, solid(sq(0.0, 0.0, 100.0, 50.0, true)));
        let f = finish(&p, &laser(0.2, false));
        let b = region::ring_bounds([&f.shape.outer]).unwrap();
        assert!(close(b[2] - b[0], 100.2, 1e-5) && close(b[3] - b[1], 50.2, 1e-5));
        assert!(close(b[0], -0.1, 1e-5));
        assert!(f.engrave.is_empty());
        assert_eq!(f.name, "L1");
        assert_eq!(f.kind, PartKind::Slice);
    }

    #[test]
    fn names_are_engraved_inside_the_true_size() {
        let p = part("S3", PartKind::Slice, ring_plate());
        let f = finish(&p, &laser(0.2, true));
        assert!(!f.engrave.is_empty());
        assert_inside(&p.shape, &f.engrave, LABEL_MARGIN);
        assert_eq!(f.engrave, label_inside(&p.shape, "S3", 5.0, LABEL_MARGIN));
    }

    #[test]
    fn spacers_are_not_labelled() {
        let p = part("SP", PartKind::Spacer, solid(sq(0.0, 0.0, 50.0, 50.0, true)));
        assert!(finish(&p, &laser(0.2, true)).engrave.is_empty());
        let plate = part("Base plate", PartKind::Plate, solid(sq(0.0, 0.0, 200.0, 80.0, true)));
        assert!(!finish(&plate, &laser(0.2, true)).engrave.is_empty());
    }

    #[test]
    fn plate_marks_are_centred_on_their_point() {
        let mut p = part("Base plate", PartKind::Plate, solid(sq(0.0, 0.0, 100.0, 40.0, true)));
        let line = vec![[1.0, 1.0], [2.0, 2.0]];
        p.engrave = vec![line.clone()];
        p.marks = vec![
            Mark {
                text: "12".into(),
                at: [40.0, 10.0],
                height: 4.0,
            },
            Mark {
                text: "bad".into(),
                at: [f64::NAN, 10.0],
                height: 4.0,
            },
            Mark {
                text: "zero".into(),
                at: [10.0, 10.0],
                height: 0.0,
            },
        ];
        let f = finish(&p, &laser(0.2, false));
        assert_eq!(f.engrave[0], line);
        let marks = &f.engrave[1..];
        assert_eq!(marks.len(), super::super::font::strokes("12", 4.0).len());
        let b = line_bounds(marks).unwrap();
        assert!(close((b[0] + b[2]) / 2.0, 40.0, 1e-12));
        assert!(close(b[1], 10.0, 1e-12) && close(b[3], 14.0, 1e-12));
    }

    #[test]
    fn router_relieves_the_slot_and_still_labels() {
        let p = part("X2", PartKind::Slice, slot_plate());
        let ss = SheetSettings {
            machine: Machine::Router {
                tool_diameter: 3.175,
            },
            ..SheetSettings::default()
        };
        let f = finish(&p, &ss);
        assert_eq!(f.shape, relieve_corners(&p.shape, 3.175));
        assert!(shape_area(&f.shape) < shape_area(&p.shape));
        assert!(!f.engrave.is_empty());
        assert_inside(&p.shape, &f.engrave, LABEL_MARGIN);
    }

    #[test]
    fn finish_never_panics_on_hostile_settings() {
        let p = part("L1", PartKind::Slice, slot_plate());
        let bad = [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300];
        for v in bad {
            let f = finish(&p, &laser(v, true));
            if v.is_nan() || v <= 0.0 || v > MAX_KERF {
                assert_eq!(f.shape, p.shape, "kerf {v}");
            }
            let ss = SheetSettings {
                machine: Machine::Router { tool_diameter: v },
                label_height: v,
                ..SheetSettings::default()
            };
            let f = finish(&p, &ss);
            assert_eq!(f.shape, p.shape);
            if v.is_nan() || v <= 0.0 || v.is_infinite() {
                assert!(f.engrave.is_empty(), "label height {v}");
            }
        }
        // a part with no material
        let empty = part("E", PartKind::Slice, Shape::default());
        let f = finish(&empty, &laser(0.2, true));
        assert!(f.engrave.is_empty());
        assert_eq!(area(&[f.shape]), 0.0);
    }

    // --- layout ---

    fn result(parts: Vec<FabPart>) -> FabResult {
        FabResult {
            parts,
            warnings: Vec::new(),
            thickness: 3.0,
        }
    }

    #[test]
    fn layout_names_the_setting_out_of_range() {
        let res = result(vec![part("L1", PartKind::Slice, solid(sq(0.0, 0.0, 10.0, 10.0, true)))]);
        let d = SheetSettings::default;
        let cases: Vec<(SheetSettings, &str)> = vec![
            (SheetSettings { width: 0.0, ..d() }, "width"),
            (SheetSettings { width: f64::NAN, ..d() }, "width"),
            (SheetSettings { width: 10_001.0, ..d() }, "width"),
            (SheetSettings { height: -1.0, ..d() }, "height"),
            (SheetSettings { height: f64::INFINITY, ..d() }, "height"),
            (SheetSettings { gap: -0.1, ..d() }, "gap"),
            (SheetSettings { gap: f64::NAN, ..d() }, "gap"),
            (SheetSettings { margin: -1.0, ..d() }, "margin"),
            (SheetSettings { margin: 200.0, ..d() }, "margin"),
            (SheetSettings { margin: 300.0, ..d() }, "margin"),
            (SheetSettings { machine: Machine::Laser { kerf: -0.1 }, ..d() }, "kerf"),
            (SheetSettings { machine: Machine::Laser { kerf: 5.1 }, ..d() }, "kerf"),
            (SheetSettings { machine: Machine::Laser { kerf: f64::NAN }, ..d() }, "kerf"),
            (
                SheetSettings { machine: Machine::Router { tool_diameter: 51.0 }, ..d() },
                "tool_diameter",
            ),
            (
                SheetSettings { machine: Machine::Router { tool_diameter: f64::NAN }, ..d() },
                "tool_diameter",
            ),
            (SheetSettings { label_height: 0.0, ..d() }, "label_height"),
            (SheetSettings { label_height: f64::NAN, ..d() }, "label_height"),
        ];
        for (ss, name) in cases {
            assert_eq!(layout(&res, &ss), Err(FlatError::Setting(name)), "{ss:?}");
        }
        // labels off: the label height does not matter
        let ss = SheetSettings {
            labels: false,
            label_height: f64::NAN,
            ..d()
        };
        assert!(check(&ss).is_ok());
        assert!(check(&d()).is_ok());
    }

    #[test]
    fn layout_refuses_too_many_copies() {
        let mut p = part("SP", PartKind::Spacer, solid(sq(0.0, 0.0, 5.0, 5.0, true)));
        p.copies = u32::MAX;
        let res = result(vec![p]);
        assert_eq!(layout(&res, &SheetSettings::default()), Err(FlatError::TooBig("parts")));
    }

    #[test]
    fn layout_of_no_parts_is_no_sheets_or_a_packer_error() {
        // the packer decides what an empty list gives; it must not panic
        let _ = layout(&result(Vec::new()), &SheetSettings::default());
    }

    #[test]
    fn every_part_lands_on_a_sheet_inside_its_edges() {
        let mut parts = Vec::new();
        for i in 0..6 {
            let s = Shape {
                outer: sq(0.0, 0.0, 120.0, 80.0, true),
                holes: vec![sq(50.0, 30.0, 70.0, 50.0, false)],
            };
            parts.push(part(&format!("L{i}"), PartKind::Slice, s));
        }
        let ss = SheetSettings::default();
        let sheets = layout(&result(parts), &ss).unwrap();
        let names: Vec<&String> = sheets.iter().flat_map(|s| &s.parts).collect();
        assert_eq!(names.len(), 6);
        for sh in &sheets {
            // outline and hole of each part
            assert_eq!(sh.cuts.len(), 2 * sh.parts.len());
            assert!(!sh.engrave.is_empty());
            for p in sh.cuts.iter().chain(&sh.engrave).flatten() {
                assert!(p[0] >= ss.margin - 1e-6 && p[0] <= ss.width - ss.margin + 1e-6);
                assert!(p[1] >= ss.margin - 1e-6 && p[1] <= ss.height - ss.margin + 1e-6);
            }
        }
    }

    #[test]
    fn every_spacer_copy_is_placed() {
        let mut p = part("SP", PartKind::Spacer, solid(sq(0.0, 0.0, 20.0, 20.0, true)));
        p.copies = 7;
        let sheets = layout(&result(vec![p]), &SheetSettings::default()).unwrap();
        let n: usize = sheets.iter().map(|s| s.parts.len()).sum();
        assert_eq!(n, 7);
        assert!(sheets.iter().all(|s| s.engrave.is_empty()));
    }

    #[test]
    fn a_part_bigger_than_the_sheet_is_named_by_its_part_index() {
        let small = part("A", PartKind::Slice, solid(sq(0.0, 0.0, 10.0, 10.0, true)));
        let mut copies = part("B", PartKind::Spacer, solid(sq(0.0, 0.0, 10.0, 10.0, true)));
        copies.copies = 3;
        let big = part("C", PartKind::Slice, solid(sq(0.0, 0.0, 900.0, 900.0, true)));
        let err = layout(&result(vec![small, copies, big]), &SheetSettings::default());
        assert!(matches!(err, Err(FlatError::DoesNotFit { part: 2, .. })), "{err:?}");
    }
}
