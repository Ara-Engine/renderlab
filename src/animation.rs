//! Small glTF clip sampler. Joint palettes stay separate from renderer/material APIs.
use crate::asset::skin::{Channel, Interpolation, Property, SkinnedAsset};
use crate::{Error, Mat4};
use glam::{Quat, Vec3, Vec4};

pub struct AnimationPose {
    globals: Vec<Mat4>,
}
impl AnimationPose {
    pub fn node_transform(&self, node: usize) -> Option<Mat4> {
        self.globals.get(node).copied()
    }
    /// Mesh-local palette: inverse(mesh global) * animated joint global * inverse bind.
    pub fn joint_palette(&self, asset: &SkinnedAsset, binding: usize) -> Result<Vec<Mat4>, Error> {
        let b = asset
            .bindings
            .get(binding)
            .ok_or(Error::InvalidInput("skin binding index"))?;
        let s = asset
            .skins
            .get(b.skin as usize)
            .ok_or(Error::InvalidInput("skin index"))?;
        let model = self
            .globals
            .get(b.node as usize)
            .ok_or(Error::InvalidInput("skin node index"))?;
        // Preserve the exact affine row; a general f32 inverse can round its W
        // away from one, which the renderer correctly rejects as projective.
        let inverse = Mat4::from(glam::Affine3A::from_mat4(*model).inverse());
        if !inverse.is_finite() {
            return Err(Error::InvalidInput("animated mesh transform is singular"));
        }
        s.joints
            .iter()
            .zip(&s.inverse_bind)
            .map(|(&j, m)| {
                self.globals
                    .get(j as usize)
                    .map(|global| inverse * *global * Mat4::from_cols_array_2d(m))
                    .ok_or(Error::InvalidInput("pose joint outside sampled nodes"))
            })
            .collect()
    }
}

/// A single active clip with looping, seconds-based playback. Blending and root-motion
/// extraction belong to the caller; sampling itself is independent of frame rate.
pub struct ClipPlayer {
    pub clip: usize,
    pub looping: bool,
    pub speed: f32,
    time: f32,
}
impl ClipPlayer {
    pub fn new(clip: usize) -> Self {
        Self {
            clip,
            looping: true,
            speed: 1.,
            time: 0.,
        }
    }
    pub fn time(&self) -> f32 {
        self.time
    }
    pub fn seek(&mut self, seconds: f32) -> Result<(), Error> {
        if !seconds.is_finite() {
            return Err(Error::InvalidInput("clip time must be finite"));
        }
        self.time = seconds;
        Ok(())
    }
    pub fn advance(&mut self, seconds: f32) -> Result<(), Error> {
        self.seek(self.time + seconds * self.speed)
    }
    pub fn sample(&self, asset: &SkinnedAsset) -> Result<AnimationPose, Error> {
        let clip = asset
            .clips
            .get(self.clip)
            .ok_or(Error::InvalidInput("animation clip index"))?;
        let duration = clip.duration();
        let t = if self.looping && duration > 0. {
            self.time.rem_euclid(duration)
        } else {
            self.time.clamp(0., duration)
        };
        let mut trs: Vec<_> = asset
            .nodes
            .iter()
            .map(|n| {
                (
                    Vec3::from(n.translation),
                    Quat::from_array(n.rotation).normalize(),
                    Vec3::from(n.scale),
                )
            })
            .collect();
        for c in &clip.channels {
            let value = sample_channel(c, t);
            let dst = trs
                .get_mut(c.node as usize)
                .ok_or(Error::InvalidInput("animation node index"))?;
            match c.property {
                Property::Translation => dst.0 = value.truncate(),
                Property::Rotation => dst.1 = Quat::from_array(value.to_array()).normalize(),
                Property::Scale => dst.2 = value.truncate(),
            }
        }
        let local: Vec<_> = trs
            .into_iter()
            .map(|(t, r, s)| Mat4::from_scale_rotation_translation(s, r, t))
            .collect();
        let mut globals = vec![Mat4::IDENTITY; local.len()];
        let mut done = vec![false; local.len()];
        for root in 0..local.len() {
            let mut chain = vec![];
            let mut at = Some(root);
            while let Some(i) = at {
                if i >= local.len() || chain.len() >= local.len() {
                    return Err(Error::InvalidInput("invalid animation hierarchy"));
                }
                if done[i] {
                    break;
                }
                chain.push(i);
                at = asset.nodes[i].parent.map(|p| p as usize);
            }
            for i in chain.into_iter().rev() {
                globals[i] = asset.nodes[i]
                    .parent
                    .map_or(Mat4::IDENTITY, |p| globals[p as usize])
                    * local[i];
                done[i] = true;
                if !globals[i].is_finite() {
                    return Err(Error::InvalidInput("animated transform overflow"));
                }
            }
        }
        Ok(AnimationPose { globals })
    }
}

fn sample_channel(c: &Channel, t: f32) -> Vec4 {
    let cubic = c.interpolation == Interpolation::CubicSpline;
    let stride = if cubic { 3 } else { 1 };
    let value = |key: usize| Vec4::from(c.values[key * stride + usize::from(cubic)]);
    let upper = c.times.partition_point(|&k| k <= t);
    if upper == 0 {
        return value(0);
    }
    if upper == c.times.len() {
        return value(upper - 1);
    }
    let lower = upper - 1;
    if c.interpolation == Interpolation::Step {
        return value(lower);
    }
    let dt = c.times[upper] - c.times[lower];
    let f = (t - c.times[lower]) / dt;
    if cubic {
        let f2 = f * f;
        let f3 = f2 * f;
        let a = value(lower);
        let b = value(upper);
        let out = Vec4::from(c.values[lower * 3 + 2]) * dt;
        let input = Vec4::from(c.values[upper * 3]) * dt;
        return a * (2. * f3 - 3. * f2 + 1.)
            + out * (f3 - 2. * f2 + f)
            + b * (-2. * f3 + 3. * f2)
            + input * (f3 - f2);
    }
    if c.property == Property::Rotation {
        Vec4::from_array(
            Quat::from_array(value(lower).to_array())
                .slerp(Quat::from_array(value(upper).to_array()), f)
                .to_array(),
        )
    } else {
        value(lower).lerp(value(upper), f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mesh_local_palette_preserves_an_exact_affine_row() {
        let model = Mat4::from_scale_rotation_translation(
            Vec3::new(1.37, 0.73, 2.43),
            Quat::from_euler(glam::EulerRot::XYZ, 0.3, 0.7, -0.2),
            Vec3::new(2., 3., 4.),
        );
        let joint = Mat4::from_translation(Vec3::new(1., 2., 3.));
        let pose = AnimationPose {
            globals: vec![model, model * joint],
        };
        let asset = SkinnedAsset {
            geometry: Default::default(),
            nodes: vec![],
            influences: vec![],
            clips: vec![],
            skins: vec![crate::asset::skin::Skin {
                joints: vec![1],
                inverse_bind: vec![joint.inverse().to_cols_array_2d()],
            }],
            bindings: vec![crate::asset::skin::Binding {
                mesh: 0,
                node: 0,
                skin: 0,
            }],
        };
        let palette = pose.joint_palette(&asset, 0).unwrap();
        assert!(crate::normal::NormalTransform::new(palette[0]).valid());
        assert!(palette[0].abs_diff_eq(Mat4::IDENTITY, 1e-5));
    }
    #[test]
    fn clip_sampling_handles_steps_shortest_rotations_and_cubic_tangent_units() {
        let mut c = Channel {
            node: 0,
            property: Property::Translation,
            interpolation: Interpolation::Linear,
            times: vec![0., 2.],
            values: vec![[0.; 4], [2., 0., 0., 0.]],
        };
        assert_eq!(sample_channel(&c, 1.).x, 1.);
        c.interpolation = Interpolation::Step;
        assert_eq!(sample_channel(&c, 1.).x, 0.);
        c.interpolation = Interpolation::CubicSpline;
        c.values = vec![
            [0.; 4],
            [0.; 4],
            [1., 0., 0., 0.],
            [1., 0., 0., 0.],
            [2., 0., 0., 0.],
            [0.; 4],
        ];
        assert!((sample_channel(&c, 0.5).x - 0.5).abs() < 1e-6);
        c.property = Property::Rotation;
        c.interpolation = Interpolation::Linear;
        c.values = vec![[0., 0., 0., 1.], [0., 0., 0., -1.]];
        assert!(sample_channel(&c, 1.).w.abs() > 0.999);
    }
}
