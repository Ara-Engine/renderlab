//! Block-compressed material textures on the traced frame (`forward::BlockTexture`): a
//! texture drawn from its blocks against the same texels uploaded as RGBA8.
use super::texture_cases::{apart, floor, floor_in};
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    asset::{
        BlockFormat, CompressedTexture, Filter, Mip, MipmapFilter, Sampler, TangentBasis,
        TangentStream, Texture, TextureRole,
    },
    backend_ara::forward::{
        BlockTexture, ForwardObject, ForwardScene, Frames, NormalMap, ObjectProgram, ObjectTexture,
        ObjectTextures, Primary, ScenePrograms, SceneTextures, SunShadows, render,
    },
};

/// A deterministic stream of bytes.
struct Bits(u64);
impl Bits {
    fn next(&mut self) -> u8 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u8
    }
}

/// A `side` x `side` BC7 chain of random blocks (every mode but the reserved eighth:
/// each block's first byte is nonzero) and its RGBA8 decode by `texture2ddecoder`, the
/// decoder Ara's bridge uploads RGBA8 from.
fn bc7(side: u32, format: BlockFormat, seed: u64) -> (BlockTexture, Texture) {
    let mut bits = Bits(seed);
    let (mut levels, mut mips) = (vec![], vec![]);
    let mut w = side;
    loop {
        let blocks = w.div_ceil(4);
        let data: Vec<u8> = (0..blocks * blocks * 16)
            .map(|i| {
                let byte = bits.next();
                if i % 16 == 0 { byte.max(1) } else { byte }
            })
            .collect();
        let padded = (blocks * 4) as usize;
        let mut pixels = vec![0u32; padded * padded];
        texture2ddecoder::decode_bc7(&data, padded, padded, &mut pixels).unwrap();
        // The level's own texels: the top-left w x w of its padded blocks.
        let rgba8 = (0..w)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let [b, g, r, a] = pixels[y as usize * padded + x as usize].to_le_bytes();
                [r, g, b, a]
            })
            .collect();
        levels.push(data);
        mips.push(Mip {
            width: w,
            height: w,
            rgba8,
        });
        if w == 1 {
            break;
        }
        w /= 2;
    }
    let compressed = CompressedTexture {
        format,
        width: side,
        height: side,
        mips: levels,
    };
    compressed.validate(16384).unwrap();
    (
        BlockTexture::new(compressed),
        Texture {
            role: format.role(),
            mips,
        },
    )
}

/// One BC5 block whose sixteen texels are all (`x`, `y`): equal endpoints, index zero.
fn bc5_block(x: u8, y: u8) -> [u8; 16] {
    [x, x, 0, 0, 0, 0, 0, 0, y, y, 0, 0, 0, 0, 0, 0]
}
/// A 16x16 BC5 normal map whose level `l` is the block `level(l)` throughout.
fn bc5(level: impl Fn(u32) -> [u8; 16]) -> BlockTexture {
    let mips = (0..5)
        .map(|l| {
            let blocks = (16u32 >> l).div_ceil(4);
            (0..blocks * blocks).flat_map(|_| level(l)).collect()
        })
        .collect();
    let compressed = CompressedTexture {
        format: BlockFormat::Bc5,
        width: 16,
        height: 16,
        mips,
    };
    compressed.validate(16384).unwrap();
    BlockTexture::new(compressed)
}

fn everywhere(_: u32, _: u32) -> bool {
    true
}

/// A program that shows the normal map's variance (`material_normal_variance`: the
/// sample's shortening plus, for a map with variance code 1, its stored spread squared)
/// instead of light. The standard program does not use it (`SPECULAR_AA` is off).
const VARIANCE: &str = "struct Surface { variance:f32 };
fn surface(input:SurfaceInput) -> Surface { return Surface(material_normal_variance(input)); }
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> { return vec3<f32>(s.variance); }
";

/// The mean of `VARIANCE` over a floor seen face on from 4 m, `map` repeating `repeats`
/// times across it and sampled trilinearly.
fn variance_seen(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    map: &BlockTexture,
    repeats: f32,
) -> Result<f32> {
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position: [f32; 3]| {
            Vertex {
                position,
                normal: [0., 0., 1.],
                uv: [
                    (position[0] + 4.) / 8. * repeats,
                    (4. - position[1]) / 8. * repeats,
                ],
                color: [1.; 4],
            }
        });
    let tangents = TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1., 0., 0., 1.]; 4],
    };
    // A traced frame needs a sun and its shadows; the program shows neither.
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-0.3, 0.2, -1.),
            lux: [1000.; 3],
        }],
        environment: [0.; 3],
        exposure_stops: 0.,
    };
    let shadows = SunShadows {
        split_ends: vec![12.],
        disk_angle: 0.02,
    };
    let images = render(
        gpu,
        cache,
        &ForwardScene {
            meshes: None,
            materials: None,
            instances: None,
            objects: &[ForwardObject {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: MaterialDescriptor {
                    double_sided: true,
                    ..Default::default()
                },
            }],
            lighting: &lighting,
            camera: Camera::look_at(
                Vec3::new(0., 0., 4.),
                Vec3::ZERO,
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.,
            )?,
            size: [128, 96],
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: Some(&shadows),
            primary: Primary::Traced,
            post: None,
            indirect: None,
            reconstruction: None,
            reflections: None,
            textures: Some(SceneTextures {
                objects: &[ObjectTextures {
                    normal: Some(NormalMap {
                        texture: ObjectTexture {
                            data: map.into(),
                            sampler: Sampler {
                                mag: Filter::Linear,
                                min: Filter::Linear,
                                mipmap: MipmapFilter::Linear,
                                ..Default::default()
                            },
                            key: None,
                        },
                        tangents: &tangents,
                    }),
                    ..Default::default()
                }],
                anisotropy_clamp: 1,
            }),
            local_shadows: None,
            transparent: &[],
            programs: Some(ScenePrograms {
                objects: &[Some(ObjectProgram {
                    source: VARIANCE,
                    parameters: &[],
                })],
                transparent: &[],
            }),
            sky_map: None,
            skins: None,
            casts: None,
            colour_lut: None,
            ids: None,
        },
    )?;
    let reds: Vec<f32> = images
        .hdr
        .as_chunks::<8>()
        .0
        .iter()
        .map(|p| f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32())
        .collect();
    Ok(reds.iter().sum::<f32>() / reds.len() as f32)
}

/// A floor textured from blocks and from their decode is the same frame.
///
/// - **BC7**, as sRGB base colour and as linear metallic-roughness, random blocks of
///   every mode: the two frames are equal bit for bit, sampled nearest and magnified
///   (the GPU's decode is the texels Ara's decoder gives) and trilinear and minified
///   (the small levels, down to those under 4x4 texels).
/// - **BC5** normals, a constant tilted normal: the frame is the one an RGBA8 map with
///   Z rebuilt on the CPU draws, to half a percent (that map's Z is rounded to 8 bits),
///   and far from the flat floor, so the shading programs rebuild Z from X and Y.
/// - **The variance layer**: a BC5 map whose top level alternates two slopes but whose
///   smaller levels are flat, minified until only flat levels are sampled, against a
///   map that is flat throughout. The sampled normals are the same, so the sample's own
///   shortening is nil for both; a program that shows `material_normal_variance` sees
///   the spread of the top level's normals for the first and none for the second, which
///   it can only read from the map's second layer.
/// - A texture identity that named RGBA8 texels cannot later name blocks of that size.
///
/// Where the device does not sample BC there is nothing to check: says so and stops.
#[test]
fn blocks_draw_what_their_decode_draws() -> Result<()> {
    let path = root("compressed-textures");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    if !gpu.info().texture_compression_bc {
        eprintln!("skipped: the device does not sample BC textures");
        return Ok(());
    }
    let cache = ShaderCache::new(path.join("shaders"))?;
    let nearest = Sampler {
        mag: Filter::Nearest,
        min: Filter::Nearest,
        mipmap: MipmapFilter::Nearest,
        ..Default::default()
    };
    let trilinear = Sampler {
        mag: Filter::Linear,
        min: Filter::Linear,
        mipmap: MipmapFilter::Linear,
        ..Default::default()
    };
    let mut report = String::new();

    for (format, seed) in [(BlockFormat::Bc7Srgb, 17), (BlockFormat::Bc7Linear, 19)] {
        let (blocks, decoded) = bc7(32, format, seed);
        for (sampler, repeats) in [(nearest, 1.), (trilinear, 24.)] {
            let mut shot = |data| {
                let slot = Some(ObjectTexture {
                    data,
                    sampler,
                    key: None,
                });
                let textures = if format == BlockFormat::Bc7Srgb {
                    ObjectTextures {
                        base_color: slot,
                        ..Default::default()
                    }
                } else {
                    ObjectTextures {
                        metallic_roughness: slot,
                        ..Default::default()
                    }
                };
                floor(
                    &mut gpu,
                    &cache,
                    Primary::Traced,
                    4.,
                    repeats,
                    MaterialDescriptor {
                        metallic: 1.,
                        roughness: 1.,
                        ..Default::default()
                    },
                    Some(textures),
                )
            };
            let from_blocks = shot((&blocks).into())?;
            let from_texels = shot((&decoded).into())?;
            let (worst, _) = apart(&from_blocks, &from_texels, everywhere);
            ensure!(
                from_blocks == from_texels,
                "{format:?} at {repeats} repeats: the blocks' frame is {worst:.6} from their decode's"
            );
            report += &format!("{format:?} x{repeats}: {worst:.6}; ");
        }
    }

    // BC5: a normal tilted along the tangent, (0.28, 0, 0.96).
    let tilt = ((0.28f32 * 0.5 + 0.5) * 255.).round() as u8;
    let tangents = TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1., 0., 0., 1.]; 4],
    };
    let tilted = bc5(|_| bc5_block(tilt, 128));
    let (x, y) = (f32::from(tilt) / 255. * 2. - 1., 128. / 255. * 2. - 1.);
    let z = (1. - x * x - y * y).max(0.).sqrt();
    let rebuilt = Texture {
        role: TextureRole::NormalLinear,
        mips: (0..5)
            .map(|l| {
                let side = 16u32 >> l;
                Mip {
                    width: side,
                    height: side,
                    rgba8: (0..side * side)
                        .flat_map(|_| [tilt, 128, ((z * 0.5 + 0.5) * 255.).round() as u8, 255])
                        .collect(),
                }
            })
            .collect(),
    };
    let mut mapped = |data, sampler, repeats| {
        floor(
            &mut gpu,
            &cache,
            Primary::Traced,
            4.,
            repeats,
            MaterialDescriptor {
                roughness: 0.3,
                ..Default::default()
            },
            Some(ObjectTextures {
                normal: Some(NormalMap {
                    texture: ObjectTexture {
                        data,
                        sampler,
                        key: None,
                    },
                    tangents: &tangents,
                }),
                ..Default::default()
            }),
        )
    };
    let from_blocks = mapped((&tilted).into(), nearest, 1.)?;
    let from_texels = mapped((&rebuilt).into(), nearest, 1.)?;
    let flat_map = bc5(|_| bc5_block(128, 128));
    let flat = mapped((&flat_map).into(), nearest, 1.)?;
    let (worst, _) = apart(&from_blocks, &from_texels, everywhere);
    let (tilt_shows, _) = apart(&from_blocks, &flat, everywhere);
    ensure!(
        worst < 5e-3 && tilt_shows > 0.02,
        "BC5: {worst:.5} from the CPU-rebuilt map, {tilt_shows:.4} from the flat floor"
    );
    report += &format!("BC5 Z: {worst:.5} from the rebuilt map, {tilt_shows:.3} from flat; ");

    // The variance layer: rough at the top level only (columns alternating X = -0.6 and
    // +0.6), flat below it, drawn where the footprint is four texels and more.
    let striped = {
        // BC4 red: endpoints 204 and 51 (the eight-value palette), index 1 (51) in the
        // even columns and 0 (204) in the odd ones; green stays 128.
        let mut red = 204u64 | 51 << 8;
        for texel in (0..16).step_by(2) {
            red |= 1 << (16 + 3 * texel);
        }
        let mut columns = bc5_block(128, 128);
        columns[..8].copy_from_slice(&red.to_le_bytes());
        bc5(move |l| if l == 0 { columns } else { bc5_block(128, 128) })
    };
    // 64 repeats of 16 texels across about 100 pixels: a footprint of ten texels, so the
    // levels sampled are the flat ones. The rough map's variance there is its layer 1.
    let rough_above = variance_seen(&mut gpu, &cache, &striped, 64.)?;
    let flat_below = variance_seen(&mut gpu, &cache, &flat_map, 64.)?;
    ensure!(
        rough_above > 0.05 && flat_below < 1e-3,
        "the variance layer: {rough_above:.4} seen for a map that is rough at its top level, \
         {flat_below:.4} for a flat one, where both sample flat levels"
    );
    report += &format!("variance layer: {rough_above:.3} rough, {flat_below:.4} flat");

    // An identity keeps naming the texture it named: RGBA8 texels, then blocks of the
    // same size and levels, is refused.
    let (blocks, decoded) = bc7(32, BlockFormat::Bc7Srgb, 23);
    let mut named = Frames::default();
    let mut keyed = |frames: &mut Frames, data| {
        floor_in(
            &mut gpu,
            &cache,
            Some(frames),
            Primary::Traced,
            4.,
            1.,
            MaterialDescriptor::default(),
            Some(ObjectTextures {
                base_color: Some(ObjectTexture {
                    data,
                    sampler: nearest,
                    key: Some(9),
                }),
                ..Default::default()
            }),
        )
    };
    keyed(&mut named, (&decoded).into())?;
    ensure!(
        keyed(&mut named, (&blocks).into()).is_err(),
        "an identity that named RGBA8 texels was reused for blocks"
    );
    named.retire(&mut gpu)?;
    eprintln!("compressed textures: {report}");
    cache.retire(&mut gpu)?;
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
