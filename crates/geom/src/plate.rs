//! Flat plates for display: a part cut from sheet stock (its outline and holes as polygons in
//! its own plane) shown as a slab `thickness` thick, centred on that plane. The faces reuse the
//! profile triangulation ([`Region2Geom::triangulate`]); the walls are one quad per side. A
//! display mesh only: nothing is decided from it.

use solidcore_record::{Loop2, Region2, Seg2, Vec2, Vec3};

use crate::Mesh;
use crate::prelude::*;

/// Where a plate sits: its point `(x, y)` is at `origin + x u + y v`, and the slab reaches
/// `thickness / 2` to each side along `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateFrame {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub n: Vec3,
}

/// Most corners a plate takes; past it nothing is drawn (the triangulation bounds itself too).
const MAX_CORNERS: usize = 20_000;

/// The plate a part makes: `rings` are its outline then its holes (`[x, y]` corners, either
/// way round), `frame` places it and `thickness` is the sheet's. Triangles face outwards, each
/// with its own corners and flat normal; the outline and holes are drawn as edges on both faces.
/// Empty for a ring with fewer than three corners, a non-finite value or a thickness not above 0.
pub fn plate_mesh(rings: &[Vec<[f64; 2]>], frame: &PlateFrame, thickness: f64) -> Mesh {
    let mut mesh = Mesh::default();
    let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
    let Some(n) = frame.n.normalized() else { return mesh };
    if !(thickness.is_finite() && thickness > 0.0) || !finite(frame.origin) || !finite(frame.u) || !finite(frame.v) {
        return mesh;
    }
    let Some(loops) = polygons(rings) else { return mesh };
    let region = region_of(&loops);
    let half = n * (thickness * 0.5);
    let at = |p: Vec2| frame.origin + frame.u * p.x + frame.v * p.y;
    for [a, b, c] in region.triangulate(1e-3) {
        let (a, b, c) = (at(a), at(b), at(c));
        push_facing(&mut mesh, [a + half, b + half, c + half], n);
        push_facing(&mut mesh, [a - half, b - half, c - half], -n);
    }
    for pts in &loops {
        for k in 0..pts.len() {
            let (p, q) = (pts[k], pts[(k + 1) % pts.len()]);
            let d = q - p;
            // The material is on the left, so outwards is to the right of p → q.
            let out = frame.u * d.y - frame.v * d.x;
            let (a, b) = (at(p), at(q));
            push_facing(&mut mesh, [a - half, b - half, b + half], out);
            push_facing(&mut mesh, [a - half, b + half, a + half], out);
        }
        let ring: Vec<Vec3> = pts.iter().chain(pts.first()).map(|p| at(*p)).collect();
        mesh.edges.push(ring.iter().map(|p| *p + half).collect());
        mesh.edges.push(ring.iter().map(|p| *p - half).collect());
    }
    mesh.display_only_edges = true;
    mesh.tri_face = vec![0; mesh.triangles.len()];
    mesh
}

/// The rings as polygons, the first counter-clockwise and the rest clockwise (so the material of
/// a part is on the left of every side), repeated corners dropped. `None` for no ring, a ring
/// with fewer than three corners, a non-finite corner or more than [`MAX_CORNERS`] in all.
fn polygons(rings: &[Vec<[f64; 2]>]) -> Option<Vec<Vec<Vec2>>> {
    if rings.is_empty() || rings.iter().map(Vec::len).sum::<usize>() > MAX_CORNERS {
        return None;
    }
    let mut loops: Vec<Vec<Vec2>> = Vec::with_capacity(rings.len());
    for (i, ring) in rings.iter().enumerate() {
        let mut pts: Vec<Vec2> = Vec::with_capacity(ring.len());
        for &[x, y] in ring {
            if !(x.is_finite() && y.is_finite()) {
                return None;
            }
            let p = Vec2::new(x, y);
            if pts.last().is_none_or(|q: &Vec2| q.dist(p) > crate::EPS) {
                pts.push(p);
            }
        }
        if pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.dist(*b) <= crate::EPS) {
            pts.pop();
        }
        if pts.len() < 3 {
            return None;
        }
        let area2: f64 = (0..pts.len()).map(|k| pts[k].cross(pts[(k + 1) % pts.len()])).sum();
        if (i == 0) != (area2 > 0.0) {
            pts.reverse();
        }
        loops.push(pts);
    }
    Some(loops)
}

/// The polygons as a region of straight sides: the first the outline, the rest its holes.
fn region_of(loops: &[Vec<Vec2>]) -> Region2 {
    let to_loop = |pts: &[Vec2]| Loop2 { segs: (0..pts.len()).map(|k| Seg2::Line { a: pts[k], b: pts[(k + 1) % pts.len()] }).collect() };
    let mut all = loops.iter().map(|l| to_loop(l));
    Region2 { outer: all.next().unwrap_or_default(), holes: all.collect() }
}

/// Triangles filling closed polygons `[x, y]` by the even-odd rule: a part's outline and holes,
/// or every ring cut on a sheet (a part inside another's hole fills again). For drawing; empty
/// when the rings are not polygons ([`plate_mesh`] says which).
pub fn fill_triangles(rings: &[Vec<[f64; 2]>]) -> Vec<[Vec2; 3]> {
    polygons(rings).map(|loops| region_of(&loops).triangulate(1e-3)).unwrap_or_default()
}

/// One triangle with its own corners, wound so its normal points along `out`.
fn push_facing(mesh: &mut Mesh, [a, b, c]: [Vec3; 3], out: Vec3) {
    let normal = (b - a).cross(c - a);
    let Some(unit) = normal.normalized() else { return };
    let (b, c, unit) = if normal.dot(out) < 0.0 { (c, b, -unit) } else { (b, c, unit) };
    let Ok(first) = u32::try_from(mesh.positions.len()) else { return };
    mesh.positions.extend([a, b, c]);
    mesh.normals.extend([unit; 3]);
    mesh.triangles.push([first, first + 1, first + 2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, side: f64) -> Vec<[f64; 2]> {
        vec![[x0, y0], [x0 + side, y0], [x0 + side, y0 + side], [x0, y0 + side]]
    }

    fn frame() -> PlateFrame {
        PlateFrame { origin: Vec3::new(0.0, 0.0, 0.0), u: Vec3::new(1.0, 0.0, 0.0), v: Vec3::new(0.0, 1.0, 0.0), n: Vec3::new(0.0, 0.0, 1.0) }
    }

    #[test]
    fn a_plate_holds_its_area_times_the_thickness() {
        // 40 x 30 less a 10 mm square hole, 3 mm thick: (1200 - 100) * 3.
        let rings = vec![vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]], square(10.0, 10.0, 10.0)];
        let mesh = plate_mesh(&rings, &frame(), 3.0);
        let m = mesh.measure();
        assert!((m.volume - 3300.0).abs() < 1e-6, "{}", m.volume);
        // Faces 2 * 1100, outer walls 140 * 3, hole walls 40 * 3.
        assert!((m.area - (2200.0 + 420.0 + 120.0)).abs() < 1e-6, "{}", m.area);
        assert_eq!(mesh.edges.len(), 4);
        assert_eq!(mesh.tri_face.len(), mesh.triangles.len());
        assert!(mesh.mass_properties().is_none(), "unwelded corners: a display mesh");
    }

    #[test]
    fn a_plate_is_centred_on_its_plane_whatever_the_frame_and_ring_direction() {
        // A frame turned about X, moved off the origin, its normal against u x v, and the
        // outline given clockwise and the hole counter-clockwise.
        let f =
            PlateFrame { origin: Vec3::new(5.0, -7.0, 12.0), u: Vec3::new(1.0, 0.0, 0.0), v: Vec3::new(0.0, 0.0, 1.0), n: Vec3::new(0.0, 1.0, 0.0) };
        let mut outer = square(0.0, 0.0, 20.0);
        outer.reverse();
        let rings = vec![outer, square(5.0, 5.0, 4.0)];
        let mesh = plate_mesh(&rings, &f, 6.0);
        let m = mesh.measure();
        // Signed: a triangle facing inwards would take its share off instead of adding it.
        assert!((m.volume - (400.0 - 16.0) * 6.0).abs() < 1e-6, "{}", m.volume);
        let b = mesh.bounds();
        assert!((b.min.y + 10.0).abs() < 1e-9 && (b.max.y + 4.0).abs() < 1e-9, "{b:?}");
        assert!((b.min.x - 5.0).abs() < 1e-9 && (b.max.z - 32.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn a_sheet_fills_by_the_even_odd_rule() {
        // A 40 mm square with a 20 mm hole, and a 10 mm part packed inside that hole.
        let rings = vec![square(0.0, 0.0, 40.0), square(10.0, 10.0, 20.0), square(15.0, 15.0, 10.0)];
        let area: f64 = fill_triangles(&rings).iter().map(|[a, b, c]| ((*b - *a).cross(*c - *a) * 0.5).abs()).sum();
        assert!((area - (1600.0 - 400.0 + 100.0)).abs() < 1e-6, "{area}");
        assert!(fill_triangles(&[vec![[0.0, 0.0], [1.0, 1.0]]]).is_empty());
    }

    #[test]
    fn hostile_rings_make_no_plate() {
        let f = frame();
        assert!(plate_mesh(&[], &f, 3.0).triangles.is_empty());
        assert!(plate_mesh(&[vec![[0.0, 0.0], [1.0, 0.0]]], &f, 3.0).triangles.is_empty());
        assert!(plate_mesh(&[square(0.0, 0.0, 1.0)], &f, 0.0).triangles.is_empty());
        assert!(plate_mesh(&[square(0.0, 0.0, 1.0)], &f, f64::NAN).triangles.is_empty());
        assert!(plate_mesh(&[vec![[0.0, 0.0], [f64::INFINITY, 0.0], [1.0, 1.0]]], &f, 1.0).triangles.is_empty());
        let flat = PlateFrame { n: Vec3::new(0.0, 0.0, 0.0), ..f };
        assert!(plate_mesh(&[square(0.0, 0.0, 1.0)], &flat, 1.0).triangles.is_empty());
    }
}
