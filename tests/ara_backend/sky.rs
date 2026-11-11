//! Scalar raster-sky oracle with analytic disk coverage and trilinear fog sampling.
use super::{
    atmosphere_reference::Model,
    reference::{Image, sample},
};
use glam::{Mat4, Vec2};
use renderlab::{Atmosphere, Camera, HeightFog, Vec3};

const PI: f32 = std::f32::consts::PI;
const FOG_SLICES: u32 = 64;
const VOLUME_SLICES: u32 = 96;

struct View {
    matrix: Mat4,
    inverse: Mat4,
    eye: Vec3,
    viewport: [u32; 2],
}

impl View {
    fn direction(&self, ndc: Vec2) -> Vec3 {
        let point = self.inverse * ndc.extend(1.).extend(1.);
        (point.truncate() / point.w - self.eye).normalize()
    }

    fn project_direction(&self, direction: Vec3) -> Vec2 {
        let clip = self.matrix * (self.eye + direction).extend(1.);
        clip.truncate().truncate() / clip.w.max(1e-6)
    }
}

/// CDF of the sum of two centred uniforms, expressed symmetrically about zero.
fn rim_coverage(distance: f32, a: f32, b: f32) -> f32 {
    let half_width = 0.5 * (a + b);
    if distance >= half_width {
        return 1.;
    }
    if distance <= -half_width {
        return 0.;
    }
    let wide = a.max(b);
    let narrow = a.min(b);
    if narrow < 1e-7 {
        return ((distance + half_width) / wide.max(1e-7)).clamp(0., 1.);
    }
    if distance.abs() <= 0.5 * (wide - narrow) {
        return 0.5 + distance / wide;
    }
    let edge = half_width - distance.abs();
    let tail = edge * edge / (2. * wide * narrow);
    if distance < 0. { tail } else { 1. - tail }
}

fn sun_coverage(view: &View, direction: Vec3, sun: Vec3, radius: f32) -> f32 {
    let offset = direction - sun;
    let separation = offset.length();
    let normal = offset / separation.max(1e-7);
    let ndc = view.project_direction(direction);
    let dx = view.direction(ndc + Vec2::new(2. / view.viewport[0] as f32, 0.)) - direction;
    let dy = view.direction(ndc + Vec2::new(0., 2. / view.viewport[1] as f32)) - direction;
    rim_coverage(
        2. * (radius * 0.5).sin() - separation,
        normal.dot(dx).abs(),
        normal.dot(dy).abs(),
    )
}

/// Rationalized near intersection retains metre distances at planet-scale radii.
fn backdrop_distance(model: &Model, direction: Vec3) -> f32 {
    let h = model.altitude;
    let b = (model.medium.ground_radius_km + h) * direction.y;
    let c = h * (2. * model.medium.ground_radius_km + h);
    let discriminant = b * b - c;
    if b >= 0. || discriminant < 0. {
        1e30
    } else {
        1000. * c / (-b + discriminant.sqrt())
    }
}

fn fog_depth(fog: HeightFog, eye: Vec3, direction: Vec3, near: f32, far: f32) -> f32 {
    if far <= near || fog.density <= 0. {
        return 0.;
    }
    let base = fog.density
        * (-fog.falloff * (eye.y - fog.base_height))
            .clamp(-80., 80.)
            .exp();
    let slope = fog.falloff * direction.y;
    let length = far - near;
    let depth = if (slope * length).abs() < 1e-4 {
        base * length * (-slope * 0.5 * (near + far)).clamp(-80., 80.).exp()
    } else {
        base * ((-slope * near).clamp(-80., 80.).exp() - (-slope * far).clamp(-80., 80.).exp())
            / slope
    };
    depth.min(1e4)
}

/// Clamp filtering operates independently within each spatial texture dimension.
fn volume_sample(volume: &Image, uv: Vec2, coordinate: f32) -> [f32; 4] {
    let height = volume.size[1] / VOLUME_SLICES;
    let size = [volume.size[0] as i32, height as i32, VOLUME_SLICES as i32];
    let position = [
        uv.x * size[0] as f32 - 0.5,
        uv.y * size[1] as f32 - 0.5,
        (coordinate - 1.).clamp(0., FOG_SLICES as f32 - 1.),
    ];
    let lo = position.map(|value| value.floor() as i32);
    let fraction = std::array::from_fn::<_, 3, _>(|axis| position[axis] - lo[axis] as f32);
    let mut value = [0.; 4];
    for z in 0..2 {
        for y in 0..2 {
            for x in 0..2 {
                let offsets = [x, y, z];
                let cell = std::array::from_fn::<_, 3, _>(|axis| {
                    (lo[axis] + offsets[axis]).clamp(0, size[axis] - 1) as usize
                });
                let weight: f32 = (0..3)
                    .map(|axis| {
                        if offsets[axis] == 0 {
                            1. - fraction[axis]
                        } else {
                            fraction[axis]
                        }
                    })
                    .product();
                let index =
                    (cell[2] * height as usize + cell[1]) * volume.size[0] as usize + cell[0];
                let texel = volume.pixels[index];
                for (value, texel) in value.iter_mut().zip(texel) {
                    *value += texel * weight;
                }
            }
        }
    }
    let fade = coordinate.clamp(0., 1.);
    [
        value[0] * fade,
        value[1] * fade,
        value[2] * fade,
        1. - fade + value[3] * fade,
    ]
}

/// Reference for `fs_sky`, returning RGBA32Float without a final half store.
///
/// `illuminance` is the uniform's exposure-scaled sun RGB. The model must contain
/// the uploaded sun direction, medium and clamped planet altitude in kilometres.
/// `trans` includes the mean-sky ambient texel at (0,64); `volume` packs all 96
/// slices into rows. Inputs are their stored texel values, including half rounding.
/// The fragment entry renders sky directly; `settings.sky` does not gate this draw.
#[allow(clippy::too_many_arguments)]
pub fn render(
    model: &Model,
    trans: &Image,
    sky: &Image,
    volume: &Image,
    settings: Atmosphere,
    camera: Camera,
    viewport: [u32; 2],
    illuminance: [f32; 3],
) -> Image {
    assert!(viewport[0] > 0 && viewport[1] > 0);
    assert!(trans.size[0] > 0 && trans.size[1] > 64);
    assert!(volume.size[0] > 0 && volume.size[1] >= VOLUME_SLICES);
    assert_eq!(volume.size[1] % VOLUME_SLICES, 0);
    let matrix = camera.matrix(viewport[0], viewport[1]);
    let view = View {
        matrix,
        inverse: matrix.inverse(),
        eye: camera.view().inverse().w_axis.truncate(),
        viewport,
    };
    let expose = |value: Vec3| (value * Vec3::from_array(illuminance)).min(Vec3::splat(65504.));
    let sun_transmittance = model.trans_sample(
        trans,
        model.medium.ground_radius_km + model.altitude,
        model.sun.y,
    );
    let ambient_texel = trans.pixels[(64 * trans.size[0]) as usize];
    let ambient = Vec3::new(ambient_texel[0], ambient_texel[1], ambient_texel[2]);
    let pixels = (0..viewport[0] * viewport[1])
        .map(|pixel| {
            let uv = Vec2::new(
                ((pixel % viewport[0]) as f32 + 0.5) / viewport[0] as f32,
                ((pixel / viewport[0]) as f32 + 0.5) / viewport[1] as f32,
            );
            let direction = view.direction(Vec2::new(uv.x * 2. - 1., 1. - uv.y * 2.));
            let texel = sample(sky, model.sky_uv(direction));
            let mut radiance = Vec3::new(texel[0], texel[1], texel[2]);
            let radius = model.medium.sun_angular_radius;
            if settings.sky && settings.sun_disk {
                let coverage = sun_coverage(&view, direction, model.sun, radius);
                let normalized_radius =
                    (direction - model.sun).length() / (2. * (0.5 * radius).sin()).max(1e-6);
                let cosine = (1. - normalized_radius * normalized_radius).max(0.).sqrt();
                let limb = (1. - 0.6 * (1. - cosine)) / 0.8;
                let origin = Vec3::new(0., model.medium.ground_radius_km + model.altitude, 0.);
                if model.sphere(origin, direction, model.medium.ground_radius_km) < 0. {
                    radiance += sun_transmittance * limb * coverage / (PI * radius * radius);
                }
            }
            let mut exposed = expose(radiance);
            let end = backdrop_distance(model, direction);
            if let Some(fog) = settings.height_fog {
                let start = match settings.volumetric_fog {
                    Some(volume) => fog.start_distance.max(volume.distance),
                    None => fog.start_distance,
                };
                let transmittance =
                    (-fog_depth(fog, view.eye, direction, start, end.min(1e5))).exp();
                let g = fog.anisotropy;
                let phase_denominator = (1. + g * g - 2. * g * direction.dot(model.sun)).max(1e-6);
                let phase = (1. - g * g) / (4. * PI * phase_denominator * phase_denominator.sqrt());
                let source = Vec3::from_array(fog.albedo) * (sun_transmittance * phase + ambient);
                exposed = exposed * transmittance + expose(source) * (1. - transmittance);
            }
            if let Some(fog) = settings.volumetric_fog {
                let ndc = view.project_direction(direction);
                let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
                let coordinate =
                    (end.min(fog.distance).max(0.) / fog.distance.max(1e-6) * 31. + 1.).log2() / 5.
                        * FOG_SLICES as f32;
                let integrated = volume_sample(volume, uv, coordinate);
                exposed = exposed * integrated[3]
                    + expose(Vec3::new(integrated[0], integrated[1], integrated[2]));
            }
            [exposed.x, exposed.y, exposed.z, 1.]
        })
        .collect();
    Image {
        size: viewport,
        pixels,
    }
}

/// Every combination of sun disk, height fog and volumetric fog for one fixture.
/// Bits 0, 1 and 2 of the case index enable those effects respectively.
pub fn settings_cases(base: Atmosphere) -> [Atmosphere; 8] {
    std::array::from_fn(|case| Atmosphere {
        sky: true,
        sun_disk: case & 1 != 0,
        height_fog: (case & 2 != 0).then_some(base.height_fog.unwrap_or_default()),
        volumetric_fog: (case & 4 != 0).then_some(base.volumetric_fog.unwrap_or_default()),
        ..base
    })
}
