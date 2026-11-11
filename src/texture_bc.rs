//! Block-compressed material textures (U-BC, docs/texture-compression.md).
//!
//! A `TextureFormat::Bc7` or `Bc5` texture uploads its blocks as they are when the
//! device has `TEXTURE_COMPRESSION_BC` and level 0 is whole blocks (wgpu requires that
//! of a compressed texture). Otherwise each level is decoded to RGBA8 as it is written,
//! so callers never branch on the device and the retained source stays compressed.
//!
//! A BC5 normal map holds tangent-space X and Y only. Uploaded as blocks or decoded, the
//! sampling helpers rebuild Z from the filtered sample (`rl_normal_texel`,
//! shaders/program_api.wgsl), so both kinds of device shade it the same way.
use crate::Error;
use crate::asset::{Mip, Texture, TextureFormat, TextureRole};

const BLOCK: usize = TextureFormat::BLOCK as usize;

/// How a device holds a prepared texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Storage {
    /// The source's own bytes: uncompressed texels, or blocks the device samples.
    Source,
    /// RGBA8 decoded from blocks at upload.
    Decoded,
}

/// How a device with `features` holds `data`.
pub(crate) fn storage(features: wgpu::Features, data: &Texture) -> Storage {
    let whole_blocks = data.mips.first().is_some_and(|m| {
        m.width.is_multiple_of(TextureFormat::BLOCK)
            && m.height.is_multiple_of(TextureFormat::BLOCK)
    });
    if !data.format.compressed()
        || (features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) && whole_blocks)
    {
        Storage::Source
    } else {
        Storage::Decoded
    }
}

/// Validates `data` (`material::validate_texture`) and returns how this device holds
/// it and the GPU bytes of its whole chain, which `max_bytes` also bounds.
pub(crate) fn admit(
    data: &Texture,
    features: wgpu::Features,
    max_dimension: u32,
    max_bytes: u64,
) -> Result<(Storage, u64), Error> {
    crate::material::validate_texture(data, max_dimension, max_bytes)?;
    let storage = storage(features, data);
    let bytes = data.mips.iter().map(|m| level_bytes(m, storage)).sum();
    if bytes > max_bytes {
        return Err(Error::InvalidInput("texture exceeds max_texture_bytes"));
    }
    Ok((storage, bytes))
}

/// The GPU bytes of one level.
pub(crate) fn level_bytes(mip: &Mip, storage: Storage) -> u64 {
    match storage {
        Storage::Source => mip.rgba8.len() as u64,
        Storage::Decoded => u64::from(mip.width) * u64::from(mip.height) * 4,
    }
}

/// The GPU format of a texture as held: base colour decodes sRGB when sampled.
pub(crate) fn gpu_format(data: &Texture, storage: Storage) -> wgpu::TextureFormat {
    let srgb = data.role == TextureRole::BaseColorSrgb;
    match (data.format, storage) {
        (TextureFormat::Bc7, Storage::Source) if srgb => wgpu::TextureFormat::Bc7RgbaUnormSrgb,
        (TextureFormat::Bc7, Storage::Source) => wgpu::TextureFormat::Bc7RgbaUnorm,
        (TextureFormat::Bc5, Storage::Source) => wgpu::TextureFormat::Bc5RgUnorm,
        _ if srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        _ if data.role == TextureRole::NormalSlopeMoments => wgpu::TextureFormat::Rgba16Float,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// How one level's bytes are laid out for a copy, and the extent the copy covers: whole
/// blocks for sampled blocks, so a level smaller than a block copies one block.
pub(crate) fn level_copy(
    data: &Texture,
    mip: &Mip,
    storage: Storage,
) -> (wgpu::TexelCopyBufferLayout, wgpu::Extent3d) {
    let layers = data.role.layers();
    let (row_bytes, rows, width, height) = if data.format.compressed() && storage == Storage::Source
    {
        let block = TextureFormat::BLOCK;
        let (across, down) = (mip.width.div_ceil(block), mip.height.div_ceil(block));
        (
            across * TextureFormat::BLOCK_BYTES as u32,
            down,
            across * block,
            down * block,
        )
    } else {
        (
            mip.width * (data.role.texel_bytes() as u32 / layers),
            mip.height,
            mip.width,
            mip.height,
        )
    };
    (
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
    )
}

/// Queues source level `mip` into `texture`'s level `level`, decoded first when the
/// device holds RGBA8. Returns the bytes written.
pub(crate) fn write_level(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    data: &Texture,
    mip: &Mip,
    storage: Storage,
) -> u64 {
    let decoded;
    let bytes = match storage {
        Storage::Source => &mip.rgba8,
        Storage::Decoded => {
            decoded = decode_level(data.format, mip);
            &decoded.rgba8
        }
    };
    let (layout, extent) = level_copy(data, mip, storage);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            mip_level: level,
            ..texture.as_image_copy()
        },
        bytes,
        layout,
        extent,
    );
    bytes.len() as u64
}

/// The texture as RGBA8, each level decoded from its own blocks; an uncompressed
/// texture is returned as it is. BC5 gives X and Y in R and G, the unit normal's Z in B
/// (rounded to 8 bits) and A = 255: a three-channel normal map. Level bytes must be
/// those `Texture::level_bytes` gives; missing blocks decode to zero.
pub fn decode_texture(data: &Texture) -> Texture {
    if !data.format.compressed() {
        return data.clone();
    }
    Texture {
        role: data.role,
        format: TextureFormat::Uncompressed,
        mips: data
            .mips
            .iter()
            .map(|mip| decode_level(data.format, mip))
            .collect(),
    }
}

fn decode_level(format: TextureFormat, mip: &Mip) -> Mip {
    let (width, height) = (mip.width as usize, mip.height as usize);
    let mut rgba8 = vec![0; width * height * 4];
    let mut blocks = mip.rgba8.chunks_exact(TextureFormat::BLOCK_BYTES);
    for top in (0..height).step_by(BLOCK) {
        for left in (0..width).step_by(BLOCK) {
            let Some(block) = blocks.next() else {
                break;
            };
            let texels = decode_block(format, block);
            // Edge blocks keep only the texels inside the level.
            for y in 0..BLOCK.min(height - top) {
                let across = BLOCK.min(width - left);
                let at = ((top + y) * width + left) * 4;
                rgba8[at..at + across * 4].copy_from_slice(&texels[y * 16..y * 16 + across * 4]);
            }
        }
    }
    Mip {
        width: mip.width,
        height: mip.height,
        rgba8,
    }
}

/// One block's 16 RGBA8 texels in row order.
fn decode_block(format: TextureFormat, block: &[u8]) -> [u8; 64] {
    let mut texels = [0; 64];
    match format {
        TextureFormat::Bc7 => bcdec_rs::bc7(block, &mut texels, 16),
        TextureFormat::Bc5 => {
            let mut xy = [0; 32];
            bcdec_rs::bc5(block, &mut xy, 8, false);
            for (texel, pair) in texels.chunks_exact_mut(4).zip(xy.chunks_exact(2)) {
                let [x, y] = [pair[0], pair[1]].map(|v| f32::from(v) / 255.0 * 2.0 - 1.0);
                let z = (1.0 - x * x - y * y).max(0.0).sqrt();
                let blue = ((z * 0.5 + 0.5) * 255.0).round() as u8;
                texel.copy_from_slice(&[pair[0], pair[1], blue, 255]);
            }
        }
        TextureFormat::Uncompressed => unreachable!("only compressed levels decode"),
    }
    texels
}

#[cfg(test)]
pub(crate) mod fixture {
    //! BC test data without an encoder dependency: one block per BC7 mode with seeded
    //! contents (the decoder's ground truth is the GPU, `gpu_bc_decode_matches_cpu`), and
    //! smooth images encoded by the two simplest encoders: BC7 mode 6 (one line through
    //! colour space per block, so a block whose channels vary independently is coarse)
    //! and BC5 with endpoints at each block's range. Tests compare a render of the
    //! blocks with a render of their decode, so the encoders' quality does not matter.
    use super::*;

    /// A deterministic byte stream (SplitMix64).
    pub fn bytes(seed: u64, count: usize) -> Vec<u8> {
        let mut state = seed;
        (0..count.div_ceil(8))
            .flat_map(|_| {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                (z ^ (z >> 31)).to_le_bytes()
            })
            .take(count)
            .collect()
    }

    /// Seeded blocks for a `width` x `height` level.
    fn seeded(seed: u64, width: u32, height: u32) -> Mip {
        let blocks = (width.div_ceil(4) * height.div_ceil(4)) as usize;
        Mip {
            width,
            height,
            rgba8: bytes(seed, blocks * 16),
        }
    }

    /// A level of BC7 blocks cycling through modes 0-7, their other bits seeded.
    pub fn bc7_all_modes(seed: u64, width: u32, height: u32) -> Mip {
        let mut mip = seeded(seed, width, height);
        for (index, block) in mip.rgba8.chunks_exact_mut(16).enumerate() {
            let mode = index % 8;
            // The mode is the position of the lowest set bit of the first byte.
            block[0] = (block[0] | 1 << mode) & !((1 << mode) - 1);
        }
        mip
    }

    /// A level of seeded BC5 blocks: every endpoint order and index pattern.
    pub fn bc5_seeded(seed: u64, width: u32, height: u32) -> Mip {
        seeded(seed, width, height)
    }

    /// A full chain of independently seeded levels, so every filter weight shows in a
    /// sample: BC7 in every mode, or BC5.
    pub fn seeded_chain(role: TextureRole, format: TextureFormat, size: u32, seed: u64) -> Texture {
        let levels = size.trailing_zeros() + 1;
        Texture {
            role,
            format,
            mips: (0..levels)
                .map(|level| {
                    let (side, seed) = (size >> level, seed + u64::from(level));
                    match format {
                        TextureFormat::Bc5 => bc5_seeded(seed, side, side),
                        _ => bc7_all_modes(seed, side, side),
                    }
                })
                .collect(),
        }
    }

    /// BC7 mode 6 (one subset, 7-bit RGBA endpoints with a shared low bit each, 4-bit
    /// indices) of tightly packed RGBA8: endpoints at the block's per-channel range,
    /// each texel the nearest of the 16 steps along it.
    pub fn encode_bc7(width: u32, height: u32, rgba8: &[u8]) -> Vec<u8> {
        const WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
        let mut out = Vec::new();
        for_blocks(width, height, rgba8, |texels| {
            let mut low = [255u8; 4];
            let mut high = [0u8; 4];
            for texel in texels.chunks_exact(4) {
                for (c, &v) in texel.iter().enumerate() {
                    low[c] = low[c].min(v);
                    high[c] = high[c].max(v);
                }
            }
            // Endpoints as 7 bits plus the shared low bit, expanded as the decoder does.
            let mut ends = [low, high].map(|e| {
                let p = u8::from(e.iter().map(|&v| u32::from(v & 1)).sum::<u32>() >= 2);
                (e.map(|v| v >> 1), p)
            });
            let expanded = ends.map(|(e, p)| e.map(|v| v << 1 | p));
            let index = |texel: &[u8]| {
                (0..16u32)
                    .min_by_key(|&i| {
                        (0..4)
                            .map(|c| {
                                let [a, b] = [expanded[0][c], expanded[1][c]].map(u32::from);
                                let v =
                                    ((64 - WEIGHTS[i as usize]) * a + WEIGHTS[i as usize] * b + 32)
                                        >> 6;
                                v.abs_diff(u32::from(texel[c])).pow(2)
                            })
                            .sum::<u32>()
                    })
                    .unwrap()
            };
            let mut indices: Vec<u32> = texels.chunks_exact(4).map(index).collect();
            // The first index has no top bit: swap the endpoints when it would need one.
            if indices[0] >= 8 {
                ends.swap(0, 1);
                indices.iter_mut().for_each(|i| *i = 15 - *i);
            }
            let mut bits = Bits::default();
            bits.push(1 << 6, 7);
            for c in 0..4 {
                bits.push(u64::from(ends[0].0[c]), 7);
                bits.push(u64::from(ends[1].0[c]), 7);
            }
            bits.push(u64::from(ends[0].1), 1);
            bits.push(u64::from(ends[1].1), 1);
            bits.push(u64::from(indices[0]), 3);
            for &i in &indices[1..] {
                bits.push(u64::from(i), 4);
            }
            out.extend(bits.0.to_le_bytes());
        });
        out
    }

    /// BC5 of the R and G channels of tightly packed RGBA8: per channel, endpoints at the
    /// block's range (the eight-value form) and each texel the nearest step.
    pub fn encode_bc5(width: u32, height: u32, rgba8: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for_blocks(width, height, rgba8, |texels| {
            for c in 0..2 {
                let values: Vec<u8> = texels.chunks_exact(4).map(|t| t[c]).collect();
                let (low, high) = (*values.iter().min().unwrap(), *values.iter().max().unwrap());
                // high > low selects six interpolated values; equal endpoints decode to
                // the endpoint for every index.
                let palette: Vec<f32> = [7, 0, 6, 5, 4, 3, 2, 1]
                    .iter()
                    .map(|&w| (f32::from(high) * w as f32 + f32::from(low) * (7 - w) as f32) / 7.0)
                    .collect();
                let mut bits = Bits::default();
                bits.push(u64::from(high), 8);
                bits.push(u64::from(low), 8);
                for v in values {
                    let nearest = (0..8)
                        .min_by(|&a, &b| {
                            (palette[a] - f32::from(v))
                                .abs()
                                .total_cmp(&(palette[b] - f32::from(v)).abs())
                        })
                        .unwrap();
                    bits.push(nearest as u64, 3);
                }
                out.extend(&bits.0.to_le_bytes()[..8]);
            }
        });
        out
    }

    /// Calls `block` with each 4x4 block's RGBA8 texels in row order, edge texels repeated.
    fn for_blocks(width: u32, height: u32, rgba8: &[u8], mut block: impl FnMut(&[u8])) {
        let (width, height) = (width as usize, height as usize);
        for top in (0..height).step_by(4) {
            for left in (0..width).step_by(4) {
                let mut gathered = Vec::with_capacity(64);
                for y in 0..4 {
                    for x in 0..4 {
                        let at =
                            ((top + y).min(height - 1) * width + (left + x).min(width - 1)) * 4;
                        gathered.extend(&rgba8[at..at + 4]);
                    }
                }
                block(&gathered);
            }
        }
    }

    /// A little-endian bit writer over one 128-bit block.
    #[derive(Default)]
    struct Bits(u128, u32);
    impl Bits {
        fn push(&mut self, value: u64, count: u32) {
            self.0 |= u128::from(value) << self.1;
            self.1 += count;
        }
    }

    /// The full chain of a `size` square image in `format`, each level encoded from the
    /// box-filtered level above (normals renormalized when `format` is BC5).
    pub fn chain(role: TextureRole, format: TextureFormat, size: u32, top: Vec<u8>) -> Texture {
        let mut levels = vec![(size, top)];
        while levels.last().unwrap().0 > 1 {
            let (side, texels) = levels.last().unwrap();
            let half = side / 2;
            let mut next = Vec::with_capacity((half * half * 4) as usize);
            for y in 0..half {
                for x in 0..half {
                    let tap = |dx: u32, dy: u32| {
                        let at = (((2 * y + dy) * side + 2 * x + dx) * 4) as usize;
                        &texels[at..at + 4]
                    };
                    let taps = [tap(0, 0), tap(1, 0), tap(0, 1), tap(1, 1)];
                    let mean = |c: usize| taps.iter().map(|t| f32::from(t[c])).sum::<f32>() / 4.0;
                    let mut texel = [mean(0), mean(1), mean(2), mean(3)];
                    if role.is_normal() {
                        let n = [0, 1, 2].map(|c| texel[c] / 255.0 * 2.0 - 1.0);
                        let length = n.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
                        for (value, n) in texel.iter_mut().zip(n) {
                            *value = (n / length * 0.5 + 0.5) * 255.0;
                        }
                    }
                    next.extend(texel.map(|v| v.round() as u8));
                }
            }
            levels.push((half, next));
        }
        Texture {
            role,
            format,
            mips: levels
                .into_iter()
                .map(|(side, texels)| Mip {
                    width: side,
                    height: side,
                    rgba8: match format {
                        TextureFormat::Bc7 => encode_bc7(side, side, &texels),
                        TextureFormat::Bc5 => encode_bc5(side, side, &texels),
                        TextureFormat::Uncompressed => texels,
                    },
                })
                .collect(),
        }
    }

    /// A smooth colour image: sums of low-frequency waves per channel, opaque.
    pub fn colour(size: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
                let wave = |a: f32, b: f32, phase: f32| {
                    0.5 + 0.4 * (std::f32::consts::TAU * (a * u + b * v) + phase).sin()
                };
                out.extend(
                    [
                        wave(1.0, 0.0, 0.0),
                        wave(0.0, 1.0, 1.0),
                        wave(1.0, 1.0, 2.0),
                        1.0,
                    ]
                    .map(|c| (c * 255.0).round() as u8),
                );
            }
        }
        out
    }

    /// A smooth tangent-space normal map of gentle bumps, as RGBA8 with Z in B.
    pub fn normals(size: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let (u, v) = (x as f32 / size as f32, y as f32 / size as f32);
                let nx = 0.35 * (std::f32::consts::TAU * 2.0 * u).sin();
                let ny = 0.35 * (std::f32::consts::TAU * 2.0 * v).cos();
                let nz = (1.0 - nx * nx - ny * ny).sqrt();
                out.extend([nx, ny, nz, 1.0].map(|c| ((c * 0.5 + 0.5) * 255.0).round() as u8));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(role: TextureRole, format: TextureFormat, mips: Vec<Mip>) -> Texture {
        Texture { role, format, mips }
    }

    #[test]
    fn levels_copy_as_whole_blocks_and_small_levels_as_one() {
        let data = fixture::chain(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            8,
            fixture::colour(8),
        );
        assert_eq!(
            data.mips.iter().map(|m| m.rgba8.len()).collect::<Vec<_>>(),
            [64, 16, 16, 16]
        );
        let (layout, extent) = level_copy(&data, &data.mips[0], Storage::Source);
        assert_eq!(
            (layout.bytes_per_row, layout.rows_per_image),
            (Some(32), Some(2))
        );
        assert_eq!((extent.width, extent.height), (8, 8));
        // The 2x2 and 1x1 levels are one block each, copied at the block's extent.
        for mip in &data.mips[2..] {
            let (layout, extent) = level_copy(&data, mip, Storage::Source);
            assert_eq!(
                (layout.bytes_per_row, layout.rows_per_image),
                (Some(16), Some(1))
            );
            assert_eq!((extent.width, extent.height), (4, 4));
            assert_eq!(level_bytes(mip, Storage::Source), 16);
        }
        // Decoded, the same levels are RGBA8 rows of their own extent.
        let (layout, extent) = level_copy(&data, &data.mips[2], Storage::Decoded);
        assert_eq!(
            (layout.bytes_per_row, layout.rows_per_image),
            (Some(8), Some(2))
        );
        assert_eq!(
            (extent.width, extent.height, extent.depth_or_array_layers),
            (2, 2, 1)
        );
        assert_eq!(level_bytes(&data.mips[2], Storage::Decoded), 16);
        assert_eq!(level_bytes(&data.mips[0], Storage::Decoded), 256);
    }

    #[test]
    fn blocks_upload_only_with_the_feature_and_whole_block_extents() {
        let bc = wgpu::Features::TEXTURE_COMPRESSION_BC;
        let mip = |width, height| Mip {
            width,
            height,
            rgba8: vec![0; 16],
        };
        let none = wgpu::Features::empty();
        let whole = texture(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            vec![mip(4, 4)],
        );
        assert_eq!(storage(bc, &whole), Storage::Source);
        assert_eq!(storage(none, &whole), Storage::Decoded);
        // wgpu refuses a compressed texture whose level 0 is not whole blocks.
        let ragged = texture(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            vec![mip(3, 4)],
        );
        assert_eq!(storage(bc, &ragged), Storage::Decoded);
        assert_eq!(storage(none, &decode_texture(&whole)), Storage::Source);
        let format = |data: &Texture, features| gpu_format(data, storage(features, data));
        assert_eq!(format(&whole, bc), wgpu::TextureFormat::Bc7RgbaUnormSrgb);
        assert_eq!(format(&whole, none), wgpu::TextureFormat::Rgba8UnormSrgb);
        let linear = texture(
            TextureRole::MetallicRoughnessLinear,
            TextureFormat::Bc7,
            vec![mip(4, 4)],
        );
        assert_eq!(format(&linear, bc), wgpu::TextureFormat::Bc7RgbaUnorm);
        assert_eq!(format(&linear, none), wgpu::TextureFormat::Rgba8Unorm);
        let normal = texture(
            TextureRole::NormalLinear,
            TextureFormat::Bc5,
            vec![mip(4, 4)],
        );
        assert_eq!(format(&normal, bc), wgpu::TextureFormat::Bc5RgUnorm);
        assert_eq!(format(&normal, none), wgpu::TextureFormat::Rgba8Unorm);
        // The policy limit bounds what the device holds: 16 bytes of blocks, 64 decoded.
        assert_eq!(admit(&whole, bc, 4096, 16).unwrap(), (Storage::Source, 16));
        assert!(admit(&whole, none, 4096, 16).is_err());
        assert_eq!(
            admit(&whole, none, 4096, 64).unwrap(),
            (Storage::Decoded, 64)
        );
    }

    #[test]
    fn mode_six_and_bc5_fixtures_decode_close_to_their_sources() {
        let size = 128;
        let worst =
            |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        let colour = fixture::colour(size);
        let bc7 = fixture::chain(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            size,
            colour.clone(),
        );
        let decoded = decode_texture(&bc7);
        assert_eq!(decoded.format, TextureFormat::Uncompressed);
        assert_eq!(decoded.mips.len(), 8);
        // One line per block for three independent waves: coarse, but a wrong bit layout
        // would be off by half the range.
        assert!(worst(&decoded.mips[0].rgba8, &colour) <= 16);

        let normals = fixture::normals(size);
        let bc5 = fixture::chain(
            TextureRole::NormalLinear,
            TextureFormat::Bc5,
            size,
            normals.clone(),
        );
        let decoded = decode_texture(&bc5);
        assert!(worst(&decoded.mips[0].rgba8, &normals) <= 3);
        // Z is the unit normal's, alpha opaque.
        for texel in decoded.mips[0].rgba8.chunks_exact(4) {
            let n = [0, 1, 2].map(|c| f32::from(texel[c]) / 255.0 * 2.0 - 1.0);
            assert!((n.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 0.02);
            assert_eq!(texel[3], 255);
        }
    }

    #[test]
    fn edge_blocks_keep_only_texels_inside_the_level() {
        // 6x5: two blocks across and down, the right and bottom ones cut. A grey ramp,
        // which one line per block represents.
        let source: Vec<u8> = (0..6 * 5u32)
            .flat_map(|i| {
                let v = (i % 6 * 20 + i / 6 * 10) as u8;
                [v, v, v, 255]
            })
            .collect();
        let data = texture(
            TextureRole::BaseColorSrgb,
            TextureFormat::Bc7,
            vec![Mip {
                width: 6,
                height: 5,
                rgba8: fixture::encode_bc7(6, 5, &source),
            }],
        );
        assert_eq!(data.level_bytes(6, 5), Some(64));
        let decoded = decode_texture(&data);
        assert_eq!(decoded.mips[0].rgba8.len(), 6 * 5 * 4);
        let worst = decoded.mips[0]
            .rgba8
            .iter()
            .zip(&source)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 6, "worst {worst}");
    }
}
