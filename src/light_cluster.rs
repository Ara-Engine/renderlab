//! LC1, many lights (docs/m7-lc1.md): clustered light culling and shadows for the most
//! important local lights.
//!
//! Every lit frame a compute pass bins the lights into the camera's froxel grid
//! (`TILES_X` x `TILES_Y` screen tiles x `SLICES` logarithmic depth slices,
//! shaders/light_cluster.wgsl). The generated shading entries look up their receiver's
//! cluster once, and the program API's light loop (`light_count`, `light_sample`) walks it,
//! so the standard program and user programs get culled lights without source changes.
//! Point and spot lights cull by their range (the inverse-square falloff's windowed
//! cutoff); directional and unlimited-range lights meet every cluster.
//!
//! Each frame the CPU also ranks the local lights by estimated contribution and shadows the
//! first `LocalShadows::count`: by four disk rays per pixel and light (one at radius zero)
//! where the shading module binds an acceleration structure, otherwise from pooled cube
//! and spot maps with blocker-search soft PCF (3x3 PCF at radius zero). Maps redraw only
//! when the light placement or a caster changes. Lights beyond the count are unshadowed.
use crate::{Camera, Error, Light, Vec3};

pub(crate) const TILES_X: u32 = 32;
pub(crate) const TILES_Y: u32 = 18;
pub(crate) const SLICES: u32 = 24;
/// Lights one cluster stores; more are counted (the count word's high half) but dropped.
pub(crate) const MAX_PER_CLUSTER: u32 = 128;
pub(crate) const CLUSTERS: u32 = TILES_X * TILES_Y * SLICES;
/// Words per cluster: its count, then u16 index pairs.
pub(crate) const STRIDE: u32 = 1 + MAX_PER_CLUSTER / 2;
/// Pooled shadow slots the shading header can address (six map layers each).
pub(crate) const MAX_SLOTS: u32 = 16;
/// Clusters per row of the list texture, which is `LIST_SIZE` texels of R32Uint.
pub(crate) const ROW: u32 = 4;
pub(crate) const LIST_SIZE: [u32; 2] = [ROW * STRIDE, CLUSTERS / ROW];
/// The `LcFrame` uniform: view, projection, info and far (112 bytes), then one
/// world-to-clip matrix per pooled map layer.
pub(crate) const FRAME_BYTES: usize = 112 + (MAX_SLOTS * 6 * 64) as usize;
/// Lights the u16 lists can name; larger sets walk every light.
pub(crate) const MAX_CLUSTERED: usize = 0xffff;
/// Spot lights wider than this half-angle use a six-face slot.
const WIDE_SPOT: f32 = 1.0;

/// Shadows for the most important point and spot lights, chosen every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalShadows {
    /// Local lights shadowed per frame, ranked by illuminance at the camera (candela over
    /// squared distance, at least 1 m) among lights whose range meets the view. 0 disables
    /// local shadows; at most 16.
    pub count: u32,
    /// Square resolution of each pooled map face, on frames that use maps.
    pub resolution: u32,
    /// Near plane of each local shadow view in metres. Casters closer to the light are
    /// not drawn into its maps (rays have no near plane).
    pub near: f32,
}
impl Default for LocalShadows {
    fn default() -> Self {
        Self {
            count: 4,
            resolution: 1024,
            near: 0.02,
        }
    }
}
impl LocalShadows {
    pub(crate) fn validate(&self, max_resolution: u32) -> Result<(), Error> {
        if self.count > MAX_SLOTS
            || self.resolution == 0
            || self.resolution > max_resolution
            || !self.near.is_finite()
            || self.near <= 0.0
        {
            return Err(Error::InvalidInput(
                "local shadows need count <= 16, a device-valid nonzero resolution and a positive finite near",
            ));
        }
        Ok(())
    }
}

/// What the last lit frame culled and shadowed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightClusterStats {
    /// The frame binned its lights (false: disabled or more than 65,535 lights).
    pub clustered: bool,
    /// Global light index per pooled shadow slot, or None for a free slot.
    pub shadowed: Vec<Option<u32>>,
    /// The frame's shadowed lights used pooled maps (false: inline rays, or none).
    pub maps: bool,
    /// Map faces redrawn by this frame, and since creation.
    pub faces_drawn: u32,
    pub total_faces_drawn: u64,
    /// Caster draws submitted to those faces.
    pub caster_draws: u32,
}

/// The WGSL constants and frame struct both the culling kernel and the shading API use.
pub(crate) fn constants() -> String {
    format!(
        "const LC_X:u32={TILES_X}u;
const LC_Y:u32={TILES_Y}u;
const LC_Z:u32={SLICES}u;
const LC_CLUSTERS:u32={CLUSTERS}u;
const LC_MAX:u32={MAX_PER_CLUSTER}u;
const LC_STRIDE:u32={STRIDE}u;
const LC_ROW:u32={ROW}u;
const LC_SLOTS:u32={MAX_SLOTS}u;
// view: world to view (forward -Z). proj: x and y projection scales, near (metres), slices
// per log2 unit. info: x lights, y clustered, z TR1 sample. far: x camera far, y shadow near.
// shadows: pooled map layers.
struct LcFrame {{ view:mat4x4<f32>, proj:vec4<f32>, info:vec4<u32>, far:vec4<f32>, shadows:array<mat4x4<f32>,{}> }};
",
        MAX_SLOTS * 6
    )
}
pub(crate) const KERNEL: &str = include_str!("../shaders/light_cluster.wgsl");
const API: &str = include_str!("../shaders/light_cluster_api.wgsl");
/// Local-light visibility: hard at radius zero, four disk rays otherwise (LC2).
const RAY: &str = include_str!("../shaders/local_light_ray.wgsl");
const MAP: &str = "fn rl_local_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 { return rl_local_shadow_map(light,receiver,l); }\n";

/// The acceleration structure a composed head binds for shading, if any: the hardware
/// head's on every stage, the fresh head's only on traced stages (its raster layout has
/// none). The renderer's `maps_needed` mirrors this choice.
pub(crate) fn ray_scene(head: &str, traced: bool) -> Option<&'static str> {
    if head.contains("var hardware_scene:acceleration_structure") {
        Some("hardware_scene")
    } else if traced && head.contains("var fresh_scene:acceleration_structure") {
        Some("fresh_scene")
    } else {
        None
    }
}
/// The shading-side API composed into every program module (src/program.rs).
pub(crate) fn shading_api(head: &str, traced: bool) -> String {
    let visibility = match ray_scene(head, traced) {
        Some(scene) => RAY.replace("LC_SCENE", scene),
        None => MAP.to_owned(),
    };
    format!("{}{API}{visibility}", constants())
}

fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}
/// A local light's position, candela, range and spot (unit travel, outer) parameters.
type Local = (Vec3, [f32; 3], Option<f32>, Option<(Vec3, f32)>);
fn local(light: &Light) -> Option<Local> {
    match *light {
        Light::Directional { .. } => None,
        Light::Point {
            position,
            candela,
            range,
            ..
        } => Some((position, candela, range, None)),
        Light::Spot {
            position,
            travel,
            candela,
            range,
            outer,
            ..
        } => Some((position, candela, range, Some((travel.normalize(), outer)))),
    }
}

/// One frame's choice of shadowed lights: `previous` holds each slot's light, and keeps it
/// while its boosted score still ranks among the first `count` (hysteresis against
/// flicker between near-equal lights). Returns the new slot assignment.
pub(crate) fn select(
    lights: &[Light],
    view_projection: glam::Mat4,
    eye: Vec3,
    previous: &[Option<u32>],
    count: u32,
) -> Vec<Option<u32>> {
    let rows = view_projection.transpose();
    // Left, right, bottom, top and near planes of the reversed-depth clip volume.
    let planes = [
        rows.w_axis + rows.x_axis,
        rows.w_axis - rows.x_axis,
        rows.w_axis + rows.y_axis,
        rows.w_axis - rows.y_axis,
        rows.w_axis - rows.z_axis,
    ];
    let mut scored: Vec<(f32, u32)> = lights
        .iter()
        .enumerate()
        .filter_map(|(index, light)| {
            let (position, candela, range, _) = local(light)?;
            let radius = range.unwrap_or(f32::INFINITY);
            let visible = planes.iter().all(|p| {
                let length = p.truncate().length();
                length == 0.0 || p.truncate().dot(position) + p.w >= -radius * length
            });
            let distance = position.distance(eye).max(1.0);
            let mut score = luminance(candela) / (distance * distance);
            if !visible || score <= 0.0 || !score.is_finite() {
                return None;
            }
            if previous.contains(&Some(index as u32)) {
                score *= 1.25;
            }
            Some((score, index as u32))
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let chosen: Vec<u32> = scored.iter().take(count as usize).map(|s| s.1).collect();
    let mut slots: Vec<Option<u32>> = (0..count as usize)
        .map(|s| {
            previous
                .get(s)
                .copied()
                .flatten()
                .filter(|l| chosen.contains(l))
        })
        .collect();
    for light in chosen {
        if !slots.contains(&Some(light))
            && let Some(free) = slots.iter_mut().find(|s| s.is_none())
        {
            *free = Some(light);
        }
    }
    slots
}

/// Whether a shadowed light takes a six-face slot (points and wide spots).
pub(crate) fn cube(light: &Light) -> bool {
    match *light {
        Light::Spot { outer, .. } => outer > WIDE_SPOT,
        _ => true,
    }
}
/// The slot's world-to-clip matrices with reversed depth (near 1, far 0): a spot light's
/// cone, or the six cube faces in the order hard_shadow's face selection uses
/// (+X, -X, +Y, -Y, +Z, -Z).
pub(crate) fn matrices(light: &Light, near: f32) -> Vec<glam::Mat4> {
    let Some((position, _, range, spot)) = local(light) else {
        return vec![];
    };
    // Unlimited-range lights shadow out to 1 km.
    let far = range.unwrap_or(1000.0).max(near * 2.0);
    let look = |forward: Vec3| {
        let up = if forward.y.abs() > 0.9 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        glam::Mat4::look_at_rh(position, position + forward, up)
    };
    match spot.filter(|_| !cube(light)) {
        Some((travel, outer)) => {
            // A sliver wider than the cone keeps PCF taps at its edge inside the map.
            let projection = glam::Mat4::perspective_rh(2.0 * outer * 1.02, 1.0, far, near);
            vec![projection * look(travel)]
        }
        None => {
            let projection =
                glam::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, far, near);
            [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]
                .into_iter()
                .map(|f| projection * look(f))
                .collect()
        }
    }
}
/// A light's influence sphere (its position and range), for caster culling.
pub(crate) fn sphere(light: &Light) -> Option<(Vec3, f32)> {
    let (position, _, range, _) = local(light)?;
    Some((position, range.unwrap_or(f32::INFINITY)))
}

/// The `LcFrame` uniform the kernel and the shading API read: world-to-view, projection
/// scales, near and slices per log2 unit, (light count, clustered), far, then the pooled
/// map layers' world-to-clip matrices.
pub(crate) fn header(
    camera: &Camera,
    viewport: [u32; 2],
    count: u32,
    enabled: bool,
    layers: &[glam::Mat4],
) -> Vec<u8> {
    let fov = camera.fov_y();
    let h = (0.5 * fov).cos() / (0.5 * fov).sin();
    let w = h / (viewport[0] as f32 / viewport[1] as f32);
    let (near, far) = (camera.near(), camera.far());
    let scale = SLICES as f32 / (far / near).log2().max(1e-6);
    let mut words = vec![0u32; FRAME_BYTES / 4];
    for (k, v) in camera.view().to_cols_array().into_iter().enumerate() {
        words[k] = v.to_bits();
    }
    for (k, v) in [w, h, near, scale].into_iter().enumerate() {
        words[16 + k] = v.to_bits();
    }
    words[20] = count;
    words[21] = u32::from(enabled);
    words[24] = far.to_bits();
    for (layer, m) in layers.iter().enumerate() {
        for (k, v) in m.to_cols_array().into_iter().enumerate() {
            words[28 + layer * 16 + k] = v.to_bits();
        }
    }
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

#[cfg(test)]
/// The CPU reference of one cluster's light list (the kernel's arithmetic in f32), for
/// tests and the scene report.
pub(crate) fn reference_cluster(
    lights: &[Light],
    camera: &Camera,
    viewport: [u32; 2],
    cluster: u32,
) -> Vec<u32> {
    let uniform = header(camera, viewport, lights.len() as u32, true, &[]);
    let f = |k: usize| f32::from_ne_bytes(uniform[k * 4..k * 4 + 4].try_into().unwrap());
    let (px, py, near, scale, far) = (f(16), f(17), f(18), f(19), f(24));
    let depth = |slice: u32| {
        if slice >= SLICES {
            far
        } else {
            near * (slice as f32 / scale).exp2()
        }
    };
    let (tx, ty, tz) = (
        cluster % TILES_X,
        (cluster / TILES_X) % TILES_Y,
        cluster / (TILES_X * TILES_Y),
    );
    let x0 = -1.0 + 2.0 * tx as f32 / TILES_X as f32;
    let x1 = -1.0 + 2.0 * (tx + 1) as f32 / TILES_X as f32;
    let y1 = 1.0 - 2.0 * ty as f32 / TILES_Y as f32;
    let y0 = 1.0 - 2.0 * (ty + 1) as f32 / TILES_Y as f32;
    let (d0, d1) = (
        depth(tz),
        if tz + 1 >= SLICES { far } else { depth(tz + 1) },
    );
    let xs = [x0 * d0, x0 * d1, x1 * d0, x1 * d1].map(|x| x / px);
    let ys = [y0 * d0, y0 * d1, y1 * d0, y1 * d1].map(|y| y / py);
    let lo = Vec3::new(
        xs.into_iter().fold(f32::INFINITY, f32::min),
        ys.into_iter().fold(f32::INFINITY, f32::min),
        -d1,
    );
    let hi = Vec3::new(
        xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
        ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
        -d0,
    );
    let view = camera.view();
    lights
        .iter()
        .enumerate()
        .filter(|(_, light)| {
            let Some((position, _, Some(range), spot)) = local(light) else {
                return true;
            };
            let (mut centre, mut radius) = (position, range);
            if let Some((travel, outer)) = spot {
                let c = outer.cos().clamp(0.0, 1.0);
                if c > std::f32::consts::FRAC_1_SQRT_2 {
                    radius = range / (2.0 * c);
                    centre += travel * radius;
                } else {
                    radius = range * (1.0 - c * c).max(0.0).sqrt();
                    centre += travel * (range * c);
                }
            }
            let s = view.transform_point3(centre);
            let r = radius * 1.001 + 1e-4;
            s.distance_squared(s.clamp(lo, hi)) <= r * r
        })
        .map(|(i, _)| i as u32)
        .collect()
}

#[cfg(test)]
/// The cluster a world point shades with, as rl_lights_prepare computes it, or None
/// outside the grid.
pub(crate) fn cluster_of(camera: &Camera, viewport: [u32; 2], world: Vec3) -> Option<u32> {
    let uniform = header(camera, viewport, 0, true, &[]);
    let f = |k: usize| f32::from_ne_bytes(uniform[k * 4..k * 4 + 4].try_into().unwrap());
    let (px, py, near, scale) = (f(16), f(17), f(18), f(19));
    let v = camera.view().transform_point3(world);
    let depth = -v.z;
    if depth.partial_cmp(&near) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let ndc = [px * v.x / depth, py * v.y / depth];
    if ndc.iter().any(|n| n.abs() > 1.0) {
        return None;
    }
    let tx = (((ndc[0] * 0.5 + 0.5) * TILES_X as f32) as u32).min(TILES_X - 1);
    let ty = (((0.5 - ndc[1] * 0.5) * TILES_Y as f32) as u32).min(TILES_Y - 1);
    let tz = (((depth / near).log2() * scale).max(0.0) as u32).min(SLICES - 1);
    Some((tz * TILES_Y + ty) * TILES_X + tx)
}
