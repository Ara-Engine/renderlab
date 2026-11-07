//! Prepared asset inputs for the measurement tracer. Full mip chains stay intact; converged
//! references integrate level zero over pixel samples instead of filtering the BSDF twice.
use super::*;
use renderlab_asset as asset;

impl Scene {
    /// Consume an already validated prepared asset, preserving placements, material textures,
    /// authored tangent frames and vertex colours. No importer or image codec enters the library.
    pub fn from_prepared(asset: asset::Asset) -> Self {
        let materials = asset
            .materials
            .into_iter()
            .map(|m| Material {
                base_color: [m.base_color[0], m.base_color[1], m.base_color[2]],
                metallic: m.metallic,
                roughness: m.roughness,
                base_color_texture: m.base_color_texture,
                metallic_roughness_texture: m.metallic_roughness_texture,
                normal_texture: m.normal_texture,
                ..Default::default()
            })
            .collect();
        let mut meshes = Vec::new();
        let mut source_meshes = Vec::new();
        for mesh in asset.meshes {
            let mut parts = Vec::new();
            for p in mesh.primitives {
                parts.push((meshes.len(), p.material as usize));
                meshes.push(Mesh {
                    positions: p.vertices.iter().map(|v| v.position).collect(),
                    normals: p.vertices.iter().map(|v| v.normal).collect(),
                    uvs: p.vertices.iter().map(|v| v.uv).collect(),
                    colors: p.vertices.iter().map(|v| v.color).collect(),
                    tangents: p.tangents.map_or_else(Vec::new, |t| t.values),
                    indices: p.indices,
                });
            }
            source_meshes.push(parts);
        }
        let instances = asset
            .placements
            .into_iter()
            .flat_map(|p| {
                source_meshes[p.mesh as usize]
                    .iter()
                    .map(move |&(mesh, material)| Instance {
                        mesh,
                        material,
                        transform: Mat4::from_cols_array_2d(&p.transform),
                    })
            })
            .collect();
        Self {
            meshes,
            materials,
            instances,
            textures: asset.textures,
            ..Default::default()
        }
    }
}

pub(super) fn validate(scene: &Scene) -> Result<(), Error> {
    for m in &scene.materials {
        for (binding, role) in [
            (m.base_color_texture.map(|t| t.texture), 0),
            (m.metallic_roughness_texture.map(|t| t.texture), 1),
            (m.normal_texture.map(|t| t.texture), 2),
        ] {
            if let Some(index) = binding {
                let t = scene
                    .textures
                    .get(index as usize)
                    .ok_or(Error::InvalidInput("reference texture missing"))?;
                let correct = match role {
                    0 => t.role == asset::TextureRole::BaseColorSrgb,
                    1 => t.role == asset::TextureRole::MetallicRoughnessLinear,
                    _ => t.role.is_normal(),
                };
                if !correct {
                    return Err(Error::InvalidInput("reference texture role mismatch"));
                }
            }
        }
        if m.normal_texture.is_some_and(|t| !t.scale.is_finite()) {
            return Err(Error::InvalidInput("reference normal scale must be finite"));
        }
    }
    for t in &scene.textures {
        if t.mips.is_empty()
            || t.mips.iter().any(|m| {
                m.width == 0
                    || m.height == 0
                    || u64::from(m.width) * u64::from(m.height) * 4 != m.rgba8.len() as u64
            })
        {
            return Err(Error::InvalidInput("invalid reference texture mip"));
        }
    }
    if scene
        .environment
        .as_ref()
        .is_some_and(|e| !e.yaw.is_finite() || !e.intensity.is_finite() || e.intensity < 0.0)
    {
        return Err(Error::InvalidInput(
            "invalid reference environment transform",
        ));
    }
    Ok(())
}

fn sampler(texture: Option<(u32, asset::Sampler)>) -> [u32; 4] {
    let wrap = |w| match w {
        asset::Wrap::Clamp => 0,
        asset::Wrap::Mirror => 1,
        asset::Wrap::Repeat => 2,
    };
    texture.map_or([0; 4], |(i, s)| {
        [
            i + 1,
            wrap(s.wrap_u),
            wrap(s.wrap_v),
            u32::from(s.mag == asset::Filter::Linear),
        ]
    })
}

pub(super) fn material_bytes(scene: &Scene, values: Vec<u8>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(scene.materials.len() * 96);
    for (m, base) in scene.materials.iter().zip(values.chunks_exact(48)) {
        bytes.extend(base);
        for b in [
            sampler(m.base_color_texture.map(|t| (t.texture, t.sampler))),
            sampler(m.metallic_roughness_texture.map(|t| (t.texture, t.sampler))),
            sampler(m.normal_texture.map(|t| (t.texture, t.sampler))),
        ] {
            bytes.extend(b.into_iter().flat_map(u32::to_le_bytes));
        }
    }
    bytes
}

/// Exact solid angle of a cube texel. Used for importance weights, with a uniform support
/// floor because bilinear lookup can return energy from a neighbouring texel.
fn solid_angle(size: u32, x: u32, y: u32) -> f64 {
    let edge = |i| 2.0 * f64::from(i) / f64::from(size) - 1.0;
    let area = |u: f64, v: f64| (u * v).atan2((u * u + v * v + 1.0).sqrt());
    area(edge(x + 1), edge(y + 1)) - area(edge(x), edge(y + 1)) - area(edge(x + 1), edge(y))
        + area(edge(x), edge(y))
}

pub(super) fn environment_data(e: &Environment) -> Vec<[f32; 4]> {
    let n = e.radiance.size();
    let texels = e.radiance.texels();
    let lum = |v: &[f32; 3]| {
        f64::from(v[0]) * 0.2126 + f64::from(v[1]) * 0.7152 + f64::from(v[2]) * 0.0722
    };
    let average = texels.iter().map(lum).sum::<f64>() / texels.len() as f64;
    let weights: Vec<_> = texels
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let k = i as u32 % (n * n);
            (lum(v) + 0.01 * average.max(1e-10)) * solid_angle(n, k % n, k / n)
        })
        .collect();
    let total = weights.iter().sum::<f64>();
    let mut cdf = 0.0;
    texels
        .iter()
        .zip(weights)
        .map(|(v, w)| {
            cdf += w / total;
            [v[0], v[1], v[2], cdf.min(1.0) as f32]
        })
        .collect()
}

pub(super) fn upload(
    device: &wgpu::Device,
    scene: &Scene,
) -> Result<(wgpu::Buffer, wgpu::Buffer, wgpu::Buffer), Error> {
    let count: usize = scene
        .textures
        .iter()
        .flat_map(|t| &t.mips)
        .map(|m| m.rgba8.len())
        .sum();
    if count as u64 > u64::from(device.limits().max_storage_buffer_binding_size) {
        return Err(Error::InvalidInput(
            "reference texture data exceeds storage binding limit",
        ));
    }
    let mut texels = Vec::with_capacity(count.max(4));
    let mut info = vec![[0u32; 4]; scene.textures.len()];
    for (i, t) in scene.textures.iter().enumerate() {
        info[i] = [
            info.len() as u32,
            t.mips.len() as u32,
            u32::from(t.role == asset::TextureRole::BaseColorSrgb),
            0,
        ];
        for mip in &t.mips {
            info.push([(texels.len() / 4) as u32, mip.width, mip.height, 0]);
            texels.extend(&mip.rgba8);
        }
    }
    if texels.is_empty() {
        texels.resize(4, 0);
    }
    if info.is_empty() {
        info.push([0; 4]);
    }
    let flags = wgpu::BufferUsages::empty();
    let textures = storage(device, "reference texture texels", &texels, flags);
    let metadata = storage(
        device,
        "reference texture mips",
        &info
            .into_iter()
            .flatten()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        flags,
    );
    let env = scene
        .environment
        .as_ref()
        .map_or_else(|| vec![[0.0; 4]], environment_data);
    let environment = storage(
        device,
        "reference environment CDF",
        &floats(env.into_iter().flatten()),
        flags,
    );
    Ok((textures, metadata, environment))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cube_weights_cover_the_sphere_and_keep_support() {
        let n = 8;
        let sum: f64 = (0..n)
            .flat_map(|y| (0..n).map(move |x| solid_angle(n, x, y)))
            .sum();
        assert!((sum * 6.0 - 4.0 * std::f64::consts::PI).abs() < 1e-12);
        let e = Environment {
            radiance: asset::environment::CubeLevel::new(
                n,
                vec![[0.0; 3]; (6 * n * n) as usize],
                asset::environment::Limits::unbounded(),
            )
            .unwrap(),
            yaw: 0.0,
            intensity: 1.0,
        };
        let data = environment_data(&e);
        assert!(data.windows(2).all(|w| w[1][3] > w[0][3]));
        assert_eq!(data.last().unwrap()[3], 1.0);
    }
}
