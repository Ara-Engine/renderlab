//! Scalar integer-boundary replay of original programmable TP glass composition.
use super::reference::{Image, half};
use glam::{Mat4, Vec3};
use half::f16;
use renderlab::Vertex;

/// Original glass target sample count; seed all samples before each pane draw.
pub const SAMPLES: u32 = 4;

/// Authored program composed by the original glass_source with parameters enabled.
/// No transmittance, reflection, lighting or indirect-light hooks are declared.
pub const PROGRAM: &str = r#"
struct Parameters { gain:vec4<f32>, bias:vec4<f32> };
struct Surface { rgb:vec3<f32> };
fn surface(input:SurfaceInput) -> Surface {
    return Surface(sample_base_color(input).rgb*input.material.base_color.rgb*input.color.rgb);
}
fn shade(s:Surface, input:SurfaceInput) -> vec3<f32> {
    return s.rgb*params.gain.rgb+params.bias.rgb;
}
"#;

/// Fixed controls for the original four-sample seeded glass pass.
#[derive(Clone, Copy, Debug)]
pub enum Case {
    /// Far pane, then a reflected near pane; both retain their source-front side.
    Ordered,
    /// Reverse the reflected pane's source triangles and reject them as single sided.
    BackFacingMirroredPane,
    /// Both packed opacity halves are zero; nonzero IOR halves preserve transparency ABI.
    ZeroOpacity,
}

/// One indexed quad draw, with original source-layout material and parameter bytes.
pub struct Pane {
    /// Integer pixel rectangle [left, top, right, bottom), checked against geometry.
    pub bounds: [u32; 4],
    /// Four 48-byte vertices; color and normal are constant across each pane.
    pub vertices: Vec<Vertex>,
    /// Two triangle-list primitives; the source order controls fragment sidedness.
    pub indices: Vec<u32>,
    /// Instance-stream value addressing Fixture models and normals.
    pub slot: u32,
    /// Material48B, including packed half opacity/IOR in word eleven.
    pub material: [u32; 12],
    /// Parameters32B: gain vec4 then bias vec4; fourth components are padding values.
    pub parameters: [u32; 8],
    /// TpDraw16B; zero thickness makes behind sampling the current pixel.
    pub draw: [u32; 4],
    /// One stored RGBA16Float texel, sampled through a nearest clamp sampler.
    pub base: Image,
}

/// Original binding data, seeded color/depth inputs and independently composited pixels.
pub struct Fixture {
    /// Eight by six pixels, with four identical geometric coverage samples per pixel.
    pub size: [u32; 2],
    /// Original Camera80B: identity projection and zero view mode.
    pub camera: [u32; 20],
    /// Original TpFrame336B; eye(0,0,2), forward -Z and exposure one.
    pub frame: [u32; 84],
    /// LightingUniform48B, zero light count and environment-map disabled.
    pub lighting: [u32; 12],
    /// AtmosphereUniform432B, physically valid defaults with all flag bits zero.
    pub atmosphere: [u32; 108],
    /// Two model records, identity and reflection in X, in source column-major order.
    pub models: Vec<[u32; 16]>,
    /// Two inverse-transpose records, with determinant sign in info.x.
    pub normals: Vec<[u32; 16]>,
    /// Far-to-near ordered draws; each draw samples the preceding resolved image.
    pub panes: Vec<Pane>,
    /// Nonuniform HDR color, already stored through RGBA16Float before MSAA seeding.
    pub background: Image,
    /// R32Float linear opaque depth; 1.4 rejects both panes, 1.6 admits only near.
    pub opaque_depth: Vec<f32>,
    /// One valid array layer, containing one (0.5,0.5,1,1) normal-map texel.
    pub normal_map: Image,
    /// Initialized one-texel metallic/roughness binding, even if source compaction drops it.
    pub metallic_roughness: Image,
    /// Expected half-rounded image after each pane draw and resolve.
    pub expected: Vec<Image>,
}

impl Fixture {
    /// Compare each draw snapshot, preserving untouched pixels exactly.
    ///
    /// Changed pixels permit two half ULPs for shader FMA and native sampling/resolve
    /// rounding. The CPU rounds after every draw; it never blends all panes in f32 first.
    pub fn assert_matches(&self, actual_images: &[Image]) {
        assert_eq!(actual_images.len(), self.expected.len());
        for (draw, (actual, expected)) in actual_images.iter().zip(&self.expected).enumerate() {
            assert_eq!(actual.size, self.size);
            assert_eq!(actual.pixels.len(), expected.pixels.len());
            let previous_expected = if draw == 0 {
                &self.background
            } else {
                &self.expected[draw - 1]
            };
            let previous_actual = if draw == 0 {
                &self.background
            } else {
                &actual_images[draw - 1]
            };
            for (pixel, (a, e)) in actual.pixels.iter().zip(&expected.pixels).enumerate() {
                if e.map(f32::to_bits) == previous_expected.pixels[pixel].map(f32::to_bits) {
                    assert_eq!(
                        a.map(f32::to_bits),
                        previous_actual.pixels[pixel].map(f32::to_bits),
                        "preserved draw {draw}, pixel {pixel}"
                    );
                } else {
                    for channel in 0..3 {
                        half_close(a[channel], e[channel], draw, pixel);
                    }
                    assert_eq!(a[3], 1., "alpha at draw {draw}, pixel {pixel}");
                }
            }
        }
    }
}

fn half_close(actual: f32, expected: f32, draw: usize, pixel: usize) {
    let a = f16::from_f32(actual);
    let e = f16::from_f32(expected);
    assert!(
        actual.is_finite() && actual >= 0. && actual.to_bits() == a.to_f32().to_bits(),
        "non-half output at draw {draw}, pixel {pixel}: {actual}"
    );
    assert!(
        a.to_bits().abs_diff(e.to_bits()) <= 2,
        "half error at draw {draw}, pixel {pixel}: {actual} versus {expected}"
    );
}

fn floats<const N: usize>(words: &[u32; N]) -> [f32; N] {
    words.map(f32::from_bits)
}

fn put(words: &mut [u32], offset: usize, values: &[f32]) {
    for (word, value) in words[offset..].iter_mut().zip(values) {
        *word = value.to_bits();
    }
}

fn finite(value: f32) -> f32 {
    if value >= 0. { value.min(65504.) } else { 0. }
}

/// Check the bounded rectangle contract from actual transformed and indexed geometry.
fn geometry(fixture: &Fixture, pane: &Pane) -> (bool, f32) {
    assert_eq!(pane.vertices.len(), 4);
    assert_eq!(pane.indices.len(), 6);
    assert_eq!(
        pane.draw, [0; 4],
        "refraction thickness is outside this fixture"
    );
    let model = Mat4::from_cols_array(&floats(&fixture.models[pane.slot as usize]));
    let points: Vec<_> = pane
        .vertices
        .iter()
        .map(|v| model.transform_point3(Vec3::from_array(v.position)))
        .collect();
    let mut minimum = [f32::INFINITY; 2];
    let mut maximum = [f32::NEG_INFINITY; 2];
    for (&p, vertex) in points.iter().zip(&pane.vertices) {
        assert_eq!(vertex.color, pane.vertices[0].color);
        assert_eq!(p.z, points[0].z);
        let screen = [
            (p.x * 0.5 + 0.5) * fixture.size[0] as f32,
            (0.5 - p.y * 0.5) * fixture.size[1] as f32,
        ];
        for axis in 0..2 {
            minimum[axis] = minimum[axis].min(screen[axis]);
            maximum[axis] = maximum[axis].max(screen[axis]);
        }
    }
    for (actual, expected) in [minimum[0], minimum[1], maximum[0], maximum[1]]
        .into_iter()
        .zip(pane.bounds)
    {
        assert!(
            (actual - expected as f32).abs() < 2e-5,
            "quad must project to integer boundaries"
        );
    }
    let winding = f32::from_bits(fixture.normals[pane.slot as usize][12]) > 0.;
    let front = |indices: &[u32]| {
        let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| points[indices[i] as usize]);
        let orientation = (b - a).cross(c - a).z;
        assert!(orientation.abs() > 0.01);
        (orientation > 0.) == winding
    };
    let source_front = front(&pane.indices[..3]);
    assert_eq!(source_front, front(&pane.indices[3..]));
    let frame = floats(&fixture.frame);
    let eye = Vec3::from_slice(&frame[48..51]);
    let forward = Vec3::from_slice(&frame[52..55]);
    (source_front, (points[0] - eye).dot(forward))
}

/// Surface/shade and original fallback transmission, evaluated once per covered pixel.
///
/// Each sample sees the same rectangle, constant attributes and constant depth.
/// Rasterizing the shared quad diagonal cannot change the result. Zero thickness,
/// nearest clamp sampling at pixel centres and disabled atmosphere make the sampled
/// behind color exactly the preceding stored pixel. Fractional edge AA, refraction,
/// atmosphere, and declared transmission/reflection hooks are outside this oracle.
pub fn render(fixture: &Fixture) -> Vec<Image> {
    assert_eq!(fixture.background.size, fixture.size);
    assert_eq!(fixture.opaque_depth.len(), fixture.background.pixels.len());
    let identity = Mat4::IDENTITY.to_cols_array().map(f32::to_bits);
    assert_eq!(&fixture.camera[..16], &identity);
    assert_eq!(&fixture.frame[..16], &identity);
    assert_eq!(
        fixture.atmosphere[27], 0,
        "all atmosphere flags must be off"
    );
    let mut behind = Image {
        size: fixture.size,
        pixels: fixture
            .background
            .pixels
            .iter()
            .copied()
            .map(half)
            .collect(),
    };
    let mut snapshots = Vec::new();
    for pane in &fixture.panes {
        let (source_front, depth) = geometry(fixture, pane);
        let material = floats(&pane.material);
        let parameters = floats(&pane.parameters);
        assert_eq!(pane.base.size, [1, 1]);
        assert_eq!(pane.base.pixels.len(), 1);
        let sampled = half(pane.base.pixels[0]);
        let vertex = pane.vertices[0].color;
        let alpha = material[3] * sampled[3] * vertex[3];
        let opacity = if pane.material[11] == 0 {
            1.
        } else {
            f16::from_bits(pane.material[11] as u16).to_f32()
        };
        let coverage = if pane.material[11] == 0 {
            1.
        } else {
            (opacity * alpha).clamp(0., 1.)
        };
        let lit: [f32; 3] = std::array::from_fn(|channel| {
            let rgb = sampled[channel] * material[channel] * vertex[channel];
            finite(rgb * parameters[channel] + parameters[channel + 4])
        });
        let mut output = behind.clone();
        if pane.material[6] != 0 || source_front {
            let [left, top, right, bottom] = pane.bounds;
            for y in top..bottom {
                for x in left..right {
                    let pixel = (y * fixture.size[0] + x) as usize;
                    if depth > fixture.opaque_depth[pixel] {
                        continue;
                    }
                    output.pixels[pixel] = half([
                        finite(
                            finite(lit[0] * coverage)
                                + (1. - coverage) * behind.pixels[pixel][0].max(0.),
                        ),
                        finite(
                            finite(lit[1] * coverage)
                                + (1. - coverage) * behind.pixels[pixel][1].max(0.),
                        ),
                        finite(
                            finite(lit[2] * coverage)
                                + (1. - coverage) * behind.pixels[pixel][2].max(0.),
                        ),
                        1.,
                    ]);
                }
            }
        }
        behind = output.clone();
        snapshots.push(output);
    }
    snapshots
}

fn material(base: [f32; 4], opacity: f32, ior: f32) -> [u32; 12] {
    let mut words = [0; 12];
    put(&mut words, 0, &base);
    put(&mut words, 4, &[0.15, 0.6]);
    words[7] = 1; // Thin; authored shade does not consume a receiver lighting service.
    words[8] = 1f32.to_bits();
    words[11] = u32::from(f16::from_f32(opacity).to_bits())
        | (u32::from(f16::from_f32(ior).to_bits()) << 16);
    words
}

fn pane(
    bounds: [u32; 4],
    z: f32,
    slot: u32,
    mirror: bool,
    color: [f32; 4],
    size: [u32; 2],
) -> Pane {
    let [left, top, right, bottom] = bounds;
    let mut x = [
        2. * left as f32 / size[0] as f32 - 1.,
        2. * right as f32 / size[0] as f32 - 1.,
    ];
    if mirror {
        x = [-x[1], -x[0]];
    }
    let y = [
        1. - 2. * bottom as f32 / size[1] as f32,
        1. - 2. * top as f32 / size[1] as f32,
    ];
    let vertices = [(x[0], y[0]), (x[1], y[0]), (x[1], y[1]), (x[0], y[1])]
        .into_iter()
        .enumerate()
        .map(|(i, (x, y))| Vertex {
            position: [x, y, z],
            normal: [0., 0., 1.],
            uv: [[0., 1.], [1., 1.], [1., 0.], [0., 0.]][i],
            color,
        })
        .collect();
    Pane {
        bounds,
        vertices,
        indices: vec![0, 1, 2, 0, 2, 3],
        slot,
        material: [0; 12],
        parameters: [0; 8],
        draw: [0; 4],
        base: Image {
            size: [1, 1],
            pixels: vec![[1.; 4]],
        },
    }
}

/// Fixed raw buffers for two ordered panes and independent per-draw half stores.
/// Ordered draws change six then thirteen pixels, with three pixels changed twice.
/// Reversing the mirrored pane preserves the first snapshot; zero opacity preserves both.
pub fn fixture(case: Case) -> Fixture {
    let size = [8, 6];
    let identity = Mat4::IDENTITY.to_cols_array();
    let mut camera = [0; 20];
    put(&mut camera, 0, &identity);
    let mut frame = [0; 84];
    for offset in [0, 16, 32] {
        put(&mut frame, offset, &identity);
    }
    put(&mut frame, 48, &[0., 0., 2., 1.]);
    put(&mut frame, 52, &[0., 0., -1., 0.]);
    put(&mut frame, 56, &[1., 0., 0., 0.]);
    put(&mut frame, 60, &[0., 1., 0., 0.]);
    put(
        &mut frame,
        64,
        &[
            size[0] as f32,
            size[1] as f32,
            1. / size[0] as f32,
            1. / size[1] as f32,
        ],
    );
    put(&mut frame, 68, &[0., 0., 1., 0.]);
    put(&mut frame, 72, &[2., 1., 0.5, 1.]);
    frame[76..80].copy_from_slice(&[0, 0, size[0], size[1]]);
    put(&mut frame, 80, &[0., 0., 1., 0.]);
    let mut lighting = [0; 12];
    put(&mut lighting, 0, &[0., 0., 2.]);
    put(&mut lighting, 4, &[0.05, 0.04, 0.03, 1.]);
    let mut atmosphere = [0; 108];
    for (offset, value) in [
        (0, [0.005802, 0.013558, 0.0331, 0.125]),
        (4, [0.003996, 0.003996, 0.003996, 1. / 1.2]),
        (8, [0.000444, 0.000444, 0.000444, 0.8]),
        (12, [0.00065, 0.001881, 0.000085, 25.]),
        (16, [6360., 6460., 0., 15.]),
        (20, [0.3, 0.3, 0.3, 0.00465]),
        (24, [0., 0., 1., 0.]),
        (28, [2., 1., 0.5, 1.]),
        (32, [0., 0., 2., 0.001]),
        (36, [0., 0.01, 0., 0.]),
        (40, [1., 1., 1., 0.]),
        (44, [32., 0., 0., 1.]),
        (48, [0., 0., 2., 0.]),
    ] {
        put(&mut atmosphere, offset, &value);
    }
    for offset in [52, 68, 84] {
        put(&mut atmosphere, offset, &identity);
    }
    atmosphere[100..104].copy_from_slice(&[1, 1, 0, 0]);
    put(
        &mut atmosphere,
        104,
        &[size[0] as f32, size[1] as f32, 0., 0.],
    );
    let models = vec![
        identity.map(f32::to_bits),
        Mat4::from_scale(Vec3::new(-1., 1., 1.))
            .to_cols_array()
            .map(f32::to_bits),
    ];
    let normals = [1f32, -1.]
        .map(|sign| {
            let mut n = identity;
            n[0] = sign;
            n[12..].copy_from_slice(&[sign, 0., 0., 0.]);
            n.map(f32::to_bits)
        })
        .to_vec();
    let mut panes = vec![
        pane([1, 1, 6, 5], 0.25, 0, false, [0.75, 1., 0.5, 0.6], size),
        pane([3, 0, 8, 4], 0.5, 1, true, [1., 0.5, 0.8, 0.7], size),
    ];
    panes[0].material = material([0.9, 0.5, 1.2, 0.8], 0.43, 1.5);
    panes[0].parameters = [2f32, 0.5, 3., 0., 0.125, 0.0625, 0.125, 0.].map(f32::to_bits);
    panes[0].base.pixels[0] = half([0.8, 1.5, 0.25, 0.7]);
    panes[1].material = material([0.6, 1.4, 0.9, 0.75], 0.68, 1.75);
    panes[1].parameters = [0.75f32, 2., 1.5, 0., 0.25, 0.125, -0.0625, 0.].map(f32::to_bits);
    panes[1].base.pixels[0] = half([1.25, 0.375, 0.875, 0.65]);
    match case {
        Case::Ordered => {}
        Case::BackFacingMirroredPane => panes[1].indices = vec![0, 2, 1, 0, 3, 2],
        Case::ZeroOpacity => {
            for p in &mut panes {
                p.material[11] &= 0xffff_0000;
            }
        }
    }
    let background = Image {
        size,
        pixels: (0..size[0] * size[1])
            .map(|i| {
                let x = (i % size[0]) as f32;
                let y = (i / size[0]) as f32;
                half([
                    0.15 + x * 0.21 + y * 0.03,
                    0.3 + y * 0.28 + x * 0.04,
                    1.8 + (x + y) * 0.09,
                    1.,
                ])
            })
            .collect(),
    };
    let opaque_depth = (0..size[0] * size[1])
        .map(|i| match (i % size[0] + 2 * (i / size[0])) % 3 {
            0 => 1.4,
            1 => 1.6,
            _ => 2.,
        })
        .collect();
    let mut fixture = Fixture {
        size,
        camera,
        frame,
        lighting,
        atmosphere,
        models,
        normals,
        panes,
        background,
        opaque_depth,
        normal_map: Image {
            size: [1, 1],
            pixels: vec![half([0.5, 0.5, 1., 1.])],
        },
        metallic_roughness: Image {
            size: [1, 1],
            pixels: vec![[1.; 4]],
        },
        expected: Vec::new(),
    };
    fixture.expected = render(&fixture);
    fixture
}
