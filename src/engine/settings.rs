//! Engine-level lighting, sky and quality settings, and how they map onto the renderer.
//!
//! These types are deliberately coarse: each names a choice a game makes, not an
//! estimator's tuning. The renderer's own tuning structs (`IndirectSettings`,
//! `Reconstruction`, `VirtualShadowSettings`, ...) change as features are measured; the
//! engine keeps these stable and maps them to each path's measured configuration.
use super::{EnvironmentId, RenderPath};
use crate::{
    Atmosphere, AtmosphereMedium, Error, HeightFog, IndirectSettings, Light, Lighting,
    Reconstruction, RenderScale, Renderer, ShadowRequest, Shadows, SoftShadows, Vec3,
    VirtualLookup, VirtualShadowSettings, VolumetricFog,
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

/// Frame-wide rendering choices. Construct with `..Default::default()`: new fields are
/// added with defaults as features land, so struct literals that spell out every field
/// may need updating. Defaults follow each feature unit's default ruling (docs/api.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub quality: Quality,
    /// Indirect light: sky visibility and bounce light from the world radiance cache
    /// (M7 G2). On by default (G2b). Off, programs read the unoccluded sky.
    pub global_illumination: bool,
    /// Temporal reconstruction (TR1): each frame samples at a new sub-pixel offset and
    /// accumulates history, antialiasing edges and textures. On by default (TR1b).
    pub temporal_reconstruction: bool,
    /// Internal resolution while temporal reconstruction is on: `Native`, or the
    /// `Quality` (2/3 per axis) and `Performance` (1/2) upscaling presets, which TR1
    /// reconstructs to the target size. Needs `temporal_reconstruction`.
    pub render_scale: RenderScale,
    pub exposure: Exposure,
    /// Atmospheric scattering along every camera ray to a surface (AT1): distant surfaces
    /// fade toward the sky. On by default; it uses the physical atmosphere's medium
    /// whatever the [`Sky`].
    pub aerial_perspective: bool,
    /// Analytic exponential height fog on every camera ray, lit by the sun and the sky
    /// mean (AT1). On by default (`HeightFog::default()`: about 2 % extinction per 100 m
    /// near y = 0); None turns it off.
    pub height_fog: Option<HeightFog>,
    /// Froxel volumetric fog over the first `distance` metres, with sun shafts (AT1).
    /// Off by default. Its sun visibility is traced on `HardwareRt`; on `NonRt` it is
    /// unshadowed at this revision.
    pub volumetric_fog: Option<VolumetricFog>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: Quality::Realtime,
            global_illumination: true,
            temporal_reconstruction: true,
            render_scale: RenderScale::Native,
            exposure: Exposure::default(),
            aerial_perspective: true,
            height_fog: Some(HeightFog::default()),
            volumetric_fog: None,
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
            Background::Sky => [0.0; 3],
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
        // unrequested and LC1 shadows the most important of them (docs/m7-lc1.md).
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

/// Applies `settings` to the renderer's estimators on `path`. Called only on change:
/// re-applying identical GI settings is cheap, but the engine does not rely on it.
pub(crate) fn apply(r: &mut Renderer, path: RenderPath, settings: &Settings) -> Result<(), Error> {
    if !settings.temporal_reconstruction && settings.render_scale != RenderScale::Native {
        return Err(Error::InvalidInput(
            "render_scale below Native needs temporal_reconstruction",
        ));
    }
    r.set_indirect(if settings.global_illumination {
        IndirectSettings::default()
    } else {
        IndirectSettings::off()
    })?;
    r.set_reconstruction(if settings.temporal_reconstruction {
        Reconstruction {
            scale: settings.render_scale,
            ..Reconstruction::default()
        }
    } else {
        Reconstruction::off()
    })?;
    match path {
        RenderPath::HardwareRt => r.set_soft_shadows(match settings.quality {
            Quality::Realtime => SoftShadows::Interactive,
            Quality::Reference => SoftShadows::Exact,
        }),
        RenderPath::NonRt => r.set_virtual_shadow_settings(VirtualShadowSettings {
            lookup: match settings.quality {
                Quality::Realtime => VirtualShadowSettings::default().lookup,
                Quality::Reference => VirtualLookup::Exact,
            },
            ..VirtualShadowSettings::default()
        })?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_atmosphere_sky_turns_on_the_physical_sky() {
        let settings = Settings::default();
        let physical = atmosphere(Sky::default(), &settings);
        assert!(physical.sky && physical.sun_disk && physical.environment);
        assert!(physical.sun_transmittance && physical.aerial_perspective);
        assert_eq!(physical.height_fog, settings.height_fog);
        let uniform = atmosphere(Sky::Uniform([1.0; 3]), &settings);
        assert!(!uniform.sky && !uniform.sun_disk && !uniform.environment);
        assert!(!uniform.sun_transmittance);
        // The view-path media follow the settings under every sky.
        assert!(uniform.aerial_perspective);
        let clear = Settings {
            aerial_perspective: false,
            height_fog: None,
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
