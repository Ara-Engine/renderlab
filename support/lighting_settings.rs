//! Tool-only JSON boundary. Runtime lighting has no serde/file/window dependency.
use renderlab::{Light, Lighting, Renderer, Vec3};
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
    },
    Spot {
        position: [f32; 3],
        travel: [f32; 3],
        candela: [f32; 3],
        range: Option<f32>,
        inner: f32,
        outer: f32,
    },
}
pub fn parse(source: &str) -> Result<Lighting, serde_json::Error> {
    let settings: Settings = serde_json::from_str(source)?;
    Ok(Lighting {
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
                } => Light::Point {
                    position: Vec3::from_array(position),
                    candela,
                    range,
                },
                Source::Spot {
                    position,
                    travel,
                    candela,
                    range,
                    inner,
                    outer,
                } => Light::Spot {
                    position: Vec3::from_array(position),
                    travel: Vec3::from_array(travel),
                    candela,
                    range,
                    inner,
                    outer,
                },
            })
            .collect(),
        environment: settings.environment,
        exposure_stops: settings.exposure_stops,
    })
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
            .and_then(|s| parse(&s).map_err(|e| e.to_string()))
            .and_then(|v| renderer.set_lighting(&v).map_err(|e| e.to_string()));
        match update {
            Ok(()) => eprintln!(
                "lighting_updated path={} stats={:?} transport=unshadowed_direct_constant_environment diffuse=plain_lambert tone=per_channel_reinhard",
                path.display(),
                renderer.lighting_stats()
            ),
            Err(e) => eprintln!(
                "lighting_rejected path={} last_valid_retained=true diagnostic={e}",
                path.display()
            ),
        }
    }
}
