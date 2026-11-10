//! Orthographic scalar replay of the original premultiplied particle billboards.
use super::reference::{Image, half, sample};
use glam::Vec4Swizzles;
use half::f16;
use renderlab::{Mat4, Vec3};

/// Original draw inputs, stored texture bytes and half-rounded final color.
pub struct Fixture {
    /// `TpFrame`, 336 bytes in source-layout words.
    pub frame: [u32; 84],
    /// `TpEmitter`, 512 bytes; capacity is five while storage contains padding.
    pub emitter: [u32; 128],
    /// Eight source-layout particles, including readable padding records.
    pub particles: Vec<[u32; 12]>,
    /// Far-to-near packed keys, followed by zero-depth dead/padded ranks.
    pub sorted: Vec<u32>,
    /// `LightingUniform`, 48 bytes; its exposure scales particle scattering/emission.
    pub lighting: [u32; 12],
    /// Non-indexed indirect arguments: six vertices and eight instances.
    pub draw_args: [u32; 4],
    /// Single-sample RGBA16Float target dimensions.
    pub viewport: [u32; 2],
    /// Opaque linear view depth, sampled by integer pixel coordinate.
    pub depth: Vec<f32>,
    /// Little-endian R32Float upload bytes for `depth`.
    pub depth_bytes: Vec<u8>,
    /// Four stored RGBA16Float sprite texels, with distinct color and opacity.
    pub sprite: Image,
    /// Little-endian RGBA16Float upload bytes for `sprite`.
    pub sprite_bytes: Vec<u8>,
    /// Nonblack color clear, already rounded through the target's half format.
    pub background: [f32; 4],
    /// Final stored pixels after every source-over blend rounds through half.
    pub expected: Image,
}

fn vector(words: &[u32], offset: usize) -> Vec3 {
    Vec3::new(
        f32::from_bits(words[offset]),
        f32::from_bits(words[offset + 1]),
        f32::from_bits(words[offset + 2]),
    )
}

fn put(words: &mut [u32], offset: usize, values: &[f32]) {
    for (word, value) in words[offset..].iter_mut().zip(values) {
        *word = value.to_bits();
    }
}

fn fraction_exponent(value: f32) -> (f32, i32) {
    if value == 0. {
        return (value, 0);
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32;
    if exponent == 0 {
        let (fraction, exponent) = fraction_exponent(value * 16_777_216.);
        return (fraction, exponent - 24);
    }
    (
        f32::from_bits((bits & 0x807f_ffff) | (126 << 23)),
        exponent - 126,
    )
}

/// Scene exposure's normal f32 order and its overflow-safe mantissa/exponent path.
fn exposed(a: Vec3, b: Vec3, exposure: f32) -> Vec3 {
    let product = a * b;
    let result = product * exposure;
    let normal = (f32::MIN_POSITIVE..=3e38).contains(&exposure)
        && (0..3).all(|channel| {
            ((f32::MIN_POSITIVE..=3e38).contains(&product[channel])
                && result[channel] >= f32::MIN_POSITIVE)
                || (product[channel] == 0. && (a[channel] == 0. || b[channel] == 0.))
        });
    if normal {
        return result.min(Vec3::splat(65504.));
    }
    Vec3::from_array(std::array::from_fn(|channel| {
        let (af, ae) = fraction_exponent(a[channel]);
        let (bf, be) = fraction_exponent(b[channel]);
        let (ef, ee) = fraction_exponent(exposure);
        let mantissa = af * bf * ef;
        let exponent = (ae + be + ee).clamp(-126, 20);
        ((f64::from(mantissa) * 2f64.powi(exponent)) as f32).min(65504.)
    }))
}

fn interpolate(a: f32, b: f32, amount: f32) -> f32 {
    a * (1. - amount) + b * amount
}

/// Shade sorted axis-aligned billboards and blend into the original half-float target.
///
/// The bounded fixture uses positive orthographic X/Y scales, camera right X/up Y,
/// forward -Z and unclipped live quads. The internal two-triangle diagonal has one
/// owner, so each covered rectangle pixel is evaluated once. Atmosphere flags must
/// be off; its original surface API then contributes identity transmittance and no
/// in-scattering. Sprite filtering uses ideal scalar bilinear clamp; native sampler
/// fraction quantization may require a small tolerance for the textured variant.
pub fn render(fixture: &Fixture) -> Image {
    let [width, height] = fixture.viewport;
    assert!(width > 0 && height > 0);
    assert_eq!(fixture.depth.len(), (width * height) as usize);
    assert_eq!(fixture.draw_args[0], 6);
    assert_eq!(fixture.draw_args[2..], [0, 0]);
    assert!(fixture.draw_args[1] as usize <= fixture.sorted.len());
    let matrix = Mat4::from_cols_array(&std::array::from_fn(|i| f32::from_bits(fixture.frame[i])));
    assert!(matrix.x_axis.x > 0. && matrix.y_axis.y > 0.);
    assert_eq!(matrix.x_axis.yzw(), Vec3::ZERO);
    assert_eq!(matrix.y_axis.xzw(), Vec3::ZERO);
    assert_eq!(matrix.z_axis.xyw(), Vec3::ZERO);
    assert_eq!(matrix.w_axis.w, 1.);
    assert_eq!(vector(&fixture.frame, 56), Vec3::X);
    assert_eq!(vector(&fixture.frame, 60), Vec3::Y);
    assert_eq!(vector(&fixture.frame, 52), Vec3::NEG_Z);
    let eye = vector(&fixture.frame, 48);
    let emitter = &fixture.emitter;
    let exposure = f32::from_bits(fixture.lighting[7]);
    let soft_distance = f32::from_bits(emitter[36]);
    let textured = f32::from_bits(emitter[38]) != 0.;
    let mut image = Image {
        size: fixture.viewport,
        pixels: vec![half(fixture.background); (width * height) as usize],
    };
    for &key in fixture.sorted.iter().take(fixture.draw_args[1] as usize) {
        let slot = key & 0x7ff;
        // The original vertex entry loads this record before its dead/padding guard.
        let particle = fixture.particles[slot as usize].map(f32::from_bits);
        if key >> 11 == 0 || slot >= emitter[32] {
            continue;
        }
        let t = (particle[3] / particle[7].max(1e-6)).clamp(0., 1.);
        let size = interpolate(f32::from_bits(emitter[23]), f32::from_bits(emitter[27]), t);
        let half_size = size * 0.5;
        assert!(size > 0.);
        let position = Vec3::new(particle[0], particle[1], particle[2]);
        let bottom_left = matrix * (position - Vec3::new(half_size, half_size, 0.)).extend(1.);
        let top_right = matrix * (position + Vec3::new(half_size, half_size, 0.)).extend(1.);
        for corner in [bottom_left, top_right] {
            assert!(
                corner.w == 1.
                    && corner.x.abs() <= 1.
                    && corner.y.abs() <= 1.
                    && (0. ..=1.).contains(&corner.z),
                "live billboard must stay inside the orthographic clip volume"
            );
        }
        let left = (bottom_left.x * 0.5 + 0.5) * width as f32;
        let right = (top_right.x * 0.5 + 0.5) * width as f32;
        let top = (0.5 - top_right.y * 0.5) * height as f32;
        let bottom = (0.5 - bottom_left.y * 0.5) * height as f32;
        let fade_in = if f32::from_bits(emitter[30]) > 0. {
            (t / f32::from_bits(emitter[30])).clamp(0., 1.)
        } else {
            1.
        };
        let fade_out = if f32::from_bits(emitter[31]) > 0. {
            ((1. - t) / f32::from_bits(emitter[31])).clamp(0., 1.)
        } else {
            1.
        };
        let color: [f32; 4] = std::array::from_fn(|i| {
            interpolate(
                f32::from_bits(emitter[12 + i]),
                f32::from_bits(emitter[16 + i]),
                t,
            )
        });
        let color_alpha = color[3] * (fade_in * fade_out);
        let emission = Vec3::from_array(std::array::from_fn(|i| {
            interpolate(
                f32::from_bits(emitter[20 + i]),
                f32::from_bits(emitter[24 + i]),
                t,
            )
        })) * fade_in
            * fade_out;
        let radiance = Vec3::new(particle[8], particle[9], particle[10])
            * (Vec3::new(color[0], color[1], color[2]) / std::f32::consts::PI);
        let depth = (position - eye).dot(Vec3::NEG_Z);
        for y in 0..height {
            for x in 0..width {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if px < left || px >= right || py < top || py >= bottom {
                    continue;
                }
                let index = (y * width + x) as usize;
                let opaque = fixture.depth[index];
                if depth > opaque {
                    continue;
                }
                let uv = [(px - left) / (right - left), (py - top) / (bottom - top)];
                let (shape, tint) = if textured {
                    let sprite = sample(&fixture.sprite, uv);
                    (sprite[3], Vec3::new(sprite[0], sprite[1], sprite[2]))
                } else {
                    let dx = uv[0] * 2. - 1.;
                    let dy = uv[1] * 2. - 1.;
                    let radial = (1. - (dx * dx + dy * dy).sqrt()).clamp(0., 1.);
                    (radial * radial, Vec3::ONE)
                };
                let soft = if soft_distance > 0. {
                    ((opaque - depth) / soft_distance).clamp(0., 1.)
                } else {
                    1.
                };
                let alpha = (color_alpha * shape * soft).clamp(0., 1.);
                let rgb = (exposed(radiance, tint * alpha, exposure)
                    + exposed(emission, tint * (shape * soft), exposure))
                .clamp(Vec3::ZERO, Vec3::splat(65504.));
                let destination = image.pixels[index];
                image.pixels[index] = half([
                    rgb.x + destination[0] * (1. - alpha),
                    rgb.y + destination[1] * (1. - alpha),
                    rgb.z + destination[2] * (1. - alpha),
                    alpha + destination[3] * (1. - alpha),
                ]);
            }
        }
    }
    image
}

/// Three overlapping live particles, two dead slots and three padded sort ranks.
pub fn fixture(textured: bool) -> Fixture {
    let viewport = [24, 20];
    let projection = Mat4::from_cols(
        glam::Vec4::new(2. / viewport[0] as f32, 0., 0., 0.),
        glam::Vec4::new(0., 2. / viewport[1] as f32, 0., 0.),
        glam::Vec4::new(0., 0., -0.1, 0.),
        glam::Vec4::W,
    );
    let mut frame = [0; 84];
    put(&mut frame, 0, &projection.to_cols_array());
    put(&mut frame, 16, &projection.inverse().to_cols_array());
    put(&mut frame, 32, &Mat4::IDENTITY.to_cols_array());
    put(&mut frame, 48, &[0., 0., 0., 1.75]);
    put(&mut frame, 52, &[0., 0., -1., 0.]);
    put(&mut frame, 56, &[1., 0., 0., 0.]);
    put(&mut frame, 60, &[0., 1., 0., 0.]);
    put(&mut frame, 64, &[24., 20., 1. / 24., 1. / 20.]);
    frame[76..80].copy_from_slice(&[0, 0, 24, 20]);
    let mut emitter = [0; 128];
    emitter[124] = 1f32.to_bits(); // Preserve square sprites in the extended ABI.
    put(&mut emitter, 12, &[0.8, 0.35, 0.2, 0.8]);
    put(&mut emitter, 16, &[0.2, 0.55, 0.9, 0.35]);
    put(&mut emitter, 20, &[0.12, 0.06, 0.03, 10.]);
    put(&mut emitter, 24, &[0.025, 0.04, 0.09, 6.]);
    put(&mut emitter, 28, &[0.5, 2., 0.25, 0.25]);
    emitter[32] = 5;
    put(
        &mut emitter,
        36,
        &[1.5, 8., f32::from(u8::from(textured)), 0.],
    );
    let rows = [
        (10., 10., 6., 0.125, 1., [3., 1.5, 0.75]),
        (13.25, 9.25, 4., 0.5, 1., [0.8, 2.5, 1.25]),
        (12., 12., 2.5, 0.875, 1., [1.25, 0.6, 3.]),
        (11., 8., 3., 2., 1., [8., 8., 8.]),
        (9., 11., 5., 0., 0., [9., 9., 9.]),
        (12., 10., 2., 0.5, 1., [30., 20., 40.]),
        (11., 9., 2., 0.5, 1., [30., 20., 40.]),
        (10., 10., 2., 0.5, 1., [30., 20., 40.]),
    ];
    let particles = rows
        .into_iter()
        .map(|(x, y, depth, age, life, light)| {
            [
                x - 12.,
                10. - y,
                -depth,
                age,
                0.,
                0.,
                0.,
                life,
                light[0],
                light[1],
                light[2],
                0.,
            ]
            .map(f32::to_bits)
        })
        .collect();
    let sorted = vec![
        6f32.to_bits() & !0x7ff,
        (4f32.to_bits() & !0x7ff) | 1,
        (2.5f32.to_bits() & !0x7ff) | 2,
        7,
        6,
        5,
        4,
        3,
    ];
    let mut lighting = [0; 12];
    put(&mut lighting, 4, &[0.05, 0.04, 0.03, 1.75]);
    put(&mut lighting, 8, &[1., 0., 1., 0.]);
    let depth: Vec<_> = (0..viewport[0] * viewport[1])
        .map(|index| {
            let x = index % viewport[0];
            let y = index / viewport[0];
            if x == 14 && y == 12 {
                2.5
            } else if x < 12 {
                3.
            } else if y < 10 {
                4.75
            } else {
                6.25
            }
        })
        .collect();
    let depth_bytes = depth
        .iter()
        .flat_map(|value| f32::to_le_bytes(*value))
        .collect();
    let sprite = Image {
        size: [2, 2],
        pixels: [
            [1., 0.25, 0.125, 0.2],
            [0.2, 0.8, 0.4, 0.9],
            [0.35, 0.15, 0.95, 0.65],
            [0.85, 0.65, 0.1, 0.4],
        ]
        .map(half)
        .to_vec(),
    };
    let sprite_bytes = sprite
        .pixels
        .iter()
        .flatten()
        .flat_map(|value| f16::from_f32(*value).to_bits().to_le_bytes())
        .collect();
    let background = half([0.03125, 0.0625, 0.125, 0.625]);
    let mut fixture = Fixture {
        frame,
        emitter,
        particles,
        sorted,
        lighting,
        draw_args: [6, 8, 0, 0],
        viewport,
        depth,
        depth_bytes,
        sprite,
        sprite_bytes,
        background,
        expected: Image {
            size: viewport,
            pixels: Vec::new(),
        },
    };
    fixture.expected = render(&fixture);
    fixture
}
