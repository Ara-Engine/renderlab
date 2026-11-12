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
/// the eye; its `shade` is then added on top (the standard program: Fresnel, its diffuse
/// share scaled by coverage, and a base-colour tint only for glass, an `ior` above one; at
/// an `ior` of one it blends as glTF BLEND does). Other programs blend by coverage:
/// `coverage * shade + (1 - coverage) * behind`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transparency {
    /// Coverage factor in [0, 1], multiplied with the base-colour alpha.
    pub opacity: f32,
    /// Index of refraction in [1, 3]: Fresnel reflectance and the thin-slab refraction. Above
    /// one the standard program treats the pane as coloured glass (its base colour tints the
    /// light it passes, and its sun shadow); one is a plain blended film, untinted.
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

/// Where an emitter's particles spawn, around its `position`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SpawnShape {
    /// Uniformly inside a ball of `spawn_radius`.
    #[default]
    Sphere,
    /// Uniformly inside an axis-aligned box of these half extents (metres).
    Box { half_extent: Vec3 },
    /// Inside a cone with its tip at `position`, opening along `axis` (whose length is its
    /// height) to a base `spawn_radius` wide; denser toward the base.
    Cone { axis: Vec3 },
}

/// A key of a colour-over-life curve (`EmitterDescriptor::color_keys`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorKey {
    /// Fraction of life, in [0, 1], increasing from key to key.
    pub t: f32,
    /// Linear RGBA as `EmitterDescriptor::color`.
    pub color: [f32; 4],
    /// Emitted radiance as `EmitterDescriptor::emission`.
    pub emission: [f32; 3],
}

/// A key of a size-over-life curve (`EmitterDescriptor::size_keys`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SizeKey {
    /// Fraction of life, in [0, 1], increasing from key to key.
    pub t: f32,
    /// Billboard width in metres.
    pub size: f32,
}

/// How an emitter's billboards are oriented (`EmitterDescriptor::alignment`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SpriteAlignment {
    /// Facing the camera, upright on screen.
    #[default]
    Camera,
    /// Facing the camera, the sprite's vertical axis along the particle's velocity as
    /// seen on screen, `stretch` (at least 1) times as long as it is wide.
    Velocity { stretch: f32 },
    /// Lying in the plane through the particle perpendicular to `normal` (nonzero), as a
    /// ripple on the ground.
    Plane { normal: Vec3 },
}

/// A sprite that is a sheet of animation frames (`EmitterDescriptor::flipbook`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flipbook {
    /// Columns and rows of frames, read left to right, top to bottom.
    pub grid: [u32; 2],
    /// Frames per second of particle age, looping; neighbouring frames cross-fade.
    pub fps: f32,
    /// Play the frames once over each particle's life instead (`fps` is then ignored).
    pub over_life: bool,
}

/// Keys a colour or size curve holds at most.
pub const MAX_CURVE_KEYS: usize = 4;

/// Accelerations beyond `EmitterDescriptor::acceleration`, applied every step in this
/// order after drag. All zero by default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Forces {
    /// Divergence-free curl noise: spatial frequency (1/m) and strength (m/s²). The field
    /// is the curl of a sinusoidal potential, phase-shifted per particle.
    pub curl: [f32; 2],
    /// Where the curl field is centred: particles sample it at their position minus this
    /// (an embedder's effect-local space). Zero, the default, is world space.
    pub curl_origin: Vec3,
    /// The curl phase per second of a particle's birth time (rad/s): each particle samples
    /// the field shifted by its birth time, counted from the emitter's start (prewarm
    /// included), times this, so particles born close together drift alike. Zero, the
    /// default, phases particles by their spawn ordinal, 0.013 rad apart.
    pub curl_phase_rate: f32,
    /// Swirl about the line through `vortex_center` along `vortex_axis`; the axis's length
    /// is the strength (1/s²), its direction the sense of rotation.
    pub vortex_center: Vec3,
    pub vortex_axis: Vec3,
    /// Softened inverse-square pull toward `attractor`: strength (m³/s²) and softening
    /// (m², > 0) in `attractor_strength`.
    pub attractor: Vec3,
    pub attractor_strength: [f32; 2],
}

/// A plane particles bounce off (`EmitterDescriptor::collision`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaneCollision {
    /// Outward normal, nonzero; normalized on use.
    pub normal: Vec3,
    /// The plane is `dot(normal, p) = offset` with the normal normalized.
    pub offset: f32,
    /// Share of the inward speed kept after a bounce, in [0, 1].
    pub restitution: f32,
}

/// A GPU particle emitter (`Renderer::try_create_emitter`). Particles spawn in a sphere,
/// box or cone, integrate velocity under constant acceleration, drag and optional
/// forces, may bounce off a plane, and interpolate size, colour and emission over their
/// life (linearly from birth to death, or along curves of up to `MAX_CURVE_KEYS` keys). They are lit by each directional light with a
/// world-space shadow lookup and by the environment's irradiance, fade against opaque depth
/// (soft particles), and draw as camera-facing billboards sorted back to front on the GPU.
#[derive(Clone, Copy)]
pub struct EmitterDescriptor<'a> {
    /// Centre of the spawn sphere or box, or the cone's tip, metres.
    pub position: Vec3,
    pub spawn_radius: f32,
    pub spawn_shape: SpawnShape,
    /// New particles per second.
    pub spawn_rate: f32,
    /// Seconds, uniform in [min, max].
    pub lifetime: [f32; 2],
    /// Initial velocity (m/s), plus a uniform ball of `velocity_spread` radius and a
    /// uniform box of `velocity_box` half extents (per-axis variation).
    pub velocity: Vec3,
    pub velocity_spread: f32,
    pub velocity_box: Vec3,
    /// Constant acceleration (m/s²): gravity, buoyancy or wind.
    pub acceleration: Vec3,
    /// Velocity damping per second.
    pub drag: f32,
    pub forces: Forces,
    pub collision: Option<PlaneCollision>,
    /// Billboard width in metres at birth and at death.
    pub size: [f32; 2],
    /// Linear RGBA at birth and at death: rgb is the scattering albedo, a the opacity.
    pub color: [[f32; 4]; 2],
    /// Emitted radiance (unexposed, the lights' units per steradian) at birth and death.
    pub emission: [[f32; 3]; 2],
    /// Colour and emission over life, replacing `color` and `emission` when not empty.
    pub color_keys: &'a [ColorKey],
    /// Size over life, replacing `size` when not empty.
    pub size_keys: &'a [SizeKey],
    /// Fractions of life spent fading in and out.
    pub fade: [f32; 2],
    /// Soft-particle fade distance against opaque depth, metres.
    pub soft_distance: f32,
    /// Optional sprite: rgb multiplies colour, alpha multiplies opacity.
    pub texture: Option<&'a Texture>,
    /// The sprite as a sheet of frames played over each particle's life.
    pub flipbook: Option<Flipbook>,
    pub alignment: SpriteAlignment,
    /// Sprite height over width (1: square), along the sprite's vertical axis.
    pub aspect: f32,
    /// Turn each particle's sprite by its own random angle (breaks up repeating smoke).
    pub random_rotation: bool,
    /// Ring capacity, at most `MAX_PARTICLES`; spawning replaces the oldest particle.
    /// `spawn_rate * lifetime[1]` keeps every particle to the end of its life.
    pub capacity: u32,
    pub seed: u32,
    /// Seconds the emitter has already been running when it is created: its first frame
    /// spawns the particles of that span (up to the longest lifetime) at their ages, moved
    /// by velocity, acceleration and drag. Ignored by updates.
    pub prewarm: f32,
    /// Particles spawned at once in the emitter's first frame, besides its rate.
    pub burst: u32,
}
impl Default for EmitterDescriptor<'_> {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            spawn_radius: 0.1,
            spawn_shape: SpawnShape::Sphere,
            spawn_rate: 64.0,
            lifetime: [2.0, 3.0],
            velocity: Vec3::new(0.0, 0.0, 0.6),
            velocity_spread: 0.2,
            velocity_box: Vec3::ZERO,
            acceleration: Vec3::ZERO,
            drag: 0.2,
            forces: Forces::default(),
            collision: None,
            size: [0.2, 0.8],
            color: [[0.6, 0.6, 0.6, 0.5], [0.5, 0.5, 0.5, 0.0]],
            emission: [[0.0; 3]; 2],
            color_keys: &[],
            size_keys: &[],
            fade: [0.1, 0.4],
            soft_distance: 0.3,
            texture: None,
            flipbook: None,
            alignment: SpriteAlignment::Camera,
            aspect: 1.0,
            random_rotation: false,
            capacity: 256,
            seed: 1,
            prewarm: 0.0,
            burst: 0,
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
        let shape = match self.spawn_shape {
            SpawnShape::Sphere => true,
            SpawnShape::Box { half_extent } => {
                half_extent.is_finite() && half_extent.min_element() >= 0.0
            }
            SpawnShape::Cone { axis } => axis.is_finite(),
        };
        let f = &self.forces;
        let forces = [
            f.vortex_center,
            f.vortex_axis,
            f.attractor,
            self.velocity_box,
        ]
        .iter()
        .all(|v| v.is_finite())
            && self.velocity_box.min_element() >= 0.0
            && f.curl
                .iter()
                .chain(&f.attractor_strength)
                .chain(&[f.curl_phase_rate])
                .all(|v| v.is_finite())
            && f.curl_origin.is_finite()
            && f.curl[0] >= 0.0
            && (f.attractor_strength[0] == 0.0 || f.attractor_strength[1] > 0.0);
        let collision = self.collision.is_none_or(|c| {
            c.normal.is_finite()
                && c.normal.length_squared() > 1e-12
                && c.offset.is_finite()
                && (0.0..=1.0).contains(&c.restitution)
        });
        fn ordered(mut t: impl Iterator<Item = f32>) -> bool {
            let mut last = -1.0f32;
            t.all(|t| {
                let ok = t.is_finite() && (0.0..=1.0).contains(&t) && t > last;
                last = t;
                ok
            })
        }
        let curves = self.color_keys.len() <= MAX_CURVE_KEYS
            && self.size_keys.len() <= MAX_CURVE_KEYS
            && ordered(self.color_keys.iter().map(|k| k.t))
            && ordered(self.size_keys.iter().map(|k| k.t))
            && self.color_keys.iter().all(|k| {
                k.color
                    .iter()
                    .chain(&k.emission)
                    .all(|v| v.is_finite() && *v >= 0.0)
            })
            && self
                .size_keys
                .iter()
                .all(|k| k.size.is_finite() && k.size >= 0.0);
        let sprite = self.flipbook.is_none_or(|f| {
            (1..=64).contains(&f.grid[0])
                && (1..=64).contains(&f.grid[1])
                && f.fps.is_finite()
                && f.fps >= 0.0
        }) && match self.alignment {
            SpriteAlignment::Camera => true,
            SpriteAlignment::Velocity { stretch } => stretch.is_finite() && stretch >= 1.0,
            SpriteAlignment::Plane { normal } => {
                normal.is_finite() && normal.length_squared() > 1e-12
            }
        };
        let sprite = sprite && self.aspect.is_finite() && self.aspect > 0.0;
        if !(shape && forces && collision && curves && sprite)
            || !(self.prewarm.is_finite() && self.prewarm >= 0.0)
        {
            return Err(crate::Error::InvalidInput(
                "emitter shape, forces, collision, prewarm, curves and sprite need finite values: a 1..=64 flipbook grid, a stretch of at least 1, a nonzero plane normal, nonnegative extents and frequency, a nonzero collision normal, restitution in [0,1], positive attractor softening, and at most MAX_CURVE_KEYS keys at increasing times in [0,1]",
            ));
        }
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
