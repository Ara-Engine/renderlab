//! P-S4B (docs/p-s4b.md): the arithmetic of S4's per-direction test and its reject/certify bound,
//! dependency-free so the standalone CPU oracle (`tools/p_s4b_oracle.rs`) includes this file by
//! path. shaders/sunvis_s4.wgsl ports `hits` and `classify` line for line in f32.
//!
//! Patent contract (docs/patent-via-8471853.md, US 8,471,853 claim 1): a candidate triangle is a
//! record of four planes, never three vertices: its supporting plane and three edge planes. The
//! test is a boolean half-line / half-space predicate: the ray from `a = o + eps d` crosses the
//! supporting plane (a sign test) and the crossing lies on the inner side of each edge plane (a
//! sign test of a 2x2 determinant of plane values). It solves no coordinate weights, decides no
//! consistency from weights, and reconstructs no occluder depth or hit distance.

/// One candidate triangle's planes (world space, unit normals): supporting plane `p`, edge planes
/// `e[k]` (each contains one edge; positive inside), the triangle's lowest and highest height
/// along the build's sun axis `L`, `|e[k] . L|`, and `|p . L|`. `l_planes`: every edge plane
/// contains `L` (the certificate needs it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planes {
    pub p: [f32; 4],
    pub e: [[f32; 4]; 3],
    pub h: [f32; 2],
    pub c: [f32; 3],
    pub pl: f32,
    pub l_planes: bool,
}

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let n = dot3(a, a).sqrt();
    (n > 0.0 && n.is_finite()).then(|| [a[0] / n, a[1] / n, a[2] / n])
}
fn bits(p: [f64; 3]) -> [u64; 3] {
    p.map(f64::to_bits)
}

/// The planes of world triangle `v` for sun axis `l` (unit), or None when it is degenerate or
/// thinner than f32 resolution at its coordinates' magnitude.
/// Edge planes contain the edge and `l`, built from the edge's endpoints in a canonical order, so
/// two triangles sharing an edge get exactly negated planes. An edge (nearly) parallel to `l`, or
/// a triangle seen edge-on along `l`, takes the plane through the edge along the triangle's normal.
pub fn planes(v: [[f64; 3]; 3], l: [f64; 3]) -> Option<Planes> {
    let n = unit(cross(sub(v[1], v[0]), sub(v[2], v[0])))?;
    let scale = v.iter().flatten().fold(1.0f64, |m, x| m.max(x.abs()));
    let mut e = [[0f32; 4]; 3];
    let mut c = [0f32; 3];
    let mut l_planes = true;
    for k in 0..3 {
        let (mut a, mut b) = (v[k], v[(k + 1) % 3]);
        if bits(a) > bits(b) {
            std::mem::swap(&mut a, &mut b);
        }
        let edge = sub(b, a);
        let along = unit(cross(edge, l)).filter(|u| dot3(*u, n).abs() < 1.0 - 1e-6);
        let u = match along {
            Some(u) => u,
            None => {
                l_planes = false;
                unit(cross(edge, n))?
            }
        };
        let mut w = -dot3(u, a);
        let mut u = u;
        // The third vertex orients the plane. Within f32 resolution of it (a sliver, or a
        // degenerate triangle with two coincident vertices), the orientation is noise and the
        // prism a half-space: no f32 ray test can hit such a triangle, so it gets no record.
        let side = dot3(u, v[(k + 2) % 3]) + w;
        if side.abs() <= 4.0 * f64::from(f32::EPSILON) * scale {
            return None;
        }
        if side < 0.0 {
            u = u.map(|x| -x);
            w = -w;
        }
        e[k] = [u[0] as f32, u[1] as f32, u[2] as f32, w as f32];
        // Rounded up: the bound must not shrink.
        c[k] = (dot3(u, l).abs() * (1.0 + 1e-6) + 1e-7) as f32;
    }
    let h = v.map(|p| dot3(p, l));
    Some(Planes {
        p: [n[0] as f32, n[1] as f32, n[2] as f32, -dot3(n, v[0]) as f32],
        e,
        h: [
            h.iter().copied().fold(f64::INFINITY, f64::min) as f32,
            h.iter().copied().fold(f64::NEG_INFINITY, f64::max) as f32,
        ],
        c,
        pl: (dot3(n, l).abs() * (1.0 - 1e-6)) as f32,
        l_planes,
    })
}

fn fdot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0].mul_add(b[0], a[1].mul_add(b[1], a[2] * b[2]))
}
fn plane(q: [f32; 4], x: [f32; 3]) -> f32 {
    fdot([q[0], q[1], q[2]], x) + q[3]
}

/// The edge predicates' absolute slack at point `a` (metres): shared edges overlap by it rather
/// than crack under rounding.
pub fn tolerance(a: [f32; 3]) -> f32 {
    2e-7 * (a[0].abs() + a[1].abs() + a[2].abs()) + 1e-7
}

/// Whether the ray from `o` along unit `d`, from parameter `eps` on, meets the triangle: the
/// boolean contract above (WGSL `s4_hits`).
pub fn hits(t: &Planes, o: [f32; 3], d: [f32; 3], eps: f32) -> bool {
    let a = [
        d[0].mul_add(eps, o[0]),
        d[1].mul_add(eps, o[1]),
        d[2].mul_add(eps, o[2]),
    ];
    let sa = plane(t.p, a);
    let sd = fdot([t.p[0], t.p[1], t.p[2]], d);
    if sd == 0.0 || (sa > 0.0 && sd > 0.0) || (sa < 0.0 && sd < 0.0) {
        return false;
    }
    let m = sd.abs();
    let g = if sd > 0.0 { sa } else { -sa };
    let tol = tolerance(a) * m;
    t.e.iter().all(|e| {
        let ea = plane(*e, a);
        let ed = fdot([e[0], e[1], e[2]], d);
        ea.mul_add(m, -(g * ed)) >= -tol
    })
}

/// A receiver's verdict for one triangle over its whole direction cone: no direction hits, every
/// direction hits, or each must be tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Reject,
    Certify,
    Test,
}

/// The reject/certify bound (WGSL `s4_classify`). The receiver's rays start at `o` (`thin`: within
/// `2 eps` of it, per direction) and run along unit directions within `slope` of the build's sun
/// axis `l` (the cone of `vsm_direction`), each from parameter `eps`.
/// - A hit at parameter t has height at least `t cos` above `o`, so t <= T for any hit point.
/// - Edge plane k changes along a ray by at most `t (c_k + slope)`: the edge value stays within
///   `r_k` of its value at `o` for t <= T. One edge below `-r_k` everywhere: reject.
/// - Certify: every edge plane contains `l` (the prism along `l` over the triangle), every edge
///   value exceeds `r_k` (each ray stays in the prism up to T), every vertex lies 4 eps above
///   the ray start and the plane faces the cone one way: each ray enters the prism below the
///   triangle and leaves it above, so it crosses the triangle.
///   `margin` covers f32 evaluation at `o`'s magnitude.
pub fn classify(t: &Planes, o: [f32; 3], l: [f32; 3], eps: f32, slope: f32, thin: bool) -> Verdict {
    let big = o[0].abs().max(o[1].abs()).max(o[2].abs()) + 1.0;
    let margin = 4e-6 * big + if thin { 2.0 * eps } else { 0.0 };
    let ho = fdot(l, o);
    let hmax = t.h[1] - ho;
    let hmin = t.h[0] - ho;
    if hmax < 0.5 * eps - margin {
        return Verdict::Reject;
    }
    let s = slope * 1.001 + 2e-6;
    let reach = hmax.max(0.0) * (1.0 + s * s) * 1.0001 + eps + margin;
    let mut all_in = true;
    for k in 0..3 {
        let v = plane(t.e[k], o);
        let r = reach * (t.c[k] + s) + margin;
        if v + r < 0.0 {
            return Verdict::Reject;
        }
        if v - r <= margin {
            all_in = false;
        }
    }
    if !thin && all_in && t.l_planes && t.pl > 2.0 * s + 1e-3 && hmin > 4.0 * eps + margin {
        return Verdict::Certify;
    }
    Verdict::Test
}
