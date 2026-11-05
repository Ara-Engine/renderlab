//! Exact-position shared edges for the primary receiver repair. No topology guesses.
use crate::MeshData;
use std::collections::HashMap;

pub(crate) const MISSING: u32 = u32::MAX;

pub(crate) struct Adjacency {
    /// Triangle-major, opposite-vertex edge order, matching barycentric weights.
    pub neighbours: Vec<u32>,
    pub boundary_halfedges: usize,
    pub ambiguous_halfedges: usize,
    pub degenerate_halfedges: usize,
}

pub(crate) fn build(data: &MeshData<'_>) -> Adjacency {
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
}
