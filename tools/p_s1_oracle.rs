//! P-S1 CPU oracle for the umbra certificate (`vsm_umbra`; docs/p-s1.md). S1 was rejected at its
//! gate, so the shader function lives in the unit's evidence patch (`p-s1/s1-umbra.patch`), not
//! in shaders/virtual_shadow.wgsl. Kept for any later certificate (design S4) to reuse.
//!
//! A false certificate would add shadow, so every certified case must block every direction.
//! The oracle ports the certificate and the watertight test (`vsm_hits_sheared`) to f32. For
//! each case it builds a sun, a shared origin and a triangle: synthetic fixtures (a box face
//! over ground, scan-sized triangles, far world offsets, grazing suns) and adversarial
//! triangles whose nearest edge passes within a hair of the cone. Each certified case must then:
//!   1. hit under the f32 watertight test for all 256 sequence directions at several pixel
//!      shifts, and for dense samples of the disk and its rim, with the directions and shears
//!      perturbed by GPU-scale rounding and the vertices re-transformed in another rounding
//!      order (another call site);
//!   2. pass an exact f64 cone-in-triangle test.
//! It also reports how often a clearly contained cone certifies (the certificate's
//! conservativeness).
//!
//! Standalone, no dependencies: `rustc -O --edition 2021 tools/p_s1_oracle.rs -o p_s1_oracle.exe`.
//! Exits 1 on any false certificate.

type V3 = [f32; 3];
type D3 = [f64; 3];

/// xorshift64*: deterministic cases.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn sphere(&mut self) -> D3 {
        loop {
            let v = [self.range(-1., 1.), self.range(-1., 1.), self.range(-1., 1.)];
            let n = dot64(v, v);
            if n > 1e-4 && n <= 1. {
                return scale64(v, 1. / n.sqrt());
            }
        }
    }
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn normalize(a: V3) -> V3 {
    scale(a, 1. / dot(a, a).sqrt())
}
fn dot64(a: D3, b: D3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub64(a: D3, b: D3) -> D3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add64(a: D3, b: D3) -> D3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale64(a: D3, s: f64) -> D3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn cross64(a: D3, b: D3) -> D3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn f32s(a: D3) -> V3 {
    [a[0] as f32, a[1] as f32, a[2] as f32]
}
fn f64s(a: V3) -> D3 {
    [a[0] as f64, a[1] as f64, a[2] as f64]
}

/// The per-pixel ray frame (vsm_prepare_rays): the sun's dominant-axis permutation and the
/// central shear.
struct Frame {
    axes: [usize; 3],
    centre: V3,
}
impl Frame {
    fn new(l: V3) -> Self {
        let a = [l[0].abs(), l[1].abs(), l[2].abs()];
        let mut z = 2;
        if a[0] >= a[1] && a[0] >= a[2] {
            z = 0;
        } else if a[1] >= a[2] {
            z = 1;
        }
        let (mut x, mut y) = ((z + 1) % 3, (z + 2) % 3);
        if l[z] < 0. {
            std::mem::swap(&mut x, &mut y);
        }
        let axes = [x, y, z];
        let c = permute(axes, l);
        Frame { axes, centre: [c[0] / c[2], c[1] / c[2], 1. / c[2]] }
    }
    fn shear(&self, d: V3) -> V3 {
        let d = permute(self.axes, d);
        [d[0] / d[2], d[1] / d[2], 1. / d[2]]
    }
}
fn permute(axes: [usize; 3], v: V3) -> V3 {
    [v[axes[0]], v[axes[1]], v[axes[2]]]
}

fn fma(a: f32, b: f32, c: f32) -> f32 {
    a.mul_add(b, c)
}
/// vsm_difference: a*b - c*d with one rounding each way.
fn difference(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let w = c * d;
    fma(a, b, -w) + fma(-c, d, w)
}
/// vsm_hits_sheared, line for line.
fn hits_sheared(s: V3, a: V3, b: V3, c: V3, tmin: f32) -> bool {
    let ax = fma(-s[0], a[2], a[0]);
    let ay = fma(-s[1], a[2], a[1]);
    let bx = fma(-s[0], b[2], b[0]);
    let by = fma(-s[1], b[2], b[1]);
    let cx = fma(-s[0], c[2], c[0]);
    let cy = fma(-s[1], c[2], c[1]);
    let u = difference(cx, by, cy, bx);
    let v = difference(ax, cy, ay, cx);
    let w = difference(bx, ay, by, ax);
    if (u < 0. || v < 0. || w < 0.) && (u > 0. || v > 0. || w > 0.) {
        return false;
    }
    let det = u + v + w;
    if det == 0. {
        return false;
    }
    let t = fma(u, s[2] * a[2], fma(v, s[2] * b[2], w * (s[2] * c[2]))) / det;
    t >= tmin
}

/// vsm_direction.
fn direction(index: u32, shift: [f32; 2], l: V3, u: V3, v: V3, slope: f32) -> V3 {
    let f = index as f32;
    let fract = |x: f32| x - x.floor();
    let r = [fract(f * 0.754_877_7 + shift[0]), fract(f * 0.569_840_3 + shift[1])];
    let a = std::f32::consts::TAU * r[1];
    let w = add(scale(u, a.cos()), scale(v, a.sin()));
    normalize(add(l, scale(w, slope * r[0].sqrt())))
}

/// The pixel's sequence state (vsm_seq_*) and the receiver.
struct Pixel {
    l: V3,
    u: V3,
    v: V3,
    slope: f32,
    origin: V3,
    epsilon: f32,
    frame: Frame,
}

/// vsm_umbra, line for line (after the triangle's world vertices; `translation` is m[3].xyz).
fn umbra(p: &Pixel, a: V3, b: V3, c: V3, translation: V3) -> bool {
    let o = p.origin;
    let axes = p.frame.axes;
    let l = permute(axes, p.l);
    let lz = l[2].abs();
    let (u, w) = (p.u, p.v);
    let wmax = p.slope * (dot(u, u).max(dot(w, w)) + dot(u, w).abs()).sqrt() * (1. + 1. / 1024.) + 1e-6;
    if !(wmax < 0.5 * lz) {
        return false;
    }
    let lxy = (l[0] * l[0] + l[1] * l[1]).sqrt();
    let ln = dot(l, l).sqrt();
    let along = p.slope * (dot(u, p.l).abs() + dot(w, p.l).abs()) / dot(p.l, p.l).sqrt() * (1. + 1. / 1024.) + 1e-6;
    let radius = (wmax * ln + along * (lz + lxy)) / ((lz - wmax) * lz) * (1. + 1. / 1024.) + 3.8e-6;
    let pa = permute(axes, sub(a, o));
    let pb = permute(axes, sub(b, o));
    let pc = permute(axes, sub(c, o));
    let s = p.frame.centre;
    let q = |p: V3| [fma(-s[0], p[2], p[0]), fma(-s[1], p[2], p[1])];
    let (qa, qb, qc) = (q(pa), q(pb), q(pc));
    let toward = if s[2] > 0. { 1. } else if s[2] < 0. { -1. } else { 0. };
    let mut world = 0f32;
    for v in [a, b, c, o, translation] {
        for x in v {
            world = world.max(x.abs());
        }
    }
    let ew = world * 9.536_743e-7;
    let lowest = (pa[2] * toward).min((pb[2] * toward).min(pc[2] * toward));
    let len = |v: V3| dot(v, v).sqrt();
    let (e1, e2) = (sub(pb, pa), sub(pc, pa));
    let normal = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
    let area = len(normal);
    let far = len(pa).max(len(pb).max(len(pc)));
    let tilt = 4. * (far * 2.384_185_8e-7 + ew) * (len(e1) + len(e2)) / area;
    let facing = dot(normal, l) / (area * len(l));
    let dist = dot(normal, pa) / area;
    let plane = area > 0.
        && facing.abs() > 1.01 * wmax / len(l) + tilt + 1e-6
        && facing * dist > 0.
        && dist.abs() - far * tilt - 2. * ew - 1e-5 * far > 4. * p.epsilon;
    if !(lowest * (1. - 1e-6) - 2. * ew > 4. * p.epsilon || plane) {
        return false;
    }
    let x = |q: [f32; 2], h: f32| q[0].abs() + q[1].abs() + 2. * radius * h.abs();
    let (xa, xb, xc) = (x(qa, pa[2]), x(qb, pb[2]), x(qc, pc[2]));
    let e = |x: f32| x * 2.384_185_8e-7 + 3. * ew;
    let (ea, eb, ec) = (e(xa), e(xb), e(xc));
    let e0 = [
        qc[0] * qb[1] - qc[1] * qb[0],
        qa[0] * qc[1] - qa[1] * qc[0],
        qb[0] * qa[1] - qb[1] * qa[0],
    ];
    let g = |h1: f32, q1: [f32; 2], h2: f32, q2: [f32; 2]| {
        let v = [h1 * q1[0] - h2 * q2[0], h1 * q1[1] - h2 * q2[1]];
        (v[0] * v[0] + v[1] * v[1]).sqrt()
    };
    let gu = g(pb[2], qc, pc[2], qb);
    let gv = g(pc[2], qa, pa[2], qc);
    let gw = g(pa[2], qb, pb[2], qa);
    let k = 9.536_743e-7;
    let slack = [
        4. * (ec * xb + eb * xc) + k * (xb * xc + radius * (pb[2].abs() * xc + pc[2].abs() * xb)),
        4. * (ea * xc + ec * xa) + k * (xa * xc + radius * (pc[2].abs() * xa + pa[2].abs() * xc)),
        4. * (eb * xa + ea * xb) + k * (xa * xb + radius * (pa[2].abs() * xb + pb[2].abs() * xa)),
    ];
    let side = if e0[0] > 0. { 1. } else { -1. };
    let least = [side * e0[0] - radius * gu, side * e0[1] - radius * gv, side * e0[2] - radius * gw];
    (0..3).all(|i| least[i] > slack[i])
}

/// Exact (f64) containment of the ideal cone {l + w : |w| in the disk} in the triangle seen
/// from the origin, with every vertex beyond epsilon toward the light.
fn exact_contains(p: &Pixel, a: D3, b: D3, c: D3) -> bool {
    let o = f64s(p.origin);
    let (l, u, v) = (f64s(p.l), f64s(p.u), f64s(p.v));
    let (pa, pb, pc) = (sub64(a, o), sub64(b, o), sub64(c, o));
    let orient = dot64(cross64(pa, pb), pc);
    if orient == 0. {
        return false;
    }
    for (x, y) in [(pa, pb), (pb, pc), (pc, pa)] {
        let n = scale64(cross64(x, y), orient.signum());
        let least = dot64(n, l) - p.slope as f64 * (dot64(n, u).powi(2) + dot64(n, v).powi(2)).sqrt();
        if least <= 0. {
            return false;
        }
    }
    // The plane is in front: the cone's rays reach it at positive t beyond epsilon.
    let normal = cross64(sub64(pb, pa), sub64(pc, pa));
    let dist = dot64(normal, pa) / dot64(normal, normal).sqrt();
    dist.abs() > p.epsilon as f64 && dot64(normal, pa).signum() == dot64(normal, l).signum()
}

/// A model transform: rotation about y, uniform scale and a translation, applied in two rounding
/// orders (the certificate's call site and another walk's).
struct Model {
    m: [[f32; 4]; 4],
}
impl Model {
    fn new(angle: f64, scale: f64, t: D3) -> Self {
        let (s, c) = angle.sin_cos();
        let s = s * scale;
        let c = c * scale;
        Model {
            m: [
                [c as f32, 0., -s as f32, 0.],
                [0., scale as f32, 0., 0.],
                [s as f32, 0., c as f32, 0.],
                [t[0] as f32, t[1] as f32, t[2] as f32, 1.],
            ],
        }
    }
    fn inverse(&self, w: D3) -> V3 {
        let m = self.m;
        let t = [m[3][0] as f64, m[3][1] as f64, m[3][2] as f64];
        let d = sub64(w, t);
        let (c, s) = (m[0][0] as f64, m[2][0] as f64);
        let k = c * c + s * s;
        let y = m[1][1] as f64;
        f32s([(c * d[0] - s * d[2]) / k, d[1] / y, (s * d[0] + c * d[2]) / k])
    }
    fn apply(&self, v: V3, order: bool) -> V3 {
        let m = self.m;
        let mut out = [0f32; 3];
        for i in 0..3 {
            out[i] = if order {
                fma(m[0][i], v[0], fma(m[1][i], v[1], fma(m[2][i], v[2], m[3][i])))
            } else {
                ((m[0][i] * v[0] + m[1][i] * v[1]) + m[2][i] * v[2]) + m[3][i]
            };
        }
        out
    }
    fn exact(&self, v: V3) -> D3 {
        let m = self.m;
        let v = f64s(v);
        let mut out = [0f64; 3];
        for i in 0..3 {
            out[i] = m[0][i] as f64 * v[0] + m[1][i] as f64 * v[1] + m[2][i] as f64 * v[2] + m[3][i] as f64;
        }
        out
    }
}

/// A sun basis as the shadow view gives it: l, then u and v normalized from f32 cross products.
fn basis(l64: D3) -> (V3, V3, V3) {
    let l = f32s(l64);
    let up = if l64[1].abs() < 0.9 { [0., 1., 0.] } else { [1., 0., 0.] };
    let u = normalize(f32s(cross64(up, l64)));
    let v = normalize(f32s(cross64(l64, f64s(u))));
    (l, u, v)
}

#[derive(Default)]
struct Tally {
    cases: u64,
    certified: u64,
    contained: u64,
    contained_certified: u64,
    false_hits: u64,
    false_exact: u64,
}

/// Every check on one case; returns whether it certified.
fn check(rng: &mut Rng, p: &Pixel, model: &Model, local: [V3; 3], tally: &mut Tally, name: &str) {
    tally.cases += 1;
    let world = [model.apply(local[0], true), model.apply(local[1], true), model.apply(local[2], true)];
    let translation = [model.m[3][0], model.m[3][1], model.m[3][2]];
    let exact = [model.exact(local[0]), model.exact(local[1]), model.exact(local[2])];
    let contained = exact_contains(p, exact[0], exact[1], exact[2]);
    let certified = umbra(p, world[0], world[1], world[2], translation);
    if contained {
        tally.contained += 1;
        tally.contained_certified += certified as u64;
    }
    if !certified {
        return;
    }
    tally.certified += 1;
    if !contained {
        tally.false_exact += 1;
        eprintln!("FALSE (exact f64) {name}: {:?} origin {:?} l {:?} slope {}", exact, p.origin, p.l, p.slope);
    }
    // The other call site's vertices, relative to the origin, permuted.
    let other = [model.apply(local[0], false), model.apply(local[1], false), model.apply(local[2], false)];
    let rel = |w: V3| permute(p.frame.axes, sub(w, p.origin));
    let sets = [[rel(world[0]), rel(world[1]), rel(world[2])], [rel(other[0]), rel(other[1]), rel(other[2])]];
    let mut directions = Vec::with_capacity(1600);
    for _ in 0..4 {
        let shift = [rng.unit() as f32, rng.unit() as f32];
        for i in 0..256 {
            directions.push(direction(i, shift, p.l, p.u, p.v, p.slope));
        }
    }
    for k in 0..256 {
        // The rim (r.x just below 1) and the disk at random.
        let a = std::f32::consts::TAU * (k as f32 / 256.);
        let w = add(scale(p.u, a.cos()), scale(p.v, a.sin()));
        directions.push(normalize(add(p.l, scale(w, p.slope * (1. - f32::EPSILON)))));
        let rho = (rng.unit() as f32).sqrt() * p.slope;
        let b = rng.unit() as f32 * std::f32::consts::TAU;
        directions.push(normalize(add(p.l, scale(add(scale(p.u, b.cos()), scale(p.v, b.sin())), rho))));
    }
    for d in directions {
        for jitter in 0..3 {
            let mut d = d;
            if jitter > 0 {
                // GPU-scale rounding of cos/sin, sqrt, normalize: a few 1e-7 per component.
                for x in &mut d {
                    *x += (rng.range(-1., 1.) * 6e-7) as f32;
                }
            }
            let mut s = p.frame.shear(d);
            if jitter == 2 {
                // An approximate division (2 ulp).
                for x in &mut s {
                    *x *= 1. + (rng.range(-1., 1.) * 2.5e-7) as f32;
                }
            }
            for t in &sets {
                if !hits_sheared(s, t[0], t[1], t[2], p.epsilon) {
                    tally.false_hits += 1;
                    if tally.false_hits <= 10 {
                        eprintln!("FALSE (watertight miss) {name}: dir {:?} tri {:?} origin {:?}", d, t, p.origin);
                    }
                    return;
                }
            }
        }
    }
}

fn pixel(l64: D3, slope: f64, origin: D3, epsilon: f64, skew: f64, rng: &mut Rng) -> Pixel {
    let (l, mut u, v) = basis(l64);
    if skew > 0. {
        // A shadow view whose x row is not exactly perpendicular to the sun.
        u = normalize(add(u, f32s(scale64(rng.sphere(), skew))));
    }
    Pixel { l, u, v, slope: slope as f32, origin: f32s(origin), epsilon: epsilon as f32, frame: Frame::new(l) }
}

/// Ray (f64) against a triangle: t, or None.
fn ray_tri(o: D3, d: D3, a: D3, b: D3, c: D3) -> Option<f64> {
    let (e1, e2) = (sub64(b, a), sub64(c, a));
    let p = cross64(d, e2);
    let det = dot64(e1, p);
    if det.abs() < 1e-300 {
        return None;
    }
    let s = sub64(o, a);
    let u = dot64(s, p) / det;
    let q = cross64(s, e1);
    let v = dot64(d, q) / det;
    let t = dot64(e2, q) / det;
    (u >= 0. && v >= 0. && u + v <= 1. && t > 0.).then_some(t)
}

fn log_uniform(rng: &mut Rng, lo: f64, hi: f64) -> f64 {
    (lo.ln() + (hi.ln() - lo.ln()) * rng.unit()).exp()
}

/// Elevation 10-85 degrees at any azimuth; one case in eight anywhere on the sphere.
fn sun(rng: &mut Rng) -> D3 {
    if rng.next() % 8 == 0 {
        return rng.sphere();
    }
    let (e, a) = (rng.range(10f64, 85.).to_radians(), rng.range(0., std::f64::consts::TAU));
    [e.cos() * a.cos(), e.sin(), e.cos() * a.sin()]
}

fn main() {
    let mut rng = Rng(0x5eed_5151_d00d_beef);
    let mut tallies: Vec<(&str, Tally)> = Vec::new();
    // Sun slopes: the quarry/original game sun (0.00465), the courtyard/wide sun (0.04), wider.
    let slopes = [0.00465, 0.0175, 0.04, 0.08];
    let per = 40_000;

    // 1. Boxes on ground (the game scene's casters): a ground receiver in the box's shadow, and
    //    every box triangle the central ray crosses (the walk's candidates: entry and exit).
    let mut t = Tally::default();
    let quad = [[0usize, 1, 2], [0, 2, 3]];
    for i in 0..per {
        let slope = slopes[i % 4];
        let l64 = sun(&mut rng);
        if l64[1] < 0.2 {
            continue;
        }
        let size = [rng.range(0.2, 4.), rng.range(0.2, 4.), rng.range(0.2, 4.)];
        let model = Model::new(rng.range(0., 6.28), 1., [rng.range(-400., 400.), 0., rng.range(-400., 400.)]);
        let c = |x: usize, y: usize, z: usize| [x as f32 * size[0] as f32, y as f32 * size[1] as f32, z as f32 * size[2] as f32];
        let faces = [
            [c(0, 0, 0), c(1, 0, 0), c(1, 1, 0), c(0, 1, 0)],
            [c(0, 0, 1), c(0, 1, 1), c(1, 1, 1), c(1, 0, 1)],
            [c(0, 0, 0), c(0, 1, 0), c(0, 1, 1), c(0, 0, 1)],
            [c(1, 0, 0), c(1, 0, 1), c(1, 1, 1), c(1, 1, 0)],
            [c(0, 1, 0), c(1, 1, 0), c(1, 1, 1), c(0, 1, 1)],
        ];
        // A point inside the box, traced back along the sun to the ground.
        let inside = model.exact([rng.range(0., size[0]) as f32, rng.range(0., size[1]) as f32, rng.range(0., size[2]) as f32]);
        let ground = sub64(inside, scale64(l64, inside[1] / l64[1]));
        let eps = rng.range(1e-4, 2e-3);
        let origin = add64(ground, [0., eps, 0.]);
        let p = pixel(l64, slope, origin, eps, if i % 5 == 0 { 1e-3 } else { 0. }, &mut rng);
        let mut crossed: Vec<[V3; 3]> = Vec::new();
        for f in &faces {
            for q in &quad {
                let tri = [f[q[0]], f[q[1]], f[q[2]]];
                let w = [model.exact(tri[0]), model.exact(tri[1]), model.exact(tri[2])];
                if ray_tri(f64s(p.origin), f64s(p.l), w[0], w[1], w[2]).is_some() {
                    crossed.push(tri);
                }
            }
        }
        for tri in crossed {
            check(&mut rng, &p, &model, tri, &mut t, "box");
        }
    }
    tallies.push(("boxes on ground (crossed triangles)", t));

    // 2. Scan-sized triangles (1-30 mm, any orientation) over the central ray, at far offsets.
    let mut t = Tally::default();
    for i in 0..per {
        let slope = slopes[i % 4];
        let l64 = sun(&mut rng);
        let base = [rng.range(-800., 800.), rng.range(-20., 60.), rng.range(-800., 800.)];
        let p = pixel(l64, slope, base, rng.range(1e-5, 1e-3), 0., &mut rng);
        let size = rng.range(0.001, 0.03);
        let h = (size / slope * rng.range(0.02, 1.5)).max(0.002);
        let centre = add64(add64(f64s(p.origin), scale64(f64s(p.l), h)), scale64(rng.sphere(), size * 0.2));
        let tri = [add64(centre, scale64(rng.sphere(), size)), add64(centre, scale64(rng.sphere(), size)), add64(centre, scale64(rng.sphere(), size))];
        let model = Model::new(rng.range(0., 6.28), 1., [rng.range(-800., 800.), rng.range(-10., 10.), rng.range(-800., 800.)]);
        let local = [model.inverse(tri[0]), model.inverse(tri[1]), model.inverse(tri[2])];
        check(&mut rng, &p, &model, local, &mut t, "scan");
    }
    tallies.push(("scan-sized triangles", t));

    // 3. Adversarial rims: one edge passes the cone at its radius times (1 + delta), delta
    //    log-uniform from 1e-7 to 3, so cases straddle the certificate's own threshold.
    let mut t = Tally::default();
    for i in 0..per {
        let slope = slopes[i % 4];
        let l64 = rng.sphere();
        let base = [rng.range(-200., 200.), rng.range(-200., 200.), rng.range(-200., 200.)];
        let p = pixel(l64, slope, base, rng.range(1e-5, 1e-3), if i % 3 == 0 { 3e-3 } else { 0. }, &mut rng);
        let h = log_uniform(&mut rng, 0.003, 30.);
        let l = normalize64(f64s(p.l));
        let u = normalize64(sub64(f64s(p.u), scale64(l, dot64(f64s(p.u), l))));
        let v = cross64(l, u);
        let centre = add64(f64s(p.origin), scale64(l, h));
        let cone = slope * h;
        let r = cone * (1. + log_uniform(&mut rng, 1e-7, 3.));
        let a = rng.range(0., 6.28);
        let (n, tdir) = (add64(scale64(u, a.cos()), scale64(v, a.sin())), add64(scale64(u, -a.sin()), scale64(v, a.cos())));
        let far = cone * rng.range(2., 40.) + 1e-3;
        // The plane tilts about the rim edge's line, so its far vertex rises or sinks.
        let tilt = scale64(l, rng.range(-0.8, 0.8) * far);
        let tri = [
            add64(add64(centre, scale64(n, r)), scale64(tdir, far * 4.)),
            add64(add64(centre, scale64(n, r)), scale64(tdir, -far * 4.)),
            add64(add64(centre, scale64(n, -far * 4.)), tilt),
        ];
        let model = Model::new(0., 1., [0., 0., 0.]);
        check(&mut rng, &p, &model, [f32s(tri[0]), f32s(tri[1]), f32s(tri[2])], &mut t, "rim");
    }
    tallies.push(("adversarial rim edges", t));

    // 4. Adversarial heights: a wide triangle, tilted up to 80 degrees from facing the sun,
    //    whose plane passes 0.5-12 epsilon from the origin along the sun.
    let mut t = Tally::default();
    for i in 0..per {
        let slope = slopes[i % 4];
        let l64 = rng.sphere();
        let base = [rng.range(-60., 60.), rng.range(-60., 60.), rng.range(-60., 60.)];
        let eps = rng.range(1e-5, 1e-3);
        let p = pixel(l64, slope, base, eps, 0., &mut rng);
        let l = normalize64(f64s(p.l));
        let u = normalize64(sub64(f64s(p.u), scale64(l, dot64(f64s(p.u), l))));
        let v = cross64(l, u);
        let h = eps * rng.range(0.5, 12.);
        let centre = add64(f64s(p.origin), scale64(l, h));
        let tilt = rng.range(0., 80f64.to_radians());
        let (a, b) = (add64(scale64(u, tilt.cos()), scale64(l, tilt.sin())), v);
        let s = rng.range(0.01, 3.);
        let tri = [
            add64(centre, scale64(a, 2. * s)),
            add64(centre, add64(scale64(a, -s), scale64(b, 2. * s))),
            add64(centre, add64(scale64(a, -s), scale64(b, -2. * s))),
        ];
        let model = Model::new(0., 1., [0., 0., 0.]);
        check(&mut rng, &p, &model, [f32s(tri[0]), f32s(tri[1]), f32s(tri[2])], &mut t, "height");
    }
    tallies.push(("adversarial heights near epsilon", t));

    let mut failed = false;
    for (name, t) in &tallies {
        println!(
            "{name}: cases {} certified {} | exact-contained {} of which certified {} ({:.2}%) | false certificates: watertight {} exact {}",
            t.cases,
            t.certified,
            t.contained,
            t.contained_certified,
            100. * t.contained_certified as f64 / t.contained.max(1) as f64,
            t.false_hits,
            t.false_exact
        );
        failed |= t.false_hits + t.false_exact != 0;
    }
    println!("{}", if failed { "FAIL" } else { "PASS: no false certificate" });
    std::process::exit(failed as i32);
}

fn normalize64(a: D3) -> D3 {
    scale64(a, 1. / dot64(a, a).sqrt())
}
