//! CPU-only shadow geometry. Inputs are full submitted world bounds, never camera-visible IDs.
//! This private seam has no lighting, allocation, atlas, cache, or virtual-cut ownership.
#![allow(dead_code)] // The M6 GPU adapter follows M5-A acceptance.

use crate::{Camera, Error, Mat4, Vec3};
use glam::{DMat4, DVec3};

/// One conservative world AABB per submitted caster, in caller order. Zero extent is valid.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    fn corners(self) -> [DVec3; 8] {
        std::array::from_fn(|i| {
            DVec3::new(
                f64::from(if i & 1 == 0 { self.min.x } else { self.max.x }),
                f64::from(if i & 2 == 0 { self.min.y } else { self.max.y }),
                f64::from(if i & 4 == 0 { self.min.z } else { self.max.z }),
            )
        })
    }
}

/// Requested radial coverage, not photometric attenuation. Geometry nearer than `near`
/// is not guaranteed to cast. An unlimited M5 light still needs a finite shadow domain.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LocalRange {
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ViewKind {
    /// Positive camera-forward distances. Adjacent intervals share their boundary plane.
    Cascade {
        near: f32,
        far: f32,
    },
    Spot,
    Point(CubeFace),
}

/// Each view owns its values. Upload `world_to_clip` to a DISTINCT range per GPU view.
/// Matrices are RH, -Z forward, depth 0..1 (near..far), without a texture-Y transform.
#[derive(Clone, Debug)]
pub(crate) struct ShadowView {
    pub kind: ViewKind,
    pub resolution: u32,
    pub world_to_view: Mat4,
    pub view_to_clip: Mat4,
    /// Authoritative combined matrix, composed in f64 before rounding to f32.
    pub world_to_clip: Mat4,
    /// Axial light-view distances, which differ from local radial coverage.
    pub depth: [f32; 2],
    pub world_units_per_texel: Option<f32>,
    /// Indices into the input bounds, in input order. An empty list needs only a clear.
    pub caster_indices: Vec<usize>,
}

/// Fit camera receiver slices with pose-independent sphere extents and world-grid XY
/// snapping. `split_ends` are strictly increasing positive camera-forward distances;
/// the first slice starts at camera.near(). The last end is the explicit shadow distance.
/// It may conservatively exceed the camera far plane (Camera has no far accessor).
pub(crate) fn directional(
    camera: &Camera,
    viewport: [u32; 2],
    travel: Vec3,
    split_ends: &[f32],
    resolution: u32,
    casters: &[Bounds],
) -> Result<Vec<ShadowView>, Error> {
    validate_bounds(casters)?;
    if resolution < 2 || viewport.contains(&0) || split_ends.is_empty() {
        return Err(Error::InvalidInput(
            "directional shadows need resolution >= 2, a nonempty viewport and splits",
        ));
    }
    let mut previous = camera.near();
    for &end in split_ends {
        if !end.is_finite() || end <= previous {
            return Err(Error::InvalidInput(
                "shadow splits must be finite and increasing",
            ));
        }
        previous = end;
    }
    let camera_to_world = camera.view().as_dmat4().inverse();
    let tan_y = (f64::from(camera.fov_y()) * 0.5).tan();
    let tan_x = tan_y * f64::from(viewport[0]) / f64::from(viewport[1]);
    if !camera_to_world.is_finite() || !tan_x.is_finite() || tan_y <= 0.0 {
        return Err(Error::InvalidInput(
            "shadow camera must have a finite inverse and FOV",
        ));
    }
    let rotation = orientation(travel)?;
    // Transform once per light, not once per cascade. Full bounds include offscreen casters.
    let light_bounds: Vec<_> = casters
        .iter()
        .map(|bounds| extents(bounds.corners().map(|p| rotation.transform_point3(p))))
        .collect();
    let mut views = Vec::with_capacity(split_ends.len());
    let mut near = camera.near();
    for &far in split_ends {
        let n = f64::from(near);
        let f = f64::from(far);
        let center = camera_to_world.transform_point3(DVec3::new(0.0, 0.0, -(n + f) * 0.5));
        let center_light = rotation.transform_point3(center);
        // The far-plane diagonal and half-depth enclose BOTH planes; no scene-tight XY fit.
        let radius = DVec3::new(f * tan_x, f * tan_y, (f - n) * 0.5).length();
        let guard = roundoff(center.abs().max_element() + radius);
        // Half a texel of additional extent covers rounding the center to the nearest texel.
        let half = (radius + guard) / (1.0 - 1.0 / f64::from(resolution));
        let texel = 2.0 * half / f64::from(resolution);
        let x = (center_light.x / texel).round() * texel;
        let y = (center_light.y / texel).round() * texel;
        let receivers = std::array::from_fn::<_, 8, _>(|i| {
            let depth = if i & 4 == 0 { n } else { f };
            let p = DVec3::new(
                if i & 1 == 0 {
                    -depth * tan_x
                } else {
                    depth * tan_x
                },
                if i & 2 == 0 {
                    -depth * tan_y
                } else {
                    depth * tan_y
                },
                -depth,
            );
            rotation.transform_point3(camera_to_world.transform_point3(p))
        });
        let (receiver_min, receiver_max) = extents(receivers);
        let (mut z_min, mut z_max) = (receiver_min.z, receiver_max.z);
        let mut world_magnitude = center.abs().max_element() + radius;
        let mut caster_indices = Vec::new();
        for (index, &(min, max)) in light_bounds.iter().enumerate() {
            let tolerance = roundoff(min.abs().max(max.abs()).max_element());
            if max.x + tolerance < x - half
                || min.x - tolerance > x + half
                || max.y + tolerance < y - half
                || min.y - tolerance > y + half
                // +Z is lightward. A caster entirely behind all receivers cannot block them.
                || max.z + tolerance < receiver_min.z
            {
                continue;
            }
            caster_indices.push(index);
            // Keep the entire assigned AABB in Z, even its unnecessary downstream portion.
            z_min = z_min.min(min.z);
            z_max = z_max.max(max.z);
            world_magnitude = world_magnitude.max(f64::from(
                casters[index]
                    .min
                    .abs()
                    .max(casters[index].max.abs())
                    .max_element(),
            ));
        }
        // Use world-coordinate magnitudes too: a small light-space Z can be the
        // difference of large world terms, so its value alone underestimates rounding.
        let pad = roundoff(world_magnitude.max(z_min.abs()).max(z_max.abs())).max(0.001);
        let view = DMat4::from_translation(DVec3::new(-x, -y, -(z_max + 2.0 * pad))) * rotation;
        let depth = [pad, z_max - z_min + 3.0 * pad];
        let projection = DMat4::orthographic_rh(-half, half, -half, half, depth[0], depth[1]);
        let mut planned = make_view(
            ViewKind::Cascade { near, far },
            resolution,
            view,
            projection,
            depth,
        )?;
        let texel = texel as f32;
        if !texel.is_normal() || texel <= 0.0 {
            return Err(Error::InvalidInput(
                "shadow texel size is not representable",
            ));
        }
        planned.world_units_per_texel = Some(texel);
        planned.caster_indices = caster_indices;
        views.push(planned);
        near = far;
    }
    Ok(views)
}

/// Square perspective view contains the circular spot cone. 90-degree half angles
/// cannot be represented by this projection; the lighting adapter must reject or handle them.
pub(crate) fn spot(
    position: Vec3,
    travel: Vec3,
    outer_half_angle: f32,
    range: LocalRange,
    resolution: u32,
    casters: &[Bounds],
) -> Result<ShadowView, Error> {
    validate_local(position, range, resolution, casters)?;
    if !outer_half_angle.is_finite()
        || outer_half_angle <= 0.0
        || outer_half_angle >= std::f32::consts::FRAC_PI_2
    {
        return Err(Error::InvalidInput(
            "shadow spot half angle must be in (0, pi/2)",
        ));
    }
    let half = f64::from(outer_half_angle);
    // Radial near at the cone rim has this axial distance; do not lose rim blockers.
    perspective_view(
        ViewKind::Spot,
        position,
        orientation(travel)?,
        half,
        [f64::from(range.near) * half.cos(), f64::from(range.far)],
        resolution,
        casters,
    )
}

/// Six independent square views in +X,-X,+Y,-Y,+Z,-Z order. Orientations are explicit
/// matrices, not an implicit texture-cube sampling convention.
pub(crate) fn point(
    position: Vec3,
    range: LocalRange,
    resolution: u32,
    casters: &[Bounds],
) -> Result<[ShadowView; 6], Error> {
    validate_local(position, range, resolution, casters)?;
    let depth = [f64::from(range.near) / 3.0_f64.sqrt(), f64::from(range.far)];
    let faces = [
        (CubeFace::PositiveX, Vec3::X),
        (CubeFace::NegativeX, Vec3::NEG_X),
        (CubeFace::PositiveY, Vec3::Y),
        (CubeFace::NegativeY, Vec3::NEG_Y),
        (CubeFace::PositiveZ, Vec3::Z),
        (CubeFace::NegativeZ, Vec3::NEG_Z),
    ];
    // Avoid partially published arrays on failure; there is no mutable planning state.
    let mut views = Vec::with_capacity(6);
    for (face, travel) in faces {
        views.push(perspective_view(
            ViewKind::Point(face),
            position,
            orientation(travel)?,
            std::f64::consts::FRAC_PI_4,
            depth,
            resolution,
            casters,
        )?);
    }
    Ok(views.try_into().expect("exactly six point faces"))
}

fn perspective_view(
    kind: ViewKind,
    position: Vec3,
    rotation: DMat4,
    half_angle: f64,
    depth: [f64; 2],
    resolution: u32,
    casters: &[Bounds],
) -> Result<ShadowView, Error> {
    let view = rotation * DMat4::from_translation(-position.as_dvec3());
    let projection = DMat4::perspective_rh(2.0 * half_angle, 1.0, depth[0], depth[1]);
    let mut planned = make_view(kind, resolution, view, projection, depth)?;
    planned.caster_indices = casters
        .iter()
        .enumerate()
        .filter_map(|(i, bounds)| intersects(planned.world_to_clip, *bounds).then_some(i))
        .collect();
    Ok(planned)
}

fn make_view(
    kind: ViewKind,
    resolution: u32,
    view: DMat4,
    projection: DMat4,
    depth: [f64; 2],
) -> Result<ShadowView, Error> {
    let depth = depth.map(|v| v as f32);
    if !depth[0].is_normal() || !depth[1].is_normal() || depth[0] <= 0.0 || depth[1] <= depth[0] {
        return Err(Error::InvalidInput(
            "shadow depth interval is not representable",
        ));
    }
    Ok(ShadowView {
        kind,
        resolution,
        world_to_view: checked_matrix(view)?,
        view_to_clip: checked_matrix(projection)?,
        world_to_clip: checked_matrix(projection * view)?,
        depth,
        world_units_per_texel: None,
        caster_indices: Vec::new(),
    })
}

fn checked_matrix(matrix: DMat4) -> Result<Mat4, Error> {
    let matrix = matrix.as_mat4();
    let determinant = matrix.as_dmat4().determinant();
    if !matrix.is_finite()
        || !determinant.is_finite()
        || determinant == 0.0
        || matrix.to_cols_array().iter().any(|v| v.is_subnormal())
    {
        return Err(Error::InvalidInput(
            "shadow matrix is nonfinite, subnormal or singular in f32",
        ));
    }
    Ok(matrix)
}

fn validate_bounds(bounds: &[Bounds]) -> Result<(), Error> {
    if bounds
        .iter()
        .any(|b| !b.min.is_finite() || !b.max.is_finite() || b.min.cmpgt(b.max).any())
    {
        return Err(Error::InvalidInput(
            "shadow bounds must be finite and ordered",
        ));
    }
    Ok(())
}

fn validate_local(
    position: Vec3,
    range: LocalRange,
    resolution: u32,
    bounds: &[Bounds],
) -> Result<(), Error> {
    validate_bounds(bounds)?;
    if !position.is_finite()
        || resolution == 0
        || !range.near.is_finite()
        || !range.far.is_finite()
        || range.near <= 0.0
        || range.far <= range.near
    {
        return Err(Error::InvalidInput(
            "shadow position/radial range/resolution is invalid",
        ));
    }
    Ok(())
}

fn orientation(travel: Vec3) -> Result<DMat4, Error> {
    if !travel.is_finite() || travel == Vec3::ZERO {
        return Err(Error::InvalidInput(
            "shadow travel must be finite and nonzero",
        ));
    }
    let forward = travel.as_dvec3().normalize();
    let a = forward.abs();
    let up = if a.x <= a.y && a.x <= a.z {
        DVec3::X
    } else if a.y <= a.z {
        DVec3::Y
    } else {
        DVec3::Z
    };
    Ok(DMat4::look_to_rh(DVec3::ZERO, forward, up))
}

fn extents(points: [DVec3; 8]) -> (DVec3, DVec3) {
    points.into_iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(min, max), p| (min.min(p), max.max(p)),
    )
}

/// Quantized coordinate magnitude keeps the precision guard fixed within each power-of-two
/// bin. At large translations it spends XY/depth precision conservatively, not silently clips.
fn roundoff(magnitude: f64) -> f64 {
    16.0 * f64::from(f32::EPSILON) * magnitude.max(1.0).log2().ceil().exp2()
}

/// Reject only if every corner lies outside one homogeneous plane. This deliberately
/// admits false positives, including some boxes beyond the circular cone/radial far limit.
fn intersects(world_to_clip: Mat4, bounds: Bounds) -> bool {
    let m = world_to_clip.as_dmat4().transpose();
    let planes = [
        m.w_axis + m.x_axis,
        m.w_axis - m.x_axis,
        m.w_axis + m.y_axis,
        m.w_axis - m.y_axis,
        m.z_axis,
        m.w_axis - m.z_axis,
    ];
    // Weight the original clip rows, before subtraction: near-parallel far planes can
    // cancel almost completely while the GPU still rounds two large clip coordinates.
    let weights = [
        m.w_axis.abs() + m.x_axis.abs(),
        m.w_axis.abs() + m.x_axis.abs(),
        m.w_axis.abs() + m.y_axis.abs(),
        m.w_axis.abs() + m.y_axis.abs(),
        m.z_axis.abs(),
        m.w_axis.abs() + m.z_axis.abs(),
    ];
    let min = bounds.min.as_dvec3();
    let max = bounds.max.as_dvec3();
    let center = ((min + max) * 0.5).extend(1.0);
    let extent = ((max - min) * 0.5).extend(0.0);
    let magnitude = min.abs().max(max.abs()).extend(1.0);
    for (plane, weight) in planes.into_iter().zip(weights) {
        let upper = plane.dot(center) + plane.abs().dot(extent);
        let tolerance =
            32.0 * f64::from(f32::EPSILON) * weight.dot(magnitude) + f64::from(f32::MIN_POSITIVE);
        if upper < -tolerance {
            return false;
        }
    }
    true
}

#[cfg(test)]
#[path = "shadow_plan_tests.rs"]
mod tests;
