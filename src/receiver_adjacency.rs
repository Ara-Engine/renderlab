//! Exact-position shared edges for the primary receiver repair. No topology guesses.
use crate::MeshData;
use rayon::prelude::*;

pub(crate) const MISSING: u32 = u32::MAX;

#[cfg(test)]
#[path = "adjacency_parallel_cpu_tests.rs"]
mod parallel_cpu_tests;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Adjacency {
    /// Triangle-major, opposite-vertex edge order, matching barycentric weights.
    pub neighbours: Vec<u32>,
    pub boundary_halfedges: usize,
    pub ambiguous_halfedges: usize,
    pub degenerate_halfedges: usize,
}

pub(crate) fn build(data: &MeshData<'_>) -> Adjacency {
    #[cfg(test)]
    if LEGACY.with(std::cell::Cell::get) {
        return legacy(data);
    }
    // Reuse the welding records for edges. Both sorts are in-place; there is no
    // hash table, merge-sort scratch, geometry copy or per-worker mesh copy.
    #[derive(Clone, Copy)]
    struct Record {
        key: [u32; 3],
        index: u32,
    }
    let mut records = Vec::with_capacity(data.vertices.len().max(data.indices.len()));
    records.extend(data.vertices.iter().enumerate().map(|(i, v)| Record {
        key: v.position.map(|x| if x == 0. { 0 } else { x.to_bits() }),
        index: i as u32,
    }));
    records.par_sort_unstable_by_key(|r| r.key);
    let mut canonical = vec![0; data.vertices.len()];
    let mut start = 0;
    while start < records.len() {
        let mut end = start + 1;
        while end < records.len() && records[end].key == records[start].key {
            end += 1;
        }
        let group = &records[start..end];
        let first = group.iter().map(|r| r.index).min().unwrap();
        for record in group {
            canonical[record.index as usize] = first;
        }
        start = end;
    }
    records.clear();
    let mut result = Adjacency {
        neighbours: vec![MISSING; data.indices.len()],
        boundary_halfedges: 0,
        ambiguous_halfedges: 0,
        degenerate_halfedges: 0,
    };
    for (triangle, indices) in data.indices.chunks_exact(3).enumerate() {
        for edge in 0..3 {
            let a = canonical[indices[(edge + 1) % 3] as usize];
            let b = canonical[indices[(edge + 2) % 3] as usize];
            if a == b {
                result.degenerate_halfedges += 1;
            } else {
                records.push(Record {
                    key: [a.min(b), a.max(b), 0],
                    index: (triangle * 3 + edge) as u32,
                });
            }
        }
    }
    records.par_sort_unstable_by_key(|r| (r.key[0], r.key[1]));
    let mut start = 0;
    while start < records.len() {
        let mut end = start + 1;
        while end < records.len() && records[end].key == records[start].key {
            end += 1;
        }
        let group = &records[start..end];
        if let [a, b] = group {
            let opposite = |r: &Record| canonical[data.indices[r.index as usize] as usize];
            if a.index / 3 != b.index / 3 && opposite(a) != opposite(b) {
                result.neighbours[a.index as usize] = b.index / 3;
                result.neighbours[b.index as usize] = a.index / 3;
            } else {
                result.ambiguous_halfedges += 2;
            }
        } else if group.len() == 1 {
            result.boundary_halfedges += 1;
        } else {
            result.ambiguous_halfedges += group.len();
        }
        start = end;
    }
    result
}

/// Owned immutable CPU geometry prepared for upload. Created by
/// `Renderer::prepare_mesh_uploads`; moving source vectors avoids geometry copies
/// and keeps the adjacency inseparable from the exact vertices/indices it describes.
pub struct MeshUpload {
    pub(crate) primitive: crate::asset::Primitive,
    pub(crate) adjacency: Option<Adjacency>,
}

pub(crate) fn prepare_uploads(
    primitives: Vec<crate::asset::Primitive>,
    eligible: Vec<bool>,
) -> Vec<MeshUpload> {
    let prepare = |(primitive, eligible): (crate::asset::Primitive, bool)| {
        let adjacency = eligible.then(|| {
            build(&MeshData {
                vertices: &primitive.vertices,
                indices: &primitive.indices,
            })
        });
        MeshUpload {
            primitive,
            adjacency,
        }
    };
    #[cfg(test)]
    if LEGACY.with(std::cell::Cell::get) {
        // Reproduce the previous upload-time construction for startup timing.
        return primitives
            .into_iter()
            .map(|primitive| MeshUpload {
                primitive,
                adjacency: None,
            })
            .collect();
    }
    // Indexed collect preserves primitive order independently of scheduling.
    primitives
        .into_par_iter()
        .zip(eligible)
        .map(prepare)
        .collect()
}

#[cfg(test)]
thread_local! {
    pub(crate) static LEGACY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// Frozen pre-parallel builder: independent byte-for-byte CPU/GPU oracle.
#[cfg(test)]
pub(crate) fn legacy(data: &MeshData<'_>) -> Adjacency {
    use std::collections::HashMap;
    let mut positions = HashMap::with_capacity(data.vertices.len());
    let canonical: Vec<_> = data
        .vertices
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let key = v.position.map(|x| if x == 0. { 0 } else { x.to_bits() });
            *positions.entry(key).or_insert(i as u32)
        })
        .collect();
    drop(positions);
    #[derive(Clone, Copy)]
    struct Edge {
        key: u64,
        half: u32,
    }
    let mut edges = Vec::with_capacity(data.indices.len());
    let mut result = Adjacency {
        neighbours: vec![MISSING; data.indices.len()],
        boundary_halfedges: 0,
        ambiguous_halfedges: 0,
        degenerate_halfedges: 0,
    };
    for (triangle, indices) in data.indices.chunks_exact(3).enumerate() {
        for edge in 0..3 {
            let a = canonical[indices[(edge + 1) % 3] as usize];
            let b = canonical[indices[(edge + 2) % 3] as usize];
            if a == b {
                result.degenerate_halfedges += 1;
                continue;
            }
            edges.push(Edge {
                key: (u64::from(a.min(b)) << 32) | u64::from(a.max(b)),
                half: (triangle * 3 + edge) as u32,
            });
        }
    }
    edges.sort_unstable_by_key(|e| e.key);
    let mut start = 0;
    while start < edges.len() {
        let mut end = start + 1;
        while end < edges.len() && edges[end].key == edges[start].key {
            end += 1;
        }
        let group = &edges[start..end];
        if let [a, b] = group {
            let opposite = |e: Edge| canonical[data.indices[e.half as usize] as usize];
            if a.half / 3 != b.half / 3 && opposite(*a) != opposite(*b) {
                result.neighbours[a.half as usize] = b.half / 3;
                result.neighbours[b.half as usize] = a.half / 3;
            } else {
                result.ambiguous_halfedges += 2;
            }
        } else if group.len() == 1 {
            result.boundary_halfedges += 1;
        } else {
            result.ambiguous_halfedges += group.len();
        }
        start = end;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vertex;
    fn fixture(p: &[[f32; 3]], indices: &[u32]) -> Adjacency {
        let vertices: Vec<_> = p
            .iter()
            .map(|&position| Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [0.; 2],
                color: [1.; 4],
            })
            .collect();
        build(&MeshData {
            vertices: &vertices,
            indices,
        })
    }
    #[test]
    fn exact_position_seams_signed_zero_and_edge_order() {
        let p = [
            [0., 0., 0.],
            [1., 0., 1.],
            [0., 1., 0.],
            [-0., 0., 0.],
            [0., 1., 0.],
            [-1., 0., 1.],
        ];
        let a = fixture(&p, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(a.neighbours, [MISSING, 1, MISSING, MISSING, MISSING, 0]);
        assert_eq!(a.boundary_halfedges, 4);
        assert_eq!(a.ambiguous_halfedges, 0);
        assert_eq!(a.neighbours, fixture(&p, &[0, 1, 2, 3, 4, 5]).neighbours);
    }
    #[test]
    fn nonmanifold_duplicate_and_degenerate_edges_keep_the_query_fallback() {
        let p = [
            [0., 0., 0.],
            [1., 0., 0.],
            [0., 1., 0.],
            [0., -1., 0.],
            [0., 0., 1.],
        ];
        let a = fixture(&p, &[0, 1, 2, 1, 0, 3, 0, 1, 4]);
        assert_eq!(a.ambiguous_halfedges, 3);
        assert!(a.neighbours.iter().all(|&n| n == MISSING));
        let duplicate = fixture(&p, &[0, 1, 2, 0, 1, 2]);
        assert_eq!(duplicate.ambiguous_halfedges, 6);
        assert!(duplicate.neighbours.iter().all(|&n| n == MISSING));
        let degenerate = fixture(&p, &[0, 0, 1]);
        assert_eq!(degenerate.degenerate_halfedges, 1);
        assert!(degenerate.neighbours.iter().all(|&n| n == MISSING));
    }

    #[test]
    fn parallel_welding_matches_legacy_across_sort_partitions() {
        // Duplicate every corner (UV seams), reverse alternating cell winding,
        // and include duplicate/nonmanifold/degenerate triangles across partitions.
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for y in 0..64 {
            for x in 0..64 {
                for p in [
                    [x, y],
                    [x + 1, y],
                    [x, y + 1],
                    [x + 1, y],
                    [x + 1, y + 1],
                    [x, y + 1],
                ] {
                    indices.push(vertices.len() as u32);
                    vertices.push(Vertex {
                        position: [p[0] as f32, p[1] as f32, -0.],
                        normal: [0., 0., 1.],
                        uv: [0.; 2],
                        color: [1.; 4],
                    });
                }
                if (x + y) % 2 == 0 {
                    let n = indices.len();
                    indices.swap(n - 1, n - 2);
                }
            }
        }
        indices.extend_from_within(..9);
        indices.extend_from_slice(&[0, 0, 1]);
        let data = MeshData {
            vertices: &vertices,
            indices: &indices,
        };
        let expected = legacy(&data);
        for threads in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            assert_eq!(pool.install(|| build(&data)), expected);
        }
    }
}
