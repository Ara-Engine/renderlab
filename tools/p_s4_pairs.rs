//! P-S4 CPU pair-count replay (docs/design-nonrt-sunvis-s4.md): the rejection gate for design S4,
//! occluder-centric sun visibility. It counts the occluder-receiver pairs S4 would process on one
//! frame and how many direction tests they need, to set against the current lookup's walk visits.
//!
//! It loads a prepared asset (RLA v1-3 geometry; materials and textures are skipped), moves every
//! placement's triangles into the sun's frame (x, y across the sun, z toward it), builds a
//! median-split BVH there, and casts the frame's primary rays to find the receivers. Receivers are
//! taken in whole 8x4 trace tiles (the lookup's tile), every `tile_step`-th tile in x and y, and
//! the totals are scaled by `tile_step^2`.
//!
//! A receiver's directions all lie in the cone {x : h > eps, |q| <= slope h} about its origin
//! (q: across the sun, h: toward it). Per receiver it counts:
//!   - `nodes`: BVH nodes a per-receiver cone traversal visits (variant S4-R);
//!   - `culls`: leaf triangles tested against the cone; `pairs`: triangles that pass the
//!     conservative cone cull (the GPU's vsm_test cull);
//!   - for each pair, the hits among the receiver's 256 sequence directions (f64 Moller-Trumbore
//!     from the origin, t >= eps): `zero` (cull survivor, no hit), `full` (all 256, a certificate
//!     candidate) or `partial` (needs per-direction tests);
//!   - its visible count of 256 (umbra 0, penumbra, lit 256).
//! Per tile (variant S4-T, one traversal per tile with the union of its receivers' cones):
//! `tile_nodes`, `tile_cands` (triangles passing the union cull, loaded once per tile) and
//! `tile_tests` (cands x receivers, the per-receiver culls in registers). `hybrid`: per tile, the
//! cheaper of the two in loads. `nodes4`: a 4-wide BVH's node fetches for S4-R (estimate).
//! Variant S4-G (light-space caster grid, `cell=`): `g_cells`, `g_open`, `g_cands`; its pair
//! sets must equal S4-R's (`g_mismatch` counts receivers where they differ).
//!
//! Standalone, no dependencies: `rustc -O --edition 2021 tools/p_s4_pairs.rs -o p_s4_pairs.exe`.
//! Usage: p_s4_pairs rla=<path> eye=x,y,z target=x,y,z fov=<rad> size=1920,1080
//!        light=x,y,z radius=<rad> cell=<m> tile_step=4 threads=4 out=<json> [pgm=<path>]

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

struct In<R: Read>(R);
impl<R: Read> In<R> {
    fn u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.0.read_exact(&mut b).expect("truncated RLA");
        u32::from_le_bytes(b)
    }
    fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    fn skip(&mut self, n: u64) {
        std::io::copy(&mut (&mut self.0).take(n), &mut std::io::sink()).unwrap();
    }
}

/// World triangles in the sun frame: vertices and index triples.
struct Scene {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
}

fn load(path: &str, frame: [V3; 3]) -> Scene {
    let mut r = In(BufReader::with_capacity(1 << 22, std::fs::File::open(path).unwrap()));
    let mut magic = [0u8; 4];
    r.0.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, b"RLA\0");
    let version = r.u32();
    assert!(version <= 3, "RLA v{version}: only v1-3 geometry layouts are read");
    let (meshes, placements) = (r.u32(), r.u32());
    let (_materials, _textures) = (r.u32(), r.u32());
    // Per mesh, per primitive: object-space positions and indices.
    let mut mesh_data: Vec<Vec<(Vec<[f32; 3]>, Vec<u32>)>> = vec![];
    for _ in 0..meshes {
        let mut prims = vec![];
        for _ in 0..r.u32() {
            let _material = r.u32();
            let (nv, ni) = (r.u32() as usize, r.u32() as usize);
            let nt = if version >= 2 {
                let _basis = r.u32();
                r.u32() as u64
            } else {
                0
            };
            let mut pos = Vec::with_capacity(nv);
            for _ in 0..nv {
                pos.push([r.f32(), r.f32(), r.f32()]);
                r.skip(9 * 4); // normal, uv, color
            }
            r.skip(nt * 16);
            let idx = (0..ni).map(|_| r.u32()).collect();
            prims.push((pos, idx));
        }
        mesh_data.push(prims);
    }
    let mut placed = vec![];
    for _ in 0..placements {
        let mesh = r.u32() as usize;
        let mut m = [[0f64; 4]; 4];
        for column in &mut m {
            for x in column.iter_mut() {
                *x = r.f32() as f64;
            }
        }
        placed.push((mesh, m));
    }
    let mut scene = Scene { verts: vec![], tris: vec![] };
    for (mesh, m) in placed {
        for (pos, idx) in &mesh_data[mesh] {
            let base = scene.verts.len() as u32;
            for p in pos {
                let p = [p[0] as f64, p[1] as f64, p[2] as f64];
                let w: V3 = std::array::from_fn(|k| m[0][k] * p[0] + m[1][k] * p[1] + m[2][k] * p[2] + m[3][k]);
                scene.verts.push([dot(w, frame[0]) as f32, dot(w, frame[1]) as f32, dot(w, frame[2]) as f32]);
            }
            for t in idx.chunks_exact(3) {
                scene.tris.push([base + t[0], base + t[1], base + t[2]]);
            }
        }
    }
    scene
}

#[derive(Clone, Copy)]
struct Node {
    lo: [f32; 3],
    hi: [f32; 3],
    /// Leaf: first triangle in `order`, count > 0. Inner: left child index, count == 0.
    first: u32,
    count: u32,
}

struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

fn build(s: &Scene) -> Bvh {
    let cent: Vec<[f32; 3]> = s
        .tris
        .iter()
        .map(|t| std::array::from_fn(|k| (s.verts[t[0] as usize][k] + s.verts[t[1] as usize][k] + s.verts[t[2] as usize][k]) / 3.0))
        .collect();
    let mut order: Vec<u32> = (0..s.tris.len() as u32).collect();
    let mut nodes = vec![Node { lo: [0.0; 3], hi: [0.0; 3], first: 0, count: 0 }];
    let mut stack = vec![(0usize, 0usize, order.len())];
    while let Some((n, a, b)) = stack.pop() {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut clo = [f32::MAX; 3];
        let mut chi = [f32::MIN; 3];
        for &t in &order[a..b] {
            for &v in &s.tris[t as usize] {
                for k in 0..3 {
                    lo[k] = lo[k].min(s.verts[v as usize][k]);
                    hi[k] = hi[k].max(s.verts[v as usize][k]);
                }
            }
            for k in 0..3 {
                clo[k] = clo[k].min(cent[t as usize][k]);
                chi[k] = chi[k].max(cent[t as usize][k]);
            }
        }
        nodes[n].lo = lo;
        nodes[n].hi = hi;
        if b - a <= 4 {
            nodes[n].first = a as u32;
            nodes[n].count = (b - a) as u32;
            continue;
        }
        let axis = (0..3).max_by(|&i, &j| (chi[i] - clo[i]).total_cmp(&(chi[j] - clo[j]))).unwrap();
        let mid = (a + b) / 2;
        order[a..b].select_nth_unstable_by(mid - a, |&x, &y| cent[x as usize][axis].total_cmp(&cent[y as usize][axis]));
        let left = nodes.len();
        nodes.push(nodes[n]);
        nodes.push(nodes[n]);
        nodes[n].first = left as u32;
        nodes[n].count = 0;
        stack.push((left, a, mid));
        stack.push((left + 1, mid, b));
    }
    Bvh { nodes, order }
}

fn tri(s: &Scene, t: u32) -> [V3; 3] {
    s.tris[t as usize].map(|v| s.verts[v as usize].map(|x| x as f64))
}

/// Moller-Trumbore, both faces; the hit distance along a unit `d`.
fn hit(o: V3, d: V3, v: &[V3; 3]) -> Option<f64> {
    let e1 = sub(v[1], v[0]);
    let e2 = sub(v[2], v[0]);
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-30 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(o, v[0]);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let w = dot(d, q) * inv;
    if w < 0.0 || u + w > 1.0 {
        return None;
    }
    Some(dot(e2, q) * inv)
}

fn slab(o: V3, inv: V3, n: &Node, tmax: f64) -> bool {
    let mut t0: f64 = 0.0;
    let mut t1 = tmax;
    for k in 0..3 {
        let a = (n.lo[k] as f64 - o[k]) * inv[k];
        let b = (n.hi[k] as f64 - o[k]) * inv[k];
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    t0 <= t1
}

/// Closest primary hit: distance and triangle.
fn primary(s: &Scene, bvh: &Bvh, o: V3, d: V3) -> Option<(f64, u32)> {
    let inv = d.map(|x| 1.0 / x);
    let mut best: Option<(f64, u32)> = None;
    let mut stack = vec![0u32];
    while let Some(n) = stack.pop() {
        let node = &bvh.nodes[n as usize];
        if !slab(o, inv, node, best.map_or(f64::MAX, |b| b.0)) {
            continue;
        }
        if node.count == 0 {
            stack.push(node.first);
            stack.push(node.first + 1);
            continue;
        }
        for &t in &bvh.order[node.first as usize..(node.first + node.count) as usize] {
            if let Some(h) = hit(o, d, &tri(s, t)) {
                if h > 1e-6 && best.map_or(true, |b| h < b.0) {
                    best = Some((h, t));
                }
            }
        }
    }
    best
}

/// The conservative cone cull of a light-frame box: some point above `h0 + eps` whose q lies
/// within `slope * (h - h0)` of the receivers' q box [qlo, qhi].
fn cone_box(lo: [f64; 3], hi: [f64; 3], qlo: [f64; 2], qhi: [f64; 2], h0: f64, eps: f64, slope: f64) -> bool {
    let rise = hi[2] - h0;
    if rise < eps {
        return false;
    }
    let gx = (lo[0] - qhi[0]).max(qlo[0] - hi[0]).max(0.0);
    let gy = (lo[1] - qhi[1]).max(qlo[1] - hi[1]).max(0.0);
    let reach = slope * rise * (1.0 + 1e-6) + 1e-6;
    gx * gx + gy * gy <= reach * reach
}

fn tri_box(v: &[V3; 3]) -> ([f64; 3], [f64; 3]) {
    let lo = std::array::from_fn(|k| v[0][k].min(v[1][k]).min(v[2][k]));
    let hi = std::array::from_fn(|k| v[0][k].max(v[1][k]).max(v[2][k]));
    (lo, hi)
}

/// Variant S4-G: casters binned into a uniform light-space grid (cell `c` across the sun), each
/// cell with its triangle list and highest point. Triangles covering more than `GRID_WIDE`
/// cells go to one list every receiver tests.
const GRID_WIDE: usize = 4096;
struct Grid {
    lo: [f64; 2],
    c: f64,
    dims: [usize; 2],
    start: Vec<u32>,
    items: Vec<u32>,
    top: Vec<f32>,
    wide: Vec<u32>,
}

fn grid(s: &Scene, c: f64) -> Grid {
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    for v in &s.verts {
        for k in 0..2 {
            lo[k] = lo[k].min(v[k] as f64);
            hi[k] = hi[k].max(v[k] as f64);
        }
    }
    let dims = [((hi[0] - lo[0]) / c) as usize + 1, ((hi[1] - lo[1]) / c) as usize + 1];
    let span = |t: u32| {
        let (a, b) = tri_box(&tri(s, t));
        let x0 = ((a[0] - lo[0]) / c) as usize;
        let y0 = ((a[1] - lo[1]) / c) as usize;
        let x1 = (((b[0] - lo[0]) / c) as usize).min(dims[0] - 1);
        let y1 = (((b[1] - lo[1]) / c) as usize).min(dims[1] - 1);
        (x0, y0, x1, y1, b[2])
    };
    let mut count = vec![0u32; dims[0] * dims[1] + 1];
    let mut top = vec![f32::MIN; dims[0] * dims[1]];
    let mut wide = vec![];
    for t in 0..s.tris.len() as u32 {
        let (x0, y0, x1, y1, h) = span(t);
        if (x1 - x0 + 1) * (y1 - y0 + 1) > GRID_WIDE {
            wide.push(t);
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                count[y * dims[0] + x] += 1;
                top[y * dims[0] + x] = top[y * dims[0] + x].max(h as f32);
            }
        }
    }
    let mut start = vec![0u32; count.len()];
    let mut acc = 0u32;
    for (i, n) in count.iter().enumerate() {
        start[i] = acc;
        acc += n;
    }
    let mut items = vec![0u32; acc as usize];
    let mut fill = start.clone();
    for t in 0..s.tris.len() as u32 {
        let (x0, y0, x1, y1, _) = span(t);
        if (x1 - x0 + 1) * (y1 - y0 + 1) > GRID_WIDE {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                items[fill[y * dims[0] + x] as usize] = t;
                fill[y * dims[0] + x] += 1;
            }
        }
    }
    Grid { lo, c, dims, start, items, top, wide }
}

/// S4-G per receiver: cells whose square lies within the cone's reach at the cell's top are read;
/// their lists (and the wide list) are culled. Returns the unique pair list.
fn grid_pairs(s: &Scene, g: &Grid, htop: f64, r: &Receiver, slope: f64, st: &mut Stats) -> Vec<u32> {
    let q = [r.o[0], r.o[1]];
    let reach = slope * (htop - r.o[2]).max(0.0) * (1.0 + 1e-6) + 1e-6;
    let mut out = vec![];
    let test = |t: u32, st: &mut Stats, out: &mut Vec<u32>| {
        st.g_cands += 1;
        let (lo, hi) = tri_box(&tri(s, t));
        if cone_box(lo, hi, q, q, r.o[2], r.eps, slope) {
            out.push(t);
        }
    };
    for &t in &g.wide {
        test(t, st, &mut out);
    }
    let cell = |v: f64, k: usize| (((v - g.lo[k]) / g.c).floor().max(0.0) as usize).min(g.dims[k] - 1);
    for y in cell(q[1] - reach, 1)..=cell(q[1] + reach, 1) {
        for x in cell(q[0] - reach, 0)..=cell(q[0] + reach, 0) {
            st.g_cells += 1;
            let i = y * g.dims[0] + x;
            let lo = [g.lo[0] + x as f64 * g.c, g.lo[1] + y as f64 * g.c, 0.0];
            let hi = [lo[0] + g.c, lo[1] + g.c, g.top[i] as f64];
            if g.start[i + 1] == g.start[i] || !cone_box(lo, hi, q, q, r.o[2], r.eps, slope) {
                continue;
            }
            st.g_open += 1;
            for &t in &g.items[g.start[i] as usize..g.start[i + 1] as usize] {
                test(t, st, &mut out);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

struct Receiver {
    o: V3,
    eps: f64,
    dirs: Vec<V3>,
}

#[derive(Default, Clone)]
struct Stats {
    receivers: u64,
    tiles: u64,
    tiles_with_receivers: u64,
    nodes: u64,
    culls: u64,
    pairs: u64,
    zero: u64,
    full: u64,
    partial: u64,
    umbra: u64,
    penumbra: u64,
    lit: u64,
    pairs_umbra: u64,
    pairs_penumbra: u64,
    pairs_lit: u64,
    tile_nodes: u64,
    tile_cands: u64,
    tile_tests: u64,
    nodes4: u64,
    hybrid: u64,
    g_cells: u64,
    g_open: u64,
    g_cands: u64,
    g_mismatch: u64,
    per_receiver_pairs: Vec<u32>,
    per_receiver_gcands: Vec<u32>,
    per_receiver_loads: Vec<u32>,
    per_tile_cands: Vec<u32>,
}

/// One per-receiver cone traversal (S4-R): visited nodes, leaf culls and the pair list.
fn cone_pairs(s: &Scene, bvh: &Bvh, r: &Receiver, slope: f64, st: &mut Stats) -> Vec<u32> {
    let q = [r.o[0], r.o[1]];
    let mut out = vec![];
    // (node, depth). A 4-wide BVH fetches one record per passing inner node at even depth.
    let mut stack = vec![(0u32, 0u32)];
    while let Some((n, depth)) = stack.pop() {
        st.nodes += 1;
        let node = &bvh.nodes[n as usize];
        if !cone_box(node.lo.map(|x| x as f64), node.hi.map(|x| x as f64), q, q, r.o[2], r.eps, slope) {
            continue;
        }
        if node.count == 0 {
            if depth % 2 == 0 {
                st.nodes4 += 1;
            }
            stack.push((node.first, depth + 1));
            stack.push((node.first + 1, depth + 1));
            continue;
        }
        for &t in &bvh.order[node.first as usize..(node.first + node.count) as usize] {
            st.culls += 1;
            let (lo, hi) = tri_box(&tri(s, t));
            if cone_box(lo, hi, q, q, r.o[2], r.eps, slope) {
                out.push(t);
            }
        }
    }
    out
}

fn hash(mut x: u32) -> u32 {
    x = (x ^ (x >> 16)).wrapping_mul(0x7feb352d);
    x = (x ^ (x >> 15)).wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}

struct Camera {
    eye: V3,
    fwd: V3,
    right: V3,
    up: V3,
    tan: f64,
    size: [u32; 2],
}

fn run_tiles(s: &Scene, bvh: &Bvh, g: &Grid, htop: f64, cam: &Camera, slope: f64, tiles: &[[u32; 2]]) -> Stats {
    let mut st = Stats::default();
    let aspect = cam.size[0] as f64 / cam.size[1] as f64;
    for &[tx, ty] in tiles {
        st.tiles += 1;
        let mut rs = vec![];
        for py in ty * 4..(ty * 4 + 4).min(cam.size[1]) {
            for px in tx * 8..(tx * 8 + 8).min(cam.size[0]) {
                let x = (2.0 * (px as f64 + 0.5) / cam.size[0] as f64 - 1.0) * cam.tan * aspect;
                let y = (1.0 - 2.0 * (py as f64 + 0.5) / cam.size[1] as f64) * cam.tan;
                let d = norm(add(cam.fwd, add(scale(cam.right, x), scale(cam.up, y))));
                let Some((t, id)) = primary(s, bvh, cam.eye, d) else { continue };
                let p = add(cam.eye, scale(d, t));
                let v = tri(s, id);
                let mut n = norm(cross(sub(v[1], v[0]), sub(v[2], v[0])));
                if dot(n, d) > 0.0 {
                    n = scale(n, -1.0);
                }
                // A metre-scale bias with a distance term, like the renderer's receiver epsilon.
                let eps = 2e-4 + 2e-5 * t;
                let o = add(p, scale(n, eps));
                // The pixel's 256 sequence directions (vsm_direction) in the sun frame (l = z).
                let seed = hash(px * 73856093 ^ py * 19349663);
                let shift = [(seed >> 8) as f64 / 16777216.0, (hash(seed ^ 0xa511e9b3) >> 8) as f64 / 16777216.0];
                let dirs = (0..256u32)
                    .map(|i| {
                        let rx = (i as f64 * 0.7548776662466927 + shift[0]).fract();
                        let ry = (i as f64 * 0.5698402909980532 + shift[1]).fract();
                        let a = std::f64::consts::TAU * ry;
                        norm([slope * rx.sqrt() * a.cos(), slope * rx.sqrt() * a.sin(), 1.0])
                    })
                    .collect();
                rs.push(Receiver { o, eps, dirs });
            }
        }
        if rs.is_empty() {
            st.per_tile_cands.push(0);
            continue;
        }
        st.tiles_with_receivers += 1;
        // S4-T: the tile's union cull, one traversal.
        let mut qlo = [f64::MAX; 2];
        let mut qhi = [f64::MIN; 2];
        let mut h0 = f64::MAX;
        let mut eps = f64::MAX;
        for r in &rs {
            for k in 0..2 {
                qlo[k] = qlo[k].min(r.o[k]);
                qhi[k] = qhi[k].max(r.o[k]);
            }
            h0 = h0.min(r.o[2]);
            eps = eps.min(r.eps);
        }
        let mut cands = 0u64;
        let mut tile_nodes_t = 0u64;
        let mut stack = vec![0u32];
        while let Some(n) = stack.pop() {
            st.tile_nodes += 1;
            tile_nodes_t += 1;
            let node = &bvh.nodes[n as usize];
            if !cone_box(node.lo.map(|x| x as f64), node.hi.map(|x| x as f64), qlo, qhi, h0, eps, slope) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first);
                stack.push(node.first + 1);
                continue;
            }
            for &t in &bvh.order[node.first as usize..(node.first + node.count) as usize] {
                let (lo, hi) = tri_box(&tri(s, t));
                if cone_box(lo, hi, qlo, qhi, h0, eps, slope) {
                    cands += 1;
                }
            }
        }
        st.tile_cands += cands;
        st.tile_tests += cands * rs.len() as u64;
        st.per_tile_cands.push(cands as u32);
        // S4-R per receiver, and the direction verdicts of every pair.
        let r_before = st.nodes + st.culls;
        for r in &rs {
            let loads_before = st.nodes + st.culls;
            st.receivers += 1;
            let mut pairs = cone_pairs(s, bvh, r, slope, &mut st);
            let before = st.g_cands;
            let gp = grid_pairs(s, g, htop, r, slope, &mut st);
            st.per_receiver_gcands.push((st.g_cands - before) as u32);
            pairs.sort_unstable();
            // The two searches must find the same pairs (a check of the replay itself).
            if gp != pairs {
                st.g_mismatch += 1;
            }
            st.pairs += pairs.len() as u64;
            st.per_receiver_pairs.push(pairs.len() as u32);
            st.per_receiver_loads.push((st.nodes + st.culls - loads_before) as u32);
            let mut blocked = [0u64; 4];
            for &t in &pairs {
                let v = tri(s, t);
                let mut hits = 0;
                for (j, &d) in r.dirs.iter().enumerate() {
                    if hit(r.o, d, &v).is_some_and(|h| h >= r.eps) {
                        hits += 1;
                        blocked[j / 64] |= 1 << (j % 64);
                    }
                }
                match hits {
                    0 => st.zero += 1,
                    256 => st.full += 1,
                    _ => st.partial += 1,
                }
            }
            let visible = 256 - blocked.iter().map(|b| b.count_ones()).sum::<u32>();
            let n = pairs.len() as u64;
            match visible {
                0 => {
                    st.umbra += 1;
                    st.pairs_umbra += n;
                }
                256 => {
                    st.lit += 1;
                    st.pairs_lit += n;
                }
                _ => {
                    st.penumbra += 1;
                    st.pairs_penumbra += n;
                }
            }
        }
        // Hybrid: per tile, the cheaper of the tile gather (cooperative traversal, then one
        // broadcast load per candidate; the lanes' culls are register work, 1/32 of a load each)
        // and per-receiver traversals (their loads summed).
        let gather = tile_nodes_t + cands + cands * rs.len() as u64 / 32;
        st.hybrid += gather.min(st.nodes + st.culls - r_before);
    }
    st
}

fn parse(v: &str) -> Vec<f64> {
    v.split(',').map(|x| x.trim().parse().unwrap()).collect()
}

fn pct(v: &mut [u32], p: &[f64]) -> String {
    v.sort_unstable();
    let items: Vec<String> = p
        .iter()
        .map(|&q| format!("\"p{}\": {}", q, v.get(((v.len() as f64 - 1.0) * q / 100.0).round() as usize).copied().unwrap_or(0)))
        .collect();
    format!("{{{}}}", items.join(", "))
}

fn main() {
    let args: HashMap<String, String> = std::env::args()
        .skip(1)
        .filter_map(|a| a.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    let get = |k: &str| args.get(k).unwrap_or_else(|| panic!("missing {k}=")).clone();
    let l = norm(parse(&get("light")).try_into().unwrap());
    // The sun frame: u, v across the sun, l toward it.
    let helper = if l[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let u = norm(cross(helper, l));
    let v = cross(l, u);
    let frame = [u, v, l];
    let to_frame = |w: V3| [dot(w, u), dot(w, v), dot(w, l)];
    let slope = get("radius").parse::<f64>().unwrap().tan();
    let size = parse(&get("size"));
    let size = [size[0] as u32, size[1] as u32];
    let eye: V3 = parse(&get("eye")).try_into().unwrap();
    let target: V3 = parse(&get("target")).try_into().unwrap();
    let fwd = norm(sub(target, eye));
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    let cam = Camera {
        eye: to_frame(eye),
        fwd: to_frame(fwd),
        right: to_frame(right),
        up: to_frame(up),
        tan: (get("fov").parse::<f64>().unwrap() * 0.5).tan(),
        size,
    };
    let step: u32 = get("tile_step").parse().unwrap();
    let threads: usize = get("threads").parse().unwrap();

    let t0 = std::time::Instant::now();
    let scene = load(&get("rla"), frame);
    let t_load = t0.elapsed().as_secs_f64();
    let bvh = build(&scene);
    let t_build = t0.elapsed().as_secs_f64() - t_load;
    let cell: f64 = get("cell").parse().unwrap();
    let g = grid(&scene, cell);
    let htop = scene.verts.iter().map(|v| v[2] as f64).fold(f64::MIN, f64::max);
    eprintln!("grid {}x{} cells of {cell} m, {} entries, {} wide", g.dims[0], g.dims[1], g.items.len(), g.wide.len());
    eprintln!("{} triangles, {} vertices, {} nodes; load {t_load:.1}s build {t_build:.1}s", scene.tris.len(), scene.verts.len(), bvh.nodes.len());

    // pgm=<path>: a coverage check, primary-hit distance per pixel (black: no hit).
    if let Some(path) = args.get("pgm") {
        let aspect = size[0] as f64 / size[1] as f64;
        let mut img = format!("P2\n{} {}\n255\n", size[0], size[1]);
        for py in 0..size[1] {
            for px in 0..size[0] {
                let x = (2.0 * (px as f64 + 0.5) / size[0] as f64 - 1.0) * cam.tan * aspect;
                let y = (1.0 - 2.0 * (py as f64 + 0.5) / size[1] as f64) * cam.tan;
                let d = norm(add(cam.fwd, add(scale(cam.right, x), scale(cam.up, y))));
                let g = primary(&scene, &bvh, cam.eye, d).map_or(0, |(t, _)| (255.0 - 20.0 * t).clamp(16.0, 255.0) as u32);
                img += &format!("{g} ");
            }
            img += "\n";
        }
        std::fs::write(path, img).unwrap();
        return;
    }
    let grid = [size[0].div_ceil(8), size[1].div_ceil(4)];
    let tiles: Vec<[u32; 2]> = (0..grid[1])
        .step_by(step as usize)
        .flat_map(|y| (0..grid[0]).step_by(step as usize).map(move |x| [x + step / 2, y + step / 2]))
        .filter(|t| t[0] < grid[0] && t[1] < grid[1])
        .collect();
    let chunk = tiles.len().div_ceil(threads);
    let parts: Vec<Stats> = std::thread::scope(|sc| {
        let hs: Vec<_> = tiles.chunks(chunk).map(|c| sc.spawn(|| run_tiles(&scene, &bvh, &g, htop, &cam, slope, c))).collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut st = Stats::default();
    for p in parts {
        macro_rules! sum {
            ($($f:ident),*) => {$(st.$f += p.$f;)*};
        }
        sum!(receivers, tiles, tiles_with_receivers, nodes, culls, pairs, zero, full, partial, umbra, penumbra, lit, pairs_umbra, pairs_penumbra, pairs_lit, tile_nodes, tile_cands, tile_tests, nodes4, hybrid, g_cells, g_open, g_cands, g_mismatch);
        st.per_receiver_pairs.extend(p.per_receiver_pairs);
        st.per_receiver_loads.extend(p.per_receiver_loads);
        st.per_receiver_gcands.extend(p.per_receiver_gcands);
        st.per_tile_cands.extend(p.per_tile_cands);
    }
    let k = (step * step) as u64;
    // Pairs carried by the top fraction of receivers by pair count (bound for a queued subset).
    let mut sorted = st.per_receiver_pairs.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let top = |frac: f64| -> u64 { sorted.iter().take((sorted.len() as f64 * frac).round() as usize).map(|&x| x as u64).sum::<u64>() * k };
    let mut json = String::from("{\n");
    json += &format!("  \"triangles\": {}, \"bvh_nodes\": {}, \"tile_step\": {}, \"scale\": {},\n", scene.tris.len(), bvh.nodes.len(), step, k);
    json += &format!("  \"slope\": {slope}, \"seconds\": {:.1},\n", t0.elapsed().as_secs_f64());
    macro_rules! out {
        ($($f:ident),*) => {$(json += &format!("  \"{}\": {},\n", stringify!($f), st.$f * k);)*};
    }
    out!(receivers, tiles, tiles_with_receivers, nodes, culls, pairs, zero, full, partial, umbra, penumbra, lit, pairs_umbra, pairs_penumbra, pairs_lit, tile_nodes, tile_cands, tile_tests, nodes4, hybrid, g_cells, g_open, g_cands, g_mismatch);
    json += &format!("  \"grid_cell\": {cell}, \"grid_dims\": [{}, {}], \"grid_entries\": {}, \"grid_wide\": {},
", g.dims[0], g.dims[1], g.items.len(), g.wide.len());
    json += &format!("  \"per_receiver_gcands\": {},
", pct(&mut st.per_receiver_gcands, &[50.0, 90.0, 99.0, 99.9, 100.0]));
    let mut loads = st.per_receiver_loads.clone();
    loads.sort_unstable_by(|a, b| b.cmp(a));
    for f in [0.05, 0.1, 0.25, 0.5] {
        json += &format!("  \"pairs_top_{}\": {},\n", f, top(f));
        let sum: u64 = loads.iter().take((loads.len() as f64 * f).round() as usize).map(|&x| x as u64).sum();
        json += &format!("  \"loads_top_{}\": {},\n", f, sum * k);
    }
    json += &format!("  \"per_receiver_pairs\": {},\n", pct(&mut st.per_receiver_pairs, &[50.0, 90.0, 99.0, 99.9, 100.0]));
    json += &format!("  \"per_tile_cands\": {}\n}}\n", pct(&mut st.per_tile_cands, &[50.0, 90.0, 99.0, 99.9, 100.0]));
    std::fs::File::create(get("out")).unwrap().write_all(json.as_bytes()).unwrap();
    print!("{json}");
}
