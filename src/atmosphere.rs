//! AT1: the physical sky, aerial perspective, height fog and volumetric fog
//! (docs/m7-at1.md). A Hillaire-2020 atmosphere (transmittance, multiple-scattering and
//! sky-view LUTs plus a camera volume), written from the paper. The sky is the default
//! primary-miss output on every path, aerial perspective and an analytic exponential
//! height fog apply to every camera-path surface after its program shades it, and
//! volumetric fog adds froxel in-scattering shadowed by world-space sun visibility.
//! `Renderer::set_atmosphere` replaces the settings; `Atmosphere::default()` is the
//! product default (including VF1's subtle shadowed volumetric haze).
use crate::{Error, Vec3};

/// Everything the renderer draws of the atmosphere. Validated by `Renderer::set_atmosphere`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Atmosphere {
    /// Primary misses show the sky (traced, visibility-buffer and raster frames). With
    /// `Renderer::set_visible_environment` on and a map selected, misses show that map instead.
    pub sky: bool,
    /// The sun's disk on the visible sky (the first directional light).
    pub sun_disk: bool,
    /// Aerial perspective over every camera-path surface.
    pub aerial_perspective: bool,
    /// Directional lights reach surfaces through the atmosphere's transmittance at the camera's
    /// altitude, so their lux is the illuminance outside the atmosphere. Applies only while no
    /// captured environment map is selected: a captured sky keeps its lights as given.
    pub sun_transmittance: bool,
    /// Procedural environment source: while no captured map is selected
    /// (`Renderer::set_environment(None, ..)`), the renderer generates the environment from
    /// this sky (diffuse irradiance and a prefiltered specular chain, without the sun disk), so
    /// image-based light and the world radiance cache take their sky from it. Updated in place
    /// when the sun, medium or camera altitude changes; GI relights progressively without
    /// losing geometric visibility. A selected captured map wins.
    pub environment: bool,
    pub medium: AtmosphereMedium,
    /// None disables the analytic height fog.
    pub height_fog: Option<HeightFog>,
    /// Shadowed volumetric haze on both paths by default; None disables it.
    pub volumetric_fog: Option<VolumetricFog>,
}

/// The planet and its atmosphere. Coefficients are per kilometre, Hillaire 2020's Earth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphereMedium {
    pub ground_radius_km: f32,
    pub top_radius_km: f32,
    /// World y (metres) of the planet's surface.
    pub ground_height: f32,
    pub rayleigh_scattering: [f32; 3],
    pub rayleigh_scale_height_km: f32,
    pub mie_scattering: [f32; 3],
    pub mie_absorption: [f32; 3],
    pub mie_scale_height_km: f32,
    /// Mie phase asymmetry (Cornette-Shanks), in (-1, 1).
    pub mie_g: f32,
    /// Ozone absorption at its peak, a tent of `ozone_half_width_km` about `ozone_peak_km`.
    pub ozone_absorption: [f32; 3],
    pub ozone_peak_km: f32,
    pub ozone_half_width_km: f32,
    pub ground_albedo: [f32; 3],
    /// The visible sun disk's angular radius (radians).
    pub sun_angular_radius: f32,
}

/// Exponential height fog: extinction `density * exp(-falloff * (y - base_height))` per metre,
/// integrated in closed form along each camera ray from `start_distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightFog {
    pub density: f32,
    pub falloff: f32,
    pub base_height: f32,
    pub start_distance: f32,
    /// Single-scattering albedo: the fog scatters `albedo * extinction`.
    pub albedo: [f32; 3],
    /// Henyey-Greenstein asymmetry of the sun's in-scattering, in (-1, 1).
    pub anisotropy: f32,
}

/// Froxel volumetric fog over the first `distance` metres of every camera ray. Its medium
/// is the height fog's (when on) plus `extra_density` per metre; beyond `distance` the
/// analytic height fog continues.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumetricFog {
    pub distance: f32,
    pub extra_density: f32,
    /// History share of each froxel's reprojected value, in [0, 1).
    pub history_weight: f32,
}

impl Default for AtmosphereMedium {
    fn default() -> Self {
        Self {
            ground_radius_km: 6360.0,
            top_radius_km: 6460.0,
            ground_height: 0.0,
            rayleigh_scattering: [5.802e-3, 13.558e-3, 33.1e-3],
            rayleigh_scale_height_km: 8.0,
            mie_scattering: [3.996e-3; 3],
            mie_absorption: [4.40e-3; 3],
            mie_scale_height_km: 1.2,
            mie_g: 0.8,
            ozone_absorption: [0.650e-3, 1.881e-3, 0.085e-3],
            ozone_peak_km: 25.0,
            ozone_half_width_km: 15.0,
            ground_albedo: [0.3; 3],
            sun_angular_radius: 0.004_65,
        }
    }
}
impl Default for HeightFog {
    /// A light ground haze: about 2% extinction per 100 m at the base height, thinning
    /// with a 50 m scale height.
    fn default() -> Self {
        Self {
            density: 2e-4,
            falloff: 0.02,
            base_height: 0.0,
            start_distance: 0.0,
            albedo: [0.9; 3],
            anisotropy: 0.8,
        }
    }
}
impl Default for VolumetricFog {
    fn default() -> Self {
        Self {
            distance: 128.0,
            extra_density: 0.001,
            history_weight: 0.9,
        }
    }
}
impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            sky: true,
            sun_disk: true,
            aerial_perspective: true,
            sun_transmittance: true,
            environment: true,
            medium: AtmosphereMedium::default(),
            height_fog: Some(HeightFog::default()),
            volumetric_fog: Some(VolumetricFog::default()),
        }
    }
}
impl Atmosphere {
    /// No sky, aerial perspective, sun transmittance or fog: frames match a renderer before AT1.
    pub fn off() -> Self {
        Self {
            sky: false,
            sun_disk: false,
            aerial_perspective: false,
            sun_transmittance: false,
            environment: false,
            medium: AtmosphereMedium::default(),
            height_fog: None,
            volumetric_fog: None,
        }
    }
    /// Whether any part runs on a lit frame.
    pub fn active(&self) -> bool {
        self.sky
            || self.aerial_perspective
            || self.height_fog.is_some()
            || self.volumetric_fog.is_some()
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let m = &self.medium;
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite() && *x >= 0.0);
        if !(m.ground_radius_km.is_finite()
            && m.ground_radius_km > 0.0
            && m.top_radius_km.is_finite()
            && m.top_radius_km > m.ground_radius_km
            && m.ground_height.is_finite()
            && m.rayleigh_scale_height_km.is_finite()
            && m.rayleigh_scale_height_km > 0.0
            && m.mie_scale_height_km.is_finite()
            && m.mie_scale_height_km > 0.0
            && m.ozone_half_width_km.is_finite()
            && m.ozone_half_width_km > 0.0
            && m.ozone_peak_km.is_finite()
            && m.mie_g.abs() < 1.0
            && m.sun_angular_radius.is_finite()
            && m.sun_angular_radius > 0.0
            && m.sun_angular_radius < 0.5
            && finite(&m.rayleigh_scattering)
            && finite(&m.mie_scattering)
            && finite(&m.mie_absorption)
            && finite(&m.ozone_absorption)
            && finite(&m.ground_albedo))
        {
            return Err(Error::InvalidInput(
                "atmosphere medium needs positive radii (top above ground), positive scale heights, |mie_g| < 1, a sun radius in (0, 0.5) and finite nonnegative coefficients",
            ));
        }
        if let Some(f) = self.height_fog
            && !(f.density.is_finite()
                && f.density >= 0.0
                && f.falloff.is_finite()
                && f.falloff >= 0.0
                && f.base_height.is_finite()
                && f.start_distance.is_finite()
                && f.start_distance >= 0.0
                && finite(&f.albedo)
                && f.anisotropy.abs() < 1.0)
        {
            return Err(Error::InvalidInput(
                "height fog needs finite nonnegative density, falloff, start and albedo, and |anisotropy| < 1",
            ));
        }
        if let Some(v) = self.volumetric_fog
            && !(v.distance.is_finite()
                && v.distance > 0.0
                && v.extra_density.is_finite()
                && v.extra_density >= 0.0
                && (0.0..1.0).contains(&v.history_weight))
        {
            return Err(Error::InvalidInput(
                "volumetric fog needs a positive finite distance, nonnegative extra density and a history weight in [0, 1)",
            ));
        }
        Ok(())
    }

    /// Transmittance from altitude `altitude_m` above the ground toward unit `toward_sun`,
    /// to the top of the atmosphere: zero once the ray enters the ground. The renderer
    /// multiplies directional lux by it when `sun_transmittance` applies.
    pub fn transmittance(&self, altitude_m: f32, toward_sun: Vec3) -> [f32; 3] {
        let m = &self.medium;
        let (bottom, top) = (f64::from(m.ground_radius_km), f64::from(m.top_radius_km));
        let h = (f64::from(altitude_m) * 1e-3).clamp(1e-3, top - bottom - 1e-3);
        let d = toward_sun.as_dvec3().normalize_or_zero();
        let p = glam::DVec3::new(0.0, bottom + h, 0.0);
        if sphere(p, d, bottom).is_some() {
            return [0.0; 3];
        }
        let t = sphere(p, d, top).unwrap_or(0.0);
        let steps = 256;
        let mut depth = [0.0f64; 3];
        for i in 0..steps {
            let q = p + d * (t * (f64::from(i) + 0.5) / f64::from(steps));
            let e = extinction(m, q.length() - bottom);
            for k in 0..3 {
                depth[k] += e[k] * t / f64::from(steps);
            }
        }
        depth.map(|x| (-x).exp() as f32)
    }
}

/// The shading API composed into camera-pixel program modules (src/program.rs) and the
/// raster sky: the shared definitions over the lighting group's DFG sampler, then the API.
pub(crate) fn api_source() -> String {
    format!(
        "{}{}",
        include_str!("../shaders/atmosphere_common.wgsl")
            .replace("atmosphere_sampler", "dfg_sampler"),
        include_str!("../shaders/atmosphere_api.wgsl")
    )
}

/// Extinction (/km) of `medium` at altitude h (km); the same terms as shaders/atmosphere_common.wgsl.
fn extinction(m: &AtmosphereMedium, h: f64) -> [f64; 3] {
    let h = h.max(0.0);
    let dr = (-h / f64::from(m.rayleigh_scale_height_km)).exp();
    let dm = (-h / f64::from(m.mie_scale_height_km)).exp();
    let dozone =
        (1.0 - (h - f64::from(m.ozone_peak_km)).abs() / f64::from(m.ozone_half_width_km)).max(0.0);
    std::array::from_fn(|k| {
        f64::from(m.rayleigh_scattering[k]) * dr
            + f64::from(m.mie_scattering[k] + m.mie_absorption[k]) * dm
            + f64::from(m.ozone_absorption[k]) * dozone
    })
}
/// Nearest nonnegative distance from p along unit d to the sphere of `radius` about the origin.
fn sphere(p: glam::DVec3, d: glam::DVec3, radius: f64) -> Option<f64> {
    let b = p.dot(d);
    let disc = b * b - (p.dot(p) - radius * radius);
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    [-b - s, -b + s].into_iter().find(|t| *t >= 0.0)
}

/// Each directional light record's lux scaled by `scale` (lighting.rs packs 64-byte records:
/// rgb at bytes 32..44, kind 0 = directional at 48..52).
pub(crate) fn scale_directional(records: &[u8], scale: [f32; 3]) -> Vec<u8> {
    let mut out = records.to_vec();
    for record in out.chunks_exact_mut(64) {
        if u32::from_ne_bytes(record[48..52].try_into().unwrap()) != 0 {
            continue;
        }
        for (k, s) in scale.iter().enumerate() {
            let at = 32 + 4 * k;
            let v = f32::from_ne_bytes(record[at..at + 4].try_into().unwrap()) * s;
            record[at..at + 4].copy_from_slice(&v.to_ne_bytes());
        }
    }
    out
}

#[cfg(test)]
#[path = "atmosphere_tests.rs"]
mod tests;
