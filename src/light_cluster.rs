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
//! LC3 traces every contributing local light where the shading head binds an acceleration
//! structure: four disk rays at positive source radius, one at zero. Map heads rank by
//! screen influence and energy, fade slot changes, and retain fixed tiles in a six-layer
//! atlas. Maps redraw only when their light placement or intersecting casters change.
use crate::{Camera, Error, Light, Vec3};

pub(crate) const TILES_X: u32 = 32;
pub(crate) const TILES_Y: u32 = 18;
pub(crate) const SLICES: u32 = 24;
/// Lights one cluster stores; more are counted (the count word's high half) but dropped.
pub(crate) const MAX_PER_CLUSTER: u32 = 128;
pub(crate) const CLUSTERS: u32 = TILES_X * TILES_Y * SLICES;
/// Words per cluster: its count, then u16 index pairs.
pub(crate) const STRIDE: u32 = 1 + MAX_PER_CLUSTER / 2;
/// Map slots the 64 KiB uniform can address (six transforms and one tile per slot).
pub(crate) const MAX_SLOTS: u32 = 128;
/// Clusters per row of the list texture, which is `LIST_SIZE` texels of R32Uint.
pub(crate) const ROW: u32 = 4;
pub(crate) const LIST_SIZE: [u32; 2] = [ROW * STRIDE, CLUSTERS / ROW];
/// The `LcFrame` uniform: view, projection, info and far (112 bytes), then one
/// world-to-clip matrix per pooled map layer.
pub(crate) const TILE_OFFSET: usize = 112 + (MAX_SLOTS * 6 * 64) as usize;
pub(crate) const FRAME_BYTES: usize = TILE_OFFSET + MAX_SLOTS as usize * 16;
/// Lights the u16 lists can name; larger sets walk every light.
pub(crate) const MAX_CLUSTERED: usize = 0xffff;
/// Spot lights wider than this half-angle use a six-face slot.
const WIDE_SPOT: f32 = 1.0;

/// Shadows for the most important point and spot lights, chosen every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalShadows {
    /// Map budget, at most 128 local lights, ranked by projected influence and energy.
    /// RT shadows every contributing clustered local light when this is nonzero.
    /// Zero disables automatic local shadows on both paths.
    pub count: u32,
    /// Maximum map-face resolution. The atlas is four times this wide; smaller lights
    /// receive smaller cached tiles. Held tiles keep their resolution to avoid shimmer.
    pub resolution: u32,
    /// Near plane of each local shadow view in metres. Casters closer to the light are
    /// not drawn into its maps (rays have no near plane).
    pub near: f32,
}
impl Default for LocalShadows {
    fn default() -> Self {
        Self {
            count: 128,
            resolution: 1024,
            near: 0.02,
        }
    }
}
impl LocalShadows {
    pub(crate) fn validate(&self, max_resolution: u32) -> Result<(), Error> {
        if self.count > MAX_SLOTS
            || self.resolution == 0
            || self.resolution > max_resolution / 4
            || !self.near.is_finite()
            || self.near <= 0.0
        {
            return Err(Error::InvalidInput(
                "local shadows need count <= 128, resolution <= device maximum / 4 and a positive finite near",
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
    /// Global light index per map slot (None for free slots), or all ray-shadowed lights.
    pub shadowed: Vec<Option<u32>>,
    /// The frame's shadowed lights used pooled maps (false: inline rays, or none).
    pub maps: bool,
    /// Map faces redrawn by this frame, and since creation.
    pub faces_drawn: u32,
    pub total_faces_drawn: u64,
    /// Caster draws submitted to those faces.
    pub caster_draws: u32,
    /// Current map face resolution and fade strength per slot (zero for free slots).
    pub resolutions: Vec<u32>,
    pub strengths: Vec<f32>,
    /// Allocated atlas depth bytes, excluding uniforms; zero for ray-only frames.
    pub atlas_bytes: u64,
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
// per log2 unit. info: x lights, y clustered, z TR1 sample, w restart disk rays. far: x camera
// far, y shadow near.
// shadows: pooled map layers.
struct LcFrame {{ view:mat4x4<f32>, proj:vec4<f32>, info:vec4<u32>, far:vec4<f32>, shadows:array<mat4x4<f32>,{}>, tiles:array<vec4<f32>,{MAX_SLOTS}> }};
",
        MAX_SLOTS * 6
    )
}
pub(crate) const KERNEL: &str = include_str!("../shaders/light_cluster.wgsl");
const API: &str = include_str!("../shaders/light_cluster_api.wgsl");
/// Local-light visibility: hard at radius zero, four disk rays otherwise (LC2).
const RAY: &str = include_str!("../shaders/local_light_ray.wgsl");
/// P-RELIT's per-pixel restart budget for local lights (U-SPECK2, docs/speck2.md), on traced
/// ray stages: a camera receiver the resolve restarts this frame (`rl_restarts`) has no
/// history to average its penumbrae, so its soft local lights trace `LcFrame.info.w` disk
/// rays instead of four. Every other composition gets the empty stub.
const RESTART_BUDGET: &str = "fn rl_local_budget(world:vec3<f32>) { let restart=rl_light_frame.info.w; if restart<=4u { return; } if rl_restarts(world) { rl_local_rays=restart; } }\n";
/// Whether the resolve restarts a static receiver at `world` this frame: the resolve's own
/// tests (shaders/temporal_zones.wgsl) on the same record, for the estimators that give a
/// restarted receiver more samples (local-light disk rays here, glossy strata in
/// shaders/reflections.wgsl). Traced stages only: the others have no frame record.
const RESTARTS: &str = "fn rl_restarts(world:vec3<f32>)->bool { return rl_zone_caster_shadow(world) || rl_zone_relit(world); }\n";
const RESTART_BUDGET_STUB: &str = "fn rl_local_budget(world:vec3<f32>) {}\n";
/// Disk rays a restarted receiver traces per soft local light (`LcFrame.info.w`).
const RESTART_RAYS: u32 = 16;
/// The count in force. Measurement switch: `RENDERLAB_LOCAL_RESTART_RAYS` replaces it for a
/// process (4 or less switches the budget off).
fn restart_rays() -> u32 {
    static RAYS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *RAYS.get_or_init(|| {
        std::env::var("RENDERLAB_LOCAL_RESTART_RAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RESTART_RAYS)
            .min(64)
    })
}
const MAP: &str = "fn rl_local_shadow(light:Light, receiver:Receiver, l:vec3<f32>) -> f32 { let slot=(light.kind.w&0xffffu)-1u; if slot>=LC_SLOTS { return 1.0; } if rl_light_frame.tiles[slot].z<=0.0 { return 1.0; } return mix(1.0,rl_local_shadow_map(light,receiver,l),rl_light_frame.tiles[slot].w); }\n";

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
    // Only traced stages see the frame's restart zones (`primary_frame.zones`), and only ray
    // stages trace disk rays.
    let budget = if traced && !head.is_empty() {
        let zones = crate::renderer::primary_ray_gpu::temporal_reconstruction::zones_source(
            "primary_frame.zones",
        )
        .replace("tr1_", "rl_zone_");
        let budget = if ray_scene(head, traced).is_some() {
            RESTART_BUDGET
        } else {
            RESTART_BUDGET_STUB
        };
        format!("{budget}{RESTARTS}{zones}")
    } else {
        RESTART_BUDGET_STUB.to_owned()
    };
    format!("{}{API}{visibility}{budget}", constants())
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
    select_ranked(lights, view_projection, eye, previous, count, true)
}
#[cfg(test)]
pub(crate) fn select_legacy(
    lights: &[Light],
    vp: glam::Mat4,
    eye: Vec3,
    previous: &[Option<u32>],
    count: u32,
) -> Vec<Option<u32>> {
    select_ranked(lights, vp, eye, previous, count, false)
}
fn select_ranked(
    lights: &[Light],
    view_projection: glam::Mat4,
    eye: Vec3,
    previous: &[Option<u32>],
    count: u32,
    screen_weighted: bool,
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
            let mut score = luminance(candela) / (distance * distance)
                * if screen_weighted {
                    coverage(light, view_projection)
                } else {
                    1.0
                };
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

/// Conservative clipped screen area of the influence sphere, in fractions of the view.
/// It weights shadow allocation only; light culling and programmable incidence are unchanged.
pub(crate) fn coverage(light: &Light, vp: glam::Mat4) -> f32 {
    let Some((position, _, range, _)) = local(light) else {
        return 0.0;
    };
    let Some(radius) = range else {
        return 1.0;
    };
    let clip = vp * position.extend(1.0);
    let rows = vp.transpose();
    let minimum_w = clip.w - radius * rows.w_axis.truncate().length();
    if minimum_w <= 0.0 {
        return 1.0;
    }
    let half = glam::Vec2::new(
        rows.x_axis.truncate().length(),
        rows.y_axis.truncate().length(),
    ) * radius
        / minimum_w;
    let centre = clip.truncate().truncate() / clip.w;
    let lo = (centre - half).clamp(glam::Vec2::splat(-1.0), glam::Vec2::ONE);
    let hi = (centre + half).clamp(glam::Vec2::splat(-1.0), glam::Vec2::ONE);
    let extent = (hi - lo).max(glam::Vec2::ZERO);
    extent.x * extent.y * 0.25
}

/// Slot transitions never replace a nonzero shadow. Outgoing maps remain resident for
/// eight admitted frames; incoming maps then fade in over eight frames.
pub(crate) fn transition(
    previous: &[Option<u32>],
    strengths: &[f32],
    desired: &[Option<u32>],
    count: u32,
) -> (Vec<Option<u32>>, Vec<f32>) {
    let mut slots = vec![None; count as usize];
    let mut next = vec![0.0; count as usize];
    for (slot, light) in previous.iter().take(count as usize).enumerate() {
        let Some(light) = light else {
            continue;
        };
        let strength = strengths.get(slot).copied().unwrap_or(0.0);
        let strength = if desired.contains(&Some(*light)) {
            (strength + 0.125).min(1.0)
        } else {
            (strength - 0.125).max(0.0)
        };
        if strength > 0.0 {
            slots[slot] = Some(*light);
            next[slot] = strength;
        }
    }
    for light in desired.iter().flatten() {
        if !slots.contains(&Some(*light))
            && let Some(slot) = slots.iter().position(Option::is_none)
        {
            slots[slot] = Some(*light);
            // A just-released outgoing light reached zero this frame; the replacement
            // starts at zero as well, avoiding even a one-step discontinuity.
            next[slot] = 0.0;
        }
    }
    (slots, next)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tile {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

pub(crate) fn atlas_extent(maximum: u32) -> u32 {
    (maximum * 4).max(16)
}

/// A small aligned square allocator. Existing tiles never move when the camera moves.
/// Resolution falls back by halves when larger squares cannot fit; no silent eviction.
pub(crate) fn allocate_tile(existing: &[Option<Tile>], maximum: u32, desired: u32) -> Option<Tile> {
    let minimum = (maximum / 8).max(1);
    // Quantize to a shared grid even for non-power-of-two maxima. Otherwise a
    // 1001-square tile can strand thin strips and invalidate the reserved-area bound.
    let extent = atlas_extent(maximum) / minimum * minimum;
    let units = (desired.min(maximum).max(minimum) / minimum).max(1);
    let mut size = minimum * (1 << (31 - units.leading_zeros()));
    let occupied: u64 = existing
        .iter()
        .flatten()
        .map(|t| u64::from(t.size).pow(2))
        .sum();
    let remaining = existing
        .len()
        .saturating_sub(existing.iter().flatten().count() + 1);
    loop {
        let fits_budget =
            occupied + u64::from(size).pow(2) + remaining as u64 * u64::from(minimum).pow(2)
                <= u64::from(extent).pow(2);
        for y in (0..extent).step_by(size as usize).filter(|_| fits_budget) {
            for x in (0..extent).step_by(size as usize) {
                if x + size > extent || y + size > extent {
                    continue;
                }
                let free = existing.iter().flatten().all(|t| {
                    x + size <= t.x || t.x + t.size <= x || y + size <= t.y || t.y + t.size <= y
                });
                if free {
                    return Some(Tile { x, y, size });
                }
            }
        }
        if size <= minimum {
            return None;
        }
        size = (size / 2).max(minimum);
    }
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
    words[23] = restart_rays();
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
