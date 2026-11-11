//! Independent original-toon shading for one unshadowed directional-light fixture.
use super::{
    mesh_reference::{self, Fixture, Fragment, Rasterized, unit},
    reference::{Image, sample},
};
use renderlab::Vec3;

const PI: f32 = std::f32::consts::PI;

/// The original toon program's 48-byte authored parameter block.
#[derive(Clone, Copy, Debug)]
pub struct Parameters {
    /// Tint at zero quantized incidence, before the light's RGB is applied.
    pub shadow_tint: [f32; 3],
    /// Number of quantized incidence bands, clamped by the shader to 2..16.
    pub bands: f32,
    /// Exposed rim RGB, independent of lighting exposure.
    pub rim_color: [f32; 3],
    /// Power of the view-normal rim falloff.
    pub rim_power: f32,
    /// Lower and upper light-incidence thresholds.
    pub thresholds: [f32; 2],
    /// Nonnegative multiplier on constant-environment fill.
    pub fill: f32,
    /// Zero selects smooth vertex normals; nonzero selects the triangle plane.
    pub flat_shaded: u32,
}

impl Default for Parameters {
    fn default() -> Self {
        Self {
            shadow_tint: [0.12, 0.08, 0.24],
            bands: 4.,
            rim_color: [0.12, 0.2, 0.3],
            rim_power: 4.,
            thresholds: [0.12, 0.9],
            fill: 0.25,
            flat_shaded: 0,
        }
    }
}

impl Parameters {
    /// Upload words at group 2 binding 7; the final word is a raw u32.
    pub fn words(self) -> [u32; 12] {
        [
            self.shadow_tint[0].to_bits(),
            self.shadow_tint[1].to_bits(),
            self.shadow_tint[2].to_bits(),
            self.bands.to_bits(),
            self.rim_color[0].to_bits(),
            self.rim_color[1].to_bits(),
            self.rim_color[2].to_bits(),
            self.rim_power.to_bits(),
            self.thresholds[0].to_bits(),
            self.thresholds[1].to_bits(),
            self.fill.to_bits(),
            self.flat_shaded,
        ]
    }
}

/// Exactly one directional light and constant environment, with shadows disabled.
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    /// Unit world direction from a receiver toward the light.
    pub direction: Vec3,
    /// Directional illuminance before exposure.
    pub illuminance: [f32; 3],
    /// Constant environment radiance; no environment map is selected.
    pub environment: [f32; 3],
    /// World eye position used for the view-dependent rim.
    pub eye: Vec3,
    /// Positive finite linear exposure multiplier.
    pub exposure: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            direction: Vec3::new(-0.55, 0.25, 0.8).normalize(),
            illuminance: [2., 1., 0.5],
            environment: [0.05, 0.04, 0.03],
            eye: Vec3::ZERO,
            exposure: 1.,
        }
    }
}

impl Lighting {
    /// Group 3 binding 0: eye/count, environment/exposure and map control.
    /// The light count is one and map.w is zero, selecting the constant environment.
    pub fn uniform_words(self) -> [u32; 12] {
        [
            self.eye.x.to_bits(),
            self.eye.y.to_bits(),
            self.eye.z.to_bits(),
            1,
            self.environment[0].to_bits(),
            self.environment[1].to_bits(),
            self.environment[2].to_bits(),
            self.exposure.to_bits(),
            1f32.to_bits(),
            0,
            1f32.to_bits(),
            0,
        ]
    }

    /// Group 3 binding 1: one 80-byte `Light`, with travel opposite `direction`.
    /// Zero kind words select a directional light without opaque or local shadows.
    pub fn light_words(self) -> [u32; 20] {
        let mut words = [0; 20];
        words[4..7].copy_from_slice(&(-self.direction).to_array().map(f32::to_bits));
        words[8..11].copy_from_slice(&self.illuminance.map(f32::to_bits));
        words
    }
}

/// Binary decomposition independent of the shader's frexp intrinsic.
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

/// Preserve normal f32 multiplication order, with a binary-scaled extreme-unit path.
fn exposed_product(a: Vec3, b: Vec3, exposure: f32) -> Vec3 {
    let product = a * b;
    let exposed = product * exposure;
    let fast = (f32::MIN_POSITIVE..=3e38).contains(&exposure)
        && (0..3).all(|channel| {
            (product[channel] >= f32::MIN_POSITIVE
                && product[channel] <= 3e38
                && exposed[channel] >= f32::MIN_POSITIVE)
                || (product[channel] == 0. && (a[channel] == 0. || b[channel] == 0.))
        });
    if fast {
        return exposed.min(Vec3::splat(65504.));
    }
    Vec3::from_array(std::array::from_fn(|channel| {
        let (af, ae) = fraction_exponent(a[channel]);
        let (bf, be) = fraction_exponent(b[channel]);
        let (ef, ee) = fraction_exponent(exposure);
        let mantissa = af * bf * ef;
        let exponent = (ae + be + ee).clamp(-126, 20);
        // Power-of-two scaling is exact in f64; the final cast supplies the f32
        // rounding, including a subnormal result at the lower exponent clamp.
        ((f64::from(mantissa) * 2f64.powi(exponent)) as f32).min(65504.)
    }))
}

fn shade(
    fixture: &Fixture,
    parameters: Parameters,
    lighting: Lighting,
    input: Fragment,
) -> [f32; 4] {
    let sampled = sample(&fixture.base, input.uv);
    let albedo = Vec3::new(
        fixture.material.base_color[0] * input.color[0] * sampled[0],
        fixture.material.base_color[1] * input.color[1] * sampled[1],
        fixture.material.base_color[2] * input.color[2] * sampled[2],
    )
    .clamp(Vec3::ZERO, Vec3::ONE);
    let mut normal = input.geometric;
    if parameters.flat_shaded == 0 && input.normal.length_squared() > 1e-12 {
        normal = unit(input.normal);
    }
    if normal.length_squared() < 0.5 {
        normal = Vec3::Z;
    }
    if fixture.material.double_sided && !input.source_front {
        normal = -normal;
    }

    let levels = (parameters.bands.clamp(2., 16.).floor() - 1.).max(1.);
    let low = parameters.thresholds[0].clamp(0., 0.999);
    let high = parameters.thresholds[1].clamp(low + 0.001, 1.);
    let incident = ((normal.dot(lighting.direction) - low) / (high - low)).clamp(0., 1.);
    let band = (incident * levels + 0.5).floor() / levels;
    let shadow = Vec3::from_array(parameters.shadow_tint).clamp(Vec3::ZERO, Vec3::ONE);
    // The original visibility routine returns one for this light's zero shadow
    // counts. Its unshadowed quantized incidence still retains the authored tint.
    let tint = shadow * (1. - band) + Vec3::splat(band);
    let environment_irradiance = PI * Vec3::from_array(lighting.environment);
    let mut color = exposed_product(
        albedo * environment_irradiance * (parameters.fill.max(0.) / PI),
        Vec3::ONE,
        lighting.exposure,
    );
    color += exposed_product(
        Vec3::from_array(lighting.illuminance),
        albedo * tint * (1. / PI),
        lighting.exposure,
    );
    let mut view = unit(lighting.eye - input.world);
    if view.length_squared() < 0.5 {
        view = normal;
    }
    let rim = (1. - normal.dot(view).clamp(0., 1.)).powf(parameters.rim_power.max(0.001));
    color += Vec3::from_array(parameters.rim_color).max(Vec3::ZERO) * rim;
    // Entry-level rl_finite: comparisons reject NaN and negatives before clamping.
    let rgb = color.to_array().map(|channel| {
        if channel >= 0. {
            channel.min(65504.)
        } else {
            0.
        }
    });
    [rgb[0], rgb[1], rgb[2], 1.]
}

/// Original toon surface/shade with unclustered, unshadowed directional lighting.
///
/// GPU inputs must use `uniform_words` and `light_words`, empty decals, disabled
/// transmission, disabled atmosphere surface flags, and `LcFrame.info.y == 0`.
/// The real GPU shader retains those services and all their bound resources.
/// Flat normals use the triangle plane equivalent to the fragment derivative cross;
/// the rasterizer's inherited unclipped, single-sample fixture limits still apply.
pub fn rasterize(fixture: &Fixture, parameters: Parameters, lighting: Lighting) -> Rasterized {
    mesh_reference::rasterize_with(fixture, |input| shade(fixture, parameters, lighting, input))
}

/// Render RGBA32Float toon color while dropping the CPU depth stream.
pub fn render(fixture: &Fixture, parameters: Parameters, lighting: Lighting) -> Image {
    rasterize(fixture, parameters, lighting).image
}
