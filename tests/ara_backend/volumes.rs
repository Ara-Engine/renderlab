//! Scalar camera-volume integration; packed rows represent successive Z slices.
use super::{
    atmosphere_reference::Model,
    reference::{Image, half},
};
use renderlab::{Atmosphere, Camera, Vec3};

const FOG_SLICES: u32 = 64;
const AERIAL_SLICES: u32 = 32;
const VOLUME_SLICES: u32 = FOG_SLICES + AERIAL_SLICES;
const PI: f32 = std::f32::consts::PI;

fn empty_volume(grid: [u32; 2]) -> Image {
    assert!(grid[0] > 0 && grid[1] > 0);
    Image {
        size: [grid[0], grid[1] * VOLUME_SLICES],
        pixels: vec![[0.; 4]; (grid[0] * grid[1] * VOLUME_SLICES) as usize],
    }
}

/// Pixel-centred world rays use the actual camera eye, including translated views.
fn rays(camera: Camera, viewport: [u32; 2], grid: [u32; 2]) -> impl Iterator<Item = Vec3> {
    let inverse = camera.matrix(viewport[0], viewport[1]).inverse();
    let eye = camera.view().inverse().w_axis.truncate();
    (0..grid[0] * grid[1]).map(move |column| {
        let x = (column % grid[0]) as f32 + 0.5;
        let y = (column / grid[0]) as f32 + 0.5;
        let ndc = glam::Vec4::new(
            x / grid[0] as f32 * 2. - 1.,
            1. - y / grid[1] as f32 * 2.,
            1.,
            1.,
        );
        let point = inverse * ndc;
        (point.truncate() / point.w - eye).normalize()
    })
}

/// Aerial luminance and mean RGB transmittance in packed slices 64..96.
///
/// Two midpoint samples integrate each quadratic kilometre interval. State remains
/// f32 between intervals; each output store alone rounds through RGBA16Float.
/// Slices 0..64 are zero so callers can compose this with the fog oracle.
pub fn aerial(
    model: &Model,
    trans: &Image,
    multi: &Image,
    camera: Camera,
    viewport: [u32; 2],
    grid: [u32; 2],
) -> Image {
    let mut image = empty_volume(grid);
    let columns = (grid[0] * grid[1]) as usize;
    let bottom = model.medium.ground_radius_km;
    let origin = Vec3::new(0., bottom + model.altitude, 0.);
    for (column, direction) in rays(camera, viewport, grid).enumerate() {
        let exit = model
            .sphere(origin, direction, model.medium.top_radius_km)
            .max(0.);
        let cosine = direction.dot(model.sun);
        let rayleigh_phase = 3. * (1. + cosine * cosine) / (16. * PI);
        let g = model.medium.mie_g;
        let mie_phase = 3. / (8. * PI) * (1. - g * g) / (2. + g * g) * (1. + cosine * cosine)
            / (1. + g * g - 2. * g * cosine).max(1e-6).powf(1.5);
        let mut luminance = Vec3::ZERO;
        let mut throughput = Vec3::ONE;
        let mut near = 0.;
        for slice in 0..AERIAL_SLICES {
            let coordinate = (slice + 1) as f32 / AERIAL_SLICES as f32;
            let far = (32. * coordinate * coordinate).min(exit).max(near);
            let step = (far - near) * 0.5;
            for sample in 0..2 {
                let distance = near + (sample as f32 + 0.5) * step;
                let point = origin + direction * distance;
                let radius = point.length();
                let (scattering, extinction, rayleigh, mie) = model.medium(radius - bottom);
                let sun_cosine = (point / radius).dot(model.sun);
                let sunlight = if model.sphere(point, model.sun, bottom) > 0. {
                    Vec3::ZERO
                } else {
                    model.trans_sample(trans, radius, sun_cosine)
                };
                let source = (rayleigh * rayleigh_phase + mie * mie_phase) * sunlight
                    + scattering * model.multi_sample(multi, radius, sun_cosine);
                let attenuation = (-extinction * step).exp();
                // Closed-form constant-medium contribution over this midpoint cell.
                let integral = (source - source * attenuation) / extinction.max(Vec3::splat(1e-7));
                luminance += throughput * integral;
                throughput *= attenuation;
            }
            near = far;
            let mean = throughput.dot(Vec3::splat(1. / 3.));
            image.pixels[(FOG_SLICES + slice) as usize * columns + column] =
                half([luminance.x, luminance.y, luminance.z, mean]);
        }
    }
    image
}

/// Integrate visibility/extinction cells into packed fog slices 0..64.
///
/// `scatter` is an RGBA16Float stream packed as `[grid.x, grid.y * 64]`.
/// Cell X is sun visibility and W is extinction per metre. The ambient argument
/// passes through the same half store as the transmittance LUT's ambient row.
/// Slices 64..96 are zero; fog integration does not apply exposure or sun lux.
#[allow(clippy::too_many_arguments)]
pub fn fog(
    model: &Model,
    trans: &Image,
    ambient: [f32; 3],
    scatter: &Image,
    settings: Atmosphere,
    camera: Camera,
    viewport: [u32; 2],
    grid: [u32; 2],
) -> Image {
    let mut image = empty_volume(grid);
    assert_eq!(scatter.size, [grid[0], grid[1] * FOG_SLICES]);
    let columns = (grid[0] * grid[1]) as usize;
    assert_eq!(scatter.pixels.len(), columns * FOG_SLICES as usize);
    // The disabled height-fog uniform still uses the default albedo and phase;
    // its zero density is immaterial because extinction comes from `scatter`.
    let height = settings.height_fog.unwrap_or_default();
    let range = settings.volumetric_fog.unwrap_or_default().distance;
    let albedo = Vec3::from_array(height.albedo);
    let rounded = half([ambient[0], ambient[1], ambient[2], 1.]);
    let ambient = Vec3::new(rounded[0], rounded[1], rounded[2]);
    let sun_transmittance = model.trans_sample(
        trans,
        model.medium.ground_radius_km + model.altitude,
        model.sun.y,
    );
    for (column, direction) in rays(camera, viewport, grid).enumerate() {
        let g = height.anisotropy;
        let denominator = (1. + g * g - 2. * g * direction.dot(model.sun)).max(1e-6);
        let phase = (1. - g * g) / (4. * PI * denominator * denominator.sqrt());
        let sun = sun_transmittance * phase;
        let mut luminance = Vec3::ZERO;
        let mut throughput = 1.;
        let mut near = 0.;
        for slice in 0..FOG_SLICES {
            let coordinate = (slice + 1) as f32 / FOG_SLICES as f32;
            let far = range * ((5. * coordinate).exp2() - 1.) / 31.;
            let cell = scatter.pixels[slice as usize * columns + column];
            let attenuation = (-cell[3] * (far - near)).exp();
            let source = albedo * (sun * cell[0] + ambient);
            luminance += throughput * source * (1. - attenuation);
            throughput *= attenuation;
            near = far;
            image.pixels[slice as usize * columns + column] =
                half([luminance.x, luminance.y, luminance.z, throughput]);
        }
    }
    image
}

/// Deterministic half-float cells with varying visibility and positive extinction.
pub fn scatter_fixture(grid: [u32; 2]) -> Image {
    assert!(grid[0] > 0 && grid[1] > 0);
    let columns = grid[0] * grid[1];
    let pixels = (0..columns * FOG_SLICES)
        .map(|index| {
            let x = index % grid[0];
            let y = index / grid[0] % grid[1];
            let slice = index / columns;
            let visibility = ((3 * x + 5 * y + 7 * slice) % 17) as f32 / 16.;
            let extinction = 0.0005 + ((x + 2 * y + 3 * slice) % 13) as f32 * 0.00025;
            half([visibility, 0., 0., extinction])
        })
        .collect();
    Image {
        size: [grid[0], grid[1] * FOG_SLICES],
        pixels,
    }
}
