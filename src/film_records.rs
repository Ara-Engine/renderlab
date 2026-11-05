//! T2 world-anchored sample records (docs/m6-motion-aa.md): temporal film accumulation under
//! camera motion, opt-in through `FilmMotion::Records4`/`Records8` with
//! `SoftShadows::Temporal`. Each pixel keeps K exact camera-ray samples as records: the program
//! output, the sample's position in the pixel and its depth. When the camera moves, records
//! move as points to wherever their surface point now projects and count only in the pixel they
//! land in, after geometric validation against this frame's fresh hits. Nothing is
//! interpolated, so a solid footprint can only collect solid samples. Static frames keep T1a's
//! cumulative mean byte for byte: the T1a pipelines run unchanged, the records stay as the last
//! moving frame left them, and `film_seed` starts T1a's mean from them when motion stops.
//!
//! Modules: the motion kernel (the soft kernels with the T1 ray seams, the `MOTION_*` seams,
//! the T1 sequence and the record passes) and, per program, the motion shading (the T1
//! temporal shading head, the program, `film_records_store.wgsl` and the entry with every
//! store routed through it). Exact, Interactive, hard and T1a modules are never edited.
//! Moving frames shade the fresh sample with the program once per pixel, as Interactive does.
use super::*;
use crate::program::{self, Composed, Stage};
use film_accumulation::{FILM_RAY_SEAM, FILM_ROTATION_SEAM, FILM_SEQUENCE, seam};

/// Record encoding, shared by both modules.
const SHARED: &str = include_str!("../shaders/film_records.wgsl");
/// Motion trace hooks and the validation, reset and seed passes.
const KERNEL: &str = include_str!("../shaders/film_records_kernel.wgsl");
/// `film_motion_store`, between the program and the motion shading entry.
const STORE: &str = include_str!("../shaders/film_records_store.wgsl");
/// Bytes of WGSL `FilmMotion`: three matrices, eye, background, size and flags.
pub(super) const MOTION_BYTES: usize = 3 * 64 + 4 * 16;
/// Records carry placement + 1 in 16 bits.
const MAX_PLACEMENTS: usize = 0xfffe;
/// Test diagnostics end with three record probes of 8,192 u32 (support/film_t2_probe.wgsl).
pub(crate) const PROBE_BYTES: u64 = 3 * 8192 * 4;

/// The motion trace: the fused Interactive trace choosing each pixel's sample from the
/// records that land in it (mode 0), or retracing cold pixels at the centre (mode 1), and
/// publishing each pixel's fresh plane.
const MOTION_SEAMS: [(&str, &str); 5] = [
    (
        "    let inside=all(id.xy<size);\n",
        "    let inside=all(id.xy<size);\n    if inside {film_motion_begin(id.xy,size);}\n",
    ),
    (
        "if inside && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {trace=primary_closest(id.xy,size);}",
        "if inside && !film_skip && all(id.xy>=scene.xy) && all(id.xy<scene.zw) {trace=primary_closest(id.xy,size);}",
    ),
    (
        "    if all(id.xy==group.xy*PRIMARY_TILE) {textureStore(primary_tile_output,group.xy,vec4<u32>(covered));}\n    if !inside || covered==0u {return;}\n",
        "    if all(id.xy==group.xy*PRIMARY_TILE) {film_tile(group.xy,covered);}\n    if !inside {return;}\n    if film_skip {film_retrace_tile(vec2<i32>(id.xy),covered);return;}\n    if covered==0u && film_motion.size.z==0u {film_plane(vec2<i32>(id.xy),trace,FRESH_SKIP);return;}\n",
    ),
    (
        "        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n",
        "        let receiver=primary_receiver(trace.point,trace.positions,trace.record.x-1u,p);\n        film_geometric=receiver.geometric;\n",
    ),
    (
        "    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));\n}",
        "    textureStore(fresh_output,p,vec4<u32>(select((256u/FRESH_DIRECTIONS)*visible,256u,visible==FRESH_SKIP)));\n    film_plane(p,trace,visible);\n}",
    ),
];
/// The motion shading entry's stores, each routed through `film_motion_store`, and the
/// sample selection before its gradients read the camera ray.
const ENTRY_SEAMS: [(&str, &str); 6] = [
    (
        "{textureStore(primary_hdr,vec2<i32>(p),primary_frame.clear);}",
        "{film_motion_clear(vec2<i32>(p));}",
    ),
    (
        "textureStore(primary_hdr,vec2<i32>(id.xy),primary_frame.clear);",
        "film_motion_clear(vec2<i32>(id.xy));",
    ),
    (
        "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);",
        "let pixel=vec2<i32>(id);let record=textureLoad(primary_hit,pixel,0);\n    film_motion_begin(id,size.x);",
    ),
    (
        "if record.x==0u {textureStore(primary_hdr,pixel,primary_frame.clear);return;}",
        "if record.x==0u {film_motion_store(pixel,film_clear_colour().rgb,0u);return;}",
    ),
    (
        "{textureStore(primary_hdr,pixel,vec4<f32>(0.0));return;}",
        "{film_motion_store(pixel,vec3<f32>(0.0),record.x);return;}",
    ),
    (
        "textureStore(primary_hdr,pixel,vec4<f32>(rl_finite(shade(surface_output,surface_input)),1.0));",
        "film_motion_store(pixel,rl_finite(shade(surface_output,surface_input)),record.x);",
    ),
];

/// The motion kernel module from the soft kernel module (`shader_sources()[1]`).
pub(crate) fn kernel_source(soft_kernels: &str) -> Result<String, Error> {
    let mut source = soft_kernels.to_owned();
    for edit in [FILM_RAY_SEAM, FILM_ROTATION_SEAM]
        .into_iter()
        .chain(MOTION_SEAMS)
    {
        source = seam(&source, edit)?;
    }
    Ok(format!("{source}\n{FILM_SEQUENCE}\n{SHARED}\n{KERNEL}"))
}

/// One program's motion shading module on `head` (`film_accumulation::shading_head`): the
/// traced composition with every store of its entry routed through `film_motion_store`. The
/// program's byte range, which diagnostics map lines through, is unchanged.
pub(crate) fn compose_shading(head: &str, source: &str, parameters: bool) -> Composed {
    let mut composed = program::compose(head, source, Stage::Traced, parameters);
    let mut entry = composed.source.split_off(composed.program.end);
    for edit in ENTRY_SEAMS {
        // The entry is engine text after the program, so these always apply.
        entry = seam(&entry, edit).expect("motion shading entry seam");
    }
    composed.source.push('\n');
    composed.source.push_str(SHARED);
    composed.source.push_str(STORE);
    composed.source.push_str(&entry);
    composed
}

/// One pair of timestamps around two passes: the first pass's begin, the second's end.
fn split_stamps(
    stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) -> [Option<wgpu::ComputePassTimestampWrites<'_>>; 2] {
    let Some(t) = stamps else {
        return [None, None];
    };
    [
        Some(wgpu::ComputePassTimestampWrites {
            query_set: t.query_set,
            beginning_of_pass_write_index: t.beginning_of_pass_write_index,
            end_of_pass_write_index: None,
        }),
        Some(wgpu::ComputePassTimestampWrites {
            query_set: t.query_set,
            beginning_of_pass_write_index: None,
            end_of_pass_write_index: t.end_of_pass_write_index,
        }),
    ]
}

/// The bind groups a record pass uses besides its own group 0.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Binds {
    Own,
    /// The draw geometry (group 1).
    Geometry,
    /// The draw geometry and the lighting (group 3).
    Lit,
}

fn array_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
    )
}
fn array_image(binding: u32) -> wgpu::BindGroupLayoutEntry {
    entry(
        binding,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba32Uint,
            view_dimension: wgpu::TextureViewDimension::D2Array,
        },
    )
}
fn read_write(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    buffer(
        binding,
        wgpu::BufferBindingType::Storage { read_only: false },
        size,
    )
}

/// Pipeline layouts of the record passes and the motion shading.
pub(super) struct RecordLayouts {
    gap_bindings: Owned<wgpu::BindGroupLayout>,
    trace_bindings: Owned<wgpu::BindGroupLayout>,
    validate_bindings: Owned<wgpu::BindGroupLayout>,
    init_bindings: Owned<wgpu::BindGroupLayout>,
    seed_bindings: Owned<wgpu::BindGroupLayout>,
    shade_bindings: Owned<wgpu::BindGroupLayout>,
    gap: Owned<wgpu::PipelineLayout>,
    trace: Owned<wgpu::PipelineLayout>,
    validate: Owned<wgpu::PipelineLayout>,
    init: Owned<wgpu::PipelineLayout>,
    seed: Owned<wgpu::PipelineLayout>,
    shade: Owned<wgpu::PipelineLayout>,
}
impl RecordLayouts {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
        materials: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::TextureFormat as F;
        let uint = wgpu::TextureSampleType::Uint;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let motion = read(31, MOTION_BYTES as u64);
        let planes = read_write(32, 16);
        let gap_bindings = group(
            "film records gap choice",
            &[
                uniform(10, FRAME_BYTES as u64),
                motion,
                planes,
                texture(33, uint),
                array_texture(34),
            ],
        );
        let trace_bindings = group(
            "film records trace",
            &[
                scene(TRACE_SCENE),
                scene(2),
                uniform(10, FRAME_BYTES as u64),
                image(13, F::Rgba32Uint),
                image(5, F::R32Uint),
                image(20, F::R32Uint),
                motion,
                planes,
                texture(33, uint),
                array_texture(34),
            ],
        );
        let validate_bindings = group(
            "film records validation",
            &[
                scene(2),
                uniform(10, FRAME_BYTES as u64),
                motion,
                planes,
                texture(33, uint),
                array_texture(34),
                array_image(35),
                read_write(36, 8),
            ],
        );
        let init_bindings = group(
            "film records reset",
            &[
                texture(7, uint),
                texture(21, uint),
                motion,
                planes,
                image(37, F::Rgba32Uint),
                texture(38, wgpu::TextureSampleType::Float { filterable: false }),
                array_image(35),
            ],
        );
        let seed_bindings = group(
            "film records seed",
            &[
                motion,
                planes,
                texture(33, uint),
                array_texture(34),
                image(39, F::Rgba32Float),
                image(40, F::Rg32Uint),
                image(41, F::R32Uint),
            ],
        );
        let shade_bindings = group(
            "film records shading",
            &[
                uniform(0, 80),
                texture(7, uint),
                uniform(10, FRAME_BYTES as u64),
                image(18, F::Rgba16Float),
                texture(21, uint),
                read(22, 16),
                texture(6, uint),
                read(32, 16),
                array_texture(42),
                image(37, F::Rgba32Uint),
            ],
        );
        let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        Self {
            gap: layout("film records gap choice ABI", &[Some(&gap_bindings)]),
            trace: layout(
                "film records trace ABI",
                &[Some(&trace_bindings), Some(arrays), None, Some(lighting)],
            ),
            validate: layout(
                "film records validation ABI",
                &[Some(&validate_bindings), Some(arrays)],
            ),
            init: layout(
                "film records reset ABI",
                &[Some(&init_bindings), Some(arrays)],
            ),
            seed: layout("film records seed ABI", &[Some(&seed_bindings)]),
            shade: layout(
                "film records shading ABI",
                &[
                    Some(&shade_bindings),
                    Some(arrays),
                    Some(materials),
                    Some(lighting),
                ],
            ),
            gap_bindings,
            trace_bindings,
            validate_bindings,
            init_bindings,
            seed_bindings,
            shade_bindings,
        }
    }
}

/// The record pipelines of one prelude revision and record count: the motion trace,
/// validation, reset, seed and gap passes from one kernel module, and the motion shading of
/// the last program that shaded a moving frame.
pub(super) struct RecordPipelines {
    /// (renderer revision, K) the kernel pipelines were compiled for.
    kernel_key: (u64, u32),
    /// Motion trace, validation, reset, seed and gap choice.
    kernel: [Owned<wgpu::ComputePipeline>; 5],
    /// (renderer revision, program, K) and that program's motion shading.
    shading: Option<((u64, u64, u32), Owned<wgpu::ComputePipeline>)>,
}

/// One side of the records: fresh records (slot 0, one per pixel) and moved records
/// (slots 1..K-1, one array layer each), at the HDR size.
struct RecordSide {
    fresh: Depth,
    moved: Depth,
}

/// Per-pixel records, ping-ponged by moving frame, and this frame's fresh planes.
/// 32 K + 16 bytes per pixel: 0.30 GB (K = 4) or 0.56 GB (K = 8) at 1080p, of which the
/// records are 32 K bytes.
pub(in crate::renderer) struct FilmRecords {
    /// K, records per pixel.
    pub count: u32,
    sides: [RecordSide; 2],
    planes: Buffer,
    /// `FilmMotion` for the motion trace (and every other pass) and for the centre retrace.
    motion: [Buffer; 2],
    /// Test diagnostics per pixel (film_records_kernel.wgsl `film_diagnostics`).
    diagnostics: Buffer,
    /// The side holding the latest records.
    pub side: usize,
    /// The FilmMotion bytes the motion buffers hold (mode word aside).
    written: Option<[u8; MOTION_BYTES]>,
    /// Temporal frames since the reset: the implied records' frame index.
    frames: u32,
    /// This frame moves the records (camera motion), or seeds T1a from them (motion stopped).
    pub moving: bool,
    pub seed: bool,
    /// Per written side: trace mode 0, trace mode 1, validation, shading, reset, gap choice;
    /// and seed groups per (record side, T1a read side).
    #[allow(clippy::type_complexity)]
    groups: Option<(
        u64,
        [[Owned<wgpu::BindGroup>; 6]; 2],
        [[Owned<wgpu::BindGroup>; 2]; 2],
    )>,
}
impl FilmRecords {
    pub(super) fn new(r: &Renderer, size: wgpu::Extent3d, count: u32) -> Self {
        let target = |label, layers: u32| {
            let texture = r.ledger.own(
                r.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        depth_or_array_layers: layers,
                        ..size
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Uint,
                    usage: wgpu::TextureUsages::STORAGE_BINDING
                        | wgpu::TextureUsages::TEXTURE_BINDING
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
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(if layers == 1 {
                        wgpu::TextureViewDimension::D2
                    } else {
                        wgpu::TextureViewDimension::D2Array
                    }),
                    ..Default::default()
                }),
                ResourceKind::TextureView,
            );
            let bytes = 16 * u64::from(size.width) * u64::from(size.height) * u64::from(layers);
            Depth {
                texture,
                view,
                _allocation: r.ledger.allocate(AllocationKind::Target, bytes),
            }
        };
        let side = || RecordSide {
            fresh: target("film fresh records", 1),
            moved: target("film moved records", count - 1),
        };
        let pixels = u64::from(size.width) * u64::from(size.height);
        let storage = wgpu::BufferUsages::STORAGE
            | if cfg!(test) {
                wgpu::BufferUsages::COPY_SRC
            } else {
                wgpu::BufferUsages::empty()
            };
        let motion = || Buffer::new(r, "film motion frame", MOTION_BYTES as u64, storage);
        Self {
            count,
            sides: [side(), side()],
            planes: Buffer::new(r, "film fresh planes", 16 * pixels, storage),
            motion: [motion(), motion()],
            diagnostics: Buffer::new(
                r,
                "film record diagnostics",
                if cfg!(test) { 8 * pixels + PROBE_BYTES } else { 8 },
                storage,
            ),
            side: 0,
            written: None,
            frames: 0,
            moving: false,
            seed: false,
            groups: None,
        }
    }
    /// The fresh and moved record textures of `side`, and the diagnostics buffer (tests).
    #[cfg(test)]
    pub fn test_resources(&self, side: usize) -> (&wgpu::Texture, &wgpu::Texture, &wgpu::Buffer) {
        (
            &self.sides[side].fresh.texture,
            &self.sides[side].moved.texture,
            &self.diagnostics.value,
        )
    }
    /// This frame's fresh planes (tests).
    #[cfg(test)]
    pub fn test_planes(&self) -> &wgpu::Buffer {
        &self.planes.value
    }
}

impl PrimaryState {
    /// The record kernel pipelines from one `kernel_source` module, with K records.
    fn record_kernel(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        source: &str,
        count: u32,
    ) -> [Owned<wgpu::ComputePipeline>; 5] {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("film records kernels"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let l = &self.record_layouts;
        [
            ("primary_trace_interactive", &l.trace),
            ("film_validate", &l.validate),
            ("film_records_init", &l.init),
            ("film_seed", &l.seed),
            ("film_gap", &l.gap),
        ]
        .map(|(entry, layout)| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[
                            ("FRESH_DIRECTIONS", 8.0),
                            ("FILM_RECORDS", f64::from(count)),
                        ],
                        ..Default::default()
                    },
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        })
    }
    /// A program's motion shading pipeline from its `compose_shading` module, with K records.
    pub fn record_shade_pipeline(
        &self,
        device: &wgpu::Device,
        ledger: &Ledger,
        module: &wgpu::ShaderModule,
        count: u32,
    ) -> Owned<wgpu::ComputePipeline> {
        ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("film records program shading"),
                layout: Some(&self.record_layouts.shade),
                module,
                entry_point: Some("primary_shade_tiles"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("FILM_RECORDS", f64::from(count))],
                    ..Default::default()
                },
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        )
    }
}

impl Renderer {
    /// The record kernels and the frame program's motion shading for this revision and K,
    /// compiled when a record frame first needs them (other modes never pay for them).
    pub(in crate::renderer) fn ensure_record_pipelines(&mut self, count: u32) -> Result<(), Error> {
        let revision = self.revision.0;
        let program = self.primary.as_ref().unwrap().program;
        let current = self.primary.as_ref().unwrap().record_pipelines.as_ref();
        let kernel_stale = current.is_none_or(|p| p.kernel_key != (revision, count));
        let shading_stale = kernel_stale
            || current
                .unwrap()
                .shading
                .as_ref()
                .is_none_or(|(key, _)| *key != (revision, program, count));
        if !kernel_stale && !shading_stale {
            return Ok(());
        }
        let (kernels, heads) = self.primary_sources.as_ref().unwrap();
        let kernel_text = kernel_stale
            .then(|| kernel_source(&kernels[1]))
            .transpose()?;
        let slot = self.program_slot(program);
        let head = film_accumulation::shading_head(&heads[1])?;
        let composed = compose_shading(
            &head,
            &slot.source,
            crate::program::declares_parameters(&slot.source),
        );
        let scopes = OperationScopes::new(&self.device);
        let state = self.primary.as_ref().unwrap();
        let kernel = kernel_text
            .map(|source| state.record_kernel(&self.device, &self.ledger, &source, count));
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(slot.label.as_str()),
                source: wgpu::ShaderSource::Wgsl(composed.source.as_str().into()),
            });
        let shading = state.record_shade_pipeline(&self.device, &self.ledger, &module, count);
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => Error::Shader(error.to_string()),
                _ => self.fail(error.to_string()),
            });
        }
        self.pipelines.created += 1 + 5 * u64::from(kernel.is_some());
        let state = self.primary.as_mut().unwrap();
        match (kernel, state.record_pipelines.as_mut()) {
            (None, Some(p)) => p.shading = Some(((revision, program, count), shading)),
            (Some(kernel), _) => {
                state.record_pipelines = Some(RecordPipelines {
                    kernel_key: (revision, count),
                    kernel,
                    shading: Some(((revision, program, count), shading)),
                })
            }
            (None, None) => unreachable!("the kernel is compiled when none exists"),
        }
        Ok(())
    }

    /// Publishes a record frame after `prepare_film` set its reset, motion and parity: the
    /// record side it writes, the FilmMotion state of both trace modes and the bind groups.
    /// `previous` and `current` are the view-projections with the far plane at infinity
    /// (`Camera::infinite_matrix`), whose far plane depth is `far_depth`.
    pub(in crate::renderer) fn prepare_records(
        &mut self,
        previous: Mat4,
        current: Mat4,
        eye: Vec3,
        angle: f32,
        far_depth: f32,
        placements: usize,
    ) -> Result<(), Error> {
        if placements > MAX_PLACEMENTS {
            return Err(Error::InvalidInput(
                "film records support at most 65,534 placements",
            ));
        }
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let clear = self.lit_clear();
        let state = self.primary.as_mut().unwrap();
        #[cfg(test)]
        let clear = state.test_film_background.unwrap_or(clear);
        let generation = state.bindings_generation;
        let film = state.film.as_mut().unwrap();
        let (reset, moving) = (film.reset, film.moving);
        let records = film.records.as_mut().unwrap();
        records.frames = if reset {
            0
        } else {
            records.frames.wrapping_add(1)
        };
        if reset || moving {
            records.side = 1 - records.side;
        }
        // The previous view-projection's inverse composed with this one in f64: records
        // re-expressed through it keep about f32 precision.
        let reproject = (current.as_dmat4() * previous.as_dmat4().inverse()).as_mat4();
        let floats = [
            reproject.to_cols_array(),
            current.to_cols_array(),
            current.as_dmat4().inverse().as_mat4().to_cols_array(),
        ]
        .concat()
        .into_iter()
        .chain(eye.to_array())
        .chain([angle])
        .chain(clear.map(|v| v as f32))
        .chain([far_depth])
        .map(f32::to_bits);
        #[cfg(test)]
        let (diagnose, probe) = (state.test_record_diagnostics, state.test_record_probe);
        #[cfg(not(test))]
        let (diagnose, probe) = (false, [0; 3]);
        let words = [
            size.width,
            size.height,
            0,
            records.frames,
            u32::from(diagnose),
            probe[0],
            probe[1],
            probe[2],
        ];
        let mut bytes = [0; MOTION_BYTES];
        for (dst, v) in bytes.chunks_exact_mut(4).zip(floats.chain(words)) {
            dst.copy_from_slice(&v.to_ne_bytes());
        }
        if records.written != Some(bytes) {
            for (mode, buffer) in records.motion.iter().enumerate() {
                let mut bytes = bytes;
                bytes[3 * 64 + 2 * 16 + 8..3 * 64 + 2 * 16 + 12]
                    .copy_from_slice(&(mode as u32).to_ne_bytes());
                self.queue.write_buffer(&buffer.value, 0, &bytes);
            }
            records.written = Some(bytes);
            self.uploads.view_settings += 2 * MOTION_BYTES as u64;
            self.uploads.write_calls += 2;
        }
        if records.groups.as_ref().is_none_or(|g| g.0 != generation) {
            let (sides, seeds) = self.record_groups();
            let state = self.primary.as_mut().unwrap();
            let records = state.film.as_mut().unwrap().records.as_mut().unwrap();
            if let Some((_, sides, seeds)) = records.groups.replace((generation, sides, seeds)) {
                // Frames in flight may still bind them.
                let old = sides
                    .into_iter()
                    .flatten()
                    .chain(seeds.into_iter().flatten());
                state
                    .retired
                    .push((self.submissions, Retired::RecordGroups(old.collect())));
            }
        }
        Ok(())
    }

    /// Bind groups over the current targets: per written side, trace modes 0 and 1,
    /// validation, shading, reset and gap choice; per (record side, T1a read side), the seed.
    #[allow(clippy::type_complexity)]
    fn record_groups(
        &self,
    ) -> (
        [[Owned<wgpu::BindGroup>; 6]; 2],
        [[Owned<wgpu::BindGroup>; 2]; 2],
    ) {
        use wgpu::BindingResource as R;
        let state = self.primary.as_ref().unwrap();
        let film = state.film.as_ref().unwrap();
        let records = film.records.as_ref().unwrap();
        let t = state.targets.as_ref().unwrap();
        let f = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let hardware = self.hardware_shadows.as_ref().unwrap();
        let frame = &state.frame.as_ref().unwrap().0.value;
        let region = &state.region_buffer.as_ref().unwrap().value;
        let l = &state.record_layouts;
        let group = |layout: &wgpu::BindGroupLayout, entries: &[(u32, R<'_>)]| {
            let entries: Vec<_> = entries
                .iter()
                .map(|(binding, resource)| wgpu::BindGroupEntry {
                    binding: *binding,
                    resource: resource.clone(),
                })
                .collect();
            self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("film records bindings"),
                    layout,
                    entries: &entries,
                }),
                ResourceKind::BindGroup,
            )
        };
        let planes = || (32, records.planes.value.as_entire_binding());
        let sides = [0, 1].map(|written| {
            let read = 1 - written;
            let (w, r) = (&records.sides[written], &records.sides[read]);
            let trace = |mode: usize, moved: &Depth| {
                group(
                    &l.trace_bindings,
                    &[
                        (TRACE_SCENE, hardware.scene().as_binding()),
                        (2, hardware.scene().as_binding()),
                        (10, frame.as_entire_binding()),
                        (13, R::TextureView(&t.hit.view)),
                        (5, R::TextureView(&f.visibility.view)),
                        (20, R::TextureView(&t.tiles.view)),
                        (31, records.motion[mode].value.as_entire_binding()),
                        planes(),
                        (33, R::TextureView(&r.fresh.view)),
                        (34, R::TextureView(&moved.view)),
                    ],
                )
            };
            [
                trace(0, &r.moved),
                trace(1, &w.moved),
                group(
                    &l.validate_bindings,
                    &[
                        (2, hardware.scene().as_binding()),
                        (10, frame.as_entire_binding()),
                        (31, records.motion[0].value.as_entire_binding()),
                        planes(),
                        (33, R::TextureView(&r.fresh.view)),
                        (34, R::TextureView(&r.moved.view)),
                        (35, R::TextureView(&w.moved.view)),
                        (36, records.diagnostics.value.as_entire_binding()),
                    ],
                ),
                group(
                    &l.shade_bindings,
                    &[
                        (0, self.uniform.as_entire_binding()),
                        (7, R::TextureView(&t.hit.view)),
                        (10, frame.as_entire_binding()),
                        (18, R::TextureView(&hdr.view)),
                        (21, R::TextureView(&t.tiles.view)),
                        (22, region.as_entire_binding()),
                        (6, R::TextureView(&f.visibility.view)),
                        (32, records.planes.value.as_entire_binding()),
                        (42, R::TextureView(&w.moved.view)),
                        (37, R::TextureView(&w.fresh.view)),
                    ],
                ),
                group(
                    &l.init_bindings,
                    &[
                        (7, R::TextureView(&t.hit.view)),
                        (21, R::TextureView(&t.tiles.view)),
                        (31, records.motion[0].value.as_entire_binding()),
                        planes(),
                        (37, R::TextureView(&w.fresh.view)),
                        (38, R::TextureView(&hdr.view)),
                        (35, R::TextureView(&w.moved.view)),
                    ],
                ),
                group(
                    &l.gap_bindings,
                    &[
                        (10, frame.as_entire_binding()),
                        (31, records.motion[0].value.as_entire_binding()),
                        planes(),
                        (33, R::TextureView(&r.fresh.view)),
                        (34, R::TextureView(&r.moved.view)),
                    ],
                ),
            ]
        });
        let seeds = [0, 1].map(|latest| {
            [0, 1].map(|t1a| {
                let [colour, anchor, tiles] = film.views(t1a);
                group(
                    &l.seed_bindings,
                    &[
                        (31, records.motion[0].value.as_entire_binding()),
                        planes(),
                        (33, R::TextureView(&records.sides[latest].fresh.view)),
                        (34, R::TextureView(&records.sides[latest].moved.view)),
                        (39, R::TextureView(colour)),
                        (40, R::TextureView(anchor)),
                        (41, R::TextureView(tiles)),
                    ],
                )
            })
        });
        (sides, seeds)
    }

    /// Dispatches `pipeline` over the image in trace tiles with group 0 `group` and the groups
    /// `binds` names.
    fn record_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::ComputePipeline,
        group: &wgpu::BindGroup,
        binds: Binds,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.primary.as_ref().unwrap();
        let size = state.targets.as_ref().unwrap().hit.texture.size();
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        if binds != Binds::Own {
            pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        }
        if binds == Binds::Lit {
            pass.set_bind_group(3, self.lighting.bind(), &[]);
        }
        pass.dispatch_workgroups(
            size.width.div_ceil(TRACE_TILE[0]),
            size.height.div_ceil(TRACE_TILE[1]),
            1,
        );
    }

    /// After a reset frame's Interactive image and `film_init`: the records start from it.
    pub(in crate::renderer) fn encode_records_init(&self, encoder: &mut wgpu::CommandEncoder) {
        let state = self.primary.as_ref().unwrap();
        let records = state.film.as_ref().unwrap().records.as_ref().unwrap();
        let groups = &records.groups.as_ref().unwrap().1[records.side];
        let kernel = &state.record_pipelines.as_ref().unwrap().kernel;
        self.record_pass(
            encoder,
            "film records reset",
            &kernel[2],
            &groups[4],
            Binds::Geometry,
            None,
        );
    }

    /// Before the first static frame after motion: T1a's history (its read side `t1a`)
    /// starts from the latest records.
    pub(in crate::renderer) fn encode_records_seed(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        t1a: usize,
    ) {
        let state = self.primary.as_ref().unwrap();
        let records = state.film.as_ref().unwrap().records.as_ref().unwrap();
        let group = &records.groups.as_ref().unwrap().2[records.side][t1a];
        let kernel = &state.record_pipelines.as_ref().unwrap().kernel;
        self.record_pass(
            encoder,
            "film records seed",
            &kernel[3],
            group,
            Binds::Own,
            None,
        );
    }

    /// One moving frame: the gap choice, the motion trace, validation, the centre retrace of
    /// cold pixels and the program's motion shading. The trace timestamps span the gap choice and
    /// the motion trace; the compute timestamps, validation and the centre retrace.
    pub(in crate::renderer) fn encode_records_motion(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        trace_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        compute_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        shade_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        #[cfg(test)]
        if self.primary.as_ref().unwrap().test_scatter_active {
            return self.test_encode_scatter_motion(
                encoder,
                trace_stamps,
                compute_stamps,
                shade_stamps,
            );
        }
        let state = self.primary.as_ref().unwrap();
        let records = state.film.as_ref().unwrap().records.as_ref().unwrap();
        let groups = &records.groups.as_ref().unwrap().1[records.side];
        let pipelines = state.record_pipelines.as_ref().unwrap();
        let [trace, validate, _, _, gap] = &pipelines.kernel;
        let [begin, end] = split_stamps(trace_stamps);
        let [validated, retraced] = split_stamps(compute_stamps);
        self.record_pass(
            encoder,
            "film records gap choice",
            gap,
            &groups[5],
            Binds::Own,
            begin,
        );
        self.record_pass(
            encoder,
            "film records motion trace",
            trace,
            &groups[0],
            Binds::Lit,
            end,
        );
        self.record_pass(
            encoder,
            "film records validation",
            validate,
            &groups[2],
            Binds::Geometry,
            validated,
        );
        self.record_pass(
            encoder,
            "film records centre retrace",
            trace,
            &groups[1],
            Binds::Lit,
            retraced,
        );
        let production = &pipelines.shading.as_ref().unwrap().1;
        #[cfg(test)]
        let shading = state.test_record_shading.as_deref().unwrap_or(production);
        #[cfg(not(test))]
        let shading = production;
        self.encode_shade_group(encoder, shading, &groups[3], SHADE_TILE, shade_stamps);
    }
}

#[cfg(test)]
impl Renderer {
    /// Recompiles the record kernels with exact-once text edits of their source, for cost
    /// attribution (variants that drop the exact rays or the gap gather). The frame's record
    /// pipelines must already be compiled; a reload or a K change restores the production ones.
    pub(in crate::renderer) fn test_record_kernel_edits(&mut self, edits: &[(String, String)]) {
        let count = self.film_motion.records();
        let mut source = kernel_source(&self.primary_sources.as_ref().unwrap().0[1]).unwrap();
        for (find, replace) in edits {
            source = seam(&source, (find, replace)).unwrap();
        }
        let state = self.primary.as_ref().unwrap();
        let kernel = state.record_kernel(&self.device, &self.ledger, &source, count);
        self.primary
            .as_mut()
            .unwrap()
            .record_pipelines
            .as_mut()
            .unwrap()
            .kernel = kernel;
    }

    /// Compiles the scatter variant of the record gathers (support/film_t2_scatter.wgsl) for the
    /// frame's K, with exact-once text `edits` of its module (cost attribution). Moving frames
    /// use it while `PrimaryState::test_scatter_active` is set. The record pipelines must
    /// already be compiled.
    pub(in crate::renderer) fn test_record_scatter(&mut self, edits: &[(String, String)]) {
        let count = self.film_motion.records();
        let mut source = format!(
            "{}\n{}",
            kernel_source(&self.primary_sources.as_ref().unwrap().0[1]).unwrap(),
            include_str!("../support/film_t2_scatter.wgsl")
        );
        for (find, replace) in edits {
            source = seam(&source, (find, replace)).unwrap();
        }
        let size = self.lighting.hdr.as_ref().unwrap().resolved.texture.size();
        let pixels = u64::from(size.width) * u64::from(size.height);
        let state = self.primary.as_ref().unwrap();
        let device = &self.device;
        let uint = wgpu::TextureSampleType::Uint;
        let arrivals = [read_write(43, 4), read_write(44, 4)];
        let motion = read(31, MOTION_BYTES as u64);
        let planes = read_write(32, 16);
        let sources = [texture(33, uint), array_texture(34)];
        let frame = uniform(10, FRAME_BYTES as u64);
        let entries: [Vec<wgpu::BindGroupLayoutEntry>; 3] = [
            [motion].into_iter().chain(sources).chain(arrivals).collect(),
            [frame, motion, planes]
                .into_iter()
                .chain(sources)
                .chain(arrivals)
                .collect(),
            [scene(2), frame, motion, planes]
                .into_iter()
                .chain(sources)
                .chain([array_image(35), read_write(36, 8)])
                .chain(arrivals)
                .collect(),
        ];
        let layouts = entries.map(|entries| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("film records scatter"),
                entries: &entries,
            })
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("film records scatter"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipelines = [
            ("film_scatter", false),
            ("film_scatter_gap", false),
            ("film_scatter_validate", true),
        ]
        .into_iter()
        .zip(&layouts)
        .map(|((entry, geometry), bindings)| {
            let groups: &[Option<&wgpu::BindGroupLayout>] = if geometry {
                &[Some(bindings), Some(&*state.array_bindings)]
            } else {
                &[Some(bindings)]
            };
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("film records scatter ABI"),
                bind_group_layouts: groups,
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("FRESH_DIRECTIONS", 8.0), ("FILM_RECORDS", f64::from(count))],
                    ..Default::default()
                },
                cache: None,
            })
        })
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        // Arrival counts start zeroed; scatter validation zeroes each pixel's after reading it.
        let capacity = u64::from((2 * count).min(16));
        let prototype = ScatterPrototype {
            counts: buffer("film records scatter counts", 4 * pixels),
            keys: buffer("film records scatter keys", 4 * capacity * pixels),
            layouts,
            pipelines,
        };
        self.primary.as_mut().unwrap().test_scatter = Some(prototype);
    }

    /// The scatter variant of `encode_records_motion`: the scatter and its gap choice in place
    /// of film_gap, and scatter validation in place of film_validate, with the same timestamps.
    fn test_encode_scatter_motion(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        trace_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        compute_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
        shade_stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        use wgpu::BindingResource as R;
        let state = self.primary.as_ref().unwrap();
        let records = state.film.as_ref().unwrap().records.as_ref().unwrap();
        let groups = &records.groups.as_ref().unwrap().1[records.side];
        let pipelines = state.record_pipelines.as_ref().unwrap();
        let scatter = state.test_scatter.as_ref().unwrap();
        let (written, read_side) = (&records.sides[records.side], &records.sides[1 - records.side]);
        let frame = &state.frame.as_ref().unwrap().0.value;
        let scene = self.hardware_shadows.as_ref().unwrap().scene();
        let common = [
            (31, records.motion[0].value.as_entire_binding()),
            (33, R::TextureView(&read_side.fresh.view)),
            (34, R::TextureView(&read_side.moved.view)),
            (43, scatter.counts.as_entire_binding()),
            (44, scatter.keys.as_entire_binding()),
        ];
        let planes = (32, records.planes.value.as_entire_binding());
        let extra: [Vec<(u32, R<'_>)>; 3] = [
            vec![],
            vec![(10, frame.as_entire_binding()), planes.clone()],
            vec![
                (2, scene.as_binding()),
                (10, frame.as_entire_binding()),
                planes,
                (35, R::TextureView(&written.moved.view)),
                (36, records.diagnostics.value.as_entire_binding()),
            ],
        ];
        let binds: Vec<wgpu::BindGroup> = scatter
            .layouts
            .iter()
            .zip(extra)
            .map(|(layout, extra)| {
                let entries: Vec<_> = common
                    .iter()
                    .cloned()
                    .chain(extra)
                    .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
                    .collect();
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("film records scatter"),
                    layout,
                    entries: &entries,
                })
            })
            .collect();
        let trace: &wgpu::ComputePipeline = &pipelines.kernel[0];
        let [begin, end] = split_stamps(trace_stamps);
        let [validated, retraced] = split_stamps(compute_stamps);
        let passes: [(&str, &wgpu::ComputePipeline, &wgpu::BindGroup, Binds, _); 5] = [
            ("film records scatter", &scatter.pipelines[0], &binds[0], Binds::Own, begin),
            ("film records scatter gap choice", &scatter.pipelines[1], &binds[1], Binds::Own, None),
            ("film records motion trace", trace, &groups[0], Binds::Lit, end),
            ("film records scatter validation", &scatter.pipelines[2], &binds[2], Binds::Geometry, validated),
            ("film records centre retrace", trace, &groups[1], Binds::Lit, retraced),
        ];
        for (label, pipeline, group, kind, stamps) in passes {
            self.record_pass(encoder, label, pipeline, group, kind, stamps);
        }
        let production = &pipelines.shading.as_ref().unwrap().1;
        let shading = state.test_record_shading.as_deref().unwrap_or(production);
        self.encode_shade_group(encoder, shading, &groups[3], SHADE_TILE, shade_stamps);
    }
}

/// Test-only scatter variant of the record gathers (support/film_t2_scatter.wgsl): per-pixel
/// arrival counts and gather-order keys (capacity 2K, at most 16), and the scatter, gap-choice and
/// validation pipelines.
#[cfg(test)]
pub(in crate::renderer) struct ScatterPrototype {
    counts: wgpu::Buffer,
    keys: wgpu::Buffer,
    layouts: [wgpu::BindGroupLayout; 3],
    pipelines: [wgpu::ComputePipeline; 3],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_compositions_apply_every_seam_once() {
        let prelude = include_str!("../shaders/scene.wgsl");
        let kernels = shader_sources(prelude).unwrap();
        let heads = shading_heads(prelude).unwrap();
        let kernel = kernel_source(&kernels[1]).unwrap();
        assert!(kernel.contains("film_motion_begin(id.xy,size);"));
        assert!(kernel.contains("film_plane(p,trace,visible);"));
        let head = film_accumulation::shading_head(&heads[1]).unwrap();
        let standard = compose_shading(&head, crate::STANDARD_PROGRAM, false);
        // Every store goes through film_motion_store or film_motion_clear: the module's only
        // HDR writes are theirs.
        assert_eq!(
            standard.source.matches("textureStore(primary_hdr").count(),
            3
        );
        // One clear-colour site, which tests black out for sun-only and visibility images.
        assert_eq!(standard.source.matches("primary_frame.clear").count(), 1);
        assert_eq!(
            &standard.source[standard.program.clone()],
            crate::STANDARD_PROGRAM
        );
        for source in [kernel, standard.source] {
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
        // The default and T1a modules never see a record seam.
        assert!(!kernels[1].contains("film_motion"));
        let t1a = film_accumulation::kernel_source(&kernels[1]).unwrap();
        assert!(!t1a.contains("film_motion"));
    }

    #[test]
    fn record_layout_constants_match_the_shaders() {
        // FilmMotion: three mat4x4, three vec4<f32>-sized rows and the size/flags words.
        assert_eq!(MOTION_BYTES, 256);
        assert!(KERNEL.contains("struct FilmMotion {"));
        // The mode word the CPU patches is size.z.
        assert!(KERNEL.contains("film_motion.size.z!=0u"));
    }
}
