//! Skinned meshes on the engine entry. A palette is applied on the CPU when it is set,
//! with SK1's linear blend skinning (`shaders/skinning.wgsl`, `skin_vertices`) written out
//! in the same f32 operations. The frame then draws the posed vertices as geometry
//! rebuilt for each pose, with the pose before for reconstruction's motion.
use crate::{
    Error, Mat4, Vec3, Vertex,
    asset::{Primitive, TangentStream, skin::Influence},
};

/// A skinned mesh: its bind pose and influences, and the pose it is drawn in.
pub(super) struct Skinned {
    /// Its identity to the frames: the draw's lasting name, whatever the pose.
    pub key: u64,
    source: Vec<Vertex>,
    tangent_source: Option<TangentStream>,
    influences: Vec<Influence>,
    palette: Vec<Mat4>,
    pub indices: Vec<u32>,
    /// The vertices of the pose drawn next, and their tangent frames.
    pub vertices: Vec<Vertex>,
    pub tangents: Option<TangentStream>,
    /// The vertices of the pose before it, for motion: the same as `vertices` until a
    /// second pose is set, as SK1's first dispatch leaves them.
    pub previous: Vec<Vertex>,
    /// Counts the poses set, from 1 for the bind pose (`SkinnedMesh::revision`).
    pub revision: u64,
}

impl Skinned {
    /// Checks the influences as `Renderer::try_upload_skinned_mesh` does: one per
    /// vertex, joints below `joint_count`, finite nonnegative weights summing to one.
    pub fn new(
        key: u64,
        primitive: &Primitive,
        influences: &[Influence],
        joint_count: u32,
    ) -> Result<Self, Error> {
        if influences.len() != primitive.vertices.len()
            || joint_count == 0
            || joint_count > 65536
            || influences.iter().any(|v| {
                v.joints.iter().any(|&j| u32::from(j) >= joint_count)
                    || v.weights.iter().any(|w| !w.is_finite() || *w < 0.)
                    || (v.weights.iter().sum::<f32>() - 1.).abs() > 1e-4
            })
        {
            return Err(Error::InvalidInput(
                "invalid skin vertex influences or joint count",
            ));
        }
        if let Some(tangents) = &primitive.tangents {
            crate::asset::validate_tangents(&primitive.vertices, &primitive.indices, tangents)
                .map_err(|_| Error::InvalidInput("invalid unit tangent frame or handedness"))?;
        }
        Ok(Self {
            key,
            source: primitive.vertices.clone(),
            tangent_source: primitive.tangents.clone(),
            influences: influences.to_vec(),
            palette: vec![Mat4::IDENTITY; joint_count as usize],
            indices: primitive.indices.clone(),
            vertices: primitive.vertices.clone(),
            tangents: primitive.tangents.clone(),
            previous: primitive.vertices.clone(),
            revision: 1,
        })
    }

    /// Poses the mesh with mesh-local joint matrices, checked as the wgpu engine checks
    /// them. The same palette again changes nothing.
    pub fn set_palette(&mut self, palette: &[Mat4]) -> Result<(), Error> {
        if palette.len() != self.palette.len()
            || palette
                .iter()
                .any(|m| !m.is_finite() || !crate::normal::NormalTransform::new(*m).valid())
        {
            return Err(Error::InvalidInput(
                "skin palette must contain finite nonsingular affine joint transforms",
            ));
        }
        if self.palette == palette {
            return Ok(());
        }
        let mut vertices = Vec::with_capacity(self.source.len());
        let mut tangents = self.tangent_source.as_ref().map(|t| TangentStream {
            basis: t.basis,
            values: Vec::with_capacity(t.values.len()),
        });
        for (i, (source, influence)) in self.source.iter().zip(&self.influences).enumerate() {
            let tangent = self.tangent_source.as_ref().map(|t| t.values[i]);
            let (vertex, tangent) = skin(source, influence, palette, tangent);
            if !vertex.position.iter().all(|v| v.is_finite()) {
                return Err(Error::InvalidInput("skinned position overflow"));
            }
            vertices.push(vertex);
            if let (Some(out), Some(tangent)) = (&mut tangents, tangent) {
                out.values.push(tangent);
            }
        }
        // SK1's first dispatch copies the pose into the previous stream too.
        self.previous = if self.revision == 1 {
            vertices.clone()
        } else {
            std::mem::take(&mut self.vertices)
        };
        self.vertices = vertices;
        self.tangents = tangents;
        self.palette.copy_from_slice(palette);
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
}

/// `skin_vertices` for one vertex: the blended matrix, the position, the normal through
/// the cofactor matrix (flipped with a negative determinant), and the tangent turned by
/// the matrix and made perpendicular to the normal.
fn skin(
    source: &Vertex,
    influence: &Influence,
    palette: &[Mat4],
    tangent: Option<[f32; 4]>,
) -> (Vertex, Option<[f32; 4]>) {
    let (j, w) = (influence.joints.map(usize::from), influence.weights);
    let m =
        palette[j[0]] * w[0] + palette[j[1]] * w[1] + palette[j[2]] * w[2] + palette[j[3]] * w[3];
    let p = (m * Vec3::from_array(source.position).extend(1.)).truncate();
    let n0 = Vec3::from_array(source.normal);
    let (a, b, c) = (
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    );
    let cofactor = b.cross(c) * n0.x + c.cross(a) * n0.y + a.cross(b) * n0.z;
    let sign = if a.dot(b.cross(c)) >= 0. { 1. } else { -1. };
    let n = safe_normal(cofactor * sign, n0);
    let tangent = tangent.map(|t| {
        let q = a * t[0] + b * t[1] + c * t[2];
        let u = safe_normal(q - n * q.dot(n), Vec3::new(t[0], t[1], t[2]));
        [u.x, u.y, u.z, t[3] * sign]
    });
    (
        Vertex {
            position: p.to_array(),
            normal: n.to_array(),
            ..*source
        },
        tangent,
    )
}

fn safe_normal(v: Vec3, fallback: Vec3) -> Vec3 {
    let l = v.dot(v);
    if l > 1e-20 {
        v * (1. / l.sqrt())
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A joint turned a quarter about +Y moves a vertex it fully weights by that turn,
    /// normal and tangent included; the first pose is also the previous one, and the
    /// second keeps the first as previous. The same palette twice is one pose.
    #[test]
    fn a_turned_joint_turns_its_vertices_and_keeps_the_pose_before() -> Result<(), Error> {
        let vertex = Vertex {
            position: [1., 0., 0.],
            normal: [0., 0., 1.],
            uv: [0.5, 0.5],
            color: [1.; 4],
        };
        let primitive = Primitive {
            vertices: vec![vertex; 3],
            tangents: Some(TangentStream {
                basis: crate::asset::TangentBasis::Authored,
                values: vec![[1., 0., 0., 1.]; 3],
            }),
            indices: vec![0, 1, 2],
            material: 0,
        };
        let full = Influence {
            joints: [1, 0, 0, 0],
            weights: [1., 0., 0., 0.],
        };
        let mut mesh = Skinned::new(7, &primitive, &vec![full; 3], 2)?;
        let turn = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        mesh.set_palette(&[Mat4::IDENTITY, turn])?;
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6);
        let v = mesh.vertices[0];
        assert!(close(v.position, [0., 0., -1.]), "{:?}", v.position);
        assert!(close(v.normal, [1., 0., 0.]), "{:?}", v.normal);
        let t = mesh.tangents.as_ref().map(|t| t.values[0]);
        assert!(t.is_some_and(|t| close([t[0], t[1], t[2]], [0., 0., -1.]) && t[3] == 1.));
        assert_eq!((mesh.previous[0], mesh.revision), (v, 2));
        mesh.set_palette(&[Mat4::IDENTITY, turn])?;
        assert_eq!(mesh.revision, 2);
        mesh.set_palette(&[Mat4::IDENTITY, Mat4::IDENTITY])?;
        assert_eq!(
            (mesh.previous[0], mesh.vertices[0], mesh.revision),
            (v, vertex, 3)
        );
        assert!(mesh.set_palette(&[Mat4::IDENTITY]).is_err());
        assert!(mesh.set_palette(&[Mat4::IDENTITY, Mat4::ZERO]).is_err());
        Ok(())
    }
}
