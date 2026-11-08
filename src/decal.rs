//! Projected box decals. The local unit cube is [-0.5, 0.5] on every axis;
//! local XY supplies UVs and local +Z is the receiving surface's preferred normal.
use crate::{Error, Mat4, Vec3};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum DecalBlend {
    #[default]
    Alpha,
    Multiply,
    Add,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecalTextureKind {
    BaseColorSrgb,
    MetallicRoughness,
    Normal,
}

/// Immutable CPU texture, shared between decals and packed once per decal publication.
/// Mips are generated in linear light, with premultiplied alpha for base colour.
#[derive(Clone, Debug)]
pub struct DecalTexture(Arc<TextureData>);
#[derive(Debug)]
struct TextureData {
    kind: DecalTextureKind,
    size: [u32; 2],
    levels: u32,
    texels: Vec<[f32; 4]>,
}
impl DecalTexture {
    pub fn from_rgba8(kind: DecalTextureKind, size: [u32; 2], rgba8: &[u8]) -> Result<Self, Error> {
        let count = u64::from(size[0]) * u64::from(size[1]);
        if size.contains(&0) || count.checked_mul(4) != Some(rgba8.len() as u64) {
            return Err(Error::InvalidInput("decal texture extent/payload mismatch"));
        }
        let mut texels: Vec<_> = rgba8
            .chunks_exact(4)
            .map(|p| {
                let mut v: [f32; 4] = std::array::from_fn(|i| f32::from(p[i]) / 255.);
                if kind == DecalTextureKind::BaseColorSrgb {
                    let alpha = v[3];
                    for c in &mut v[..3] {
                        *c = if *c <= 0.04045 {
                            *c / 12.92
                        } else {
                            ((*c + 0.055) / 1.055).powf(2.4)
                        };
                        *c *= alpha;
                    }
                }
                v
            })
            .collect();
        let (mut width, mut height, mut offset, mut levels) = (size[0], size[1], 0usize, 1);
        while width > 1 || height > 1 {
            let next = [width.div_ceil(2), height.div_ceil(2)];
            let start = texels.len();
            for y in 0..next[1] {
                for x in 0..next[0] {
                    let mut mean = [0.; 4];
                    let mut n = 0.;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let (sx, sy) = (2 * x + dx, 2 * y + dy);
                            if sx < width && sy < height {
                                for (sum, c) in mean
                                    .iter_mut()
                                    .zip(texels[offset + (sy * width + sx) as usize])
                                {
                                    *sum += c;
                                }
                                n += 1.;
                            }
                        }
                    }
                    texels.push(mean.map(|v| v / n));
                }
            }
            offset = start;
            [width, height] = next;
            levels += 1;
        }
        Ok(Self(Arc::new(TextureData {
            kind,
            size,
            levels,
            texels,
        })))
    }
}

#[derive(Clone, Debug, Default)]
pub struct DecalTextures {
    pub base_color: Option<DecalTexture>,
    /// Green is roughness, blue is metallic, matching glTF.
    pub metallic_roughness: Option<DecalTexture>,
    /// Tangent-space XYZ in [0,1], in the decal's projection frame.
    pub normal: Option<DecalTexture>,
}

#[derive(Clone, Debug)]
pub struct Decal {
    pub transform: Mat4,
    pub textures: DecalTextures,
    /// Linear colour and coverage alpha.
    pub base_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub opacity: f32,
    /// Blend strengths for base colour, roughness, metallic and normal, respectively.
    pub channels: [f32; 4],
    pub blend: DecalBlend,
    /// Ascending order; equal orders retain their input order.
    pub sort_order: i32,
    /// Fade from full at this angle to zero at angle_fade_end, in radians.
    pub angle_fade_start: f32,
    pub angle_fade_end: f32,
    /// Fraction of each local half-extent used for the box-edge fade, in [0,1].
    pub edge_fade: f32,
}
impl Default for Decal {
    fn default() -> Self {
        Self {
            transform: Mat4::IDENTITY,
            textures: DecalTextures::default(),
            base_color: [1.; 4],
            roughness: 0.8,
            metallic: 0.,
            opacity: 1.,
            channels: [1., 0., 0., 0.],
            blend: DecalBlend::Alpha,
            sort_order: 0,
            angle_fade_start: 60f32.to_radians(),
            angle_fade_end: 85f32.to_radians(),
            edge_fade: 0.08,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DecalSettings {
    /// World-space grid dimensions; lists never truncate overlapping decals.
    pub grid_resolution: [u32; 3],
}
impl Default for DecalSettings {
    fn default() -> Self {
        Self {
            grid_resolution: [16; 3],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DecalStats {
    pub decals: u32,
    pub grid_cells: u32,
    pub references: u32,
    pub max_cell_decals: u32,
    pub bytes: u64,
}

pub(crate) const HEADER: usize = 16;
pub(crate) const RECORD: usize = 48;

/// One read-only table contains the grid, sort-ordered records and mipmapped texels.
/// This needs only one extra storage binding, including on devices without RT.
pub(crate) fn pack(
    decals: &[Decal],
    settings: DecalSettings,
    max_bytes: u64,
) -> Result<(Vec<u32>, DecalStats), Error> {
    let invalid = || Error::InvalidInput("invalid decal transform, factors, texture role or grid");
    if decals.is_empty() {
        return Ok((
            vec![0; HEADER],
            DecalStats {
                bytes: (HEADER * 4) as u64,
                ..Default::default()
            },
        ));
    }
    let dims = settings.grid_resolution;
    let cells = dims
        .into_iter()
        .try_fold(1u64, |n, d| {
            if d == 0 {
                None
            } else {
                n.checked_mul(u64::from(d))
            }
        })
        .ok_or_else(invalid)?;
    let base_words = (HEADER as u64)
        .checked_add(
            (decals.len() as u64)
                .checked_mul(RECORD as u64)
                .ok_or_else(invalid)?,
        )
        .and_then(|n| n.checked_add(cells.checked_mul(2)?))
        .ok_or_else(invalid)?;
    if base_words > max_bytes / 4 || base_words > u32::MAX as u64 || cells > u32::MAX as u64 {
        return Err(invalid());
    }
    let mut sorted: Vec<_> = decals.iter().collect();
    sorted.sort_by_key(|d| d.sort_order);
    let mut bounds = Vec::with_capacity(decals.len());
    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for d in &sorted {
        let affine = d.transform.w_axis.w == 1.
            && d.transform.x_axis.w == 0.
            && d.transform.y_axis.w == 0.
            && d.transform.z_axis.w == 0.;
        let unit = d.base_color.iter().chain(d.channels.iter()).chain([
            &d.roughness,
            &d.metallic,
            &d.opacity,
            &d.edge_fade,
        ]);
        if !affine
            || !d.transform.is_finite()
            || !d.transform.inverse().is_finite()
            || !unit
                .into_iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            || !d.angle_fade_start.is_finite()
            || !d.angle_fade_end.is_finite()
            || !(0. ..=std::f32::consts::PI).contains(&d.angle_fade_start)
            || d.angle_fade_start >= d.angle_fade_end
            || d.angle_fade_end > std::f32::consts::PI
        {
            return Err(invalid());
        }
        let center = d.transform.w_axis.truncate();
        let extent = (d.transform.x_axis.truncate().abs()
            + d.transform.y_axis.truncate().abs()
            + d.transform.z_axis.truncate().abs())
            * 0.5;
        let b = (center - extent, center + extent);
        if !b.0.is_finite() || !b.1.is_finite() {
            return Err(invalid());
        }
        low = low.min(b.0);
        high = high.max(b.1);
        bounds.push(b);
    }
    let span = high - low;
    if !span.is_finite() || span.min_element() <= 0. {
        return Err(invalid());
    }
    let scale = Vec3::from_array(dims.map(|d| d as f32)) / span;
    if !scale.is_finite() {
        return Err(invalid());
    }
    let grid = HEADER + RECORD * decals.len();
    let mut words = vec![0u32; base_words as usize];
    while !words.len().is_multiple_of(4) {
        words.push(0);
    }
    words[0] = decals.len() as u32;
    words[1] = grid as u32;
    words[2] = HEADER as u32;
    words[4..7].copy_from_slice(&low.to_array().map(f32::to_bits));
    words[8..11].copy_from_slice(&scale.to_array().map(f32::to_bits));
    words[12..15].copy_from_slice(&dims);
    let mut textures = HashMap::new();
    for (i, d) in sorted.iter().enumerate() {
        let mut texture_offsets = [0u32; 3];
        for (k, (texture, kind)) in [
            (&d.textures.base_color, DecalTextureKind::BaseColorSrgb),
            (
                &d.textures.metallic_roughness,
                DecalTextureKind::MetallicRoughness,
            ),
            (&d.textures.normal, DecalTextureKind::Normal),
        ]
        .into_iter()
        .enumerate()
        {
            if let Some(texture) = texture {
                if texture.0.kind != kind {
                    return Err(invalid());
                }
                let key = Arc::as_ptr(&texture.0);
                texture_offsets[k] = if let Some(&offset) = textures.get(&key) {
                    offset
                } else {
                    let t = &texture.0;
                    let needed = words.len() as u64 + 4 + t.texels.len() as u64 * 4;
                    if needed > max_bytes / 4 || needed > u32::MAX as u64 {
                        return Err(Error::InvalidInput(
                            "decal table exceeds device storage binding budget",
                        ));
                    }
                    let offset = words.len() as u32;
                    words.extend([t.size[0], t.size[1], t.levels, 0]);
                    words.extend(t.texels.iter().flatten().map(|v| v.to_bits()));
                    textures.insert(key, offset);
                    offset
                };
            }
        }
        let start = HEADER + i * RECORD;
        let inverse = d.transform.inverse();
        let normal = inverse.transpose().z_axis.truncate().normalize();
        let tangent = d.transform.x_axis.truncate().normalize();
        let bitangent = normal.cross(tangent).normalize() * d.transform.determinant().signum();
        let values: Vec<_> = inverse
            .to_cols_array()
            .into_iter()
            .chain(tangent.extend(0.).to_array())
            .chain(bitangent.extend(0.).to_array())
            .chain(normal.extend(0.).to_array())
            .chain(d.base_color)
            .chain([d.roughness, d.metallic, d.opacity, d.edge_fade])
            .chain([d.angle_fade_end.cos(), d.angle_fade_start.cos(), 0., 0.])
            .chain(d.channels)
            .collect();
        words[start..start + 44]
            .copy_from_slice(&values.into_iter().map(f32::to_bits).collect::<Vec<_>>());
        words[start + 44..start + 47].copy_from_slice(&texture_offsets);
        words[start + 47] = d.blend as u32;
    }
    let mut lists = vec![Vec::new(); cells as usize];
    let cell = |point: Vec3| {
        ((point - low) * scale)
            .floor()
            .max(Vec3::ZERO)
            .to_array()
            .map(|v| v as u32)
    };
    let mut references = 0u64;
    for (i, (min, max)) in bounds.into_iter().enumerate() {
        let a = cell(min);
        let b = cell(max);
        for z in a[2].min(dims[2] - 1)..=b[2].min(dims[2] - 1) {
            for y in a[1].min(dims[1] - 1)..=b[1].min(dims[1] - 1) {
                for x in a[0].min(dims[0] - 1)..=b[0].min(dims[0] - 1) {
                    references += 1;
                    if words.len() as u64 + references > max_bytes / 4
                        || words.len() as u64 + references > u32::MAX as u64
                    {
                        return Err(Error::InvalidInput(
                            "decal grid references exceed device storage binding budget",
                        ));
                    }
                    lists[(x + dims[0] * (y + dims[1] * z)) as usize].push(i as u32);
                }
            }
        }
    }
    let mut max_cell_decals = 0;
    for (cell, list) in lists.into_iter().enumerate() {
        words[grid + cell * 2] = words.len() as u32;
        words[grid + cell * 2 + 1] = list.len() as u32;
        max_cell_decals = max_cell_decals.max(list.len() as u32);
        words.extend(list);
    }
    while !words.len().is_multiple_of(4) {
        words.push(0);
    }
    if words.len() as u64 * 4 > max_bytes {
        return Err(invalid());
    }
    let stats = DecalStats {
        decals: decals.len() as u32,
        grid_cells: cells as u32,
        references: references as u32,
        max_cell_decals,
        bytes: words.len() as u64 * 4,
    };
    Ok((words, stats))
}

#[cfg(test)]
#[path = "decal_tests.rs"]
mod tests;
