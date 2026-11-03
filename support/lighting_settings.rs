//! Legacy lighting-only boundary for the frozen M5 evidence tool.
#[path = "lighting_controls.rs"]
mod controls;
use renderlab::Lighting;

pub fn parse(source: &str) -> Result<Lighting, serde_json::Error> {
    let (lighting, shadows) = controls::parse_with_shadows(source)?;
    if !shadows.requests.is_empty() {
        return Err(<serde_json::Error as serde::de::Error>::custom(
            "shadow requests require parse_with_shadows and atomic renderer publication",
        ));
    }
    Ok(lighting)
}
