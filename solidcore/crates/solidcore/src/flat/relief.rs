//! A router's corner reliefs cut into the rings of a flat part.
//!
//! A round bit leaves a round in every inside corner of a part's outline and holes. A ring's
//! vertex is a corner as [`crate::dogbone`] describes one: its walls are the ring's two edges
//! there, the material is on the ring's left and the void (where the bit runs) on its right, so a
//! corner turning right is an inside corner. [`ring_reliefs`] places each inside corner's relief
//! circle with the dogbone module's wedge computation, the one the reliefs along a solid's edges
//! use, and [`relieved_ring`] splices the circles' arcs into the ring.
//!
//! Every function checks its input and returns [`SolidError::InvalidInput`] for a number that is
//! not finite or out of range; nothing here panics on input.
//!
//! Deviations: none (no upstream counterpart). No recursion.

use crate::dogbone::{self, Options, Skip};
use crate::math::{Vec2, Vec3};
use crate::{Result, SolidError};

use super::{Ring, P2};

const OP: &str = "flat::relief";

/// Most vertices in a ring.
const MAX_RING: usize = 1_000_000;

/// Largest coordinate magnitude of a ring's point accepted (mm): far past any sheet.
const MAX_COORD: f64 = 1.0e6;

/// Most points [`relieved_ring`] writes for one relief's arc.
const MAX_ARC_POINTS: usize = 4096;

/// What one vertex of a ring gets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Corner {
    /// A relief.
    Relief(Relief),
    /// No relief, and why.
    Skipped(Skip),
}

/// A relief at a vertex of a ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Relief {
    /// The relief circle's centre.
    pub(crate) centre: P2,
    /// Its radius: the bit's.
    pub(crate) radius: f64,
    /// How far along the incoming edge, back from the corner, the bite reaches: `[near, far]`,
    /// or `None` where the circle doesn't cut that edge.
    bite_in: Option<[f64; 2]>,
    /// How far along the outgoing edge the bite reaches: `[near, far]`, or `None`.
    bite_out: Option<[f64; 2]>,
}

/// The relief (or why none) at each vertex of `ring`, a closed loop (the last vertex joins the
/// first, which is not repeated) with the material on its left: an outline counter-clockwise, a
/// hole clockwise, as a region's rings run. A corner turning right is an inside (concave) corner
/// and gets a relief; one turning left is convex.
///
/// # Errors
/// [`SolidError::InvalidInput`] for options out of range, fewer than 3 or more than
/// [`MAX_RING`] vertices, or a coordinate that is not finite or larger than [`MAX_COORD`].
pub(crate) fn ring_reliefs(ring: &[P2], options: &Options) -> Result<Vec<Corner>> {
    let o = options.checked()?;
    check_ring(ring)?;
    let mut out = Vec::new();
    for (i, at) in ring.iter().enumerate() {
        let prev = if i == 0 { ring.last() } else { ring.get(i - 1) };
        let next = ring.get(i + 1).or_else(|| ring.first());
        let (Some(prev), Some(next)) = (prev, next) else {
            return Err(invalid("a ring's vertices"));
        };
        out.push(corner(*prev, *at, *next, &o)?);
    }
    Ok(out)
}

/// The relief at `at`, reached from `prev` and left towards `next`, the material on the left.
///
/// # Errors
/// As [`ring_reliefs`].
fn corner(prev: P2, at: P2, next: P2, options: &Options) -> Result<Corner> {
    let o = options.checked()?;
    if ![prev, at, next].iter().flatten().all(|c| c.is_finite()) {
        return Err(invalid("ring points must be finite"));
    }
    let a = Vec3::new(prev[0] - at[0], prev[1] - at[1], 0.0);
    let b = Vec3::new(next[0] - at[0], next[1] - at[1], 0.0);
    let (la, lb) = (a.length(), b.length());
    if la == 0.0 || lb == 0.0 {
        return Ok(Corner::Skipped(Skip::Degenerate));
    }
    // The turn from the incoming direction (-a) to the outgoing (b), on unit directions: its
    // sine, the z of -a × b, is negative for a right turn, with the material on the left an
    // inside corner; `along` is its cosine.
    let turn = -a.cross(b).z / (la * lb);
    let along = -a.dot(b) / (la * lb);
    if turn.abs() <= dogbone::STRAIGHT {
        // In line, or folded straight back.
        return Ok(Corner::Skipped(if along > 0.0 {
            Skip::Straight
        } else {
            Skip::Degenerate
        }));
    }
    if turn > 0.0 {
        return Ok(Corner::Skipped(Skip::Convex));
    }
    Ok(match dogbone::wedge_relief(a, b, la, lb, &o)? {
        Ok(w) => Corner::Relief(Relief {
            centre: [at[0] + w.centre.x, at[1] + w.centre.y],
            radius: o.radius,
            bite_in: w.bite_a,
            bite_out: w.bite_b,
        }),
        Err(skip) => Corner::Skipped(skip),
    })
}

/// `ring` with every relief of [`ring_reliefs`] cut into it: each relieved corner replaced by
/// the circle's arc through the material (points no more than `chord_tolerance` from the arc,
/// at most [`MAX_ARC_POINTS`] per arc), the rest of the ring as it was. The result runs the
/// same way, the material still on its left. A minimal relief whose circle misses the corner
/// leaves the corner point between its two bites.
///
/// # Errors
/// As [`ring_reliefs`]; [`SolidError::InvalidInput`] for a chord tolerance that is not positive
/// and finite, and where a bite runs past the end of its edge or into the next corner's bite
/// (an edge too short for the reliefs at its ends: cut those with a boolean instead).
pub(crate) fn relieved_ring(ring: &[P2], options: &Options, chord_tolerance: f64) -> Result<Ring> {
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
        let length = Vec2::from(*start).distance(Vec2::from(*end));
        let from_start = match corners.get(i) {
            Some(Corner::Relief(r)) => r.bite_out.map_or(0.0, |[_, far]| far),
            _ => 0.0,
        };
        let next = if i + 1 == ring.len() {
            corners.first()
        } else {
            corners.get(i + 1)
        };
        let from_end = match next {
            Some(Corner::Relief(r)) => r.bite_in.map_or(0.0, |[_, far]| far),
            _ => 0.0,
        };
        if from_start + from_end > length {
            return Err(invalid(
                "a relief runs past the end of its edge or into the next relief",
            ));
        }
    }
    let mut out = Vec::new();
    for (i, (at, c)) in ring.iter().zip(&corners).enumerate() {
        match c {
            Corner::Relief(r) => {
                let prev = if i == 0 { ring.last() } else { ring.get(i - 1) };
                let next = ring.get(i + 1).or_else(|| ring.first());
                let (Some(prev), Some(next)) = (prev, next) else {
                    return Err(invalid("a ring's vertices"));
                };
                splice(&mut out, *prev, *at, *next, r, chord_tolerance)?;
            }
            Corner::Skipped(_) => out.push(*at),
        }
    }
    Ok(out)
}

/// The points that stand for corner `at` (reached from `prev`, left towards `next`) with relief
/// `r` in [`relieved_ring`].
fn splice(out: &mut Ring, prev: P2, at: P2, next: P2, r: &Relief, tol: f64) -> Result<()> {
    let on_in = |s: f64| point_towards(at, prev, s);
    let on_out = |s: f64| point_towards(at, next, s);
    // A circle through the corner has its centre the radius away from it, up to the rounding
    // the bites already read as the corner itself.
    let through =
        Vec2::from(at).distance(Vec2::from(r.centre)) <= r.radius * (1.0 + dogbone::AT_CORNER);
    if through {
        // One bite, from the far end of the bite in the incoming wall round through the material
        // to the far end of the bite in the outgoing wall (either may be the corner itself, where
        // the circle doesn't cut that wall).
        let from = match r.bite_in {
            Some([_, far]) => on_in(far)?,
            None => at,
        };
        let to = match r.bite_out {
            Some([_, far]) => on_out(far)?,
            None => at,
        };
        arc(out, r.centre, r.radius, from, to, tol)?;
    } else {
        // Two bites, the corner point between them.
        if let Some([near, far]) = r.bite_in {
            arc(out, r.centre, r.radius, on_in(far)?, on_in(near)?, tol)?;
        }
        out.push(at);
        if let Some([near, far]) = r.bite_out {
            arc(out, r.centre, r.radius, on_out(near)?, on_out(far)?, tol)?;
        }
    }
    Ok(())
}

/// The point `s` from `at` towards `p`.
///
/// # Errors
/// [`SolidError::InvalidInput`] where `p` is `at`: no direction (a relieved corner's edges have
/// one, or the wedge computation would have found the corner degenerate).
fn point_towards(at: P2, p: P2, s: f64) -> Result<P2> {
    let at = Vec2::from(at);
    let d = (Vec2::from(p) - at).unitize()?;
    Ok((at + d * s).to_array())
}

/// The clockwise arc of the circle about `c` from `from` to `to` (both on it), as points from
/// `from` to `to` inclusive, each chord within `tol` of the arc.
///
/// # Errors
/// [`SolidError::InvalidInput`] where that takes more than [`MAX_ARC_POINTS`] points.
fn arc(out: &mut Ring, c: P2, r: f64, from: P2, to: P2, tol: f64) -> Result<()> {
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
    // A chord of angle φ lies r (1 - cos(φ/2)) off its arc. With the tolerance positive (as
    // `relieved_ring` checks) the step is in (0, π]; where tol / r is lost in rounding it is 0,
    // and the count of pieces is infinite and refused below.
    let step = if tol >= r {
        std::f64::consts::PI
    } else {
        2.0 * (1.0 - tol / r).acos()
    };
    let pieces = (sweep / step).ceil().max(1.0);
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

fn check_ring(ring: &[P2]) -> Result<()> {
    if ring.len() < 3 || ring.len() > MAX_RING {
        return Err(invalid("a ring has 3 to 1 000 000 vertices"));
    }
    if !ring
        .iter()
        .flatten()
        .all(|c| c.is_finite() && c.abs() <= MAX_COORD)
    {
        return Err(invalid("ring points must be finite and within 10⁶ mm"));
    }
    Ok(())
}

fn invalid(reason: &'static str) -> SolidError {
    SolidError::InvalidInput {
        operation: OP,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, PI};

    use super::*;
    use crate::dogbone::{Side, Style};

    const R: f64 = 1.5;

    fn opts(style: Style) -> Options {
        Options {
            radius: R,
            style,
            max_angle: PI,
        }
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    /// The shoelace area of a ring (positive counter-clockwise).
    fn area(ring: &[P2]) -> f64 {
        let n = ring.len();
        (0..n)
            .map(|i| {
                let (p, q) = (ring[i], ring[(i + 1) % n]);
                p[0] * q[1] - q[0] * p[1]
            })
            .sum::<f64>()
            / 2.0
    }

    /// What a relief of radius `R` takes beyond the walls of a square corner, in closed form: a
    /// normal one two circular segments cut off at `r sin 45°` from its centre, `(π/2 − 1) r²`;
    /// a minimal one the two smaller segments cut off at `d = (r + gap) sin 45°`,
    /// `r² acos(d/r) − d √(r² − d²)` each; a T-bone, its centre on the bitten wall's line, half
    /// its disc.
    fn square_corner_area(style: Style) -> f64 {
        match style {
            Style::Normal => (FRAC_PI_2 - 1.0) * R * R,
            Style::Minimal { gap } => {
                let d = (R + gap) * FRAC_1_SQRT_2;
                2.0 * (R * R * (d / R).acos() - d * (R * R - d * d).sqrt())
            }
            Style::TBone { .. } => PI * R * R / 2.0,
        }
    }

    #[test]
    fn a_rectangular_hole_gets_four_reliefs_and_an_outline_none() {
        // A 20 x 12 hole runs clockwise: every corner turns right, an inside corner.
        let hole = [[0.0, 0.0], [0.0, 12.0], [20.0, 12.0], [20.0, 0.0]];
        let o = Options {
            radius: R,
            style: Style::Normal,
            max_angle: FRAC_PI_2,
        };
        let corners = ring_reliefs(&hole, &o).unwrap();
        assert_eq!(corners.len(), 4);
        let mut centres = Vec::new();
        for c in &corners {
            let Corner::Relief(r) = c else {
                panic!("{c:?}")
            };
            assert_eq!(r.radius, R);
            centres.push(r.centre);
        }
        // Each centre on its corner's bisector, R from the corner.
        let d = R * FRAC_1_SQRT_2;
        let want = [[d, d], [d, 12.0 - d], [20.0 - d, 12.0 - d], [20.0 - d, d]];
        for (c, w) in centres.iter().zip(&want) {
            assert!(
                close(c[0], w[0], 1e-12) && close(c[1], w[1], 1e-12),
                "{c:?} {w:?}"
            );
        }
        // The same rectangle as an outline (counter-clockwise): four convex corners.
        let outline: Vec<P2> = hole.iter().rev().copied().collect();
        assert!(ring_reliefs(&outline, &o)
            .unwrap()
            .iter()
            .all(|c| *c == Corner::Skipped(Skip::Convex)));
        // The relieved hole is larger by four reliefs' areas (to the chords' error).
        let tol = 1e-4;
        let relieved = relieved_ring(&hole, &o, tol).unwrap();
        let grown = -area(&relieved) - (-area(&hole));
        let want = 4.0 * square_corner_area(Style::Normal);
        assert!(
            close(grown, want, 4.0 * 2.0 * PI * R * tol),
            "{grown} vs {want}"
        );
        // It still runs clockwise and passes through no corner point.
        assert!(area(&relieved) < 0.0);
        assert!(!relieved.iter().any(|p| hole.contains(p)));
        // Every arc point is on its circle.
        for p in &relieved {
            let on_some = centres.iter().any(|c| {
                close(
                    ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt(),
                    R,
                    1e-9,
                )
            });
            let on_edge = p[0].abs() < 1e-12
                || p[1].abs() < 1e-12
                || close(p[0], 20.0, 1e-12)
                || close(p[1], 12.0, 1e-12);
            assert!(on_some || on_edge, "{p:?}");
        }
    }

    #[test]
    fn an_l_shaped_outline_has_one_inside_corner() {
        // Counter-clockwise L: five convex corners and the inside one at (10, 10).
        let l = [
            [0.0, 0.0],
            [30.0, 0.0],
            [30.0, 10.0],
            [10.0, 10.0],
            [10.0, 30.0],
            [0.0, 30.0],
        ];
        let o = opts(Style::Normal);
        let corners = ring_reliefs(&l, &o).unwrap();
        let inside: Vec<usize> = corners
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Corner::Relief(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(inside, vec![3]);
        assert_eq!(
            corners
                .iter()
                .filter(|c| **c == Corner::Skipped(Skip::Convex))
                .count(),
            5
        );
        let Corner::Relief(r) = corners[3] else {
            unreachable!()
        };
        // The void at (10, 10) is the quadrant x > 10, y > 10.
        let d = R * FRAC_1_SQRT_2;
        assert!(close(r.centre[0], 10.0 + d, 1e-12) && close(r.centre[1], 10.0 + d, 1e-12));
        // The material shrinks by one relief.
        let tol = 1e-5;
        let relieved = relieved_ring(&l, &o, tol).unwrap();
        let lost = area(&l) - area(&relieved);
        assert!(
            close(lost, square_corner_area(Style::Normal), 2.0 * PI * R * tol),
            "{lost}"
        );
    }

    #[test]
    fn relieved_rings_splice_every_style() {
        let hole = [[0.0, 0.0], [0.0, 12.0], [20.0, 12.0], [20.0, 0.0]];
        for style in [
            Style::Minimal { gap: 0.2 },
            Style::TBone { side: Side::Longer },
            Style::TBone {
                side: Side::Shorter,
            },
        ] {
            let o = opts(style);
            let tol = 1e-5;
            let relieved = relieved_ring(&hole, &o, tol).unwrap();
            let grown = -area(&relieved) + area(&hole);
            let want = 4.0 * square_corner_area(style);
            assert!(
                close(grown, want, 4.0 * 2.0 * PI * R * tol),
                "{style:?}: {grown} vs {want}"
            );
            // The minimal style keeps the corner points between its bites; a square T-bone's arc
            // ends at the corner, tangent to the wall it doesn't bite.
            assert!(hole.iter().all(|p| relieved.contains(p)), "{style:?}");
        }
        // A T-bone on the longer (20 mm) walls bites them, not the 12 mm ones: each centre on the
        // line of its corner's 20 mm wall.
        let o = opts(Style::TBone { side: Side::Longer });
        for (c, vertex) in ring_reliefs(&hole, &o).unwrap().iter().zip(&hole) {
            let Corner::Relief(r) = c else { panic!() };
            assert!(close(r.centre[1], vertex[1], 1e-12), "{r:?}");
        }
    }

    #[test]
    fn reliefs_too_big_for_their_edge_are_refused_by_the_splice() {
        // A 2 mm slot: bites of √2 r ≈ 2.1 mm from each end overlap.
        let slot = [[0.0, 0.0], [0.0, 2.0], [20.0, 2.0], [20.0, 0.0]];
        assert!(relieved_ring(&slot, &opts(Style::Normal), 1e-3).is_err());
        // The reliefs themselves are still reported.
        assert_eq!(
            ring_reliefs(&slot, &opts(Style::Normal))
                .unwrap()
                .iter()
                .filter(|c| matches!(c, Corner::Relief(_)))
                .count(),
            4
        );
    }

    #[test]
    fn the_splice_adds_the_bites_from_both_ends_of_an_edge() {
        // Each bite takes √2 r ≈ 2.12 of an edge: two fit on a 5 mm edge, not on a 4 mm one,
        // though either alone would. The short edge is the ring's last (it closes the ring) or a
        // middle one.
        let o = opts(Style::Normal);
        for width in [4.0, 5.0] {
            let closing = [[0.0, width], [20.0, width], [20.0, 0.0], [0.0, 0.0]];
            let middle = [[0.0, 0.0], [0.0, width], [20.0, width], [20.0, 0.0]];
            for hole in [closing, middle] {
                assert_eq!(
                    relieved_ring(&hole, &o, 1e-3).is_ok(),
                    width == 5.0,
                    "{hole:?}"
                );
            }
        }
    }

    #[test]
    fn degenerate_ring_corners_are_skipped() {
        let o = opts(Style::Normal);
        // A repeated point, and a point in line with its neighbours.
        assert_eq!(
            corner([0.0, 0.0], [0.0, 0.0], [1.0, 0.0], &o).unwrap(),
            Corner::Skipped(Skip::Degenerate)
        );
        assert_eq!(
            corner([0.0, 0.0], [1.0, 0.0], [2.0, 0.0], &o).unwrap(),
            Corner::Skipped(Skip::Straight)
        );
        // Folded back on itself.
        assert_eq!(
            corner([0.0, 0.0], [1.0, 0.0], [0.0, 0.0], &o).unwrap(),
            Corner::Skipped(Skip::Degenerate)
        );
        assert_eq!(
            corner([0.0, 0.0], [1.0, 0.0], [0.5, 0.0], &o).unwrap(),
            Corner::Skipped(Skip::Degenerate)
        );
    }

    #[test]
    fn turns_are_measured_on_unit_directions() {
        let o = opts(Style::Normal);
        // Along a diagonal: straight on, and folded back.
        assert_eq!(
            corner([0.0, 0.0], [1.0, 1.0], [2.0, 2.0], &o).unwrap(),
            Corner::Skipped(Skip::Straight)
        );
        assert_eq!(
            corner([0.0, 0.0], [1.0, 1.0], [0.5, 0.5], &o).unwrap(),
            Corner::Skipped(Skip::Degenerate)
        );
        assert_eq!(
            corner([0.0, 0.0], [1.0, 1.0], [1.0, 2.0], &o).unwrap(),
            Corner::Skipped(Skip::Convex)
        );
        assert!(matches!(
            corner([0.0, 0.0], [1.0, 1.0], [2.0, 1.0], &o).unwrap(),
            Corner::Relief(_)
        ));
        // Long edges turning by less than the straightness threshold (radians, not the cross
        // product of the edges, which is a million times larger here) are in line.
        let tiny = 2e-10f64;
        let long = [1000.0 * tiny.cos(), 1000.0 * tiny.sin()];
        assert_eq!(
            corner([-1000.0, 0.0], [0.0, 0.0], long, &o).unwrap(),
            Corner::Skipped(Skip::Straight)
        );
        // Short edges turning by more than it are corners.
        let small = [1e-3 * (1e-6f64).cos(), 1e-3 * (1e-6f64).sin()];
        assert_eq!(
            corner([-1e-3, 0.0], [0.0, 0.0], small, &o).unwrap(),
            Corner::Skipped(Skip::Convex)
        );
        // An inside corner of uneven edges is relieved whatever their lengths: a square corner,
        // its centre on the bisector.
        let Corner::Relief(r) = corner([1000.0, 0.0], [0.0, 0.0], [0.0, 0.01], &o).unwrap() else {
            panic!()
        };
        let d = R * FRAC_1_SQRT_2;
        assert!(
            close(r.centre[0], d, 1e-12) && close(r.centre[1], d, 1e-12),
            "{r:?}"
        );
    }

    #[test]
    fn hostile_options_and_rings_are_refused() {
        let triangle = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        for radius in [0.0, -1.0, f64::NAN, f64::INFINITY, 2e6] {
            let o = Options {
                radius,
                style: Style::Normal,
                max_angle: PI,
            };
            assert!(ring_reliefs(&triangle, &o).is_err(), "{radius}");
        }
        let o = opts(Style::Normal);
        assert!(ring_reliefs(&[[0.0, 0.0], [1.0, 0.0]], &o).is_err());
        assert!(ring_reliefs(&[[0.0, 0.0], [1.0, f64::NAN], [0.0, 1.0]], &o).is_err());
        assert!(ring_reliefs(&[[0.0, 0.0], [1.0, 2e6], [0.0, 1.0]], &o).is_err());
        let big = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
        assert!(relieved_ring(&big, &o, 0.0).is_err());
        assert!(relieved_ring(&big, &o, f64::NAN).is_err());
    }
}
