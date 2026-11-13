//! A router's corner reliefs for the rings of a flat part.
//!
//! A round bit leaves a round in every inside corner of a part's outline and holes. A ring's
//! vertex is a corner as [`crate::dogbone`] describes one: its walls are the ring's two edges
//! there, the material is on the ring's left and the void (where the bit runs) on its right, so a
//! corner turning right is an inside corner. [`ring_reliefs`] places each inside corner's relief
//! circle with the dogbone module's wedge computation, the one the reliefs along a solid's edges
//! use; [`super::finish::relieve_corners`] cuts the circles out of the part's material.
//!
//! Every function checks its input and returns [`SolidError::InvalidInput`] for a number that is
//! not finite or out of range; nothing here panics on input.
//!
//! Deviations: none (no upstream counterpart). No recursion.

use crate::dogbone::{self, Options, Skip};
use crate::math::Vec3;
use crate::{Result, SolidError};

use super::P2;

const OP: &str = "flat::relief";

/// Most vertices in a ring.
const MAX_RING: usize = 1_000_000;

/// Largest coordinate magnitude of a ring's point accepted (mm): far past any sheet.
const MAX_COORD: f64 = 1.0e6;

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
        }),
        Err(skip) => Corner::Skipped(skip),
    })
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

    /// The relief circles' centres of `corners`, each of radius `R`; every corner has one.
    fn centres(corners: &[Corner]) -> Vec<P2> {
        corners
            .iter()
            .map(|c| {
                let Corner::Relief(r) = c else {
                    panic!("{c:?}")
                };
                assert_eq!(r.radius, R);
                r.centre
            })
            .collect()
    }

    fn assert_near(got: &[P2], want: &[P2]) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (c, w) in got.iter().zip(want) {
            assert!(
                close(c[0], w[0], 1e-12) && close(c[1], w[1], 1e-12),
                "{c:?} {w:?}"
            );
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
        // Each centre on its corner's bisector, R from the corner.
        let d = R * FRAC_1_SQRT_2;
        assert_near(
            &centres(&ring_reliefs(&hole, &o).unwrap()),
            &[[d, d], [d, 12.0 - d], [20.0 - d, 12.0 - d], [20.0 - d, d]],
        );
        // The same rectangle as an outline (counter-clockwise): four convex corners.
        let outline: Vec<P2> = hole.iter().rev().copied().collect();
        assert!(ring_reliefs(&outline, &o)
            .unwrap()
            .iter()
            .all(|c| *c == Corner::Skipped(Skip::Convex)));
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
        let corners = ring_reliefs(&l, &opts(Style::Normal)).unwrap();
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
        // The void at (10, 10) is the quadrant x > 10, y > 10.
        let d = R * FRAC_1_SQRT_2;
        assert_near(&centres(&corners[3..4]), &[[10.0 + d, 10.0 + d]]);
    }

    #[test]
    fn a_slot_narrower_than_its_reliefs_still_gets_all_four() {
        // A 2 mm slot: the reliefs at both ends of each short edge overlap, and every corner
        // still gets its own.
        let slot = [[0.0, 0.0], [0.0, 2.0], [20.0, 2.0], [20.0, 0.0]];
        let d = R * FRAC_1_SQRT_2;
        assert_near(
            &centres(&ring_reliefs(&slot, &opts(Style::Normal)).unwrap()),
            &[[d, d], [d, 2.0 - d], [20.0 - d, 2.0 - d], [20.0 - d, d]],
        );
    }

    #[test]
    fn a_t_bone_sits_on_the_wall_it_bites() {
        // A T-bone on the longer (20 mm) walls bites them, not the 12 mm ones: each centre on the
        // line of its corner's 20 mm wall, R from the corner.
        let hole = [[0.0, 0.0], [0.0, 12.0], [20.0, 12.0], [20.0, 0.0]];
        let o = opts(Style::TBone { side: Side::Longer });
        assert_near(
            &centres(&ring_reliefs(&hole, &o).unwrap()),
            &[[R, 0.0], [R, 12.0], [20.0 - R, 12.0], [20.0 - R, 0.0]],
        );
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
    }
}
