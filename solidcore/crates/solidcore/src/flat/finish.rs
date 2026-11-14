//! A flat part as a machine cuts it, and its engraving; the parts packed onto sheets.
//!
//! A laser burns away a kerf, so a part comes out smaller by half the kerf on every edge and
//! slotted parts fall apart: [`kerf`] grows the outline and shrinks the holes to make up for
//! it. A router bit is round, so it leaves a fillet in every inside corner and a mating part
//! does not seat in a slot: [`relieve_corners`] cuts a relief into each of those corners.
//! Labels are single-stroke text ([`super::font`]) placed where they fit inside the part.
//!
//! Deviations: none (no upstream counterpart). No recursion.

use serde::{Deserialize, Serialize};

use super::region::{self, Join};
use super::relief;
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
pub struct SheetSettings {
    /// Sheet width (mm).
    pub width: f64,
    /// Sheet height (mm).
    pub height: f64,
    /// The least distance between the paths the machine follows round two parts (mm): for a
    /// laser between the burnt outlines, for a router between the bit's paths, so the parts
    /// themselves end up the bit's diameter further apart.
    pub gap: f64,
    /// The least distance from the sheet's edge (mm).
    pub margin: f64,
    /// The machine.
    pub machine: Machine,
    /// Engrave each part's name on it where the name fits.
    pub labels: bool,
    /// The capital height of the names (mm).
    pub label_height: f64,
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
/// kept sharp (corners sharper than [`region::MITER_MIN_ANGLE`], about 29 degrees, are
/// bevelled), so the part is true size once the beam has burnt its line. A negative width
/// shrinks it. The largest piece is kept if the offset splits it; a shape shrunk to nothing is
/// empty. A width of 0 leaves the shape as given. Rings may come in either direction; the result
/// has its outline counter-clockwise and its holes clockwise.
///
/// # Errors
/// [`FlatError::Setting`] (`kerf`) for a width that is not finite or beyond [`MAX_KERF`] mm
/// either way.
pub fn kerf(shape: &Shape, width: f64) -> FlatResult<Shape> {
    if !width.is_finite() || width.abs() > MAX_KERF {
        return Err(FlatError::Setting {
            name: "kerf",
            limit: "finite and at most 1000 mm either way",
        });
    }
    if width == 0.0 {
        return Ok(shape.clone());
    }
    Ok(largest(region::offset(
        &material(shape),
        width / 2.0,
        Join::Miter,
    )))
}

/// The largest kerf width [`kerf`] applies (mm): no beam is a metre wide, and offsets that far
/// leave the coordinate range the polygon engine resolves well.
pub const MAX_KERF: f64 = 1000.0;

/// The largest void angle of a corner a router relief is cut into (radians): 135°, so square
/// slots and sharper corners get one and gently bent outlines (a sliced curve) do not.
pub const RELIEF_MAX_ANGLE: f64 = 0.75 * std::f64::consts::PI;

/// How far the polygon that cuts a relief reaches past the relief's circle, at most (mm).
pub const RELIEF_CHORD: f64 = 0.005;

/// The shape with a dogbone relief cut into every inside corner a router bit of
/// `tool_diameter` mm cannot reach: a circle of the bit's radius through the corner, its centre
/// on the corner's bisector ([`crate::dogbone`]), for corners whose void angle is at most
/// [`RELIEF_MAX_ANGLE`]. The circles of every ring, outline and holes, are cut out of the
/// material in one region difference, so a relief that reaches into the next relief, through a
/// thin wall or into a hole takes what it covers there too. Each circle is cut as a polygon round
/// it ([`region::disk_around`]), so the corner, which lies on the circle, keeps no notch, and the
/// cut is over by at most [`RELIEF_CHORD`]. The largest piece is kept.
///
/// # Errors
/// [`FlatError::Kernel`] with the relief computation's [`crate::SolidError::InvalidInput`] for
/// a diameter that is not positive and finite or past 2 km, and for a shape whose material has
/// no outline of three or more points, more than a million points in a ring or a point beyond a
/// kilometre.
pub fn relieve_corners(shape: &Shape, tool_diameter: f64) -> FlatResult<Shape> {
    let options = crate::dogbone::Options {
        radius: tool_diameter / 2.0,
        style: crate::dogbone::Style::Normal,
        max_angle: RELIEF_MAX_ANGLE,
    };
    let norm = largest(material(shape));
    let mut circles: Region = Vec::new();
    for ring in norm.rings() {
        for r in relief::ring_reliefs(ring, &options)? {
            circles.extend(region::disk_around(r.centre, r.radius, RELIEF_CHORD));
        }
    }
    Ok(largest(region::difference(&[norm], &circles)))
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
    let Some([x0, y0, _, _]) = region::ring_bounds(&lines) else {
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
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
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
        h *= LABEL_SHRINK;
    }
    heights.push(height.min(MIN_LABEL));

    // the lettering scales linearly, so lay it out once per direction at height 1
    let unit = [
        label(text, [0.0, 0.0], 1.0, 0.0),
        label(text, [0.0, 0.0], 1.0, std::f64::consts::FRAC_PI_2),
    ];
    for h in heights {
        for u in &unit {
            let Some([ux0, uy0, ux1, uy1]) = region::ring_bounds(u) else {
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
    let step = (w.min(t).min(PLACE_STEP_MAX) / 2.0)
        .max((x1 - x0).min(y1 - y0) / PLACE_STEPS)
        .max(PLACE_STEP_MIN);
    let (gx0, gy0) = (x0 + w / 2.0, y0 + t / 2.0);
    let nx = grid_count(gx0, x1 - w / 2.0 + 1e-9, step);
    let ny = grid_count(gy0, y1 - t / 2.0 + 1e-9, step);
    let total = nx.checked_mul(ny)?;
    if total == 0 {
        return None;
    }
    let take = total.min(MAX_CANDIDATES);
    let mut cands: Vec<(f64, P2)> = (0..take)
        .filter_map(|k| {
            // evenly strided over the row-major grid when there are too many
            let j = if total > MAX_CANDIDATES {
                k * total / MAX_CANDIDATES
            } else {
                k
            };
            // nx is not zero (total is not)
            let (col, row) = (j.checked_rem(nx)?, j.checked_div(nx)?);
            Some([gx0 + col as f64 * step, gy0 + row as f64 * step])
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

/// The height the label search steps down to; a smaller `label_height` is engraved as given.
pub const MIN_LABEL: f64 = 2.0;

/// Each try of the label search is this much of the one before: about ten tries from a 10 mm
/// name down to [`MIN_LABEL`].
const LABEL_SHRINK: f64 = 0.85;

/// The label's candidate centres step at most half the label box's smaller side, so no gap
/// the box fits in is stepped over, and at most half of this (mm), so a big label still tries
/// the narrow places of a part.
const PLACE_STEP_MAX: f64 = 5.0;

/// The candidate grid steps at least this share of the room's smaller side: a few hundred
/// candidates across even a big part.
const PLACE_STEPS: f64 = 200.0;

/// The finest candidate step (mm): moving a label by less changes nothing worth engraving
/// and only grows the search.
const PLACE_STEP_MIN: f64 = 0.25;

/// The room (mm) a part's name keeps from its edges and holes.
pub const LABEL_MARGIN: f64 = 1.0;

/// Everything within `m` mm of the polylines, and a little more at their corners: a rectangle
/// round each segment, reaching `m` to both its sides and past both its ends (a square of side
/// `2 m` round a segment of no length).
fn line_band(lines: &[Ring], m: f64) -> Region {
    let quads: Vec<Ring> = lines
        .iter()
        .flat_map(|l| l.iter().zip(l.iter().skip(1)))
        .map(|(&a, &b)| {
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            // `u` along the segment and `n` to its left, each `m` long
            let u = if len > 0.0 {
                [(b[0] - a[0]) / len * m, (b[1] - a[1]) / len * m]
            } else {
                [m, 0.0]
            };
            let n = [-u[1], u[0]];
            vec![
                [a[0] - u[0] - n[0], a[1] - u[1] - n[1]],
                [b[0] + u[0] - n[0], b[1] + u[1] - n[1]],
                [b[0] + u[0] + n[0], b[1] + u[1] + n[1]],
                [a[0] - u[0] + n[0], a[1] - u[1] + n[1]],
            ]
        })
        .collect();
    region::from_rings(&quads, false)
}

/// The part as the machine cuts it, with its engraving: router reliefs, the slot numbers and
/// guide lines, its name (not on spacers), then the laser kerf last so the name sits on the true
/// size. Each [`super::slice::Mark`] is centred on its point's x with its baseline on its y.
/// Settings are taken as given: a kerf or tool that is not positive does nothing, a label
/// height that is not positive engraves no name.
///
/// # Errors
/// The refusals of [`relieve_corners`] and [`kerf`].
pub fn finish(part: &super::slice::FabPart, ss: &SheetSettings) -> FlatResult<Finished> {
    let mut shape = part.shape.clone();
    if let Machine::Router { tool_diameter } = ss.machine {
        if tool_diameter > 0.0 {
            shape = relieve_corners(&shape, tool_diameter)?;
        }
    }
    let mut engrave = part.engrave.clone();
    // what the name keeps clear of: the band along each guide line already on the part (not
    // its box: a guide is a neighbour's whole outline, and the name may sit inside it) and the
    // box round each mark
    let grown = |[x0, y0, x1, y1]: [f64; 4], dx: f64, dy: f64| {
        region::rect(
            x0 + dx - LABEL_MARGIN,
            y0 + dy - LABEL_MARGIN,
            x1 + dx + LABEL_MARGIN,
            y1 + dy + LABEL_MARGIN,
        )
    };
    let mut marked: Region = line_band(&part.engrave, LABEL_MARGIN);
    for m in &part.marks {
        if !(m.at[0].is_finite() && m.at[1].is_finite()) {
            continue;
        }
        let lines = super::font::strokes(&m.text, m.height);
        if let Some(b) = region::ring_bounds(&lines) {
            let dx = m.at[0] - (b[0] + b[2]) / 2.0;
            marked.extend(grown(b, dx, m.at[1]));
            engrave.extend(
                lines
                    .into_iter()
                    .map(|l| l.into_iter().map(|p| [p[0] + dx, p[1] + m.at[1]]).collect()),
            );
        }
    }
    if ss.labels && part.kind != PartKind::Spacer {
        // the pieces the guide lines and marks leave, largest first: the name goes in the
        // first that has room for it (a thin band round a neighbour's outline may have none)
        let rooms: Region = if marked.is_empty() {
            vec![shape.clone()]
        } else {
            region::pieces(&region::difference(&material(&shape), &marked), 0.0)
        };
        if let Some(name) = rooms
            .iter()
            .map(|room| label_inside(room, &part.name, ss.label_height, LABEL_MARGIN))
            .find(|name| !name.is_empty())
        {
            engrave.extend(name);
        }
    }
    if let Machine::Laser { kerf: k } = ss.machine {
        if k > 0.0 {
            shape = kerf(&shape, k)?;
        }
    }
    Ok(Finished {
        name: part.name.clone(),
        kind: part.kind,
        shape,
        engrave,
    })
}

/// A setting is finite and within `lo..=hi`.
fn within(v: f64, lo: f64, hi: f64) -> bool {
    v.is_finite() && v >= lo && v <= hi
}

/// The sheet settings checked: the first one out of range, by name.
fn check(ss: &SheetSettings) -> FlatResult<()> {
    const SIZE: &str = "more than 0 and at most 10000 mm";
    let fail = |name, limit| Err(FlatError::Setting { name, limit });
    if !(within(ss.width, 0.0, 10_000.0) && ss.width > 0.0) {
        return fail("width", SIZE);
    }
    if !(within(ss.height, 0.0, 10_000.0) && ss.height > 0.0) {
        return fail("height", SIZE);
    }
    if !within(ss.gap, 0.0, 10_000.0) {
        return fail("gap", "0 to 10000 mm");
    }
    if !within(ss.margin, 0.0, 10_000.0)
        || ss.width - 2.0 * ss.margin <= 0.0
        || ss.height - 2.0 * ss.margin <= 0.0
    {
        return fail(
            "margin",
            "0 to 10000 mm and less than half the sheet's width and height",
        );
    }
    match ss.machine {
        Machine::Laser { kerf } if !within(kerf, 0.0, 5.0) => return fail("kerf", "0 to 5 mm"),
        Machine::Router { tool_diameter } if !within(tool_diameter, 0.0, 50.0) => {
            return fail("tool_diameter", "0 to 50 mm")
        }
        _ => {}
    }
    if ss.labels && !(within(ss.label_height, 0.0, 10_000.0) && ss.label_height > 0.0) {
        return fail("label_height", SIZE);
    }
    Ok(())
}

/// Every part (every copy) finished and packed onto as few sheets as it takes, turning parts by
/// 90° where that packs better. Each sheet holds its parts' cut rings (outline and holes) and
/// engraving in sheet coordinates, and their names in the order placed. `gap` is the least
/// distance between the paths the machine follows round two parts: a laser's outline already is
/// its path, and a router's bit runs outside the part's edge, so router parts end up the bit's
/// diameter further apart.
///
/// # Errors
/// [`FlatError::Setting`] for sheet settings out of range (naming the field: `width`,
/// `height`, `gap`, `margin`, `kerf`, `tool_diameter`, `label_height`);
/// [`FlatError::TooBig`] past [`super::nest::MAX_PARTS`] copies; [`FlatError::DoesNotFit`] when a part fits
/// on no sheet, its `part` being the index of the part in `res.parts` (not of the copy);
/// the refusals of [`finish`] and the packer's other errors.
pub fn layout(res: &FabResult, ss: &SheetSettings) -> FlatResult<Vec<Sheet>> {
    check(ss)?;
    if res.count() > super::nest::MAX_PARTS as u64 {
        return Err(FlatError::TooBig("parts"));
    }
    // every copy, with the index of the part it is a copy of
    let mut pieces: Vec<(usize, Finished)> = Vec::new();
    for (i, part) in res.parts.iter().enumerate() {
        let f = finish(part, ss)?;
        for _ in 0..part.copies {
            pieces.push((i, f.clone()));
        }
    }
    let outlines: Vec<Ring> = pieces.iter().map(|(_, f)| f.shape.outer.clone()).collect();
    let apart = match ss.machine {
        Machine::Laser { .. } => ss.gap,
        Machine::Router { tool_diameter } => ss.gap + tool_diameter,
    };
    let places = super::nest::nest(&outlines, ss.width, ss.height, apart, ss.margin, true)
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
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, SQRT_2};

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

    /// The sheets these tests start from: 600 x 400 for a laser.
    fn sheet_settings() -> SheetSettings {
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

    fn laser(k: f64, labels: bool) -> SheetSettings {
        SheetSettings {
            machine: Machine::Laser { kerf: k },
            labels,
            ..sheet_settings()
        }
    }

    // --- kerf ---

    #[test]
    fn kerf_grows_a_square_and_keeps_its_corners() {
        let g = kerf(&solid(sq(0.0, 0.0, 50.0, 50.0, true)), 0.2).unwrap();
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
        let g = kerf(&holed, 0.2).unwrap();
        assert_eq!(g.holes.len(), 1);
        assert!(ring_area(&g.outer) > 0.0 && ring_area(&g.holes[0]) < 0.0);
        assert!(close(-ring_area(&g.holes[0]), 9.8 * 9.8, 1e-3));
        assert!(close(shape_area(&g), 50.2 * 50.2 - 9.8 * 9.8, 1e-3));
    }

    #[test]
    fn kerf_of_zero_leaves_the_shape_and_of_no_number_is_refused() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        for w in [0.0, -0.0] {
            assert_eq!(kerf(&s, w), Ok(s.clone()));
        }
        for w in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(kerf(&s, w), Err(FlatError::Setting { name: "kerf", .. })),
                "{w}"
            );
        }
    }

    #[test]
    fn kerf_takes_rings_in_either_direction() {
        let cw = solid(sq(0.0, 0.0, 10.0, 10.0, false));
        let g = kerf(&cw, 1.0).unwrap();
        assert!(close(shape_area(&g), 121.0, 1e-3));
        assert!(ring_area(&g.outer) > 0.0);
        // a counter-clockwise hole is still a hole
        let holed = Shape {
            outer: sq(0.0, 0.0, 50.0, 50.0, false),
            holes: vec![sq(20.0, 20.0, 30.0, 30.0, true)],
        };
        let g = kerf(&holed, 0.2).unwrap();
        assert_eq!(g.holes.len(), 1);
        assert!(close(shape_area(&g), 50.2 * 50.2 - 9.8 * 9.8, 1e-3));
    }

    #[test]
    fn kerf_beyond_a_metre_is_refused() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        for w in [1e300, -1e300, MAX_KERF * 1.001] {
            assert!(
                matches!(kerf(&s, w), Err(FlatError::Setting { name: "kerf", .. })),
                "{w}"
            );
        }
        assert!(close(
            shape_area(&kerf(&s, MAX_KERF).unwrap()),
            1010.0 * 1010.0,
            1e-3
        ));
    }

    #[test]
    fn negative_kerf_shrinks_and_can_shrink_to_nothing() {
        let s = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        assert!(close(shape_area(&kerf(&s, -2.0).unwrap()), 64.0, 1e-3));
        assert_eq!(kerf(&s, -20.0), Ok(Shape::default()));
        assert_eq!(kerf(&Shape::default(), 1.0), Ok(Shape::default()));
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
        let g = kerf(&dumbbell, -1.0).unwrap();
        assert!(close(shape_area(&g), 19.0 * 19.0, 1e-3));
        let b = region::ring_bounds([&g.outer]).unwrap();
        assert!(close(b[2], 19.5, 1e-5));
    }

    /// Two points of the material next to an inside corner `c` whose void opens towards
    /// `(vx, vy)` (each ±1), both within 1.906 mm of the centre of a radius-2 relief through the
    /// corner on its bisector: one on each wall, so a relief takes both away.
    fn bitten(c: [f64; 2], v: [f64; 2]) -> [[f64; 2]; 2] {
        [
            [c[0] - 0.2 * v[0], c[1] + 0.4 * v[1]],
            [c[0] + 0.4 * v[0], c[1] - 0.2 * v[1]],
        ]
    }

    /// The farthest a point of `out`'s rings, or the middle of one of their sides, lies inside
    /// the circle of radius `r` about `c` (mm); 0 or less when none is inside.
    fn inside_circle(out: &Shape, c: P2, r: f64) -> f64 {
        let short = |p: P2| r - (p[0] - c[0]).hypot(p[1] - c[1]);
        out.rings()
            .flat_map(|ring| ring.iter().zip(ring.iter().cycle().skip(1)))
            .map(|(a, b)| short(*a).max(short([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])))
            .fold(f64::NEG_INFINITY, f64::max)
    }

    /// How far the polygon engine can move a point of `relieve_corners`' result towards a
    /// relief's centre (mm), for a shape whose bounds hold the relief circles too. iOverlay
    /// scales the operands of each boolean by `2^(29 − ⌈log2 h⌉)`, `h` half the larger side of
    /// their bounds (`i_float`'s conservative 29-bit budget for `i32`), and rounds every point to
    /// that grid: half a step on each axis, `step / √2` at most. A point of the result is a
    /// rounded point of the material, or a rounded crossing of a relief polygon's rounded side,
    /// and each relief polygon side lies outside its circle, so a point lands at most two
    /// roundings, `step · √2`, inside the circle.
    fn grid_slack(shape: &Shape) -> f64 {
        let b = region::ring_bounds(shape.rings()).unwrap();
        let h = (b[2] - b[0]).max(b[3] - b[1]) / 2.0;
        SQRT_2 * 2f64.powi(h.log2().ceil() as i32 - 29)
    }

    /// The most a relief of radius `r` in a square inside corner cuts past its circle (mm²): its
    /// polygon lies within the circle of radius `r + RELIEF_CHORD`, so what it cuts past the
    /// circle lies in the ring between the two, on the material's side of one of the corner's
    /// walls. Each wall's line is `r sin 45°` from the centre, so on the material's side of it
    /// the ring spans an angle of at most `φ = 2 acos(r sin 45° / (r + RELIEF_CHORD))`, and the
    /// ring's area over an angle `φ` is `φ / 2 ((r + RELIEF_CHORD)² − r²)`: twice that for the
    /// two walls.
    fn square_corner_slack(r: f64) -> f64 {
        let outer = r + RELIEF_CHORD;
        let phi = 2.0 * (r * FRAC_1_SQRT_2 / outer).acos();
        phi * (outer * outer - r * r)
    }

    /// What a normal relief of radius `r` takes of a square inside corner, in closed form: two
    /// circular segments cut off by the walls at `r sin 45°` from its centre, `(π/2 − 1) r²`.
    fn square_corner_area(r: f64) -> f64 {
        (FRAC_PI_2 - 1.0) * r * r
    }

    #[test]
    fn reliefs_go_into_the_slot_bottom_corners_only() {
        // the slot (x 47..53, from y 30 up) has two inside corners, (47, 30) and (53, 30); a 4 mm
        // bit's circle runs through each with its centre on the bisector, in the slot
        let s = slot_plate();
        let out = relieve_corners(&s, 4.0).unwrap();
        assert!(out.holes.is_empty());
        for (c, v) in [([53.0, 30.0], [-1.0, 1.0]), ([47.0, 30.0], [1.0, 1.0])] {
            for p in bitten(c, v) {
                assert!(contains(&material(&s), p), "{p:?}");
                assert!(!contains(&material(&out), p), "{c:?} {p:?}");
            }
        }
        // the outside corners and the walls away from the corners keep their material
        for p in [
            [0.5, 0.5],
            [99.5, 59.5],
            [50.0, 27.0],
            [54.0, 50.0],
            [46.0, 50.0],
        ] {
            assert!(contains(&material(&out), p), "{p:?}");
        }
        // what goes is the two corners' segments, and at most the chord's slack more
        let r = 2.0;
        let removed = shape_area(&s) - shape_area(&out);
        let each = square_corner_area(r);
        assert!(
            removed >= 2.0 * each - 1e-6 && removed <= 2.0 * (each + square_corner_slack(r)),
            "{removed} vs 2 x {each}"
        );
    }

    #[test]
    fn a_square_pocket_loses_its_four_corners_to_within_the_chord() {
        // a 20 x 20 pocket in a 60 x 60 plate, a 4 mm bit
        let plate = Shape {
            outer: sq(0.0, 0.0, 60.0, 60.0, true),
            holes: vec![sq(20.0, 20.0, 40.0, 40.0, false)],
        };
        let r = 2.0;
        let out = relieve_corners(&plate, 2.0 * r).unwrap();
        assert_eq!(out.holes.len(), 1);
        // a hole's corners open into the hole: each relief's centre on the corner's bisector, in
        // the hole, r from the corner; no material is left inside a relief's circle, so no notch
        // of a corner stays
        let d = r * FRAC_1_SQRT_2;
        for c in [[20.0, 20.0], [40.0, 20.0], [40.0, 40.0], [20.0, 40.0]] {
            let v = [
                if c[0] < 30.0 { 1.0 } else { -1.0 },
                if c[1] < 30.0 { 1.0 } else { -1.0 },
            ];
            for p in bitten(c, v) {
                assert!(!contains(&material(&out), p), "{c:?} {p:?}");
            }
            let centre = [c[0] + d * v[0], c[1] + d * v[1]];
            let inside = inside_circle(&out, centre, r);
            assert!(inside <= grid_slack(&plate), "{c:?}: {inside}");
        }
        // the outline's corners are outside corners: untouched
        assert!(close(ring_area(&out.outer), 3600.0, 1e-9));
        // the four corners' segments go, and at most the chord's slack more (the polygon engine
        // resolves a 60 mm part far below the 1e-6 mm² allowed under the exact value)
        let removed = shape_area(&plate) - shape_area(&out);
        let exact = 4.0 * square_corner_area(r);
        assert!(
            removed >= exact - 1e-6 && removed <= exact + 4.0 * square_corner_slack(r),
            "{removed} vs {exact}"
        );
    }

    #[test]
    fn a_slot_narrower_than_two_bits_loses_its_whole_bottom_edge() {
        // a 3 mm slot with a 4 mm bit: the two corner circles overlap across the slot's bottom
        // edge, and each reaches past the far corner's wall too
        let s = solid(vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 10.0],
            [11.5, 10.0],
            [11.5, 5.0],
            [8.5, 5.0],
            [8.5, 10.0],
            [0.0, 10.0],
        ]);
        let r = 2.0;
        let out = relieve_corners(&s, 2.0 * r).unwrap();
        for (c, v) in [([8.5, 5.0], [1.0, 1.0]), ([11.5, 5.0], [-1.0, 1.0])] {
            for p in bitten(c, v) {
                assert!(!contains(&material(&out), p), "{c:?} {p:?}");
            }
            let d = r * FRAC_1_SQRT_2;
            let centre = [c[0] + d * v[0], c[1] + d * v[1]];
            let inside = inside_circle(&out, centre, r);
            assert!(inside <= grid_slack(&s), "{c:?}: {inside}");
        }
        // the bottom edge between the corners is gone; the material out of the circles' reach
        // stays
        assert!(!contains(&material(&out), [10.0, 4.9]));
        for p in [[1.0, 1.0], [10.0, 2.9], [6.4, 9.0], [13.6, 9.0]] {
            assert!(contains(&material(&out), p), "{p:?}");
        }
    }

    #[test]
    fn a_relief_that_reaches_into_a_hole_opens_it() {
        // an L whose inside corner (10, 10) opens to x > 10, y > 10, and a round hole of radius
        // 2.5 whose edge comes within 0.5 mm of the corner's wall x = 10. The relief circle
        // (radius 2, its centre on the bisector) reaches 0.586 mm past that wall: through it and
        // into the hole, which opens to the void
        let r = 2.0;
        let d = r * FRAC_1_SQRT_2;
        let mut hole = region::disk([7.0, 10.0 + d], 2.5, 0.001)[0].outer.clone();
        hole.reverse();
        let l = Shape {
            outer: vec![
                [0.0, 0.0],
                [30.0, 0.0],
                [30.0, 10.0],
                [10.0, 10.0],
                [10.0, 30.0],
                [0.0, 30.0],
            ],
            holes: vec![hole],
        };
        let out = relieve_corners(&l, 2.0 * r).unwrap();
        assert!(out.holes.is_empty(), "{:?}", out.holes.len());
        let inside = inside_circle(&out, [10.0 + d, 10.0 + d], r);
        assert!(inside <= grid_slack(&l), "{inside}");
        // the wall where the circle and the hole meet is cut through; the hole's far side and
        // the rest of the wall stay
        assert!(contains(&material(&l), [9.7, 10.0 + d]));
        assert!(!contains(&material(&out), [9.7, 10.0 + d]));
        for p in [[3.0, 10.0 + d], [9.75, 15.0], [9.75, 9.0]] {
            assert!(contains(&material(&out), p), "{p:?}");
        }
    }

    #[test]
    fn a_bad_bit_or_a_shape_without_material_is_refused() {
        let s = slot_plate();
        for d in [0.0, -4.0, f64::NAN, f64::INFINITY, 5e6] {
            assert!(
                matches!(
                    relieve_corners(&s, d),
                    Err(FlatError::Kernel(crate::SolidError::InvalidInput { .. }))
                ),
                "{d}"
            );
        }
        assert!(matches!(
            relieve_corners(&Shape::default(), 4.0),
            Err(FlatError::Kernel(crate::SolidError::InvalidInput { .. }))
        ));
        // an outline with no inside corner gets nothing
        let sq_shape = solid(sq(0.0, 0.0, 10.0, 10.0, true));
        assert!(close(
            shape_area(&relieve_corners(&sq_shape, 3.0).unwrap()),
            100.0,
            1e-9
        ));
    }

    // --- label ---

    #[test]
    fn label_box_starts_at_its_corner() {
        let s = label("A12", [10.0, 20.0], 5.0, 0.0);
        let b = region::ring_bounds(&s).unwrap();
        assert!(close(b[0], 10.0, 1e-12) && close(b[1], 20.0, 1e-12));
        assert!(close(b[3], 25.0, 1e-12));
        // three characters: two advances and one glyph width
        assert!(close(b[2], 10.0 + (2.0 * 0.8 + 0.6) * 5.0, 1e-12));
    }

    #[test]
    fn label_turns_upright_about_its_corner() {
        let s = label("A12", [10.0, 20.0], 5.0, FRAC_PI_2);
        let b = region::ring_bounds(&s).unwrap();
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
            assert!(
                !(9.0 < x && x < 51.0 && 9.0 < y && y < 51.0),
                "{x} {y} in the hole"
            );
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
        let b = region::ring_bounds(&s).unwrap();
        assert!(close(b[3] - b[1], 6.0, 1e-9));
    }

    #[test]
    fn label_inside_shrinks_to_fit_a_strip() {
        let strip = solid(sq(0.0, 0.0, 80.0, 6.0, true));
        let s = label_inside(&strip, "AB", 20.0, 1.0);
        assert!(!s.is_empty());
        assert_inside(&strip, &s, 1.0);
        let b = region::ring_bounds(&s).unwrap();
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
        let b = region::ring_bounds(&s).unwrap();
        let h = 5.0 * 0.85 * 0.85;
        assert!(close(b[2] - b[0], h, 1e-9));
        assert!(close(b[3] - b[1], 3.0 * h, 1e-9));
    }

    #[test]
    fn label_inside_below_the_smallest_height_keeps_it() {
        let s = label_inside(&solid(sq(0.0, 0.0, 50.0, 50.0, true)), "X", 1.5, 1.0);
        let b = region::ring_bounds(&s).unwrap();
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
        let p = part(
            "L1",
            PartKind::Slice,
            solid(sq(0.0, 0.0, 100.0, 50.0, true)),
        );
        let f = finish(&p, &laser(0.2, false)).unwrap();
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
        let f = finish(&p, &laser(0.2, true)).unwrap();
        assert!(!f.engrave.is_empty());
        assert_inside(&p.shape, &f.engrave, LABEL_MARGIN);
        assert_eq!(f.engrave, label_inside(&p.shape, "S3", 5.0, LABEL_MARGIN));
    }

    #[test]
    fn spacers_are_not_labelled() {
        let p = part(
            "SP",
            PartKind::Spacer,
            solid(sq(0.0, 0.0, 50.0, 50.0, true)),
        );
        assert!(finish(&p, &laser(0.2, true)).unwrap().engrave.is_empty());
        let plate = part(
            "Base plate",
            PartKind::Plate,
            solid(sq(0.0, 0.0, 200.0, 80.0, true)),
        );
        assert!(!finish(&plate, &laser(0.2, true))
            .unwrap()
            .engrave
            .is_empty());
    }

    #[test]
    fn the_name_keeps_clear_of_the_slot_numbers() {
        // a long plate with a row of numbers along its middle: the name goes above or below them
        let mut p = part(
            "Base plate",
            PartKind::Plate,
            solid(sq(0.0, 0.0, 200.0, 40.0, true)),
        );
        p.marks = (0..10)
            .map(|i| Mark {
                text: format!("{}", i + 1),
                at: [10.0 + 20.0 * f64::from(i), 17.0],
                height: 4.0,
            })
            .collect();
        let f = finish(&p, &laser(0.0, true)).unwrap();
        // the numbers' strokes come first; every later stroke (the name) stays out of their band
        let numbers: usize = (0..10)
            .map(|i| super::super::font::strokes(&format!("{}", i + 1), 4.0).len())
            .sum();
        let named = &f.engrave[numbers..];
        assert!(!named.is_empty());
        for p in named.iter().flatten() {
            assert!(
                p[1] < 17.0 - LABEL_MARGIN - 1e-9 || p[1] > 21.0 + LABEL_MARGIN + 1e-9,
                "{p:?}"
            );
        }
    }

    #[test]
    fn plate_marks_are_centred_on_their_point() {
        let mut p = part(
            "Base plate",
            PartKind::Plate,
            solid(sq(0.0, 0.0, 100.0, 40.0, true)),
        );
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
        let f = finish(&p, &laser(0.2, false)).unwrap();
        assert_eq!(f.engrave[0], line);
        let marks = &f.engrave[1..];
        assert_eq!(marks.len(), super::super::font::strokes("12", 4.0).len());
        let b = region::ring_bounds(marks).unwrap();
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
            ..sheet_settings()
        };
        let f = finish(&p, &ss).unwrap();
        assert_eq!(f.shape, relieve_corners(&p.shape, 3.175).unwrap());
        assert!(shape_area(&f.shape) < shape_area(&p.shape));
        assert!(!f.engrave.is_empty());
        assert_inside(&p.shape, &f.engrave, LABEL_MARGIN);
    }

    #[test]
    fn finish_refuses_a_kerf_or_bit_out_of_range_and_never_panics() {
        let p = part("L1", PartKind::Slice, slot_plate());
        let bad = [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300];
        for v in bad {
            // a kerf or bit that is not positive does nothing; one past its range is refused
            let lasered = finish(&p, &laser(v, true));
            let ss = SheetSettings {
                machine: Machine::Router { tool_diameter: v },
                label_height: v,
                ..sheet_settings()
            };
            let routed = finish(&p, &ss);
            if v > 0.0 {
                assert!(
                    matches!(lasered, Err(FlatError::Setting { name: "kerf", .. })),
                    "kerf {v}"
                );
                assert!(matches!(routed, Err(FlatError::Kernel(_))), "bit {v}");
            } else {
                assert_eq!(lasered.unwrap().shape, p.shape, "kerf {v}");
                let f = routed.unwrap();
                assert_eq!(f.shape, p.shape, "bit {v}");
                assert!(f.engrave.is_empty(), "label height {v}");
            }
        }
        // a part with no material
        let empty = part("E", PartKind::Slice, Shape::default());
        let f = finish(&empty, &laser(0.2, true)).unwrap();
        assert!(f.engrave.is_empty());
        assert_eq!(area(&[f.shape]), 0.0);
    }

    // --- layout ---

    #[test]
    fn layout_keeps_router_parts_a_bit_apart() {
        let squares = result(vec![
            part("A", PartKind::Slice, solid(sq(0.0, 0.0, 10.0, 10.0, true))),
            part("B", PartKind::Slice, solid(sq(0.0, 0.0, 10.0, 10.0, true))),
        ]);
        // the distance between the two parts' outlines on their sheet: for squares on one
        // sheet, the larger of the gaps between their bounds across x and across y
        let apart = |machine: Machine| {
            let ss = SheetSettings {
                width: 100.0,
                height: 100.0,
                gap: 3.0,
                margin: 5.0,
                machine,
                labels: false,
                label_height: 5.0,
            };
            let sheets = layout(&squares, &ss).unwrap();
            assert_eq!(sheets.len(), 1);
            let bounds: Vec<[f64; 4]> = sheets[0]
                .cuts
                .iter()
                .map(|r| {
                    r.iter()
                        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
                            [
                                b[0].min(p[0]),
                                b[1].min(p[1]),
                                b[2].max(p[0]),
                                b[3].max(p[1]),
                            ]
                        })
                })
                .collect();
            let [a, b] = bounds[..] else {
                panic!("{bounds:?}")
            };
            let dx = (b[0] - a[2]).max(a[0] - b[2]);
            let dy = (b[1] - a[3]).max(a[1] - b[3]);
            dx.max(dy)
        };
        // a 6 mm bit runs 3 mm outside each part: 3 mm between its paths is 9 between the parts
        let router = apart(Machine::Router { tool_diameter: 6.0 });
        assert!(router >= 9.0 - 1e-6, "{router}");
        // a laser's outline is its path
        let laser = apart(Machine::Laser { kerf: 0.0 });
        assert!(laser >= 3.0 - 1e-6, "{laser}");
        assert!(router > laser, "{router} {laser}");
    }

    fn result(parts: Vec<FabPart>) -> FabResult {
        FabResult {
            parts,
            warnings: Vec::new(),
            thickness: 3.0,
        }
    }

    #[test]
    fn layout_names_the_setting_out_of_range() {
        let res = result(vec![part(
            "L1",
            PartKind::Slice,
            solid(sq(0.0, 0.0, 10.0, 10.0, true)),
        )]);
        let d = sheet_settings;
        let cases: Vec<(SheetSettings, &str)> = vec![
            (SheetSettings { width: 0.0, ..d() }, "width"),
            (
                SheetSettings {
                    width: f64::NAN,
                    ..d()
                },
                "width",
            ),
            (
                SheetSettings {
                    width: 10_001.0,
                    ..d()
                },
                "width",
            ),
            (
                SheetSettings {
                    height: -1.0,
                    ..d()
                },
                "height",
            ),
            (
                SheetSettings {
                    height: f64::INFINITY,
                    ..d()
                },
                "height",
            ),
            (SheetSettings { gap: -0.1, ..d() }, "gap"),
            (
                SheetSettings {
                    gap: f64::NAN,
                    ..d()
                },
                "gap",
            ),
            (
                SheetSettings {
                    margin: -1.0,
                    ..d()
                },
                "margin",
            ),
            (
                SheetSettings {
                    margin: 200.0,
                    ..d()
                },
                "margin",
            ),
            (
                SheetSettings {
                    margin: 300.0,
                    ..d()
                },
                "margin",
            ),
            (
                SheetSettings {
                    machine: Machine::Laser { kerf: -0.1 },
                    ..d()
                },
                "kerf",
            ),
            (
                SheetSettings {
                    machine: Machine::Laser { kerf: 5.1 },
                    ..d()
                },
                "kerf",
            ),
            (
                SheetSettings {
                    machine: Machine::Laser { kerf: f64::NAN },
                    ..d()
                },
                "kerf",
            ),
            (
                SheetSettings {
                    machine: Machine::Router {
                        tool_diameter: 51.0,
                    },
                    ..d()
                },
                "tool_diameter",
            ),
            (
                SheetSettings {
                    machine: Machine::Router {
                        tool_diameter: f64::NAN,
                    },
                    ..d()
                },
                "tool_diameter",
            ),
            (
                SheetSettings {
                    label_height: 0.0,
                    ..d()
                },
                "label_height",
            ),
            (
                SheetSettings {
                    label_height: f64::NAN,
                    ..d()
                },
                "label_height",
            ),
        ];
        for (ss, name) in cases {
            assert!(
                matches!(layout(&res, &ss), Err(FlatError::Setting { name: n, .. }) if n == name),
                "{ss:?}"
            );
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
        assert_eq!(
            layout(&res, &sheet_settings()),
            Err(FlatError::TooBig("parts"))
        );
    }

    #[test]
    fn a_name_keeps_clear_of_the_guide_lines() {
        // a stacked layer with a guide line across its middle
        let mut p = part("L1", PartKind::Slice, solid(sq(0.0, 0.0, 50.0, 50.0, true)));
        p.engrave = vec![vec![[0.0, 25.0], [50.0, 25.0]]];
        let f = finish(&p, &sheet_settings()).unwrap();
        let name = f.engrave.get(1..).unwrap_or_default();
        let [x0, y0, x1, y1] = region::ring_bounds(name).expect("the name is engraved");
        let [gx0, gy0, gx1, gy1] = [
            0.0 - LABEL_MARGIN,
            25.0 - LABEL_MARGIN,
            50.0 + LABEL_MARGIN,
            25.0 + LABEL_MARGIN,
        ];
        assert!(
            x1 < gx0 || x0 > gx1 || y1 < gy0 || y0 > gy1,
            "the name {:?} crosses the guide line",
            [x0, y0, x1, y1]
        );
    }

    #[test]
    fn a_name_goes_in_a_smaller_piece_when_the_largest_has_no_room() {
        // a 3 mm strip with a 20 mm square at its end, a guide line between them: the strip
        // is the larger piece but too thin for any name
        let mut p = part(
            "L1",
            PartKind::Slice,
            solid(vec![
                [0.0, 0.0],
                [320.0, 0.0],
                [320.0, 20.0],
                [300.0, 20.0],
                [300.0, 3.0],
                [0.0, 3.0],
            ]),
        );
        p.engrave = vec![vec![[300.0, 0.0], [300.0, 3.0]]];
        let f = finish(&p, &sheet_settings()).unwrap();
        let name = f.engrave.get(1..).unwrap_or_default();
        assert!(!name.is_empty(), "no name");
        // on the square, not the strip
        for [x, _] in points(name) {
            assert!(x > 300.0, "the name reaches x {x}");
        }
    }

    #[test]
    fn layout_of_no_parts_is_no_sheets() {
        assert!(matches!(
            layout(&result(Vec::new()), &sheet_settings()),
            Ok(sheets) if sheets.is_empty()
        ));
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
        let ss = sheet_settings();
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
        let mut p = part(
            "SP",
            PartKind::Spacer,
            solid(sq(0.0, 0.0, 20.0, 20.0, true)),
        );
        p.copies = 7;
        let sheets = layout(&result(vec![p]), &sheet_settings()).unwrap();
        let n: usize = sheets.iter().map(|s| s.parts.len()).sum();
        assert_eq!(n, 7);
        assert!(sheets.iter().all(|s| s.engrave.is_empty()));
    }

    #[test]
    fn a_part_bigger_than_the_sheet_is_named_by_its_part_index() {
        let small = part("A", PartKind::Slice, solid(sq(0.0, 0.0, 10.0, 10.0, true)));
        let mut copies = part("B", PartKind::Spacer, solid(sq(0.0, 0.0, 10.0, 10.0, true)));
        copies.copies = 3;
        let big = part(
            "C",
            PartKind::Slice,
            solid(sq(0.0, 0.0, 900.0, 900.0, true)),
        );
        let err = layout(&result(vec![small, copies, big]), &sheet_settings());
        assert!(
            matches!(err, Err(FlatError::DoesNotFit { part: 2, .. })),
            "{err:?}"
        );
    }
}
