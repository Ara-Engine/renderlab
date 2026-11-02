use renderlab::{
    Camera, Draw, Instances, Mat4, Material, MaterialDescriptor, Mesh, MeshData, Renderer, Texture,
    TextureBinding, Vec3, Visibility, asset,
};
use std::{
    fs::File,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadProgress {
    Textures(usize),
    Materials(usize),
    Geometry(usize),
    Instances(usize),
    Ready,
}
struct Batch {
    primitive: usize,
    models: Vec<Mat4>,
}
pub struct Prepared {
    source: asset::Asset,
    textures: Vec<Texture>,
    materials: Vec<Material>,
    meshes: Vec<Mesh>,
    instances: Vec<Instances>,
    batches: Vec<Batch>,
    primitives: Vec<(usize, usize)>,
    progress: UploadProgress,
    upload_started: Option<Instant>,
    upload_cpu: Duration,
    phase: f32,
    pub center: Vec3,
    pub radius: f32,
}
impl Prepared {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let limits = asset::Limits::default();
        let start = Instant::now();
        let mut bytes = Vec::new();
        File::open(path)?
            .take(limits.max_file_bytes as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limits.max_file_bytes {
            return Err("prepared file exceeds input byte limit".into());
        }
        let io = start.elapsed();
        let start = Instant::now();
        let source = asset::decode(&bytes, limits)?;
        eprintln!(
            "asset_read path={} bytes={} io_ms={:.3} decode_ms={:.3} counts={:?}",
            path.display(),
            bytes.len(),
            io.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1000.0,
            source.validate(limits)?
        );
        Self::new(source)
    }
    pub fn new(source: asset::Asset) -> Result<Self, Box<dyn std::error::Error>> {
        source.validate(asset::Limits::default())?;
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut by_mesh = vec![Vec::new(); source.meshes.len()];
        for p in &source.placements {
            by_mesh[p.mesh as usize].push(Mat4::from_cols_array_2d(&p.transform));
        }
        let slots = source
            .meshes
            .iter()
            .zip(&by_mesh)
            .try_fold(0usize, |total, (mesh, models)| {
                total.checked_add(mesh.primitives.len().checked_mul(models.len())?)
            });
        if slots.is_none_or(|n| n > 1_000_000) {
            return Err("asset exceeds viewer's one million primitive-placement slot limit".into());
        }
        let mut batches = Vec::new();
        let mut primitives = Vec::new();
        for (mesh_id, (mesh, models)) in source.meshes.iter().zip(by_mesh).enumerate() {
            for (primitive_id, primitive) in mesh.primitives.iter().enumerate() {
                let primitive_index = primitives.len();
                primitives.push((mesh_id, primitive_id));
                let mut local_min = Vec3::splat(f32::INFINITY);
                let mut local_max = Vec3::splat(f32::NEG_INFINITY);
                for vertex in &primitive.vertices {
                    let p = Vec3::from_array(vertex.position);
                    local_min = local_min.min(p);
                    local_max = local_max.max(p);
                }
                for model in &models {
                    for corner in 0..8 {
                        let p = Vec3::new(
                            if corner & 1 == 0 {
                                local_min.x
                            } else {
                                local_max.x
                            },
                            if corner & 2 == 0 {
                                local_min.y
                            } else {
                                local_max.y
                            },
                            if corner & 4 == 0 {
                                local_min.z
                            } else {
                                local_max.z
                            },
                        );
                        let p = model.transform_point3(p);
                        if !p.is_finite() {
                            return Err("asset framing transform overflow".into());
                        }
                        min = min.min(p);
                        max = max.max(p);
                    }
                }
                if !models.is_empty() {
                    batches.push(Batch {
                        primitive: primitive_index,
                        models: models.clone(),
                    });
                }
            }
        }
        if !min.is_finite() || !max.is_finite() {
            return Err("asset has no finite placed geometry".into());
        }
        let center = min * 0.5 + max * 0.5;
        let radius = ((max - min) * 0.5).length().max(0.1);
        if !center.is_finite()
            || !radius.is_finite()
            || center.abs().max_element() > 1e12
            || radius > 1e12
        {
            return Err("asset framing extent exceeds supported numerical range".into());
        }
        Ok(Self {
            source,
            textures: Vec::new(),
            materials: Vec::new(),
            meshes: Vec::new(),
            instances: Vec::new(),
            batches,
            primitives,
            progress: UploadProgress::Textures(0),
            upload_started: None,
            upload_cpu: Duration::ZERO,
            phase: 0.0,
            center,
            radius,
        })
    }
    /// Each accepted resource advances exactly once. Busy retains all preceding uploads.
    pub fn advance(&mut self, r: &mut Renderer) -> Result<(), renderlab::Error> {
        if self.ready() {
            return Ok(());
        }
        self.upload_started.get_or_insert_with(Instant::now);
        let start = Instant::now();
        while !self.ready() {
            let before = self.progress;
            self.advance_one(r)?;
            if self.progress == before || start.elapsed() >= Duration::from_millis(2) {
                break;
            }
        }
        Ok(())
    }
    fn advance_one(&mut self, r: &mut Renderer) -> Result<(), renderlab::Error> {
        let start = Instant::now();
        match self.progress {
            UploadProgress::Textures(i) => {
                if let Some(data) = self.source.textures.get(i) {
                    if let Some(t) = r.try_upload_texture(data)? {
                        self.textures.push(t);
                        self.progress = UploadProgress::Textures(i + 1);
                    }
                } else {
                    self.progress = UploadProgress::Materials(0);
                }
            }
            UploadProgress::Materials(i) => {
                if let Some(data) = self.source.materials.get(i) {
                    let binding = |id: u32, sampler| TextureBinding {
                        texture: &self.textures[id as usize],
                        sampler,
                    };
                    let descriptor = MaterialDescriptor {
                        base_color: data.base_color,
                        metallic: data.metallic,
                        roughness: data.roughness,
                        double_sided: data.double_sided,
                        base_color_texture: data
                            .base_color_texture
                            .map(|t| binding(t.texture, t.sampler)),
                        metallic_roughness_texture: data
                            .metallic_roughness_texture
                            .map(|t| binding(t.texture, t.sampler)),
                    };
                    if let Some(m) = r.try_create_material(&descriptor)? {
                        self.materials.push(m);
                        self.progress = UploadProgress::Materials(i + 1);
                    }
                } else {
                    self.progress = UploadProgress::Geometry(0);
                }
            }
            UploadProgress::Geometry(i) => {
                if let Some(&(mesh, primitive)) = self.primitives.get(i) {
                    let p = &self.source.meshes[mesh].primitives[primitive];
                    if let Some(m) = r.try_upload_mesh(&MeshData {
                        vertices: &p.vertices,
                        indices: &p.indices,
                    })? {
                        self.meshes.push(m);
                        self.progress = UploadProgress::Geometry(i + 1);
                    }
                } else {
                    self.progress = UploadProgress::Instances(0);
                }
            }
            UploadProgress::Instances(i) => {
                if let Some(batch) = self.batches.get(i) {
                    if let Some(instances) = r.try_upload_instances(&batch.models)? {
                        self.instances.push(instances);
                        self.progress = UploadProgress::Instances(i + 1);
                    }
                } else {
                    self.progress = UploadProgress::Ready;
                    eprintln!(
                        "asset_uploaded upload_wall_ms={:.3} setup_attempt_cpu_ms={:.3} primitives={} batches={} textures={} materials={} uploads={:?}",
                        self.upload_started.unwrap().elapsed().as_secs_f64() * 1000.0,
                        self.upload_cpu.as_secs_f64() * 1000.0,
                        self.meshes.len(),
                        self.batches.len(),
                        self.textures.len(),
                        self.materials.len(),
                        r.stats().uploads
                    );
                }
            }
            UploadProgress::Ready => {}
        }
        self.upload_cpu += start.elapsed();
        Ok(())
    }
    pub fn ready(&self) -> bool {
        self.progress == UploadProgress::Ready
    }
    pub fn max_instances(&self) -> u32 {
        self.batches
            .iter()
            .map(|b| b.models.len() as u32)
            .max()
            .unwrap_or(1)
    }
    pub fn animate(&mut self, phase: f32) -> Result<(), renderlab::Error> {
        if self.phase.to_bits() == phase.to_bits() {
            return Ok(());
        }
        let rotation = Mat4::from_translation(self.center)
            * Mat4::from_rotation_y(phase)
            * Mat4::from_translation(-self.center);
        for (instances, batch) in self.instances.iter_mut().zip(&self.batches) {
            for (slot, base) in batch.models.iter().enumerate() {
                instances.set(slot, rotation * *base)?;
            }
        }
        self.phase = phase;
        Ok(())
    }
    pub fn camera(
        &self,
        yaw: f32,
        elevation: f32,
        translation: Vec3,
        near_view: bool,
        aspect: f32,
    ) -> Result<Camera, renderlab::Error> {
        let target = self.center + translation * self.radius * 0.4;
        let half_fov = 27.5_f32.to_radians();
        let angle = half_fov.min((half_fov.tan() * aspect).atan());
        let distance = self.radius / angle.sin() * if near_view { 0.65 } else { 1.1 };
        let direction = Vec3::new(
            yaw.sin() * elevation.cos(),
            elevation.sin(),
            yaw.cos() * elevation.cos(),
        );
        Camera::look_at(
            target + direction * distance,
            target,
            Vec3::Y,
            half_fov * 2.0,
            (self.radius * 0.001).max(1e-4),
            distance + self.radius * 4.0,
        )
    }
    pub fn draws(&mut self, visibility: Visibility) -> Vec<Draw<'_>> {
        self.instances
            .iter_mut()
            .zip(&self.batches)
            .map(|(instances, batch)| {
                let (mesh, primitive) = self.primitives[batch.primitive];
                let material = self.source.meshes[mesh].primitives[primitive].material;
                Draw::new(&self.meshes[batch.primitive], instances)
                    .with_material(&self.materials[material as usize])
                    .with_visibility(visibility)
            })
            .collect()
    }
}
