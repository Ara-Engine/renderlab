//! Engine-level lighting, sky and quality settings, and how they map onto the renderer.
//!
//! These types are deliberately coarse: each names a choice a game makes, not an
//! estimator's tuning. The renderer's own tuning structs (`IndirectSettings`,
//! `Reconstruction`, `VirtualShadowSettings`, ...) change as features are measured; the
//! engine keeps these stable and maps them to each path's measured configuration.
use super::{EnvironmentId, RenderPath};
use crate::{
    Error, IndirectSettings, Light, Lighting, Reconstruction, RenderScale, Renderer, ShadowRequest,
    Shadows, SoftShadows, Vec3, VirtualLookup, VirtualShadowSettings,
};

/// The scene's sun: its one shadowed directional light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// The direction sunlight travels (from the sun toward the ground). Normalized on use.
    pub travel: Vec3,
    /// Illuminance on a surface facing the sun, in lux per linear Rec.709 channel.
    /// Clear midday sun is about 100,000 lux.
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
            illuminance: [100_000.0; 3],
            angular_radius: 0.04,
        }
    }
}

/// What lights the scene from infinitely far away: sky light, reflections of the sky, and
/// the background seen where camera rays miss all geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Sky {
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
        Self::Uniform([0.0; 3])
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
    /// (M7 G2). Off, programs read the unoccluded sky. Off by default until the GI unit
    /// rules it on.
    pub global_illumination: bool,
    /// Temporal reconstruction (TR1): each frame samples at a new sub-pixel offset and
    /// accumulates history, antialiasing edges and textures. On by default (TR1b).
    pub temporal_reconstruction: bool,
    /// Internal resolution while temporal reconstruction is on: `Native`, or the
    /// `Quality` (2/3 per axis) and `Performance` (1/2) upscaling presets, which TR1
    /// reconstructs to the target size. Needs `temporal_reconstruction`.
    pub render_scale: RenderScale,
    pub exposure: Exposure,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: Quality::Realtime,
            global_illumination: false,
            temporal_reconstruction: true,
            render_scale: RenderScale::Native,
            exposure: Exposure::default(),
        }
    }
}

/// The renderer's light vector and shadow requests for a sun and local lights.
/// The sun, when present, is light 0 and the only shadowed light.
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
        // Current renderer limit (shadow_gpu.rs validate_shadow_settings): soft sun
        // visibility needs the sun to be the only light. Many-light shading (LC1) lifts it.
        if sun.angular_radius > 0.0 && !lights.is_empty() {
            return Err(Error::InvalidInput(
                "a soft sun (angular_radius > 0) cannot be combined with local lights yet; use angular_radius 0 or no local lights",
            ));
        }
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

/// Applies `settings` to the renderer's estimators on `path`. Called only on change:
/// re-applying identical GI settings is cheap, but the engine does not rely on it.
pub(crate) fn apply(r: &mut Renderer, path: RenderPath, settings: &Settings) -> Result<(), Error> {
    if !settings.temporal_reconstruction && settings.render_scale != RenderScale::Native {
        return Err(Error::InvalidInput(
            "render_scale below Native needs temporal_reconstruction",
        ));
    }
    r.set_indirect(IndirectSettings {
        enabled: settings.global_illumination,
        ..IndirectSettings::default()
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
    fn sun_is_light_zero_and_the_only_shadowed_light() {
        let lamp = Light::Point {
            position: Vec3::Y,
            candela: [100.0; 3],
            range: Some(5.0),
        };
        let soft = Sun::default();
        assert!(super::lighting(Some(soft), &[lamp], Sky::default(), Exposure::default()).is_err());
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

        // No sun: an empty request vector disables shadows for every light.
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
