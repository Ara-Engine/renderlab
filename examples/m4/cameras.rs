use renderlab::{Camera, Vec3};
use serde::Deserialize;
use std::{fs::File, io::Read, path::Path};
#[derive(Clone, Deserialize)]
pub struct Pose {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
    pub width: u32,
    pub height: u32,
    pub frame: u64,
    pub phase: String,
}
impl Pose {
    pub fn camera(&self) -> Result<Camera, renderlab::Error> {
        Camera::look_at(
            Vec3::from_array(self.eye),
            Vec3::from_array(self.target),
            Vec3::from_array(self.up),
            self.fov_y_radians,
            self.near,
            self.far,
        )
    }
}
#[derive(Deserialize)]
pub struct Cameras {
    pub frames: Vec<Pose>,
}
impl Cameras {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut bytes = vec![];
        File::open(path)?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("camera JSON exceeds 8MiB tooling cap".into());
        }
        let result: Self = serde_json::from_slice(&bytes)?;
        if result.frames.is_empty() || result.frames.len() > 10000 {
            return Err("camera count outside 1..10000".into());
        }
        for pose in &result.frames {
            pose.camera()?;
            if pose.width == 0 || pose.height == 0 || pose.width > 8192 || pose.height > 8192 {
                return Err("camera extent unsupported".into());
            }
        }
        Ok(result)
    }
}
