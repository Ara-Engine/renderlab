//! CPU-only rays and distance evidence. All populations and seeds are explicit.
use crate::{Result, cluster_bvh, ensure};
use renderlab_asset::{
    cluster_bvh::{Bvh, Limits, Ray},
    virtual_mesh::{Package, VirtualLimits},
};
use serde_json::{Value, json};
use std::time::Instant;
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let r = v.iter().map(|v| v * v).sum::<f64>().sqrt();
    v.map(|v| v / r)
}
fn random(state: &mut u64) -> f64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 11) as f64) / ((1u64 << 53) as f64)
}
fn triangles(b: &Bvh, level: usize) -> u64 {
    b.level_leaves[b.levels[level].leaves.range(b.level_leaves.len()).unwrap()]
        .iter()
        .map(|&id| u64::from(b.leaves[id as usize].indices.count / 3))
        .sum()
}
fn bounds(b: &Bvh, level: usize) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for &id in &b.level_leaves[b.levels[level].leaves.range(b.level_leaves.len()).unwrap()] {
        let (a, z) = b.leaf_bounds(&b.leaves[id as usize]);
        for i in 0..3 {
            lo[i] = lo[i].min(a[i]);
            hi[i] = hi[i].max(z[i]);
        }
    }
    (lo, hi)
}
fn ray(b: &Bvh, level: usize, i: usize, reflection: bool, seed: &mut u64) -> Ray {
    let cut = &b.level_leaves[b.levels[level].leaves.range(b.level_leaves.len()).unwrap()];
    let leaf = cut[((random(seed) * cut.len() as f64) as usize).min(cut.len() - 1)];
    let t = (random(seed) * f64::from(b.leaves[leaf as usize].indices.count / 3)) as u32;
    let tri = b.triangle(leaf, t);
    let p: [f64; 3] = std::array::from_fn(|a| (tri[0][a] + tri[1][a] + tri[2][a]) / 3.);
    let z = random(seed) * 2. - 1.;
    let phi = random(seed) * std::f64::consts::TAU;
    let r = (1. - z * z).sqrt();
    let mut d = [r * phi.cos(), z, r * phi.sin()];
    let (lo, hi) = bounds_cached(b, level);
    let extent = (0..3)
        .map(|a| (hi[a] - lo[a]).powi(2))
        .sum::<f64>()
        .sqrt()
        .max(1e-3);
    if reflection {
        // RF1-like puddle, 20..40 degree pitched reflected view. A deterministic
        // angular perturbation exercises roughness bands 0, .05 and .1. It is
        // a geometry ray population, not a BRDF/image reference for those bands.
        let roughness = [0., 0.05, 0.1][i % 3];
        let pitch = ((20. + 20. * random(seed)).to_radians() + (random(seed) - 0.5) * roughness)
            .clamp(0.1, 1.4);
        let heading = phi + (random(seed) - 0.5) * roughness;
        d = [
            pitch.cos() * heading.cos(),
            pitch.sin(),
            pitch.cos() * heading.sin(),
        ];
        let plane = lo[1] - extent * 0.05;
        let distance = (p[1] - plane) / d[1];
        let origin = [p[0] - d[0] * distance, plane, p[2] - d[2] * distance];
        Ray {
            origin,
            direction: d,
            t_min: 0.,
            t_max: distance + extent * 3.,
        }
    } else {
        // World-cell-like origins, including inward, outward and axis-parallel rays.
        if i.is_multiple_of(17) {
            d = [1., 0., 0.];
        }
        let origin = std::array::from_fn(|a| p[a] - d[a] * (extent * 0.05 + 0.0001));
        Ray {
            origin,
            direction: normalize(d),
            t_min: 1e-6,
            t_max: extent * 3.,
        }
    }
}
// The caller supplies per-mesh bounds through the first root's enclosed boxes;
// unlike full surface enumeration this is O(4), even for the 20k-ray screen.
fn bounds_cached(b: &Bvh, level: usize) -> ([f64; 3], [f64; 3]) {
    let root = b.levels[level].root;
    if root & renderlab_asset::cluster_bvh::LEAF_BIT != 0 {
        return b.leaf_bounds(&b.leaves[(root & !renderlab_asset::cluster_bvh::LEAF_BIT) as usize]);
    }
    let node = &b.nodes[root as usize];
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for slot in 0..4 {
        if node.children[slot] == renderlab_asset::cluster_bvh::EMPTY {
            continue;
        }
        let (a, z) = node.child_bounds(slot);
        for i in 0..3 {
            lo[i] = lo[i].min(a[i]);
            hi[i] = hi[i].max(z[i]);
        }
    }
    (lo, hi)
}

/// Gives a conservative directed surface-distance upper bound: a cluster lies
/// in its AABB ball (c,r), and distance(x,target) <= distance(c,target)+r.
/// It is deliberately loose, unlike a claimed unmeasured Hausdorff certificate
/// from the simplifier. Inflation covers ordinary f64 diagnostic roundoff.
fn surface_bound(source: &Bvh, level: usize, target: &Bvh, target_level: usize) -> (f64, f64) {
    let mut upper = 0f64;
    let mut sampled = 0f64;
    for &id in &source.level_leaves[source.levels[level]
        .leaves
        .range(source.level_leaves.len())
        .unwrap()]
    {
        let (lo, hi) = source.leaf_bounds(&source.leaves[id as usize]);
        let c = std::array::from_fn(|a| (lo[a] + hi[a]) * 0.5);
        let r = (0..3)
            .map(|a| ((hi[a] - lo[a]) * 0.5).powi(2))
            .sum::<f64>()
            .sqrt();
        let d = target.nearest_distance(target_level, c).unwrap();
        upper = upper.max(d + r);
        let tri = source.triangle(id, 0);
        let centroid = std::array::from_fn(|a| (tri[0][a] + tri[1][a] + tri[2][a]) / 3.);
        sampled = sampled.max(target.nearest_distance(target_level, centroid).unwrap());
    }
    (upper * (1. + 1e-12) + 1e-9, sampled)
}

/// One exact oracle mesh at a time bounds transient geometry memory. The full
/// detail cut comes from original-source triangles in the existing cooked DAG.
pub fn validate(
    b: &Bvh,
    package: &Package,
    limits: VirtualLimits,
    mut load: impl FnMut(usize) -> Result<renderlab_asset::virtual_mesh::PageGeometry>,
    rays_per_population: usize,
    brute_per_mesh: usize,
) -> Result<Value> {
    let start = Instant::now();
    let mut rows = vec![];
    let count = package.meshes.len();
    for mesh in 0..count {
        // Each mesh has independent page/cluster IDs. One exact mesh at a time
        // keeps the oracle's geometry transient and its memory measurable.
        let (single, page_map) = mesh_package(package, mesh)?;
        let oracle = cluster_bvh::prepare(
            &single,
            &cluster_bvh::Settings {
                errors: vec![0.],
                limits: Limits {
                    max_bytes: 900_000_000,
                },
            },
            limits,
            |p| load(page_map[p]),
        )?;
        let n = rays_per_population / count + usize::from(mesh < rays_per_population % count);
        let mut populations = [Vec::with_capacity(n), Vec::with_capacity(n)];
        for (band, population) in populations.iter_mut().enumerate() {
            let mut seed = 0x723063_u64 ^ (mesh as u64 * 7919) ^ (band as u64 * 104729);
            for i in 0..n {
                population.push(ray(&oracle, 0, i, band == 1, &mut seed));
            }
        }
        let oracle_hits: Vec<_> = populations
            .iter()
            .map(|p| p.iter().map(|&r| oracle.trace(0, r)).collect::<Vec<_>>())
            .collect();
        for (li, level) in b
            .levels
            .iter()
            .enumerate()
            .filter(|(_, l)| l.mesh as usize == mesh)
        {
            let before = Instant::now();
            let mut results = vec![];
            for (pi, population) in populations.iter().enumerate() {
                let mut mismatches = 0;
                let mut hit_miss = 0;
                let mut max_delta = 0f64;
                let mut sum_delta = 0f64;
                let mut both = 0;
                let mut over_footprint = 0;
                let mut brute = 0;
                for (i, &ray) in population.iter().enumerate() {
                    let cone = renderlab_asset::cluster_bvh::Cone {
                        width: f64::from(level.requested_error) * 4.,
                        spread: if pi == 0 {
                            0.01
                        } else {
                            [0., 0.05, 0.1][i % 3]
                        },
                    };
                    let selected = b
                        .select_cone_level(level.mesh, ray, cone, 0.25)
                        .expect("current cooked level fits its cone");
                    ensure(
                        b.levels[selected].max_error <= (cone.width_at(ray.t_min) * 0.25) as f32,
                        "cone selector exceeds error budget",
                    )?;
                    let hit = b.trace(li, ray);
                    if i < brute_per_mesh {
                        mismatches += usize::from(hit != b.trace_brute(li, ray));
                        brute += 1;
                    }
                    match (hit, oracle_hits[pi][i]) {
                        (Some(a), Some(o)) => {
                            let delta = (a.distance - o.distance).abs();
                            max_delta = max_delta.max(delta);
                            sum_delta += delta;
                            both += 1;
                            over_footprint += usize::from(delta > cone.width_at(o.distance));
                        }
                        (None, None) => {}
                        _ => hit_miss += 1,
                    }
                }
                ensure(mismatches == 0, "CPU BVH differs from same-LOD brute force")?;
                results.push(json!({"population":if pi==0{"GI-cell"}else{"RF1-like-planar-reflection"},"rays":population.len(),"same_lod_brute_rays":brute,"same_lod_mismatches":mismatches,"full_detail_hit_miss_changes":hit_miss,"both_hit":both,"max_hit_distance_delta":max_delta,"mean_hit_distance_delta":sum_delta/(both.max(1) as f64),"distance_delta_over_width_k025":over_footprint}));
            }
            let (full_to_lod, sampled_full) = surface_bound(&oracle, 0, b, li);
            let (lod_to_full, sampled_lod) = surface_bound(b, li, &oracle, 0);
            let (lo, hi) = bounds(&oracle, 0);
            rows.push(json!({"mesh":mesh,"level":li,"requested_error":level.requested_error,"accumulated_simplifier_error":level.max_error,"triangles":triangles(b,li),"full_detail_triangles":triangles(&oracle,0),"oracle_bytes":oracle.gpu_bytes(),"object_bounds":[lo,hi],"surface_distance_bound":{"full_to_lod":full_to_lod,"lod_to_full":lod_to_full,"symmetric":full_to_lod.max(lod_to_full)},"sampled_centroid_surface_distance":{"full_to_lod":sampled_full,"lod_to_full":sampled_lod},"populations":results,"validation_us":before.elapsed().as_micros()}));
        }
    }
    Ok(
        json!({"rays_per_population":rays_per_population,"rows":rows,"validation_us":start.elapsed().as_micros(),"distance_units":"object-space; transform bounds by the placement linear operator norm"}),
    )
}
fn mesh_package(package: &Package, mesh: usize) -> Result<(Package, Vec<usize>)> {
    let mut m = package.meshes[mesh].clone();
    let mut page_map: Vec<_> = m.clusters.iter().map(|c| c.page.index()).collect();
    page_map.sort_unstable();
    page_map.dedup();
    // Page packing is per mesh, so source indices are already a complete cover.
    for c in &mut m.clusters {
        c.page = renderlab_asset::virtual_mesh::PageId(
            page_map.binary_search(&c.page.index()).unwrap() as u32,
        );
    }
    let p = Package {
        meshes: vec![m],
        pages: page_map.iter().map(|&i| package.pages[i].clone()).collect(),
        placements: vec![],
        resources: package.resources.clone(),
    };
    Ok((p, page_map))
}
