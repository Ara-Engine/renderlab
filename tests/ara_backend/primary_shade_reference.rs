//! Independent material interpolation and original tile/list addressing fixtures.
use ara_gpu::sampler::{AddressMode, Filter, SamplerDesc};
use glam::{DMat4, DVec2, DVec3};
use half::f16;
use renderlab::{Mat4, Vec3, backend_ara::shader::InputKey};
use std::collections::BTreeMap;

/// Authored test material, composed with the unchanged traced program API/entries.
/// It uses three sampled arrays and the owner's Parameters without lighting hooks.
pub const PROGRAM: &str = r#"
struct Parameters { gain:vec4<f32>, bias:vec4<f32> };
const DEFAULT_PARAMETERS:Parameters=Parameters(vec4<f32>(1.0),vec4<f32>(0.0));
struct Surface { rgb:vec3<f32> };
fn surface(input:SurfaceInput)->Surface {
    let base=sample_base_color(input).rgb;
    let mr=sample_metallic_roughness(input).rgb;
    let normal=sample_normal_layer_grad(input,input.uv,input.uv_dx,input.uv_dy,1).rgb;
    return Surface((base+0.5*mr+0.25*normal)*input.material.base_color.rgb*input.color.rgb);
}
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
    return s.rgb*params.gain.rgb+params.bias.rgb+abs(input.world)*params.gain.w;
}
"#;

/// A nearest-sampled, single-mip 2x2 texture; normal maps have two layers.
pub struct Sampled {
    /// Layer-major RGBA16Float texels, already exactly representable as f16.
    pub pixels: Vec<[f32; 4]>,
    /// Number of 2x2 layers; base/MR are ordinary 2D views, normals are 2D-array.
    pub layers: u32,
    /// Per-owner regular sampler; every filter is nearest.
    pub sampler: SamplerDesc,
}

/// One actual owner shared by every logical material/geometry array.
pub struct Batch {
    /// Logical index 3 or 37, independent of physical owner order.
    pub index: u32,
    /// Original three-vec4 vertex packing: position/normal, normal/UV, colour.
    pub vertices: Vec<[u32; 4]>,
    /// Original triangle indices, deliberately reordered.
    pub indices: Vec<u32>,
    /// Four floats per source vertex; the second placement takes the tangent path.
    pub tangents: Vec<[u32; 4]>,
    /// Original 48-byte Material, with both sides enabled.
    pub material: [u32; 12],
    /// Original 32-byte Parameters: gain at byte0, bias at byte16.
    pub parameters: [u32; 8],
    /// Logical (2,11)/(2,12) image/sampler pair.
    pub base: Sampled,
    /// Logical (2,13)/(2,14) image/sampler pair.
    pub mr: Sampled,
    /// Logical (2,15)/(2,16), sampled explicitly at layer1.
    pub normal: Sampled,
}

/// One original per-program list record, followed by its dispatch shape.
pub struct List {
    /// 272-byte RlList: slot/base/capacity/flags, then64 packed owner program IDs.
    pub record: [u32; 68],
    /// Workgroup counts, including one surplus group to test the length guard.
    pub dispatch: [u32; 3],
}

/// Raw original-layout inputs and snapshots for monolithic versus split shading.
pub struct Fixture {
    /// RGBA32Uint hit and RGBA16Float HDR dimensions, including partial clear blocks.
    pub size: [u32; 2],
    /// R32Uint trace coverage extent, ceil(size/[8,4]).
    pub coverage_size: [u32; 2],
    /// Original hit records, with invalid primitive/bary words on covered misses.
    pub hit: Vec<[u32; 4]>,
    /// Includes an uncovered interior tile filled with stale nonzero hit records.
    pub coverage: Vec<u32>,
    /// Storage primary_region; both shading paths use this same rectangle.
    pub region: [u32; 4],
    /// Original scalar buffer inputs, including disabled decal/light/atmosphere state.
    pub buffers: BTreeMap<InputKey, Vec<u32>>,
    /// Two actual owners, deliberately stored as37 then3.
    pub batches: Vec<Batch>,
    /// (0,39) two list segments with nonzero bases, padding and allocation guards.
    pub list_words: Vec<u32>,
    /// Program0 writes its hits/misses/clear blocks; program1 fills its own hits.
    pub lists: Vec<List>,
    /// Original monolithic dispatch: region rows followed by clear-block rows.
    pub tiles_dispatch: [u32; 3],
    /// Exactly representable HDR sentinel preserved by list skips.
    pub initial: Vec<[f32; 4]>,
    /// Full output of primary_shade_tiles.
    pub tiles_expected: Vec<[f32; 4]>,
    /// Cumulative output after each primary_shade_list dispatch.
    pub list_expected: Vec<Vec<[f32; 4]>>,
}

fn key(group: u32, binding: u32) -> InputKey {
    InputKey { group, binding }
}
fn float(word: u32) -> f64 {
    f64::from(f32::from_bits(word))
}
fn quantize(pixel: [f32; 4]) -> [f32; 4] {
    pixel.map(|v| f16::from_f32(v).to_f32())
}

fn sample(texture: &Sampled, uv: DVec2, layer: usize) -> DVec3 {
    assert_eq!(texture.pixels.len(), texture.layers as usize * 4);
    let address = |coordinate: f64, mode| {
        let index = (coordinate * 2.).floor() as i32;
        match mode {
            AddressMode::ClampToEdge => index.clamp(0, 1) as usize,
            AddressMode::Repeat => index.rem_euclid(2) as usize,
            AddressMode::MirrorRepeat => {
                let k = index.rem_euclid(4);
                (if k < 2 { k } else { 3 - k }) as usize
            }
        }
    };
    let x = address(uv.x, texture.sampler.address[0]);
    let y = address(uv.y, texture.sampler.address[1]);
    DVec3::from_array(std::array::from_fn(|c| {
        f64::from(texture.pixels[layer * 4 + y * 2 + x][c])
    }))
}

/// Independently interpolate colour/UV, select addressed texels, then evaluate the
/// authored affine material in f64 and quantize its final RGBA16Float store.
fn material(f: &Fixture, hit: [u32; 4]) -> [f32; 4] {
    let placements = &f.buffers[&key(1, 5)];
    let logical = placements[(hit[0] - 1) as usize] & 0x1fff_ffff;
    let batch = f
        .batches
        .iter()
        .find(|b| b.index == logical)
        .expect("actual fixture owner");
    let u = float(hit[2]);
    let v = float(hit[3]);
    let mut uv = DVec2::ZERO;
    let mut colour = DVec3::ZERO;
    let mut position = DVec3::ZERO;
    for (corner, weight) in [1. - u - v, u, v].into_iter().enumerate() {
        let vertex = batch.indices[hit[1] as usize * 3 + corner] as usize * 3;
        let a = batch.vertices[vertex];
        position += DVec3::new(float(a[0]), float(a[1]), float(a[2])) * weight;
        let b = batch.vertices[vertex + 1];
        uv += DVec2::new(float(b[2]), float(b[3])) * weight;
        let c = batch.vertices[vertex + 2];
        colour += DVec3::new(float(c[0]), float(c[1]), float(c[2])) * weight;
    }
    let base = sample(&batch.base, uv, 0);
    let mr = sample(&batch.mr, uv, 0);
    let normal = sample(&batch.normal, uv, 1);
    let value = (base + mr * 0.5 + normal * 0.25) * colour;
    let offset = (hit[0] - 1) as usize * 16;
    let model = DMat4::from_cols_array(&std::array::from_fn(|i| {
        float(f.buffers[&key(1, 0)][offset + i])
    }));
    let world = model.transform_point3(position);
    let rgb = std::array::from_fn::<_, 3, _>(|c| {
        ((value[c] * float(batch.material[c]) * float(batch.parameters[c])
            + float(batch.parameters[4 + c])
            + world[c].abs() * float(batch.parameters[3]))
        .clamp(0., 65504.)) as f32
    });
    quantize([rgb[0], rgb[1], rgb[2], 1.])
}

fn inside(f: &Fixture, x: u32, y: u32) -> bool {
    x >= f.region[0] && y >= f.region[1] && x < f.region[2] && y < f.region[3]
}
fn clear(f: &Fixture) -> [f32; 4] {
    quantize(std::array::from_fn(|c| {
        f32::from_bits(f.buffers[&key(0, 10)][16 + c])
    }))
}
fn covered_hit(f: &Fixture, x: u32, y: u32) -> bool {
    f.coverage[((y / 4) * f.coverage_size[0] + x / 8) as usize] != 0
        && f.hit[(y * f.size[0] + x) as usize][0] != 0
}

/// Classify pixels directly; coverage, misses and outside-region clear blocks all
/// resolve to the frame's clear colour when the original full dispatch is used.
pub fn render_tiles(f: &Fixture) -> Vec<[f32; 4]> {
    (0..f.size[0] * f.size[1])
        .map(|at| {
            let x = at % f.size[0];
            let y = at / f.size[0];
            if inside(f, x, y) && covered_hit(f, x, y) {
                material(f, f.hit[at as usize])
            } else {
                clear(f)
            }
        })
        .collect()
}

/// Apply the original packed list addressing and ownership mask to an existing
/// image. Pixel values come from the independent interpolation oracle above.
pub fn render_list(f: &Fixture, list: &List, initial: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let mut image = initial.to_vec();
    let base = list.record[1] as usize;
    let length = f.list_words[base] as usize;
    for &entry in &f.list_words[base + 1..base + 1 + length] {
        let tile = [entry & 0x7fff, (entry >> 15) & 0x7fff];
        for y in 0..f.size[1] {
            for x in 0..f.size[0] {
                let at = (y * f.size[0] + x) as usize;
                if entry & 0x4000_0000 != 0 {
                    if x / 16 == tile[0] && y / 16 == tile[1] && !inside(f, x, y) {
                        image[at] = clear(f);
                    }
                } else {
                    let origin = [f.region[0] + tile[0] * 4, f.region[1] + tile[1] * 4];
                    if x < origin[0] || y < origin[1] || x >= origin[0] + 4 || y >= origin[1] + 4 {
                        continue;
                    }
                    if !covered_hit(f, x, y) {
                        if entry & 0x8000_0000 != 0 {
                            image[at] = clear(f);
                        }
                    } else {
                        let owner =
                            f.buffers[&key(1, 5)][(f.hit[at][0] - 1) as usize] & 0x1fff_ffff;
                        if list.record[4 + owner as usize] == list.record[0] {
                            image[at] = material(f, f.hit[at]);
                        }
                    }
                }
            }
        }
    }
    image
}

/// Half-store tolerance only: source factors/texels are dyadic and nearest sampling
/// avoids filter-fraction uncertainty. Skipped list texels must match the sentinel.
pub fn assert_matches(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (pixel, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for c in 0..4 {
            if expected[0] < 0. {
                assert_eq!(
                    actual[c].to_bits(),
                    expected[c].to_bits(),
                    "preserved pixel {pixel}"
                );
            } else {
                assert!(
                    actual[c].is_finite() && (actual[c] - expected[c]).abs() <= 0.001,
                    "shade pixel {pixel} channel {c}: actual {}, expected {}",
                    actual[c],
                    expected[c]
                );
            }
        }
    }
}

fn texture(owner: u32, role: u32, layers: u32, address: AddressMode) -> Sampled {
    let pixels = (0..layers * 4)
        .map(|texel| {
            std::array::from_fn(|c| {
                if layers == 2 && texel < 4 {
                    0.
                } else {
                    (owner * 16 + role * 8 + texel * 4 + c as u32 + 1) as f32 / 64.
                }
            })
        })
        .collect();
    Sampled {
        pixels,
        layers,
        sampler: SamplerDesc {
            min: Filter::Nearest,
            mag: Filter::Nearest,
            mip: Filter::Nearest,
            address: [address; 3],
            ..Default::default()
        },
    }
}

fn batch(index: u32, owner: u32) -> Batch {
    let positions = [[-1., -1., 0.5], [1., -1., 0.5], [-1., 1., 0.5]];
    let uvs = [[-0.75, -0.25], [0.25, 0.25], [1.25, 0.75]];
    let colours = [
        [0.25, 0.5, 0.75, 1.],
        [0.5, 0.75, 0.25, 1.],
        [0.75, 0.25, 0.5, 1.],
    ];
    let vertices = positions
        .into_iter()
        .enumerate()
        .flat_map(|(i, p)| {
            [
                [p[0], p[1], p[2], 0.].map(f32::to_bits),
                [0., 1., uvs[i][0], uvs[i][1]].map(f32::to_bits),
                colours[(i + owner as usize) % 3].map(f32::to_bits),
            ]
        })
        .collect();
    let mut material = [0; 12];
    material[..4].copy_from_slice(
        &if owner == 0 {
            [0.5f32, 0.75, 1., 1.]
        } else {
            [1., 0.5, 0.75, 1.]
        }
        .map(f32::to_bits),
    );
    material[6] = 1; // double_sided at byte24.
    material[8] = 1f32.to_bits();
    let parameters = if owner == 0 {
        [0.5f32, 0.75, 1., 0.0625, 0.0625, 0.125, 0.25, 0.]
    } else {
        [1., 0.5, 0.75, 0.125, 0.125, 0.0625, 0.1875, 0.]
    }
    .map(f32::to_bits);
    Batch {
        index,
        vertices,
        indices: vec![2, 0, 1],
        tangents: vec![[1f32, 0., 0., 1.].map(f32::to_bits); 3],
        material,
        parameters,
        base: texture(
            owner,
            0,
            1,
            if owner == 0 {
                AddressMode::ClampToEdge
            } else {
                AddressMode::Repeat
            },
        ),
        mr: texture(
            owner,
            1,
            1,
            if owner == 0 {
                AddressMode::MirrorRepeat
            } else {
                AddressMode::ClampToEdge
            },
        ),
        normal: texture(
            owner,
            2,
            2,
            if owner == 0 {
                AddressMode::Repeat
            } else {
                AddressMode::MirrorRepeat
            },
        ),
    }
}

/// Two owners at3/37, mixed hit tiles, stale uncovered pixels and partial 16x16
/// clear blocks. The two lists partition hit work and reproduce the full dispatch.
pub fn fixture() -> Fixture {
    let size = [19, 9];
    let region = [0, 0, 16, 8];
    let coverage_size = [3, 3];
    let mut buffers = BTreeMap::new();
    let identity = Mat4::IDENTITY.to_cols_array().map(f32::to_bits);
    let mut camera = identity.to_vec();
    camera.extend([0; 4]);
    buffers.insert(key(0, 0), camera);
    let mut primary = vec![0; 36];
    primary[..16].copy_from_slice(&identity);
    primary[16..20].copy_from_slice(&[0.0625f32, 0.125, 0.25, 1.].map(f32::to_bits));
    primary[24..28].copy_from_slice(&region);
    buffers.insert(key(0, 10), primary);
    buffers.insert(key(0, 22), region.to_vec());
    let model = Mat4::from_translation(Vec3::new(0.25, -0.125, 0.))
        * Mat4::from_scale(Vec3::new(-1., 1., 1.));
    buffers.insert(
        key(1, 0),
        [Mat4::IDENTITY, model]
            .into_iter()
            .flat_map(|m| m.to_cols_array().map(f32::to_bits))
            .collect(),
    );
    buffers.insert(
        key(1, 1),
        [Mat4::IDENTITY, model]
            .into_iter()
            .flat_map(|m| {
                let mut n = m.inverse().transpose().to_cols_array();
                n[12..16].copy_from_slice(&[m.determinant().signum(), 0., 0., 0.]);
                n.map(f32::to_bits)
            })
            .collect(),
    );
    buffers.insert(key(1, 5), vec![3, 0x8000_0025]);
    let mut lighting = vec![0; 12];
    lighting[2] = 2f32.to_bits();
    lighting[7] = 1f32.to_bits();
    lighting[10] = 1f32.to_bits();
    buffers.insert(key(3, 0), lighting);
    buffers.insert(key(3, 21), vec![0; 16]);
    let mut light_frame = vec![0; 12_828];
    light_frame[..16].copy_from_slice(&identity); // info.y at word21 remains0.
    buffers.insert(key(3, 23), light_frame);
    let mut atmosphere = vec![0; 108];
    atmosphere[25] = 1f32.to_bits(); // Direction +Y; sun.w flags remain0.
    buffers.insert(key(3, 50), atmosphere);
    let hit = (0..size[0] * size[1])
        .map(|at| {
            let x = at % size[0];
            let y = at / size[0];
            if (x + y) % 5 == 0 {
                [0, u32::MAX, 0xdead_beef, 0xface_1234]
            } else {
                [
                    1 + (x + y) % 2,
                    0,
                    ((1 + x % 5) as f32 / 8.).to_bits(),
                    ((1 + y % 2) as f32 / 8.).to_bits(),
                ]
            }
        })
        .collect();
    let mut list_words = vec![0xdead_ffff; 48];
    let mut lists = Vec::new();
    for slot in 0..2 {
        let base = 4 + slot * 20;
        let mut entries: Vec<_> = (0..2)
            .flat_map(|y| {
                (0..4).map(move |x| x | y << 15 | if slot == 0 { 0x8000_0000 } else { 0 })
            })
            .collect();
        if slot == 0 {
            entries.extend([0x4000_0000, 0x4000_0001]);
        }
        list_words[base] = entries.len() as u32;
        list_words[base + 1..base + 1 + entries.len()].copy_from_slice(&entries);
        let mut record = [0; 68];
        record[..4].copy_from_slice(&[slot as u32, base as u32, 16, 0]);
        record[4 + 37] = 1;
        lists.push(List {
            record,
            dispatch: [entries.len() as u32 + 1, 1, 1],
        });
    }
    let initial = vec![[-1.25, -2., -3., 0.5]; (size[0] * size[1]) as usize];
    let mut f = Fixture {
        size,
        coverage_size,
        hit,
        coverage: vec![1, 1, 0, 0, 1, 0, 0, 0, 0],
        region,
        buffers,
        batches: vec![batch(37, 1), batch(3, 0)],
        list_words,
        lists,
        tiles_dispatch: [2, 3, 1],
        initial,
        tiles_expected: Vec::new(),
        list_expected: Vec::new(),
    };
    f.tiles_expected = render_tiles(&f);
    let mut image = f.initial.clone();
    for list in &f.lists {
        image = render_list(&f, list, &image);
        f.list_expected.push(image.clone());
    }
    assert_eq!(f.list_expected[1], f.tiles_expected);
    assert!(f.list_expected[0].iter().any(|p| p[0] < 0.));
    assert!(f.list_expected[1].iter().all(|p| p[0] >= 0.));
    f
}
