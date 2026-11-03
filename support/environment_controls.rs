//! File boundary for RLE1 loading and small, live yaw/intensity edits.
use renderlab::{EnvironmentMap, EnvironmentParameters, Renderer, asset::environment};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Settings {
    enabled: bool,
    rotation_y_radians: f32,
    intensity: f32,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            rotation_y_radians: 0.,
            intensity: 1.,
        }
    }
}

pub struct EnvironmentFile {
    pending: Option<environment::PreparedEnvironment>,
    map: Option<EnvironmentMap>,
    parameters: EnvironmentParameters,
    enabled: bool,
    observed: Option<Result<String, String>>,
}
impl EnvironmentFile {
    pub fn load(
        path: Option<&Path>,
        renderer: &Renderer,
        limits: environment::Limits,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if path.is_some() && !renderer.environment_supported() {
            return Err("RLE1 environment requires adapter FLOAT32_FILTERABLE; omit --environment for constant fallback".into());
        }
        let pending = path.map(|p| load_environment(p, limits)).transpose()?;
        Ok(Self {
            pending,
            map: None,
            parameters: Default::default(),
            enabled: true,
            observed: None,
        })
    }
    pub fn advance(&mut self, renderer: &mut Renderer) -> Result<(), renderlab::Error> {
        if let Some(data) = &self.pending
            && let Some(map) = renderer.try_upload_environment(data)?
        {
            if self.enabled {
                renderer.set_environment(Some(&map), self.parameters)?;
            }
            self.map = Some(map);
            self.pending = None;
            eprintln!(
                "environment_uploaded f32_filtered=true owned_bytes={} upload_bytes={}",
                renderer.stats().ownership.environment_bytes.live,
                renderer.stats().uploads.environment
            );
        }
        Ok(())
    }
    pub fn poll(&mut self, path: &Path, renderer: &mut Renderer) {
        let read = std::fs::read_to_string(path).map_err(|e| e.to_string());
        if self.observed.as_ref() == Some(&read) {
            return;
        }
        self.observed = Some(read.clone());
        let update = read
            .and_then(|s| serde_json::from_str::<Settings>(&s).map_err(|e| e.to_string()))
            .and_then(|s| {
                let parameters = EnvironmentParameters {
                    rotation_y_radians: s.rotation_y_radians,
                    intensity: s.intensity,
                };
                renderer
                    .set_environment(self.map.as_ref().filter(|_| s.enabled), parameters)
                    .map_err(|e| e.to_string())?;
                self.parameters = parameters;
                self.enabled = s.enabled;
                Ok(())
            });
        match update {
            Ok(()) => eprintln!(
                "environment_settings_updated enabled={} yaw={} intensity={}",
                self.enabled, self.parameters.rotation_y_radians, self.parameters.intensity
            ),
            Err(e) => {
                eprintln!("environment_settings_rejected last_valid_retained=true diagnostic={e}")
            }
        }
    }
}

pub fn load_environment(
    path: &Path,
    limits: environment::Limits,
) -> Result<environment::PreparedEnvironment, Box<dyn std::error::Error>> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let cap = limits
        .max_payload_bytes
        .saturating_add(environment::HEADER_BYTES as u64);
    if file.metadata()?.len() > cap {
        return Err("environment file exceeds payload limit".into());
    }
    let mut bytes = Vec::new();
    file.take(cap.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok(environment::decode(&bytes, limits)?)
}
