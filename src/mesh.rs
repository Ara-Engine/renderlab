use crate::{Error, Instances, Mat4, Vec3, Visibility, observe::Owned, visibility::Aabb};

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: [f32; 3],
    /// Linear RGB.
    pub color: [f32; 3],
}

pub struct MeshData<'a> {
    pub vertices: &'a [Vertex],
    pub indices: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum slots per instance collection, not the number of frame draws.
    pub max_instances: u32,
    pub max_mesh_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_instances: 1024,
            max_mesh_bytes: 64 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self, device: &wgpu::Limits) -> Result<(), Error> {
        if self.max_instances == 0
            || u64::from(self.max_instances) * 64 > device.max_buffer_size
            || u64::from(self.max_instances) * 64 > device.max_storage_buffer_binding_size
            || device.max_storage_buffers_per_shader_stage < 1
            || device.max_bind_groups < 2
            || device.max_vertex_buffers < 2
            || device.max_vertex_attributes < 3
            || device.max_vertex_buffer_array_stride < 24
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
    pub(crate) vertex: Owned<wgpu::Buffer>,
    pub(crate) index: Owned<wgpu::Buffer>,
    pub(crate) index_count: u32,
    pub(crate) bounds: Aabb,
}
pub struct Draw<'a> {
    pub(crate) mesh: &'a Mesh,
    pub(crate) instances: &'a mut Instances,
    pub(crate) visibility: Visibility,
}
impl<'a> Draw<'a> {
    pub fn new(mesh: &'a Mesh, instances: &'a mut Instances) -> Self {
        Self {
            mesh,
            instances,
            visibility: Visibility::Frustum,
        }
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
        let vertex_bytes = (self.vertices.len() as u64).checked_mul(24);
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
        if self
            .vertices
            .iter()
            .any(|v| v.position.iter().chain(&v.color).any(|x| !x.is_finite()))
        {
            return Err(Error::InvalidInput("vertex position/color must be finite"));
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
    pub(crate) fn matrix(self, width: u32, height: u32) -> Mat4 {
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
