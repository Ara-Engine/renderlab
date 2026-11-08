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
            || device.max_vertex_buffers < 3
            || device.max_vertex_attributes < 6
            || device.max_vertex_buffer_array_stride < 48
            || device.max_sampled_textures_per_shader_stage < 4
            || device.max_samplers_per_shader_stage < 4
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

/// Static geometry, or the owned output of a SkinnedMeshInstance. Drop releases
/// caller ownership; wgpu retains submitted uses.
pub struct Mesh {
    // SK1: present only for a deforming instance, automatically dispatched each frame.
    pub(crate) skin: Option<crate::renderer::skinning::SkinGpu>,
    /// Visibility-buffer clusters: original geometry, or an attached clusterLOD hierarchy.
    pub(crate) clusters: Option<crate::renderer::visibility_buffer::Clusters>,
    pub(crate) repair_scan: bool,
    pub(crate) renderer: std::sync::Arc<()>,
    pub(crate) acceleration:
        Option<std::sync::Arc<crate::renderer::hardware_shadow_gpu::MeshAcceleration>>,
    pub(crate) tangent: Option<Owned<wgpu::Buffer>>,
    pub(crate) vertex: std::sync::Arc<Owned<wgpu::Buffer>>,
    pub(crate) index: std::sync::Arc<Owned<wgpu::Buffer>>,
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

/// Right-handed camera with finite reversed depth (near 1, far 0).
/// Aspect follows the color texture. View-space forward is -Z.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    view: Mat4,
    fov_y: f32,
    near: f32,
    depth: [f32; 2],
    /// Sub-pixel projection offset in pixels (`Camera::jittered`); zero for every user
    /// camera, so their matrices are unchanged bit for bit.
    jitter: [f32; 2],
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
        // Construct before rounding. Subtracting a rounded forward-Z row loses
        // the precision that reversed depth is meant to preserve.
        let n = f64::from(near);
        let f = f64::from(far);
        let depth = [(n / (f - n)) as f32, (n * f / (f - n)) as f32];
        if depth.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(Error::InvalidInput(
                "camera depth coefficients unrepresentable",
            ));
        }
        let camera = Self {
            view: Mat4::look_at_rh(eye, target, up),
            fov_y,
            near,
            depth,
            jitter: [0.0; 2],
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
    /// The far plane, from the depth coefficients (LC1's depth slices end there).
    pub fn far(self) -> f32 {
        self.depth[1] / self.depth[0]
    }
    /// Actual world-to-clip matrix uploaded by both main raster paths.
    /// After division by W, near maps to 1 and far to 0 in WebGPU's clip volume.
    /// Depth uses strict Greater with clear 0. Unproject near at Z=1, far at Z=0.
    /// Width and height must be nonzero.
    pub fn matrix(self, width: u32, height: u32) -> Mat4 {
        // Keep glam::Mat4::perspective_rh's X/Y/W arithmetic unchanged.
        let (sin_fov, cos_fov) = (0.5 * self.fov_y).sin_cos();
        let h = cos_fov / sin_fov;
        let w = h / (width as f32 / height as f32);
        // A jitter of (x, y) pixels moves the image so that each pixel centre samples the
        // unjittered image at its centre plus (x, y) (y down): NDC shifts by -2x/width and
        // +2y/height. The view-space Z column carries it, as clip W is -Z. No jitter keeps
        // positive zeros, so unjittered matrices are the original bits.
        let shift = if self.jitter == [0.0; 2] {
            [0.0; 2]
        } else {
            [
                2.0 * self.jitter[0] / width as f32,
                -2.0 * self.jitter[1] / height as f32,
            ]
        };
        Mat4::from_cols(
            glam::Vec4::new(w, 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, h, 0.0, 0.0),
            glam::Vec4::new(shift[0], shift[1], self.depth[0], -1.0),
            glam::Vec4::new(0.0, 0.0, self.depth[1], 0.0),
        ) * self.view
    }
    /// This camera with its pixel sample `offset` pixels from each pixel centre (y down),
    /// for temporal reconstruction's jittered frames (temporal_reconstruction.rs). Traced
    /// and visibility-buffer primaries both take it from the projection, and every
    /// unprojection inverts it, so no shader edit carries the jitter.
    pub(crate) fn jittered(self, offset: [f32; 2]) -> Self {
        Self {
            jitter: offset,
            ..self
        }
    }
    pub(crate) fn jitter(self) -> [f32; 2] {
        self.jitter
    }
    pub(crate) fn unjittered(self) -> Self {
        self.jittered([0.0; 2])
    }
}

pub(crate) fn matrix_bytes(matrix: Mat4) -> [u8; 64] {
    let mut bytes = [0; 64];
    for (chunk, value) in bytes.chunks_exact_mut(4).zip(matrix.to_cols_array()) {
        chunk.copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}
