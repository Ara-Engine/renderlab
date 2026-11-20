//! PROTOTYPE (price check, not for merge): a conservative distance clipmap the lighting cache's
//! software gather and visibility rays march beyond a short triangle segment
//! (shaders/gi_rays_software.wgsl gi_sdf_march), `RENDERLAB_GI_SDF=1`. Built once at packing from
//! every placed triangle in world space, brute force: a voxel any triangle touches holds 0 (thin
//! parts read a voxel thick and block; they never leak), every other voxel its exact Euclidean
//! distance in voxels to the nearest touched one, in 8 bits. Cascades of CASCADE_N^3 voxels from
//! VOXEL0 metres, doubling until one covers the scene, centred on the scene's bounds (or
//! `RENDERLAB_GI_SDF_CENTER=x,y,z`). No scrolling, movers or skinned updates: a static fixture.
use std::sync::atomic::{AtomicU32, Ordering};

/// Voxels a cascade side.
pub(crate) const CASCADE_N: usize = 128;
/// Cascade 0's voxel edge in metres.
const VOXEL0: f32 = 0.125;
const MAX_CASCADES: usize = 10;
/// Words before the cascades' voxel data: the cascade count, then 8 words a cascade (min corner
/// xyz, voxel edge, side, data offset from the field's first word, two spare).
pub(crate) const HEADER_WORDS: usize = 1 + 8 * MAX_CASCADES;

/// The field's first word in the packed geometry buffer (0: none), for the instance header
/// (src/gi_bvh.rs header word 5).
pub(crate) static BASE: AtomicU32 = AtomicU32::new(0);
pub(crate) fn base() -> u32 {
    BASE.load(Ordering::Relaxed)
}

pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_GI_SDF").is_ok_and(|v| v == "1"))
}

fn center_override() -> Option<[f32; 3]> {
    let v = std::env::var("RENDERLAB_GI_SDF_CENTER").ok()?;
    let p: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    (p.len() == 3).then(|| [p[0], p[1], p[2]])
}

/// The field's words (header, then each cascade's bytes four to a word) for world-space
/// `triangles`, and the build's seconds.
pub(crate) fn build(triangles: &[[[f32; 3]; 3]]) -> (Vec<u32>, f64) {
    let start = std::time::Instant::now();
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for t in triangles {
        for p in t {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
    }
    let center = center_override().unwrap_or(std::array::from_fn(|a| 0.5 * (lo[a] + hi[a])));
    let extent = (0..3).map(|a| (hi[a] - center[a]).max(center[a] - lo[a])).fold(0.0f32, f32::max);
    let mut cascades = 1;
    while cascades < MAX_CASCADES && 0.5 * CASCADE_N as f32 * VOXEL0 * (1u32 << (cascades - 1)) as f32 <= extent {
        cascades += 1;
    }
    let voxels = CASCADE_N * CASCADE_N * CASCADE_N;
    let data_words = voxels / 4;
    let mut words = vec![0u32; HEADER_WORDS + cascades * data_words];
    words[0] = cascades as u32;
    for c in 0..cascades {
        let edge = VOXEL0 * (1u32 << c) as f32;
        let corner: [f32; 3] = std::array::from_fn(|a| center[a] - 0.5 * CASCADE_N as f32 * edge);
        let marked = voxelize(triangles, corner, edge);
        let distance = distance_transform(&marked);
        let h = 1 + 8 * c;
        words[h..h + 3].copy_from_slice(&corner.map(f32::to_bits));
        words[h + 3] = edge.to_bits();
        words[h + 4] = CASCADE_N as u32;
        let offset = HEADER_WORDS + c * data_words;
        words[h + 5] = offset as u32;
        for (i, chunk) in distance.chunks_exact(4).enumerate() {
            let q = |d: f32| (d.floor().min(255.0)) as u32;
            words[offset + i] = q(chunk[0]) | q(chunk[1]) << 8 | q(chunk[2]) << 16 | q(chunk[3]) << 24;
        }
    }
    (words, start.elapsed().as_secs_f64())
}

/// Marks every voxel of the cascade at `corner` with edge `edge` that a triangle overlaps
/// (separating-axis test, Akenine-Moller), slightly enlarged so a triangle on a face marks both.
fn voxelize(triangles: &[[[f32; 3]; 3]], corner: [f32; 3], edge: f32) -> Vec<bool> {
    let n = CASCADE_N;
    let mut marked = vec![false; n * n * n];
    let half = [0.5 * edge * 1.001; 3];
    for t in triangles {
        let mut lo = [0usize; 3];
        let mut hi = [0usize; 3];
        let mut outside = false;
        for a in 0..3 {
            let mn = t.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
            let mx = t.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max);
            let l = ((mn - corner[a]) / edge).floor();
            let h = ((mx - corner[a]) / edge).floor();
            if h < 0.0 || l >= n as f32 {
                outside = true;
                break;
            }
            lo[a] = l.max(0.0) as usize;
            hi[a] = (h as usize).min(n - 1);
        }
        if outside {
            continue;
        }
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    let i = (z * n + y) * n + x;
                    if marked[i] {
                        continue;
                    }
                    let c = [x, y, z].map(|v| v as f32 + 0.5);
                    let centre: [f32; 3] = std::array::from_fn(|a| corner[a] + c[a] * edge);
                    if triangle_box(t, centre, half) {
                        marked[i] = true;
                    }
                }
            }
        }
    }
    marked
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Whether triangle `t` overlaps the box at `centre` with half-extents `half` (13 separating axes).
fn triangle_box(t: &[[f32; 3]; 3], centre: [f32; 3], half: [f32; 3]) -> bool {
    let v = [sub(t[0], centre), sub(t[1], centre), sub(t[2], centre)];
    let e = [sub(v[1], v[0]), sub(v[2], v[1]), sub(v[0], v[2])];
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let separated = |axis: [f32; 3]| {
        let p = v.map(|q| dot(q, axis));
        let r = half[0] * axis[0].abs() + half[1] * axis[1].abs() + half[2] * axis[2].abs();
        let mn = p[0].min(p[1]).min(p[2]);
        let mx = p[0].max(p[1]).max(p[2]);
        mn > r || mx < -r
    };
    for ei in e {
        for a in axes {
            let axis = cross(ei, a);
            if dot(axis, axis) > 0.0 && separated(axis) {
                return false;
            }
        }
    }
    for a in axes {
        if separated(a) {
            return false;
        }
    }
    let normal = cross(e[0], e[1]);
    !(dot(normal, normal) > 0.0 && separated(normal))
}

/// Exact Euclidean distance (in voxels, centre to centre) from every voxel to the nearest marked
/// one: Felzenszwalb and Huttenlocher's separable squared transform along x, then y, then z.
fn distance_transform(marked: &[bool]) -> Vec<f32> {
    let n = CASCADE_N;
    const FAR: f32 = 1e20;
    let mut d: Vec<f32> = marked.iter().map(|&m| if m { 0.0 } else { FAR }).collect();
    let mut f = vec![0f32; n];
    let mut out = vec![0f32; n];
    let mut v = vec![0usize; n];
    let mut z = vec![0f32; n + 1];
    let index = |axis: usize, a: usize, b: usize, k: usize| match axis {
        0 => (b * n + a) * n + k,
        1 => (b * n + k) * n + a,
        _ => (k * n + b) * n + a,
    };
    for axis in 0..3 {
        for b in 0..n {
            for a in 0..n {
                for k in 0..n {
                    f[k] = d[index(axis, a, b, k)];
                }
                dt1(&f, &mut out, &mut v, &mut z);
                for k in 0..n {
                    d[index(axis, a, b, k)] = out[k];
                }
            }
        }
    }
    d.iter().map(|&s| s.sqrt()).collect()
}

/// One-dimensional squared distance transform of sampled function `f` (lower envelope of parabolas).
fn dt1(f: &[f32], out: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f32::NEG_INFINITY;
    z[1] = f32::INFINITY;
    for q in 1..n {
        let mut s;
        loop {
            let p = v[k];
            s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * (q as f32 - p as f32));
            // z[0] is minus infinity, so this stops at k = 0.
            if s <= z[k] {
                k -= 1;
            } else {
                break;
            }
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f32::INFINITY;
    }
    k = 0;
    for q in 0..n {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let p = v[k];
        out[q] = (q as f32 - p as f32).powi(2) + f[p];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_transform_counts_voxels_to_a_marked_one() {
        let n = CASCADE_N;
        let mut marked = vec![false; n * n * n];
        marked[(5 * n + 5) * n + 5] = true;
        let d = distance_transform(&marked);
        assert_eq!(d[(5 * n + 5) * n + 5], 0.0);
        assert!((d[(5 * n + 5) * n + 8] - 3.0).abs() < 1e-5);
        assert!((d[(5 * n + 9) * n + 8] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn a_thin_wall_marks_a_full_voxel_layer() {
        // A 1 mm sheet at x = 0.3 m in cascade 0 (0.125 m voxels from -8 m): one layer marked.
        let t = [[[0.3, -1.0, -1.0], [0.3, 1.0, -1.0], [0.3, 1.0, 1.0]], [[0.3, -1.0, -1.0], [0.3, 1.0, 1.0], [0.3, -1.0, 1.0]]];
        let marked = voxelize(&t, [-8.0; 3], VOXEL0);
        let n = CASCADE_N;
        let x = ((0.3f32 + 8.0) / VOXEL0).floor() as usize;
        let y = ((0.0f32 + 8.0) / VOXEL0) as usize;
        assert!(marked[(y * n + y) * n + x]);
        assert!(!marked[(y * n + y) * n + x + 2]);
    }
}
