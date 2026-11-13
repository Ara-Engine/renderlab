//! TLAS step 2 (`RENDERLAB_RS_BVH=4`): the reflection BVH4's leaves keep R1's hit identity and
//! its walk finds every closest hit the triangles hold. A CPU replay of the WGSL BVH4 walk
//! (src/program.rs RS_BVH4) against a brute-force test of every triangle record, with the same
//! per-triangle arithmetic, so the distances compare bit for bit.
use super::*;
use glam::Vec3;

type Hit = (u32, f32, f32, f32);

fn v3(w: &[u32], i: usize) -> Vec3 {
    Vec3::from_array(std::array::from_fn(|k| f32::from_bits(w[i + k])))
}
/// Triangle record `x`'s hit (primitive, u, v, t), the traversal's own test (no edge slack).
fn triangle(b: &Blas, x: usize, o: Vec3, d: Vec3) -> Option<Hit> {
    let w = &b.triangles[x * TRIANGLE_WORDS..];
    let (a, e1, e2) = (v3(w, 0), v3(w, 3), v3(w, 6));
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-30 {
        return None;
    }
    let s = o - a;
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = d.dot(q) / det;
    let t = e2.dot(q) / det;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some((w[9], u, v, t))
}
/// The WGSL BVH4 rule: the nearer hit, and at an equal distance the lower primitive.
fn better(best: Option<Hit>, h: Option<Hit>) -> Option<Hit> {
    match (best, h) {
        (Some(b), Some(h)) if h.3 < b.3 || (h.3 == b.3 && h.0 < b.0) => Some(h),
        (None, h) => h,
        (b, _) => b,
    }
}
/// Every triangle record, in order.
fn brute(b: &Blas, o: Vec3, d: Vec3) -> Option<Hit> {
    (0..b.triangles.len() / TRIANGLE_WORDS).fold(None, |best, x| better(best, triangle(b, x, o, d)))
}
/// The slab entry distance, or None on a miss (rs_box, tmin 0).
fn slab(o: Vec3, inv: Vec3, lo: Vec3, hi: Vec3, tmax: f32) -> Option<f32> {
    let (a, z) = ((lo - o) * inv, (hi - o) * inv);
    let near = a.min(z).max_element().max(0.0);
    let far = a.max(z).min_element().min(tmax);
    (near <= far).then_some(near)
}
/// The WGSL BVH4 walk: quantized boxes decoded as origin + step x byte, nearest child first.
fn walk(b: &Blas, o: Vec3, d: Vec3) -> (Option<Hit>, usize) {
    let inv = d.map(|c| {
        1.0 / if c.abs() < 1e-30 {
            1e-30f32.copysign(c)
        } else {
            c
        }
    });
    let mut best: Option<Hit> = None;
    let mut stack = vec![b.root];
    let mut deepest = 1;
    while let Some(child) = stack.pop() {
        if child & LEAF != 0 {
            let first = ((child & !LEAF) >> 4) as usize;
            for k in 0..(child & 15) as usize {
                best = better(best, triangle(b, first + k, o, d));
            }
            continue;
        }
        let n = &b.nodes[child as usize * NODE_WORDS..][..NODE_WORDS];
        let (base, step) = (v3(n, 0), v3(n, 3));
        let limit = best.map_or(f32::INFINITY, |h| h.3);
        let mut entered = vec![];
        for s in 0..4 {
            let c = n[6 + s];
            if c == EMPTY {
                continue;
            }
            let q = |w: usize| {
                Vec3::from_array(std::array::from_fn(|a| {
                    ((n[w + a] >> (8 * s)) & 0xff) as f32
                }))
            };
            if let Some(t) = slab(o, inv, base + step * q(10), base + step * q(13), limit) {
                entered.push((t, c));
            }
        }
        entered.sort_by(|a, b| b.0.total_cmp(&a.0));
        stack.extend(entered.into_iter().map(|e| e.1));
        deepest = deepest.max(stack.len());
    }
    (best, deepest)
}
/// Node levels below `child`.
fn depth(b: &Blas, child: u32) -> usize {
    if child & LEAF != 0 {
        return 0;
    }
    let n = child as usize * NODE_WORDS;
    1 + (0..4)
        .map(|s| b.nodes[n + 6 + s])
        .filter(|&c| c != EMPTY)
        .map(|c| depth(b, c))
        .max()
        .unwrap_or(0)
}
/// A bent grid of `n` x `n` quads.
fn grid(n: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut positions = vec![];
    let mut indices = vec![];
    for y in 0..=n {
        for x in 0..=n {
            positions.push([x as f32, y as f32, ((x * 7 + y * 3) % 5) as f32 * 0.01]);
        }
    }
    for y in 0..n {
        for x in 0..n {
            let (i, w) = (y * (n + 1) + x, n + 1);
            indices.extend([i, i + 1, i + w + 1, i, i + w + 1, i + w]);
        }
    }
    (positions, indices)
}
/// A deterministic soup: `count` small triangles scattered in a 10 m box (overlapping, at
/// every orientation), the shape a scanned or kitbashed mesh gives a BVH.
fn soup(count: usize, seed: u64) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut x = seed;
    let mut next = move || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 40) as f32) / (1u64 << 24) as f32
    };
    let mut positions = vec![];
    for _ in 0..count {
        let c = [next() * 10.0, next() * 10.0, next() * 10.0];
        for _ in 0..3 {
            positions.push(std::array::from_fn(|a| c[a] + (next() - 0.5) * 0.8));
        }
    }
    (positions, (0..count as u32 * 3).collect())
}
fn rays(count: usize, seed: u64) -> Vec<(Vec3, Vec3)> {
    let mut x = seed;
    let mut next = move || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 40) as f32) / (1u64 << 24) as f32
    };
    (0..count)
        .map(|_| {
            let o = Vec3::new(
                next() * 30.0 - 10.0,
                next() * 30.0 - 10.0,
                next() * 30.0 - 10.0,
            );
            let target = Vec3::new(
                next() * 12.0 - 1.0,
                next() * 12.0 - 1.0,
                next() * 12.0 - 1.0,
            );
            (o, target - o)
        })
        .collect()
}

/// Every leaf triangle is its own record once, with its own primitive index and barycentrics
/// in index-buffer corner order; rebuilds are word-identical; the asset build behind it is
/// `build_bvh_sah`'s, word for word.
#[test]
fn reflection_bvh4_keeps_primitive_identity() {
    let (positions, indices) = grid(23);
    let b = Blas::build4(&positions, &indices).unwrap();
    let mut seen: Vec<u32> = b.triangles.chunks(TRIANGLE_WORDS).map(|t| t[9]).collect();
    seen.sort_unstable();
    assert_eq!(seen, (0..indices.len() as u32 / 3).collect::<Vec<_>>());
    for t in 0..indices.len() / 3 {
        let p = |k: usize| Vec3::from(positions[indices[t * 3 + k] as usize]);
        let target = p(0) * 0.5 + p(1) * 0.3 + p(2) * 0.2;
        let o = target + Vec3::new(0.01, -0.02, 3.0);
        let (prim, u, v, t_hit) = walk(&b, o, target - o).0.unwrap();
        assert_eq!(prim as usize, t, "triangle {t}");
        assert!((u - 0.3).abs() < 1e-4 && (v - 0.2).abs() < 1e-4, "{u} {v}");
        assert!((t_hit - 1.0).abs() < 1e-4);
    }
    let again = Blas::build4(&positions, &indices).unwrap();
    assert_eq!(
        (&b.nodes, &b.triangles, b.root),
        (&again.nodes, &again.triangles, again.root)
    );
    assert_eq!(
        crate::asset::near_field::build_bvh_sah(&positions, &indices).unwrap(),
        crate::asset::near_field::build_bvh_sah_order(&positions, &indices)
            .unwrap()
            .0
    );
}

/// The walk's closest hit is the brute-force closest over every record, bit for bit (the same
/// per-triangle arithmetic, and at an equal distance the lower primitive, whatever order the
/// walk met them in), on a grid, through its shared edges and on overlapping soups. The BVH2
/// finds the same distance, a different primitive only at a tie (it keeps the first found).
/// The stack stays within the WGSL's 48 entries.
#[test]
fn reflection_bvh4_finds_every_closest_hit() {
    for (name, (positions, indices)) in [
        ("grid", grid(40)),
        ("soup", soup(5000, 7)),
        ("dense soup", soup(20000, 11)),
    ] {
        let b4 = Blas::build4(&positions, &indices).unwrap();
        let b2 = Blas::build2(&positions, &indices).unwrap();
        let levels = depth(&b4, b4.root);
        assert!(
            levels <= crate::asset::near_field::MAX_SAH_DEPTH,
            "{name}: depth {levels}"
        );
        let mut hits = 0;
        let mut ties = 0;
        // Through shared edges: the midpoint of each quad's diagonal and of its lower edge.
        let edges = (name == "grid").then(|| {
            (0..indices.len() / 6).flat_map(|q| {
                let p = |k: usize| Vec3::from(positions[indices[q * 6 + k] as usize]);
                [(p(0) + p(2)) * 0.5, (p(0) + p(1)) * 0.5].map(|target| {
                    (
                        target + Vec3::new(0.25, 0.5, 4.0),
                        Vec3::new(-0.25, -0.5, -4.0),
                    )
                })
            })
        });
        for (o, d) in rays(3000, 3).into_iter().chain(edges.into_iter().flatten()) {
            let truth = brute(&b4, o, d);
            let (found, deepest) = walk(&b4, o, d);
            assert!(deepest <= 48, "{name}: stack {deepest}");
            let old = super::tests::closest(&b2, o.to_array(), d.to_array());
            if let (Some(a), Some(f)) = (truth, found) {
                assert_eq!(
                    (a.0, a.1.to_bits(), a.2.to_bits(), a.3.to_bits()),
                    (f.0, f.1.to_bits(), f.2.to_bits(), f.3.to_bits()),
                    "{name}: bvh4 hit"
                );
            }
            if let (Some(a), Some(h)) = (truth, old) {
                ties += (a.0 != h.0) as usize;
            }
            for (label, h) in [("bvh4", found), ("bvh2", old)] {
                match (truth, h) {
                    (None, None) => {}
                    (Some(a), Some(h)) => {
                        assert_eq!(a.3.to_bits(), h.3.to_bits(), "{name} {label}: distance");
                        if a.0 != h.0 {
                            // A tie: the walk's primitive holds the same distance.
                            let x = (0..b4.triangles.len() / TRIANGLE_WORDS)
                                .find(|&x| b4.triangles[x * TRIANGLE_WORDS + 9] == h.0)
                                .unwrap();
                            assert_eq!(
                                triangle(&b4, x, o, d).map(|t| t.3.to_bits()),
                                Some(a.3.to_bits())
                            );
                        }
                    }
                    _ => panic!("{name} {label}: {truth:?} vs {h:?}"),
                }
            }
            hits += truth.is_some() as usize;
        }
        assert!(hits > 300, "{name}: only {hits} hits");
        eprintln!("{name}: {hits} hits, {ties} ties the BVH2 resolved to another primitive");
    }
}
