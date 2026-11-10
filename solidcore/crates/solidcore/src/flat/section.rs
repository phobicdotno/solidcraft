//! The section of a B-rep by a plane square to a model axis, as a polygonal [`Region`].
//!
//! The kernel's exact planar section curves
//! ([`create_planar_section_curves`]) are sampled within the caller's chord tolerance with the
//! curves' own evaluation, projected to the slice's `(u, v)` of [`Axis::directions`], chained
//! end to end into closed loops and filled even-odd, so a loop inside another is a hole.
//!
//! A plane through a vertex of the body, or lying in one of its planar faces, has no
//! well-defined section (a face in the plane is material on one side only; a sphere's pole
//! gives a curve of zero length). Like the mesh section it replaces, [`BrepSection`] then moves
//! the plane by [`NUDGES`] (at most 0.023 mm, far less than a laser kerf) to the first offset
//! that keeps [`VERTEX_CLEARANCE`] from every vertex. A planar face bounded only by edges
//! without vertices is not detected; such faces are rare in practice.

use crate::curve::curve::Curve;
use crate::geom::vector3d::{Point3, Vec3};
use crate::kernel::Kernel;
use crate::query::api_queries::brep_bounding_box;
use crate::topology::brep::p1 as brep1;
use crate::topology::brep::planar_section::{
    create_planar_section_curves, PlanarSectionOptions, SectionOutputs,
};
use crate::topology::{vertex, Brep};

use super::region;
use super::slice::{Axis, Sectioner};
use super::{FlatError, FlatResult, Region, Ring, P2};

/// The most section curves one plane may give.
pub const MAX_SECTION_CURVES: usize = 20_000;

/// The most sample points one section may have, all curves together.
pub const MAX_SECTION_SAMPLES: usize = 2_000_000;

/// The plane offsets (mm) tried in order when the plane comes too close to a vertex.
pub const NUDGES: [f64; 5] = [0.0, 0.01, -0.01, 0.023, -0.023];

/// How far (mm) a section plane keeps from every vertex of the body when it can.
pub const VERTEX_CLEARANCE: f64 = 0.002;

/// The smallest and largest chord tolerances (mm) a section accepts.
const CHORD_RANGE: (f64, f64) = (1e-4, 10.0);

/// The deepest a curve span is halved while sampling (2^24 pieces per starting span).
const MAX_DEPTH: u32 = 24;

/// The spans a curve is first cut into before halving: enough that a closed curve (a circle
/// from 0 to 2π) has interior samples to test against.
const FIRST_SPANS: usize = 8;

fn chord_ok(chord: f64) -> bool {
    chord.is_finite() && (CHORD_RANGE.0..=CHORD_RANGE.1).contains(&chord)
}

/// A B-rep sectioned by planes square to the model axes: the [`Sectioner`] of a real body.
pub struct BrepSection<'a> {
    k: &'a Kernel,
    b: &'a mut Brep,
    chord: f64,
    bounds: [f64; 6],
    /// The middle of the bounds, where each section plane's point is taken.
    centre: [f64; 3],
    vertices: Vec<[f64; 3]>,
}

impl<'a> BrepSection<'a> {
    /// The sectioner of body `b`, its outlines within `chord` mm of the exact section.
    ///
    /// The bounds are the kernel's tight bounding box (its loose box, which also encloses the
    /// body, when the tight one fails); a body without geometry has bounds that are not finite,
    /// which [`super::slice::make`] reads as nothing to cut.
    ///
    /// # Errors
    /// [`FlatError::Setting`] for a chord not finite or outside `1e-4..=10` mm; the kernel's
    /// errors listing the vertices or computing the box.
    pub fn new(k: &'a Kernel, b: &'a mut Brep, chord: f64) -> FlatResult<Self> {
        if !chord_ok(chord) {
            return Err(FlatError::Setting("chord"));
        }
        let ext = match brep_bounding_box(k, b, true) {
            Ok(e) => e,
            Err(_) => brep_bounding_box(k, b, false)?,
        };
        let bounds = if ext.is_defined() {
            [
                ext.min_at(0),
                ext.min_at(1),
                ext.min_at(2),
                ext.max_at(0),
                ext.max_at(1),
                ext.max_at(2),
            ]
        } else {
            [f64::NAN; 6]
        };
        let mut vertices = Vec::new();
        for v in brep1::vertices(b, None)? {
            let p = vertex::point(b, v)?;
            vertices.push([p.x, p.y, p.z]);
        }
        let [x0, y0, z0, x1, y1, z1] = bounds;
        let centre = [(x0 + x1) / 2.0, (y0 + y1) / 2.0, (z0 + z1) / 2.0];
        Ok(BrepSection {
            k,
            b,
            chord,
            bounds,
            centre,
            vertices,
        })
    }

    /// The chord tolerance (mm).
    #[must_use]
    pub fn chord(&self) -> f64 {
        self.chord
    }

    /// The plane position actually sectioned for `axis = at`: `at` moved by the first of
    /// [`NUDGES`] that keeps [`VERTEX_CLEARANCE`] from every vertex (the last one tried when
    /// none does). A plane strictly inside the body's bounds is never nudged out of them, so a
    /// slice just below a sphere's pole stays a small disc.
    #[must_use]
    pub fn nudged(&self, axis: Axis, at: f64) -> f64 {
        let i = axis.index();
        let (lo, hi) = (
            self.bounds.get(i).copied().unwrap_or(f64::NAN),
            self.bounds.get(i + 3).copied().unwrap_or(f64::NAN),
        );
        let inside = |c: f64| lo < c && c < hi;
        let keep_inside = inside(at);
        let clear = |c: f64| {
            (!keep_inside || inside(c))
                && self
                    .vertices
                    .iter()
                    .all(|p| p.get(i).is_none_or(|x| (x - c).abs() >= VERTEX_CLEARANCE))
        };
        let last = NUDGES.last().copied().unwrap_or(0.0);
        NUDGES
            .iter()
            .map(|d| at + d)
            .find(|&c| clear(c))
            .unwrap_or(at + last)
    }
}

impl Sectioner for BrepSection<'_> {
    fn bounds(&self) -> [f64; 6] {
        self.bounds
    }

    /// The section at the nudged position; when the kernel refuses that plane (a face whose
    /// surface it cannot cut there), the other nudges are tried, then the same planes with the
    /// section curves traced to half the chord instead of a tenth: the first that works.
    fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region> {
        let first = self.nudged(axis, at);
        let mut tries = vec![first];
        tries.extend(NUDGES.iter().map(|d| at + d).filter(|c| *c != first));
        let mut error = None;
        for loose in [false, true] {
            for &c in &tries {
                match section_at(self.k, self.b, axis, c, self.chord, self.centre, loose) {
                    Ok(r) => return Ok(r),
                    Err(e @ (FlatError::Setting(_) | FlatError::TooBig(_))) => return Err(e),
                    Err(e) => {
                        error.get_or_insert(e);
                    }
                }
            }
        }
        Err(error.unwrap_or(FlatError::Nothing))
    }
}

/// The material of body `b` on the plane `axis = at` exactly (no nudging), in the slice
/// coordinates of [`Axis::directions`], its outlines within `chord` mm of the exact section.
///
/// # Errors
/// [`FlatError::Setting`] for `at` not finite or a chord out of range; [`FlatError::TooBig`]
/// past [`MAX_SECTION_CURVES`] or [`MAX_SECTION_SAMPLES`]; the kernel's section and evaluation
/// errors.
pub fn plane_section(
    k: &Kernel,
    b: &mut Brep,
    axis: Axis,
    at: f64,
    chord: f64,
) -> FlatResult<Region> {
    section_at(k, b, axis, at, chord, [0.0; 3], false)
}

/// [`plane_section`] with the plane's point taken at `centre` moved onto the plane (the body's
/// middle, so the kernel's bounded plane sits over the body however far it is from the origin),
/// and with `loose` the section curves traced to half the chord instead of a tenth.
fn section_at(
    k: &Kernel,
    b: &mut Brep,
    axis: Axis,
    at: f64,
    chord: f64,
    centre: [f64; 3],
    loose: bool,
) -> FlatResult<Region> {
    if !at.is_finite() || at.abs() >= 1e6 {
        return Err(FlatError::Setting("section position"));
    }
    if !chord_ok(chord) {
        return Err(FlatError::Setting("chord"));
    }
    let (n, u, v) = axis.directions();
    let mut point = if centre.iter().all(|x| x.is_finite() && x.abs() < 1e6) {
        centre
    } else {
        [0.0; 3]
    };
    if let Some(p) = point.get_mut(axis.index()) {
        *p = at;
    }
    let opts = PlanarSectionOptions {
        // the section curves themselves stay well inside the outline's chord budget
        approx_tol_3d: Some((chord * if loose { 0.5 } else { 0.1 }).max(1e-6)),
        outputs: SectionOutputs {
            curves_3d: true,
            ..SectionOutputs::default()
        },
        ..PlanarSectionOptions::default()
    };
    let made = create_planar_section_curves(
        k,
        b,
        Point3::new(point[0], point[1], point[2]),
        Vec3::new(n[0], n[1], n[2]),
        &opts,
    )?;
    let curves = made.curves_3d.unwrap_or_default();
    if curves.len() > MAX_SECTION_CURVES {
        return Err(FlatError::TooBig("section curves"));
    }
    let mut budget = MAX_SECTION_SAMPLES;
    let mut pieces: Vec<Ring> = Vec::new();
    for c in &curves {
        let pts = sample_curve(k, c, chord, &mut budget)?;
        let flat: Ring = pts
            .iter()
            .map(|p| [dot(*p, u), dot(*p, v)])
            .filter(|q| q.iter().all(|x| x.is_finite()))
            .collect();
        let flat = drop_repeats(flat);
        if flat.len() >= 2 {
            pieces.push(flat);
        }
    }
    let tol = join_tolerance(chord);
    let pieces = dedupe(pieces, tol);
    let rings: Vec<Ring> = chain(pieces, tol)
        .into_iter()
        .map(|r| region::simplify_ring(&r, chord * 0.05))
        .collect();
    Ok(region::from_rings(&rings, true))
}

fn dot(p: Point3, d: [f64; 3]) -> f64 {
    p.x * d[0] + p.y * d[1] + p.z * d[2]
}

/// How far apart (mm) two curve ends may be and still join: the kernel's section curves of
/// neighbouring faces meet within its tolerances, far below any useful chord.
fn join_tolerance(chord: f64) -> f64 {
    chord.clamp(1e-3, 0.05)
}

/// The points of a curve from its start to its end, each chord within `chord` of the curve
/// (checked at the quarter points of every span), taken from `budget`.
fn sample_curve(k: &Kernel, c: &Curve, chord: f64, budget: &mut usize) -> FlatResult<Vec<Point3>> {
    let ivl = c.natural_interval();
    let (t0, t1) = (ivl.min(), ivl.max());
    if !(t0.is_finite() && t1.is_finite() && t1 > t0) {
        return Ok(Vec::new());
    }
    let Ok(first) = c.evaluate_point(k, t0) else {
        // a kind without a point evaluator (a composite curve): the kernel's tessellation
        let tess = c.tessellate(k, &ivl, chord, 10.0, 1)?;
        take(budget, tess.points.len())?;
        return Ok(tess.points);
    };
    take(budget, 1)?;
    let mut out = vec![first];
    let mut a = (t0, first);
    for i in 1..=FIRST_SPANS {
        let tb = t0 + (t1 - t0) * i as f64 / FIRST_SPANS as f64;
        let b = (tb, c.evaluate_point(k, tb)?);
        refine(k, c, a, b, chord, 0, budget, &mut out)?;
        a = b;
    }
    Ok(out)
}

/// Takes `n` samples from the budget.
fn take(budget: &mut usize, n: usize) -> FlatResult<()> {
    *budget = budget
        .checked_sub(n)
        .ok_or(FlatError::TooBig("section sample points"))?;
    Ok(())
}

/// Appends the samples of the span from `a` to `b` (`a` already out, `b` last), halving it
/// while its middle or a quarter point strays more than `chord` from the chord.
fn refine(
    k: &Kernel,
    c: &Curve,
    a: (f64, Point3),
    b: (f64, Point3),
    chord: f64,
    depth: u32,
    budget: &mut usize,
    out: &mut Vec<Point3>,
) -> FlatResult<()> {
    let tm = 0.5 * (a.0 + b.0);
    let pm = c.evaluate_point(k, tm)?;
    let mut far = off_chord(pm, a.1, b.1);
    if far <= chord {
        for f in [0.25, 0.75] {
            let p = c.evaluate_point(k, a.0 + (b.0 - a.0) * f)?;
            far = far.max(off_chord(p, a.1, b.1));
        }
    }
    if far > chord && depth < MAX_DEPTH && tm > a.0 && tm < b.0 {
        refine(k, c, a, (tm, pm), chord, depth + 1, budget, out)?;
        refine(k, c, (tm, pm), b, chord, depth + 1, budget, out)?;
    } else {
        take(budget, 1)?;
        out.push(b.1);
    }
    Ok(())
}

/// The distance from `p` to the segment `a`-`b`.
fn off_chord(p: Point3, a: Point3, b: Point3) -> f64 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// The polyline without points that repeat their predecessor.
fn drop_repeats(r: Ring) -> Ring {
    let mut out: Ring = Vec::with_capacity(r.len());
    for p in r {
        if out.last().is_none_or(|q| !near(*q, p, 1e-9)) {
            out.push(p);
        }
    }
    out
}

fn near(a: P2, b: P2, tol: f64) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= tol
}

/// The polylines with copies left out: a curve on a seam may come from the faces on both of
/// its sides. Two polylines are copies when their ends and middles meet within `tol`, in
/// either direction.
fn dedupe(pieces: Vec<Ring>, tol: f64) -> Vec<Ring> {
    // the middle is the mean of the two middle points, the same in either direction
    let key = |r: &Ring| -> Option<(P2, P2, P2)> {
        let (m0, m1) = (r.get((r.len() - 1) / 2)?, r.get(r.len() / 2)?);
        let mid = [(m0[0] + m1[0]) / 2.0, (m0[1] + m1[1]) / 2.0];
        Some((*r.first()?, mid, *r.last()?))
    };
    let mut kept: Vec<Ring> = Vec::new();
    let mut keys: Vec<(P2, P2, P2)> = Vec::new();
    for r in pieces {
        let Some((a, m, b)) = key(&r) else { continue };
        let copy = keys.iter().any(|&(c, n, d)| {
            near(m, n, tol)
                && ((near(a, c, tol) && near(b, d, tol)) || (near(a, d, tol) && near(b, c, tol)))
        });
        if !copy {
            keys.push((a, m, b));
            kept.push(r);
        }
    }
    kept
}

/// The polylines joined end to end into closed rings. A polyline that closes on itself is a
/// ring; otherwise the nearest free end within `tol` is joined (either direction). A chain
/// that does not close within `tol` is closed anyway when its gap is under a millimetre (a
/// curve the kernel left short), and dropped otherwise.
fn chain(mut pieces: Vec<Ring>, tol: f64) -> Vec<Ring> {
    // taken from the front so the result follows the kernel's curve order
    pieces.reverse();
    let mut rings: Vec<Ring> = Vec::new();
    while let Some(mut cur) = pieces.pop() {
        while let (Some(&start), Some(&end)) = (cur.first(), cur.last()) {
            if cur.len() >= 3 && near(start, end, tol) {
                break;
            }
            let Some((i, reversed)) = nearest_end(&pieces, end, tol) else {
                break;
            };
            let mut next = pieces.remove(i);
            if reversed {
                next.reverse();
            }
            cur.extend(next.into_iter().skip(1));
        }
        let (Some(&start), Some(&end)) = (cur.first(), cur.last()) else {
            continue;
        };
        if near(start, end, tol) {
            cur.pop();
        } else if !near(start, end, 1.0) {
            continue;
        }
        if cur.len() >= 3 {
            rings.push(cur);
        }
    }
    rings
}

/// The free polyline whose start (or end, `true`) is nearest to `p`, within `tol`.
fn nearest_end(pieces: &[Ring], p: P2, tol: f64) -> Option<(usize, bool)> {
    let d = |q: Option<&P2>| q.map_or(f64::INFINITY, |q| (q[0] - p[0]).hypot(q[1] - p[1]));
    let mut best: Option<(f64, usize, bool)> = None;
    for (i, r) in pieces.iter().enumerate() {
        for (dist, rev) in [(d(r.first()), false), (d(r.last()), true)] {
            if dist <= tol && best.is_none_or(|b| dist < b.0) {
                best = Some((dist, i, rev));
            }
        }
    }
    best.map(|(_, i, r)| (i, r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::api_brep::boolean_difference;
    use crate::primitive::api_primitives::{
        create_box, create_cylinder, create_sphere, create_torus,
    };
    use std::f64::consts::PI;

    const CHORD: f64 = 0.01;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    fn bounds_near(r: &Region, want: [f64; 4], tol: f64) {
        let b = region::bounds(r).unwrap();
        for (g, w) in b.iter().zip(want) {
            assert!(close(*g, w, tol), "{b:?} vs {want:?}");
        }
    }

    /// The area an inscribed polygon may miss of a disc of radius `r` with sides within
    /// `chord` of the circle: at most the perimeter times the chord.
    fn disc_slack(r: f64) -> f64 {
        2.0 * PI * r * CHORD + 1e-6
    }

    #[test]
    fn a_box_section_is_its_rectangle_with_exact_corners() {
        let mut k = Kernel::default();
        let mut b = create_box(&mut k, Point3::ZERO, 100.0, 60.0, 40.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        let bb = s.bounds();
        for (got, want) in bb.iter().zip([0.0, 0.0, 0.0, 100.0, 60.0, 40.0]) {
            assert!(close(*got, want, 1e-6), "{bb:?}");
        }
        let r = s.section(Axis::Z, 20.0).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].holes.is_empty());
        assert!(
            close(region::area(&r), 6000.0, 1e-6),
            "{}",
            region::area(&r)
        );
        assert_eq!(r[0].outer.len(), 4, "{:?}", r[0].outer);
        for corner in [[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]] {
            assert!(
                r[0].outer.iter().any(|p| near(*p, corner, 1e-6)),
                "{corner:?}"
            );
        }
        assert!(region::ring_area(&r[0].outer) > 0.0);
    }

    #[test]
    fn a_box_far_from_the_origin_is_cut_where_it_is() {
        // a part placed a metre away, as parts of an assembly are: every layer is its rectangle
        let mut k = Kernel::default();
        let mut b = create_box(&mut k, Point3::new(13.0, -1069.0, 864.0), 10.0, 100.0, 7.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        for z in [865.5, 867.5, 870.5] {
            let r = s.section(Axis::Z, z).unwrap();
            assert!(close(region::area(&r), 1000.0, 1e-6), "{z}: {}", region::area(&r));
            let bb = region::bounds(&r).unwrap();
            assert!(close(bb[0], 13.0, 1e-6) && close(bb[1], -1069.0, 1e-6), "{bb:?}");
        }
    }

    #[test]
    fn box_sections_across_x_and_y_use_the_slice_directions() {
        let mut k = Kernel::default();
        let mut b = create_box(&mut k, Point3::ZERO, 100.0, 60.0, 40.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        // X slices are (y, z); Y slices are (x, z)
        bounds_near(
            &s.section(Axis::X, 30.0).unwrap(),
            [0.0, 0.0, 60.0, 40.0],
            1e-6,
        );
        bounds_near(
            &s.section(Axis::Y, 30.0).unwrap(),
            [0.0, 0.0, 100.0, 40.0],
            1e-6,
        );
    }

    #[test]
    fn a_sphere_section_is_a_disc_within_the_chord() {
        let mut k = Kernel::default();
        let mut b = create_sphere(&mut k, Point3::new(0.0, 0.0, 50.0), 50.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        for z in [0.0, 10.0, 30.0, 45.0] {
            let rho2 = 50.0 * 50.0 - z * z;
            let r = s.section(Axis::Z, 50.0 + z).unwrap();
            assert_eq!(r.len(), 1, "z {z}");
            let a = region::area(&r);
            let want = PI * rho2;
            assert!(
                a <= want + 1e-6 && a >= want - disc_slack(rho2.sqrt()),
                "z {z}: {a} vs {want}"
            );
        }
        // across X too: a disc about (y, z) = (0, 50)
        let r = s.section(Axis::X, 30.0).unwrap();
        let want = PI * (2500.0 - 900.0);
        assert!(close(region::area(&r), want, disc_slack(40.0)));
        bounds_near(&r, [-40.0, 10.0, 40.0, 90.0], 1e-3);
    }

    #[test]
    fn a_plane_through_the_sphere_pole_is_nudged_off_it() {
        let mut k = Kernel::default();
        let mut b = create_sphere(&mut k, Point3::ZERO, 10.0).unwrap();
        let s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        assert!(close(s.nudged(Axis::Z, 10.0), 10.01, 1e-12));
        assert!(close(s.nudged(Axis::Z, -10.0), -9.99, 1e-12));
        assert!(close(s.nudged(Axis::Z, 3.0), 3.0, 0.0));
        // inside the bounds, the nudge stays inside
        assert!(close(s.nudged(Axis::Z, 9.999), 9.989, 1e-12));
        let mut s = s;
        // just inside the top: a small disc, not nothing
        assert!(!s.section(Axis::Z, 9.999).unwrap().is_empty());
    }

    #[test]
    fn a_cylinder_across_and_along_its_axis() {
        let mut k = Kernel::default();
        let mut b = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        let disc = s.section(Axis::Z, 15.0).unwrap();
        assert!(close(region::area(&disc), PI * 100.0, disc_slack(10.0)));
        // along the axis, away from the seam: the exact 20 x 30 rectangle
        let side = s.section(Axis::X, 0.0).unwrap();
        assert!(
            close(region::area(&side), 600.0, 1e-6),
            "{}",
            region::area(&side)
        );
        bounds_near(&side, [-10.0, 0.0, 10.0, 30.0], 1e-6);
        // through the seam line at y = 0: nudged by 0.01, so a hair narrower
        let seam = s.section(Axis::Y, 0.0).unwrap();
        assert!(
            close(region::area(&seam), 600.0, 1e-3),
            "{}",
            region::area(&seam)
        );
    }

    #[test]
    fn a_torus_through_its_axis_gives_two_discs() {
        let mut k = Kernel::default();
        let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        let r = s.section(Axis::X, 0.0).unwrap();
        assert_eq!(r.len(), 2, "{r:?}");
        for shape in &r {
            assert!(shape.holes.is_empty());
            let a = region::shape_area(shape);
            assert!(close(a, PI * 225.0, disc_slack(15.0)), "{a}");
            let c = region::bounds(std::slice::from_ref(shape)).unwrap();
            assert!(close(((c[0] + c[2]) / 2.0).abs(), 40.0, 1e-3), "{c:?}");
            assert!(close((c[1] + c[3]) / 2.0, 20.0, 1e-3), "{c:?}");
        }
    }

    #[test]
    fn a_torus_from_above_is_an_annulus() {
        let mut k = Kernel::default();
        let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        let r = s.section(Axis::Z, 20.0).unwrap();
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0].holes.len(), 1);
        assert!(region::ring_area(&r[0].holes[0]) < 0.0);
        let want = PI * (55.0 * 55.0 - 25.0 * 25.0);
        let slack = disc_slack(55.0) + disc_slack(25.0) + 0.01;
        assert!(close(region::area(&r), want, slack), "{}", region::area(&r));
        assert!(!region::contains(&r, [0.0, 0.0]));
        assert!(region::contains(&r, [40.0, 0.0]));
    }

    #[test]
    fn a_hollow_box_has_a_hole() {
        let mut k = Kernel::default();
        let outer = create_box(&mut k, Point3::ZERO, 50.0, 50.0, 50.0).unwrap();
        let inner = create_box(&mut k, Point3::new(10.0, 10.0, 10.0), 30.0, 30.0, 30.0).unwrap();
        let mut b = boolean_difference(&mut k, outer, inner).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        let r = s.section(Axis::Z, 25.0).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].holes.len(), 1);
        assert!(close(region::area(&r), 2500.0 - 900.0, 1e-6));
        // below the cavity: solid
        let solid = s.section(Axis::Z, 5.0).unwrap();
        assert!(close(region::area(&solid), 2500.0, 1e-6));
    }

    #[test]
    fn a_plane_that_misses_the_body_is_empty() {
        let mut k = Kernel::default();
        let mut b = create_box(&mut k, Point3::ZERO, 10.0, 10.0, 10.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        assert!(s.section(Axis::Z, 100.0).unwrap().is_empty());
        assert!(s.section(Axis::X, -5.0).unwrap().is_empty());
    }

    #[test]
    fn bad_numbers_are_refused() {
        let mut k = Kernel::default();
        let mut b = create_box(&mut k, Point3::ZERO, 10.0, 10.0, 10.0).unwrap();
        for c in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-9, 100.0] {
            assert!(matches!(
                BrepSection::new(&k, &mut b, c),
                Err(FlatError::Setting("chord"))
            ));
        }
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        for at in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300] {
            assert!(matches!(s.section(Axis::Z, at), Err(FlatError::Setting(_))));
        }
    }

    #[test]
    fn pieces_chain_in_any_direction_and_copies_go() {
        let a: Ring = vec![[0.0, 0.0], [1.0, 0.0]];
        let b: Ring = vec![[1.0, 1.0], [1.0, 0.0]]; // reversed
        let c: Ring = vec![[1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let copy: Ring = vec![[1.0, 0.0], [0.0, 0.0]];
        let pieces = dedupe(vec![a, b, c, copy], 1e-3);
        assert_eq!(pieces.len(), 3);
        let rings = chain(pieces, 1e-3);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 4);
        assert!(close(region::ring_area(&rings[0]).abs(), 1.0, 1e-12));
        // a far-open chain is dropped, a slightly short one closed
        assert!(chain(vec![vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]]], 1e-3).is_empty());
        assert_eq!(
            chain(vec![vec![[0.0, 0.0], [5.0, 0.0], [0.0, 0.5]]], 1e-3).len(),
            1
        );
    }

    #[test]
    fn drop_repeats_keeps_one_of_each_run() {
        let r = drop_repeats(vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 0.0]]);
        assert_eq!(r, vec![[0.0, 0.0], [1.0, 0.0]]);
        assert!(drop_repeats(Vec::new()).is_empty());
    }

    #[test]
    fn off_chord_measures_to_the_segment() {
        let a = Point3::ZERO;
        let b = Point3::new(10.0, 0.0, 0.0);
        assert!(close(
            off_chord(Point3::new(5.0, 3.0, 0.0), a, b),
            3.0,
            1e-12
        ));
        assert!(close(
            off_chord(Point3::new(-4.0, 3.0, 0.0), a, b),
            5.0,
            1e-12
        ));
        assert!(close(
            off_chord(Point3::new(0.0, 2.0, 0.0), a, a),
            2.0,
            1e-12
        ));
    }
}
