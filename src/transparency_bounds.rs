//! Conservative screen bounds, including boxes crossing the eye plane.
use glam::{Mat4, Vec2, Vec3, Vec4};

pub(super) fn rect(bounds: &crate::visibility::Aabb, model: Mat4, view: Mat4, size: [u32; 2]) -> [u32; 4] {
    let full = [0, 0, size[0], size[1]];
    let transform = view * model;
    let corners: [Vec4; 8] = std::array::from_fn(|i| {
        transform * Vec3::new(
            if i & 1 == 0 { bounds.min[0] } else { bounds.max[0] },
            if i & 2 == 0 { bounds.min[1] } else { bounds.max[1] },
            if i & 4 == 0 { bounds.min[2] } else { bounds.max[2] },
        ).extend(1.0)
    });
    if corners.iter().any(|p| !p.is_finite()) { return full; }
    // Clip the convex box's edges just in front of the eye, well before the camera's
    // near plane. Its projected vertices bound every visible point in the box.
    const EYE: f32 = 1e-5;
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    let mut add = |p: Vec4| {
        let ndc = p.truncate().truncate() / p.w;
        let pixel = Vec2::new((ndc.x * 0.5 + 0.5) * size[0] as f32, (0.5 - ndc.y * 0.5) * size[1] as f32);
        lo = lo.min(pixel); hi = hi.max(pixel);
    };
    for (i, &a) in corners.iter().enumerate() {
        if a.w >= EYE { add(a); }
        for bit in [1, 2, 4] {
            let j = i ^ bit;
            if j <= i { continue; }
            let b = corners[j];
            if (a.w >= EYE) != (b.w >= EYE) {
                let t = (EYE - a.w) / (b.w - a.w);
                let mut p = a + t * (b - a);
                p.w = EYE;
                add(p);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() { return [0; 4]; }
    [
        (lo.x - 2.0).floor().clamp(0.0, size[0] as f32) as u32,
        (lo.y - 2.0).floor().clamp(0.0, size[1] as f32) as u32,
        (hi.x + 2.0).ceil().clamp(0.0, size[0] as f32) as u32,
        (hi.y + 2.0).ceil().clamp(0.0, size[1] as f32) as u32,
    ]
}
