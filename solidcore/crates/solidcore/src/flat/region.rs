//! Polygonal regions: booleans, offsets, clipping of polylines, areas and point tests.
//!
//! Every result is normalised: disjoint [`Shape`]s, each an outline counter-clockwise with its
//! holes clockwise, no ring with fewer than three points. Inputs may be in either direction and
//! may overlap themselves; rings with a point that is not finite, or fewer than three points,
//! are left out. The booleans and offsets run on iOverlay's integer engine, scaled to the
//! operands' bounds (a 1 m part resolves to about a nanometre).
//!
//! Deviations: none (no upstream counterpart). No recursion.

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::clip::FloatClip;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin, OutlineStyle};
use i_overlay::string::clip::ClipRule;

use super::{Region, Ring, Shape, P2};

/// How an offset turns a corner it moves away from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    /// Sharp: the offset edges meet where they cross. Corners sharper than
    /// [`MITER_MIN_ANGLE`] are cut square instead of spiking out.
    Miter,
    /// Round: an arc of the offset's radius, within `chord` (mm) of the true arc.
    Round {
        /// The largest distance between the arc and its chords (mm).
        chord: f64,
    },
}

/// The sharpest corner (radians) a mitred offset keeps sharp: about 29°, where a miter reaches
/// four times the offset from the corner.
pub const MITER_MIN_ANGLE: f64 = 0.505;

/// The chord tolerance (mm) of arcs this module makes when the caller gives none.
pub const DEFAULT_CHORD: f64 = 0.005;

/// Finite and at most a kilometre from the origin.
fn sane(p: P2) -> bool {
    p.iter().all(|c| c.is_finite() && c.abs() < 1e6)
}

fn ring_ok(r: &Ring) -> bool {
    r.len() >= 3 && r.iter().all(|&p| sane(p))
}

/// The region as iOverlay's shapes, bad rings left out.
fn shapes_of(r: &[Shape]) -> Vec<Vec<Ring>> {
    r.iter()
        .filter(|s| ring_ok(&s.outer))
        .map(|s| s.rings().filter(|x| ring_ok(x)).cloned().collect())
        .collect()
}

fn rings_of(rings: &[Ring]) -> Vec<Ring> {
    rings.iter().filter(|r| ring_ok(r)).cloned().collect()
}

/// iOverlay's shapes (outline counter-clockwise, holes clockwise) as a region.
fn region_of(shapes: Vec<Vec<Ring>>) -> Region {
    shapes
        .into_iter()
        .filter_map(|s| {
            let mut it = s.into_iter().filter(|r| r.len() >= 3);
            let outer = it.next()?;
            Some(Shape {
                outer,
                holes: it.collect(),
            })
        })
        .collect()
}

/// The material of loose rings: outlines and holes in any direction, by `even_odd` filling
/// (a ring inside another is a hole) or non-zero (direction counts: clockwise inside
/// counter-clockwise is a hole, two counter-clockwise rings unite).
#[must_use]
pub fn from_rings(rings: &[Ring], even_odd: bool) -> Region {
    let rs = rings_of(rings);
    if rs.is_empty() {
        return Vec::new();
    }
    let fill = if even_odd {
        FillRule::EvenOdd
    } else {
        FillRule::NonZero
    };
    let none: Vec<Ring> = Vec::new();
    region_of(rs.overlay(&none, OverlayRule::Subject, fill))
}

fn boolean(a: &[Shape], b: &[Shape], rule: OverlayRule) -> Region {
    let (sa, sb) = (shapes_of(a), shapes_of(b));
    match (sa.is_empty(), sb.is_empty(), rule) {
        (true, true, _)
        | (true, _, OverlayRule::Intersect | OverlayRule::Difference)
        | (_, true, OverlayRule::Intersect) => return Vec::new(),
        _ => {}
    }
    region_of(sa.overlay(&sb, rule, FillRule::NonZero))
}

/// `a` and `b` united.
#[must_use]
pub fn union(a: &[Shape], b: &[Shape]) -> Region {
    boolean(a, b, OverlayRule::Union)
}

/// `a` less `b`.
#[must_use]
pub fn difference(a: &[Shape], b: &[Shape]) -> Region {
    boolean(a, b, OverlayRule::Difference)
}

/// Where `a` and `b` overlap.
#[must_use]
pub fn intersection(a: &[Shape], b: &[Shape]) -> Region {
    boolean(a, b, OverlayRule::Intersect)
}

/// All the regions united.
#[must_use]
pub fn union_all(regions: &[Region]) -> Region {
    let all: Vec<Shape> = regions.iter().flatten().cloned().collect();
    normalize(&all)
}

/// The region normalised: overlapping shapes united (non-zero filling).
#[must_use]
pub fn normalize(r: &[Shape]) -> Region {
    let none: Vec<Shape> = Vec::new();
    boolean(r, &none, OverlayRule::Union)
}

/// The region grown by `delta` mm (shrunk for a negative `delta`): outlines move out, holes
/// shrink. Mitred corners stay sharp, so a square stays a square of side `+ 2 delta`; round
/// corners follow arcs of radius `|delta|`. A region shrunk to nothing is empty.
#[must_use]
pub fn offset(r: &[Shape], delta: f64, join: Join) -> Region {
    let shapes = shapes_of(r);
    if shapes.is_empty() || !delta.is_finite() {
        return Vec::new();
    }
    if delta == 0.0 {
        return normalize(r);
    }
    let lj = match join {
        Join::Miter => LineJoin::Miter(MITER_MIN_ANGLE),
        Join::Round { chord } => {
            // iOverlay's arc step is the largest segment length over the radius: a chord within
            // `chord` of a radius-`|delta|` arc spans at most 2 sqrt(2 chord / |delta|) radians
            let c = if chord.is_finite() && chord > 0.0 {
                chord
            } else {
                DEFAULT_CHORD
            };
            LineJoin::Round((8.0 * c / delta.abs()).sqrt().clamp(0.005, 1.0))
        }
    };
    let style = OutlineStyle::new(delta).line_join(lj);
    region_of(shapes.outline(&style))
}

/// The signed area of a ring (mm², positive counter-clockwise).
#[must_use]
pub fn ring_area(ring: &[P2]) -> f64 {
    let a = ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .fold(0.0, |a, (p, q)| a + (p[0] * q[1] - q[0] * p[1]));
    a / 2.0
}

/// The area of a shape's material (mm²): its outline less its holes.
#[must_use]
pub fn shape_area(s: &Shape) -> f64 {
    ring_area(&s.outer).abs() - s.holes.iter().map(|h| ring_area(h).abs()).sum::<f64>()
}

/// The area of a normalised region's material (mm²).
#[must_use]
pub fn area(r: &[Shape]) -> f64 {
    r.iter().map(shape_area).sum()
}

/// The bounds `[xmin, ymin, xmax, ymax]` of the rings' points, `None` without points.
#[must_use]
pub fn ring_bounds<'a>(rings: impl IntoIterator<Item = &'a Ring>) -> Option<[f64; 4]> {
    let mut b: Option<[f64; 4]> = None;
    for p in rings.into_iter().flatten() {
        b = Some(match b {
            None => [p[0], p[1], p[0], p[1]],
            Some([x0, y0, x1, y1]) => [x0.min(p[0]), y0.min(p[1]), x1.max(p[0]), y1.max(p[1])],
        });
    }
    b
}

/// The width and height `[w, h]` of a shape's outline in its own frame (mm), `None` without
/// points.
#[must_use]
pub fn shape_size(s: &Shape) -> Option<[f64; 2]> {
    ring_bounds([&s.outer]).map(|[x0, y0, x1, y1]| [x1 - x0, y1 - y0])
}

/// The bounds `[xmin, ymin, xmax, ymax]` of a region's outlines, `None` when it is empty.
#[must_use]
pub fn bounds(r: &[Shape]) -> Option<[f64; 4]> {
    ring_bounds(r.iter().map(|s| &s.outer))
}

/// The winding number of `ring` around `p` (counter-clockwise positive); a point on the ring
/// may count either way.
fn winding(ring: &[P2], p: P2) -> i32 {
    let mut w = 0;
    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
        if a[1] <= p[1] {
            if b[1] > p[1] && cross > 0.0 {
                w += 1;
            }
        } else if b[1] <= p[1] && cross < 0.0 {
            w -= 1;
        }
    }
    w
}

/// Whether `p` lies in the region's material (non-zero rule; a point on a boundary may go
/// either way).
#[must_use]
pub fn contains(r: &[Shape], p: P2) -> bool {
    r.iter()
        .map(|s| s.rings().map(|x| winding(x, p)).sum::<i32>())
        .sum::<i32>()
        != 0
}

/// The rectangle between two corners, as a region (empty when it has no area).
#[must_use]
pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
    let (a, b) = (x0.min(x1), x0.max(x1));
    let (c, d) = (y0.min(y1), y0.max(y1));
    let ok = b > a && d > c && sane([a, c]) && sane([b, d]);
    if !ok {
        return Vec::new();
    }
    vec![Shape {
        outer: vec![[a, c], [b, c], [b, d], [a, d]],
        holes: Vec::new(),
    }]
}

/// The number of sides of a polygon inscribed in a circle of radius `r` whose sides stay within
/// `chord` of the circle: at least 24, at most 1440.
#[must_use]
pub fn circle_sides(r: f64, chord: f64) -> usize {
    let c = if chord.is_finite() && chord > 0.0 {
        chord
    } else {
        DEFAULT_CHORD
    };
    if !(r.is_finite() && r > c) {
        return 24;
    }
    let half = (1.0 - c / r).clamp(-1.0, 1.0).acos();
    let n = (std::f64::consts::PI / half).ceil();
    if n.is_finite() {
        (n as usize).clamp(24, 1440)
    } else {
        1440
    }
}

/// A disc of radius `r` about `c`: a polygon inscribed in the circle, its sides within `chord`
/// of it, with a vertex at angle 0. Empty for a radius that is not positive.
#[must_use]
pub fn disk(c: P2, r: f64, chord: f64) -> Region {
    let ok = r.is_finite() && r > 0.0 && sane(c);
    if !ok {
        return Vec::new();
    }
    let n = circle_sides(r, chord);
    let outer = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect();
    vec![Shape {
        outer,
        holes: Vec::new(),
    }]
}

/// The pieces of open polylines that lie inside the region (its boundary excluded).
#[must_use]
pub fn clip_lines(lines: &[Ring], r: &[Shape]) -> Vec<Ring> {
    let shapes = shapes_of(r);
    if shapes.is_empty() {
        return Vec::new();
    }
    let rule = ClipRule {
        invert: false,
        boundary_included: false,
    };
    lines
        .iter()
        .filter(|l| l.len() >= 2 && l.iter().all(|&p| sane(p)))
        .flat_map(|l| l.clip_by(&shapes, FillRule::NonZero, rule))
        .filter(|l| l.len() >= 2)
        .collect()
}

/// The region's separate pieces with at least `min_area` mm² of material, largest first (ties
/// keep their order).
#[must_use]
pub fn pieces(r: &[Shape], min_area: f64) -> Region {
    let mut out: Vec<(f64, Shape)> = normalize(r)
        .into_iter()
        .map(|s| (shape_area(&s), s))
        .filter(|(a, _)| *a >= min_area)
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.into_iter().map(|(_, s)| s).collect()
}

/// The stretches `(lo, hi)` of `y`, in order, where the vertical line at `x` runs through the
/// region's material. Touching stretches merge.
#[must_use]
pub fn spans_at_x(r: &[Shape], x: f64) -> Vec<(f64, f64)> {
    // the crossings of every edge with the line, with the edge's direction: material is where
    // the running winding is not zero (half-open in x, so a vertex on the line counts once)
    let mut hits: Vec<(f64, i32)> = Vec::new();
    for ring in r.iter().flat_map(Shape::rings) {
        for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
            if (a[0] <= x) != (b[0] <= x) {
                let t = (x - a[0]) / (b[0] - a[0]);
                let y = a[1] + t * (b[1] - a[1]);
                hits.push((y, if b[0] > a[0] { 1 } else { -1 }));
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut w = 0;
    let mut start = 0.0;
    for (y, d) in hits {
        let was = w;
        w += d;
        if was == 0 && w != 0 {
            start = y;
        } else if was != 0 && w == 0 {
            match out.last_mut() {
                Some(last) if start <= last.1 + 1e-9 => last.1 = last.1.max(y),
                _ => out.push((start, y)),
            }
        }
    }
    out
}

/// The region mapped point by point through `f`. `mirrored` says that `f` turns the plane over
/// (a reflection), so every ring is reversed to keep outlines counter-clockwise.
#[must_use]
pub fn map(r: &[Shape], f: impl Fn(P2) -> P2, mirrored: bool) -> Region {
    let m = |ring: &Ring| -> Ring {
        let mut out: Ring = ring.iter().map(|&p| f(p)).collect();
        if mirrored {
            out.reverse();
        }
        out
    };
    r.iter()
        .map(|s| Shape {
            outer: m(&s.outer),
            holes: s.holes.iter().map(m).collect(),
        })
        .collect()
}

/// The number of points in the region's rings.
#[must_use]
pub fn point_count(r: &[Shape]) -> usize {
    r.iter().flat_map(Shape::rings).map(Vec::len).sum()
}

/// A closed ring with points dropped while it stays within `tol` of the original
/// (Ramer–Douglas–Peucker on each half, split at the point farthest from the first).
#[must_use]
pub fn simplify_ring(ring: &[P2], tol: f64) -> Ring {
    let n = ring.len();
    if n < 8 || !(tol.is_finite() && tol > 0.0) {
        return ring.to_vec();
    }
    let Some(&first) = ring.first() else {
        return ring.to_vec();
    };
    let d2 = |p: P2| (p[0] - first[0]).powi(2) + (p[1] - first[1]).powi(2);
    let far = (0..n)
        .max_by(|&i, &j| {
            let a = ring.get(i).map_or(0.0, |&p| d2(p));
            let b = ring.get(j).map_or(0.0, |&p| d2(p));
            a.total_cmp(&b)
        })
        .unwrap_or(0);
    if far == 0 {
        return ring.to_vec();
    }
    let a: Ring = ring.iter().take(far + 1).copied().collect();
    let b: Ring = ring
        .iter()
        .skip(far)
        .chain(std::iter::once(&first))
        .copied()
        .collect();
    let mut out = rdp(&a, tol);
    out.pop();
    let mut tail = rdp(&b, tol);
    tail.pop();
    out.extend(tail);
    if out.len() < 3 {
        return ring.to_vec();
    }
    out
}

/// Ramer–Douglas–Peucker on an open polyline: its ends kept.
fn rdp(pts: &[P2], tol: f64) -> Ring {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    let mut stack = vec![(0usize, n - 1)];
    while let Some((i, j)) = stack.pop() {
        let (Some(&a), Some(&b)) = (pts.get(i), pts.get(j)) else {
            continue;
        };
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy).max(1e-12);
        let mut best = (-1.0, 0usize);
        for k in i + 1..j {
            if let Some(&p) = pts.get(k) {
                let d = (dy * (p[0] - a[0]) - dx * (p[1] - a[1])).abs() / len;
                if d > best.0 {
                    best = (d, k);
                }
            }
        }
        if best.0 > tol {
            if let Some(k) = keep.get_mut(best.1) {
                *k = true;
            }
            stack.push((i, best.1));
            stack.push((best.1, j));
        }
    }
    pts.iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, s: f64) -> Region {
        rect(x, y, x + s, y + s)
    }

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn a_shape_measures_its_outline_not_its_holes() {
        let s = Shape {
            outer: vec![[-3.0, 1.0], [7.0, 1.0], [7.0, 5.0], [-3.0, 5.0]],
            holes: vec![vec![[0.0, 2.0], [0.0, 30.0], [1.0, 2.0]]],
        };
        assert_eq!(shape_size(&s), Some([10.0, 4.0]));
        assert_eq!(shape_size(&Shape::default()), None);
    }

    #[test]
    fn booleans_of_two_squares() {
        let a = square(0.0, 0.0, 4.0);
        let b = square(2.0, 2.0, 4.0);
        assert!(close(area(&union(&a, &b)), 28.0, 1e-9));
        assert!(close(area(&intersection(&a, &b)), 4.0, 1e-9));
        assert!(close(area(&difference(&a, &b)), 12.0, 1e-9));
        assert!(difference(&[], &b).is_empty());
        assert!(intersection(&a, &[]).is_empty());
        assert!(close(area(&union(&[], &b)), 16.0, 1e-9));
        assert!(close(area(&difference(&a, &[])), 16.0, 1e-9));
    }

    #[test]
    fn a_hole_comes_out_clockwise_inside_a_counter_clockwise_outline() {
        let r = difference(&square(0.0, 0.0, 10.0), &square(3.0, 3.0, 4.0));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].holes.len(), 1);
        assert!(ring_area(&r[0].outer) > 0.0);
        assert!(ring_area(&r[0].holes[0]) < 0.0);
        assert!(close(area(&r), 84.0, 1e-9));
        assert!(contains(&r, [1.0, 1.0]));
        assert!(!contains(&r, [5.0, 5.0]));
        assert!(!contains(&r, [11.0, 5.0]));
    }

    #[test]
    fn loose_rings_by_even_odd_and_non_zero() {
        let outer: Ring = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let inner: Ring = vec![[2.0, 2.0], [8.0, 2.0], [8.0, 8.0], [2.0, 8.0]]; // also CCW
        let eo = from_rings(&[outer.clone(), inner.clone()], true);
        assert!(close(area(&eo), 64.0, 1e-9));
        let nz = from_rings(&[outer, inner], false);
        assert!(close(area(&nz), 100.0, 1e-9));
        assert!(from_rings(&[vec![[0.0, 0.0], [1.0, f64::NAN], [1.0, 1.0]]], true).is_empty());
    }

    #[test]
    fn a_mitred_offset_keeps_a_square_square() {
        let grown = offset(&square(0.0, 0.0, 50.0), 0.1, Join::Miter);
        assert!(close(area(&grown), 50.2 * 50.2, 1e-3), "{}", area(&grown));
        assert_eq!(grown[0].outer.len(), 4);
        let shrunk = offset(&square(0.0, 0.0, 50.0), -0.1, Join::Miter);
        assert!(close(area(&shrunk), 49.8 * 49.8, 1e-3));
        assert!(offset(&square(0.0, 0.0, 1.0), -0.6, Join::Miter).is_empty());
    }

    #[test]
    fn a_hole_shrinks_when_the_material_grows() {
        let r = difference(&square(0.0, 0.0, 50.0), &square(20.0, 20.0, 10.0));
        let g = offset(&r, 0.1, Join::Miter);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].holes.len(), 1);
        assert!(close(-ring_area(&g[0].holes[0]), 9.8 * 9.8, 1e-3));
        assert!(close(area(&g), 50.2 * 50.2 - 9.8 * 9.8, 2e-3));
    }

    #[test]
    fn a_round_offset_adds_the_perimeter_band_and_a_circle() {
        // a square grown with round corners: s² + 4 s d + π d²
        let (s, d) = (20.0, 2.0);
        let g = offset(&square(0.0, 0.0, s), d, Join::Round { chord: 0.001 });
        let want = s * s + 4.0 * s * d + std::f64::consts::PI * d * d;
        assert!(close(area(&g), want, 0.02), "{} vs {want}", area(&g));
    }

    #[test]
    fn a_disk_is_within_its_chord_of_the_circle() {
        let r = 10.0;
        let d = disk([1.0, 2.0], r, 0.01);
        let n = d[0].outer.len();
        assert_eq!(n, circle_sides(r, 0.01));
        // inscribed polygon: n/2 r² sin(2π/n)
        let want = n as f64 / 2.0 * r * r * (std::f64::consts::TAU / n as f64).sin();
        assert!(close(area(&d), want, 1e-9));
        assert!(std::f64::consts::PI * r * r - area(&d) < std::f64::consts::TAU * r * 0.01);
        assert!(disk([0.0, 0.0], 0.0, 0.01).is_empty());
        assert!(disk([0.0, 0.0], f64::NAN, 0.01).is_empty());
    }

    #[test]
    fn spans_along_a_vertical_line() {
        let r = difference(&square(0.0, 0.0, 10.0), &square(3.0, 3.0, 4.0));
        let s = spans_at_x(&r, 5.0);
        assert_eq!(s.len(), 2);
        assert!(close(s[0].0, 0.0, 1e-12) && close(s[0].1, 3.0, 1e-12));
        assert!(close(s[1].0, 7.0, 1e-12) && close(s[1].1, 10.0, 1e-12));
        assert_eq!(spans_at_x(&r, 1.0), vec![(0.0, 10.0)]);
        assert!(spans_at_x(&r, 12.0).is_empty());
    }

    #[test]
    fn lines_are_clipped_to_the_inside() {
        let r = square(0.0, 0.0, 2.0);
        let lines = clip_lines(&[vec![[-1.0, 1.0], [3.0, 1.0]]], &r);
        assert_eq!(lines.len(), 1);
        let xs: Vec<f64> = lines[0].iter().map(|p| p[0]).collect();
        assert!(xs.iter().all(|x| (-1e-9..=2.0 + 1e-9).contains(x)));
        assert!(clip_lines(&[vec![[5.0, 5.0], [6.0, 6.0]]], &r).is_empty());
    }

    #[test]
    fn pieces_are_largest_first_and_slivers_go() {
        let r = union(
            &union(&square(0.0, 0.0, 2.0), &square(10.0, 0.0, 5.0)),
            &square(30.0, 0.0, 0.1),
        );
        let p = pieces(&r, 0.5);
        assert_eq!(p.len(), 2);
        assert!(close(shape_area(&p[0]), 25.0, 1e-9));
        assert!(close(shape_area(&p[1]), 4.0, 1e-9));
    }

    #[test]
    fn a_mirror_keeps_outlines_counter_clockwise() {
        let r = square(0.0, 0.0, 2.0);
        let m = map(&r, |p| [-p[0], p[1]], true);
        assert!(ring_area(&m[0].outer) > 0.0);
        assert_eq!(bounds(&m), Some([-2.0, 0.0, 0.0, 2.0]));
    }

    #[test]
    fn simplify_drops_collinear_points_only() {
        let mut ring: Ring = (0..=10).map(|i| [f64::from(i), 0.0]).collect();
        ring.extend([[10.0, 5.0], [0.0, 5.0]]);
        let s = simplify_ring(&ring, 0.01);
        assert_eq!(s.len(), 4);
        assert!(close(ring_area(&s), ring_area(&ring), 1e-12));
    }

    // --- found by mutation testing (cargo mutants on this file) ---

    #[test]
    fn a_rectangle_needs_width_and_height() {
        assert!(rect(0.0, 0.0, 0.0, 5.0).is_empty());
        assert!(rect(0.0, 0.0, 5.0, 0.0).is_empty());
        assert!(rect(0.0, 0.0, f64::NAN, 5.0).is_empty());
        assert!(close(area(&rect(5.0, 5.0, 0.0, 0.0)), 25.0, 1e-12));
    }

    #[test]
    fn circle_sides_follow_the_chord() {
        // pi / acos(1 - 0.01 / 10) = 70.24...: 71 sides
        assert_eq!(circle_sides(10.0, 0.01), 71);
        assert_eq!(circle_sides(10.0, -1.0), circle_sides(10.0, DEFAULT_CHORD));
        assert_eq!(
            circle_sides(10.0, f64::NAN),
            circle_sides(10.0, DEFAULT_CHORD)
        );
        assert_eq!(circle_sides(f64::INFINITY, 0.01), 24);
        assert_eq!(circle_sides(0.001, 0.01), 24);
        assert_eq!(circle_sides(1e6, 1e-6), 1440);
    }

    #[test]
    fn round_offset_corners_stay_within_the_chord() {
        // the corner arcs of a square grown by 5 with round joins: every chord's midpoint lies
        // within 0.002 of the arc (radius 5 about the square's corner)
        let g = offset(&square(0.0, 0.0, 10.0), 5.0, Join::Round { chord: 0.001 });
        let pts = &g[0].outer;
        let n = pts.len();
        let mut on_arcs = 0;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            for c in [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]] {
                let da = ((a[0] - c[0]).powi(2) + (a[1] - c[1]).powi(2)).sqrt();
                let db = ((b[0] - c[0]).powi(2) + (b[1] - c[1]).powi(2)).sqrt();
                if (da - 5.0).abs() < 1e-3 && (db - 5.0).abs() < 1e-3 {
                    let m = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                    let dm = ((m[0] - c[0]).powi(2) + (m[1] - c[1]).powi(2)).sqrt();
                    assert!(5.0 - dm < 0.002, "{a:?} {b:?}: {dm}");
                    on_arcs += 1;
                }
            }
        }
        assert!(on_arcs >= 4 * 20, "{on_arcs}");
    }

    #[test]
    fn spans_along_a_slanted_edge_and_touching_pieces() {
        // a right triangle: at x = 4 the material runs from y = 0 to y = 6
        let tri = from_rings(&[vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]]], true);
        let s = spans_at_x(&tri, 4.0);
        assert_eq!(s.len(), 1);
        assert!(
            close(s[0].0, 0.0, 1e-12) && close(s[0].1, 6.0, 1e-12),
            "{s:?}"
        );
        // two pieces one above the other, touching: one stretch; apart: two
        let touching = vec![
            rect(0.0, 0.0, 4.0, 1.0)[0].clone(),
            rect(0.0, 1.0, 4.0, 3.0)[0].clone(),
        ];
        assert_eq!(spans_at_x(&touching, 2.0), vec![(0.0, 3.0)]);
        let apart = vec![
            rect(0.0, 0.0, 4.0, 1.0)[0].clone(),
            rect(0.0, 2.0, 4.0, 3.0)[0].clone(),
        ];
        assert_eq!(spans_at_x(&apart, 2.0), vec![(0.0, 1.0), (2.0, 3.0)]);
        // a vertex on the line counts once
        let diamond = from_rings(
            &[vec![[0.0, 5.0], [5.0, 0.0], [10.0, 5.0], [5.0, 10.0]]],
            true,
        );
        let s = spans_at_x(&diamond, 5.0);
        assert_eq!(s.len(), 1);
        assert!(
            close(s[0].0, 0.0, 1e-12) && close(s[0].1, 10.0, 1e-12),
            "{s:?}"
        );
    }

    #[test]
    fn points_are_counted_over_every_ring() {
        let r = difference(&square(0.0, 0.0, 10.0), &square(3.0, 3.0, 4.0));
        assert_eq!(point_count(&r), 8);
        assert_eq!(point_count(&[]), 0);
    }

    #[test]
    fn simplify_keeps_corners_and_short_rings() {
        // fewer than eight points: as given
        let short: Ring = vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 0.001],
            [2.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
            [0.0, 1.0],
        ];
        assert_eq!(simplify_ring(&short, 0.01), short);
        // a slanted edge with points within the tolerance of it goes to its ends; a point
        // further out than the tolerance stays
        let mut ring: Ring = (0..=10)
            .map(|i| [f64::from(i), f64::from(i) * 0.5])
            .collect();
        ring[5][1] += 0.004; // within 0.01 of the edge
        ring.extend([[10.0, 8.0], [0.0, 8.0]]);
        let s = simplify_ring(&ring, 0.01);
        assert_eq!(s.len(), 4, "{s:?}");
        ring[5][1] += 0.05; // now 0.054 off: kept, and with it its neighbours, each now more
                            // than the tolerance off the lines to the bump
        let s = simplify_ring(&ring, 0.01);
        assert_eq!(s.len(), 7, "{s:?}");
        assert!(s.contains(&ring[5]));
        // a bad tolerance leaves the ring
        assert_eq!(simplify_ring(&ring, 0.0), ring);
        assert_eq!(simplify_ring(&ring, f64::NAN), ring);
        // the result stays within the tolerance of every dropped point and keeps the area close
        let circle = disk([3.0, -2.0], 20.0, 0.0001)[0].outer.clone();
        let s = simplify_ring(&circle, 0.01);
        assert!(s.len() < circle.len() && s.len() >= 8);
        assert!(
            (ring_area(&s) - ring_area(&circle)).abs() < 2.0 * std::f64::consts::PI * 20.0 * 0.01
        );
    }

    #[test]
    fn an_offset_that_is_not_a_number_is_empty_and_a_bad_chord_takes_the_default() {
        let sq = square(0.0, 0.0, 10.0);
        assert!(offset(&sq, f64::NAN, Join::Miter).is_empty());
        assert!(offset(&sq, f64::INFINITY, Join::Round { chord: 0.01 }).is_empty());
        let default = offset(
            &sq,
            2.0,
            Join::Round {
                chord: DEFAULT_CHORD,
            },
        );
        for bad in [0.0, -1.0, f64::NAN] {
            assert_eq!(
                offset(&sq, 2.0, Join::Round { chord: bad }),
                default,
                "{bad}"
            );
        }
        assert_eq!(circle_sides(10.0, 0.0), circle_sides(10.0, DEFAULT_CHORD));
    }
}
