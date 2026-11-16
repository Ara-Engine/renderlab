//! Measurement: `RENDERLAB_SCENE_DUMP=<dir>` keeps CPU copies of the scene an embedder builds
//! (meshes, skinned meshes with their latest palettes, textures, materials, environments), so
//! one frame of a running game can be frozen and path-traced (`examples/scene_reference.rs`,
//! main-thread/ref/README.md). Armed like the frame readback: once `<dir>/arm` exists (a label
//! line), the next view rendered writes `<dir>/<label>/scene.rla` (one mesh per mesh and
//! material pair, placements at their current transforms, materials, textures) and
//! `<dir>/<label>/scene.json` (camera, output size, exposure, sun, sky, lights, each material's
//! emission), then removes the file. A procedural atmosphere sky is written as the cube shading
//! reads (`sky.f32`) with the sun as it reaches surfaces. While dumping, the engine reports no block-compressed
//! texture support, so embedders upload RGBA8 levels the tracer can read.
use super::*;
use std::{collections::HashMap, fmt::Write as _, path::PathBuf};

enum Recorded {
    Static {
        vertices: Vec<Vertex>,
        indices: Vec<u32>,
        tangents: Option<TangentStream>,
    },
    Skinned {
        primitive: Primitive,
        influences: Vec<crate::asset::skin::Influence>,
        palette: Vec<Mat4>,
    },
}

struct RecordedMaterial {
    desc: [f32; 4],
    metallic: f32,
    roughness: f32,
    textures: [Option<TextureRef>; 3],
    normal_scale: f32,
    double_sided: bool,
    emission: [f32; 3],
    transparent: bool,
}

#[derive(Default)]
pub(super) struct Recorder {
    dir: PathBuf,
    meshes: HashMap<MeshId, Recorded>,
    textures: HashMap<TextureId, Arc<TextureData>>,
    materials: HashMap<MaterialId, RecordedMaterial>,
    environments: HashMap<EnvironmentId, Arc<PreparedEnvironment>>,
}

impl Recorder {
    pub(super) fn from_env() -> Option<Self> {
        let dir = std::env::var_os("RENDERLAB_SCENE_DUMP")?;
        Some(Self { dir: dir.into(), ..Default::default() })
    }
    pub(super) fn mesh(&mut self, id: MeshId, vertices: &[Vertex], indices: &[u32], tangents: Option<&TangentStream>) {
        self.meshes.insert(id, Recorded::Static { vertices: vertices.to_vec(), indices: indices.to_vec(), tangents: tangents.cloned() });
    }
    pub(super) fn skinned(&mut self, id: MeshId, primitive: &Primitive, influences: &[crate::asset::skin::Influence]) {
        self.meshes.insert(id, Recorded::Skinned { primitive: primitive.clone(), influences: influences.to_vec(), palette: vec![] });
    }
    pub(super) fn palette(&mut self, id: MeshId, palette: &[Mat4]) {
        if let Some(Recorded::Skinned { palette: p, .. }) = self.meshes.get_mut(&id) {
            *p = palette.to_vec();
        }
    }
    pub(super) fn texture(&mut self, id: TextureId, data: Arc<TextureData>) {
        self.textures.insert(id, data);
    }
    pub(super) fn environment(&mut self, id: EnvironmentId, data: &PreparedEnvironment) {
        self.environments.insert(id, Arc::new(data.clone()));
    }
    pub(super) fn material(&mut self, id: MaterialId, desc: &MaterialDesc<'_>) {
        // Emission: the embedder's emissive program's `glow` (Ara's; the mapped glow's mean is
        // its `bounce`), else none. Standard materials do not emit.
        let vec3 = |name: &str| {
            desc.parameters.iter().find(|(n, _)| *n == name).and_then(|(_, v)| match v {
                crate::Value::Vec3(v) => Some(*v),
                _ => None,
            })
        };
        let mapped = desc.parameters.iter().any(|(n, v)| *n == "mapped" && matches!(v, crate::Value::F32(m) if *m > 0.0));
        let emission = if mapped { vec3("bounce") } else { vec3("glow") }.unwrap_or([0.0; 3]);
        self.materials.insert(id, RecordedMaterial {
            desc: desc.base_color,
            metallic: desc.metallic,
            roughness: desc.roughness,
            textures: [desc.base_color_texture, desc.metallic_roughness_texture, desc.normal_texture],
            normal_scale: desc.normal_scale,
            double_sided: desc.double_sided,
            emission,
            transparent: desc.transparency.is_some(),
        });
    }

    /// The armed label, if `<dir>/arm` exists (consumed).
    pub(super) fn armed(&self) -> Option<String> {
        let arm = self.dir.join("arm");
        // The label line ends with a newline; without one the file is still being written (an
        // empty read once dumped spinner-a as "frame"), so it waits for a later frame.
        let text = std::fs::read_to_string(&arm).ok().filter(|t| t.ends_with('\n'))?;
        let _ = std::fs::remove_file(&arm);
        Some(text.lines().next().unwrap_or("frame").trim().to_owned())
    }
}

impl Engine {
    /// Writes the frozen scene for `label` as seen by `camera` into `size` pixels (module doc).
    pub(super) fn dump_scene(&mut self, label: &str, camera: &Camera, size: [u32; 2], stops: f32) -> Result<(), Error> {
        let Some(rec) = self.dump.as_ref() else { return Ok(()) };
        let out = rec.dir.join(label);
        let _ = std::fs::create_dir_all(&out);
        let mut asset = crate::asset::Asset { meshes: vec![], placements: vec![], materials: vec![], textures: vec![] };
        let mut texture_index: HashMap<TextureId, u32> = HashMap::new();
        let mut material_index: HashMap<MaterialId, u32> = HashMap::new();
        let mut mesh_index: HashMap<(MeshId, MaterialId), u32> = HashMap::new();
        let mut emission = String::new();
        let mut skipped = 0usize;
        for batch in self.state.batches.values() {
            let (Some(recorded), Some(material)) = (rec.meshes.get(&batch.mesh), rec.materials.get(&batch.material)) else {
                skipped += batch.models.len();
                continue;
            };
            if material.transparent {
                skipped += batch.models.len();
                continue;
            }
            let m = match material_index.get(&batch.material) {
                Some(&m) => m,
                None => {
                    let mut tex = |t: Option<TextureRef>| -> Option<(u32, crate::asset::Sampler)> {
                        let t = t?;
                        let data = rec.textures.get(&t.texture)?;
                        let i = *texture_index.entry(t.texture).or_insert_with(|| {
                            asset.textures.push((**data).clone());
                            asset.textures.len() as u32 - 1
                        });
                        Some((i, t.sampler))
                    };
                    let [b, mr, n] = material.textures;
                    let m = crate::asset::Material {
                        base_color: material.desc,
                        metallic: material.metallic,
                        roughness: material.roughness,
                        base_color_texture: tex(b).map(|(texture, sampler)| crate::asset::BaseColorTexture { texture, sampler }),
                        metallic_roughness_texture: tex(mr).map(|(texture, sampler)| crate::asset::MetallicRoughnessTexture { texture, sampler }),
                        normal_texture: tex(n).map(|(texture, sampler)| crate::asset::NormalTexture { texture, sampler, scale: material.normal_scale }),
                        double_sided: material.double_sided,
                    };
                    asset.materials.push(m);
                    let index = asset.materials.len() as u32 - 1;
                    let e = material.emission;
                    let _ = write!(emission, "{}[{index},{},{},{}]", if emission.is_empty() { "" } else { "," }, e[0], e[1], e[2]);
                    material_index.insert(batch.material, index);
                    index
                }
            };
            let mesh = *mesh_index.entry((batch.mesh, batch.material)).or_insert_with(|| {
                let (vertices, indices, tangents) = match recorded {
                    Recorded::Static { vertices, indices, tangents } => (vertices.clone(), indices.clone(), tangents.clone()),
                    Recorded::Skinned { primitive, influences, palette } => (skin(primitive, influences, palette), primitive.indices.clone(), None),
                };
                asset.meshes.push(crate::asset::Mesh { primitives: vec![Primitive { vertices, tangents, indices, material: m }] });
                asset.meshes.len() as u32 - 1
            });
            for model in &batch.models {
                asset.placements.push(crate::asset::Placement { mesh, transform: model.to_cols_array_2d() });
            }
        }
        let bytes = crate::asset::encode(&asset, crate::asset::Limits::unlimited()).map_err(|e| Error::InvalidInput(e.0))?;
        std::fs::write(out.join("scene.rla"), bytes).map_err(|_| Error::InvalidInput("scene dump write failed"))?;

        let inverse = camera.view().inverse();
        let eye = inverse.w_axis.truncate();
        let target = eye - inverse.z_axis.truncate();
        let v3 = |v: Vec3| format!("[{},{},{}]", v.x, v.y, v.z);
        let a3 = |v: [f32; 3]| format!("[{},{},{}]", v[0], v[1], v[2]);
        // Under the atmosphere the sky is the procedural cube shading reads (written as raw
        // little-endian f32 RGB, `sky.f32`), and the sun's illuminance is what reaches surfaces:
        // its top-of-atmosphere value times the atmosphere's transmittance at the eye.
        let procedural = matches!(self.state.sky, Sky::Atmosphere(_)).then(|| self.renderer.procedural_sky_dump(eye)).flatten();
        let transmittance = procedural.as_ref().map_or([1.0; 3], |p| p.2);
        let sun = match self.state.sun {
            Some(s) => format!(
                "{{\"to_light\":{},\"illuminance\":{},\"top_of_atmosphere\":{},\"angular_radius\":{}}}",
                v3(-s.travel.normalize()),
                a3(std::array::from_fn(|c| s.illuminance[c] * transmittance[c])),
                a3(s.illuminance),
                s.angular_radius
            ),
            None => "null".into(),
        };
        let sky = match self.state.sky {
            Sky::Uniform(r) => format!("{{\"kind\":\"uniform\",\"radiance\":{}}}", a3(r)),
            Sky::Map { map, rotation_y, intensity, .. } => {
                let file = rec.environments.get(&map).map(|e| {
                    let bytes = crate::asset::environment::encode(e, crate::asset::environment::Limits::unbounded()).unwrap_or_default();
                    let _ = std::fs::write(out.join("environment.rle"), bytes);
                    "environment.rle"
                });
                format!("{{\"kind\":\"map\",\"file\":{},\"rotation_y\":{rotation_y},\"intensity\":{intensity}}}", file.map_or("null".into(), |f| format!("\"{f}\"")))
            }
            Sky::Atmosphere(_) => match &procedural {
                Some((size, texels, _)) => {
                    let bytes: Vec<u8> = texels.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
                    let _ = std::fs::write(out.join("sky.f32"), bytes);
                    format!("{{\"kind\":\"cube\",\"file\":\"sky.f32\",\"size\":{size},\"source\":\"atmosphere\"}}")
                }
                None => "{\"kind\":\"atmosphere\"}".into(),
            },
        };
        let mut lights = String::new();
        for l in &self.state.lights {
            let entry = match *l {
                Light::Point { position, candela, range, source_radius, .. } => format!(
                    "{{\"kind\":\"point\",\"position\":{},\"candela\":{},\"range\":{},\"radius\":{source_radius}}}",
                    v3(position), a3(candela), range.map_or("null".into(), |r| r.to_string())),
                Light::Spot { position, travel, candela, range, inner, outer, source_radius, .. } => format!(
                    "{{\"kind\":\"spot\",\"position\":{},\"travel\":{},\"candela\":{},\"range\":{},\"inner\":{inner},\"outer\":{outer},\"radius\":{source_radius}}}",
                    v3(position), v3(travel), a3(candela), range.map_or("null".into(), |r| r.to_string())),
                Light::Directional { travel, lux } => format!("{{\"kind\":\"directional\",\"travel\":{},\"lux\":{}}}", v3(travel), a3(lux)),
            };
            let _ = write!(lights, "{}{entry}", if lights.is_empty() { "" } else { "," });
        }
        let json = format!(
            "{{\"label\":\"{label}\",\"camera\":{{\"eye\":{},\"target\":{},\"fov_y\":{}}},\"size\":[{},{}],\"exposure_stops\":{stops},\"sun\":{sun},\"sky\":{sky},\"lights\":[{lights}],\"emission\":[{emission}],\"skipped_instances\":{skipped},\"path\":\"{:?}\"}}\n",
            v3(eye), v3(target), camera.fov_y(), size[0], size[1], self.path);
        std::fs::write(out.join("scene.json"), json).map_err(|_| Error::InvalidInput("scene dump write failed"))?;
        Ok(())
    }
}

/// Linear blend skinning on the CPU: the pose a skinned mesh's palette gives (mesh-local
/// joint matrices). Normals by the same matrices (rigid joints).
fn skin(primitive: &Primitive, influences: &[crate::asset::skin::Influence], palette: &[Mat4]) -> Vec<Vertex> {
    primitive
        .vertices
        .iter()
        .zip(influences)
        .map(|(v, inf)| {
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                if let Some(j) = palette.get(usize::from(inf.joints[k])) {
                    m += *j * inf.weights[k];
                }
            }
            if palette.is_empty() {
                m = Mat4::IDENTITY;
            }
            let p = m.transform_point3(Vec3::from(v.position));
            let n = m.transform_vector3(Vec3::from(v.normal)).normalize_or_zero();
            Vertex { position: p.to_array(), normal: n.to_array(), ..*v }
        })
        .collect()
}
