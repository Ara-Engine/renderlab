//! Cooker-only tangent preparation. No prepared-asset format or cache changes.
use crate::{Result, ensure};
use renderlab_asset::Vertex;
use std::collections::HashMap;

/// Absence selects generation. Invalid authored frames are errors, not a fallback.
#[derive(Clone, Copy, Debug)]
pub enum TangentSource<'a> {
    Authored(&'a [[f32; 4]]),
    GenerateMikkTSpace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TangentMode {
    Authored,
    MikkTSpace,
}

/// Base vertices and tangents share one index domain, including every seam split.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedTangentMesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    tangents: Vec<[f32; 4]>,
    mode: TangentMode,
}

impl PreparedTangentMesh {
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
    pub fn tangents(&self) -> &[[f32; 4]] {
        &self.tangents
    }
    pub fn mode(&self) -> TangentMode {
        self.mode
    }
    pub fn into_parts(self) -> (Vec<Vertex>, Vec<u32>, Vec<[f32; 4]>) {
        (self.vertices, self.indices, self.tangents)
    }
}

const FRAME_TOLERANCE: f64 = 2e-4;
const MIN_RELATIVE_AREA: f64 = 1e-6;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn frame(normal: [f32; 3], tangent: [f32; 4]) -> Result<()> {
    let t = [tangent[0], tangent[1], tangent[2]].map(f64::from);
    let n = normal.map(f64::from);
    ensure(
        tangent.iter().all(|v| v.is_finite())
            && matches!(tangent[3], -1.0 | 1.0)
            && (dot(t, t) - 1.0).abs() <= FRAME_TOLERANCE
            && dot(n, t).abs() <= FRAME_TOLERANCE,
        "tangent requires finite unit XYZ orthogonal to NORMAL and W exactly -1 or +1",
    )
}

fn validate(vertices: &[Vertex], indices: &[u32], generate: bool) -> Result<()> {
    ensure(
        !vertices.is_empty() && vertices.len() <= u32::MAX as usize,
        "tangent mesh requires vertices in the u32 index domain",
    )?;
    ensure(
        !indices.is_empty()
            && indices.len().is_multiple_of(3)
            && indices.len() <= u32::MAX as usize
            && indices.iter().all(|&i| (i as usize) < vertices.len()),
        "tangent mesh requires valid triangle indices",
    )?;
    // MikkTSpace encodes a corner as (face << 2) | corner in signed i32 storage.
    ensure(
        !generate || indices.len() / 3 <= (i32::MAX as usize) / 4,
        "MikkTSpace face index domain exceeded",
    )?;
    for vertex in vertices {
        let n = vertex.normal.map(f64::from);
        ensure(
            vertex
                .position
                .iter()
                .chain(&vertex.normal)
                .chain(&vertex.uv)
                .chain(&vertex.color)
                .all(|v| v.is_finite())
                && (dot(n, n) - 1.0).abs() <= FRAME_TOLERANCE,
            "tangent mesh requires finite attributes and unit NORMAL",
        )?;
    }
    for tri in indices.chunks_exact(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| vertices[i as usize]);
        let u = std::array::from_fn(|i| f64::from(b.position[i]) - f64::from(a.position[i]));
        let v = std::array::from_fn(|i| f64::from(c.position[i]) - f64::from(a.position[i]));
        let area = cross(u, v);
        let opposite = std::array::from_fn(|i| v[i] - u[i]);
        let longest_squared = dot(u, u).max(dot(v, v)).max(dot(opposite, opposite));
        ensure(
            dot(area, area) > MIN_RELATIVE_AREA.powi(2) * longest_squared.powi(2),
            "degenerate or near-degenerate position triangle",
        )?;
        if generate {
            let du = [b.uv[0] - a.uv[0], b.uv[1] - a.uv[1]];
            let dv = [c.uv[0] - a.uv[0], c.uv[1] - a.uv[1]];
            let det = du[0] * dv[1] - du[1] * dv[0];
            let squared = |u: [f32; 2]| u.into_iter().map(|v| f64::from(v).powi(2)).sum::<f64>();
            let longest_squared = squared(du)
                .max(squared(dv))
                .max(squared([dv[0] - du[0], dv[1] - du[1]]));
            ensure(
                det.is_normal() && f64::from(det).abs() > MIN_RELATIVE_AREA * longest_squared,
                "degenerate or near-degenerate UV triangle cannot generate tangents",
            )?;
            // The generator normalizes f32 derivative numerators. Reject its underflow domain.
            for derivative in [
                std::array::from_fn::<_, 3, _>(|i| {
                    (b.position[i] - a.position[i]) * dv[1]
                        - (c.position[i] - a.position[i]) * du[1]
                }),
                std::array::from_fn::<_, 3, _>(|i| {
                    (c.position[i] - a.position[i]) * du[0]
                        - (b.position[i] - a.position[i]) * dv[0]
                }),
            ] {
                let length_squared: f32 = derivative.iter().map(|v| v * v).sum();
                ensure(
                    length_squared.is_normal(),
                    "triangle derivatives outside MikkTSpace f32 domain",
                )?;
            }
        }
    }
    Ok(())
}

struct MikkMesh<'a> {
    vertices: &'a [Vertex],
    indices: &'a [u32],
    corners: Vec<[f32; 4]>,
}
impl MikkMesh<'_> {
    fn vertex(&self, face: usize, corner: usize) -> Vertex {
        self.vertices[self.indices[face * 3 + corner] as usize]
    }
}
impl mikktspace::Geometry for MikkMesh<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }
    fn num_vertices_of_face(&self, _: usize) -> usize {
        3
    }
    fn position(&self, face: usize, corner: usize) -> [f32; 3] {
        self.vertex(face, corner).position
    }
    fn normal(&self, face: usize, corner: usize) -> [f32; 3] {
        self.vertex(face, corner).normal
    }
    fn tex_coord(&self, face: usize, corner: usize) -> [f32; 2] {
        self.vertex(face, corner).uv
    }
    fn set_tangent_encoded(&mut self, tangent: [f32; 4], face: usize, corner: usize) {
        self.corners[face * 3 + corner] = tangent;
    }
}

/// UV and normal seams must already be represented in the base vertex domain.
/// Generation adds splits wherever MikkTSpace returns a different corner frame.
/// Valid authored frames also support degenerate UVs because no basis is inferred.
pub fn prepare(
    vertices: &[Vertex],
    indices: &[u32],
    source: TangentSource<'_>,
) -> Result<PreparedTangentMesh> {
    validate(
        vertices,
        indices,
        matches!(source, TangentSource::GenerateMikkTSpace),
    )?;
    if let TangentSource::Authored(tangents) = source {
        ensure(
            tangents.len() == vertices.len(),
            "authored tangent count differs from vertex count",
        )?;
        for (vertex, &tangent) in vertices.iter().zip(tangents) {
            frame(vertex.normal, tangent)?;
        }
        for tri in indices.chunks_exact(3) {
            ensure(
                tri.iter()
                    .all(|&i| tangents[i as usize][3] == tangents[tri[0] as usize][3]),
                "triangle has mixed tangent handedness",
            )?;
        }
        return Ok(PreparedTangentMesh {
            vertices: vertices.to_vec(),
            indices: indices.to_vec(),
            tangents: tangents.to_vec(),
            mode: TangentMode::Authored,
        });
    }
    let mut input = MikkMesh {
        vertices,
        indices,
        corners: vec![[f32::NAN; 4]; indices.len()],
    };
    ensure(
        mikktspace::generate_tangents(&mut input),
        "MikkTSpace rejected tangent mesh",
    )?;
    let mut output = PreparedTangentMesh {
        vertices: Vec::new(),
        indices: Vec::with_capacity(indices.len()),
        tangents: Vec::new(),
        mode: TangentMode::MikkTSpace,
    };
    let mut remap = HashMap::new();
    for (tri, frames) in indices.chunks_exact(3).zip(input.corners.chunks_exact(3)) {
        ensure(
            frames.iter().all(|t| t[3] == frames[0][3]),
            "MikkTSpace returned mixed triangle handedness",
        )?;
        for (&source, &tangent) in tri.iter().zip(frames) {
            frame(vertices[source as usize].normal, tangent)?;
            let key = (
                source,
                tangent.map(|v| if v == 0.0 { 0 } else { v.to_bits() }),
            );
            let index = *remap.entry(key).or_insert_with(|| {
                let i = output.vertices.len() as u32;
                output.vertices.push(vertices[source as usize]);
                output.tangents.push(tangent);
                i
            });
            output.indices.push(index);
        }
    }
    Ok(output)
}
