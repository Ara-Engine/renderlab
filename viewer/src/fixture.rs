use renderlab::{Camera, Draw, Mat4, Mesh, MeshData, Vec3, Vertex};

pub const INDICES: [u32; 36] = [
    0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 3, 7, 6, 3, 6, 2, 0, 4, 7, 0, 7, 3, 1, 2,
    6, 1, 6, 5,
];
pub fn cube() -> [Vertex; 8] {
    [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ]
    .map(|position| Vertex {
        color: [
            0.25 + (position[0] + 0.5) * 0.7,
            0.15 + (position[1] + 0.5) * 0.65,
            0.12 + (position[2] + 0.5) * 0.7,
        ],
        position,
    })
}
pub const TRIANGLE: [Vertex; 3] = [
    Vertex {
        position: [-1.5, -0.8, -1.2],
        color: [0.1, 0.8, 0.35],
    },
    Vertex {
        position: [1.5, -0.8, -1.2],
        color: [0.15, 0.35, 0.95],
    },
    Vertex {
        position: [0.0, 1.4, -1.2],
        color: [0.95, 0.7, 0.12],
    },
];

#[derive(Debug)]
pub struct Scene {
    pub yaw: f32,
    pub elevation: f32,
    pub translation: Vec3,
    pub phase: f32,
    pub animated: bool,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            yaw: 0.35,
            elevation: 0.3,
            translation: Vec3::ZERO,
            phase: 0.0,
            animated: false,
        }
    }
}
impl Scene {
    pub fn camera(&self) -> Camera {
        let eye = Vec3::new(
            self.yaw.sin() * self.elevation.cos(),
            self.elevation.sin(),
            self.yaw.cos() * self.elevation.cos(),
        ) * 5.0
            + self.translation;
        Camera::look_at(
            eye,
            self.translation,
            Vec3::Y,
            55.0_f32.to_radians(),
            0.1,
            100.0,
        )
        .unwrap()
    }
    pub fn draws<'a>(&self, meshes: &'a [Mesh]) -> [Draw<'a>; 3] {
        [
            Draw {
                mesh: &meshes[0],
                model: Mat4::from_translation(Vec3::new(-0.65, 0.0, 0.25))
                    * Mat4::from_rotation_y(self.phase),
            },
            Draw {
                mesh: &meshes[0],
                model: Mat4::from_translation(Vec3::new(0.8, -0.1, -0.2))
                    * Mat4::from_scale(Vec3::splat(0.8)),
            },
            Draw {
                mesh: &meshes[1],
                model: Mat4::IDENTITY,
            },
        ]
    }
}
pub fn upload_next(
    renderer: &mut renderlab::Renderer,
    meshes: &mut Vec<Mesh>,
) -> Result<(), renderlab::Error> {
    let vertices = cube();
    let data = if meshes.is_empty() {
        MeshData {
            vertices: &vertices,
            indices: &INDICES,
        }
    } else {
        MeshData {
            vertices: &TRIANGLE,
            indices: &[0, 1, 2],
        }
    };
    if meshes.len() < 2
        && let Some(mesh) = renderer.try_upload_mesh(&data)?
    {
        meshes.push(mesh);
    }
    Ok(())
}
