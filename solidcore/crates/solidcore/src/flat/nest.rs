//! Flat parts packed onto sheets for cutting.
//!
//! Placement is true-shape, not bounding-box: a small part may sit in the notch of an L or the
//! bay of a ring segment. Each sheet is an occupancy grid of about 120 000 cells (never finer
//! than 0.25 mm). A placed part, grown by the gap plus half a cell diagonal plus a small
//! tolerance, marks every cell whose centre it covers; a candidate pose is rasterised the same
//! way, grown by half a cell diagonal plus the tolerance. Any point of the candidate lies within
//! half a diagonal of its cell's centre, so that centre is marked in the candidate's raster, and
//! if the point were within `gap` of a placed part the same centre would be marked on the sheet:
//! "no shared cell" proves the outlines are more than `gap` apart, whatever the grid. The price
//! is a little extra spacing (up to about 1.5 cells) and positions snapped to the grid.
//!
//! The grown rasters are exact: a cell is marked when its centre lies in the outline (non-zero
//! winding) or within the growth distance of one of its edges, worked out per grid row in closed
//! form. No polygon offset is needed, and spikes or zero-width parts of an outline still count.
//!
//! Parts go largest first (ties by index); each tries its allowed turns on the first sheet where
//! it fits, at the grid spot giving the smallest used box of the sheet (lowest, then leftmost on
//! ties). Collision tests run on rows of 64-cell words: for one grid row of positions, every run
//! of the candidate's raster blocks the positions whose window over a sheet row holds a used
//! cell, so one row of positions costs a few word operations per raster row. The sheet margin is
//! applied exactly from the part's real bounding box.

use serde::{Deserialize, Serialize};

use super::{FlatError, FlatResult, Ring, P2};

/// Where one part goes: turned `angle` degrees counter-clockwise about its own origin first,
/// then moved by `(dx, dy)`; the sheet's origin is its lower-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    /// The part's index in the list given.
    pub part: usize,
    /// The sheet, from 0.
    pub sheet: usize,
    /// 0, 90, 180 or 270.
    pub angle: u16,
    /// Moved along x after turning (mm).
    pub dx: f64,
    /// Moved along y after turning (mm).
    pub dy: f64,
}

/// Grid cells per sheet: finer packs tighter, coarser runs faster.
const TARGET_CELLS: f64 = 120_000.0;
/// No point in a grid finer than the kerf (mm).
const MIN_CELL: f64 = 0.25;
/// At most this many cells along a sheet side, so a long thin sheet keeps a small grid.
const MAX_SIDE_CELLS: f64 = 4000.0;
/// Safety on top of every growth (mm): rounding of the grid arithmetic.
const TOL: f64 = 0.01;
/// Empty grid cells around the usable area (candidate rasters reach two cells out).
const PAD: usize = 3;
/// Slack (mm) when a part's box is compared with the usable area.
const FIT_EPS: f64 = 1e-9;
/// Sheet sizes, margins, gaps and part coordinates beyond this (mm) are refused.
const LIMIT: f64 = 1e5;
/// The most parts one call packs.
const MAX_PARTS: usize = 5000;
/// The most outline points over all parts.
const MAX_POINTS: usize = 200_000;
/// Rasters remembered per sheet as finding no room there.
const MAX_FAILED: usize = 32;

fn turn([x, y]: P2, angle: u16) -> P2 {
    match angle % 360 {
        90 => [-y, x],
        180 => [-x, -y],
        270 => [y, -x],
        _ => [x, y],
    }
}

/// One ring of a part as it lies on its sheet: turned, then moved. The turn is exact (a swap
/// and sign change of the coordinates); an angle other than 90, 180 or 270 (mod 360) leaves the
/// ring unturned.
#[must_use]
pub fn transform(ring: &[[f64; 2]], p: &Placement) -> Ring {
    ring.iter()
        .map(|&q| {
            let [x, y] = turn(q, p.angle);
            [x + p.dx, y + p.dy]
        })
        .collect()
}

/// A grid of cells as rows of 64-cell words; cell `c` of a row is bit `c % 64` of word `c / 64`.
#[derive(Clone, Debug, PartialEq)]
struct Bits {
    nx: usize,
    ny: usize,
    words: usize,
    data: Vec<u64>,
}

impl Bits {
    fn new(nx: usize, ny: usize) -> Self {
        let words = nx.div_ceil(64).max(1);
        Self {
            nx,
            ny,
            words,
            data: vec![0; words * ny],
        }
    }

    fn row(&self, r: usize) -> &[u64] {
        self.data
            .get(r * self.words..(r + 1) * self.words)
            .unwrap_or(&[])
    }

    fn get(&self, r: usize, c: usize) -> bool {
        self.row(r)
            .get(c / 64)
            .is_some_and(|w| w >> (c % 64) & 1 == 1)
    }

    /// Marks cells `lo..=hi` of row `r`.
    fn set_span(&mut self, r: usize, lo: usize, hi: usize) {
        let w = self.words;
        let Some(row) = self.data.get_mut(r * w..(r + 1) * w) else {
            return;
        };
        let (lw, hw) = (lo / 64, hi / 64);
        for (k, word) in row.iter_mut().enumerate().take(hw + 1).skip(lw) {
            let a = if k == lw { lo % 64 } else { 0 };
            let b = if k == hw { hi % 64 } else { 63 };
            *word |= (u64::MAX >> (63 - b)) & (u64::MAX << a);
        }
    }

    fn count(&self) -> usize {
        self.data.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// Whether every cell of `other` is a cell of `self` (both from their corner).
    fn covers(&self, other: &Bits) -> bool {
        other.nx <= self.nx
            && other.ny <= self.ny
            && (0..other.ny).all(|r| {
                other
                    .row(r)
                    .iter()
                    .zip(self.row(r))
                    .all(|(o, m)| o & !m == 0)
            })
    }

    /// The runs `(first cell, length)` of marked cells in each row.
    fn runs(&self) -> Vec<Vec<(usize, usize)>> {
        (0..self.ny)
            .map(|r| {
                let mut out = Vec::new();
                let mut start: Option<usize> = None;
                for c in 0..=self.nx {
                    let on = c < self.nx && self.get(r, c);
                    match (on, start) {
                        (true, None) => start = Some(c),
                        (false, Some(s)) => {
                            out.push((s, c - s));
                            start = None;
                        }
                        _ => {}
                    }
                }
                out
            })
            .collect()
    }
}

/// `v` rounded towards minus infinity, as an index between -1 and `n` (NaN gives -1).
fn floor_idx(v: f64, n: usize) -> i64 {
    if v.is_nan() {
        return -1;
    }
    v.floor().clamp(-1.0, n as f64) as i64
}

/// `v` rounded towards plus infinity, as an index between -1 and `n` (NaN gives `n`).
fn ceil_idx(v: f64, n: usize) -> i64 {
    if v.is_nan() {
        return n as i64;
    }
    v.ceil().clamp(-1.0, n as f64) as i64
}

/// The cells `lo..=hi` (of `0..n`) whose centres lie in `[a, b]`, for cell 0 starting at `o`.
fn centres_in(a: f64, b: f64, o: f64, cell: f64, n: usize) -> Option<(usize, usize)> {
    if n == 0 {
        return None;
    }
    let lo = ceil_idx((a - o) / cell - 0.5, n).max(0);
    let hi = floor_idx((b - o) / cell - 0.5, n).min(n as i64 - 1);
    (lo <= hi).then_some((lo as usize, hi as usize))
}

/// The x range, along the line at height `y`, of the points within `d` of segment `a`-`b`.
/// That set is convex; its ends lie on the circles about the end points or on the two lines
/// `d` either side of the segment, and every candidate taken lies in the set.
fn capsule_span(a: P2, b: P2, y: f64, d: f64) -> Option<(f64, f64)> {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut take = |x: f64| {
        lo = lo.min(x);
        hi = hi.max(x);
    };
    for e in [a, b] {
        let dy = y - e[1];
        if dy.abs() <= d {
            let s = (d * d - dy * dy).max(0.0).sqrt();
            take(e[0] - s);
            take(e[0] + s);
        }
    }
    let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
    let len = ex.hypot(ey);
    if len > 0.0 && ey != 0.0 {
        let (nx, ny) = (-ey / len * d, ex / len * d);
        for s in [1.0, -1.0] {
            let t = (y - (a[1] + s * ny)) / ey;
            if (0.0..=1.0).contains(&t) {
                take(a[0] + s * nx + t * ex);
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// Marks every cell of `bits` whose centre lies in `ring` (non-zero winding) or within `delta`
/// of its boundary; cell (0, 0) has its lower-left corner at `(x0, y0)` in the ring's frame.
fn stamp(bits: &mut Bits, ring: &[P2], x0: f64, y0: f64, cell: f64, delta: f64) {
    let (nx, ny) = (bits.nx, bits.ny);
    let Some((ymin, ymax)) = ring.iter().fold(None, |acc: Option<(f64, f64)>, p| {
        Some(acc.map_or((p[1], p[1]), |(lo, hi)| (lo.min(p[1]), hi.max(p[1]))))
    }) else {
        return;
    };
    let yc = |r: usize| y0 + (r as f64 + 0.5) * cell;
    let edges = || {
        ring.iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
    };

    // inside: scanline crossings per row, half-open so a shared vertex counts once
    if let Some((r0, r1)) = centres_in(ymin, ymax, y0, cell, ny) {
        let mut cross: Vec<Vec<(f64, i32)>> = vec![Vec::new(); r1 - r0 + 1];
        for (&a, &b) in edges() {
            if a[1] == b[1] {
                continue;
            }
            let Some((ra, rb)) = centres_in(a[1].min(b[1]), a[1].max(b[1]), y0, cell, ny) else {
                continue;
            };
            for r in ra.max(r0)..=rb.min(r1) {
                let y = yc(r);
                if (a[1] <= y) != (b[1] <= y) {
                    let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    let dir = if b[1] > a[1] { 1 } else { -1 };
                    if let Some(v) = cross.get_mut(r - r0) {
                        v.push((x, dir));
                    }
                }
            }
        }
        for (k, v) in cross.iter_mut().enumerate() {
            v.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut wind = 0;
            for pair in v.windows(2) {
                if let [p, q] = pair {
                    wind += p.1;
                    if wind != 0 {
                        if let Some((c0, c1)) = centres_in(p.0, q.0, x0, cell, nx) {
                            bits.set_span(r0 + k, c0, c1);
                        }
                    }
                }
            }
        }
    }

    // the boundary grown by `delta`: one closed-form span per edge and row
    for (&a, &b) in edges() {
        let lo = a[1].min(b[1]) - delta;
        let hi = a[1].max(b[1]) + delta;
        let Some((ra, rb)) = centres_in(lo, hi, y0, cell, ny) else {
            continue;
        };
        for r in ra..=rb {
            if let Some((xa, xb)) = capsule_span(a, b, yc(r), delta) {
                if let Some((c0, c1)) = centres_in(xa, xb, x0, cell, nx) {
                    bits.set_span(r, c0, c1);
                }
            }
        }
    }
}

/// `dst` bit `x` |= `src` bit `x + s` (bits past the end of `src` are clear).
fn shr_or(dst: &mut [u64], src: &[u64], s: usize) {
    let (q, r) = (s / 64, s % 64);
    for (w, d) in dst.iter_mut().enumerate() {
        let mut v = src.get(w + q).copied().unwrap_or(0) >> r;
        if r > 0 {
            v |= src.get(w + q + 1).copied().unwrap_or(0) << (64 - r);
        }
        *d |= v;
    }
}

/// `buf` bit `x` |= `buf` bit `x + s`, in place (word `w` reads only words not yet changed).
fn shr_or_self(buf: &mut [u64], s: usize) {
    let (q, r) = (s / 64, s % 64);
    for w in 0..buf.len() {
        let mut v = buf.get(w + q).copied().unwrap_or(0) >> r;
        if r > 0 {
            v |= buf.get(w + q + 1).copied().unwrap_or(0) << (64 - r);
        }
        if let Some(d) = buf.get_mut(w) {
            *d |= v;
        }
    }
}

/// `out` bit `x` = whether any of `src` bits `x..x + len` is set.
fn window(src: &[u64], len: usize, out: &mut Vec<u64>) {
    out.clear();
    out.extend_from_slice(src);
    let mut have = 1;
    while have * 2 <= len {
        shr_or_self(out, have);
        have *= 2;
    }
    if have < len {
        // windows `x..x + have` and `x + len - have..x + len` overlap and cover `x..x + len`
        shr_or_self(out, len - have);
    }
}

/// Whether bits `0..nb` of `v` are all set.
fn all_set(v: &[u64], nb: usize) -> bool {
    v.iter().enumerate().all(|(w, &x)| {
        let left = nb.saturating_sub(w * 64);
        left == 0 || x | u64::MAX.checked_shl(left as u32).unwrap_or(0) == u64::MAX
    })
}

/// The first clear bit below `nb`.
fn first_clear(v: &[u64], nb: usize) -> Option<usize> {
    v.iter()
        .enumerate()
        .find_map(|(w, &x)| (x != u64::MAX).then(|| w * 64 + (!x).trailing_zeros() as usize))
        .filter(|&i| i < nb)
}

/// One turn of a part, ready to place.
struct Pose {
    angle: u16,
    /// The turned bounding box's lower-left corner: `dx = x on sheet - ox`.
    ox: f64,
    oy: f64,
    w: f64,
    h: f64,
    /// The turned outline, its bounding box moved to the origin.
    pts: Ring,
    /// The cells the part could touch, cell (0, 0) being cell (-2, -2) of its box.
    mask: Bits,
    runs: Vec<Vec<(usize, usize)>>,
    cells: usize,
    /// The last grid column and row the box corner may take.
    imax: usize,
    jmax: usize,
}

/// The last grid step `n` with `n cell + size <= span` (within [`FIT_EPS`]), if any.
fn last_step(span: f64, size: f64, cell: f64) -> Option<usize> {
    if size > span + FIT_EPS {
        return None;
    }
    let ok = |n: usize| n as f64 * cell + size <= span + FIT_EPS;
    let mut n = ((span - size) / cell).floor().clamp(0.0, 1e9) as usize;
    while n > 0 && !ok(n) {
        n -= 1;
    }
    while ok(n + 1) {
        n += 1;
    }
    Some(n)
}

/// The sheet's grid.
struct Grid {
    cell: f64,
    /// Half a cell diagonal: anything touching a cell is this close to its centre.
    hd: f64,
    /// The usable area inside the margin.
    iw: f64,
    ih: f64,
    /// Cells across and up, with [`PAD`] around the usable area.
    gw: usize,
    gh: usize,
}

/// The distinct allowed turns of a part that fit an empty sheet.
fn poses(ring: &[P2], angles: &[u16], g: &Grid) -> Vec<Pose> {
    let mut out: Vec<Pose> = Vec::new();
    for &angle in angles {
        let rp: Ring = ring.iter().map(|&q| turn(q, angle)).collect();
        let Some([x0, y0, x1, y1]) = super::region::ring_bounds([&rp]) else {
            continue;
        };
        let (w, h) = (x1 - x0, y1 - y0);
        let (Some(imax), Some(jmax)) = (last_step(g.iw, w, g.cell), last_step(g.ih, h, g.cell))
        else {
            continue;
        };
        let pts: Ring = rp.iter().map(|p| [p[0] - x0, p[1] - y0]).collect();
        let mut mask = Bits::new(
            (w / g.cell).ceil() as usize + 5,
            (h / g.cell).ceil() as usize + 5,
        );
        stamp(
            &mut mask,
            &pts,
            -2.0 * g.cell,
            -2.0 * g.cell,
            g.cell,
            g.hd + TOL,
        );
        if out
            .iter()
            .any(|p| (p.w - w).abs() < 1e-6 && (p.h - h).abs() < 1e-6 && p.mask == mask)
        {
            continue; // a symmetric part: this turn looks the same as an earlier one
        }
        out.push(Pose {
            angle,
            ox: x0,
            oy: y0,
            w,
            h,
            pts,
            runs: mask.runs(),
            cells: mask.count(),
            mask,
            imax,
            jmax,
        });
    }
    out
}

struct Sheet {
    occ: Bits,
    free: usize,
    /// Rasters that found no room here (cells only get used, so they never will).
    failed: Vec<Bits>,
    /// The used box from the margin corner.
    ex: f64,
    ey: f64,
}

/// The best grid spot `(score, j, i)` scoring below `bound` for `pose` on `sheet`: the smallest
/// used box, then the lowest row, then the leftmost column.
fn best_spot(
    sheet: &Sheet,
    pose: &Pose,
    cell: f64,
    bound: f64,
    win: &mut Vec<u64>,
    blocked: &mut Vec<u64>,
) -> Option<(f64, usize, usize)> {
    let mut best = None;
    let mut lim = bound;
    let ex = |i: usize| sheet.ex.max(i as f64 * cell + pose.w);
    for j in 0..=pose.jmax {
        let ey = sheet.ey.max(j as f64 * cell + pose.h);
        // the score only grows with i and j: nothing further on can beat `lim`
        if ey * ex(0) >= lim {
            break;
        }
        let ilim = if lim.is_finite() {
            ((lim / ey - pose.w) / cell)
                .ceil()
                .clamp(0.0, pose.imax as f64) as usize
        } else {
            pose.imax
        };
        let nb = ilim + 1;
        blocked.clear();
        blocked.resize(nb.div_ceil(64), 0);
        let mut full = false;
        // raster cell (r, c) of a pose at grid (j, i) lands on sheet cell (r + j + 1, c + i + 1)
        'rows: for (r, runs) in pose.runs.iter().enumerate() {
            if runs.is_empty() {
                continue;
            }
            let row = sheet.occ.row(r + j + 1);
            if row.iter().all(|&w| w == 0) {
                continue;
            }
            for &(a, len) in runs {
                window(row, len, win);
                shr_or(blocked, win, a + 1);
                if all_set(blocked, nb) {
                    full = true;
                    break 'rows;
                }
            }
        }
        if full {
            continue;
        }
        if let Some(i) = first_clear(blocked, nb) {
            let s = ey * ex(i);
            if s < lim {
                lim = s;
                best = Some((s, j, i));
            }
        }
    }
    best
}

fn does_not_fit(k: usize, ring: Option<&Ring>) -> FlatError {
    let [x0, y0, x1, y1] = ring
        .and_then(|r| super::region::ring_bounds([r]))
        .unwrap_or([0.0; 4]);
    FlatError::DoesNotFit {
        part: k,
        width: x1 - x0,
        height: y1 - y0,
    }
}

/// Every part placed once, on as few `sheet_w` x `sheet_h` sheets as it takes. Each part is its
/// outline (holes do not matter); parts end at least `gap` apart and at least `margin` from the
/// sheet's edges; `rotate` allows turns by 90°. One placement per part, in part order.
///
/// Parts go largest first, each on the first sheet where one of its turns fits, at the grid
/// spot keeping the sheet's used box smallest (lowest, then leftmost on ties). Spacing may
/// exceed `gap` by up to about 1.5 grid cells (the grid has about 120 000 cells per sheet).
///
/// # Errors
/// [`FlatError::Setting`] for a sheet with no room inside its margin, a sheet, margin or gap
/// that is negative, not finite or beyond 100 m, or a part with fewer than three points or a
/// point not finite or beyond 100 m; [`FlatError::DoesNotFit`] for a part no empty sheet takes
/// (its index and unturned size); [`FlatError::TooBig`] past 5000 parts or 200 000 points.
pub fn nest(
    outlines: &[Ring],
    sheet_w: f64,
    sheet_h: f64,
    gap: f64,
    margin: f64,
    rotate: bool,
) -> FlatResult<Vec<Placement>> {
    let sane = |v: f64| v.is_finite() && v.abs() <= LIMIT;
    if !sane(sheet_w) || !sane(sheet_h) {
        return Err(FlatError::Setting("sheet"));
    }
    if !sane(margin) || margin < 0.0 {
        return Err(FlatError::Setting("margin"));
    }
    let (iw, ih) = (sheet_w - 2.0 * margin, sheet_h - 2.0 * margin);
    if !(iw > 0.0 && ih > 0.0) {
        return Err(FlatError::Setting("sheet"));
    }
    if !sane(gap) || gap < 0.0 {
        return Err(FlatError::Setting("gap"));
    }
    if outlines.is_empty() {
        return Ok(Vec::new());
    }
    if outlines.len() > MAX_PARTS {
        return Err(FlatError::TooBig("parts to nest"));
    }
    if outlines.iter().map(Vec::len).sum::<usize>() > MAX_POINTS {
        return Err(FlatError::TooBig("points to nest"));
    }
    if outlines
        .iter()
        .any(|r| r.len() < 3 || r.iter().flatten().any(|&c| !sane(c)))
    {
        return Err(FlatError::Setting("part"));
    }

    let cell = MIN_CELL
        .max((iw * ih / TARGET_CELLS).sqrt())
        .max(iw.max(ih) / MAX_SIDE_CELLS);
    let g = Grid {
        cell,
        hd: cell * std::f64::consts::FRAC_1_SQRT_2,
        iw,
        ih,
        gw: (iw / cell).ceil() as usize + 2 * PAD + 1,
        gh: (ih / cell).ceil() as usize + 2 * PAD + 1,
    };
    let cells = g.gw * g.gh;
    let angles: &[u16] = if rotate { &[0, 90, 180, 270] } else { &[0] };

    let mut all = Vec::with_capacity(outlines.len());
    for (k, ring) in outlines.iter().enumerate() {
        let ps = poses(ring, angles, &g);
        if ps.is_empty() {
            return Err(does_not_fit(k, Some(ring)));
        }
        all.push(ps);
    }

    let areas: Vec<f64> = outlines
        .iter()
        .map(|r| super::region::ring_area(r).abs())
        .collect();
    let area = |k: usize| areas.get(k).copied().unwrap_or(0.0);
    let mut order: Vec<usize> = (0..outlines.len()).collect();
    order.sort_by(|&a, &b| area(b).total_cmp(&area(a)).then(a.cmp(&b)));

    let mut sheets: Vec<Sheet> = Vec::new();
    let mut result: Vec<Option<Placement>> = vec![None; outlines.len()];
    let (mut win, mut blocked) = (Vec::new(), Vec::new());
    for k in order {
        let Some(cands) = all.get(k) else {
            continue;
        };
        let mut s = 0;
        loop {
            if s == sheets.len() {
                sheets.push(Sheet {
                    occ: Bits::new(g.gw, g.gh),
                    free: cells,
                    failed: Vec::new(),
                    ex: 0.0,
                    ey: 0.0,
                });
            }
            let Some(sheet) = sheets.get_mut(s) else {
                return Err(does_not_fit(k, outlines.get(k)));
            };
            let fresh = sheet.free == cells;
            let mut best: Option<((f64, usize, usize), &Pose)> = None;
            for pose in cands {
                // cheap rejects: too few free cells, or the raster covers one that failed here
                if pose.cells > sheet.free || sheet.failed.iter().any(|f| pose.mask.covers(f)) {
                    continue;
                }
                let bound = best.map_or(f64::INFINITY, |(b, _)| b.0);
                match best_spot(sheet, pose, g.cell, bound, &mut win, &mut blocked) {
                    Some(spot) => best = Some((spot, pose)),
                    // with no bound nothing was pruned: the pose has no room here at all
                    None if bound.is_infinite() && sheet.failed.len() < MAX_FAILED => {
                        sheet.failed.push(pose.mask.clone());
                    }
                    None => {}
                }
            }
            if let Some(((_, j, i), pose)) = best {
                let (x, y) = (i as f64 * g.cell, j as f64 * g.cell);
                // mark every cell the part grown by the gap could touch
                stamp(
                    &mut sheet.occ,
                    &pose.pts,
                    -((PAD + i) as f64) * g.cell,
                    -((PAD + j) as f64) * g.cell,
                    g.cell,
                    gap + g.hd + TOL,
                );
                sheet.free = cells - sheet.occ.count().min(cells);
                sheet.ex = sheet.ex.max(x + pose.w);
                sheet.ey = sheet.ey.max(y + pose.h);
                if let Some(slot) = result.get_mut(k) {
                    *slot = Some(Placement {
                        part: k,
                        sheet: s,
                        angle: pose.angle,
                        dx: margin + x - pose.ox,
                        dy: margin + y - pose.oy,
                    });
                }
                break;
            }
            if fresh {
                // an empty sheet takes every pose that passed `poses`: never loop on
                return Err(does_not_fit(k, outlines.get(k)));
            }
            s += 1;
        }
    }
    Ok(result.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn rect(w: f64, h: f64, x: f64, y: f64) -> Ring {
        vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
    }

    /// A wavy closed outline around the origin, `n` points.
    fn blob(r: f64, n: usize, seed: f64, stretch: f64) -> Ring {
        (0..n)
            .map(|i| {
                let t = 2.0 * PI * i as f64 / n as f64;
                let rr = r * (1.0 + 0.3 * (5.0 * t + seed).sin());
                [rr * t.cos() * stretch, rr * t.sin()]
            })
            .collect()
    }

    /// An L: `a` wide, `b` tall, arms `t` thick.
    fn ell(a: f64, b: f64, t: f64) -> Ring {
        vec![[0.0, 0.0], [a, 0.0], [a, t], [t, t], [t, b], [0.0, b]]
    }

    /// A fixed-seed generator for test shapes.
    struct Lcg(u64);
    impl Lcg {
        fn uniform(&mut self, lo: f64, hi: f64) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            lo + (hi - lo) * ((self.0 >> 11) as f64 / (1u64 << 53) as f64)
        }
    }

    fn pt_seg(p: P2, a: P2, b: P2) -> f64 {
        let (ex, ey) = (b[0] - a[0], b[1] - a[1]);
        let l2 = ex * ex + ey * ey;
        let t = if l2 > 0.0 {
            (((p[0] - a[0]) * ex + (p[1] - a[1]) * ey) / l2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p[0] - a[0] - t * ex).hypot(p[1] - a[1] - t * ey)
    }

    fn orient(a: P2, b: P2, c: P2) -> f64 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    }

    fn seg_seg(a: P2, b: P2, c: P2, d: P2) -> f64 {
        let (o1, o2) = (orient(a, b, c), orient(a, b, d));
        let (o3, o4) = (orient(c, d, a), orient(c, d, b));
        if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
            return 0.0;
        }
        pt_seg(a, c, d)
            .min(pt_seg(b, c, d))
            .min(pt_seg(c, a, b))
            .min(pt_seg(d, a, b))
    }

    fn edges(r: &[P2]) -> Vec<(P2, P2)> {
        (0..r.len()).map(|i| (r[i], r[(i + 1) % r.len()])).collect()
    }

    /// Non-zero winding test.
    fn inside(p: P2, r: &[P2]) -> bool {
        let mut w = 0;
        for (a, b) in edges(r) {
            if (a[1] <= p[1]) != (b[1] <= p[1]) {
                let x = a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                if x > p[0] {
                    w += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
        w != 0
    }

    /// The exact distance between two filled outlines (0 when they overlap).
    fn poly_dist(a: &[P2], b: &[P2]) -> f64 {
        if inside(a[0], b) || inside(b[0], a) {
            return 0.0;
        }
        let mut d = f64::INFINITY;
        for (p, q) in edges(a) {
            for (r, s) in edges(b) {
                d = d.min(seg_seg(p, q, r, s));
            }
        }
        d
    }

    fn bbox(r: &[P2]) -> [f64; 4] {
        super::super::region::ring_bounds([&r.to_vec()]).unwrap()
    }

    /// Every part once in part order, turns valid, inside the margin, every pair on a sheet
    /// more than `gap` apart, sheets numbered without gaps. Returns the parts per sheet.
    fn check(
        parts: &[Ring],
        pl: &[Placement],
        w: f64,
        h: f64,
        gap: f64,
        margin: f64,
    ) -> Vec<usize> {
        assert_eq!(pl.len(), parts.len());
        let mut placed: Vec<Vec<Ring>> = Vec::new();
        for (k, p) in pl.iter().enumerate() {
            assert_eq!(p.part, k);
            assert!([0, 90, 180, 270].contains(&p.angle));
            let o = transform(&parts[k], p);
            let [x0, y0, x1, y1] = bbox(&o);
            assert!(
                x0 >= margin - 1e-6 && x1 <= w - margin + 1e-6,
                "part {k} x {x0} {x1}"
            );
            assert!(
                y0 >= margin - 1e-6 && y1 <= h - margin + 1e-6,
                "part {k} y {y0} {y1}"
            );
            if placed.len() <= p.sheet {
                placed.resize(p.sheet + 1, Vec::new());
            }
            placed[p.sheet].push(o);
        }
        assert!(
            placed.iter().all(|s| !s.is_empty()),
            "an empty sheet in between"
        );
        for s in &placed {
            for a in 0..s.len() {
                let ba = bbox(&s[a]);
                for b in a + 1..s.len() {
                    let bb = bbox(&s[b]);
                    let dx = (bb[0] - ba[2]).max(ba[0] - bb[2]).max(0.0);
                    let dy = (bb[1] - ba[3]).max(ba[1] - bb[3]).max(0.0);
                    if dx.hypot(dy) > gap + 1e-3 {
                        continue;
                    }
                    let d = poly_dist(&s[a], &s[b]);
                    assert!(
                        d >= gap + 1e-3,
                        "parts {a} and {b} are {d} apart, gap {gap}"
                    );
                }
            }
        }
        placed.iter().map(Vec::len).collect()
    }

    #[test]
    fn transform_turns_then_moves() {
        let ring = vec![[1.0, 0.0], [2.0, 1.0]];
        let p = |angle| Placement {
            part: 0,
            sheet: 0,
            angle,
            dx: 10.0,
            dy: 20.0,
        };
        assert_eq!(transform(&ring, &p(0)), vec![[11.0, 20.0], [12.0, 21.0]]);
        assert_eq!(transform(&ring, &p(90)), vec![[10.0, 21.0], [9.0, 22.0]]);
        assert_eq!(transform(&ring, &p(180)), vec![[9.0, 20.0], [8.0, 19.0]]);
        assert_eq!(transform(&ring, &p(270)), vec![[10.0, 19.0], [11.0, 18.0]]);
        assert_eq!(transform(&ring, &p(450)), transform(&ring, &p(90)));
        assert_eq!(transform(&ring, &p(45)), transform(&ring, &p(0)));
        assert_eq!(transform(&[], &p(90)), Ring::new());
        // exact: no trigonometry, so a quarter turn of 0.1 is exactly -0.1
        assert_eq!(transform(&[[0.0, 0.1]], &p(90)), vec![[10.0 - 0.1, 20.0]]);
    }

    #[test]
    fn mixed_shapes_never_overlap() {
        let mut rng = Lcg(3);
        let mut parts = Vec::new();
        for k in 0..40 {
            match k % 3 {
                0 => {
                    let (w, h, x) = (
                        rng.uniform(10.0, 80.0),
                        rng.uniform(10.0, 60.0),
                        rng.uniform(-50.0, 50.0),
                    );
                    parts.push(rect(w, h, x, 7.0));
                }
                1 => parts.push(blob(rng.uniform(8.0, 30.0), 60, f64::from(k), 1.0)),
                _ => {
                    let (a, b, t) = (
                        rng.uniform(30.0, 70.0),
                        rng.uniform(30.0, 70.0),
                        rng.uniform(8.0, 15.0),
                    );
                    parts.push(ell(a, b, t));
                }
            }
        }
        let pl = nest(&parts, 300.0, 200.0, 3.0, 5.0, true).unwrap();
        check(&parts, &pl, 300.0, 200.0, 3.0, 5.0);
    }

    #[test]
    fn gap_and_margin_respected_for_other_values() {
        let parts: Vec<Ring> = (0..12).map(|k| blob(20.0, 80, f64::from(k), 1.0)).collect();
        for (gap, margin) in [(6.0, 12.0), (0.0, 0.0), (1.5, 2.5), (20.0, 1.0)] {
            let pl = nest(&parts, 250.0, 180.0, gap, margin, true).unwrap();
            check(&parts, &pl, 250.0, 180.0, gap, margin);
        }
    }

    #[test]
    fn overflow_opens_new_sheets() {
        // 90 + 3 + 90 fits twice across the 190 mm usable width: four squares a sheet
        let parts: Vec<Ring> = (0..10).map(|_| rect(90.0, 90.0, 0.0, 0.0)).collect();
        let pl = nest(&parts, 200.0, 200.0, 3.0, 5.0, true).unwrap();
        assert_eq!(check(&parts, &pl, 200.0, 200.0, 3.0, 5.0), vec![4, 4, 2]);
        // the first lies in the margin corner, unturned (a square's turns all look the same)
        assert_eq!(
            pl[0],
            Placement {
                part: 0,
                sheet: 0,
                angle: 0,
                dx: 5.0,
                dy: 5.0
            }
        );
    }

    #[test]
    fn rotation_when_it_is_the_only_way() {
        let strip = vec![rect(50.0, 150.0, 0.0, 0.0)]; // 150 tall, usable height only 110
        let pl = nest(&strip, 200.0, 120.0, 3.0, 5.0, true).unwrap();
        assert!(pl[0].angle == 90 || pl[0].angle == 270);
        check(&strip, &pl, 200.0, 120.0, 3.0, 5.0);
        assert!(matches!(
            nest(&strip, 200.0, 120.0, 3.0, 5.0, false),
            Err(FlatError::DoesNotFit { part: 0, .. })
        ));
    }

    #[test]
    fn rotation_saves_a_sheet() {
        // the big part leaves a 57 mm wide column; the strips only fit there standing up
        let parts = vec![
            rect(140.0, 100.0, 0.0, 0.0),
            rect(90.0, 22.0, 0.0, 0.0),
            rect(90.0, 22.0, 0.0, 0.0),
        ];
        let pl = nest(&parts, 210.0, 110.0, 3.0, 5.0, true).unwrap();
        assert_eq!(check(&parts, &pl, 210.0, 110.0, 3.0, 5.0), vec![3]);
        assert!(pl[1].angle == 90 || pl[1].angle == 270);
        assert!(pl[2].angle == 90 || pl[2].angle == 270);
        let pl = nest(&parts, 210.0, 110.0, 3.0, 5.0, false).unwrap();
        assert!(pl.iter().all(|p| p.angle == 0));
        assert_eq!(check(&parts, &pl, 210.0, 110.0, 3.0, 5.0).len(), 2);
    }

    #[test]
    fn small_part_sits_in_the_notch_of_an_l() {
        // the L fills the whole usable 200 x 200; only true-shape nesting finds the notch
        let parts = vec![
            ell(200.0, 200.0, 60.0),
            rect(100.0, 100.0, 0.0, 0.0),
            blob(8.0, 100, 0.0, 1.0),
        ];
        let pl = nest(&parts, 210.0, 210.0, 3.0, 5.0, true).unwrap();
        assert_eq!(check(&parts, &pl, 210.0, 210.0, 3.0, 5.0), vec![3]);
    }

    #[test]
    fn too_big_does_not_fit_with_index_and_size() {
        let parts = vec![rect(50.0, 50.0, 0.0, 0.0), rect(250.0, 40.0, 0.0, 0.0)];
        assert_eq!(
            nest(&parts, 200.0, 300.0, 3.0, 5.0, false),
            Err(FlatError::DoesNotFit {
                part: 1,
                width: 250.0,
                height: 40.0
            })
        );
        let pl = nest(&parts, 200.0, 300.0, 3.0, 5.0, true).unwrap(); // but it fits standing up
        assert!(pl[1].angle == 90 || pl[1].angle == 270);
        check(&parts, &pl, 200.0, 300.0, 3.0, 5.0);
        assert_eq!(
            nest(
                &[rect(300.0, 300.0, 7.0, 7.0)],
                200.0,
                200.0,
                3.0,
                5.0,
                true
            ),
            Err(FlatError::DoesNotFit {
                part: 0,
                width: 300.0,
                height: 300.0
            })
        );
        // a hair too wide for the usable 190 mm
        assert!(matches!(
            nest(
                &[rect(190.001, 10.0, 0.0, 0.0)],
                200.0,
                200.0,
                3.0,
                5.0,
                false
            ),
            Err(FlatError::DoesNotFit { part: 0, .. })
        ));
    }

    #[test]
    fn exact_fit_on_an_empty_sheet() {
        let parts = vec![rect(190.0, 90.0, 33.0, -12.0)]; // exactly the usable area
        let pl = nest(&parts, 200.0, 100.0, 3.0, 5.0, true).unwrap();
        check(&parts, &pl, 200.0, 100.0, 3.0, 5.0);
        assert_eq!(
            pl[0],
            Placement {
                part: 0,
                sheet: 0,
                angle: 0,
                dx: 5.0 - 33.0,
                dy: 5.0 + 12.0
            }
        );
    }

    #[test]
    fn largest_part_goes_first_into_the_corner() {
        let parts = vec![rect(10.0, 10.0, 0.0, 0.0), rect(60.0, 40.0, 100.0, 100.0)];
        let pl = nest(&parts, 200.0, 200.0, 3.0, 5.0, true).unwrap();
        assert_eq!(
            pl[1],
            Placement {
                part: 1,
                sheet: 0,
                angle: 0,
                dx: 5.0 - 100.0,
                dy: 5.0 - 100.0
            }
        );
        check(&parts, &pl, 200.0, 200.0, 3.0, 5.0);
    }

    #[test]
    fn deterministic() {
        let parts: Vec<Ring> = (0..30)
            .map(|k| blob(10.0 + f64::from(k % 7) * 4.0, 50, f64::from(k), 1.0))
            .collect();
        let a = nest(&parts, 200.0, 150.0, 3.0, 5.0, true).unwrap();
        assert_eq!(a, nest(&parts, 200.0, 150.0, 3.0, 5.0, true).unwrap());
        check(&parts, &a, 200.0, 150.0, 3.0, 5.0);
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(nest(&[], 100.0, 100.0, 3.0, 5.0, true), Ok(Vec::new()));
    }

    #[test]
    fn bad_input_errors() {
        let sq = vec![rect(5.0, 5.0, 0.0, 0.0)];
        let line = vec![vec![[0.0, 0.0], [1.0, 1.0]]];
        assert_eq!(
            nest(&line, 100.0, 100.0, 3.0, 5.0, true),
            Err(FlatError::Setting("part"))
        );
        assert_eq!(
            nest(&sq, 10.0, 10.0, 3.0, 5.0, true),
            Err(FlatError::Setting("sheet"))
        );
        assert_eq!(
            nest(&sq, 100.0, 9.0, 3.0, 5.0, true),
            Err(FlatError::Setting("sheet"))
        );
        assert_eq!(
            nest(&sq, 100.0, 100.0, -1.0, 5.0, true),
            Err(FlatError::Setting("gap"))
        );
        assert_eq!(
            nest(&sq, 100.0, 100.0, f64::NAN, 5.0, true),
            Err(FlatError::Setting("gap"))
        );
        assert_eq!(
            nest(&sq, 100.0, 100.0, 3.0, -1.0, true),
            Err(FlatError::Setting("margin"))
        );
        assert_eq!(
            nest(&sq, f64::INFINITY, 100.0, 3.0, 5.0, true),
            Err(FlatError::Setting("sheet"))
        );
        let nan = vec![vec![[0.0, 0.0], [1.0, f64::NAN], [1.0, 1.0]]];
        assert_eq!(
            nest(&nan, 100.0, 100.0, 3.0, 5.0, true),
            Err(FlatError::Setting("part"))
        );
        let far = vec![rect(1.0, 1.0, 1e6, 0.0)];
        assert_eq!(
            nest(&far, 100.0, 100.0, 3.0, 5.0, true),
            Err(FlatError::Setting("part"))
        );
    }

    #[test]
    fn budgets_are_enforced() {
        let many: Vec<Ring> = (0..=MAX_PARTS).map(|_| rect(1.0, 1.0, 0.0, 0.0)).collect();
        assert_eq!(
            nest(&many, 100.0, 100.0, 3.0, 5.0, true),
            Err(FlatError::TooBig("parts to nest"))
        );
        let big = vec![
            blob(10.0, MAX_POINTS / 2 + 1, 0.0, 1.0),
            blob(10.0, MAX_POINTS / 2, 0.0, 1.0),
        ];
        assert_eq!(
            nest(&big, 100.0, 100.0, 3.0, 5.0, true),
            Err(FlatError::TooBig("points to nest"))
        );
    }

    #[test]
    fn hostile_numbers_never_panic() {
        let vals = [
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            -1.0,
            1e-300,
            5e-324,
            1e300,
            1e5,
            2e5,
            0.3,
            50.0,
        ];
        let parts = vec![rect(5.0, 5.0, 0.0, 0.0), blob(3.0, 20, 0.0, 1.0)];
        for &w in &vals {
            for &h in &vals {
                for &g in &[f64::NAN, -1.0, 0.0, 1.0, 1e5, 1e300] {
                    for &m in &[f64::NAN, -1.0, 0.0, 0.1, 1e300] {
                        if let Ok(pl) = nest(&parts, w, h, g, m, true) {
                            check(&parts, &pl, w, h, g, m);
                        }
                    }
                }
            }
        }
        // degenerate outlines: a zero-width sliver, repeated points, a self-crossing bow tie
        let odd = vec![
            vec![[0.0, 0.0], [50.0, 0.0], [25.0, 0.0]],
            vec![[1.0, 1.0], [1.0, 1.0], [1.0, 1.0]],
            vec![[0.0, 0.0], [20.0, 20.0], [20.0, 0.0], [0.0, 20.0]],
            rect(30.0, 30.0, 0.0, 0.0),
            vec![
                [0.0, 0.0],
                [40.0, 0.0],
                [40.0, 10.0],
                [20.0, 10.0],
                [20.0, 60.0],
                [20.0, 10.0],
                [0.0, 10.0],
            ],
        ];
        let pl = nest(&odd, 120.0, 80.0, 2.0, 3.0, true).unwrap();
        check(&odd, &pl, 120.0, 80.0, 2.0, 3.0);
        // a long thin sheet keeps a bounded grid
        let pl = nest(&parts, 1e5, 20.0, 1.0, 1.0, true).unwrap();
        check(&parts, &pl, 1e5, 20.0, 1.0, 1.0);
        // a huge gap: one part per sheet
        let pl = nest(&parts, 100.0, 100.0, 1e5, 5.0, true).unwrap();
        assert_eq!(check(&parts, &pl, 100.0, 100.0, 1e5, 5.0), vec![1, 1]);
    }

    #[test]
    fn rasters_mark_exactly_the_grown_outline() {
        let shapes = [
            ell(20.0, 15.0, 4.0),
            blob(6.0, 40, 1.0, 1.3),
            vec![[0.0, 0.0], [9.0, 0.0], [4.0, 0.0]],
        ];
        for ring in &shapes {
            for (cell, delta) in [(0.7, 0.5), (1.3, 3.2), (0.25, 0.0)] {
                let mut b = Bits::new(70, 60);
                let (x0, y0) = (-15.0, -12.0);
                stamp(&mut b, ring, x0, y0, cell, delta);
                for r in 0..b.ny {
                    for c in 0..b.nx {
                        let p = [x0 + (c as f64 + 0.5) * cell, y0 + (r as f64 + 0.5) * cell];
                        let d = if inside(p, ring) {
                            0.0
                        } else {
                            edges(ring)
                                .iter()
                                .map(|&(a, q)| pt_seg(p, a, q))
                                .fold(f64::INFINITY, f64::min)
                        };
                        if (d - delta).abs() < 1e-9 || d < 1e-9 && delta == 0.0 {
                            continue; // on the boundary: either way
                        }
                        assert_eq!(b.get(r, c), d <= delta, "cell {r} {c} at distance {d}");
                    }
                }
            }
        }
    }

    #[test]
    fn word_windows_and_shifts() {
        let src = vec![1u64 << 3 | 1 << 63, 1 << 5];
        let mut w = Vec::new();
        window(&src, 4, &mut w);
        // bit x set when any of bits x..x+4 is: 0..=3, 60..=63, 66..=69
        for x in 0..128 {
            let want = x <= 3 || (60..=63).contains(&x) || (66..=69).contains(&x);
            assert_eq!(w[x / 64] >> (x % 64) & 1 == 1, want, "bit {x}");
        }
        let mut d = vec![0u64; 2];
        shr_or(&mut d, &src, 3);
        assert_eq!(d, vec![1 | 1 << 60, 1 << 2]);
        assert!(all_set(&[u64::MAX, 0b111], 67));
        assert!(!all_set(&[u64::MAX, 0b011], 67));
        assert_eq!(first_clear(&[u64::MAX, 0b1011], 70), Some(66));
        assert_eq!(first_clear(&[u64::MAX, 0b1111], 68), None);
        assert_eq!(last_step(10.0, 4.0, 2.0), Some(3));
        assert_eq!(last_step(10.0, 10.0, 2.0), Some(0));
        assert_eq!(last_step(10.0, 10.1, 2.0), None);
    }

    #[test]
    fn symmetric_turns_are_tried_once() {
        let g = Grid {
            cell: 1.0,
            hd: std::f64::consts::FRAC_1_SQRT_2,
            iw: 200.0,
            ih: 100.0,
            gw: 207,
            gh: 107,
        };
        let all = [0, 90, 180, 270];
        assert_eq!(poses(&rect(10.0, 10.0, 0.0, 0.0), &all, &g).len(), 1);
        assert_eq!(poses(&rect(20.0, 10.0, 0.0, 0.0), &all, &g).len(), 2);
        assert_eq!(poses(&ell(20.0, 10.0, 3.0), &all, &g).len(), 4);
        assert_eq!(
            poses(&rect(20.0, 150.0, 0.0, 0.0), &all, &g)
                .iter()
                .map(|p| p.angle)
                .collect::<Vec<_>>(),
            vec![90]
        );
    }

    #[test]
    fn hundred_detailed_parts_nest() {
        let mut rng = Lcg(1);
        let parts: Vec<Ring> = (0..100)
            .map(|k| blob(rng.uniform(15.0, 45.0), 300, f64::from(k), 1.5))
            .collect();
        let t = std::time::Instant::now();
        let pl = nest(&parts, 600.0, 400.0, 3.0, 5.0, true).unwrap();
        let took = t.elapsed();
        let per = check(&parts, &pl, 600.0, 400.0, 3.0, 5.0);
        let area: f64 = parts
            .iter()
            .map(|r| super::super::region::ring_area(r).abs())
            .sum();
        println!(
            "100 parts of 300 points: {took:?}, sheets {per:?}, part area {:.0}% of the sheets",
            100.0 * area / (590.0 * 390.0 * per.len() as f64)
        );
        // the parts cover about 4 usable sheets; a true-shape packer needs few more
        assert!(
            per.len() <= (area / (590.0 * 390.0) / 0.5).ceil() as usize,
            "{per:?}"
        );
    }
}
