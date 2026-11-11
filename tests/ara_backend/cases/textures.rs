//! Material textures on the assembled frame, against untextured frames of the same
//! values.
use super::*;
use renderlab::{
    Camera, Light, Lighting, Mat4, MaterialDescriptor, Vec3, Vertex,
    asset::{Filter, Mip, MipmapFilter, Sampler, Texture, TextureRole, Wrap},
    backend_ara::forward::{
        ForwardObject, ForwardScene, ObjectTexture, ObjectTextures, Primary, SceneTextures,
        SunShadows, render,
    },
};

const SIZE: [u32; 2] = [128, 96];

/// sRGB decoding of one byte, as a base-colour texture is sampled.
fn linear(byte: u8) -> f32 {
    let c = f32::from(byte) / 255.;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// A floor facing the camera from `distance`, its uv running 0 to `repeats` across it,
/// under a sun and a uniform sky. Returns the HDR as RGB triples.
fn floor(
    gpu: &mut GpuDevice,
    cache: &ShaderCache,
    primary: Primary,
    distance: f32,
    repeats: f32,
    material: MaterialDescriptor<'_>,
    textures: Option<ObjectTextures<'_>>,
) -> Result<Vec<[f32; 3]>> {
    let vertices =
        [[-4., -4., 0.], [4., -4., 0.], [4., 4., 0.], [-4., 4., 0.]].map(|position: [f32; 3]| {
            Vertex {
                position,
                normal: [0., 0., 1.],
                // u to the right, v down the image.
                uv: [
                    (position[0] + 4.) / 8. * repeats,
                    (4. - position[1]) / 8. * repeats,
                ],
                color: [1.; 4],
            }
        });
    let lighting = Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-0.3, 0.2, -1.),
            lux: [1000.; 3],
        }],
        environment: [30., 30., 30.],
        exposure_stops: -8.,
    };
    let shadows = SunShadows {
        split_ends: vec![12.],
        disk_angle: 0.02,
    };
    let slots = textures.map(|t| [t]);
    let images = render(
        gpu,
        cache,
        &ForwardScene {
            objects: &[ForwardObject {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
                transform: Mat4::IDENTITY,
                material: MaterialDescriptor {
                    double_sided: true,
                    ..material
                },
            }],
            lighting: &lighting,
            camera: Camera::look_at(
                Vec3::new(0., 0., distance),
                Vec3::ZERO,
                Vec3::Y,
                55f32.to_radians(),
                0.1,
                100.,
            )?,
            size: SIZE,
            atmosphere: renderlab::Atmosphere::off(),
            sun_shadows: (primary != Primary::Raster).then_some(&shadows),
            primary,
            post: None,
            indirect: None,
            reconstruction: None,
            reflections: None,
            textures: slots.as_ref().map(|objects| SceneTextures {
                objects,
                anisotropy_clamp: 1,
            }),
            local_shadows: None,
        },
    )?;
    Ok(images
        .hdr
        .as_chunks::<8>()
        .0
        .iter()
        .map(|p| {
            std::array::from_fn(|c| {
                f16::from_bits(u16::from_le_bytes([p[c * 2], p[c * 2 + 1]])).to_f32()
            })
        })
        .collect())
}

/// The largest relative difference between two frames over the pixels `inside` keeps.
fn apart(a: &[[f32; 3]], b: &[[f32; 3]], inside: impl Fn(u32, u32) -> bool) -> (f32, usize) {
    let (mut worst, mut pixels) = (0f32, 0);
    for y in 0..SIZE[1] {
        for x in 0..SIZE[0] {
            if !inside(x, y) {
                continue;
            }
            let at = (y * SIZE[0] + x) as usize;
            for c in 0..3 {
                worst = worst.max(
                    (a[at][c] - b[at][c]).abs() / a[at][c].abs().max(b[at][c].abs()).max(1e-3),
                );
            }
            pixels += 1;
        }
    }
    (worst, pixels)
}

/// A 2x2 base-colour texture and a 2x2 metallic-roughness texture on a floor, sampled
/// nearest, on the mesh pass and on the traced frame. Each quadrant of the floor must
/// be the untextured floor whose factors are that texel's values: its colour decoded
/// from sRGB, its roughness from green and its metalness from blue. That holds the
/// texture upload, both formats, the per-object and per-draw bindings and the samplers
/// to the same shading the untextured frame already passed.
#[test]
fn a_textured_floor_is_quadrant_by_quadrant_the_untextured_floor_of_that_texel() -> Result<()> {
    let path = root("textures-texels");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    // Texels in row order: top left, top right, bottom left, bottom right.
    let colours: [[u8; 3]; 4] = [[230, 60, 40], [40, 200, 90], [60, 90, 220], [200, 200, 200]];
    let surfaces: [[u8; 2]; 4] = [[255, 0], [128, 0], [255, 255], [90, 255]];
    let nearest = Sampler {
        wrap_u: Wrap::Clamp,
        wrap_v: Wrap::Clamp,
        mag: Filter::Nearest,
        min: Filter::Nearest,
        mipmap: MipmapFilter::Nearest,
    };
    let base = Texture {
        role: TextureRole::BaseColorSrgb,
        mips: vec![Mip {
            width: 2,
            height: 2,
            rgba8: colours
                .iter()
                .flat_map(|c| [c[0], c[1], c[2], 255])
                .collect(),
        }],
    };
    // glTF packing: roughness in green, metalness in blue.
    let surface = Texture {
        role: TextureRole::MetallicRoughnessLinear,
        mips: vec![Mip {
            width: 2,
            height: 2,
            rgba8: surfaces.iter().flat_map(|s| [0, s[0], s[1], 255]).collect(),
        }],
    };
    let textures = ObjectTextures {
        base_color: Some(ObjectTexture {
            data: &base,
            sampler: nearest,
        }),
        metallic_roughness: Some(ObjectTexture {
            data: &surface,
            sampler: nearest,
        }),
        normal: None,
    };
    let mut report = String::new();
    for primary in [Primary::Raster, Primary::Traced] {
        let textured = floor(
            &mut gpu,
            &cache,
            primary,
            4.,
            1.,
            MaterialDescriptor::default(),
            Some(textures),
        )?;
        let mut worst = 0f32;
        for (texel, (colour, [roughness, metal])) in colours.iter().zip(surfaces).enumerate() {
            let reference = floor(
                &mut gpu,
                &cache,
                primary,
                4.,
                1.,
                MaterialDescriptor {
                    base_color: [linear(colour[0]), linear(colour[1]), linear(colour[2]), 1.],
                    metallic: f32::from(metal) / 255.,
                    roughness: f32::from(roughness) / 255.,
                    ..Default::default()
                },
                None,
            )?;
            // The texel's quadrant, three pixels clear of its borders.
            let (left, top) = (texel % 2 == 0, texel / 2 == 0);
            let inside = |x: u32, y: u32| {
                let (half_w, half_h) = (SIZE[0] / 2, SIZE[1] / 2);
                (if left {
                    x + 3 < half_w
                } else {
                    x >= half_w + 3
                }) && (if top { y + 3 < half_h } else { y >= half_h + 3 })
            };
            let (difference, pixels) = apart(&textured, &reference, inside);
            ensure!(
                pixels > 2000 && difference < 0.005,
                "{primary:?}: texel {texel} differs from its untextured floor by {difference} \
                 over {pixels} pixels"
            );
            worst = worst.max(difference);
        }
        report += &format!("{primary:?}: four quadrants within {worst:.5}; ");
    }
    eprintln!("{report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}

/// A mip chain whose levels are flat and distinct, tiled so that one pixel covers about
/// four texels from 4 m and about two from 2 m, sampled with nearest mip selection: the
/// floor must be the untextured floor of level two's colour from 4 m and of level one's
/// from 2 m. The mesh pass selects the level from screen derivatives, the traced frame
/// from each pixel's ray footprint; both must land on the same level.
#[test]
fn a_mip_chain_shows_the_level_the_pixel_footprint_selects() -> Result<()> {
    let path = root("textures-mips");
    let mut gpu = GpuDevice::new(&[], path.join("device"))?;
    let cache = ShaderCache::new(path.join("shaders"))?;
    let greys: [u8; 7] = [250, 40, 200, 90, 160, 20, 120];
    let chain = Texture {
        role: TextureRole::BaseColorSrgb,
        mips: greys
            .iter()
            .enumerate()
            .map(|(level, grey)| {
                let side = 64u32 >> level;
                Mip {
                    width: side,
                    height: side,
                    rgba8: [*grey, *grey, *grey, 255].repeat((side * side) as usize),
                }
            })
            .collect(),
    };
    let textures = ObjectTextures {
        base_color: Some(ObjectTexture {
            data: &chain,
            sampler: Sampler {
                wrap_u: Wrap::Repeat,
                wrap_v: Wrap::Repeat,
                mag: Filter::Linear,
                min: Filter::Linear,
                mipmap: MipmapFilter::Nearest,
            },
        }),
        metallic_roughness: None,
        normal: None,
    };
    let material = |grey: u8| MaterialDescriptor {
        base_color: [linear(grey), linear(grey), linear(grey), 1.],
        metallic: 0.,
        roughness: 1.,
        ..Default::default()
    };
    let mut report = String::new();
    for primary in [Primary::Raster, Primary::Traced] {
        for (distance, level) in [(4f32, 2usize), (2., 1)] {
            // Texels a pixel covers: 64 x 12 texels over the floor's 8 m, against the
            // view's height in metres over its 96 rows.
            let density =
                64. * 12. / 8. * (2. * distance * (27.5f32.to_radians()).tan()) / SIZE[1] as f32;
            ensure!(
                (density.log2() - level as f32).abs() < 0.2,
                "the fixture's footprint is {density} texels, not level {level}"
            );
            let textured = floor(
                &mut gpu,
                &cache,
                primary,
                distance,
                12.,
                MaterialDescriptor {
                    metallic: 0.,
                    roughness: 1.,
                    ..Default::default()
                },
                Some(textures),
            )?;
            let centre = |x: u32, y: u32| (16..112).contains(&x) && (12..84).contains(&y);
            let mut nearest = (f32::MAX, 0);
            for (candidate, grey) in greys.iter().enumerate() {
                let reference = floor(
                    &mut gpu,
                    &cache,
                    primary,
                    distance,
                    12.,
                    material(*grey),
                    None,
                )?;
                let difference = apart(&textured, &reference, centre).0;
                if difference < nearest.0 {
                    nearest = (difference, candidate);
                }
            }
            ensure!(
                nearest == (nearest.0, level) && nearest.0 < 0.005,
                "{primary:?} from {distance} m shows level {} within {} (expected level {level})",
                nearest.1,
                nearest.0
            );
            report += &format!(
                "{primary:?} from {distance} m: level {level} within {:.5}; ",
                nearest.0
            );
        }
    }
    eprintln!("{report}");
    gpu.collect()?;
    gpu.assert_clean()?;
    Ok(())
}
