//! RT1d: fixed current-frame geometry-only filtering of incident sun visibility.
use super::*;

impl Renderer {
    /// Opts traced Interactive shadows into a fixed 5x5 geometric cross-bilateral
    /// gather. Programs still receive visibility, before any material or shading.
    /// Raster, Exact, temporal reconstruction and non-RT shadows are unchanged.
    /// This experimental setting is off until the complete HRT floor passes.
    pub fn set_interactive_shadow_filter(&mut self, enabled: bool) -> Result<(), Error> {
        let hardware = self.hardware_shadows.as_mut().ok_or(Error::InvalidInput(
            "Interactive visibility filtering requires hardware shadows",
        ))?;
        hardware.visibility_filter = enabled;
        Ok(())
    }
}

pub(super) fn compose(heads: &mut [String; 2]) {
    let hard = &mut heads[0];
    *hard = hard.replacen("fn hard_shadow(", "fn rl_shadow_raw(", 1);
    hard.push_str("\nfn hard_shadow(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {\n    if primary_frame.sided.z==0u || light.kind.z==0u {return rl_shadow_raw(light,receiver,l);}\n    return rl_shadow_visibility(light,receiver,l);\n}\nfn rl_shadow_sample(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {return rl_shadow_raw(light,receiver,l);}\n");
    hard.push_str(include_str!("../shaders/shadow_visibility_filter.wgsl"));
    let head = &mut heads[1];
    const RAW: &str = "return f32(textureLoad(fresh_visibility,receiver.pixel,0).x)/256.0;";
    assert_eq!(head.matches(RAW).count(), 1);
    let filtered = format!(
        "if primary_frame.sided.z==0u {{{RAW}}}\n    return rl_shadow_visibility(light,receiver,l);"
    );
    *head = head.replacen(RAW, &filtered, 1);
    head.push_str("\nfn rl_shadow_sample(light:Light,receiver:Receiver,l:vec3<f32>)->f32 {let count=textureLoad(fresh_visibility,receiver.pixel,0).x;return f32(count)/256.0;}\n");
    head.push_str(include_str!("../shaders/shadow_visibility_filter.wgsl"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_seam_only_changes_visibility_and_has_no_history_or_colour_inputs() {
        let mut heads = [
            "fn hard_shadow(".to_owned(),
            "return f32(textureLoad(fresh_visibility,receiver.pixel,0).x)/256.0;".to_owned(),
        ];
        compose(&mut heads);
        assert!(heads[1].contains("return rl_shadow_visibility(light,receiver,l);"));
        let filter = include_str!("../shaders/shadow_visibility_filter.wgsl");
        for absent in [
            "textureSample",
            "primary_hdr",
            "surface(",
            "shade(",
            "reproject",
            "variance",
            "luminance",
        ] {
            assert!(
                !filter.contains(absent),
                "unexpected filter input: {absent}"
            );
        }
        assert!(filter.contains("y=-2;y<=2"));
        assert!(filter.contains("x=-2;x<=2"));
    }
}
