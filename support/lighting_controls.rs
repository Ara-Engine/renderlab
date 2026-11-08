//! Tool-only JSON boundary. Runtime lighting has no serde/file/window dependency.
use renderlab::{Light, Lighting, Renderer, ShadowRequest, Shadows, Vec3};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    lights: Vec<Source>,
    #[serde(default)]
    environment: [f32; 3],
    #[serde(default)]
    exposure_stops: f32,
    #[serde(default)]
    shadows: Option<ShadowSettings>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Source {
    Directional {
        travel: [f32; 3],
        lux: [f32; 3],
    },
    Point {
        position: [f32; 3],
        candela: [f32; 3],
        range: Option<f32>,
        #[serde(default)]
        source_radius: f32,
    },
    Spot {
        position: [f32; 3],
        travel: [f32; 3],
        candela: [f32; 3],
        range: Option<f32>,
        #[serde(default)]
        source_radius: f32,
        inner: f32,
        outer: f32,
    },
}
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ShadowSettings {
    requests: Vec<ShadowSource>,
    resolution: u32,
    depth_bias: f32,
    normal_bias: f32,
    directional_disk_angle: f32,
    max_bytes: Option<u64>,
}
impl Default for ShadowSettings {
    fn default() -> Self {
        let value = Shadows::default();
        Self {
            requests: vec![],
            resolution: value.resolution,
            depth_bias: value.depth_bias,
            normal_bias: value.normal_bias,
            directional_disk_angle: value.directional_disk_angle,
            max_bytes: value.max_bytes,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ShadowSource {
    None,
    Directional { split_ends: Vec<f32> },
    Point { near: f32, far: f32 },
    Spot { near: f32, far: f32 },
}
pub fn parse_with_shadows(source: &str) -> Result<(Lighting, Shadows), serde_json::Error> {
    let settings: Settings = serde_json::from_str(source)?;
    let value = settings.shadows.unwrap_or_default();
    let shadows = Shadows {
        requests: value
            .requests
            .into_iter()
            .map(|r| match r {
                ShadowSource::None => ShadowRequest::None,
                ShadowSource::Directional { split_ends } => {
                    ShadowRequest::Directional { split_ends }
                }
                ShadowSource::Point { near, far } => ShadowRequest::Point { near, far },
                ShadowSource::Spot { near, far } => ShadowRequest::Spot { near, far },
            })
            .collect(),
        resolution: value.resolution,
        depth_bias: value.depth_bias,
        normal_bias: value.normal_bias,
        directional_disk_angle: value.directional_disk_angle,
        max_bytes: value.max_bytes,
    };
    let lighting = Lighting {
        lights: settings
            .lights
            .into_iter()
            .map(|s| match s {
                Source::Directional { travel, lux } => Light::Directional {
                    travel: Vec3::from_array(travel),
                    lux,
                },
                Source::Point {
                    position,
                    candela,
                    range,
                    source_radius,
                } => Light::Point {
                    position: Vec3::from_array(position),
                    candela,
                    range,
                    source_radius,
                },
                Source::Spot {
                    position,
                    travel,
                    candela,
                    range,
                    source_radius,
                    inner,
                    outer,
                } => Light::Spot {
                    position: Vec3::from_array(position),
                    travel: Vec3::from_array(travel),
                    candela,
                    range,
                    source_radius,
                    inner,
                    outer,
                },
            })
            .collect(),
        environment: settings.environment,
        exposure_stops: settings.exposure_stops,
    };
    Ok((lighting, shadows))
}
pub fn default_lighting() -> Lighting {
    Lighting {
        lights: vec![Light::Directional {
            travel: Vec3::new(-0.4, -0.8, -0.3),
            lux: [1000.; 3],
        }],
        environment: [30.; 3],
        exposure_stops: -8.,
    }
}
#[derive(Default)]
pub struct LightingFile {
    observed: Option<Result<String, String>>,
}
impl LightingFile {
    pub fn poll(&mut self, path: &Path, renderer: &mut Renderer) {
        let read = std::fs::read_to_string(path).map_err(|e| e.to_string());
        if self.observed.as_ref() == Some(&read) {
            return;
        }
        self.observed = Some(read.clone());
        let update = read
            .and_then(|s| parse_with_shadows(&s).map_err(|e| e.to_string()))
            .and_then(|(v, shadows)| {
                renderer
                    .set_lighting_and_shadows(&v, &shadows)
                    .map_err(|e| e.to_string())
            });
        match update {
            Ok(()) => eprintln!(
                "lighting_updated path={} stats={:?} shadow_limits={:?} transport=direct_constant_environment_optional_hard_shadows diffuse=plain_lambert tone=per_channel_reinhard",
                path.display(),
                renderer.lighting_stats(),
                renderer.shadow_limits()
            ),
            Err(e) => eprintln!(
                "lighting_rejected path={} last_valid_retained=true diagnostic={e}",
                path.display()
            ),
        }
    }
}
