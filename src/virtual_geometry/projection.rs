use super::*;
/// Virtual transforms are finite nonsingular affine matrices admitted by M3 normal handling.
/// Mirroring, nonuniform scale and shear are supported. Raw draws keep their wider domain.
pub fn validate_transform(model: Mat4) -> Result<(), Error> {
    if !crate::normal::NormalTransform::new(model).valid() {
        return Err(Error::InvalidInput(
            "virtual transforms require finite nonsingular affine matrices with condition estimate <= 1e4",
        ));
    }
    Ok(())
}
/// Conservative sphere/affine scale envelope, with near/interior fail-to-detail behavior.
/// The topological maximum makes every owner's error at least every incoming child's.
/// This is a projected geometric-error heuristic, not a bound on rendered image error.
pub fn project_errors(
    mesh: &VirtualMesh,
    camera: &Camera,
    model: Mat4,
    height: u32,
) -> Result<Vec<f64>, Error> {
    validate_transform(model)?;
    if height == 0 {
        return Err(Error::InvalidInput("zero virtual viewport"));
    }
    let m = model.as_dmat4();
    let linear = glam::DMat3::from_mat4(m);
    let columns = linear.to_cols_array();
    // sqrt(||A||1 * ||A||infinity) bounds the spectral norm, exact for uniform scales.
    let one = (0..3)
        .map(|c| (0..3).map(|r| columns[c * 3 + r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let infinity = (0..3)
        .map(|r| (0..3).map(|c| columns[c * 3 + r].abs()).sum::<f64>())
        .fold(0., f64::max);
    let scale = (one * infinity).sqrt() * (1. + 64. * f64::from(f32::EPSILON));
    let view = camera.view().as_dmat4();
    let focal = f64::from(height) * 0.5 / (f64::from(camera.fov_y()) * 0.5).tan();
    let near = f64::from(camera.near());
    let mut errors = Vec::with_capacity(mesh.groups.len());
    for group in &mesh.groups {
        if group.simplified.error == f32::MAX {
            errors.push(f64::INFINITY);
            continue;
        }
        let b = group.simplified;
        let center = view
            .transform_point3(m.transform_point3(glam::DVec3::from_array(b.center.map(f64::from))));
        let radius = f64::from(b.radius) * scale;
        let distance = -center.z - radius;
        let error = if !distance.is_finite() || distance <= near {
            f64::INFINITY
        } else {
            f64::from(b.error) * scale * focal / distance
        };
        errors.push(error);
    }
    for cluster in &mesh.clusters {
        if let Some(child) = cluster.refined {
            errors[cluster.owner.index()] =
                errors[cluster.owner.index()].max(errors[child.index()]);
        }
    }
    Ok(errors)
}
