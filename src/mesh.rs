use crate::{Error, Instances, Mat4, Vec3, Visibility, observe::Owned, visibility::Aabb};

pub use renderlab_asset::Vertex;

/// Finite vertex attributes with nonzero normals, including for the raw unlit path.
pub struct MeshData<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum slots per instance collection, not the number of frame draws.
    pub max_instances: u32,
    pub max_mesh_bytes: u64,
    /// Per immutable texture upload, including every prepared mip.
    pub max_texture_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_instances: 1024,
            max_mesh_bytes: 64 * 1024 * 1024,
            max_texture_bytes: 64 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self, device: &wgpu::Limits) -> Result<(), Error> {
        if self.max_instances == 0
            || u64::from(self.max_instances) * 64 > device.max_buffer_size
            || u64::from(self.max_instances) * 64 > device.max_storage_buffer_binding_size
            || device.max_storage_buffers_per_shader_stage < 2
            || device.max_bind_groups < 3
            || device.max_vertex_buffers < 2
            || device.max_vertex_attributes < 5
            || device.max_vertex_buffer_array_stride < 48
            || device.max_sampled_textures_per_shader_stage < 2
            || device.max_samplers_per_shader_stage < 2
            || self.max_texture_bytes == 0
            || self.max_mesh_bytes == 0
            || self.max_mesh_bytes > device.max_buffer_size
        {
            return Err(Error::InvalidInput(
                "instance/mesh limits must fit device buffers and vertex-stage bindings",
            ));
        }
        Ok(())
    }
}

/// Immutable geometry. Drop releases caller ownership; wgpu retains submitted uses.
pub struct Mesh {
    pub(crate) renderer: std::sync::Arc<()>,
    pub(crate) vertex: Owned<wgpu::Buffer>,
    pub(crate) index: Owned<wgpu::Buffer>,
    pub(crate) index_count: u32,
    pub(crate) bounds: Aabb,
}
pub struct Draw<'a> {
    pub(crate) mesh: &'a Mesh,
    pub(crate) instances: &'a mut Instances,
    pub(crate) visibility: Visibility,
    pub(crate) material: Option<&'a crate::Material>,
}
impl<'a> Draw<'a> {
    pub fn new(mesh: &'a Mesh, instances: &'a mut Instances) -> Self {
        Self {
            mesh,
            instances,
            visibility: Visibility::Frustum,
            material: None,
        }
    }
    /// Omission uses linear vertex color with a white, double-sided default material.
    pub fn with_material(mut self, material: &'a crate::Material) -> Self {
        self.material = Some(material);
        self
    }
    /// Frustum assumes camera * model * original bounded vertex positions.
    /// Use Unculled for position-changing shaders; layout validation cannot check this contract.
    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }
}

impl MeshData<'_> {
    pub(crate) fn validate(&self, limits: Limits) -> Result<(u64, u64), Error> {
        if self.vertices.is_empty()
            || self.indices.is_empty()
            || !self.indices.len().is_multiple_of(3)
            || self.indices.len() > u32::MAX as usize
        {
            return Err(Error::InvalidInput(
                "mesh needs vertices and a nonempty u32 triangle list",
            ));
        }
        let vertex_bytes = (self.vertices.len() as u64).checked_mul(48);
        let index_bytes = (self.indices.len() as u64).checked_mul(4);
        let (Some(vertex_bytes), Some(index_bytes)) = (vertex_bytes, index_bytes) else {
            return Err(Error::InvalidInput("mesh byte count overflow"));
        };
        if vertex_bytes
            .checked_add(index_bytes)
            .is_none_or(|bytes| bytes > limits.max_mesh_bytes)
        {
            return Err(Error::InvalidInput("mesh exceeds max_mesh_bytes"));
        }
        if self
            .indices
            .iter()
            .any(|&i| i as usize >= self.vertices.len())
        {
            return Err(Error::InvalidInput("mesh index outside vertices"));
        }
        if self.vertices.iter().any(|v| {
            v.position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.color)
                .any(|x| !x.is_finite())
        }) {
            return Err(Error::InvalidInput("vertex attributes must be finite"));
        }
        if self
            .vertices
            .iter()
            .any(|v| v.normal.iter().all(|n| *n == 0.0))
        {
            return Err(Error::InvalidInput("vertex normal must be nonzero"));
        }
        Ok((vertex_bytes, index_bytes))
    }
}

/// Right-handed camera with WebGPU's 0..1 depth range. Aspect follows the color texture.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    view: Mat4,
    fov_y: f32,
    near: f32,
    far: f32,
}
impl Camera {
    pub fn look_at(
        eye: Vec3,
        target: Vec3,
        up: Vec3,
        fov_y: f32,
        near: f32,
        far: f32,
    ) -> Result<Self, Error> {
        let direction = target - eye;
        if !eye.is_finite()
            || !target.is_finite()
            || !up.is_finite()
            || !fov_y.is_finite()
            || !near.is_finite()
            || !far.is_finite()
            || direction.length_squared() < 1e-12
            || up.length_squared() < 1e-12
            || direction.normalize().cross(up.normalize()).length_squared() < 1e-12
            || !(0.0..std::f32::consts::PI).contains(&fov_y)
            || fov_y == 0.0
            || near <= 0.0
            || far <= near
        {
            return Err(Error::InvalidInput(
                "invalid look-at vectors or perspective planes/FOV",
            ));
        }
        let camera = Self {
            view: Mat4::look_at_rh(eye, target, up),
            fov_y,
            near,
            far,
        };
        if !camera.matrix(1, 1).is_finite() {
            return Err(Error::InvalidInput("camera matrix overflow"));
        }
        Ok(camera)
    }
    /// Exact right-handed view used by rasterization; forward is -Z.
    pub fn view(self) -> Mat4 {
        self.view
    }
    pub fn fov_y(self) -> f32 {
        self.fov_y
    }
    pub fn near(self) -> f32 {
        self.near
    }
    pub fn matrix(self, width: u32, height: u32) -> Mat4 {
        Mat4::perspective_rh(
            self.fov_y,
            width as f32 / height as f32,
            self.near,
            self.far,
        ) * self.view
    }
}

pub(crate) fn matrix_bytes(matrix: Mat4) -> [u8; 64] {
    let mut bytes = [0; 64];
    for (chunk, value) in bytes.chunks_exact_mut(4).zip(matrix.to_cols_array()) {
        chunk.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}
