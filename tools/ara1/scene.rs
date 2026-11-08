//! Shared measurement inputs; no engine-specific scene defaults enter this contract.
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub fov_y: f32,
    pub phase: String,
    pub cut: bool,
    pub capture: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalLight {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub lumens: f32,
    pub range: f32,
    /// None is a point light; spot angles are cone half angles in radians.
    pub spot: Option<Spot>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Spot {
    pub travel: [f32; 3],
    pub inner: f32,
    pub outer: f32,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub version: u32,
    pub name: String,
    pub asset: PathBuf,
    pub size: [u32; 2],
    /// Direction from the scene toward the sun, normalized by each adapter.
    pub to_light: [f32; 3],
    pub sun_lux: [f32; 3],
    pub sun_radius: f32,
    pub environment: [f32; 3],
    pub exposure_stops: f32,
    pub lights: Vec<LocalLight>,
    pub frames: Vec<Frame>,
    pub notes: Vec<String>,
    /// Tiny, explicitly unqualified startup smoke profile; never a quality/performance row.
    #[serde(default)]
    pub startup_minimal: bool,
}

pub fn load(path: &Path) -> Result<Scene, Box<dyn Error>> {
    let mut scene: Scene = serde_json::from_slice(&fs::read(path)?)?;
    if scene.asset.is_relative() {
        scene.asset = path
            .parent()
            .ok_or("scene has no parent")?
            .join(&scene.asset);
    }
    if scene.version != 1 || scene.size.contains(&0) || scene.frames.is_empty() {
        return Err("unsupported or empty ARA1 scene".into());
    }
    Ok(scene)
}
