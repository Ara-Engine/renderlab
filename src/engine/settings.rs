//! Engine-level lighting, sky and quality settings, and how they map onto the renderer.
//!
//! These types are deliberately coarse: each names a choice a game makes, not an
//! estimator's tuning. The renderer's own tuning structs (`IndirectSettings`,
//! `Reconstruction`, `VirtualShadowSettings`, ...) change as features are measured; the
//! engine keeps these stable and maps them to each path's measured configuration.
use super::{EnvironmentId, RenderPath};
use crate::{
    Atmosphere, AtmosphereMedium, ColourLut, Error, HeightFog, IndirectSettings, Light, Lighting,
    LocalShadows, PostSettings, Reconstruction, RenderScale, Renderer, ShadowRequest, Shadows,
    SoftShadows, Vec3, VirtualLookup, VirtualShadowSettings, VolumetricFog,
};

/// The scene's sun: its one shadowed directional light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// The direction sunlight travels (from the sun toward the ground). Normalized on use.
    pub travel: Vec3,
    /// Illuminance on a surface facing the sun, in lux per linear Rec.709 channel. Under
    /// [`Sky::Atmosphere`] it is the illuminance outside the atmosphere (about 120,000 lux),
    /// which the atmosphere's transmittance dims and colours toward the horizon; under the
    /// other skies it reaches surfaces as given (clear midday is about 100,000 lux).
    pub illuminance: [f32; 3],
    /// Angular radius of the sun disk, in radians, in [0, 0.1]. It sets penumbra width:
    /// 0 gives hard shadows. The default, 0.04, is the measured shipping configuration on
    /// both paths (the real sun is about 0.0047).
    pub angular_radius: f32,
}
impl Sun {
    /// A default sun (illuminance, disk) at `elevation` above the horizon and `azimuth`
    /// clockwise from north seen from above, both in radians. World axes: +Y up, -Z north,
    /// +X east, so azimuth pi/2 is a sun in the east. Changing the sun every frame is cheap
    /// (AT2 updates the procedural sky in place and relights GI progressively).
    pub fn at(elevation: f32, azimuth: f32) -> Self {
        let toward = Vec3::new(
            elevation.cos() * azimuth.sin(),
            elevation.sin(),
            -elevation.cos() * azimuth.cos(),
        );
        Self {
            travel: -toward,
            ..Self::default()
        }
    }
    /// The sun at local solar time `hour` (0-24) on an equinox day at `latitude`
    /// (radians, positive north): rising in the east at 6, highest toward the equator at 12
    /// and setting in the west at 18. None while it is below the horizon. Axes as for
    /// [`Sun::at`].
    pub fn at_hour(hour: f32, latitude: f32) -> Option<Self> {
        let angle = (hour - 12.0) * std::f32::consts::PI / 12.0;
        // Toward the sun with zero declination: east -sin H, north -sin(lat) cos H,
        // up cos(lat) cos H.
        let up = latitude.cos() * angle.cos();
        (up > 0.0 && hour.is_finite() && latitude.is_finite()).then(|| {
            let toward = Vec3::new(-angle.sin(), up, latitude.sin() * angle.cos());
            Self {
                travel: -toward.normalize(),
                ..Self::default()
            }
        })
    }
    /// The sun's angle above the horizon, radians.
    pub fn elevation(&self) -> f32 {
        (-self.travel.normalize().y).asin()
    }
}
impl Default for Sun {
    fn default() -> Self {
        Self {
            travel: Vec3::new(-0.4, -0.8, -0.3),
            illuminance: [120_000.0; 3],
            angular_radius: 0.04,
        }
    }
}

/// What lights the scene from infinitely far away: sky light, reflections of the sky, and
/// the background seen where camera rays miss all geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Sky {
    /// The physical atmosphere (AT1, docs/m7-at1.md), the default: a Hillaire-2020 sky
    /// computed from the medium and the sun, which lights the scene through a procedural
    /// environment (image-based light and the GI cache take their sky from it), is the
    /// background with a limb-darkened sun disk, and colours the sun through its
    /// transmittance. `AtmosphereMedium::default()` is Earth.
    Atmosphere(AtmosphereMedium),
    /// Uniform radiance in cd/m² (linear Rec.709) from every direction. It is also the
    /// background colour.
    Uniform([f32; 3]),
    /// A prepared HDR environment ([`Engine::create_environment`](super::Engine::create_environment))
    /// that lights the scene.
    Map {
        map: EnvironmentId,
        /// Rotation about +Y, in radians.
        rotation_y: f32,
        /// Radiance scale, nonnegative.
        intensity: f32,
        background: Background,
    },
}
impl Default for Sky {
    fn default() -> Self {
        Self::Atmosphere(AtmosphereMedium::default())
    }
}

/// What camera rays that miss all geometry show under [`Sky::Map`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Background {
    /// A constant radiance in cd/m², exposed like the scene.
    Color([f32; 3]),
    /// The map itself along each pixel's ray, exposed and scaled like the light it gives
    /// (TR1b's visible sky, `Renderer::set_visible_environment`).
    Sky,
    /// The map along each pixel's ray at its own radiance scale (nonnegative), in place
    /// of the sky's `intensity`: a sky that lights the scene more or less brightly than it
    /// looks (an engine's sky-light strength). Reflections still see the lighting scale.
    ScaledSky(f32),
}

/// How scene luminance maps to display values before tone mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Exposure {
    /// Scene luminance is scaled by 2^stops. With sunlight in lux, about -14 to -16.
    Manual { stops: f32 },
}
impl Default for Exposure {
    fn default() -> Self {
        Self::Manual { stops: 0.0 }
    }
}

/// Estimator presets. Every preset renders the same features; they differ in how exactly
/// each pixel's light is integrated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Quality {
    /// Each path's measured real-time configuration: on hardware RT, traced primaries
    /// with eight sun-disk directions per pixel (`SoftShadows::Interactive`); without RT,
    /// the visibility buffer with the cached sun clipmap (`SoftShadows::Virtual`, Surface
    /// lookup).
    #[default]
    Realtime,
    /// Exact sun visibility, for stills, captures and A/B checks: all 256 disk directions
    /// per pixel on RT (`SoftShadows::Exact`), and the exact triangle-tested map lookup
    /// without RT (`VirtualLookup::Exact`). Several times the shadow cost; not a
    /// real-time preset.
    Reference,
}

/// How one view looks (`ViewDesc::look`): the exposure, presentation, estimator quality and
/// reflections. Everything that lights the world is engine-wide [`Settings`].
#[derive(Clone, Debug, PartialEq)]
pub struct ViewLook {
    pub exposure: Exposure,
    pub quality: Quality,
    /// Presentation after reconstruction (PP1): bloom, motion blur, optional depth of field,
    /// the tone mapper, white balance, contrast, saturation, vignette, grain and the colour
    /// LUT's strength. On by default; `PostSettings { enabled: false, .. }` presents the
    /// plain tone-mapped image.
    pub post: PostSettings,
    /// The display-linear 3D colour LUT (PP1), applied at `post.lut_strength`.
    pub colour_lut: Option<ColourLut>,
    /// Traced reflections on both paths (M7-R1 on HardwareRt, R1n's software rays on NonRt)
    /// for programs that declare `reflection_lobe`: on by default up to roughness 0.5.
    /// `ReflectionSettings::off()` leaves every program its untraced estimate.
    pub reflections: crate::ReflectionSettings,
}
impl Default for ViewLook {
    fn default() -> Self {
        Self {
            exposure: Exposure::default(),
            quality: Quality::Realtime,
            post: PostSettings::default(),
            colour_lut: None,
            reflections: crate::ReflectionSettings::default(),
        }
    }
}

/// Engine-wide choices: what lights the world and which features run. Construct with
/// `..Default::default()`: new fields are added with defaults as features land.
/// Defaults follow each feature unit's default ruling (docs/api.md). How each view looks
/// is its [`ViewLook`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Indirect light: sky visibility and bounce light from the world radiance cache
    /// (M7 G2). On by default (G2b). Off, programs read the unoccluded sky.
    pub global_illumination: bool,
    /// Temporal reconstruction (TR1): each frame samples at a new sub-pixel offset and
    /// accumulates history, antialiasing edges and textures. On by default (TR1b). Each
    /// view's internal resolution is its `ViewDesc::render_scale`.
    pub temporal_reconstruction: bool,
    /// Atmospheric scattering along every camera ray to a surface (AT1): distant surfaces
    /// fade toward the sky. On by default; it uses the physical atmosphere's medium
    /// whatever the [`Sky`].
    pub aerial_perspective: bool,
    /// Analytic exponential height fog on every camera ray, lit by the sun and the sky
    /// mean (AT1). On by default (`HeightFog::default()`: about 2 % extinction per 100 m
    /// near y = 0); None turns it off.
    pub height_fog: Option<HeightFog>,
    /// Froxel volumetric fog over the first `distance` metres, with sun shafts (VF1).
    /// On by default with a subtle 0.001/m haze. Sun visibility uses hardware rays or
    /// the non-RT path's cached shadow depth; None disables it.
    pub volumetric_fog: Option<VolumetricFog>,
    /// Local visibility (LC3): every contributing clustered light on `HardwareRt`, or
    /// up to `count` cached atlas lights on `NonRt` (default/max 128). `resolution` is
    /// the maximum map-face size. Zero count disables automatic local shadows on both
    /// paths; any positive count enables all-light RT. Unselected map lights still light
    /// the scene. Each light's source radius controls its soft penumbra.
    pub local_shadows: LocalShadows,
    /// How brightly the physical atmosphere's sky lights the scene, relative to how it
    /// looks (an engine's sky-light strength): it scales the procedural environment the
    /// atmosphere lights through (image-based light, GI and reflected sky), not the sky
    /// drawn, the sun or the fog. 1, the default, is physical; finite and nonnegative.
    /// [`Sky::Map`] has its own `intensity` and `Background::ScaledSky`.
    pub sky_light: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            global_illumination: true,
            temporal_reconstruction: true,
            aerial_perspective: true,
            height_fog: Some(HeightFog::default()),
            volumetric_fog: Some(VolumetricFog::default()),
            local_shadows: LocalShadows::default(),
            sky_light: 1.0,
        }
    }
}

/// The renderer's light vector and shadow requests for a sun and local lights.
/// The sun, when present, is light 0 and the only explicit shadow request. Local lights
/// use the automatic local-shadow pool; their source radii pass through unchanged.
pub(crate) fn lighting(
    sun: Option<Sun>,
    lights: &[Light],
    sky: Sky,
    exposure: Exposure,
) -> Result<(Lighting, Shadows), Error> {
    if lights
        .iter()
        .any(|l| matches!(l, Light::Directional { .. }))
    {
        return Err(Error::InvalidInput(
            "directional light belongs in the sun (Engine::set_sun)",
        ));
    }
    let Exposure::Manual { stops } = exposure;
    let environment = match sky {
        // The atmosphere's procedural environment lights the scene and its sky fills misses.
        Sky::Atmosphere(_) => [0.0; 3],
        Sky::Uniform(radiance) => radiance,
        // A visible-sky background clears to black wherever the map is not drawn.
        Sky::Map { background, .. } => match background {
            Background::Color(radiance) => radiance,
            Background::Sky | Background::ScaledSky(_) => [0.0; 3],
        },
    };
    let mut all = Vec::with_capacity(lights.len() + 1);
    let mut requests = Vec::new();
    if let Some(sun) = sun {
        if !(0.0..=0.1).contains(&sun.angular_radius) {
            return Err(Error::InvalidInput(
                "sun angular_radius must be in [0, 0.1]",
            ));
        }
        // The soft sun is light 0 and the only shadow request; local lights follow it
        // unrequested and LC3 supplies ray or cached-atlas visibility (docs/m7-lc3.md).
        all.push(Light::Directional {
            travel: sun.travel,
            lux: sun.illuminance,
        });
        // Cascade ends for the raster shadow maps the paths still build for coverage;
        // traced and Virtual visibility are not bounded by them (the shipping split set).
        requests.push(ShadowRequest::Directional {
            split_ends: vec![4.0, 12.0, 40.0],
        });
        requests.extend(lights.iter().map(|_| ShadowRequest::None));
    }
    all.extend_from_slice(lights);
    let shadows = Shadows {
        requests,
        directional_disk_angle: sun.map_or(0.0, |s| s.angular_radius),
        ..Default::default()
    };
    Ok((
        Lighting {
            lights: all,
            environment,
            exposure_stops: stops,
        },
        shadows,
    ))
}

/// The renderer's atmosphere for a sky choice and the frame's media settings. Only
/// [`Sky::Atmosphere`] turns on the physical sky, its procedural environment and the sun's
/// transmittance; aerial perspective and fog follow `settings` under every sky.
pub(crate) fn atmosphere(sky: Sky, settings: &Settings) -> Atmosphere {
    let medium = match sky {
        Sky::Atmosphere(medium) => Some(medium),
        _ => None,
    };
    let physical = medium.is_some();
    Atmosphere {
        sky: physical,
        sun_disk: physical,
        aerial_perspective: settings.aerial_perspective,
        sun_transmittance: physical,
        environment: physical,
        medium: medium.unwrap_or_default(),
        height_fog: settings.height_fog,
        volumetric_fog: settings.volumetric_fog,
    }
}

/// Measurement ablations (P-SHOW2, docs/p-show2.md): `RENDERLAB_ABLATE` lists features
/// to switch off whatever the embedder asks for, comma-separated: `gi`, `reflections`,
/// `local_shadows`, `media` (aerial perspective, height and volumetric fog), `post`, `tr1`.
/// Lets a harness split a fused pass's cost without changing the embedder.
pub(crate) fn ablated(mut s: Settings) -> Settings {
    let Ok(list) = std::env::var("RENDERLAB_ABLATE") else {
        return s;
    };
    for feature in list.split(',').map(str::trim) {
        match feature {
            "gi" => s.global_illumination = false,
            "local_shadows" => s.local_shadows.count = 0,
            "media" => {
                s.aerial_perspective = false;
                s.height_fog = None;
                s.volumetric_fog = None;
            }
            "tr1" => s.temporal_reconstruction = false,
            _ => {}
        }
    }
    s
}

/// Measurement overrides of the GI cache's tuning (U-SPECK, docs/speck.md): `RENDERLAB_GI`
/// lists `key=value` pairs separated by `;`, applied over the engine's own choice whatever
/// the embedder asks for, so a defect seen through an embedder can be bisected there:
/// `lookup=nearest|trilinear`, `filter`, `filter_edge`, `rotate`, `bounce`, `directions`,
/// `cell_pixels`, `level_blend`, `gathers`, `gather_cells`, `claim_cells`, `near_light_rays`,
/// `slots_log2`, `evict_frames`, `pressure_eviction`, `hit_cone_cells`, `gather_pixels_first`,
/// `mover_hit_levels`.
/// Unset, it changes nothing. An unknown key, a pair without `=`
/// or a value that does not parse is an error, named on stderr: a mistyped switch must not
/// pass for a measurement of the default.
fn indirect_measured(mut s: IndirectSettings) -> Result<IndirectSettings, Error> {
    let Ok(list) = std::env::var("RENDERLAB_GI") else {
        return Ok(s);
    };
    for pair in list.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((key, value)) = pair.split_once('=') else {
            eprintln!("RENDERLAB_GI: `{pair}` is not key=value");
            return Err(Error::InvalidInput("RENDERLAB_GI: a pair is not key=value"));
        };
        let number = value.parse::<u32>().ok();
        let real = value.parse::<f32>().ok();
        match (key, number, real) {
            ("lookup", ..) if value == "nearest" => s.lookup = crate::IndirectLookup::Nearest,
            ("lookup", ..) if value == "trilinear" => s.lookup = crate::IndirectLookup::Trilinear,
            ("filter", Some(v), _) => s.filter = v,
            ("filter_edge", _, Some(v)) => s.filter_edge = v,
            ("rotate", Some(v), _) => s.rotate_directions = v != 0,
            ("bounce", Some(v), _) => s.bounce = v != 0,
            ("directions", Some(v), _) => s.directions = v,
            ("cell_pixels", _, Some(v)) => s.cell_pixels = v,
            ("level_blend", _, Some(v)) => s.level_blend = v,
            ("gathers", Some(v), _) => s.gathers = v,
            ("gather_cells", Some(v), _) => s.gather_cells = v,
            ("claim_cells", Some(v), _) => s.claim_cells = v,
            ("near_light_rays", Some(v), _) => s.near_light_rays = v,
            ("slots_log2", Some(v), _) => s.slots_log2 = v,
            ("evict_frames", Some(v), _) => s.evict_frames = v,
            ("pressure_eviction", Some(v), _) => s.pressure_eviction = v != 0,
            ("hit_cone_cells", _, Some(v)) => s.hit_cone_cells = v,
            ("gather_pixels_first", Some(v), _) => s.gather_pixels_first = v != 0,
            ("mover_hit_levels", Some(v), _) => s.mover_hit_levels = v,
            _ => {
                eprintln!("RENDERLAB_GI: unknown key or unreadable value in `{pair}`");
                return Err(Error::InvalidInput(
                    "RENDERLAB_GI: unknown key or unreadable value",
                ));
            }
        }
    }
    Ok(s)
}

/// With `RENDERLAB_GI_LOG=<file>` set, appends one JSON line of the GI cache's counters after
/// each frame the engine renders (a blocking readback: measurement only).
pub(crate) fn log_indirect(r: &mut Renderer) -> Result<(), Error> {
    use std::io::Write;
    let Some(path) = std::env::var_os("RENDERLAB_GI_LOG") else {
        return Ok(());
    };
    let Some(c) = r.indirect_counters()? else {
        return Ok(());
    };
    let line = format!(
        "{{\"misses\":{},\"miss_overflow\":{},\"created\":{},\"full\":{},\"lost\":{},\"orphans\":{},\"traces\":{},\"invalidated\":{},\"splits\":{},\"local_overflow\":{},\"shades\":{},\"gathers\":{},\"gather_rays\":{},\"gather_misses\":{},\"gather_requests\":{},\"gather_deferred\":{},\"claim_deferred\":{},\"bounce_queued\":{},\"evicted\":{},\"evicted_recent\":{},\"full_pixel\":{},\"table_bytes\":{}}}\n",
        c.misses,
        c.miss_overflow,
        c.created,
        c.full,
        c.lost,
        c.orphans,
        c.traces,
        c.invalidated,
        c.splits,
        c.local_overflow,
        c.shades,
        c.gathers,
        c.gather_rays,
        c.gather_misses,
        c.gather_requests,
        c.gather_deferred,
        c.claim_deferred,
        c.bounce_queued,
        c.evicted,
        c.evicted_recent,
        c.full_pixel,
        r.indirect_stats().table_bytes,
    );
    // A log that cannot be written is not a rendering error.
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
    Ok(())
}

/// `RENDERLAB_ABLATE`'s `post` and `reflections` switch those off in every view's look.
pub(crate) fn ablated_look(mut look: ViewLook) -> ViewLook {
    let Ok(list) = std::env::var("RENDERLAB_ABLATE") else {
        return look;
    };
    for feature in list.split(',').map(str::trim) {
        match feature {
            "reflections" => look.reflections = crate::ReflectionSettings::off(),
            "post" => look.post.enabled = false,
            _ => {}
        }
    }
    look
}

/// Applies the engine-wide `settings` to the renderer's estimators. Called only on change:
/// re-applying identical GI settings is cheap, but the engine does not rely on it.
pub(crate) fn apply(r: &mut Renderer, settings: &Settings) -> Result<(), Error> {
    r.set_local_shadows(settings.local_shadows)?;
    r.set_indirect(if settings.global_illumination {
        indirect_measured(IndirectSettings::default())?
    } else {
        IndirectSettings::off()
    })
}

/// Applies a view's look, and its render scale under the engine's `settings`, to the
/// renderer on `path`. Validates everything before the first setter.
pub(crate) fn apply_look(
    r: &mut Renderer,
    path: RenderPath,
    settings: &Settings,
    look: &ViewLook,
    scale: RenderScale,
) -> Result<(), Error> {
    validate_look(settings, look, scale)?;
    r.set_post_settings(look.post)?;
    r.set_colour_lut(look.colour_lut.clone())?;
    r.set_reconstruction(if settings.temporal_reconstruction {
        Reconstruction {
            scale,
            ..Reconstruction::default()
        }
    } else {
        Reconstruction::off()
    })?;
    r.set_reflections(look.reflections)?;
    match path {
        RenderPath::HardwareRt => r.set_soft_shadows(match look.quality {
            Quality::Realtime => SoftShadows::Interactive,
            Quality::Reference => SoftShadows::Exact,
        }),
        RenderPath::NonRt => r.set_virtual_shadow_settings(VirtualShadowSettings {
            lookup: match look.quality {
                Quality::Realtime => VirtualShadowSettings::default().lookup,
                Quality::Reference => VirtualLookup::Exact,
            },
            ..VirtualShadowSettings::default()
        })?,
    }
    Ok(())
}

/// What `apply_look` checks before it changes anything.
pub(crate) fn validate_look(
    settings: &Settings,
    look: &ViewLook,
    scale: RenderScale,
) -> Result<(), Error> {
    look.post.validate()?;
    if let Some(lut) = &look.colour_lut {
        lut.validate()?;
    }
    if !settings.temporal_reconstruction && scale != RenderScale::Native {
        return Err(Error::InvalidInput(
            "render_scale below Native needs temporal_reconstruction",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_shadow_facade_follows_renderer_defaults_and_accepts_the_larger_budget() {
        let defaults = Settings::default();
        assert_eq!(defaults.local_shadows, LocalShadows::default());
        assert_eq!(defaults.local_shadows.count, 128);
        for count in [0, 4, 16, 128] {
            let settings = Settings {
                local_shadows: LocalShadows {
                    count,
                    ..defaults.local_shadows
                },
                ..defaults
            };
            assert!(settings.local_shadows.validate(4096).is_ok());
        }
        assert!(
            LocalShadows {
                count: 129,
                ..defaults.local_shadows
            }
            .validate(4096)
            .is_err()
        );
    }

    #[test]
    fn sun_angles_and_hours_follow_the_documented_axes() {
        let east = Sun::at(0.0, std::f32::consts::FRAC_PI_2);
        assert!(
            (east.travel - Vec3::NEG_X).length() < 1e-6,
            "an eastern sun shines west"
        );
        let overhead = Sun::at(std::f32::consts::FRAC_PI_2, 1.0);
        assert!((overhead.elevation() - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        // Equator at noon: straight down. Northern latitude at noon: toward the south (+Z).
        let noon = Sun::at_hour(12.0, 0.0).unwrap();
        assert!((noon.travel - Vec3::NEG_Y).length() < 1e-6);
        let north = Sun::at_hour(12.0, 0.7).unwrap();
        assert!(
            north.travel.z < 0.0
                && (north.elevation() - (0.5 * std::f32::consts::PI - 0.7)).abs() < 1e-5
        );
        // Dusk is low in the west: its light travels east.
        let dusk = Sun::at_hour(17.5, 0.7).unwrap();
        assert!(dusk.travel.x > 0.9 && dusk.elevation() > 0.0 && dusk.elevation() < 0.15);
        assert!(Sun::at_hour(5.0, 0.7).is_none() && Sun::at_hour(19.0, 0.7).is_none());
    }

    #[test]
    fn only_the_atmosphere_sky_turns_on_the_physical_sky() {
        let settings = Settings::default();
        let physical = atmosphere(Sky::default(), &settings);
        assert!(physical.sky && physical.sun_disk && physical.environment);
        assert!(physical.sun_transmittance && physical.aerial_perspective);
        assert_eq!(physical.height_fog, settings.height_fog);
        assert_eq!(physical.volumetric_fog, Some(VolumetricFog::default()));
        let uniform = atmosphere(Sky::Uniform([1.0; 3]), &settings);
        assert!(!uniform.sky && !uniform.sun_disk && !uniform.environment);
        assert!(!uniform.sun_transmittance);
        // The view-path media follow the settings under every sky.
        assert!(uniform.aerial_perspective);
        let clear = Settings {
            aerial_perspective: false,
            height_fog: None,
            volumetric_fog: None,
            ..settings
        };
        let uniform = atmosphere(Sky::Uniform([1.0; 3]), &clear);
        assert_eq!(uniform, Atmosphere::off());
    }

    #[test]
    fn sun_is_light_zero_and_the_only_explicit_shadow_request() {
        let lamp = Light::Point {
            position: Vec3::Y,
            candela: [100.0; 3],
            range: Some(5.0),
            source_radius: 0.125,
        };
        let soft = Sun::default();
        // LC1: a soft sun renders with local lights.
        let (_, soft_shadows) =
            super::lighting(Some(soft), &[lamp], Sky::default(), Exposure::default()).unwrap();
        assert_eq!(soft_shadows.requests[1..], [ShadowRequest::None]);
        let sun = Sun {
            angular_radius: 0.0,
            ..soft
        };
        let (lighting, shadows) = lighting(
            Some(sun),
            &[lamp, lamp],
            Sky::Uniform([1.0, 2.0, 3.0]),
            Exposure::Manual { stops: -3.0 },
        )
        .unwrap();
        assert!(matches!(lighting.lights[0], Light::Directional { .. }));
        assert_eq!(lighting.lights.len(), 3);
        assert_eq!(
            lighting.lights[1], lamp,
            "source radius survives the engine facade"
        );
        assert_eq!(lighting.environment, [1.0, 2.0, 3.0]);
        assert_eq!(lighting.exposure_stops, -3.0);
        assert!(matches!(
            shadows.requests[0],
            ShadowRequest::Directional { .. }
        ));
        assert!(
            shadows.requests[1..]
                .iter()
                .all(|r| *r == ShadowRequest::None)
        );
        assert_eq!(shadows.directional_disk_angle, sun.angular_radius);

        // No sun: no explicit shadow requests; the automatic local pool still shadows lamps.
        let (lighting, shadows) =
            super::lighting(None, &[lamp], Sky::default(), Exposure::default()).unwrap();
        assert_eq!(lighting.lights.len(), 1);
        assert!(shadows.requests.is_empty());

        let directional = Light::Directional {
            travel: Vec3::NEG_Y,
            lux: [1.0; 3],
        };
        assert!(
            super::lighting(None, &[directional], Sky::default(), Exposure::default()).is_err()
        );
        let wide = Sun {
            angular_radius: 0.2,
            ..soft
        };
        assert!(super::lighting(Some(wide), &[], Sky::default(), Exposure::default()).is_err());
    }
}
