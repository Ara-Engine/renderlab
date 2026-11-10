//! P-S4B CPU oracle (docs/p-s4b.md): S4's reject/certify bound (`classify`) against its own f32
//! per-direction test (`hits`), both from src/sunvis_s4_math.rs, which shaders/sunvis_s4.wgsl
//! ports. Standalone: `rustc -O --edition 2021 tools/p_s4b_oracle.rs`.
//!
//! A false reject drops a hit (adds light); a false certify invents one. Each case's receiver is
//! tested against every sequence direction at two pixel shifts and against rim and disk
//! directions, each unperturbed and with GPU rounding noise on the direction (6e-7 per component)
//! and origin (1 ulp). Thin receivers move the origin by up to 2 eps per direction. The f32 test
//! is also compared with an exact f64 segment/triangle test (ties only, reported).
#[path = "../src/sunvis_s4_math.rs"]
#[allow(dead_code)]
mod m;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next()
    }
}

fn norm(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn f32s(v: [f64; 3]) -> [f32; 3] {
    v.map(|x| x as f32)
}

/// Exact (f64) ray/triangle hit at parameter >= eps, Moller-Trumbore.
fn exact(v: [[f64; 3]; 3], o: [f64; 3], d: [f64; 3], eps: f64) -> bool {
    let e1 = add(v[1], v[0], -1.0);
    let e2 = add(v[2], v[0], -1.0);
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-300 {
        return false;
    }
    let s = add(o, v[0], -1.0);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let w = dot(d, q) / det;
    let t = dot(e2, q) / det;
    u >= 0.0 && w >= 0.0 && u + w <= 1.0 && t >= eps
}

struct Sun {
    l: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    slope: f64,
}
impl Sun {
    fn new(r: &mut Rng) -> Self {
        let elevation = r.range(0.15, 1.5);
        let azimuth = r.range(0.0, 6.283);
        let l = norm([
            elevation.cos() * azimuth.cos(),
            elevation.sin(),
            elevation.cos() * azimuth.sin(),
        ]);
        let u = norm(cross(l, [0.0, 0.0, 1.0]));
        let v = cross(l, u);
        let slope = [0.00465, 0.02, 0.08][(r.next() * 3.0) as usize % 3];
        Sun { l, u, v, slope }
    }
    /// vsm_direction in f32 (WGSL formula).
    fn sequence(&self, i: u32, shift: [f32; 2]) -> [f64; 3] {
        let fi = i as f32;
        let rx = (fi * 0.754_877_7 + shift[0]).fract();
        let ry = (fi * 0.569_840_3 + shift[1]).fract();
        let a = 6.283_185_3f32 * ry;
        let s = self.slope as f32 * rx.sqrt();
        let (l, u, v) = (f32s(self.l), f32s(self.u), f32s(self.v));
        let w: [f32; 3] = std::array::from_fn(|k| l[k] + s * (a.cos() * u[k] + a.sin() * v[k]));
        let n = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        w.map(|x| (x / n) as f64)
    }
    fn disk(&self, radius: f64, angle: f64) -> [f64; 3] {
        norm(add(
            add(self.l, self.u, self.slope * radius * angle.cos()),
            self.v,
            self.slope * radius * angle.sin(),
        ))
    }
}

#[derive(Default)]
struct Tally {
    cases: u64,
    reject: u64,
    certify: u64,
    false_reject: u64,
    false_certify: u64,
    tests: u64,
    f32_vs_exact: u64,
}

fn run(
    t: &mut Tally,
    r: &mut Rng,
    sun: &Sun,
    tri: [[f64; 3]; 3],
    o: [f64; 3],
    eps: f64,
    thin: bool,
) {
    let Some(p) = m::planes(tri, sun.l) else {
        return;
    };
    t.cases += 1;
    let verdict = m::classify(&p, f32s(o), f32s(sun.l), eps as f32, sun.slope as f32, thin);
    match verdict {
        m::Verdict::Reject => t.reject += 1,
        m::Verdict::Certify => t.certify += 1,
        m::Verdict::Test => {}
    }
    let mut dirs = vec![];
    for s in 0..2 {
        let shift = [r.next() as f32, r.next() as f32];
        let _ = s;
        dirs.extend((0..256).map(|i| sun.sequence(i, shift)));
    }
    for k in 0..256 {
        let radius = if k % 2 == 0 { 1.0 } else { r.next().sqrt() };
        dirs.push(sun.disk(radius, r.range(0.0, 6.2832)));
    }
    let mut bad = false;
    for d in dirs {
        for noise in 0..3 {
            let dn = if noise == 0 {
                d
            } else {
                norm(d.map(|x| x + r.range(-6e-7, 6e-7)))
            };
            let mut on = if noise == 2 {
                o.map(|x| x * (1.0 + r.range(-1.2e-7, 1.2e-7)))
            } else {
                o
            };
            if thin {
                on = add(on, sun.l, r.range(-2.0, 2.0) * eps);
            }
            let hit = m::hits(&p, f32s(on), f32s(dn), eps as f32);
            t.tests += 1;
            if noise == 0 && hit != exact(tri, on, dn, eps) {
                t.f32_vs_exact += 1;
            }
            match verdict {
                m::Verdict::Reject if hit => bad = true,
                m::Verdict::Certify if !hit => bad = true,
                _ => {}
            }
        }
    }
    if bad {
        match verdict {
            m::Verdict::Reject => t.false_reject += 1,
            _ => t.false_certify += 1,
        }
        if t.false_reject + t.false_certify <= 5 {
            eprintln!(
                "false {verdict:?}: tri {tri:?} o {o:?} eps {eps} thin {thin} slope {}",
                sun.slope
            );
        }
    }
}

fn main() {
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut rows = vec![];
    for fixture in ["boxes", "scan", "rims", "heights", "thin"] {
        let mut t = Tally::default();
        for _ in 0..6000 {
            let sun = Sun::new(&mut r);
            let offset = [
                r.range(-200.0, 200.0),
                r.range(-5.0, 40.0),
                r.range(-200.0, 200.0),
            ];
            let eps = r.range(2e-4, 4e-3);
            let o = offset;
            // A point h above the receiver along the cone, offset sideways by (a, b) in units of
            // the cone's radius there.
            let at = |h: f64, a: f64, b: f64| {
                add(
                    add(add(o, sun.l, h), sun.u, a * sun.slope * h),
                    sun.v,
                    b * sun.slope * h,
                )
            };
            let tris: Vec<[[f64; 3]; 3]> = match fixture {
                "boxes" => {
                    // A box of 0.2-3 m at 0-8 m along the cone: its 12 triangles.
                    let c = at(r.range(0.0, 8.0), r.range(-3.0, 3.0), r.range(-3.0, 3.0));
                    let e = [r.range(0.1, 1.5), r.range(0.1, 1.5), r.range(0.1, 1.5)];
                    let corner = |i: usize| {
                        std::array::from_fn(|k| c[k] + if i >> k & 1 == 1 { e[k] } else { -e[k] })
                    };
                    let faces = [
                        [0, 1, 3, 2],
                        [4, 6, 7, 5],
                        [0, 4, 5, 1],
                        [2, 3, 7, 6],
                        [0, 2, 6, 4],
                        [1, 5, 7, 3],
                    ];
                    faces
                        .iter()
                        .flat_map(|f| {
                            [
                                [corner(f[0]), corner(f[1]), corner(f[2])],
                                [corner(f[0]), corner(f[2]), corner(f[3])],
                            ]
                        })
                        .collect()
                }
                "scan" => {
                    let c = at(r.range(0.001, 2.0), r.range(-2.0, 2.0), r.range(-2.0, 2.0));
                    let s = r.range(0.001, 0.03);
                    vec![std::array::from_fn(|_| {
                        [
                            c[0] + r.range(-s, s),
                            c[1] + r.range(-s, s),
                            c[2] + r.range(-s, s),
                        ]
                    })]
                }
                "rims" => {
                    // An edge tangent to the cone at its radius times (1 + tiny).
                    let h = r.range(0.01, 6.0);
                    let k = 1.0 + 10f64.powf(r.range(-7.0, 0.5));
                    let side = if r.next() < 0.5 { k } else { -k };
                    let a = at(h, side, -3.0);
                    let b = at(h, side, 3.0);
                    let c = at(h + r.range(-0.5, 0.5), side * 3.0, r.range(-1.0, 1.0));
                    vec![
                        [a, b, c],
                        [
                            at(h, -side, -3.0),
                            at(h, -side, 3.0),
                            at(h, side * 5.0, 0.0),
                        ],
                    ]
                }
                "heights" => {
                    // A wide plane 0.5-12 eps above the start, tilted up to 80 degrees.
                    let h = eps * r.range(0.5, 12.0);
                    let tilt = r.range(0.0, 1.396);
                    let axis = norm(add(sun.u, sun.v, r.range(-1.0, 1.0)));
                    let n = norm(add(sun.l.map(|x| x * tilt.cos()), axis, tilt.sin()));
                    let t1 = norm(cross(n, sun.u));
                    let t2 = cross(n, t1);
                    let c = at(h, 0.0, 0.0);
                    let s = r.range(0.01, 3.0);
                    vec![[
                        add(c, t1, s),
                        add(add(c, t1, -s), t2, s),
                        add(add(c, t1, -s), t2, -s),
                    ]]
                }
                _ => {
                    let c = at(r.range(0.0, 1.0), r.range(-2.0, 2.0), r.range(-2.0, 2.0));
                    let s = r.range(0.01, 1.0);
                    vec![std::array::from_fn(|_| {
                        [
                            c[0] + r.range(-s, s),
                            c[1] + r.range(-s, s),
                            c[2] + r.range(-s, s),
                        ]
                    })]
                }
            };
            for tri in tris {
                run(&mut t, &mut r, &sun, tri, o, eps, fixture == "thin");
            }
        }
        println!(
            "{fixture:8} cases {:6} reject {:6} certify {:6} false-reject {} false-certify {} tests {} f32-vs-f64 {}",
            t.cases, t.reject, t.certify, t.false_reject, t.false_certify, t.tests, t.f32_vs_exact
        );
        rows.push(t.false_reject + t.false_certify);
    }
    std::process::exit(i32::from(rows.iter().any(|&n| n > 0)));
}
