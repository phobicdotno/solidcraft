//! Dogbone reliefs: the round bite a router's tool cuts into an inside corner so that a square
//! part seats in it.
//!
//! Not upstream. A round tool of radius `r` cannot cut a sharp inside corner: it leaves a round
//! of radius `r` there, and a mating part with a square corner does not go in. A relief is a
//! circle of the tool's radius placed so that the tool reaches (or nearly reaches) the sharp
//! corner. This module computes where those circles go, for the concave corners of a planar loop
//! ([`ring_reliefs`], [`relieved_ring`]) and for a concave straight edge between two planes of a
//! solid ([`edge_corner`], [`edge_relief`]). Both reduce to one wedge computation
//! ([`wedge_relief`]): one implementation, whatever the caller.
//!
//! A corner is described from the void's side: `a` and `b` are the unit directions of its two
//! walls away from the corner point, the void (where the tool runs) is the wedge between them, of
//! angle `θ` in (0, π), and the material is outside it. The styles:
//! - [`Style::Normal`]: the centre on the wedge's bisector, at `r` from the corner: the circle
//!   passes through the corner. Of all circles of radius `r` through the corner with their centre
//!   in the wedge, this one removes the least material (the area beyond the walls is
//!   `r² (π − θ − sin θ)`, smallest for the centre on the bisector).
//! - [`Style::Minimal`]: the centre on the bisector at `r + gap`: the bite stops `gap` short of
//!   the sharp corner, for a mating corner that is slightly broken. `gap = 0` is `Normal`.
//! - [`Style::TBone`]: the centre on one wall's line, at `r` from the corner: the circle passes
//!   through the corner and the bite is cut into that wall only (at θ ≤ 90°, tangent to the other
//!   wall at θ = 90°), so the relief hides along one side.
//!
//! Every function checks its input and returns [`SolidError::InvalidInput`] for a number that is
//! not finite or out of range; nothing here panics on input.

use crate::{Result, SolidError};

const OP: &str = "dogbone";

/// Largest radius, gap or coordinate magnitude accepted (mm): far past any real tool or part.
pub const MAX_SIZE: f64 = 1.0e6;

/// Most vertices in a ring.
pub const MAX_RING: usize = 1_000_000;

/// Most points [`relieved_ring`] writes for one relief's arc.
pub const MAX_ARC_POINTS: usize = 4096;

/// Below this (relative to the walls' lengths), a turn is no corner: the walls are in line.
const STRAIGHT: f64 = 1.0e-9;

/// Angular slack (radians) on the largest corner angle relieved, so a corner computed at 90°
/// plus rounding is still a 90° corner.
pub const ANGLE_SLACK: f64 = 1.0e-6;

/// Which wall a T-bone's bite is cut into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    /// The longer of the two walls (the first, `a`, when they are equally long).
    #[default]
    Longer,
    /// The shorter of the two walls (the second, `b`, when they are equally long).
    Shorter,
}

/// Where the relief circle goes in its corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Style {
    /// On the bisector, through the corner.
    Normal,
    /// On the bisector, stopping `gap` short of the corner (`gap >= 0`).
    Minimal {
        /// How far short of the sharp corner the circle stops (mm).
        gap: f64,
    },
    /// On one wall's line, through the corner.
    TBone {
        /// Which wall the bite goes into.
        side: Side,
    },
}

/// What a relief is cut with and which corners get one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// The tool's radius (mm), positive.
    pub radius: f64,
    /// Where the circle goes.
    pub style: Style,
    /// The largest corner (void wedge) angle relieved, radians in (0, π]: a corner more open than
    /// this (plus [`ANGLE_SLACK`]) is skipped. π/2 relieves square and sharper corners.
    pub max_angle: f64,
}

impl Options {
    /// The options checked: a positive finite radius, a finite gap at least 0, an angle in (0, π].
    ///
    /// # Errors
    /// [`SolidError::InvalidInput`] naming the value out of range.
    pub fn checked(self) -> Result<Options> {
        if !(self.radius.is_finite() && self.radius > 0.0 && self.radius <= MAX_SIZE) {
            return Err(invalid("the tool radius must be positive and finite"));
        }
        if let Style::Minimal { gap } = self.style {
            if !(gap.is_finite() && (0.0..=MAX_SIZE).contains(&gap)) {
                return Err(invalid(
                    "the minimal style's gap must be finite and at least 0",
                ));
            }
        }
        if !(self.max_angle.is_finite()
            && self.max_angle > 0.0
            && self.max_angle <= std::f64::consts::PI)
        {
            return Err(invalid("the largest corner angle must be in (0°, 180°]"));
        }
        Ok(self)
    }
}

/// Why a corner gets no relief.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Skip {
    /// The corner is convex (an outside corner): a round tool cuts it sharp.
    Convex,
    /// The walls are in line: no corner.
    Straight,
    /// The corner is more open than the largest angle relieved: its angle (radians).
    TooOpen {
        /// The corner's (void wedge's) angle, radians.
        angle: f64,
    },
    /// A minimal relief whose gap is so large that the circle reaches neither wall.
    Unreached,
    /// A wall of zero length, or walls folded back onto each other.
    Degenerate,
}

/// A relief in a corner, in the corner's own terms: offsets from the corner point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wedge {
    /// The corner's (void wedge's) angle, radians in (0, π).
    pub angle: f64,
    /// The circle's centre less the corner point.
    pub centre: [f64; 3],
    /// Where the circle crosses wall `a`, as distances from the corner along it: `[near, far]`
    /// with `0 <= near <= far`, or `None` where it doesn't cut into that wall. For a circle
    /// through the corner, `near` is 0.
    pub bite_a: Option<[f64; 2]>,
    /// The same on wall `b`.
    pub bite_b: Option<[f64; 2]>,
}

/// The relief of the corner whose walls leave it in unit directions `a` and `b`, the void the
/// wedge between them (`a` and `b` need not be exactly unit: they are normalised), walls `la` and
/// `lb` long (only a T-bone reads them, to choose its side). Works in 2D (z = 0) and 3D alike.
///
/// # Errors
/// [`SolidError::InvalidInput`] for options out of range or a direction or length that is not
/// finite.
pub fn wedge_relief(
    a: [f64; 3],
    b: [f64; 3],
    la: f64,
    lb: f64,
    options: &Options,
) -> Result<std::result::Result<Wedge, Skip>> {
    let o = options.checked()?;
    if !(finite3(a) && finite3(b) && la.is_finite() && lb.is_finite()) {
        return Err(invalid("a corner's walls must be finite"));
    }
    let (Some(a), Some(b)) = (unit(a), unit(b)) else {
        return Ok(Err(Skip::Degenerate));
    };
    let angle = angle_between(a, b);
    if angle <= STRAIGHT {
        return Ok(Err(Skip::Degenerate));
    }
    if angle >= std::f64::consts::PI - STRAIGHT {
        return Ok(Err(Skip::Straight));
    }
    if angle > o.max_angle + ANGLE_SLACK {
        return Ok(Err(Skip::TooOpen { angle }));
    }
    let r = o.radius;
    // a + b has length 2 cos(θ/2) > 0 for θ < π.
    let Some(bisector) = unit(add(a, b)) else {
        return Ok(Err(Skip::Degenerate));
    };
    let centre = match o.style {
        Style::Normal => scale(bisector, r),
        Style::Minimal { gap } => scale(bisector, r + gap),
        Style::TBone { side } => {
            let a_first = match side {
                Side::Longer => la >= lb,
                Side::Shorter => la < lb,
            };
            scale(if a_first { a } else { b }, r)
        }
    };
    let (bite_a, bite_b) = (bite(centre, a, r), bite(centre, b, r));
    if bite_a.is_none() && bite_b.is_none() {
        return Ok(Err(Skip::Unreached));
    }
    Ok(Ok(Wedge {
        angle,
        centre,
        bite_a,
        bite_b,
    }))
}

/// Where the circle of radius `r` about `centre` (relative to the corner) crosses the ray from
/// the corner along unit `d`: the stretch of the ray inside the circle, its start clamped to the
/// corner, or `None` when the circle doesn't reach past the corner along it.
fn bite(centre: [f64; 3], d: [f64; 3], r: f64) -> Option<[f64; 2]> {
    // |s d - c|² = r²: s² - 2 s (c·d) + |c|² - r² = 0.
    let h = dot(centre, d);
    let disc = h * h - (dot(centre, centre) - r * r);
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let far = h + root;
    let near = (h - root).max(0.0);
    // A circle through the corner gives near = 0 up to rounding: exactly 0.
    let near = if near <= r * 1e-12 { 0.0 } else { near };
    (far > near + r * 1e-9).then_some([near, far])
}

/// What one vertex of a ring gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner2 {
    /// A relief.
    Relief(Relief2),
    /// No relief, and why.
    Skipped(Skip),
}

/// A relief at a vertex of a planar ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Relief2 {
    /// The corner point (the ring's vertex).
    pub corner: [f64; 2],
    /// The relief circle's centre.
    pub centre: [f64; 2],
    /// Its radius: the tool's.
    pub radius: f64,
    /// The corner's (void wedge's) angle, radians.
    pub angle: f64,
    /// How far along the incoming edge, back from the corner, the bite reaches: `[near, far]`.
    pub bite_in: Option<[f64; 2]>,
    /// How far along the outgoing edge the bite reaches: `[near, far]`.
    pub bite_out: Option<[f64; 2]>,
}

/// The relief (or why none) at each vertex of `ring`, a closed planar loop (the last vertex
/// joins the first; don't repeat the first at the end) with the material on its left: an outline
/// counter-clockwise, a hole clockwise, as a region's loops run. A corner turning right is an
/// inside (concave) corner and gets a relief; one turning left is convex.
///
/// # Errors
/// [`SolidError::InvalidInput`] for options out of range, fewer than 3 or more than
/// [`MAX_RING`] vertices, or a coordinate that is not finite or larger than [`MAX_SIZE`].
pub fn ring_reliefs(ring: &[[f64; 2]], options: &Options) -> Result<Vec<Corner2>> {
    let o = options.checked()?;
    check_ring(ring)?;
    let mut out = Vec::new();
    for (i, at) in ring.iter().enumerate() {
        let prev = if i == 0 { ring.last() } else { ring.get(i - 1) };
        let next = ring.get(i + 1).or_else(|| ring.first());
        let (Some(prev), Some(next)) = (prev, next) else {
            return Err(invalid("a ring's vertices"));
        };
        out.push(corner_2d(*prev, *at, *next, &o)?);
    }
    Ok(out)
}

/// The relief at `at`, reached from `prev` and left towards `next`, the material on the left.
///
/// # Errors
/// As [`ring_reliefs`].
pub fn corner_2d(
    prev: [f64; 2],
    at: [f64; 2],
    next: [f64; 2],
    options: &Options,
) -> Result<Corner2> {
    let o = options.checked()?;
    if ![prev, at, next].iter().all(|p| finite2(*p)) {
        return Err(invalid("ring points must be finite"));
    }
    let a = [prev[0] - at[0], prev[1] - at[1], 0.0];
    let b = [next[0] - at[0], next[1] - at[1], 0.0];
    let (la, lb) = (len(a), len(b));
    if la == 0.0 || lb == 0.0 {
        return Ok(Corner2::Skipped(Skip::Degenerate));
    }
    // The turn from the incoming direction (-a) to the outgoing (b): its z, -a × b, is negative
    // for a right turn, with the material on the left an inside corner.
    let turn = -(a[0] * b[1] - a[1] * b[0]) / (la * lb);
    let along = -(a[0] * b[0] + a[1] * b[1]) / (la * lb);
    if turn.abs() <= STRAIGHT {
        // In line, or folded straight back.
        return Ok(Corner2::Skipped(if along > 0.0 {
            Skip::Straight
        } else {
            Skip::Degenerate
        }));
    }
    if turn > 0.0 {
        return Ok(Corner2::Skipped(Skip::Convex));
    }
    Ok(match wedge_relief(a, b, la, lb, &o)? {
        Ok(w) => Corner2::Relief(Relief2 {
            corner: at,
            centre: [at[0] + w.centre[0], at[1] + w.centre[1]],
            radius: o.radius,
            angle: w.angle,
            bite_in: w.bite_a,
            bite_out: w.bite_b,
        }),
        Err(skip) => Corner2::Skipped(skip),
    })
}

/// `ring` with every relief of [`ring_reliefs`] cut into it: each relieved corner replaced by
/// the circle's arc through the material (points no more than `chord_tolerance` from the arc,
/// at most [`MAX_ARC_POINTS`] per arc), the rest of the ring as it was. The result runs the same
/// way, the material still on its left. A minimal relief whose circle misses the corner leaves
/// the corner point between its two bites.
///
/// # Errors
/// As [`ring_reliefs`]; [`SolidError::InvalidInput`] for a chord tolerance that is not positive
/// and finite, and where a bite runs past the end of its edge or into the next corner's bite
/// (an edge too short for the reliefs at its ends: cut those with a boolean instead).
pub fn relieved_ring(
    ring: &[[f64; 2]],
    options: &Options,
    chord_tolerance: f64,
) -> Result<Vec<[f64; 2]>> {
    if !(chord_tolerance.is_finite() && chord_tolerance > 0.0) {
        return Err(invalid("the chord tolerance must be positive and finite"));
    }
    let corners = ring_reliefs(ring, options)?;
    // Each edge i runs from vertex i to i + 1: what the reliefs at its two ends take of it.
    for (i, start) in ring.iter().enumerate() {
        let end = ring
            .get(i + 1)
            .or_else(|| ring.first())
            .ok_or_else(|| invalid("a ring's vertices"))?;
        let length = dist2(*start, *end);
        let from_start = match corners.get(i) {
            Some(Corner2::Relief(r)) => r.bite_out.map_or(0.0, |[_, far]| far),
            _ => 0.0,
        };
        let next = if i + 1 == ring.len() {
            corners.first()
        } else {
            corners.get(i + 1)
        };
        let from_end = match next {
            Some(Corner2::Relief(r)) => r.bite_in.map_or(0.0, |[_, far]| far),
            _ => 0.0,
        };
        if from_start + from_end > length {
            return Err(invalid(
                "a relief runs past the end of its edge or into the next relief",
            ));
        }
    }
    let mut out = Vec::new();
    for (i, (at, corner)) in ring.iter().zip(&corners).enumerate() {
        match corner {
            Corner2::Relief(r) => {
                let prev = if i == 0 { ring.last() } else { ring.get(i - 1) };
                let next = ring.get(i + 1).or_else(|| ring.first());
                let (Some(prev), Some(next)) = (prev, next) else {
                    return Err(invalid("a ring's vertices"));
                };
                splice(&mut out, *prev, *at, *next, r, chord_tolerance)?;
            }
            Corner2::Skipped(_) => out.push(*at),
        }
    }
    Ok(out)
}

/// The points that stand for corner `at` (reached from `prev`, left towards `next`) with relief
/// `r` in [`relieved_ring`].
fn splice(
    out: &mut Vec<[f64; 2]>,
    prev: [f64; 2],
    at: [f64; 2],
    next: [f64; 2],
    r: &Relief2,
    tol: f64,
) -> Result<()> {
    let on_in = |s: f64| point_towards(at, prev, s);
    let on_out = |s: f64| point_towards(at, next, s);
    if dist2(at, r.centre) <= r.radius * (1.0 + 1e-12) {
        // The circle passes through the corner: one bite, from the far end of the bite in the
        // incoming wall round through the material to the far end of the bite in the outgoing
        // wall (either may be the corner itself, where the circle doesn't cut that wall).
        let from = r.bite_in.map_or(at, |[_, far]| on_in(far));
        let to = r.bite_out.map_or(at, |[_, far]| on_out(far));
        arc(out, r.centre, r.radius, from, to, tol)?;
    } else {
        // Two bites, the corner point between them.
        if let Some([near, far]) = r.bite_in {
            arc(out, r.centre, r.radius, on_in(far), on_in(near), tol)?;
        }
        out.push(at);
        if let Some([near, far]) = r.bite_out {
            arc(out, r.centre, r.radius, on_out(near), on_out(far), tol)?;
        }
    }
    Ok(())
}

/// The point `s` from `at` towards `p`.
fn point_towards(at: [f64; 2], p: [f64; 2], s: f64) -> [f64; 2] {
    let l = dist2(at, p);
    if l == 0.0 {
        return at;
    }
    [
        at[0] + (p[0] - at[0]) / l * s,
        at[1] + (p[1] - at[1]) / l * s,
    ]
}

/// The clockwise arc of the circle about `c` from `from` to `to` (both on it), as points from
/// `from` to `to` inclusive, each chord within `tol` of the arc.
fn arc(
    out: &mut Vec<[f64; 2]>,
    c: [f64; 2],
    r: f64,
    from: [f64; 2],
    to: [f64; 2],
    tol: f64,
) -> Result<()> {
    let tau = std::f64::consts::TAU;
    let start = (from[1] - c[1]).atan2(from[0] - c[0]);
    let end = (to[1] - c[1]).atan2(to[0] - c[0]);
    // Clockwise: the angle falls from `start` to `end`, by a sweep in (0, 2π].
    let mut sweep = start - end;
    while sweep <= 0.0 {
        sweep += tau;
    }
    while sweep > tau {
        sweep -= tau;
    }
    // A chord of angle φ lies r (1 - cos(φ/2)) off its arc.
    let step = if tol >= r {
        std::f64::consts::PI
    } else {
        2.0 * (1.0 - tol / r).acos()
    };
    let pieces = (sweep / step.max(1e-9)).ceil().max(1.0);
    if !(pieces.is_finite() && pieces < MAX_ARC_POINTS as f64) {
        return Err(invalid(
            "the chord tolerance is too small for the relief's arc",
        ));
    }
    let count = pieces as usize;
    out.push(from);
    for k in 1..count {
        let t = start - sweep * (k as f64) / pieces;
        out.push([c[0] + r * t.cos(), c[1] + r * t.sin()]);
    }
    out.push(to);
    Ok(())
}

/// One face of a solid along a straight edge, as [`edge_corner`] reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeFace {
    /// The face's plane's normal pointing out of the material (need not be unit).
    pub outward: [f64; 3],
    /// Whether the face lies on the left of the edge's direction (start to end) seen from the
    /// outward side: the face's boundary runs along the edge from start to end, counted with the
    /// face's own sense. For a face whose loops run counter-clockwise seen from its surface's
    /// natural normal, that is `loop runs start→end` == `the face's shell uses the natural
    /// normal`.
    pub left: bool,
}

/// What a straight edge between two planar faces is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EdgeCorner {
    /// An outside edge: the material's angle there is less than 180°.
    Convex,
    /// The faces continue each other: no corner.
    Smooth,
    /// An inside edge: the walls' directions away from it (unit, square to the edge, along each
    /// face, in the order the faces were given) and the void's angle between them.
    Concave {
        /// The walls' directions.
        walls: [[f64; 3]; 2],
        /// The angle between them, radians in (0, π).
        angle: f64,
    },
}

/// Whether the straight edge from `start` to `end` between `faces` is an inside (concave)
/// corner, and if so, its walls.
///
/// # Errors
/// [`SolidError::InvalidInput`] for a point or normal that is not finite, a zero-length edge or
/// a zero normal.
pub fn edge_corner(start: [f64; 3], end: [f64; 3], faces: [EdgeFace; 2]) -> Result<EdgeCorner> {
    if !(finite3(start) && finite3(end) && faces.iter().all(|f| finite3(f.outward))) {
        return Err(invalid(
            "an edge's points and its faces' normals must be finite",
        ));
    }
    let t = unit(sub(end, start)).ok_or_else(|| invalid("an edge of zero length"))?;
    let [fa, fb] = faces;
    let (na, nb) = (
        unit(fa.outward).ok_or_else(|| invalid("a face normal of zero length"))?,
        unit(fb.outward).ok_or_else(|| invalid("a face normal of zero length"))?,
    );
    // Into each face, square to the edge: n × t on the face's left of t, the other way on its right.
    let wall = |n: [f64; 3], left: bool| {
        let w = cross(n, t);
        if left {
            w
        } else {
            scale(w, -1.0)
        }
    };
    let (Some(wa), Some(wb)) = (unit(wall(na, fa.left)), unit(wall(nb, fb.left))) else {
        return Err(invalid("a face normal along its edge"));
    };
    // Going along face a away from the edge: in front of face b's plane (the void's side) at an
    // inside edge, behind it (in the material) at an outside one.
    let side = dot(wa, nb);
    if side.abs() <= STRAIGHT {
        return Ok(EdgeCorner::Smooth);
    }
    if side < 0.0 {
        return Ok(EdgeCorner::Convex);
    }
    Ok(EdgeCorner::Concave {
        walls: [wa, wb],
        angle: angle_between(wa, wb),
    })
}

/// The cylinder a relief along a straight inside edge is cut with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReliefCylinder {
    /// The centre of its base circle.
    pub base: [f64; 3],
    /// Its unit axis, from the base.
    pub axis: [f64; 3],
    /// Its radius: the tool's.
    pub radius: f64,
    /// Its length along the axis.
    pub height: f64,
    /// The corner's (void wedge's) angle, radians.
    pub angle: f64,
}

/// The relief cylinder of the inside edge from `start` to `end` with walls `walls` (from
/// [`EdgeCorner::Concave`]), `lengths` long (a T-bone's side), running the edge's length plus
/// `extend[0]` past `start` and `extend[1]` past `end` (0 at a blind end, such as a pocket's
/// floor, so the relief cuts nothing below it); or why none.
///
/// # Errors
/// [`SolidError::InvalidInput`] for options or an extension out of range, a point that is not
/// finite or a zero-length edge.
pub fn edge_relief(
    start: [f64; 3],
    end: [f64; 3],
    walls: [[f64; 3]; 2],
    lengths: [f64; 2],
    extend: [f64; 2],
    options: &Options,
) -> Result<std::result::Result<ReliefCylinder, Skip>> {
    let o = options.checked()?;
    let extensions = extend
        .iter()
        .all(|e| e.is_finite() && (0.0..=MAX_SIZE).contains(e));
    if !(finite3(start) && finite3(end) && extensions) {
        return Err(invalid(
            "an edge's points and extensions must be finite (extensions at least 0)",
        ));
    }
    let along = sub(end, start);
    let l = len(along);
    let axis = unit(along).ok_or_else(|| invalid("an edge of zero length"))?;
    let [wa, wb] = walls;
    let [la, lb] = lengths;
    let w = match wedge_relief(wa, wb, la, lb, &o)? {
        Ok(w) => w,
        Err(skip) => return Ok(Err(skip)),
    };
    let base = add(add(start, w.centre), scale(axis, -extend[0]));
    Ok(Ok(ReliefCylinder {
        base,
        axis,
        radius: o.radius,
        height: l + extend[0] + extend[1],
        angle: w.angle,
    }))
}

/// Whether the end `here` of a straight edge (its other end `there`) is open: the face there,
/// with outward normal `outward`, faces on the way the edge runs past `here`, so a relief may run
/// on through empty space; a pocket's floor faces back along the edge (blind).
///
/// # Errors
/// [`SolidError::InvalidInput`] for a point or vector that is not finite, a zero-length edge or
/// a zero normal.
pub fn end_is_open(there: [f64; 3], here: [f64; 3], outward: [f64; 3]) -> Result<bool> {
    if !(finite3(there) && finite3(here) && finite3(outward)) {
        return Err(invalid("points and normals must be finite"));
    }
    let (Some(d), Some(n)) = (unit(sub(here, there)), unit(outward)) else {
        return Err(invalid("a vector of zero length"));
    };
    Ok(dot(d, n) > STRAIGHT)
}

/// Whether the segment from `start` to `end` runs along `axis` (either way), within `tolerance`
/// radians.
///
/// # Errors
/// [`SolidError::InvalidInput`] for a vector that is not finite or of zero length, or a
/// tolerance out of [0, π/2].
pub fn is_along(start: [f64; 3], end: [f64; 3], axis: [f64; 3], tolerance: f64) -> Result<bool> {
    let tolerated =
        tolerance.is_finite() && (0.0..=std::f64::consts::FRAC_PI_2).contains(&tolerance);
    if !(finite3(start) && finite3(end) && finite3(axis) && tolerated) {
        return Err(invalid(
            "is_along: finite points, axis and a tolerance in [0, π/2]",
        ));
    }
    let (Some(d), Some(a)) = (unit(sub(end, start)), unit(axis)) else {
        return Err(invalid("a vector of zero length"));
    };
    let off = angle_between(d, a);
    Ok(off <= tolerance || std::f64::consts::PI - off <= tolerance)
}

/// The unit normal of the plane `origin + u u_axis + v v_axis`: `u_axis × v_axis`, normalised.
///
/// # Errors
/// [`SolidError::InvalidInput`] for an axis that is not finite, or axes in line.
pub fn plane_normal(u_axis: [f64; 3], v_axis: [f64; 3]) -> Result<[f64; 3]> {
    if !(finite3(u_axis) && finite3(v_axis)) {
        return Err(invalid("plane axes must be finite"));
    }
    unit(cross(u_axis, v_axis)).ok_or_else(|| invalid("plane axes in line"))
}

/// The unit normal of the plane through `origin`, `u_point` and `v_point`:
/// `(u_point - origin) × (v_point - origin)`, normalised.
///
/// # Errors
/// As [`plane_normal`].
pub fn plane_normal_through(
    origin: [f64; 3],
    u_point: [f64; 3],
    v_point: [f64; 3],
) -> Result<[f64; 3]> {
    if !(finite3(origin) && finite3(u_point) && finite3(v_point)) {
        return Err(invalid("plane points must be finite"));
    }
    plane_normal(sub(u_point, origin), sub(v_point, origin))
}

/// The distance between two points.
#[must_use]
pub fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    len(sub(a, b))
}

/// The point halfway between two points.
#[must_use]
pub fn midpoint(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    scale(add(a, b), 0.5)
}

/// `v` reversed when `flip`.
#[must_use]
pub fn flipped(v: [f64; 3], flip: bool) -> [f64; 3] {
    if flip {
        scale(v, -1.0)
    } else {
        v
    }
}

/// The area a relief of `options` removes beyond the walls of a corner of angle `angle`
/// (radians), in closed form: what a full-depth relief cuts away is this times its depth. `None`
/// for an angle out of (0, π) and for a T-bone sharper than 90° (its bite then crosses both
/// walls and has no short closed form here).
///
/// - Normal: two circular segments, each cut off by a wall at `d = r sin(θ/2)` from the centre,
///   `r² acos(d/r) − d √(r² − d²)` each, which sum to `r² (π − θ − sin θ)`; they meet only at
///   the corner.
/// - Minimal: the same two segments with `d = (r + gap) sin(θ/2)` (none where `d >= r`); the
///   circle misses the corner, so they don't meet.
/// - T-bone at θ ≥ 90°: the centre lies on the bitten wall's line, so half the disc lies beyond
///   it, `π r² / 2`; the other half stays inside the corner.
#[must_use]
pub fn removed_area(angle: f64, options: &Options) -> Option<f64> {
    let r = options.radius;
    let pi = std::f64::consts::PI;
    if !(angle.is_finite() && angle > 0.0 && angle < pi) {
        return None;
    }
    let segment = |d: f64| {
        if d >= r {
            0.0
        } else {
            r * r * (d / r).acos() - d * (r * r - d * d).sqrt()
        }
    };
    match options.style {
        Style::Normal => Some(r * r * (pi - angle - angle.sin())),
        Style::Minimal { gap } => Some(2.0 * segment((r + gap) * (angle / 2.0).sin())),
        Style::TBone { .. } if angle >= pi / 2.0 - ANGLE_SLACK => Some(pi * r * r / 2.0),
        Style::TBone { .. } => None,
    }
}

fn invalid(reason: &'static str) -> SolidError {
    SolidError::InvalidInput {
        operation: OP,
        reason,
    }
}

fn check_ring(ring: &[[f64; 2]]) -> Result<()> {
    if ring.len() < 3 || ring.len() > MAX_RING {
        return Err(invalid("a ring has 3 to 1 000 000 vertices"));
    }
    if !ring
        .iter()
        .all(|p| finite2(*p) && p[0].abs() <= MAX_SIZE && p[1].abs() <= MAX_SIZE)
    {
        return Err(invalid("ring points must be finite and within 10⁶ mm"));
    }
    Ok(())
}

fn finite2(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

fn finite3(p: [f64; 3]) -> bool {
    p.iter().all(|x| x.is_finite())
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn len(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

fn dist2(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = len(a);
    (l.is_finite() && l > 0.0).then(|| scale(a, 1.0 / l))
}

/// The angle between unit vectors, accurate near 0 and π (atan2 of |a × b| and a · b).
fn angle_between(a: [f64; 3], b: [f64; 3]) -> f64 {
    len(cross(a, b)).atan2(dot(a, b))
}

#[cfg(test)]
#[path = "dogbone_tests.rs"]
mod tests;
