//! Camera and display post processing. Exposure remains owned by `Lighting`.
use crate::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tonemapper {
    /// The existing component-wise x / (1 + x) curve.
    #[default]
    Reinhard,
    /// A compact ACES-like filmic shoulder, not the ACES colour-management pipeline.
    Filmic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    pub focus_distance_m: f32,
    pub focal_length_mm: f32,
    pub f_number: f32,
    pub sensor_height_mm: f32,
    pub max_radius_px: f32,
}
impl Default for DepthOfField {
    fn default() -> Self {
        Self {
            focus_distance_m: 10.0,
            focal_length_mm: 50.0,
            f_number: 2.8,
            sensor_height_mm: 24.0,
            max_radius_px: 12.0,
        }
    }
}

/// Settings are validated atomically. Pixel radii refer to the reconstructed output.
/// `None` DOF means a pinhole camera. Motion is measured between submitted frames;
/// shutter angle is the fraction of that interval integrated by the filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostSettings {
    pub enabled: bool,
    /// Fraction of scene energy redistributed by the normalized bloom pyramid.
    pub bloom: f32,
    pub motion_blur: bool,
    pub shutter_angle_degrees: f32,
    pub max_motion_radius_px: f32,
    pub depth_of_field: Option<DepthOfField>,
    pub tonemapper: Tonemapper,
    /// Camera white point; 6500 K and zero tint are identity.
    pub white_balance_kelvin: f32,
    pub tint: f32,
    pub contrast: f32,
    pub saturation: f32,
    /// Edge attenuation, in [0, 1].
    pub vignette: f32,
    /// Display-linear noise amplitude; zero disables it.
    pub grain: f32,
    pub chromatic_aberration_px: f32,
    pub lut_strength: f32,
}
impl Default for PostSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            bloom: 0.035,
            motion_blur: true,
            shutter_angle_degrees: 180.0,
            max_motion_radius_px: 16.0,
            depth_of_field: None,
            tonemapper: Tonemapper::Reinhard,
            white_balance_kelvin: 6500.0,
            tint: 0.0,
            contrast: 1.0,
            saturation: 1.0,
            vignette: 0.035,
            grain: 0.0,
            chromatic_aberration_px: 0.0,
            lut_strength: 1.0,
        }
    }
}
impl PostSettings {
    /// Exact legacy presentation, including its tone curve, without post resources.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
    /// The `Settings` record the post kernels and the tone stage read: five vec4.
    /// `serial` counts post frames from one, `height` is the output height in pixels,
    /// and the LUT strength is zero until a custom LUT is installed.
    pub(crate) fn record(
        &self,
        serial: u32,
        height: u32,
        lut_width: u32,
        lut_custom: bool,
    ) -> [f32; 20] {
        let d = self.depth_of_field.unwrap_or_default();
        let focal = d.focal_length_mm * 0.001;
        let scale = focal * focal / (d.f_number * (d.focus_distance_m - focal)) * height as f32
            / (d.sensor_height_mm * 0.001)
            * 0.5;
        // Neutral at 6500 K; bounded RGB camera adaptation, separate from exposure.
        let warmth = (6500.0 / self.white_balance_kelvin).ln();
        let white = [
            (warmth * 0.35).exp(),
            (-self.tint * 0.15).exp(),
            (-warmth * 0.35).exp(),
        ];
        [
            self.bloom,
            self.shutter_angle_degrees / 720.0,
            self.max_motion_radius_px,
            (serial & 0x00ff_ffff) as f32,
            d.focus_distance_m,
            scale,
            d.max_radius_px,
            u32::from(self.depth_of_field.is_some()) as f32,
            self.contrast,
            self.saturation,
            self.vignette,
            self.grain,
            white[0],
            white[1],
            white[2],
            u32::from(self.tonemapper == Tonemapper::Filmic) as f32,
            self.chromatic_aberration_px,
            if lut_custom { self.lut_strength } else { 0.0 },
            lut_width as f32,
            0.0,
        ]
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let range = |v: f32, lo, hi| v.is_finite() && (lo..=hi).contains(&v);
        if !range(self.bloom, 0.0, 1.0)
            || !range(self.shutter_angle_degrees, 0.0, 360.0)
            || !range(self.max_motion_radius_px, 0.0, 64.0)
            || !range(self.white_balance_kelvin, 2000.0, 12000.0)
            || !range(self.tint, -1.0, 1.0)
            || !range(self.contrast, 0.1, 3.0)
            || !range(self.saturation, 0.0, 3.0)
            || !range(self.vignette, 0.0, 1.0)
            || !range(self.grain, 0.0, 0.1)
            || !range(self.chromatic_aberration_px, 0.0, 4.0)
            || !range(self.lut_strength, 0.0, 1.0)
        {
            return Err(Error::InvalidInput("invalid post settings"));
        }
        if let Some(d) = self.depth_of_field {
            if !range(d.focus_distance_m, 0.01, 1e7)
                || !range(d.focal_length_mm, 1.0, 1000.0)
                || d.focus_distance_m * 1000.0 <= d.focal_length_mm
                || !range(d.f_number, 0.5, 64.0)
                || !range(d.sensor_height_mm, 1.0, 100.0)
                || !range(d.max_radius_px, 0.0, 32.0)
            {
                return Err(Error::InvalidInput("invalid post camera depth of field"));
            }
        }
        Ok(())
    }
}

/// Display-linear Rec.709 cube, red varying fastest, then green, then blue.
/// Values and coordinates are [0, 1]. Uploaded as a filterable 3D texture.
#[derive(Clone, Debug, PartialEq)]
pub struct ColourLut {
    pub size: u32,
    pub rgb: Vec<[f32; 3]>,
}
impl ColourLut {
    pub fn identity(size: u32) -> Result<Self, Error> {
        if size < 2 {
            return Err(Error::InvalidInput("LUT size must be at least 2"));
        }
        let count = (size as usize)
            .checked_pow(3)
            .ok_or(Error::InvalidInput("LUT size overflows address space"))?;
        let mut rgb = Vec::new();
        rgb.try_reserve_exact(count)
            .map_err(|_| Error::InvalidInput("LUT allocation failed"))?;
        rgb.extend((0..size).flat_map(|b| {
            (0..size).flat_map(move |g| {
                (0..size).map(move |r| [r, g, b].map(|x| x as f32 / (size - 1) as f32))
            })
        }));
        Ok(Self { size, rgb })
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.size < 2
            || Some(self.rgb.len()) != (self.size as usize).checked_pow(3)
            || self
                .rgb
                .iter()
                .flatten()
                .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err(Error::InvalidInput(
                "LUT requires size^3 finite RGB values in [0,1]",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_and_lut_validation() {
        PostSettings::default().validate().unwrap();
        let mut s = PostSettings::default();
        s.depth_of_field = Some(DepthOfField::default());
        s.validate().unwrap();
        s.shutter_angle_degrees = f32::NAN;
        assert!(s.validate().is_err());
        s = PostSettings::default();
        s.depth_of_field = Some(DepthOfField {
            focus_distance_m: 0.02,
            ..Default::default()
        });
        assert!(s.validate().is_err());
        let mut lut = ColourLut::identity(4).unwrap();
        lut.validate().unwrap();
        assert_eq!(lut.rgb[1], [1.0 / 3.0, 0.0, 0.0]);
        assert_eq!(lut.rgb[63], [1.0; 3]);
        lut.rgb.pop();
        assert!(lut.validate().is_err());
    }
}
