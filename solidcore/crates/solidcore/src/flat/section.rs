//! The section of a B-rep by a plane square to a model axis, as a polygonal [`Region`].
//!
//! The kernel's exact planar section curves
//! ([`create_planar_section_curves`]) are sampled by the kernel's tessellation
//! ([`Curve::tessellate`]), projected to the slice's `(u, v)` of [`Axis::directions`], chained
//! end to end into closed loops, simplified and filled even-odd, so a loop inside another is a
//! hole. Tracing, sampling and simplifying each take a share of the caller's chord tolerance,
//! the shares adding up to all of it, so the outline stays within it of the exact section.
//! Where two curves meet, the end of one is dropped for the start of the next when they are
//! within the join tolerance (the chord held to 1e-3..=0.05 mm), which may add that much
//! there: the hard bound is the chord plus the join tolerance. The kernel's curves meet far
//! closer than that (about 1e-13 mm in the tests), so in practice the outline is within the
//! chord. Curves whose ends do not join into closed loops leave the section incomplete, and it
//! is refused ([`FlatError::OpenSection`]).
//!
//! A plane through a vertex of the body, or lying in one of its planar faces, has no
//! well-defined section (a face in the plane is material on one side only; a sphere's pole
//! gives a curve of zero length). [`BrepSection`] then moves
//! the plane by [`NUDGES`] (at most 0.023 mm, far less than a laser kerf) to the first offset
//! that keeps [`VERTEX_CLEARANCE`] from every vertex. A planar face bounded only by edges
//! without vertices is not detected; such faces are rare in practice.
//!
//! Deviations: none (no upstream counterpart). No recursion.

use crate::curve::curve::Curve;
use crate::geom::vector3d::{Point3, Vec3};
use crate::kernel::Kernel;
use crate::nurbs::b_spline_curve::Tessellation;
use crate::query::api_queries::brep_bounding_box;
use crate::topology::brep::p1 as brep1;
use crate::topology::brep::planar_section::{
    create_planar_section_curves, PlanarSectionOptions, SectionOutputs,
};
use crate::topology::{vertex, Brep, FaceId};

use super::region;
use super::slice::{Axis, Sectioner};
use super::{FlatError, FlatResult, Region, Ring, P2};

/// The most section curves one plane may give.
pub const MAX_SECTION_CURVES: usize = 20_000;

/// The most sample points one section may have, all curves together.
pub const MAX_SECTION_SAMPLES: usize = 2_000_000;

/// The least and the most (mm) [`join_tolerance`] allows curve ends to be apart.
const JOIN_RANGE: (f64, f64) = (1e-3, 0.05);

/// How far (mm) a section plane keeps from every vertex of the body when it can: twice the
/// least join tolerance, so the section curves of the faces about a vertex, which pass within
/// the plane's distance of it, do not come within a join of each other there.
pub const VERTEX_CLEARANCE: f64 = 2.0 * JOIN_RANGE.0;

/// The first nudge (mm), 0.01: five clearances, so a plane on a vertex clears it with room to
/// spare, and the clearance about each nudge stays apart from the plane's own.
const NUDGE_NEAR: f64 = 5.0 * VERTEX_CLEARANCE;

/// The second nudge (mm), 0.023: not a multiple of the first, so vertices on a grid of the
/// first's pitch do not block both, and still a small part of a laser kerf (0.1 to 0.2 mm).
const NUDGE_FAR: f64 = 11.5 * VERTEX_CLEARANCE;

/// The plane offsets (mm) tried in order when the plane comes too close to a vertex.
pub const NUDGES: [f64; 5] = [0.0, NUDGE_NEAR, -NUDGE_NEAR, NUDGE_FAR, -NUDGE_FAR];

/// The smallest and largest chord tolerances (mm) a section accepts.
pub(crate) const CHORD_RANGE: (f64, f64) = (1e-4, 10.0);

// The three steps from the exact section to an outline share the chord tolerance: the kernel
// traces the section curves within `TRACE_SHARE` of it, the samples' chords stay within
// `SAMPLE_SHARE` of the traced curves, and simplifying keeps the outline within
// `SIMPLIFY_SHARE` of the samples. Their errors add up, so the shares add up to 1 and the
// outline stays within the chord of the exact section, except where curve ends are joined
// (see `join_tolerance`, which is not a share).
const TRACE_SHARE: f64 = 0.1;
const SAMPLE_SHARE: f64 = 0.85;
const SIMPLIFY_SHARE: f64 = 0.05;

/// [`CHORD_RANGE`] in words, for [`FlatError::Setting`].
pub(crate) const CHORD_LIMIT: &str = "0.0001 to 10 mm";

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
    /// The sectioner of body `b`, its outlines within `chord` mm of the exact section, plus the
    /// join tolerance where section curves meet (see the module docs).
    ///
    /// The bounds are the kernel's tight bounding box; a body without geometry has bounds that
    /// are not finite, which [`super::slice::make`] reads as nothing to cut.
    ///
    /// # Errors
    /// [`FlatError::Setting`] for a chord not finite or outside `1e-4..=10` mm; the kernel's
    /// errors listing the vertices or computing the box.
    pub fn new(k: &'a Kernel, b: &'a mut Brep, chord: f64) -> FlatResult<Self> {
        if !chord_ok(chord) {
            return Err(FlatError::Setting {
                name: "chord",
                limit: CHORD_LIMIT,
            });
        }
        let ext = brep_bounding_box(k, b, true)?;
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

    /// The plane position actually sectioned for `axis = at`: `at` moved by the first of
    /// [`NUDGES`] that keeps [`VERTEX_CLEARANCE`] from every vertex (when none does, the one
    /// farthest from every vertex). A plane strictly inside the body's bounds is never nudged
    /// out of them, so a slice just below a sphere's pole stays a small disc.
    #[must_use]
    pub fn nudged(&self, axis: Axis, at: f64) -> f64 {
        let i = axis.index();
        let (lo, hi) = (
            self.bounds.get(i).copied().unwrap_or(f64::NAN),
            self.bounds.get(i + 3).copied().unwrap_or(f64::NAN),
        );
        let inside = |c: f64| lo < c && c < hi;
        let keep_inside = inside(at);
        // the distance to the nearest vertex along the axis
        let room = |c: f64| {
            self.vertices
                .iter()
                .filter_map(|p| p.get(i))
                .fold(f64::INFINITY, |m, x| m.min((x - c).abs()))
        };
        // `at` itself (the nudge 0) is always among them
        let tried = || {
            NUDGES
                .iter()
                .map(move |d| at + d)
                .filter(move |&c| !keep_inside || inside(c))
        };
        tried()
            .find(|&c| room(c) >= VERTEX_CLEARANCE)
            .or_else(|| tried().max_by(|a, b| room(*a).total_cmp(&room(*b))))
            .unwrap_or(at)
    }
}

impl Sectioner for BrepSection<'_> {
    fn bounds(&self) -> [f64; 6] {
        self.bounds
    }

    /// The section at the nudged position ([`BrepSection::nudged`]); its errors name the
    /// position asked for.
    fn section(&mut self, axis: Axis, at: f64) -> FlatResult<Region> {
        let plane = self.nudged(axis, at);
        section_at(self.k, self.b, axis, plane, self.chord, self.centre).map_err(|e| match e {
            FlatError::Section { axis, error, .. } => FlatError::Section { axis, at, error },
            FlatError::OpenSection { axis, gap, .. } => FlatError::OpenSection { axis, at, gap },
            other => other,
        })
    }
}

/// The material of body `b` on the plane `axis = at` exactly (no nudging), in the slice
/// coordinates of [`Axis::directions`], its outlines within `chord` mm of the exact section
/// (plus [`join_tolerance`] where curves meet). The plane's point is `centre` moved onto the
/// plane (the body's middle, so the kernel's bounded plane sits over the body however far it is
/// from the origin).
///
/// # Errors
/// [`FlatError::Setting`] for `at` not finite or a chord out of range; [`FlatError::TooBig`]
/// past [`MAX_SECTION_CURVES`] or [`MAX_SECTION_SAMPLES`]; [`FlatError::Section`] for the
/// kernel's section and evaluation errors; [`FlatError::OpenSection`] for section curves that
/// do not close into outlines.
fn section_at(
    k: &Kernel,
    b: &mut Brep,
    axis: Axis,
    at: f64,
    chord: f64,
    centre: [f64; 3],
) -> FlatResult<Region> {
    let (curves, faces) = section_curves(k, b, axis, at, chord, centre)?;
    let (_, u, v) = axis.directions();
    let mut budget = MAX_SECTION_SAMPLES;
    let mut pieces: Vec<(Ring, Option<FaceId>)> = Vec::new();
    for (i, c) in curves.iter().enumerate() {
        let samples = sample_curve(k, c, chord * SAMPLE_SHARE)
            .map_err(|error| FlatError::Section { axis, at, error })?;
        take(&mut budget, samples.points.len())?;
        let flat: Ring = samples
            .points
            .iter()
            .map(|p| [dot(*p, u), dot(*p, v)])
            .filter(|q| q.iter().all(|x| x.is_finite()))
            .collect();
        let flat = drop_repeats(flat);
        if flat.len() >= 2 {
            pieces.push((flat, faces.get(i).copied()));
        }
    }
    let rings = outlines(pieces, axis, at, chord)?;
    Ok(region::from_rings(&rings, true))
}

/// The sampled pieces of the section by the plane `axis = at` as its outlines: copies left
/// out, chained into closed rings and simplified within `SIMPLIFY_SHARE` of `chord`.
///
/// # Errors
/// [`FlatError::OpenSection`] with the gap when the pieces do not close into rings within
/// the join tolerance: the section is incomplete.
fn outlines(
    pieces: Vec<(Ring, Option<FaceId>)>,
    axis: Axis,
    at: f64,
    chord: f64,
) -> FlatResult<Vec<Ring>> {
    let tol = join_tolerance(chord);
    let rings =
        chain(dedupe(pieces, tol), tol).map_err(|gap| FlatError::OpenSection { axis, at, gap })?;
    Ok(rings
        .into_iter()
        .map(|r| region::simplify_ring(&r, chord * SIMPLIFY_SHARE))
        .collect())
}

/// The kernel's exact section curves of body `b` by the plane `axis = at` through `centre`
/// moved onto the plane (see [`section_at`]), traced within `TRACE_SHARE` of `chord`, and the
/// face each curve lies on.
///
/// # Errors
/// [`FlatError::Setting`] for `at` not finite or a kilometre or more from the origin, or a chord
/// out of range; [`FlatError::Section`] for the kernel's errors and for a face list that is not
/// one face per curve; [`FlatError::TooBig`] past [`MAX_SECTION_CURVES`].
fn section_curves(
    k: &Kernel,
    b: &mut Brep,
    axis: Axis,
    at: f64,
    chord: f64,
    centre: [f64; 3],
) -> FlatResult<(Vec<Curve>, Vec<FaceId>)> {
    if !(at.is_finite() && at.abs() < region::KM) {
        return Err(FlatError::Setting {
            name: "section position",
            limit: "finite and less than 1000000 mm from the origin",
        });
    }
    if !chord_ok(chord) {
        return Err(FlatError::Setting {
            name: "chord",
            limit: CHORD_LIMIT,
        });
    }
    let (n, _, _) = axis.directions();
    let mut point = centre;
    if let Some(p) = point.get_mut(axis.index()) {
        *p = at;
    }
    let opts = PlanarSectionOptions {
        approx_tol_3d: Some(chord * TRACE_SHARE),
        outputs: SectionOutputs {
            curves_3d: true,
            faces: true,
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
    )
    .map_err(|error| FlatError::Section { axis, at, error })?;
    let curves = made.curves_3d.ok_or(FlatError::Section {
        axis,
        at,
        error: crate::SolidError::PortInvariant {
            operation: "flat section",
            reason: "the kernel's section gave no list of curves",
        },
    })?;
    if curves.len() > MAX_SECTION_CURVES {
        return Err(FlatError::TooBig("section curves"));
    }
    let faces = face_per_curve(made.faces, curves.len()).map_err(|error| FlatError::Section {
        axis,
        at,
        error,
    })?;
    Ok((curves, faces))
}

/// The kernel's face list of a section, one face per curve.
///
/// # Errors
/// [`crate::SolidError::PortInvariant`] for a list that is missing or not one face per curve:
/// without the faces, [`dedupe`] cannot tell a seam's copies from two near outlines of one face.
fn face_per_curve(faces: Option<Vec<FaceId>>, curves: usize) -> crate::Result<Vec<FaceId>> {
    match faces {
        Some(f) if f.len() == curves => Ok(f),
        _ => Err(crate::SolidError::PortInvariant {
            operation: "flat section",
            reason: "the kernel's section did not give one face per curve",
        }),
    }
}

fn dot(p: Point3, d: [f64; 3]) -> f64 {
    p.x * d[0] + p.y * d[1] + p.z * d[2]
}

/// How far apart (mm) two curve ends may be and still join, and how close two polylines must
/// be to be copies: the chord, held to `1e-3..=0.05` mm. A join drops one of the two ends, so
/// where curves meet the outline may be off by this much on top of the chord: up to ten
/// chords at the smallest chord (1e-4 mm), less than the chord above 0.05 mm. It is not a
/// share of the chord: the kernel's section curves of neighbouring faces meet far closer
/// (about 1e-13 mm in the flat and app `fab` tests), and a tolerance as tight as a share would
/// refuse sections whose curves meet only within the kernel's tolerances, and keep both
/// copies of a seam curve, whose middle samples lie up to about one chord apart along it.
fn join_tolerance(chord: f64) -> f64 {
    chord.clamp(JOIN_RANGE.0, JOIN_RANGE.1)
}

/// The samples of a curve from its start to its end, each chord within `chord` of the curve:
/// the kernel's tessellation with its chord height test alone (the angle test off, no least
/// number of segments). A curve without a finite, non-empty interval is refused.
fn sample_curve(k: &Kernel, c: &Curve, chord: f64) -> crate::Result<Tessellation> {
    let ivl = c.natural_interval();
    if !(ivl.min().is_finite() && ivl.max().is_finite() && ivl.max() > ivl.min()) {
        return Err(crate::SolidError::PortInvariant {
            operation: "flat section",
            reason: "a section curve has no finite, non-empty interval",
        });
    }
    c.tessellate(k, &ivl, chord, 0.0, 1)
}

/// Takes `n` samples from the budget.
fn take(budget: &mut usize, n: usize) -> FlatResult<()> {
    *budget = budget
        .checked_sub(n)
        .ok_or(FlatError::TooBig("section sample points"))?;
    Ok(())
}

/// How close (mm) a sample must be to the one before it to repeat it: a nanometre, the rounding
/// between the last sample of one curve span and the first of the next.
const REPEAT: f64 = 1e-9;

/// The polyline without points that repeat their predecessor (within [`REPEAT`]).
fn drop_repeats(r: Ring) -> Ring {
    let mut out: Ring = Vec::new();
    for p in r {
        if out.last().is_none_or(|q| !near(*q, p, REPEAT)) {
            out.push(p);
        }
    }
    out
}

fn near(a: P2, b: P2, tol: f64) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) <= tol
}

/// How close (mm) the ends and middles of two polylines of one face must be for them to be
/// copies (a curve on the seam of a periodic face, traced from both of its sides).
const SAME_FACE_COPY: f64 = 1e-6;

/// The polylines with copies left out: a curve on a seam may come from the faces on both of
/// its sides. Two polylines are copies when their ends and middles meet within `tol`, in
/// either direction, and they come from different faces (or a face is not known). Two
/// polylines of one face are copies only within [`SAME_FACE_COPY`]: just under the top of a
/// torus its one face gives two circles that may be closer than `tol`, and both are outlines.
fn dedupe(pieces: Vec<(Ring, Option<FaceId>)>, tol: f64) -> Vec<Ring> {
    // the middle is the mean of the two middle points, the same in either direction
    let key = |r: &Ring| -> Option<(P2, P2, P2)> {
        let (m0, m1) = (r.get((r.len() - 1) / 2)?, r.get(r.len() / 2)?);
        let mid = [(m0[0] + m1[0]) / 2.0, (m0[1] + m1[1]) / 2.0];
        Some((*r.first()?, mid, *r.last()?))
    };
    let meets = |(a, m, b): (P2, P2, P2), (c, n, d): (P2, P2, P2), tol: f64| {
        near(m, n, tol)
            && ((near(a, c, tol) && near(b, d, tol)) || (near(a, d, tol) && near(b, c, tol)))
    };
    let mut kept: Vec<Ring> = Vec::new();
    let mut keys: Vec<((P2, P2, P2), Option<FaceId>)> = Vec::new();
    for (r, face) in pieces {
        let Some((a, m, b)) = key(&r) else { continue };
        let copy = keys.iter().any(|&(other, f)| {
            let tol = match (face, f) {
                (Some(x), Some(y)) if x == y => tol.min(SAME_FACE_COPY),
                _ => tol,
            };
            meets((a, m, b), other, tol)
        });
        if !copy {
            keys.push(((a, m, b), face));
            kept.push(r);
        }
    }
    kept
}

/// The polylines joined end to end into closed rings. A polyline that closes on itself is a
/// ring; otherwise the nearest free end within `tol` is joined (either direction). A chain
/// that closes with fewer than three points encloses nothing and gives no ring.
///
/// # Errors
/// The gap (mm) between the ends of the first chain that does not close within `tol`.
fn chain(mut pieces: Vec<Ring>, tol: f64) -> Result<Vec<Ring>, f64> {
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
            // taken in place: an empty piece has no ends to find and is skipped when its turn comes
            let Some(mut next) = pieces.get_mut(i).map(std::mem::take) else {
                break;
            };
            if reversed {
                next.reverse();
            }
            cur.extend(next.into_iter().skip(1));
        }
        let (Some(&start), Some(&end)) = (cur.first(), cur.last()) else {
            continue;
        };
        if !near(start, end, tol) {
            return Err((start[0] - end[0]).hypot(start[1] - end[1]));
        }
        cur.pop();
        if cur.len() >= 3 {
            rings.push(cur);
        }
    }
    Ok(rings)
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
    use crate::curve::curve::CurveKind;
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
        let mut b =
            create_box(&mut k, Point3::new(13.0, -1069.0, 864.0), 10.0, 100.0, 7.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        for z in [865.5, 867.5, 870.5] {
            let r = s.section(Axis::Z, z).unwrap();
            assert!(
                close(region::area(&r), 1000.0, 1e-6),
                "{z}: {}",
                region::area(&r)
            );
            let bb = region::bounds(&r).unwrap();
            assert!(
                close(bb[0], 13.0, 1e-6) && close(bb[1], -1069.0, 1e-6),
                "{bb:?}"
            );
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
                Err(FlatError::Setting { name: "chord", .. })
            ));
        }
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        for at in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e300] {
            assert!(matches!(
                s.section(Axis::Z, at),
                Err(FlatError::Setting { .. })
            ));
        }
    }

    #[test]
    fn pieces_chain_in_any_direction_and_copies_go() {
        let a: Ring = vec![[0.0, 0.0], [1.0, 0.0]];
        let b: Ring = vec![[1.0, 1.0], [1.0, 0.0]]; // reversed
        let c: Ring = vec![[1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let copy: Ring = vec![[1.0, 0.0], [0.0, 0.0]];
        let pieces = dedupe(
            [a, b, c, copy].into_iter().map(|r| (r, None)).collect(),
            1e-3,
        );
        assert_eq!(pieces.len(), 3);
        let rings = chain(pieces, 1e-3).unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 4);
        assert!(close(region::ring_area(&rings[0]).abs(), 1.0, 1e-12));
        // a closed chain of fewer than three points encloses nothing
        assert_eq!(
            chain(vec![vec![[0.0, 0.0], [1.0, 0.0], [0.0, 0.0]]], 1e-3),
            Ok(Vec::new())
        );
    }

    #[test]
    fn a_chain_that_does_not_close_is_refused_with_its_gap() {
        // far open, and only a little short: both refused, whatever the gap
        let gap = |pieces: Vec<Ring>| chain(pieces, 1e-3).unwrap_err();
        assert!(close(
            gap(vec![vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0]]]),
            50f64.sqrt(),
            1e-12
        ));
        assert!(close(
            gap(vec![vec![[0.0, 0.0], [5.0, 0.0], [0.0, 0.5]]]),
            0.5,
            1e-12
        ));
        // a ring and an open chain: the section is incomplete
        let square: Ring = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0]];
        let open: Ring = vec![[3.0, 0.0], [4.0, 0.0], [4.0, 1.0]];
        assert!(close(gap(vec![square, open]), 2f64.sqrt(), 1e-12));
        // the outlines of a section carry the plane and the gap
        let err = outlines(
            vec![(vec![[0.0, 0.0], [5.0, 0.0], [0.0, 0.5]], None)],
            Axis::Y,
            12.5,
            CHORD,
        );
        assert_eq!(
            err,
            Err(FlatError::OpenSection {
                axis: Axis::Y,
                at: 12.5,
                gap: 0.5
            })
        );
        let text = err.unwrap_err().to_string();
        assert!(
            text.contains("Y = 12.500 mm") && text.contains("0.500 mm"),
            "{text}"
        );
    }

    #[test]
    fn drop_repeats_keeps_one_of_each_run() {
        let r = drop_repeats(vec![[0.0, 0.0], [0.0, 0.0], [1.0, 0.0], [1.0, 0.0]]);
        assert_eq!(r, vec![[0.0, 0.0], [1.0, 0.0]]);
        assert!(drop_repeats(Vec::new()).is_empty());
    }

    /// The distance from `p` to the segment `a`-`b`.
    fn to_segment(p: Point3, a: Point3, b: Point3) -> f64 {
        let ab = b - a;
        let t = ((p - a).dot(ab) / ab.dot(ab)).clamp(0.0, 1.0);
        p.distance(a + ab * t)
    }

    /// The farthest curve `c` strays from the chords between its samples `s`: each span
    /// between two samples measured at 64 points of the curve.
    fn largest_chord_error(k: &Kernel, c: &Curve, s: &Tessellation) -> f64 {
        let mut far: f64 = 0.0;
        for (t, p) in s.parameters.windows(2).zip(s.points.windows(2)) {
            for i in 1..64 {
                let q = c
                    .evaluate_point(k, t[0] + (t[1] - t[0]) * f64::from(i) / 64.0)
                    .unwrap();
                far = far.max(to_segment(q, p[0], p[1]));
            }
        }
        far
    }

    #[test]
    fn samples_of_circles_and_splines_stay_within_the_chord() {
        let mut k = Kernel::default();
        // across a cylinder's axis the section is a circle; a torus cut parallel to its axis,
        // 30 mm from it, gives spiric curves, which the kernel traces as B-splines. The samples
        // stay within their share of the outline's chord.
        let tol = CHORD * SAMPLE_SHARE;
        let mut cylinder = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
        let mut torus = create_torus(&mut k, Point3::ZERO, 40.0, 15.0).unwrap();
        for (b, axis, at, spline) in [
            (&mut cylinder, Axis::Z, 15.0, false),
            (&mut torus, Axis::X, 30.0, true),
        ] {
            let (curves, _) = section_curves(&k, b, axis, at, CHORD, [0.0; 3]).unwrap();
            assert!(!curves.is_empty(), "{axis:?} = {at}");
            if spline {
                assert!(
                    curves
                        .iter()
                        .all(|c| matches!(c.kind(), CurveKind::BSpline(_))),
                    "{axis:?} = {at}"
                );
            }
            for c in &curves {
                let s = sample_curve(&k, c, tol).unwrap();
                assert!(s.points.len() > 8, "{axis:?} = {at}: {}", s.points.len());
                let far = largest_chord_error(&k, c, &s);
                assert!(far <= tol, "{axis:?} = {at}: {far} over the share {tol}");
            }
        }
    }

    #[test]
    fn the_chord_shares_add_up_to_at_most_the_chord() {
        // tracing, sampling and simplifying each add their error to the outline's, so the
        // outline stays within the chord only while the three shares together stay within it
        let shares = [TRACE_SHARE, SAMPLE_SHARE, SIMPLIFY_SHARE];
        assert!(shares.iter().all(|s| *s > 0.0), "{shares:?}");
        let sum: f64 = shares.iter().sum();
        assert!(sum <= 1.0, "{shares:?} add up to {sum}");
    }

    #[test]
    fn simplifying_drops_a_bump_within_its_share_of_the_chord_and_keeps_a_larger_one() {
        // a 10 mm square whose bottom side is sampled every 1 mm, with a bump at x = 5: at a
        // 1 mm chord the simplify share lies between 0.02 and 0.2 mm, so the small bump goes
        // and the large one stays
        let square = |bump: f64| -> Ring {
            let mut r: Ring = (0..=10)
                .map(|i| [f64::from(i), if i == 5 { bump } else { 0.0 }])
                .collect();
            r.extend([[10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]);
            r
        };
        for (bump, kept) in [(0.02, false), (0.2, true)] {
            let rings = outlines(vec![(square(bump), None)], Axis::Z, 0.0, 1.0).unwrap();
            assert_eq!(rings.len(), 1, "{rings:?}");
            let r = rings.first().unwrap();
            assert_eq!(r.contains(&[5.0, bump]), kept, "bump {bump}: {r:?}");
        }
    }

    /// The farthest the points of `ring` and the middles of its sides are from the circle of
    /// radius `r` about `c`.
    fn off_circle(ring: &Ring, c: P2, r: f64) -> f64 {
        let off = |p: P2| ((p[0] - c[0]).hypot(p[1] - c[1]) - r).abs();
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .map(|(a, b)| off(*a).max(off([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0])))
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_finished_outline_stays_within_the_chord_of_the_exact_circle() {
        // traced, sampled, chained and simplified: a cylinder across its axis and a sphere
        // across two axes, at a fine chord and a coarse one
        let mut k = Kernel::default();
        for chord in [CHORD, 0.5] {
            let mut cylinder = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
            let mut sphere = create_sphere(&mut k, Point3::new(0.0, 0.0, 50.0), 50.0).unwrap();
            for (b, cuts) in [
                (&mut cylinder, vec![(Axis::Z, 15.0, [0.0, 0.0], 10.0)]),
                (
                    &mut sphere,
                    vec![
                        (Axis::Z, 80.0, [0.0, 0.0], 40.0),
                        (Axis::X, 30.0, [0.0, 50.0], 40.0),
                    ],
                ),
            ] {
                let mut s = BrepSection::new(&k, b, chord).unwrap();
                for (axis, at, centre, radius) in cuts {
                    let r = s.section(axis, at).unwrap();
                    assert!(
                        r.len() == 1 && r[0].holes.is_empty(),
                        "{axis:?} = {at}: {r:?}"
                    );
                    // the kernel traces a circle exactly: only sampling and simplifying
                    // add their shares
                    let off = off_circle(&r[0].outer, centre, radius);
                    let share = (SAMPLE_SHARE + SIMPLIFY_SHARE) * chord;
                    assert!(off <= share, "{axis:?} = {at}, chord {chord}: {off} off");
                }
            }
        }
    }

    /// The plane Y = 30 cuts the torus `(rho - 40)^2 + (z - 20)^2 = 15^2` around Z in one closed
    /// spiric curve, which the kernel can only trace. A point (x, z) of that plane moved onto the
    /// exact curve along the gradient of its distance to the torus: a point of the curve, so its
    /// distance from (x, z) is at least the distance to the curve.
    fn onto_spiric(mut x: f64, mut z: f64) -> (f64, f64) {
        for _ in 0..8 {
            let rho = x.hypot(30.0);
            let s = (rho - 40.0).hypot(z - 20.0);
            let g = ((rho - 40.0) / s * x / rho, (z - 20.0) / s);
            let step = (s - 15.0) / (g.0 * g.0 + g.1 * g.1);
            x -= step * g.0;
            z -= step * g.1;
        }
        (x, z)
    }

    fn to_segment_2d(p: P2, a: P2, b: P2) -> f64 {
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = d[0] * d[0] + d[1] * d[1];
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p[0] - a[0] - t * d[0]).hypot(p[1] - a[1] - t * d[1])
    }

    #[test]
    fn a_traced_torus_section_stays_within_the_chord_of_the_exact_curve() {
        // traced (not an exact circle), sampled, chained and simplified: every step's share
        // counts, at a coarse chord and a fine one
        let mut k = Kernel::default();
        for chord in [0.5, 0.1] {
            let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
            let mut s = BrepSection::new(&k, &mut b, chord).unwrap();
            // Y slices have x and z as their coordinates
            let r = s.section(Axis::Y, 30.0).unwrap();
            assert!(r.len() == 1 && r[0].holes.is_empty(), "{r:?}");
            let ring = &r[0].outer;
            // every point of the outline lies within the chord of the exact curve
            let mut worst: f64 = 0.0;
            for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                for t in [0.0, 0.25, 0.5, 0.75] {
                    let p = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                    let (x, z) = onto_spiric(p[0], p[1]);
                    worst = worst.max((p[0] - x).hypot(p[1] - z));
                }
            }
            assert!(
                worst <= chord,
                "chord {chord}: the outline is {worst} off the curve"
            );
            // and every point of the exact curve lies within the chord of the outline
            let t0 = (-10.0_f64 / 15.0).acos();
            let mut missed: f64 = 0.0;
            for i in 0..=2000 {
                let t = -t0 + 2.0 * t0 * f64::from(i) / 2000.0;
                let rho = 40.0 + 15.0 * t.cos();
                let x = (rho * rho - 900.0).max(0.0).sqrt();
                for p in [[x, 20.0 + 15.0 * t.sin()], [-x, 20.0 + 15.0 * t.sin()]] {
                    let near = ring
                        .iter()
                        .zip(ring.iter().cycle().skip(1))
                        .map(|(a, b)| to_segment_2d(p, *a, *b))
                        .fold(f64::INFINITY, f64::min);
                    missed = missed.max(near);
                }
            }
            assert!(
                missed <= chord,
                "chord {chord}: the curve is {missed} off the outline"
            );
        }
    }

    #[test]
    fn near_copies_of_one_face_are_both_kept() {
        // two arcs 1e-4 apart: copies from two faces, both outlines from one face
        let arc = |r: f64| -> Ring {
            (0..=8)
                .map(|i| {
                    let t = f64::from(i) * PI / 8.0;
                    [r * t.cos(), r * t.sin()]
                })
                .collect()
        };
        let mut k = Kernel::default();
        let b = create_box(&mut k, Point3::ZERO, 1.0, 1.0, 1.0).unwrap();
        let fs = brep1::faces(&b, None).unwrap();
        let (f0, f1) = (fs.first().copied().unwrap(), fs.get(1).copied().unwrap());
        let two = |x, y| dedupe(vec![(arc(40.0), x), (arc(40.0001), y)], 0.01).len();
        assert_eq!(two(Some(f0), Some(f1)), 1);
        assert_eq!(two(None, Some(f0)), 1);
        assert_eq!(two(Some(f0), Some(f0)), 2);
        // a seam traced from both of its sides by one face is still one curve
        let same = dedupe(vec![(arc(40.0), Some(f0)), (arc(40.0), Some(f0))], 0.01);
        assert_eq!(same.len(), 1);
    }

    #[test]
    fn the_kernel_gives_one_face_per_section_curve() {
        let mut k = Kernel::default();
        let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        for (axis, at) in [(Axis::Z, 20.0), (Axis::X, 0.0), (Axis::Z, 34.0)] {
            let (curves, faces) = section_curves(&k, &mut b, axis, at, CHORD, [0.0; 3]).unwrap();
            assert!(!curves.is_empty());
            assert_eq!(faces.len(), curves.len(), "{axis:?} {at}: {faces:?}");
        }
    }

    #[test]
    fn a_face_list_that_is_not_one_per_curve_is_refused() {
        let mut k = Kernel::default();
        let b = create_box(&mut k, Point3::ZERO, 1.0, 1.0, 1.0).unwrap();
        let f = brep1::faces(&b, None).unwrap();
        assert_eq!(
            face_per_curve(Some(f[..2].to_vec()), 2),
            Ok(f[..2].to_vec())
        );
        for (faces, curves) in [
            (None, 2),
            (Some(f[..1].to_vec()), 2),
            (Some(f[..3].to_vec()), 2),
        ] {
            assert!(
                matches!(
                    face_per_curve(faces, curves),
                    Err(crate::SolidError::PortInvariant { .. })
                ),
                "{curves}"
            );
        }
    }

    #[test]
    fn a_blocked_plane_inside_the_bounds_stays_inside() {
        let mut k = Kernel::default();
        let mut b = create_sphere(&mut k, Point3::ZERO, 10.0).unwrap();
        let mut s = BrepSection::new(&k, &mut b, CHORD).unwrap();
        // vertices on every nudge of -9.99 that stays inside: none is clear, and the one
        // farthest from a vertex among those inside is taken, not the last nudge (-10.013)
        s.vertices = [-9.99, -9.98, -9.967].map(|z| [0.0, 0.0, z]).to_vec();
        let c = s.nudged(Axis::Z, -9.99);
        assert!(s.bounds[2] < c && c < s.bounds[5], "{c} {:?}", s.bounds);
        // one blocked: the next nudge that clears
        s.vertices = vec![[0.0, 0.0, -9.99]];
        assert!(close(s.nudged(Axis::Z, -9.99), -9.98, 1e-12));
    }

    #[test]
    fn a_little_under_the_top_of_a_torus_is_a_thin_ring() {
        // the two circles of radius 40 +- sqrt(2 * 15 * 1e-5) are 0.035 mm apart: a ring of
        // about 8.7 mm^2 (closer to the top, see the ignored test below)
        let mut k = Kernel::default();
        let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        let r = section_at(&k, &mut b, Axis::Z, 35.0 - 1e-5, CHORD, [0.0; 3]).unwrap();
        let want = 160.0 * PI * (30.0 * 1e-5f64).sqrt();
        assert!(close(region::area(&r), want, 1.0), "{}", region::area(&r));
    }

    fn stacked(axis: Axis, thickness: f64) -> super::super::slice::FabSettings {
        super::super::slice::FabSettings {
            style: super::super::slice::Style::Stacked,
            axis,
            thickness,
            spacing: 0.0,
            count: 0,
            clearance: 0.2,
            rod_diameter: 8.0,
            rod_at: None,
            dowels: 0,
            dowel_diameter: 6.0,
            wall: 4.0,
            plate_margin: 10.0,
            guide_outline: false,
            min_area: 4.0,
            chord: CHORD,
        }
    }

    #[test]
    fn a_plane_that_only_touches_gives_nothing_and_no_error() {
        let mut k = Kernel::default();
        // tangent to a sphere at its pole, exactly and nudged
        let mut sphere = create_sphere(&mut k, Point3::ZERO, 10.0).unwrap();
        let r = section_at(&k, &mut sphere, Axis::Z, 10.0, CHORD, [0.0; 3]).unwrap();
        assert!(region::area(&r) < 1e-6, "{r:?}");
        let mut s = BrepSection::new(&k, &mut sphere, CHORD).unwrap();
        assert!(region::area(&s.section(Axis::Z, 10.0).unwrap()) < 1e-6);
        // on the flat end of a cylinder (coplanar with a face): the exact plane gives the face
        // itself (the material on the plane), the slicer's nudged plane (30.01) gives nothing
        let mut cylinder = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
        let r = section_at(&k, &mut cylinder, Axis::Z, 30.0, CHORD, [0.0; 3]).unwrap();
        assert!(
            close(region::area(&r), PI * 100.0, disc_slack(10.0)),
            "{r:?}"
        );
        let mut s = BrepSection::new(&k, &mut cylinder, CHORD).unwrap();
        assert!(region::area(&s.section(Axis::Z, 30.0).unwrap()) < 1e-6);
    }

    #[test]
    fn whole_numbers_of_sheets_stack_on_layer_middles() {
        use super::super::slice::{make, positions};
        let mut k = Kernel::default();
        // planes sit at the middles of the layers: 1.5, 4.5 .. 28.5 for 30 mm at 3
        let at = positions(0.0, 30.0, &stacked(Axis::Z, 3.0), true).unwrap();
        assert_eq!(at.len(), 10, "{at:?}");
        let mut want = 1.5;
        for z in &at {
            assert!(close(*z, want, 1e-9), "{at:?}");
            want += 3.0;
        }
        let mut bx = create_box(&mut k, Point3::ZERO, 40.0, 30.0, 30.0).unwrap();
        let mut s = BrepSection::new(&k, &mut bx, CHORD).unwrap();
        let res = make(&mut s, &stacked(Axis::Z, 3.0)).unwrap();
        assert_eq!(res.parts.len(), 10);
        assert!(res
            .parts
            .iter()
            .all(|p| close(region::shape_area(&p.shape), 1200.0, 1e-6)));
        let mut sphere = create_sphere(&mut k, Point3::new(0.0, 0.0, 15.0), 15.0).unwrap();
        let mut s = BrepSection::new(&k, &mut sphere, CHORD).unwrap();
        let res = make(&mut s, &stacked(Axis::Z, 3.0)).unwrap();
        assert_eq!(res.parts.len(), 10, "{:?}", res.warnings);
        // the layer at height h is the disc of radius sqrt(15^2 - (h - 15)^2)
        for (p, z) in res.parts.iter().zip(&at) {
            let want = PI * (225.0 - (z - 15.0).powi(2));
            let a = region::shape_area(&p.shape);
            assert!(close(a, want, disc_slack(15.0)), "z = {z}: {a} want {want}");
        }
    }

    #[test]
    #[ignore = "kernel: a plane tangent to a torus along its top circle returns that circle \
                as one closed section curve, and a plane up to 1e-6 mm under it one circle, not \
                two; either encloses the full disc"]
    fn a_plane_on_the_top_circle_of_a_torus_gives_nothing() {
        // on the top circle: nothing; just under it, the two circles of radius
        // 40 +- sqrt(2 * 15 * e) (3.5e-3 mm apart at e = 1e-7): a ring under 1 mm^2 or nothing,
        // never the disc of the outer circle (about 5026 mm^2)
        let mut k = Kernel::default();
        let mut b = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        for z in [35.0, 5.0, 35.0 - 1e-7, 35.0 - 1e-9] {
            let r = section_at(&k, &mut b, Axis::Z, z, CHORD, [0.0; 3]).unwrap();
            assert!(region::area(&r) < 10.0, "z = {z}: {}", region::area(&r));
            assert!(!region::contains(&r, [0.0, 0.0]), "z = {z}");
        }
    }

    #[test]
    #[ignore = "kernel: a plane tangent to a cylinder along a side line returns that line as \
                an open section curve, so the section is refused as open"]
    fn a_plane_along_the_side_of_a_cylinder_gives_nothing() {
        let mut k = Kernel::default();
        let mut b = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
        let r = section_at(&k, &mut b, Axis::Y, 10.0, CHORD, [0.0; 3]).unwrap();
        assert!(region::area(&r) < 1e-6, "{r:?}");
    }

    #[test]
    fn the_kernel_gives_a_tangent_side_line_once() {
        // what the kernel returns today for the two ignored cases above: the contact curve
        // itself, once (when this fails, the kernel changed: try the ignored tests)
        let mut k = Kernel::default();
        let mut b = create_cylinder(&mut k, Point3::ZERO, 10.0, 30.0).unwrap();
        let (curves, _) = section_curves(&k, &mut b, Axis::Y, 10.0, CHORD, [0.0; 3]).unwrap();
        assert_eq!(curves.len(), 1);
        match section_at(&k, &mut b, Axis::Y, 10.0, CHORD, [0.0; 3]) {
            Err(FlatError::OpenSection { gap, .. }) => assert!(close(gap, 30.0, 1e-6), "{gap}"),
            other => panic!("{other:?}"),
        }
        let mut t = create_torus(&mut k, Point3::new(0.0, 0.0, 20.0), 40.0, 15.0).unwrap();
        // on the top circle and just under it (where the exact section is two circles): one
        // curve of one face
        for z in [35.0, 35.0 - 1e-7, 35.0 - 1e-9] {
            let (curves, faces) = section_curves(&k, &mut t, Axis::Z, z, CHORD, [0.0; 3]).unwrap();
            assert_eq!(curves.len(), 1, "z = {z}");
            assert_eq!(faces.len(), 1, "z = {z}: {faces:?}");
        }
    }
}
