//! External executable linked to an immutable Ara checkout. Nothing writes into Ara.
mod ara_textures;
mod scene;
use anyhow::{Result, anyhow, ensure};
use ara_gpu::GpuDevice;
use ara_render::{
    Renderer,
    capture::{CaptureRequest, ViewMode},
    instanced as native, scene as ara,
};
use ara_render_fidelity::{
    FidelityView, FrameInput, Mode, PostSettings, Tier, presets::QualityPreset,
};
use bytemuck::Zeroable;
use glam::{Mat4, Vec3};
use renderlab_asset as asset;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

// Generated outside both repositories; points at Ara's original sample source.
include!(concat!(env!("CARGO_MANIFEST_DIR"), "/sample.rs"));

fn camera(frame: &scene::Frame) -> ara::CameraSpec {
    ara::CameraSpec::LookAt {
        position: Vec3::from_array(frame.eye),
        target: Vec3::from_array(frame.target),
        vertical_fov: frame.fov_y,
    }
}
fn tangent(n: Vec3) -> [f32; 4] {
    let axis = if n.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    n.cross(axis).normalize().extend(1.0).to_array()
}
fn light(l: &scene::LocalLight) -> ara::Light {
    let mut result = ara::Light {
        position_range: [l.position[0], l.position[1], l.position[2], l.range],
        color_lumens: [l.color[0], l.color[1], l.color[2], l.lumens],
        direction_outer: [0.0; 4],
        parameters: [0.0; 4],
        ..ara::Light::zeroed()
    };
    if let Some(spot) = &l.spot {
        let d = Vec3::from_array(spot.travel).normalize();
        result.direction_outer = [d.x, d.y, d.z, spot.outer.cos()];
        result.parameters = [1.0, spot.inner.cos(), 0.0, 0.0];
    }
    result
}
fn material(m: &asset::Material) -> native::Material {
    let mut result = native::Material::default();
    result.pbr.base_color = m.base_color;
    result.pbr.surface[0] = m.metallic;
    result.pbr.surface[1] = m.normal_texture.map_or(1.0, |t| t.scale);
    result.pbr.emissive_roughness[3] = m.roughness;
    result.pbr.textures = [
        m.base_color_texture.map_or(u32::MAX, |t| t.texture),
        m.normal_texture.map_or(u32::MAX, |t| t.texture),
        m.metallic_roughness_texture.map_or(u32::MAX, |t| t.texture),
        u32::MAX,
    ];
    if m.double_sided {
        result.pbr.flags[1] |= ara::DOUBLE_SIDED;
    }
    result
}
fn shared_mips(texture: &ara::Texture, role: asset::TextureRole) -> Vec<asset::Mip> {
    let mut mips = vec![asset::Mip {
        width: texture.size[0],
        height: texture.size[1],
        rgba8: texture.rgba.clone(),
    }];
    while let Some(last) = mips.last() {
        if last.width == 1 && last.height == 1 {
            break;
        }
        let width = (last.width / 2).max(1);
        let height = (last.height / 2).max(1);
        let mut rgba8 = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let mut c = [0.0_f32; 4];
                let mut count = 0.0;
                for yy in y * last.height / height..(y + 1) * last.height / height {
                    for xx in x * last.width / width..(x + 1) * last.width / width {
                        let offset = ((yy * last.width + xx) * 4) as usize;
                        for k in 0..4 {
                            let mut v = f32::from(last.rgba8[offset + k]) / 255.0;
                            if role == asset::TextureRole::BaseColorSrgb && k < 3 {
                                v = if v <= 0.04045 {
                                    v / 12.92
                                } else {
                                    ((v + 0.055) / 1.055).powf(2.4)
                                };
                            }
                            c[k] += v;
                        }
                        count += 1.0;
                    }
                }
                c = c.map(|v| v / count);
                if role == asset::TextureRole::NormalLinear {
                    let n = Vec3::new(c[0] * 2.0 - 1.0, c[1] * 2.0 - 1.0, c[2] * 2.0 - 1.0)
                        .normalize_or_zero();
                    c[..3].copy_from_slice(&((n + Vec3::ONE) * 0.5).to_array());
                }
                for (k, mut v) in c.into_iter().enumerate() {
                    if role == asset::TextureRole::BaseColorSrgb && k < 3 {
                        v = if v <= 0.0031308 {
                            v * 12.92
                        } else {
                            1.055 * v.powf(1.0 / 2.4) - 0.055
                        };
                    }
                    rgba8.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        mips.push(asset::Mip {
            width,
            height,
            rgba8,
        });
    }
    mips
}
fn native_scene(a: &asset::Asset, s: &scene::Scene) -> Result<native::Scene> {
    for m in &a.materials {
        ensure!(
            [
                m.base_color_texture.map(|t| t.sampler),
                m.normal_texture.map(|t| t.sampler),
                m.metallic_roughness_texture.map(|t| t.sampler)
            ]
            .into_iter()
            .flatten()
            .all(|sampler| sampler == asset::Sampler::default()),
            "ARA1 native adapter requires repeat/linear/trilinear samplers; no silent sampler substitution"
        );
    }
    let mut geometry = Vec::new();
    let mut geometry_keys = std::collections::HashMap::new();
    let mut map = Vec::new();
    for mesh in &a.meshes {
        let mut primitives = Vec::new();
        for p in &mesh.primitives {
            ensure!(
                p.vertices.iter().all(|v| v.color == [1.0; 4]),
                "Ara native has no matching per-vertex colour channel"
            );
            let vertices: Vec<_> = p
                .vertices
                .iter()
                .enumerate()
                .map(|(i, v)| native::Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                    uv1: v.uv,
                    tangent: p
                        .tangents
                        .as_ref()
                        .map_or_else(|| tangent(Vec3::from_array(v.normal)), |t| t.values[i]),
                    terrain_weights: 0,
                    #[cfg(not(ara1_v3))]
                    reserved: 0,
                    #[cfg(ara1_v3)]
                    color: u32::MAX,
                })
                .collect();
            let lo = vertices.iter().fold(Vec3::splat(f32::INFINITY), |lo, v| {
                lo.min(Vec3::from_array(v.position))
            });
            let hi = vertices
                .iter()
                .fold(Vec3::splat(f32::NEG_INFINITY), |hi, v| {
                    hi.max(Vec3::from_array(v.position))
                });
            let mut hash = Sha256::new();
            hash.update(bytemuck::cast_slice(&vertices));
            hash.update(bytemuck::cast_slice(&p.indices));
            let key: [u8; 32] = hash.finalize().into();
            if let Some(&index) = geometry_keys.get(&key) {
                primitives.push((index, p.material as usize));
                continue;
            }
            geometry_keys.insert(key, geometry.len());
            primitives.push((geometry.len(), p.material as usize));
            geometry.push(native::Geometry {
                key,
                vertices,
                lods: vec![native::Lod {
                    indices: p.indices.clone(),
                    error: 0.0,
                }],
                bounds: [lo, hi],
                deforming: false,
                revision: 0,
                #[cfg(ara1_v3)]
                vertex_colors: false,
            });
        }
        map.push(primitives);
    }
    let mut instances = Vec::new();
    for (index, p) in a.placements.iter().enumerate() {
        let world = Mat4::from_cols_array_2d(&p.transform);
        for &(geometry, material) in &map[p.mesh as usize] {
            instances.push(native::Instance {
                entity: instances.len() as u64 + 1,
                name: format!("placement-{index}"),
                geometry,
                material,
                world,
                previous_world: world,
                #[cfg(ara1_v3)]
                casts_shadows: true,
                #[cfg(ara1_v3)]
                paint: None,
                #[cfg(ara1_v3)]
                receives_decals: true,
            });
        }
    }
    let mut materials: Vec<_> = a.materials.iter().map(material).collect();
    if s.effects.alpha_panel {
        let vertices: Vec<_> = scene::PANEL_POSITIONS
            .iter()
            .enumerate()
            .map(|(i, &position)| native::Vertex {
                position,
                normal: [0.0, 0.0, 1.0],
                uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]][i],
                uv1: [0.0; 2],
                tangent: [1.0, 0.0, 0.0, 1.0],
                terrain_weights: 0,
                #[cfg(not(ara1_v3))]
                reserved: 0,
                #[cfg(ara1_v3)]
                color: u32::MAX,
            })
            .collect();
        let mut m = native::Material::default();
        m.pbr.base_color = scene::PANEL_COLOR;
        m.pbr.emissive_roughness[3] = 0.18;
        m.pbr.flags[0] = 2;
        m.pbr.flags[1] |= ara::DOUBLE_SIDED;
        let material = materials.len();
        materials.push(m);
        let index = geometry.len();
        geometry.push(native::Geometry {
            key: Sha256::digest(bytemuck::cast_slice(&vertices)).into(),
            vertices,
            lods: vec![native::Lod {
                indices: vec![0, 1, 2, 0, 2, 3],
                error: 0.0,
            }],
            bounds: [Vec3::new(0.55, 0.05, 4.3), Vec3::new(3.05, 2.55, 4.3)],
            deforming: false,
            revision: 0,
            #[cfg(ara1_v3)]
            vertex_colors: false,
        });
        instances.push(native::Instance {
            entity: instances.len() as u64 + 1,
            name: "ARA1 coverage panel".into(),
            geometry: index,
            material,
            world: Mat4::IDENTITY,
            previous_world: Mat4::IDENTITY,
            #[cfg(ara1_v3)]
            casts_shadows: true,
            #[cfg(ara1_v3)]
            paint: None,
            #[cfg(ara1_v3)]
            receives_decals: true,
        });
    }
    Ok(native::Scene {
        geometry,
        instances,
        materials,
        textures: a.textures.len() as u32,
        #[cfg(ara1_v3)]
        decals: Vec::new(),
        lights: s.lights.iter().map(light).collect(),
        sun_direction: Vec3::from_array(s.to_light).normalize(),
        sun_lux: Vec3::from_array(s.sun_lux),
        environment: vec![[s.environment[0], s.environment[1], s.environment[2], 1.0]; 8],
        environment_size: [4, 2],
    })
}
/// Flatten only for Ara's public fidelity/bake API; raster keeps true source instances.
fn legacy_scene(a: &asset::Asset, s: &scene::Scene) -> Result<ara::Scene> {
    let n = native_scene(a, s)?;
    let mut meshes = Vec::new();
    for instance in &n.instances {
        let g = &n.geometry[instance.geometry];
        let normal = instance.world.inverse().transpose();
        let vertices = g
            .vertices
            .iter()
            .map(|v| {
                let t = instance
                    .world
                    .transform_vector3(Vec3::from_array([v.tangent[0], v.tangent[1], v.tangent[2]]))
                    .normalize();
                ara::Vertex {
                    position: instance
                        .world
                        .transform_point3(Vec3::from_array(v.position))
                        .to_array(),
                    normal: normal
                        .transform_vector3(Vec3::from_array(v.normal))
                        .normalize()
                        .to_array(),
                    uv: v.uv,
                    tangent: [
                        t.x,
                        t.y,
                        t.z,
                        v.tangent[3] * instance.world.determinant().signum(),
                    ],
                }
            })
            .collect();
        meshes.push(ara::Mesh {
            entity: instance.entity,
            name: instance.name.clone(),
            vertices,
            indices: g.lods[0].indices.clone(),
            material: instance.material,
        });
    }
    let textures: Vec<_> = a
        .textures
        .iter()
        .map(|t| {
            let mip = &t.mips[0];
            ara::Texture {
                size: [mip.width, mip.height],
                rgba: mip.rgba8.clone(),
                srgb: t.role == asset::TextureRole::BaseColorSrgb,
            }
        })
        .collect();
    // Fidelity's legacy ABI samples missing texture slots 0/1. Supply neutral defaults
    // at the front and remap real slots, rather than relying on native NO_TEXTURE.
    let mut textures: Vec<_> = [
        ara::Texture {
            size: [1, 1],
            rgba: vec![255; 4],
            srgb: false,
        },
        ara::Texture {
            size: [1, 1],
            rgba: vec![128, 128, 255, 255],
            srgb: false,
        },
    ]
    .into_iter()
    .chain(textures)
    .collect();
    let materials = n
        .materials
        .iter()
        .map(|m| {
            let mut p = m.pbr;
            for (i, t) in p.textures.iter_mut().enumerate() {
                *t = if *t == u32::MAX {
                    if i == 1 { 1 } else { 0 }
                } else {
                    *t + 2
                };
            }
            p
        })
        .collect();
    Ok(ara::Scene {
        meshes,
        materials,
        textures: std::mem::take(&mut textures),
        lights: n.lights,
        sun_direction: n.sun_direction,
        sun_lux: n.sun_lux,
        environment: n.environment,
        environment_size: n.environment_size,
    })
}

fn export_sample(out: &Path) -> Result<()> {
    let s = common::scene()?;
    let mut result = asset::Asset::default();
    let mut excluded = Vec::new();
    // Only the opaque common denominator is exported. No emissive or transparent
    // surface is relabelled as an ordinary material.
    for mesh in &s.meshes {
        let m = s.materials[mesh.material];
        if m.flags[0] != 0 || m.emissive_roughness[..3] != [0.0; 3] {
            excluded.push(mesh.name.clone());
            continue;
        }
        let mut converted = asset::Material {
            base_color: m.base_color,
            metallic: m.surface[0],
            roughness: m.emissive_roughness[3],
            double_sided: true,
            ..Default::default()
        };
        for (slot, role) in [
            (0, asset::TextureRole::BaseColorSrgb),
            (1, asset::TextureRole::NormalLinear),
            (2, asset::TextureRole::MetallicRoughnessLinear),
        ] {
            let tex = &s.textures[m.textures[slot] as usize];
            let index = result.textures.len() as u32;
            result.textures.push(asset::Texture {
                role,
                mips: shared_mips(tex, role),
            });
            match slot {
                0 => {
                    converted.base_color_texture = Some(asset::BaseColorTexture {
                        texture: index,
                        sampler: asset::Sampler::default(),
                    })
                }
                1 => {
                    converted.normal_texture = Some(asset::NormalTexture {
                        texture: index,
                        sampler: asset::Sampler::default(),
                        scale: m.surface[1],
                    })
                }
                _ => {
                    converted.metallic_roughness_texture = Some(asset::MetallicRoughnessTexture {
                        texture: index,
                        sampler: asset::Sampler::default(),
                    })
                }
            }
        }
        let material = result.materials.len() as u32;
        result.materials.push(converted);
        let primitive = asset::Primitive {
            vertices: mesh
                .vertices
                .iter()
                .map(|v| asset::Vertex {
                    position: v.position,
                    normal: v.normal,
                    uv: v.uv,
                    color: [1.0; 4],
                })
                .collect(),
            tangents: Some(asset::TangentStream {
                basis: asset::TangentBasis::Authored,
                values: mesh.vertices.iter().map(|v| v.tangent).collect(),
            }),
            indices: mesh.indices.clone(),
            material,
        };
        let index = result.meshes.len() as u32;
        result.meshes.push(asset::Mesh {
            primitives: vec![primitive],
        });
        result.placements.push(asset::Placement {
            mesh: index,
            transform: Mat4::IDENTITY.to_cols_array_2d(),
        });
    }
    fs::write(out, asset::encode(&result, asset::Limits::unlimited())?)?;
    fs::write(
        out.with_extension("notes.json"),
        serde_json::to_vec_pretty(
            &json!({"source":"ara-render/examples/common/mod.rs","excluded":excluded,
        "meshes":result.meshes.len(),"placements":result.placements.len(),"materials":result.materials.len(),"textures":result.textures.len(),
        "triangles":result.meshes.iter().flat_map(|m|&m.primitives).map(|p|p.indices.len()/3).sum::<usize>(),
        "changes":["uniform environment replaces sample HDR on both engines","opaque surfaces made double sided for Renderlab path admission","shared linear-colour/normalized-normal mip generation from source mip zero"]}),
        )?,
    )?;
    Ok(())
}

fn bake(s: &scene::Scene, a: asset::Asset, cache: &Path) -> Result<()> {
    let started = Instant::now();
    let mut gpu = GpuDevice::new(&[], cache.join("device"))?;
    let source = legacy_scene(&a, s)?;
    drop(a);
    let settings = fidelity_settings(s, false);
    let initial = minimal_probes(&source, s.startup_minimal)?;
    let mut view = FidelityView::new(&mut gpu, &source, settings, initial.as_ref())?;
    drop(source);
    let camera = camera(&s.frames[0]);
    view.render(
        &mut gpu,
        &FrameInput {
            camera: &camera,
            tick: 0,
            delta_seconds: 1.0 / 30.0,
            reset: true,
            lighting_changed: false,
            deterministic: false,
        },
    )?;
    let surfaces = view.bake_probe_surfaces(&mut gpu)?;
    let lattices: Vec<_> = surfaces
        .layout
        .lattices()
        .iter()
        .map(|l| json!({"origin":l.origin,"spacing":l.spacing,"counts":l.counts}))
        .collect();
    fs::write(
        cache.join("probes.json"),
        serde_json::to_vec(
            &json!({"lattices":lattices,"texels":surfaces.texels,"statics":surfaces.statics}),
        )?,
    )?;
    fs::write(
        cache.join("bake.json"),
        serde_json::to_vec_pretty(
            &json!({"seconds":started.elapsed().as_secs_f64(),"probes":surfaces.len(),
        "method":"Ara RT acquisition; raster runtime does not use RT","startup_minimal":s.startup_minimal,
        "probes_sha256":hex(&Sha256::digest(fs::read(cache.join("probes.json"))?)),"asset_sha256":hex(&Sha256::digest(fs::read(&s.asset)?))}),
        )?,
    )?;
    view.destroy(&mut gpu)?;
    gpu.assert_clean()?;
    Ok(())
}
fn load_bake(cache: &Path, s: &scene::Scene) -> Result<ara_render::look::gi::ProbeSurfaces> {
    use ara_render::look::gi::{ProbeLattice, ProbeLayout, ProbeSurfaces};
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(cache.join("bake.json"))?)?;
    ensure!(
        meta["asset_sha256"] == hex(&Sha256::digest(fs::read(&s.asset)?)),
        "probe bake source mismatch"
    );
    ensure!(
        meta["startup_minimal"] == s.startup_minimal,
        "startup smoke bake must not enter a normal quality/performance row"
    );
    ensure!(
        meta["probes_sha256"] == hex(&Sha256::digest(fs::read(cache.join("probes.json"))?)),
        "probe cache payload changed"
    );
    let v: serde_json::Value = serde_json::from_slice(&fs::read(cache.join("probes.json"))?)?;
    let mut lattices = Vec::new();
    for l in v["lattices"]
        .as_array()
        .ok_or_else(|| anyhow!("missing lattice"))?
    {
        lattices.push(ProbeLattice {
            origin: serde_json::from_value(l["origin"].clone())?,
            spacing: serde_json::from_value(l["spacing"].clone())?,
            counts: serde_json::from_value(l["counts"].clone())?,
        });
    }
    let surfaces = ProbeSurfaces {
        layout: ProbeLayout::new(&lattices)?,
        texels: serde_json::from_value(v["texels"].clone())?,
        statics: serde_json::from_value(v["statics"].clone())?,
    };
    surfaces.validate()?;
    Ok(surfaces)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn minimal_probes(
    source: &ara::Scene,
    enabled: bool,
) -> Result<Option<ara_render_fidelity::probes::BakedProbes>> {
    if !enabled {
        return Ok(None);
    }
    // The public constructor validates the scene fingerprint. Replace its initial
    // volume before the GPU view allocates anything; eight is the valid minimum.
    let mut baked = ara_render_fidelity::probes::BakedProbes::unbaked(source)?;
    let positions = source
        .meshes
        .iter()
        .flat_map(|m| m.vertices.iter().map(|v| Vec3::from_array(v.position)));
    let (lo, hi) = positions.fold(
        (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    );
    baked.volume = ara_render_fidelity::probes::ProbeVolume {
        origin: lo - Vec3::splat(0.1),
        spacing: (hi - lo).max(Vec3::ONE) + Vec3::splat(0.2),
        counts: [2; 3],
    };
    baked.data = baked.volume.empty_data();
    Ok(Some(baked))
}
fn fidelity_settings(s: &scene::Scene, reference: bool) -> ara_render_fidelity::Settings {
    ara_render_fidelity::Settings {
        render_size: s.size,
        display_size: s.size,
        tier: Tier::Rt,
        mode: if reference {
            Mode::Reference {
                samples: 256,
                bounces: 8,
            }
        } else {
            Mode::Hybrid
        },
        upscaler: ara_render_fidelity::Upscaler::Taau,
        probe_ray_budget: if s.startup_minimal { 64 } else { 16384 },
        reflection_cutoff: 0.6,
        post: PostSettings {
            ev100: Some(-s.exposure_stops - 1.2_f32.log2()),
            bloom: 0.0,
            fog_density: 0.0,
            aperture: 0.0,
            shutter: 0.0,
            ..Default::default()
        },
    }
}

fn main() -> Result<()> {
    let begin = Instant::now();
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("export-sample") {
        return export_sample(Path::new(
            args.get(1)
                .ok_or_else(|| anyhow!("export-sample OUTPUT.rla"))?,
        ));
    }
    ensure!(
        args.len() == 4,
        "ara1-ara SCENE.json raster|hybrid|fidelity|reference|bake OUTPUT BAKE-CACHE"
    );
    let spec = scene::load(Path::new(&args[0])).map_err(|e| anyhow!(e.to_string()))?;
    ensure!(
        spec.sun_radius == 0.00465,
        "Ara main's fidelity sun radius is fixed at 0.00465; use a matched scene instead of silently ignoring its radius"
    );
    let out = PathBuf::from(&args[2]);
    fs::create_dir_all(&out)?;
    let cache = PathBuf::from(&args[3]);
    fs::create_dir_all(&cache)?;
    let bytes = fs::read(&spec.asset)?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let a = asset::decode(&bytes, asset::Limits::unlimited())?;
    drop(bytes);
    if args[1] == "bake" {
        return bake(&spec, a, &cache);
    }
    let mut rows = Vec::new();
    let mut first = None;
    if matches!(args[1].as_str(), "fidelity" | "reference") {
        let mut gpu = GpuDevice::new(&[], out.join("gpu-cache"))?;
        let settings = fidelity_settings(&spec, args[1] == "reference");
        let source = legacy_scene(&a, &spec)?;
        drop(a);
        let initial = minimal_probes(&source, spec.startup_minimal)?;
        let mut view = FidelityView::new(&mut gpu, &source, settings, initial.as_ref())?;
        drop(source);
        if args[1] == "fidelity" {
            view.set_gather_events(1)?;
            view.set_transport_stride(4)?;
            view.set_bounded_reservoirs(true);
            view.set_reservoir_sampling(&mut gpu, true)?;
        }
        for (i, f) in spec.frames.iter().enumerate() {
            let c = camera(f);
            let start = Instant::now();
            view.render(
                &mut gpu,
                &FrameInput {
                    camera: &c,
                    tick: i as u32,
                    delta_seconds: 1.0 / 30.0,
                    reset: f.cut || i == 0,
                    lighting_changed: false,
                    deterministic: false,
                },
            )?;
            let cpu = start.elapsed().as_secs_f64() * 1e3;
            let passes: Vec<_> = view
                .timings()
                .iter()
                .map(|t| json!({"pass":t.pass,"queue":format!("{:?}",t.queue),"ms":t.milliseconds}))
                .collect();
            if f.capture || i == 0 {
                let image = view.capture(&mut gpu)?;
                if first.is_none() {
                    first = Some(begin.elapsed().as_secs_f64() * 1e3);
                    fs::write(out.join("first-image.ready"), b"readback ready")?;
                }
                if f.capture {
                    image.save(out.join(format!("frame-{i:04}.png")))?;
                }
            }
            rows.push(
                json!({"index":i,"phase":f.phase,"cut":f.cut,"cpu_service_ms":cpu,"passes":passes}),
            );
        }
        fs::write(
            out.join("engine.json"),
            serde_json::to_vec_pretty(
                &json!({"engine":"ara","path":args[1],"adapter":gpu.info(),"size":spec.size,
            "startup_to_readback_ms":first,"settings":format!("{:?}",view.settings()),"geometry":"legacy flattened public fidelity API","frames":rows,
            "notes":spec.notes,"startup_minimal":spec.startup_minimal,"timing_scope":"synchronous render service; pass sums are not an end-to-end GPU span"}),
            )?,
        )?;
        view.destroy(&mut gpu)?;
        gpu.assert_clean()?;
    } else {
        ensure!(
            matches!(args[1].as_str(), "raster" | "hybrid"),
            "unknown Ara path"
        );
        let source = native_scene(&a, &spec)?;
        let textures = Box::new(ara_textures::RlaTextures::new(a.textures, digest));
        drop(a.meshes);
        drop(a.materials);
        drop(a.placements);
        let mut renderer = if args[1] == "raster" {
            Renderer::new_native(&source, textures, spec.size, &[], out.join("gpu-cache"))?
        } else {
            let empty = ara::Scene {
                meshes: vec![],
                materials: vec![ara::Material::default()],
                textures: vec![
                    ara::Texture {
                        size: [1, 1],
                        rgba: vec![255; 4],
                        srgb: false,
                    },
                    ara::Texture {
                        size: [1, 1],
                        rgba: vec![128, 128, 255, 255],
                        srgb: false,
                    },
                ],
                lights: vec![],
                sun_direction: source.sun_direction,
                sun_lux: source.sun_lux,
                environment: source.environment.clone(),
                environment_size: source.environment_size,
            };
            let mut renderer = Renderer::new_hybrid(empty, spec.size, &[], out.join("gpu-cache"))?;
            renderer.replace_native_scene(&source, textures)?;
            renderer
        };
        drop(source);
        let mut quality = QualityPreset::High.raster_quality();
        if spec.startup_minimal {
            quality.gi_budget = 1;
        }
        if args[1] == "raster" {
            quality.ray_queries = None;
        }
        if !spec.effects.reflections {
            quality.ray_queries = None;
            quality.reflections = None;
            quality.reflection_captures = None;
        }
        if !spec.effects.local_shadows {
            quality.local_shadows = None;
        }
        renderer.set_quality(quality)?;
        renderer.set_look(ara_render::look::Look {
            sky: spec
                .effects
                .atmosphere
                .then_some(ara_render::look::Sky::default()),
            fog: spec.effects.fog.then_some(ara_render::look::Fog {
                extinction: 0.01,
                height: 90.0,
                base: 0.0,
                tint: [1.0; 3],
                albedo: 0.9,
                anisotropy: 0.3,
            }),
            bloom: if spec.effects.post { 0.035 } else { 0.0 },
            ..Default::default()
        })?;
        if spec.effects.gi {
            renderer.set_probe_surfaces(Some(&load_bake(&cache, &spec)?))?;
        }
        let mut readiness_frames = 0;
        for (i, f) in spec.frames.iter().enumerate() {
            if f.cut {
                renderer.reset_temporal_history();
            }
            let c = camera(f);
            let start = Instant::now();
            renderer.render(&c, 2.0_f32.powf(spec.exposure_stops))?;
            if i == 0 && args[1] == "hybrid" {
                let deadline = Instant::now();
                while renderer.ray_pipelines_pending() {
                    ensure!(
                        deadline.elapsed().as_secs() < 120,
                        "hybrid ray pipelines did not become ready"
                    );
                    renderer.render(&c, 2.0_f32.powf(spec.exposure_stops))?;
                    readiness_frames += 1;
                }
            }
            let cpu = start.elapsed().as_secs_f64() * 1e3;
            let passes: Vec<_> = renderer
                .timings()
                .iter()
                .map(|t| json!({"pass":t.pass,"queue":format!("{:?}",t.queue),"ms":t.milliseconds}))
                .collect();
            if f.capture || i == 0 {
                // Read the live frame without resetting temporal history or disabling AA.
                let image = renderer.capture_completed(&CaptureRequest {
                    camera: c,
                    size: spec.size,
                    mode: ViewMode::Lit,
                    deterministic: false,
                    exposure: 2.0_f32.powf(spec.exposure_stops),
                    settle_frames: 0,
                    tick: i as u64,
                    seed: 7,
                    marks: None,
                })?;
                if first.is_none() {
                    first = Some(begin.elapsed().as_secs_f64() * 1e3);
                    fs::write(out.join("first-image.ready"), b"readback ready")?;
                }
                if f.capture {
                    image.image.save(out.join(format!("frame-{i:04}.png")))?;
                }
            }
            rows.push(
                json!({"index":i,"phase":f.phase,"cut":f.cut,"cpu_service_ms":cpu,"passes":passes,
                    "probe_gi_ready":renderer.probe_gi_ready(),"probe_gi_needs_frame":renderer.probe_gi_needs_frame()}),
            );
        }
        fs::write(
            out.join("engine.json"),
            serde_json::to_vec_pretty(
                &json!({"engine":"ara","path":args[1],"adapter":renderer.gpu().info(),"size":spec.size,
            "startup_to_readback_ms":first,"settings":format!("{quality:?}"),"geometry":"native instanced, shared uncompressed RLA mips","frames":rows,
            "notes":spec.notes,"startup_minimal":spec.startup_minimal,"readiness_frames":readiness_frames,"timing_scope":"synchronous render service; pass sums are not an end-to-end GPU span","probe_acquisition":"prebaked on RT; separately charged"}),
            )?,
        )?;
        renderer.gpu().assert_clean()?;
    }
    Ok(())
}
