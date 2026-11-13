//! Block-compressed material textures (`asset::CompressedTexture`): BC7 drawn from its blocks
//! equals the same texels uploaded as RGBA8, bit for bit; BC5 normals (Z rebuilt in the shading
//! API) equal an RGBA8 map whose Z was rebuilt on the CPU, within BC5's decode precision. The
//! blocks are built here from known endpoints and indices, with a reference decode of each.
use super::*;
use crate::asset::{
    BlockFormat, CompressedTexture, Filter, Mip, MipmapFilter, Sampler, TangentBasis,
    TangentStream, TextureRole,
};

/// A deterministic stream of bytes.
struct Bits(u64);
impl Bits {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
}
/// BC7 mode 6 weights (4-bit indices).
const BC7_WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
/// One BC7 mode-6 block from random endpoints and indices, and its 16 decoded RGBA8 texels in
/// row-major order (the format's own interpolation, which every decoder must reproduce).
fn bc7_block(bits: &mut Bits) -> ([u8; 16], [[u8; 4]; 16]) {
    let e: [[u32; 2]; 4] = std::array::from_fn(|_| [bits.next() & 127, bits.next() & 127]);
    let p = [bits.next() & 1, bits.next() & 1];
    // The anchor (texel 0) keeps its index's top bit zero.
    let idx: [u32; 16] = std::array::from_fn(|i| bits.next() & if i == 0 { 7 } else { 15 });
    let mut block = 0u128;
    let mut at = 0;
    let mut put = |value: u32, width: u32| {
        block |= u128::from(value) << at;
        at += width;
    };
    put(1 << 6, 7);
    for c in e {
        put(c[0], 7);
        put(c[1], 7);
    }
    put(p[0], 1);
    put(p[1], 1);
    for (i, &k) in idx.iter().enumerate() {
        put(k, if i == 0 { 3 } else { 4 });
    }
    let texels = idx.map(|k| {
        let w = BC7_WEIGHTS[k as usize];
        std::array::from_fn(|c| {
            let (a, b) = ((e[c][0] << 1) | p[0], (e[c][1] << 1) | p[1]);
            (((64 - w) * a + w * b + 32) >> 6) as u8
        })
    });
    (block.to_le_bytes(), texels)
}
/// One BC4 channel block (r0 > r1: eight interpolated values), and its 16 decoded values in
/// 0-1, as the format defines them.
fn bc4_block(bits: &mut Bits) -> ([u8; 8], [f32; 16]) {
    let (a, b) = ((bits.next() & 255) as u8, (bits.next() & 255) as u8);
    // r0 > r1 selects the eight-value palette.
    let (r0, r1) = match a.cmp(&b) {
        std::cmp::Ordering::Greater => (a, b),
        std::cmp::Ordering::Less => (b, a),
        std::cmp::Ordering::Equal => (a.max(1), a.max(1) - 1),
    };
    let idx: [u32; 16] = std::array::from_fn(|_| bits.next() & 7);
    let mut block = u64::from(r0) | u64::from(r1) << 8;
    for (i, &k) in idx.iter().enumerate() {
        block |= u64::from(k) << (16 + 3 * i);
    }
    let (a, b) = (f32::from(r0), f32::from(r1));
    let values = idx.map(|k| match k {
        0 => a / 255.0,
        1 => b / 255.0,
        k => ((8 - k) as f32 * a + (k - 1) as f32 * b) / 7.0 / 255.0,
    });
    (block.to_le_bytes(), values)
}
/// A `side` x `side` BC7 texture (a full chain) and the same texels per level as RGBA8.
fn bc7_texture(side: u32, srgb: bool, seed: u64) -> (CompressedTexture, asset::Texture) {
    let mut bits = Bits(seed);
    let (mut levels, mut mips) = (vec![], vec![]);
    let mut w = side;
    loop {
        let blocks = w.div_ceil(4);
        let mut data = vec![];
        let mut texels = vec![[0u8; 4]; (blocks * 4 * blocks * 4) as usize];
        for by in 0..blocks {
            for bx in 0..blocks {
                let (block, decoded) = bc7_block(&mut bits);
                data.extend(block);
                for (t, texel) in decoded.into_iter().enumerate() {
                    let (x, y) = (bx * 4 + t as u32 % 4, by * 4 + t as u32 / 4);
                    texels[(y * blocks * 4 + x) as usize] = texel;
                }
            }
        }
        // The level's own texels: the top-left w x w of its padded blocks.
        let rgba8 = (0..w)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| texels[(y * blocks * 4 + x) as usize])
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
    let format = if srgb {
        BlockFormat::Bc7Srgb
    } else {
        BlockFormat::Bc7Linear
    };
    (
        CompressedTexture {
            format,
            width: side,
            height: side,
            mips: levels,
        },
        asset::Texture {
            role: format.role(),
            mips,
        },
    )
}
/// A BC5 normal map and its RGBA8 counterpart with Z rebuilt per texel on the CPU (as Ara's
/// bridge rebuilds it), quantized to 8 bits.
fn bc5_texture(side: u32, seed: u64) -> (CompressedTexture, asset::Texture) {
    let mut bits = Bits(seed);
    let (mut levels, mut mips) = (vec![], vec![]);
    let mut w = side;
    loop {
        let blocks = w.div_ceil(4);
        let mut data = vec![];
        let mut xy = vec![[0f32; 2]; (blocks * 4 * blocks * 4) as usize];
        for by in 0..blocks {
            for bx in 0..blocks {
                let (r, rv) = bc4_block(&mut bits);
                let (g, gv) = bc4_block(&mut bits);
                data.extend(r);
                data.extend(g);
                for t in 0..16u32 {
                    let (x, y) = (bx * 4 + t % 4, by * 4 + t / 4);
                    xy[(y * blocks * 4 + x) as usize] = [rv[t as usize], gv[t as usize]];
                }
            }
        }
        let rgba8 = (0..w)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let [r, g] = xy[(y * blocks * 4 + x) as usize];
                let (nx, ny) = (r * 2.0 - 1.0, g * 2.0 - 1.0);
                let z = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                let q = |v: f32| (v * 255.0).round() as u8;
                [q(r), q(g), q(z * 0.5 + 0.5), 255]
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
    (
        CompressedTexture {
            format: BlockFormat::Bc5,
            width: side,
            height: side,
            mips: levels,
        },
        asset::Texture {
            role: TextureRole::NormalLinear,
            mips,
        },
    )
}

/// Every level's bytes follow the block rule (levels under 4x4 still take a whole block), the
/// chain must run to 1x1, and the base must be whole blocks.
#[test]
fn compressed_texture_layout_and_validation() {
    assert_eq!(BlockFormat::level_bytes(1, 1), 16);
    assert_eq!(BlockFormat::level_bytes(2, 8), 32);
    assert_eq!(BlockFormat::level_bytes(16, 12), 16 * 4 * 3);
    let (t, rgba) = bc7_texture(16, true, 1);
    assert_eq!(t.mips.len(), 5);
    assert_eq!(rgba.mips.len(), 5);
    assert_eq!(t.validate(16384).unwrap(), 16 * (16 + 4 + 1 + 1 + 1));
    let mut short = t.clone();
    short.mips.pop();
    assert!(short.validate(16384).is_err());
    let mut ragged = t.clone();
    ragged.width = 18;
    assert!(ragged.validate(16384).is_err());
    let mut wrong = t;
    wrong.mips[2].pop();
    assert!(wrong.validate(16384).is_err());
}

/// The same quad drawn with a texture from blocks and from their decode: BC7 base colours
/// (sRGB and linear, every level, nearest and trilinear sampling) bit for bit, BC5 normals
/// within two codes of the CPU-rebuilt map.
#[test]
#[ignore = "Vulkan GPU with TEXTURE_COMPRESSION_BC"]
fn gpu_compressed_textures_match_their_decode() {
    let _validation = validation::begin(true);
    let Some((device, queue)) = device_with(wgpu::Features::TEXTURE_COMPRESSION_BC) else {
        eprintln!("adapter without TEXTURE_COMPRESSION_BC: skipped");
        return;
    };
    let mut r = suite_renderer(device, queue, SRGB, RendererOptions::default(), SOURCE).unwrap();
    assert!(r.compressed_textures());
    let color = color_target(&r, 193, 193);
    let cam = camera(0.0);
    let corners = [
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 0.0],
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
    ];
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let vertices: Vec<Vertex> = corners
        .iter()
        .zip(uvs)
        .map(|(&position, uv)| Vertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv,
            color: [1.0; 4],
        })
        .collect();
    let tangents = TangentStream {
        basis: TangentBasis::Authored,
        values: vec![[1.0, 0.0, 0.0, 1.0]; 4],
    };
    r.test_drain("compressed mesh admission");
    let mesh = r
        .try_upload_mesh_with_tangents(
            &MeshData {
                vertices: &vertices,
                indices: &[0, 1, 2, 0, 2, 3],
            },
            Some(&tangents),
        )
        .unwrap()
        .unwrap();
    let mut inst = instances(&mut r, &[Mat4::IDENTITY]);
    let compressed = |r: &mut Renderer, data: &CompressedTexture| {
        r.test_drain("compressed texture admission");
        r.try_upload_compressed_texture(data).unwrap().unwrap()
    };
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
    // Base colour and data, magnified (level 0) and minified (the small levels).
    for (srgb, seed) in [(true, 7), (false, 11)] {
        let (blocks, decoded) = bc7_texture(16, srgb, seed);
        let a = compressed(&mut r, &blocks);
        let b = texture(&mut r, &decoded);
        for (sampler, model) in [
            (nearest, Mat4::IDENTITY),
            (trilinear, Mat4::IDENTITY),
            (trilinear, Mat4::from_scale(Vec3::splat(0.04))),
        ] {
            inst.set(0, model).unwrap();
            let mut shot = |r: &mut Renderer, t: &Texture| {
                let binding = Some(TextureBinding {
                    texture: t,
                    sampler,
                });
                let m = material(
                    r,
                    &if srgb {
                        MaterialDescriptor {
                            base_color_texture: binding,
                            ..Default::default()
                        }
                    } else {
                        MaterialDescriptor {
                            metallic_roughness_texture: binding,
                            ..Default::default()
                        }
                    },
                );
                let mode = if srgb {
                    ViewMode::BaseColor
                } else {
                    ViewMode::Roughness
                };
                pixels(
                    r,
                    &color,
                    &cam,
                    &mut [Draw::new(&mesh, &mut inst).with_material(&m)],
                    mode,
                )
            };
            let (x, y) = (shot(&mut r, &a), shot(&mut r, &b));
            assert_eq!(x, y, "BC7 srgb={srgb} sampler={sampler:?}");
        }
    }
    // BC5 normals against the CPU-rebuilt RGBA8 map.
    inst.set(0, Mat4::IDENTITY).unwrap();
    let (blocks, decoded) = bc5_texture(16, 13);
    let a = compressed(&mut r, &blocks);
    let b = texture(&mut r, &decoded);
    let mut shot = |r: &mut Renderer, t: &Texture| {
        let m = material(
            r,
            &MaterialDescriptor {
                normal_texture: Some(TextureBinding {
                    texture: t,
                    sampler: nearest,
                }),
                double_sided: true,
                ..Default::default()
            },
        );
        pixels(
            r,
            &color,
            &cam,
            &mut [Draw::new(&mesh, &mut inst).with_material(&m)],
            ViewMode::Normal,
        )
    };
    let (x, y) = (shot(&mut r, &a), shot(&mut r, &b));
    let worst = x.iter().zip(&y).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
    assert!(worst <= 2, "BC5 normals differ by {worst} codes");
    assert_ne!(x.iter().filter(|&&v| v != 0).count(), 0);
}
