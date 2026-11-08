//! TP1 public transparency and particle API (docs/m7-tp1.md). Transparent materials and
//! particle emitters draw in one forward pass after opaque shading and temporal
//! reconstruction, sorted back to front by object, on every primary path. Alpha-tested
//! (cutout) foliage is ordinary opaque geometry cut at load (`crate::cutout`).
use crate::{Texture, Vec3};
use std::sync::Arc;

/// An alpha-blended material (`MaterialDescriptor::transparency`), drawn by the transparent
/// pass instead of every opaque path. TP2 adds a coloured coverage sun shadow on both
/// paths; it remains outside the world radiance cache's receiver geometry. Coverage is `opacity * base-colour alpha * texture alpha * vertex
/// alpha`. A program declaring `fn transmittance(s: Surface, input: SurfaceInput) ->
/// vec3<f32>` returns the per-channel fraction of the light behind it that it passes to
/// the eye; its `shade` is then added on top (the standard program: Fresnel, tint and its
/// diffuse share scaled by coverage). Other programs blend by coverage:
/// `coverage * shade + (1 - coverage) * behind`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transparency {
    /// Coverage factor in [0, 1], multiplied with the base-colour alpha.
    pub opacity: f32,
    /// Index of refraction in [1, 3]: Fresnel reflectance and the thin-slab refraction.
    pub ior: f32,
    /// Thin-slab thickness in metres: the lateral shift of the light seen through it.
    /// Zero keeps the background unshifted.
    pub thickness: f32,
}
impl Default for Transparency {
    fn default() -> Self {
        Self {
            opacity: 0.1,
            ior: 1.5,
            thickness: 0.01,
        }
    }
}
impl Transparency {
    pub(crate) fn valid(&self) -> bool {
        (0.0..=1.0).contains(&self.opacity)
            && (1.0..=3.0).contains(&self.ior)
            && self.thickness.is_finite()
            && (0.0..=1.0).contains(&self.thickness)
    }
    /// The material word the shaders unpack (`transparent_opacity`, `transparent_ior`): opacity
    /// and IOR as two halves. Opaque materials store zero, which no valid IOR packs to.
    pub(crate) fn word(material: Option<&Self>) -> u32 {
        material.map_or(0, |t| {
            u32::from(f16_bits(t.opacity)) | (u32::from(f16_bits(t.ior)) << 16)
        })
    }
}

fn f16_bits(value: f32) -> u16 {
    // Values here are in [0, 3]: normal halves or zero, round to nearest even.
    if value == 0.0 {
        return 0;
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        return 0;
    }
    let mantissa = bits & 0x7f_ffff;
    let mut half = ((exponent as u32) << 10) | (mantissa >> 13);
    let rest = mantissa & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && half & 1 == 1) {
        half += 1;
    }
    half as u16
}

/// A GPU particle emitter (`Renderer::try_create_emitter`). Particles spawn in a sphere,
/// integrate velocity under constant acceleration and drag, and interpolate size, colour
/// and emission linearly over their life. They are lit by each directional light with a
/// world-space shadow lookup and by the environment's irradiance, fade against opaque depth
/// (soft particles), and draw as camera-facing billboards sorted back to front on the GPU.
#[derive(Clone, Copy)]
pub struct EmitterDescriptor<'a> {
    /// Centre of the spawn sphere, metres.
    pub position: Vec3,
    pub spawn_radius: f32,
    /// New particles per second.
    pub spawn_rate: f32,
    /// Seconds, uniform in [min, max].
    pub lifetime: [f32; 2],
    /// Initial velocity (m/s), plus a uniform ball of `velocity_spread` radius.
    pub velocity: Vec3,
    pub velocity_spread: f32,
    /// Constant acceleration (m/s²): gravity, buoyancy or wind.
    pub acceleration: Vec3,
    /// Velocity damping per second.
    pub drag: f32,
    /// Billboard width in metres at birth and at death.
    pub size: [f32; 2],
    /// Linear RGBA at birth and at death: rgb is the scattering albedo, a the opacity.
    pub color: [[f32; 4]; 2],
    /// Emitted radiance (unexposed, the lights' units per steradian) at birth and death.
    pub emission: [[f32; 3]; 2],
    /// Fractions of life spent fading in and out.
    pub fade: [f32; 2],
    /// Soft-particle fade distance against opaque depth, metres.
    pub soft_distance: f32,
    /// Optional sprite: rgb multiplies colour, alpha multiplies opacity.
    pub texture: Option<&'a Texture>,
    /// Ring capacity, at most `MAX_PARTICLES`; spawning replaces the oldest particle.
    /// `spawn_rate * lifetime[1]` keeps every particle to the end of its life.
    pub capacity: u32,
    pub seed: u32,
}
impl Default for EmitterDescriptor<'_> {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            spawn_radius: 0.1,
            spawn_rate: 64.0,
            lifetime: [2.0, 3.0],
            velocity: Vec3::new(0.0, 0.0, 0.6),
            velocity_spread: 0.2,
            acceleration: Vec3::ZERO,
            drag: 0.2,
            size: [0.2, 0.8],
            color: [[0.6, 0.6, 0.6, 0.5], [0.5, 0.5, 0.5, 0.0]],
            emission: [[0.0; 3]; 2],
            fade: [0.1, 0.4],
            soft_distance: 0.3,
            texture: None,
            capacity: 256,
            seed: 1,
        }
    }
}
/// Largest emitter capacity: the GPU sort runs in one workgroup's shared memory.
pub const MAX_PARTICLES: u32 = 2048;

impl EmitterDescriptor<'_> {
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        let finite = [self.position, self.velocity, self.acceleration]
            .iter()
            .all(|v| v.is_finite())
            && [
                self.spawn_radius,
                self.spawn_rate,
                self.velocity_spread,
                self.drag,
                self.soft_distance,
            ]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            && self
                .lifetime
                .iter()
                .chain(&self.size)
                .chain(&self.fade)
                .all(|v| v.is_finite() && *v >= 0.0)
            && self
                .color
                .iter()
                .flatten()
                .chain(self.emission.iter().flatten())
                .all(|v| v.is_finite() && *v >= 0.0);
        if !finite
            || self.lifetime[0] <= 0.0
            || self.lifetime[1] < self.lifetime[0]
            || self.fade[0] + self.fade[1] > 1.0
            || !(1..=MAX_PARTICLES).contains(&self.capacity)
        {
            return Err(crate::Error::InvalidInput(
                "emitter needs finite nonnegative values, 0 < lifetime min <= max, fades summing to at most 1, and 1..=MAX_PARTICLES capacity",
            ));
        }
        Ok(())
    }
}

/// Caller-owned emitter: it draws in every lit frame of its renderer until dropped.
pub struct Emitter(pub(crate) Arc<EmitterShared>);
pub(crate) struct EmitterShared {
    pub id: u64,
    pub renderer: Arc<()>,
}

/// Transparent-pass counters of the last encoded frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransparencyStats {
    pub transparent_draws: u32,
    pub emitters: u32,
    pub particle_capacity: u32,
    /// Times the transparent receivers' sun shadow map was rendered.
    pub shadow_map_renders: u64,
    /// Lit frames that ran the transparent pass.
    pub frames: u64,
}
