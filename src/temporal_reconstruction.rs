//! TR1 temporal reconstruction of the final lit image (docs/m7-tr1.md), opt-in through
//! `Renderer::set_reconstruction`. Each traced or visibility-buffer lit frame samples every
//! pixel at one sub-pixel offset of a rank-1 lattice, carried by the camera projection
//! (`Camera::jittered`), so both primary paths and every program's shading follow it with
//! no shader edit. On RT renderers, Interactive's eight disk directions also rotate by the
//! frame through the unused `PrimaryFrame.sided.y` word. After the program's shading, one
//! pass (shaders/temporal_resolve.wgsl) continues each pixel's history where its current
//! sample's surface point was in the previous frame, accepted by surface identity and
//! engine events only, and starts revealed pixels from this frame's samples. Every colour
//! history is one surface's; a silhouette pixel keeps a second slot for the other surface of
//! its 3x3 and the occluder's coverage of its footprint (TR1b, docs/m7-tr1b.md).
//!
//! Its histories, weights, coverages and anchors are read only by that pass, next frame
//! (P0's W5); no later stage takes them as input.
use super::*;

/// Temporal reconstruction settings (TR1). The default is off: frames are unjittered and
/// keep no history. Enabled, traced and visibility-buffer lit frames at 1x accumulate;
/// raster and diagnostic frames are unaffected. History costs 40 bytes per pixel plus an
/// 8-byte copy of the frame's samples. The other defaults are the variant measured best on
/// the Q-walk screens (docs/m7-tr1b.md); TR1 is not the renderer default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reconstruction {
    pub enabled: bool,
    /// The history's largest sample weight on still pixels.
    pub window: f32,
    /// Bilinear resampling blur budget v_max (px²): a moving pixel's window is
    /// `1 + blur_budget / sum f(1 - f)` at fractional fetch offset f (T1b's rule).
    pub blur_budget: f32,
    /// Window of moving pixels whose 4x4 history taps are all accepted, which resample by
    /// Catmull-Rom (second-moment blur free) instead of bilinear.
    pub sharp_window: f32,
    /// Window of a moving silhouette's occluder coverage, which the occluder's motion
    /// carries; still silhouettes use `window`.
    pub coverage_window: f32,
    /// Largest effective sample count a revealed pixel's spatial start may claim.
    pub start_window: f32,
    /// Tent radius (pixels) of a revealed pixel's spatial start over the 3x3 samples of its
    /// surface; 0 starts from its own sample.
    pub start_radius: f32,
    /// Sun turn (radians per frame) at which the window falls to one frame; smaller turns
    /// cap it at `turn_angle / turn`. Zero restarts every pixel on any sun turn.
    pub turn_angle: f32,
    /// Catmull-Rom history resampling where all 16 taps are accepted.
    pub catmull_rom: bool,
    /// Revealed pixels start from this frame's 3x3 samples of their surface; otherwise
    /// from their own sample.
    pub spatial_start: bool,
    /// Rotate Interactive's eight disk directions through the pixel's 256-direction
    /// sequence, 8 per frame (RT renderers).
    pub rotate_directions: bool,
    /// When no bilinear tap shows a surface, continue it from the 4x4 ring's taps that do
    /// (the design's +-1 px reverse search), at half weight.
    pub reverse_search: bool,
    /// A moving silhouette observes the occluder's coverage as this frame's tent-weighted
    /// 3x3 share instead of its own sample's hit.
    pub coverage_share: bool,
}
impl Default for Reconstruction {
    fn default() -> Self {
        Self {
            enabled: false,
            window: 256.0,
            blur_budget: 1.0,
            sharp_window: 16.0,
            coverage_window: 16.0,
            start_window: 4.0,
            start_radius: 2.0,
            turn_angle: 0.004,
            catmull_rom: true,
            spatial_start: true,
            rotate_directions: true,
            reverse_search: false,
            coverage_share: false,
        }
    }
}
impl Reconstruction {
    /// TR1 with its measured settings (docs/m7-tr1.md).
    pub fn identity() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
    pub(in crate::renderer) fn validate(&self) -> Result<(), Error> {
        let windows = [
            self.window,
            self.sharp_window,
            self.coverage_window,
            self.start_window,
        ];
        let valid = windows.iter().all(|w| (1.0..=65536.0).contains(w))
            && (0.0..=f32::MAX).contains(&self.blur_budget)
            && (0.0..=f32::MAX).contains(&self.turn_angle)
            && (0.0..=2.0).contains(&self.start_radius);
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidInput(
                "reconstruction windows in [1, 65536]; blur budget and turn angle finite and nonnegative; start radius in [0, 2]",
            ))
        }
    }
}

/// WGSL Tr1Frame.
const FRAME_BYTES: usize = 304;
/// The rank-1 lattice generator of the sample sequence (film_accumulation.wgsl's).
const GENERATOR: u32 = 71;
/// Interactive's disk directions continue by the frame's rotation word. The estimator body
/// is otherwise production's; only the reconstruction's own trace modules carry this edit.
const ROTATION_SEAM: (&str, &str) = (
    "let direction=fresh_direction(j,shift,l,u,v,view.texel.z);",
    "let direction=fresh_direction(j+primary_frame.sided.y,shift,l,u,v,view.texel.z);",
);
const RESOLVE: &str = include_str!("../shaders/temporal_resolve.wgsl");

/// Sample k's offset from the pixel centre in pixels (y down): T1's corner-free 256-point
/// lattice (film_accumulation.wgsl `film_offset`) with one symmetry for the whole image,
/// which a projection can carry. k = 0 is the centre.
pub(in crate::renderer) fn sample_offset(k: u64) -> [f32; 2] {
    let k = (k & 0xff) as u32;
    if k == 0 {
        return [0.0; 2];
    }
    let order = if k < 4 { (0x0231 >> (4 * k)) & 0xf } else { k };
    let j = order.reverse_bits() >> 24;
    [j, (GENERATOR * j) & 0xff].map(|v| (((v + 8) & 0xff) as f32 + 0.5) / 256.0 - 0.5)
}
/// Interactive's direction rotation for sample k: directions 8 (k mod 32) onward.
pub(in crate::renderer) fn rotation(k: u64) -> u32 {
    8 * (k & 31) as u32
}

/// The reconstruction's interactive trace kernels: production's entries compiled from the
/// soft kernel module with `ROTATION_SEAM`. The near-edge check reads no direction, so
/// frames keep production's.
pub(in crate::renderer) struct TraceKernels {
    pub trace: Owned<wgpu::ComputePipeline>,
    pub repair: Option<Owned<wgpu::ComputePipeline>>,
    pub retry: Option<Owned<wgpu::ComputePipeline>>,
    pub full: Option<Owned<wgpu::ComputePipeline>>,
}
/// Pipelines of the reconstruction, compiled by its first frame after each prelude.
pub(in crate::renderer) struct Pipelines {
    pub trace: Option<TraceKernels>,
    pub resolve: Owned<wgpu::ComputePipeline>,
}

pub(super) fn trace_source(soft_kernels: &str) -> Result<String, Error> {
    let (find, replace) = ROTATION_SEAM;
    if soft_kernels.matches(find).count() != 1 {
        return Err(Error::InvalidInput(
            "temporal reconstruction requires the unchanged Interactive direction seam",
        ));
    }
    Ok(soft_kernels.replacen(find, replace, 1))
}
pub(super) fn resolve_source(wgsl: &str) -> Result<String, Error> {
    let [head, _] = non_rt_heads(wgsl)?;
    Ok(format!("{head}\n{RESOLVE}"))
}

impl PrimaryState {
    pub(super) fn reconstruction_layout(
        device: &wgpu::Device,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
    ) -> (Owned<wgpu::BindGroupLayout>, Owned<wgpu::PipelineLayout>) {
        use wgpu::TextureFormat as F;
        let uint = wgpu::TextureSampleType::Uint;
        let float = wgpu::TextureSampleType::Float { filterable: false };
        let bindings = ledger.own(
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("temporal reconstruction"),
                entries: &[
                    texture(7, uint),
                    uniform(10, super::FRAME_BYTES as u64),
                    texture(21, uint),
                    read(40, FRAME_BYTES as u64),
                    texture(41, float),
                    texture(42, float),
                    texture(43, uint),
                    image(44, F::Rgba32Float),
                    image(45, F::Rg32Uint),
                    image(46, F::Rgba16Float),
                    texture(47, uint),
                    image(48, F::Rgba32Uint),
                ],
            }),
            ResourceKind::BindGroupLayout,
        );
        let layout = ledger.own(
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("temporal reconstruction ABI"),
                bind_group_layouts: &[Some(&bindings), Some(arrays)],
                immediate_size: 0,
            }),
            ResourceKind::PipelineLayout,
        );
        (bindings, layout)
    }

    /// The resolve pipeline, and on RT renderers the rotated Interactive kernels for the
    /// production entries `production` has.
    pub(super) fn reconstruction_pipelines(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        resolve: &str,
        trace: Option<(&str, &PrimaryPipelines)>,
    ) -> Pipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("temporal reconstruction"),
            source: wgpu::ShaderSource::Wgsl(resolve.into()),
        });
        let resolve = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("tr1_resolve"),
                layout: Some(&self.reconstruction.1),
                module: &module,
                entry_point: Some("tr1_resolve"),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        let trace = trace.map(|(source, production)| {
            let soft = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("temporal reconstruction Interactive trace"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let eight = [("FRESH_DIRECTIONS", 8.0)];
            let modern = production.interactive_retry.is_some();
            let deferred = production.interactive_repair.is_some();
            TraceKernels {
                trace: self.interactive_entry(
                    device,
                    ledger,
                    &soft,
                    if deferred {
                        "primary_trace_interactive_deferred"
                    } else {
                        "primary_trace_interactive"
                    },
                    &eight,
                ),
                // The modern near-edge check traces no direction: production's is reused.
                repair: (deferred && !modern).then(|| {
                    self.interactive_entry(
                        device,
                        ledger,
                        &soft,
                        "primary_repair_interactive",
                        &eight,
                    )
                }),
                retry: modern.then(|| {
                    self.interactive_entry(
                        device,
                        ledger,
                        &soft,
                        "primary_retry_interactive",
                        &eight,
                    )
                }),
                full: deferred.then(|| self.interactive_pipeline(device, ledger, &soft, &eight)),
            }
        });
        Pipelines { trace, resolve }
    }
}

/// The previous reconstructed frame's inputs.
struct Previous {
    matrix: Mat4,
    jitter: [f32; 2],
    eye: Vec3,
    exposure: f32,
    /// Directional lights' travel directions.
    suns: Vec<Vec3>,
    key: [u8; 32],
}

/// Per-pixel history at the HDR size, ping-ponged by frame: exposed mean and weight
/// (Rgba32Float) and anchors (Rg32Uint), plus the copy of the frame's shaded samples.
pub(in crate::renderer) struct ReconstructionState {
    colour: [Depth; 2],
    anchor: [Depth; 2],
    /// Silhouettes' second slot and occluder coverage (Rgba32Uint).
    layer: [Depth; 2],
    sample: Depth,
    frame: Buffer,
    /// Resolve groups per written side, keyed by the textures and frame buffer they bind.
    groups: Option<(GroupKey, [Owned<wgpu::BindGroup>; 2])>,
    /// The side this frame writes; the other holds the previous frame.
    pub parity: usize,
    previous: Option<Previous>,
    /// The next explicit restart (`cut_reconstruction`).
    pub cut: bool,
    /// The last prepared frame restarted every pixel.
    pub reset: bool,
}
type GroupKey = (wgpu::Texture, wgpu::Texture, wgpu::Texture, wgpu::Buffer);

impl ReconstructionState {
    fn new(r: &Renderer, size: wgpu::Extent3d) -> Self {
        let target = |label, format: wgpu::TextureFormat, usage| {
            let texture = r.ledger.own(
                r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | usage
                        | if cfg!(test) {
                            wgpu::TextureUsages::COPY_SRC
                        } else {
                            wgpu::TextureUsages::empty()
                        },
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let view = r.ledger.own(
                texture.create_view(&Default::default()),
                ResourceKind::TextureView,
            );
            let bytes = u64::from(size.width)
                * u64::from(size.height)
                * u64::from(format.block_copy_size(None).unwrap());
            Depth {
                texture,
                view,
                _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
            }
        };
        use wgpu::{TextureFormat as F, TextureUsages as U};
        let colour = || target("TR1 history colour", F::Rgba32Float, U::STORAGE_BINDING);
        let anchor = || target("TR1 history anchors", F::Rg32Uint, U::STORAGE_BINDING);
        let layer = || target("TR1 silhouette slots", F::Rgba32Uint, U::STORAGE_BINDING);
        Self {
            colour: [colour(), colour()],
            anchor: [anchor(), anchor()],
            layer: [layer(), layer()],
            sample: target("TR1 frame samples", F::Rgba16Float, U::COPY_DST),
            frame: Buffer::new(
                r,
                "TR1 frame state",
                FRAME_BYTES as u64,
                wgpu::BufferUsages::STORAGE,
            ),
            groups: None,
            parity: 0,
            previous: None,
            cut: false,
            reset: true,
        }
    }
    fn size(&self) -> wgpu::Extent3d {
        self.sample.texture.size()
    }
}

impl Renderer {
    /// Identity of every input a reconstructed frame depends on besides the camera, the
    /// eye, exposure and directional-light directions, which TR1 follows: a change restarts
    /// every pixel. Programs, prelude, geometry, bindings, materials, textures, transforms,
    /// light colours and kinds, the environment and every shadow and visibility setting.
    fn reconstruction_key(&self) -> [u8; 32] {
        let u = &self.uploads;
        let state = self.primary.as_ref().unwrap();
        let lights: Vec<_> = self
            .lighting
            .source_lights
            .iter()
            .map(|light| match light {
                crate::Light::Directional { lux, .. } => format!("directional {lux:?}"),
                other => format!("{other:?}"),
            })
            .collect();
        let mut hash = Sha256::new()
            .chain_update(self.revision.0.to_ne_bytes())
            .chain_update(state.program.to_ne_bytes())
            .chain_update(state.geometry_generation.to_ne_bytes())
            .chain_update(state.bindings_generation.to_ne_bytes())
            .chain_update(format!("{:?}", self.shadows.settings).as_bytes())
            .chain_update(format!("{:?}", self.virtual_settings).as_bytes())
            .chain_update(format!("{:?} {:?}", self.soft_mode, self.primary_mode).as_bytes())
            .chain_update(format!("{lights:?}").as_bytes())
            .chain_update(
                self.lighting
                    .pending
                    .environment
                    .map(f32::to_bits)
                    .map(u32::to_ne_bytes)
                    .concat(),
            )
            .chain_update(self.environment.uniform());
        for v in [
            u.environment,
            u.geometry,
            u.lut,
            u.textures,
            u.materials,
            u.normals,
            u.models,
            u.instances,
        ] {
            hash.update(v.to_ne_bytes());
        }
        hash.finalize().into()
    }

    /// Compiles the reconstruction's pipelines for the installed prelude on its first frame
    /// (and again after a prelude reload), so other frames never pay for them.
    pub(in crate::renderer) fn ensure_reconstruction(&mut self) -> Result<(), Error> {
        // Traced primaries on an RT renderer also need the rotated Interactive kernels,
        // which a visibility-buffer frame compiled before did not build.
        let traced =
            self.hardware_shadows.is_some() && self.primary_mode == PrimaryVisibility::Traced;
        if self
            .pipeline
            .primary
            .as_ref()
            .unwrap()
            .reconstruction
            .as_ref()
            .is_some_and(|p| p.trace.is_some() || !traced)
        {
            return Ok(());
        }
        let resolve = resolve_source(&self.source)?;
        let trace = if traced {
            Some(trace_source(&self.primary_sources.as_ref().unwrap().0[1])?)
        } else {
            None
        };
        let scopes = OperationScopes::new(&self.device);
        let production = self.pipeline.primary.as_ref().unwrap();
        let pipelines = self.primary.as_ref().unwrap().reconstruction_pipelines(
            &self.device,
            &self.ledger,
            &resolve,
            trace.as_deref().map(|s| (s, production)),
        );
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        self.pipelines.created += 1 + pipelines.trace.as_ref().map_or(0, |t| {
            1 + u64::from(t.repair.is_some())
                + u64::from(t.retry.is_some())
                + u64::from(t.full.is_some())
        });
        self.pipeline.primary.as_mut().unwrap().reconstruction = Some(pipelines);
        Ok(())
    }

    /// Publishes one reconstructed frame after `prepare_primary` (and the visibility
    /// buffer's): allocates history at the HDR size, decides restarts and window caps from
    /// engine state, writes the frame words and binds the frame's targets. `camera` is the
    /// frame's jittered camera; `index` its sample.
    pub(in crate::renderer) fn prepare_reconstruction(&mut self, camera: &Camera, index: u64) {
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        if self
            .reconstruction_state
            .as_ref()
            .is_none_or(|s| s.size() != size)
        {
            let cut = self.reconstruction_state.as_ref().is_some_and(|s| s.cut);
            let mut next = ReconstructionState::new(self, size);
            next.cut = cut;
            self.reconstruction_state = Some(next);
        }
        let key = self.reconstruction_key();
        let settings = self.reconstruction;
        let matrix = camera.unjittered().matrix(size.width, size.height);
        let eye = camera.view().inverse().w_axis.truncate();
        let exposure = self.lighting.pending.exposure;
        let suns: Vec<Vec3> = self
            .lighting
            .source_lights
            .iter()
            .filter_map(|l| match l {
                crate::Light::Directional { travel, .. } => Some(travel.normalize()),
                _ => None,
            })
            .collect();
        let background = self.lit_clear().map(|v| v as f32);
        let pixel_angle = 2.0 * (0.5 * camera.fov_y()).tan() / size.height as f32;
        let state = self.reconstruction_state.as_mut().unwrap();
        let continued = state
            .previous
            .as_ref()
            .filter(|p| p.key == key && p.suns.len() == suns.len())
            .map(|p| {
                // The largest directional-light turn since the previous frame.
                let turn = p
                    .suns
                    .iter()
                    .zip(&suns)
                    // Chord angle: exactly zero for an unchanged direction, where acos of
                    // a rounded unit dot product is not.
                    .map(|(a, b)| 2.0 * (0.5 * (*a - *b).length()).min(1.0).asin())
                    .fold(0.0, f32::max);
                (p.matrix, p.eye, p.exposure, turn, p.jitter)
            });
        // A sun turn caps the window at turn_angle / turn; a turn past turn_angle (any turn
        // when it is zero) restarts every pixel.
        let turn = continued.map_or(0.0, |c| c.3);
        let event_window = if turn > 0.0 {
            settings.turn_angle / turn
        } else {
            settings.window
        };
        state.reset = continued.is_none() || state.cut || event_window < 1.0;
        state.cut = false;
        let jitter = camera.jitter();
        let (previous_matrix, previous_eye, scale, previous_jitter) = match continued {
            Some((m, e, x, _, j)) if !state.reset => (m, e, exposure / x, j),
            _ => (matrix, eye, 1.0, jitter),
        };
        state.parity = 1 - state.parity;
        let options = u32::from(settings.catmull_rom)
            | u32::from(settings.spatial_start) << 1
            | u32::from(settings.reverse_search) << 2
            | u32::from(settings.coverage_share) << 3;
        let mut bytes = Vec::with_capacity(FRAME_BYTES);
        bytes.extend_from_slice(&matrix_bytes(previous_matrix));
        bytes.extend_from_slice(&matrix_bytes(matrix));
        bytes.extend_from_slice(&matrix_bytes(previous_matrix.inverse()));
        let floats: [f32; 24] = [
            previous_eye.x,
            previous_eye.y,
            previous_eye.z,
            pixel_angle,
            eye.x,
            eye.y,
            eye.z,
            previous_jitter[0],
            jitter[0],
            jitter[1],
            scale,
            previous_jitter[1],
            background[0],
            background[1],
            background[2],
            1.0,
            settings.window,
            event_window.max(1.0),
            settings.coverage_window,
            settings.blur_budget,
            settings.sharp_window,
            settings.start_window,
            settings.start_radius,
            0.0,
        ];
        for f in floats {
            bytes.extend_from_slice(&f.to_ne_bytes());
        }
        // Nothing moved: the unjittered view-projection is bit-equal (moved placements and
        // every other scene edit restart through the key).
        let still = !state.reset
            && previous_matrix.to_cols_array().map(f32::to_bits)
                == matrix.to_cols_array().map(f32::to_bits);
        for w in [
            u32::from(state.reset),
            options,
            u32::from(still),
            index as u32,
        ] {
            bytes.extend_from_slice(&w.to_ne_bytes());
        }
        debug_assert_eq!(bytes.len(), FRAME_BYTES);
        self.queue.write_buffer(&state.frame.value, 0, &bytes);
        self.uploads.view_settings += FRAME_BYTES as u64;
        self.uploads.write_calls += 1;
        state.previous = Some(Previous {
            matrix,
            jitter,
            eye,
            exposure,
            suns,
            key,
        });
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let group_key: GroupKey = (
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            (*hdr.texture).clone(),
            (**frame).clone(),
        );
        let state = self.reconstruction_state.as_ref().unwrap();
        if state.groups.as_ref().is_none_or(|(k, _)| *k != group_key) {
            let groups = self.reconstruction_groups();
            self.reconstruction_state.as_mut().unwrap().groups = Some((group_key, groups));
        }
    }

    fn reconstruction_groups(&self) -> [Owned<wgpu::BindGroup>; 2] {
        use wgpu::BindingResource as R;
        let state = self.reconstruction_state.as_ref().unwrap();
        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let frame = &primary.frame.as_ref().unwrap().0.value;
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        [0, 1].map(|written| {
            let read = 1 - written;
            let entries = [
                (7, R::TextureView(&targets.hit.view)),
                (10, frame.as_entire_binding()),
                (21, R::TextureView(&targets.tiles.view)),
                (40, state.frame.value.as_entire_binding()),
                (41, R::TextureView(&state.sample.view)),
                (42, R::TextureView(&state.colour[read].view)),
                (43, R::TextureView(&state.anchor[read].view)),
                (44, R::TextureView(&state.colour[written].view)),
                (45, R::TextureView(&state.anchor[written].view)),
                (47, R::TextureView(&state.layer[read].view)),
                (48, R::TextureView(&state.layer[written].view)),
                (46, R::TextureView(&hdr.view)),
            ]
            .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
            self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("temporal reconstruction"),
                    layout: &primary.reconstruction.0,
                    entries: &entries,
                }),
                ResourceKind::BindGroup,
            )
        })
    }

    /// The reconstruction pass after the frame's shading: copies the shaded samples, then
    /// resolves every pixel into the HDR target and the written history side.
    pub(in crate::renderer) fn encode_reconstruction(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.reconstruction_state.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let size = state.size();
        encoder.copy_texture_to_texture(
            hdr.texture.as_image_copy(),
            state.sample.texture.as_image_copy(),
            size,
        );
        let primary = self.primary.as_ref().unwrap();
        let pipelines = self.pipeline.primary.as_ref().unwrap();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("temporal reconstruction"),
            timestamp_writes,
        });
        pass.set_pipeline(&pipelines.reconstruction.as_ref().unwrap().resolve);
        pass.set_bind_group(0, &*state.groups.as_ref().unwrap().1[state.parity], &[]);
        pass.set_bind_group(1, primary.virtual_geometry().unwrap().1, &[]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_the_film_lattice_with_one_symmetry() {
        assert_eq!(sample_offset(0), [0.0; 2]);
        // T1's 16-point prefix (rows 8, 12, 4, 0, then bit-reversed), moved off the edges.
        let rows = [8u32, 12, 4, 0, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
        for (k, row) in rows.into_iter().enumerate().skip(1) {
            let expected = [16 * row, 16 * (7 * row % 16)]
                .map(|v| ((v + 8) & 0xff) as f32 / 256.0 + 0.5 / 256.0 - 0.5);
            assert_eq!(sample_offset(k as u64), expected, "k={k}");
        }
        for k in 0..512 {
            assert!(sample_offset(k).iter().all(|v| v.abs() < 0.5), "k={k}");
            assert_eq!(sample_offset(k), sample_offset(k + 256));
        }
        assert_eq!(rotation(0), 0);
        assert_eq!(rotation(33), 8);
    }

    #[test]
    fn jittered_projection_moves_each_pixel_sample_by_the_offset() {
        let camera = Camera::look_at(
            Vec3::new(0.3, 1.0, 4.0),
            Vec3::ZERO,
            Vec3::Y,
            0.8,
            0.05,
            200.0,
        )
        .unwrap();
        let [w, h] = [64u32, 36];
        assert_eq!(
            camera
                .jittered([0.0; 2])
                .matrix(w, h)
                .to_cols_array()
                .map(f32::to_bits),
            camera.matrix(w, h).to_cols_array().map(f32::to_bits)
        );
        let offset = [0.25, -0.375];
        let jittered = camera.jittered(offset);
        assert_eq!(jittered.jitter(), offset);
        assert_eq!(jittered.unjittered().jitter(), [0.0; 2]);
        let pixel = |m: Mat4, p: Vec3| {
            let c = m * p.extend(1.0);
            [
                (c.x / c.w + 1.0) * 0.5 * w as f32,
                (1.0 - c.y / c.w) * 0.5 * h as f32,
            ]
        };
        for p in [
            Vec3::ZERO,
            Vec3::new(0.4, -0.2, 0.7),
            Vec3::new(-1.0, 0.5, -2.0),
        ] {
            let [a, b] = [
                pixel(camera.matrix(w, h), p),
                pixel(jittered.matrix(w, h), p),
            ];
            // The point appears `offset` up-left, so the pixel centre that sees it now
            // sampled it at centre + offset.
            assert!((a[0] - b[0] - offset[0]).abs() < 1e-4, "{a:?} {b:?}");
            assert!((a[1] - b[1] - offset[1]).abs() < 1e-4, "{a:?} {b:?}");
        }
    }

    #[test]
    fn reconstruction_modules_validate_without_ray_queries() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let resolve = resolve_source(prelude).unwrap();
        assert!(!resolve.contains("ray_query") && !resolve.contains("rayQuery"));
        let kernels = shader_sources(prelude).unwrap();
        let trace = trace_source(&kernels[1]).unwrap();
        assert!(trace.contains("fresh_direction(j+primary_frame.sided.y"));
        for source in [resolve, trace] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
        // Production modules never see the rotation.
        assert!(!kernels[1].contains("sided.y"));
    }

    #[test]
    fn settings_validate() {
        assert!(Reconstruction::default().validate().is_ok());
        assert!(!Reconstruction::default().enabled);
        assert!(Reconstruction::identity().enabled);
        for bad in [
            Reconstruction {
                window: 0.5,
                ..Default::default()
            },
            Reconstruction {
                blur_budget: f32::NAN,
                ..Default::default()
            },
            Reconstruction {
                turn_angle: -1.0,
                ..Default::default()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }
}
