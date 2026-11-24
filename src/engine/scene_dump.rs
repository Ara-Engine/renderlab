//! Measurement: `RENDERLAB_SCENE_DUMP=<dir>` keeps CPU copies of the scene an embedder builds
//! (meshes, skinned meshes with their latest palettes, textures, materials, environments), so
//! one frame of a running game can be frozen and path-traced (`examples/scene_reference.rs`,
//! main-thread/ref/README.md). Armed like the frame readback: once `<dir>/arm` exists (a label
//! line), the next view rendered writes `<dir>/<label>/scene.rla` (one mesh per mesh and
//! material pair, placements at their current transforms, materials, textures) and
//! `<dir>/<label>/scene.json` (camera, output size, exposure, sun, sky, lights, each material's
//! emission), then removes the file. `RENDERLAB_SCENE_DUMP=<dir>;<frame>,...` also dumps the
//! listed renderer frames as `frame-<id>`, for frame-locked hosts. A procedural atmosphere sky is written as the cube shading
//! reads (`sky.f32`) with the sun as it reaches surfaces. While dumping, the engine reports no block-compressed
//! texture support, so embedders upload RGBA8 levels the tracer can read.
//!
//! An arm file whose second line is `motion` also records how the scene moves from that dump on
//! (the bench replays it, src/bench/dump.rs): `skins.rla` (each skinned mesh's rest primitive,
//! in `skins.bin` order) and `skins.bin` (per skinned mesh: its scene.rla mesh index, joint
//! count and vertex influences), then `motion.bin`, one record per rendered frame: its renderer
//! frame id, the camera, each dumped placement whose model changed since the last record (its
//! scene.rla placement index) and each skinned mesh whose palette changed. Records are deltas:
//! a replay applies every record up to a frame. Recording stops when `<dir>/motion-stop` appears.
//! A motion dump also keeps see-through batches (a reference dump leaves them out), and
//! scene.json lists their materials' transparency (`transparency`: [material, opacity, ior,
//! thickness]).
use super::*;
use std::{collections::HashMap, fmt::Write as _, io::Write as _, path::PathBuf};

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
    /// TP1's transparency (None: opaque).
    transparency: Option<crate::Transparency>,
}

#[derive(Default)]
pub(super) struct Recorder {
    dir: PathBuf,
    /// `RENDERLAB_SCENE_DUMP=<dir>;<frame>,<frame>,...`: renderer frames (the pass log's ids)
    /// that dump as `frame-<id>` without an arm file, for frame-locked hosts such as a replay.
    frames: Vec<u64>,
    meshes: HashMap<MeshId, Recorded>,
    textures: HashMap<TextureId, Arc<TextureData>>,
    materials: HashMap<MaterialId, RecordedMaterial>,
    environments: HashMap<EnvironmentId, Arc<PreparedEnvironment>>,
    motion: Option<Motion>,
}

/// A motion recording in progress (module doc).
struct Motion {
    out: std::io::BufWriter<std::fs::File>,
    /// Each dumped placement's object: its scene.rla placement index and its last model.
    objects: HashMap<ObjectId, (u32, Mat4)>,
    /// Each dumped skinned mesh: its index in skins.bin and its last palette.
    skins: HashMap<MeshId, (u32, Vec<Mat4>)>,
}

fn put_f32s(out: &mut Vec<u8>, values: impl IntoIterator<Item = f32>) {
    for v in values {
        out.extend(v.to_le_bytes());
    }
}

impl Recorder {
    pub(super) fn from_env() -> Option<Self> {
        let value = std::env::var("RENDERLAB_SCENE_DUMP").ok()?;
        let (dir, frames) = match value.split_once(';') {
            Some((dir, list)) => (dir, list.split(',').filter_map(|f| f.trim().parse().ok()).collect()),
            None => (value.as_str(), vec![]),
        };
        Some(Self { dir: dir.into(), frames, ..Default::default() })
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
            transparency: desc.transparency,
        });
    }
    /// An in-place factor edit (`Engine::set_material_factors`).
    pub(super) fn material_factors(
        &mut self,
        id: MaterialId,
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
    ) {
        if let Some(m) = self.materials.get_mut(&id) {
            (m.desc, m.metallic, m.roughness) = (base_color, metallic, roughness);
        }
    }

    /// The label renderer frame `frame` dumps under: `frame-<id>` when listed, else the label
    /// in `<dir>/arm` if it exists (consumed); and whether it starts a motion recording (the arm
    /// file's second line is `motion`).
    pub(super) fn armed(&self, frame: u64) -> Option<(String, bool)> {
        if self.frames.contains(&frame) {
            return Some((format!("frame-{frame}"), false));
        }
        let arm = self.dir.join("arm");
        // The label line ends with a newline; without one the file is still being written (an
        // empty read once dumped spinner-a as "frame"), so it waits for a later frame.
        let text = std::fs::read_to_string(&arm).ok().filter(|t| t.ends_with('\n'))?;
        let _ = std::fs::remove_file(&arm);
        let mut lines = text.lines();
        let label = lines.next().unwrap_or("frame").trim().to_owned();
        Some((label, lines.next().is_some_and(|l| l.trim() == "motion")))
    }

    /// Writes this frame's motion record when a recording runs (module doc); stops the
    /// recording once `<dir>/motion-stop` exists.
    pub(super) fn record_motion<'a>(&mut self, frame: u64, camera: &Camera, batches: impl Iterator<Item = (&'a [ObjectId], &'a [Mat4])>) {
        let Some(m) = self.motion.as_mut() else { return };
        let stop = self.dir.join("motion-stop");
        if stop.exists() {
            let _ = std::fs::remove_file(&stop);
            let _ = m.out.flush();
            self.motion = None;
            return;
        }
        let inverse = camera.view().inverse();
        let (eye, target) = (inverse.w_axis.truncate(), inverse.w_axis.truncate() - inverse.z_axis.truncate());
        let cam = [eye.x, eye.y, eye.z, target.x, target.y, target.z];
        let mut models = vec![];
        for (batch_objects, batch_models) in batches {
            for (object, model) in batch_objects.iter().zip(batch_models) {
                if let Some((index, last)) = m.objects.get_mut(object) {
                    if last != model {
                        *last = *model;
                        models.push((*index, *model));
                    }
                }
            }
        }
        let mut skins = vec![];
        for (mesh, (index, last)) in m.skins.iter_mut() {
            if let Some(Recorded::Skinned { palette, .. }) = self.meshes.get(mesh) {
                if palette != last {
                    last.clone_from(palette);
                    skins.push((*index, palette.clone()));
                }
            }
        }
        let mut rec = frame.to_le_bytes().to_vec();
        put_f32s(&mut rec, cam);
        rec.extend((models.len() as u32).to_le_bytes());
        for (index, model) in models {
            rec.extend(index.to_le_bytes());
            put_f32s(&mut rec, model.to_cols_array());
        }
        skins.sort_by_key(|s| s.0);
        rec.extend((skins.len() as u32).to_le_bytes());
        for (index, palette) in skins {
            rec.extend(index.to_le_bytes());
            rec.extend((palette.len() as u32).to_le_bytes());
            put_f32s(&mut rec, palette.iter().flat_map(|j| j.to_cols_array()));
        }
        let _ = m.out.write_all(&rec);
    }
}

impl Engine {
    /// Writes the frozen scene for `label` as seen by `camera` into `size` pixels (module doc).
    pub(super) fn dump_scene(&mut self, label: &str, camera: &Camera, size: [u32; 2], stops: f32, motion: bool) -> Result<(), Error> {
        let Some(rec) = self.dump.as_ref() else { return Ok(()) };
        // For a motion recording: each placement's object and each skinned mesh's rest data.
        let mut objects: HashMap<ObjectId, (u32, Mat4)> = HashMap::new();
        let mut skins: Vec<(MeshId, u32, Primitive, Vec<crate::asset::skin::Influence>, usize)> = vec![];
        let out = rec.dir.join(label);
        let _ = std::fs::create_dir_all(&out);
        let mut asset = crate::asset::Asset { meshes: vec![], placements: vec![], materials: vec![], textures: vec![] };
        let mut texture_index: HashMap<TextureId, u32> = HashMap::new();
        let mut material_index: HashMap<MaterialId, u32> = HashMap::new();
        let mut mesh_index: HashMap<(MeshId, MaterialId), u32> = HashMap::new();
        let mut emission = String::new();
        // A motion dump (the bench's) keeps see-through batches, listed with their transparency: they move and
        // cast like any other (the cache's moved boxes count them). A reference dump leaves them out, as the
        // dump tracer reads every mesh as opaque.
        let mut see_through = String::new();
        let mut skipped = 0usize;
        for batch in self.state.batches.values() {
            let (Some(recorded), Some(material)) = (rec.meshes.get(&batch.mesh), rec.materials.get(&batch.material)) else {
                skipped += batch.models.len();
                continue;
            };
            if material.transparency.is_some() && !motion {
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
                    if let Some(t) = material.transparency {
                        let _ = write!(see_through, "{}[{index},{},{},{}]", if see_through.is_empty() { "" } else { "," }, t.opacity, t.ior, t.thickness);
                    }
                    material_index.insert(batch.material, index);
                    index
                }
            };
            let mesh = *mesh_index.entry((batch.mesh, batch.material)).or_insert_with(|| {
                let (vertices, indices, tangents) = match recorded {
                    Recorded::Static { vertices, indices, tangents } => (vertices.clone(), indices.clone(), tangents.clone()),
                    Recorded::Skinned { primitive, influences, palette } => {
                        skins.push((batch.mesh, asset.meshes.len() as u32, primitive.clone(), influences.clone(), palette.len()));
                        (skin(primitive, influences, palette), primitive.indices.clone(), None)
                    }
                };
                asset.meshes.push(crate::asset::Mesh { primitives: vec![Primitive { vertices, tangents, indices, material: m }] });
                asset.meshes.len() as u32 - 1
            });
            for (object, model) in batch.objects.iter().zip(&batch.models) {
                objects.insert(*object, (asset.placements.len() as u32, *model));
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
            "{{\"label\":\"{label}\",\"camera\":{{\"eye\":{},\"target\":{},\"fov_y\":{}}},\"size\":[{},{}],\"exposure_stops\":{stops},\"sun\":{sun},\"sky\":{sky},\"lights\":[{lights}],\"emission\":[{emission}],\"transparency\":[{see_through}],\"skipped_instances\":{skipped},\"path\":\"{:?}\"}}\n",
            v3(eye), v3(target), camera.fov_y(), size[0], size[1], self.path);
        std::fs::write(out.join("scene.json"), json).map_err(|_| Error::InvalidInput("scene dump write failed"))?;
        if motion {
            let write_failed = |_| Error::InvalidInput("scene dump motion write failed");
            // The rest primitives (material 0 of one plain material: the bench draws them with
            // scene.rla's material for the same mesh) and their influences.
            let plain = crate::asset::Material {
                base_color: [1.0; 4],
                metallic: 0.0,
                roughness: 1.0,
                base_color_texture: None,
                metallic_roughness_texture: None,
                normal_texture: None,
                double_sided: true,
            };
            let rest = crate::asset::Asset {
                meshes: skins.iter().map(|s| crate::asset::Mesh { primitives: vec![Primitive { material: 0, ..s.2.clone() }] }).collect(),
                placements: vec![],
                materials: vec![plain],
                textures: vec![],
            };
            let bytes = crate::asset::encode(&rest, crate::asset::Limits::unlimited()).map_err(|e| Error::InvalidInput(e.0))?;
            std::fs::write(out.join("skins.rla"), bytes).map_err(write_failed)?;
            let mut bin = (skins.len() as u32).to_le_bytes().to_vec();
            for (_, mesh, _, influences, joints) in &skins {
                bin.extend(mesh.to_le_bytes());
                bin.extend((*joints as u32).to_le_bytes());
                bin.extend((influences.len() as u32).to_le_bytes());
                for inf in influences {
                    for j in inf.joints {
                        bin.extend(j.to_le_bytes());
                    }
                    put_f32s(&mut bin, inf.weights);
                }
            }
            std::fs::write(out.join("skins.bin"), bin).map_err(write_failed)?;
            let file = std::fs::File::create(out.join("motion.bin")).map_err(write_failed)?;
            let skins = skins.iter().enumerate().map(|(k, s)| (s.0, (k as u32, rec.meshes.get(&s.0).map_or(vec![], |r| match r {
                Recorded::Skinned { palette, .. } => palette.clone(),
                Recorded::Static { .. } => vec![],
            })))).collect();
            let motion = Motion { out: std::io::BufWriter::new(file), objects, skins };
            if let Some(rec) = self.dump.as_mut() {
                rec.motion = Some(motion);
            }
        }
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
