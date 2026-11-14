//! Opt-in measurement fixture matching the Outpost clip's camera discontinuity.
use renderlab::engine::{Light, Mat4, Vec3};

pub const LEAD_FRAMES: u32 = 60;
pub const CUT_FRAME: u32 = 400 + LEAD_FRAMES;
const START: Vec3 = Vec3::new(-6.0, 1.6, -6.0);
const TRAVEL: Vec3 = Vec3::new(4.0, 0.0, -4.0);
const JUMP: Vec3 = Vec3::new(-20.0, 0.0, 15.0);

pub struct Pose {
    pub eye: Vec3,
    pub forward: Vec3,
}

/// The400-frame hold precedes the canonical clip. Travel and yaw continue at the
/// original per-frame speed after the single cut, including in extended captures.
pub fn pose(frame: u32, cut_frame: u32) -> Pose {
    let clip = frame.saturating_sub(cut_frame.saturating_sub(LEAD_FRAMES));
    let t = clip as f32 / 119.0;
    let cut = frame >= cut_frame;
    let yaw = (12.0 * t + if cut { 140.0 } else { 0.0 }).to_radians();
    let pitch = 15f32.to_radians();
    Pose {
        eye: START + TRAVEL * t + if cut { JUMP } else { Vec3::ZERO },
        forward: Mat4::from_rotation_y(yaw).transform_vector3(Vec3::new(
            0.0,
            -pitch.sin(),
            pitch.cos(),
        )),
    }
}

/// Flat, foliage-free camera corridors keep the exact zero-height jump above
/// terrain. Other forest geometry retains its original rolling height field.
pub fn clear(x: f32, z: f32) -> f32 {
    let p = Vec3::new(x, 0.0, z);
    let travel = TRAVEL * (259.0 / 119.0);
    let distance = [START, START + JUMP]
        .map(|start| {
            let start = Vec3::new(start.x, 0.0, start.z);
            let t = ((p - start).dot(travel) / travel.length_squared()).clamp(0.0, 1.0);
            (p - start - travel * t).length()
        })
        .into_iter()
        .fold(f32::INFINITY, f32::min);
    let t = ((distance - 3.0) / 10.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Thirteen sources around A and twelve around B, with fixed positions/power.
/// Their aimed gather-ray workload uses the renderer's unchanged near-light path.
pub fn lights(anchor: Vec3, height: impl Fn(f32, f32) -> f32) -> Vec<Light> {
    let destination = pose(CUT_FRAME, CUT_FRAME).eye;
    (0..25)
        .map(|i| {
            let (center, index, count) = if i < 13 {
                (START, i, 13)
            } else {
                (destination, i - 13, 12)
            };
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            let radius = 7.0 + 3.0 * (index % 3) as f32;
            let x = center.x + radius * angle.cos();
            let z = center.z + radius * angle.sin();
            Light::Point {
                position: anchor + Vec3::new(x, height(x, z) + 2.5, z),
                candela: [3200.0, 2400.0, 1600.0],
                range: Some(18.0),
                source_radius: 0.12,
                cast_shadows: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_and_cut_match_the_original_clip_motion() {
        assert_eq!(pose(0, CUT_FRAME).eye, pose(399, CUT_FRAME).eye);
        assert_eq!(pose(0, CUT_FRAME).eye, pose(400, CUT_FRAME).eye);
        let before = pose(CUT_FRAME - 1, CUT_FRAME);
        let after = pose(CUT_FRAME, CUT_FRAME);
        assert!((after.eye - before.eye - TRAVEL / 119.0 - JUMP).length() < 1e-5);
        let ordinary_yaw = Mat4::from_rotation_y((12.0f32 / 119.0).to_radians());
        let expected = Mat4::from_rotation_y(140f32.to_radians())
            .transform_vector3(ordinary_yaw.transform_vector3(before.forward));
        assert!((after.forward - expected).length() < 1e-5);
        assert!((pose(CUT_FRAME + 1, CUT_FRAME).eye - after.eye - TRAVEL / 119.0).length() < 1e-5);
    }

    #[test]
    fn camera_paths_stay_in_flat_clearings() {
        for frame in 0..660 {
            let p = pose(frame, CUT_FRAME).eye;
            assert_eq!(clear(p.x, p.z), 0.0);
        }
    }

    #[test]
    fn twenty_five_constant_sources_cover_both_arrivals() {
        let sources = lights(Vec3::ZERO, |_, _| 0.0);
        assert_eq!(sources.len(), 25);
        for (i, source) in sources.iter().enumerate() {
            let Light::Point {
                position, range, ..
            } = source
            else {
                panic!("point source")
            };
            let center = if i < 13 {
                START
            } else {
                pose(CUT_FRAME, CUT_FRAME).eye
            };
            let offset = *position - center;
            assert!((offset.x * offset.x + offset.z * offset.z).sqrt() <= 13.001);
            assert_eq!(*range, Some(18.0));
        }
    }
}
