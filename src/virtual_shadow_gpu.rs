//! N8/N8b (docs/non-rt-n8.md, docs/non-rt-n8b.md): `SoftShadows::Virtual`, Interactive's
//! eight sun directions with no ray queries or acceleration structure in the visibility pass.
//!
//! Near field: the receiver triangle and its vertex 1-ring (the exact-position edge
//! adjacency stored after each mesh's indices), tested with the watertight test over the
//! whole ray, unless a per-triangle ring horizon shows no ring triangle can block. Far
//! field: a sun clipmap whose levels hold about one texel per receiver pixel, each with two
//! layers of heights and triangle ids: the highest surface toward the sun and the next one
//! below it (a depth peel). Like a virtual shadow map, work is page-driven: receivers request
//! 128-texel pages at their level and every coarser one the march continues into, only stale
//! requested pages are rasterized (stencil-masked), and rasterized pages stay cached until
//! the sun turns, the level's window moves, or a moved caster's light-space bounds touch
//! them. Memory is dense per level (no physical page pool). The march uses the map as an
//! index only: a candidate texel is decided by an exact walk to the triangle under the ray.
//!
//! This unit consumes traced hit records and receivers; the shadow pass itself only
//! rasterizes and reads textures. A raster visibility buffer producing the same
//! `(placement, primitive)` records is the remaining step for a fully non-RT frame.
use super::*;

/// Levels a frame can use; also the WGSL `VSM_MAX_LEVELS`.
pub(super) const MAX_LEVELS: usize = 8;
const MAX_RECTS: usize = 16;
const PAGE: u32 = 128;
/// WGSL `VsmFrame`: six vec4s, two uvec4s, eight levels, sixteen rectangles and the 64
/// per-batch horizon offsets.
const FRAME_BYTES: usize = 6 * 16 + 2 * 16 + MAX_LEVELS * 32 + MAX_RECTS * 16 + 256;
/// Per-draw `VsmHorizonPass` uniforms at the minimum dynamic-offset alignment.
const HORIZON_STRIDE: u64 = 256;
const TEMPLATE_BYTES: u64 = 32;
const MAX_TEMPLATES: u64 = MAX_LEVELS as u64 * crate::TRACED_DRAWS as u64;
/// Per-level `VsmPass` uniforms at the minimum dynamic-offset alignment.
const PASS_STRIDE: u64 = 256;
/// Device features the page raster needs: triangle ids and placement-based first instances.
pub const VIRTUAL_FEATURES: wgpu::Features =
    wgpu::Features::PRIMITIVE_INDEX.union(wgpu::Features::INDIRECT_FIRST_INSTANCE);
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24PlusStencil8;

/// Tuning of `SoftShadows::Virtual`. Defaults follow docs/non-rt-n8.md.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualShadowSettings {
    /// Metres beyond which other surfaces are found only through the map. The receiver's
    /// own 1-ring is always tested exactly over the whole ray.
    pub near: f32,
    /// log2 of map texels per screen pixel at the receiver; 0 is one texel per pixel.
    pub texels_per_pixel_log2: f32,
    /// Window side in 128-texel pages, a power of two from 4 to 32.
    pub pages: u32,
    /// Map samples per direction beyond `near`, geometrically spaced.
    pub march_samples: u32,
    /// Texel of the finest absolute level, metres; level k has `finest_texel * 2^k`.
    pub finest_texel: f32,
    /// Height-comparison bias in units of 2^-24 of the scene's height range.
    pub bias_ulps: u32,
    /// Walk the receiver's vertex 1-ring for the exact near test; false tests only the
    /// receiver's own triangle (cheaper; nearby own-surface blockers then come from the map).
    pub ring: bool,
    /// Also test the peel layer where it belongs to the same placement as the top layer
    /// (N8b): surfaces hidden under their own mesh, such as scan overhangs. Off, those are
    /// treated as folds the top layer decides: fewer exact walks (the quarry's static shadow
    /// cost falls from about 4.9 to 2.8 ms) for more leaks (docs/non-rt-n8b.md).
    pub peel_own: bool,
}
impl Default for VirtualShadowSettings {
    fn default() -> Self {
        Self {
            near: 0.002,
            texels_per_pixel_log2: 0.0,
            pages: 16,
            march_samples: 10,
            finest_texel: 1.0 / 16384.0,
            bias_ulps: 4,
            ring: true,
            peel_own: false,
        }
    }
}
impl VirtualShadowSettings {
    fn validate(&self) -> Result<(), Error> {
        if !(self.near.is_finite() && self.near > 0.0)
            || !self.texels_per_pixel_log2.is_finite()
            || !self.pages.is_power_of_two()
            || !(4..=32).contains(&self.pages)
            || !(2..=64).contains(&self.march_samples)
            || !(self.finest_texel.is_finite() && self.finest_texel > 0.0)
        {
            return Err(Error::InvalidInput(
                "virtual shadow settings: near > 0, pages a power of two in [4,32], 2..=64 samples, positive texel",
            ));
        }
        Ok(())
    }
}

/// Cache counters for the last prepared frame (tests and reports).
#[derive(Clone, Copy, Debug, Default)]
pub struct VirtualShadowStats {
    pub levels: u32,
    pub finest_level: i32,
    /// Levels whose cached pages were all invalidated this frame (sun, window, range).
    pub invalidated_levels: u32,
    /// Moved-caster rectangles uploaded this frame.
    pub moved_rects: u32,
    pub templates: u32,
    pub bytes: u64,
    /// Whether this frame ran the request, compaction and raster passes.
    pub page_work: bool,
}

struct Pipelines {
    mark: Owned<wgpu::ComputePipeline>,
    compact: Owned<wgpu::ComputePipeline>,
    finalize: Owned<wgpu::ComputePipeline>,
    clear: Owned<wgpu::RenderPipeline>,
    caster: Owned<wgpu::RenderPipeline>,
    caster_peel: Owned<wgpu::RenderPipeline>,
    lookup: Owned<wgpu::ComputePipeline>,
    horizon: Owned<wgpu::ComputePipeline>,
    /// The lookup with `VSM_DEBUG`, built on first use (tests only).
    debug: Option<Owned<wgpu::ComputePipeline>>,
}
struct Layouts {
    mark: Owned<wgpu::BindGroupLayout>,
    compact: Owned<wgpu::BindGroupLayout>,
    raster: Owned<wgpu::BindGroupLayout>,
    pass: Owned<wgpu::BindGroupLayout>,
    lookup: Owned<wgpu::BindGroupLayout>,
    horizon: Owned<wgpu::BindGroupLayout>,
}
/// Ring horizons (N8b lever 1): 4 bytes per triangle of every traced draw, computed once
/// per draw list by `vsm_horizon`. Elevations do not change under the similarity transforms
/// the lookup accepts, so camera, sun and instance motion keep them.
struct Horizons {
    /// Each draw's vertex buffer and triangle count, in batch order.
    key: Vec<(usize, u32)>,
    /// First word of each batch's triangles.
    offsets: Vec<u32>,
    buffer: Owned<wgpu::Buffer>,
    params: Owned<wgpu::Buffer>,
    group: Owned<wgpu::BindGroup>,
    _allocation: Allocation,
    /// Whether this frame computes them (the first frame of a draw list).
    dispatch: bool,
}
/// Map storage sized by (levels, pages) and the per-pixel level target by image size.
struct Resources {
    levels: usize,
    pages: u32,
    size: [u32; 2],
    _depth: Owned<wgpu::Texture>,
    depth_layers: Vec<Owned<wgpu::TextureView>>,
    depth_sampled: Owned<wgpu::TextureView>,
    _peel: Owned<wgpu::Texture>,
    peel_layers: Vec<Owned<wgpu::TextureView>>,
    peel_sampled: Owned<wgpu::TextureView>,
    _peel_ids: Owned<wgpu::Texture>,
    peel_id_layers: Vec<Owned<wgpu::TextureView>>,
    peel_ids_sampled: Owned<wgpu::TextureView>,
    _ids: Owned<wgpu::Texture>,
    id_layers: Vec<Owned<wgpu::TextureView>>,
    ids_sampled: Owned<wgpu::TextureView>,
    _level: Owned<wgpu::Texture>,
    level_view: Owned<wgpu::TextureView>,
    /// Per-pixel decision trace (`VSM_DEBUG`), two uvec4s per pixel; 16 bytes unless
    /// `VirtualState::debug`.
    debug: Owned<wgpu::Buffer>,
    debug_sized: bool,
    frame: Owned<wgpu::Buffer>,
    requests: Owned<wgpu::Buffer>,
    state: Owned<wgpu::Buffer>,
    dirty: Owned<wgpu::Buffer>,
    quad_args: Owned<wgpu::Buffer>,
    caster_args: Owned<wgpu::Buffer>,
    templates: Owned<wgpu::Buffer>,
    pass: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    groups: Option<Groups>,
}
/// Bind groups over this frame's receiver targets; rebuilt when any of them changes.
struct Groups {
    key: [wgpu::Texture; 4],
    models: wgpu::Buffer,
    horizons: wgpu::Buffer,
    mark: Owned<wgpu::BindGroup>,
    compact: Owned<wgpu::BindGroup>,
    /// The top pass samples the peel ids (unused there), the peel pass the top ids: a
    /// pass never samples its own colour target.
    raster: Owned<wgpu::BindGroup>,
    raster_peel: Owned<wgpu::BindGroup>,
    pass: Owned<wgpu::BindGroup>,
    lookup: Owned<wgpu::BindGroup>,
}
/// What the cache remembers between frames to decide which pages are still valid.
#[derive(Default)]
struct Cache {
    light: Option<[u32; 3]>,
    finest: Option<i32>,
    levels: usize,
    /// Window minimum page per level, in absolute light-space page units.
    origins: [Option<[i64; 2]>; MAX_LEVELS],
    epochs: [u32; MAX_LEVELS],
    /// Grow-only caster height range along the light, which the depth encoding uses.
    heights: Option<[f64; 2]>,
    models: Vec<[u32; 16]>,
    bounds: Vec<crate::shadow_plan::Bounds>,
}
/// One indirect caster draw: a draw's instances in one level.
struct Template {
    draw: usize,
    level: usize,
}
pub(super) struct VirtualState {
    pub settings: VirtualShadowSettings,
    pub stats: VirtualShadowStats,
    layouts: Layouts,
    lookup_layout: Owned<wgpu::PipelineLayout>,
    pipelines: Pipelines,
    resources: Option<Resources>,
    retired: Vec<(u64, Resources)>,
    cache: Cache,
    templates: Vec<Template>,
    levels: usize,
    /// Stencil value marking this frame's stale pages, cycling through 1..=255 so no
    /// per-frame stencil clear is needed. A page tagged 255 frames ago is only redrawn
    /// with unchanged geometry, which cannot change its depths.
    tag: u32,
    /// Page requests depend only on the camera, the sun, the casters and the cache
    /// windows. A frame identical in all of them finds nothing stale, so it skips the
    /// request, compaction and raster passes and only runs the lookup.
    key: Vec<u64>,
    page_work: bool,
    horizons: Option<Horizons>,
    /// Tests: run the `VSM_DEBUG` lookup and keep its per-pixel trace in `debug_buffer`.
    pub debug: bool,
    /// Tests: lookup override constants for variant screens (`built`: those compiled).
    pub constants: Vec<(&'static str, f64)>,
    built: Vec<(&'static str, f64)>,
    module: Owned<wgpu::ShaderModule>,
}

fn layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}
fn storage(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        visibility,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
    )
}
fn frame_uniform(visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        0,
        visibility,
        wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(FRAME_BYTES as u64),
        },
    )
}
fn sampled(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
    view_dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::ShaderStages::COMPUTE,
        wgpu::BindingType::Texture {
            sample_type,
            view_dimension,
            multisampled: false,
        },
    )
}
fn written(binding: u32) -> wgpu::BindGroupLayoutEntry {
    layout_entry(
        binding,
        wgpu::ShaderStages::COMPUTE,
        wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R32Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
        },
    )
}

/// The module source: the shared draw-slot count, then shaders/virtual_shadow.wgsl.
pub(super) fn shader_source() -> String {
    format!(
        "enable wgpu_binding_array;\nenable primitive_index;\nconst PRIMARY_BATCHES:u32={}u;\n{}",
        crate::TRACED_DRAWS,
        include_str!("../shaders/virtual_shadow.wgsl")
    )
}

fn corners(b: &crate::shadow_plan::Bounds) -> impl Iterator<Item = glam::DVec3> + '_ {
    (0..8).map(move |i| {
        glam::DVec3::new(
            f64::from(if i & 1 == 0 { b.min.x } else { b.max.x }),
            f64::from(if i & 2 == 0 { b.min.y } else { b.max.y }),
            f64::from(if i & 4 == 0 { b.min.z } else { b.max.z }),
        )
    })
}
fn buffer(binding: u32, b: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: b.as_entire_binding(),
    }
}
fn texture(binding: u32, v: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(v),
    }
}

/// Orthonormal sun basis (u, v, toward-light l), from the light direction alone so cached
/// pages survive camera motion.
fn basis(toward: glam::DVec3) -> [glam::DVec3; 3] {
    let l = toward.normalize();
    let up = if l.y.abs() < 0.999 {
        glam::DVec3::Y
    } else {
        glam::DVec3::X
    };
    let u = up.cross(l).normalize();
    let v = l.cross(u);
    [u, v, l]
}

impl VirtualState {
    pub fn new(
        device: &wgpu::Device,
        ledger: &Ledger,
        arrays: &wgpu::BindGroupLayout,
        lighting: &wgpu::BindGroupLayout,
    ) -> Self {
        use wgpu::ShaderStages as S;
        use wgpu::TextureSampleType as T;
        use wgpu::TextureViewDimension as D;
        let group = |label, entries: &[wgpu::BindGroupLayoutEntry]| {
            ledger.own(
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(label),
                    entries,
                }),
                ResourceKind::BindGroupLayout,
            )
        };
        let uint = T::Uint;
        let float = T::Float { filterable: false };
        let layouts = Layouts {
            mark: group(
                "N8 page requests",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(1, false, S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    written(14),
                ],
            ),
            compact: group(
                "N8 page cache",
                &[
                    frame_uniform(S::COMPUTE),
                    storage(1, false, S::COMPUTE),
                    storage(2, false, S::COMPUTE),
                    storage(3, false, S::COMPUTE),
                    storage(4, false, S::COMPUTE),
                    storage(5, false, S::COMPUTE),
                    storage(6, true, S::COMPUTE),
                ],
            ),
            raster: group(
                "N8 page raster",
                &[
                    frame_uniform(S::VERTEX_FRAGMENT),
                    storage(11, true, S::VERTEX),
                    storage(12, true, S::VERTEX),
                    layout_entry(
                        13,
                        S::FRAGMENT,
                        wgpu::BindingType::Texture {
                            sample_type: uint,
                            view_dimension: D::D2Array,
                            multisampled: false,
                        },
                    ),
                ],
            ),
            pass: group(
                "N8 page raster level",
                &[layout_entry(
                    30,
                    S::VERTEX_FRAGMENT,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                )],
            ),
            lookup: group(
                "N8 visibility",
                &[
                    frame_uniform(S::COMPUTE),
                    sampled(7, uint, D::D2),
                    sampled(8, uint, D::D2),
                    sampled(9, float, D::D2),
                    sampled(10, float, D::D2),
                    written(15),
                    sampled(16, T::Depth, D::D2Array),
                    sampled(17, uint, D::D2Array),
                    sampled(18, uint, D::D2),
                    sampled(19, T::Depth, D::D2Array),
                    storage(20, false, S::COMPUTE),
                    sampled(21, uint, D::D2Array),
                    storage(24, true, S::COMPUTE),
                ],
            ),
            horizon: group(
                "N8 ring horizon pass",
                &[
                    layout_entry(
                        22,
                        S::COMPUTE,
                        wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: wgpu::BufferSize::new(16),
                        },
                    ),
                    storage(23, false, S::COMPUTE),
                ],
            ),
        };
        let pipeline_layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
            ledger.own(
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: groups,
                    immediate_size: 0,
                }),
                ResourceKind::PipelineLayout,
            )
        };
        let mark_layout = pipeline_layout("N8 page request ABI", &[Some(&layouts.mark)]);
        let compact_layout = pipeline_layout("N8 page cache ABI", &[Some(&layouts.compact)]);
        let raster_layout = pipeline_layout(
            "N8 page raster ABI",
            &[Some(&layouts.raster), Some(&layouts.pass)],
        );
        let horizon_layout = pipeline_layout(
            "N8 ring horizon ABI",
            &[Some(&layouts.horizon), Some(arrays)],
        );
        let lookup_layout = pipeline_layout(
            "N8 visibility ABI",
            &[Some(&layouts.lookup), Some(arrays), None, Some(lighting)],
        );
        let module = ledger.own(
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("N8 virtual sun shadows"),
                source: wgpu::ShaderSource::Wgsl(shader_source().into()),
            }),
            ResourceKind::ShaderModule,
        );
        let compute = |entry, layout: &wgpu::PipelineLayout| {
            ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(entry),
                    layout: Some(layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                }),
                ResourceKind::ComputePipeline,
            )
        };
        let ids = [Some(wgpu::TextureFormat::R32Uint.into())];
        let raster = |label,
                      vs,
                      fs: Option<&str>,
                      targets: &[Option<wgpu::ColorTargetState>],
                      buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                      depth| {
            ledger.own(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(label),
                    layout: Some(&raster_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(vs),
                        compilation_options: Default::default(),
                        buffers,
                    },
                    fragment: fs.map(|fs| wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(fs),
                        compilation_options: Default::default(),
                        targets,
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: Some(depth),
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                }),
                ResourceKind::RenderPipeline,
            )
        };
        let face = |compare, pass_op| wgpu::StencilFaceState {
            compare,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op,
        };
        let stencil = |compare, pass_op, write_mask| wgpu::StencilState {
            front: face(compare, pass_op),
            back: face(compare, pass_op),
            read_mask: 0xff,
            write_mask,
        };
        // Stale pages: clear to the lowest height and open the stencil with the frame tag.
        let clear_state = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: stencil(
                wgpu::CompareFunction::Always,
                wgpu::StencilOperation::Replace,
                0xff,
            ),
            bias: Default::default(),
        };
        let clear = raster(
            "N8 stale page clear",
            "vs_vsm_clear",
            Some("fs_vsm_clear"),
            &ids,
            &[],
            clear_state,
        );
        // Casters keep the highest surface toward the light, only inside opened pages.
        let attributes = [wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }];
        let vertices = [Some(wgpu::VertexBufferLayout {
            array_stride: 48,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        let caster_state = wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Greater),
            stencil: stencil(
                wgpu::CompareFunction::Equal,
                wgpu::StencilOperation::Keep,
                0,
            ),
            bias: Default::default(),
        };
        let caster = raster(
            "N8 page casters",
            "vs_vsm_caster",
            Some("fs_vsm_caster"),
            &ids,
            &vertices,
            caster_state.clone(),
        );
        // The peel layer: the highest surface other than the top triangle.
        let caster_peel = raster(
            "N8 page peel",
            "vs_vsm_caster",
            Some("fs_vsm_peel"),
            &ids,
            &vertices,
            caster_state,
        );
        let pipelines = Pipelines {
            mark: compute("vsm_mark", &mark_layout),
            compact: compute("vsm_compact", &compact_layout),
            finalize: compute("vsm_finalize", &compact_layout),
            clear,
            caster,
            caster_peel,
            lookup: compute("vsm_lookup", &lookup_layout),
            horizon: compute("vsm_horizon", &horizon_layout),
            debug: None,
        };
        Self {
            settings: VirtualShadowSettings::default(),
            stats: VirtualShadowStats::default(),
            layouts,
            lookup_layout,
            pipelines,
            resources: None,
            retired: vec![],
            cache: Cache::default(),
            templates: vec![],
            levels: 0,
            tag: 0,
            key: vec![],
            page_work: true,
            horizons: None,
            debug: false,
            constants: vec![],
            built: vec![],
            module,
        }
    }

    /// The last debug frame's trace buffer (`debug` must have been set for it).
    pub fn debug_buffer(&self) -> Option<&wgpu::Buffer> {
        self.resources
            .as_ref()
            .filter(|r| r.debug_sized)
            .map(|r| &*r.debug)
    }

    pub fn set_settings(
        &mut self,
        settings: VirtualShadowSettings,
        last: u64,
    ) -> Result<(), Error> {
        settings.validate()?;
        if settings != self.settings {
            self.settings = settings;
            // Every cached page depends on the level ladder and window size.
            self.cache = Cache::default();
            if let Some(old) = self.resources.take() {
                self.retired.push((last, old));
            }
        }
        Ok(())
    }

    pub fn collect(&mut self, completed: u64) {
        self.retired.retain(|(last, _)| *last > completed);
    }

    fn ensure_resources(
        &mut self,
        device: &wgpu::Device,
        ledger: &Ledger,
        levels: usize,
        size: [u32; 2],
        last: u64,
    ) -> bool {
        let pages = self.settings.pages;
        if self.resources.as_ref().is_some_and(|r| {
            r.levels >= levels && r.pages == pages && r.size == size && r.debug_sized == self.debug
        }) {
            return false;
        }
        // Grow-only in levels, so the ladder can shift without reallocating.
        let levels = levels.max(self.resources.as_ref().map_or(0, |r| r.levels));
        if let Some(old) = self.resources.take() {
            self.retired.push((last, old));
        }
        // Fresh storage is all stale without touching the cache: its page epochs are zero,
        // and level epochs start at one.
        let side = pages * PAGE;
        let layers = |label, format: wgpu::TextureFormat, aspect| {
            let texture = ledger.own(
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: levels as u32,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }),
                ResourceKind::Texture,
            );
            let per_layer = (0..levels as u32)
                .map(|layer| {
                    ledger.own(
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2),
                            base_array_layer: layer,
                            array_layer_count: Some(1),
                            ..Default::default()
                        }),
                        ResourceKind::TextureView,
                    )
                })
                .collect();
            let all = ledger.own(
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    aspect,
                    ..Default::default()
                }),
                ResourceKind::TextureView,
            );
            (texture, per_layer, all)
        };
        let (depth, depth_layers, depth_sampled) = layers(
            "N8 clipmap heights and page stencil",
            DEPTH_FORMAT,
            wgpu::TextureAspect::DepthOnly,
        );
        let (peel, peel_layers, peel_sampled) = layers(
            "N8 clipmap peel heights and page stencil",
            DEPTH_FORMAT,
            wgpu::TextureAspect::DepthOnly,
        );
        let (peel_ids, peel_id_layers, peel_ids_sampled) = layers(
            "N8 clipmap peel triangles",
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureAspect::All,
        );
        let (ids, id_layers, ids_sampled) = layers(
            "N8 clipmap top triangles",
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureAspect::All,
        );
        let level = ledger.own(
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("N8 receiver levels"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let level_view = ledger.own(
            level.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let slots = levels as u64 * u64::from(pages * pages);
        let debug_bytes = if self.debug {
            u64::from(size[0]) * u64::from(size[1]) * 32
        } else {
            16
        };
        let buffer = |label, size: u64, usage| {
            ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            )
        };
        use wgpu::BufferUsages as B;
        let bytes = u64::from(side) * u64::from(side) * levels as u64 * 16
            + u64::from(size[0]) * u64::from(size[1]) * 4
            + slots * 12
            + MAX_TEMPLATES * (TEMPLATE_BYTES + 20);
        self.resources = Some(Resources {
            levels,
            pages,
            size,
            _depth: depth,
            depth_layers,
            depth_sampled,
            _peel: peel,
            peel_layers,
            peel_sampled,
            _peel_ids: peel_ids,
            peel_id_layers,
            peel_ids_sampled,
            _ids: ids,
            id_layers,
            ids_sampled,
            _level: level,
            level_view,
            debug: buffer("N8 decision trace", debug_bytes, B::STORAGE | B::COPY_SRC),
            debug_sized: self.debug,
            frame: buffer("N8 frame", FRAME_BYTES as u64, B::UNIFORM),
            requests: buffer("N8 page requests", slots * 4, B::STORAGE),
            state: buffer("N8 page epochs", slots * 4, B::STORAGE),
            dirty: buffer(
                "N8 stale pages",
                MAX_LEVELS as u64 * 4 + slots * 4,
                B::STORAGE,
            ),
            quad_args: buffer(
                "N8 stale page draws",
                MAX_LEVELS as u64 * 16,
                B::STORAGE | B::INDIRECT,
            ),
            caster_args: buffer(
                "N8 caster draws",
                MAX_TEMPLATES * 20,
                B::STORAGE | B::INDIRECT,
            ),
            templates: buffer(
                "N8 caster templates",
                MAX_TEMPLATES * TEMPLATE_BYTES,
                B::STORAGE,
            ),
            pass: buffer(
                "N8 level uniforms",
                MAX_LEVELS as u64 * PASS_STRIDE,
                B::UNIFORM,
            ),
            _allocation: ledger.allocate(AllocationKind::Shadow, bytes),
            groups: None,
        });
        self.stats.bytes = bytes;
        true
    }
}

impl Renderer {
    /// Tuning for `SoftShadows::Virtual`; a change drops every cached page.
    pub fn set_virtual_shadow_settings(
        &mut self,
        settings: VirtualShadowSettings,
    ) -> Result<(), Error> {
        settings.validate()?;
        match &mut self.virtual_shadows {
            Some(state) => state.set_settings(settings, self.submissions),
            None => {
                self.virtual_settings = settings;
                Ok(())
            }
        }
    }
    pub fn virtual_shadow_stats(&self) -> VirtualShadowStats {
        self.virtual_shadows
            .as_ref()
            .map_or_else(Default::default, |s| s.stats)
    }

    /// Plans this frame's levels, windows and cache invalidation, uploads them, and builds
    /// bind groups over the traced receiver targets. Call after `prepare_primary`.
    pub(super) fn prepare_virtual(
        &mut self,
        draws: &[PreparedDraw<'_>],
        camera: &Camera,
        size: wgpu::Extent3d,
    ) -> Result<(), Error> {
        if !self.device.features().contains(VIRTUAL_FEATURES) {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual requires VIRTUAL_FEATURES (primitive index, indirect first instance)",
            ));
        }
        let Some(Light::Directional { travel, .. }) = self.lighting.source_lights.first().copied()
        else {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual shadows lights[0], which must be directional",
            ));
        };
        let primary = self.primary.as_ref().unwrap();
        if self.virtual_shadows.is_none() {
            let mut state = VirtualState::new(
                &self.device,
                &self.ledger,
                primary.array_layout(),
                &self.lighting.bindings,
            );
            state.settings = self.virtual_settings;
            self.pipelines.created += 6;
            self.virtual_shadows = Some(state);
        }
        let state = self.virtual_shadows.as_mut().unwrap();
        let s = state.settings;
        let slope = f64::from(self.shadows.settings.directional_disk_angle).tan() * 1.001;
        let eye = camera.view().inverse().w_axis.truncate().as_dvec3();
        let [u, v, l] = basis(-travel.as_dvec3());
        let light_bits = (-travel).normalize().to_array().map(f32::to_bits);

        // Placements in traced order: draw order, then instance slot.
        let mut models = vec![];
        let mut bounds = vec![];
        for draw in draws {
            for &model in &draw.instances.state.models {
                models.push(model.to_cols_array().map(f32::to_bits));
                bounds.push(shadow_gpu::world_bounds(draw.mesh.bounds, model)?);
            }
        }
        let placements = models.len();
        // Light-space xy rectangle of a box, relative to the eye.
        let rect = |b: &crate::shadow_plan::Bounds| {
            let mut lo = glam::DVec2::INFINITY;
            let mut hi = glam::DVec2::NEG_INFINITY;
            for c in corners(b) {
                let p = glam::DVec2::new((c - eye).dot(u), (c - eye).dot(v));
                lo = lo.min(p);
                hi = hi.max(p);
            }
            [lo, hi]
        };
        let (mut zlo, mut zhi) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut near, mut far) = (f64::INFINITY, 0.0f64);
        for b in &bounds {
            for c in corners(b) {
                let z = c.dot(l);
                zlo = zlo.min(z);
                zhi = zhi.max(z);
                far = far.max((c - eye).length());
            }
            let nearest = eye.clamp(b.min.as_dvec3(), b.max.as_dvec3());
            near = near.min((nearest - eye).length());
        }
        if bounds.is_empty() {
            (zlo, zhi, near, far) = (0.0, 1.0, 1.0, 1.0);
        }
        let mut invalidated = [false; MAX_LEVELS];
        let cache = &mut state.cache;
        if cache.light != Some(light_bits) {
            cache.light = Some(light_bits);
            invalidated = [true; MAX_LEVELS];
        }
        // The depth encoding spans a padded, grow-only height range.
        let heights = match cache.heights {
            Some([a, b]) if a <= zlo && b >= zhi && !invalidated[0] => [a, b],
            _ => {
                let pad = 0.01 * (zhi - zlo) + 0.001;
                invalidated = [true; MAX_LEVELS];
                [zlo - pad, zhi + pad]
            }
        };
        cache.heights = Some(heights);
        let footprint = 2.0 * (f64::from(camera.fov_y()) * 0.5).tan() / f64::from(size.height);
        let level_of = |d: f64| {
            ((d.max(1e-6) * footprint / f64::from(s.finest_texel)).log2()
                - f64::from(s.texels_per_pixel_log2))
            .floor() as i32
        };
        let finest = level_of(near.max(f64::from(camera.near())));
        let levels = ((level_of(far) - finest + 2).clamp(1, MAX_LEVELS as i32)) as usize;
        if cache.finest != Some(finest) || cache.levels != levels {
            cache.finest = Some(finest);
            cache.levels = levels;
            invalidated = [true; MAX_LEVELS];
        }
        // Moved casters invalidate the pages under their old and new light-space bounds.
        let mut rects = vec![];
        if cache.models.len() != placements {
            invalidated = [true; MAX_LEVELS];
        } else {
            for (p, model) in models.iter().enumerate() {
                if *model != cache.models[p] {
                    let [a0, a1] = rect(&cache.bounds[p]);
                    let [b0, b1] = rect(&bounds[p]);
                    rects.push([a0.min(b0), a1.max(b1)]);
                }
            }
            if rects.len() > MAX_RECTS {
                invalidated = [true; MAX_LEVELS];
                rects.clear();
            }
        }
        cache.models = models;
        cache.bounds = bounds;
        let pages = i64::from(s.pages);
        let eye_light = glam::DVec2::new(eye.dot(u), eye.dot(v));
        let mut level_bytes = [0u8; MAX_LEVELS * 32];
        let mut windows = [[glam::DVec2::ZERO; 2]; MAX_LEVELS];
        for i in 0..levels {
            let texel = f64::from(s.finest_texel) * 2f64.powi(finest + i as i32);
            let page = texel * f64::from(PAGE);
            let centre = [eye_light.x, eye_light.y].map(|x| (x / page).floor() as i64);
            let keep = cache.origins[i].filter(|o| {
                !invalidated[i] && (0..2).all(|a| (centre[a] - (o[a] + pages / 2)).abs() <= 1)
            });
            let origin = keep.unwrap_or([centre[0] - pages / 2, centre[1] - pages / 2]);
            if keep.is_none() {
                invalidated[i] = true;
                cache.origins[i] = Some(origin);
                cache.epochs[i] = cache.epochs[i].wrapping_add(1).max(1);
            }
            let lo = glam::DVec2::new(origin[0] as f64 * page, origin[1] as f64 * page) - eye_light;
            windows[i] = [lo, lo + glam::DVec2::splat(page * pages as f64)];
            let entry = &mut level_bytes[i * 32..i * 32 + 32];
            for (k, value) in [lo.x as f32, lo.y as f32, texel as f32, 0.0]
                .iter()
                .enumerate()
            {
                entry[k * 4..k * 4 + 4].copy_from_slice(&value.to_ne_bytes());
            }
            for (k, value) in [cache.epochs[i], (finest + i as i32) as u32, 0, 0]
                .iter()
                .enumerate()
            {
                entry[16 + k * 4..20 + k * 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
        // Absolute level of index 0, which vsm_select reads from levels[0].info.y.
        level_bytes[20..24].copy_from_slice(&finest.to_ne_bytes());
        // Caster draws per level: every draw whose light-space bounds touch the window.
        state.templates.clear();
        let mut template_bytes = vec![];
        let mut placement = 0u32;
        let mut primitive_bits = 1u32;
        for (d, draw) in draws.iter().enumerate() {
            let n = draw.instances.state.models.len() as u32;
            let triangles = draw.mesh.index_count / 3;
            primitive_bits = primitive_bits.max(32 - triangles.max(1).leading_zeros());
            let mut lo = glam::DVec2::INFINITY;
            let mut hi = glam::DVec2::NEG_INFINITY;
            for b in &state.cache.bounds[placement as usize..(placement + n) as usize] {
                let [a, z] = rect(b);
                lo = lo.min(a);
                hi = hi.max(z);
            }
            for (i, window) in windows.iter().enumerate().take(levels) {
                if n == 0
                    || hi.x < window[0].x
                    || hi.y < window[0].y
                    || lo.x > window[1].x
                    || lo.y > window[1].y
                {
                    continue;
                }
                state.templates.push(Template { draw: d, level: i });
                for value in [draw.mesh.index_count, n, 0, 0, placement, i as u32, 0, 0] {
                    template_bytes.extend_from_slice(&value.to_ne_bytes());
                }
            }
            placement += n;
        }
        if 32 - (placements as u32 + 1).leading_zeros() + primitive_bits > 32 {
            return Err(Error::InvalidInput(
                "SoftShadows::Virtual packs placement and primitive into 32 bits; the scene exceeds that",
            ));
        }
        let mut frame = Vec::with_capacity(FRAME_BYTES);
        let mut f32s = |values: [f64; 4]| {
            for value in values {
                frame.extend_from_slice(&(value as f32).to_ne_bytes());
            }
        };
        f32s([u.x, u.y, u.z, 0.0]);
        f32s([v.x, v.y, v.z, 0.0]);
        f32s([l.x, l.y, l.z, 0.0]);
        f32s([eye.x, eye.y, eye.z, footprint]);
        let range = heights[1] - heights[0];
        f32s([heights[0], 1.0 / range, range, heights[1]]);
        f32s([
            f64::from(s.near),
            f64::from(s.finest_texel),
            slope,
            f64::from(s.texels_per_pixel_log2),
        ]);
        for value in [
            levels as u32,
            s.pages,
            state.templates.len() as u32,
            rects.len() as u32,
            primitive_bits,
            s.march_samples,
            s.bias_ulps,
            u32::from(s.ring) | u32::from(s.peel_own) << 1,
        ] {
            frame.extend_from_slice(&value.to_ne_bytes());
        }
        frame.extend_from_slice(&level_bytes);
        for i in 0..MAX_RECTS {
            let [lo, hi] = rects.get(i).copied().unwrap_or([glam::DVec2::ZERO; 2]);
            for value in [lo.x, lo.y, hi.x, hi.y] {
                frame.extend_from_slice(&(value as f32).to_ne_bytes());
            }
        }
        // Ring horizons: (re)built when the draw list's geometry changes.
        let horizon_key: Vec<(usize, u32)> = draws
            .iter()
            .map(|d| (Arc::as_ptr(&d.mesh.vertex) as usize, d.mesh.index_count / 3))
            .collect();
        if state.horizons.as_ref().is_none_or(|h| h.key != horizon_key) {
            let mut offsets = vec![];
            let mut words = 0u32;
            for &(_, triangles) in &horizon_key {
                offsets.push(words);
                words += triangles;
            }
            let store = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8 ring horizons"),
                    size: u64::from(words.max(1)) * 4,
                    usage: wgpu::BufferUsages::STORAGE,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let params = self.ledger.own(
                self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("N8 ring horizon passes"),
                    size: crate::TRACED_DRAWS as u64 * HORIZON_STRIDE,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            );
            let mut bytes = vec![0u8; crate::TRACED_DRAWS as usize * HORIZON_STRIDE as usize];
            for (d, (&(_, triangles), &offset)) in horizon_key.iter().zip(&offsets).enumerate() {
                let at = d * HORIZON_STRIDE as usize;
                for (k, value) in [d as u32, triangles, offset, 0].iter().enumerate() {
                    bytes[at + k * 4..at + k * 4 + 4].copy_from_slice(&value.to_ne_bytes());
                }
            }
            self.queue.write_buffer(&params, 0, &bytes);
            let group = self.ledger.own(
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("N8 ring horizon pass"),
                    layout: &state.layouts.horizon,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 22,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: &params,
                                offset: 0,
                                size: wgpu::BufferSize::new(16),
                            }),
                        },
                        buffer(23, &store),
                    ],
                }),
                ResourceKind::BindGroup,
            );
            state.horizons = Some(Horizons {
                key: horizon_key,
                offsets,
                _allocation: self
                    .ledger
                    .allocate(AllocationKind::Shadow, u64::from(words) * 4),
                buffer: store,
                params,
                group,
                dispatch: true,
            });
        } else if let Some(h) = &mut state.horizons {
            h.dispatch = false;
        }
        let offsets = &state.horizons.as_ref().unwrap().offsets;
        for b in 0..64 {
            let offset = offsets.get(b).copied().unwrap_or(0);
            frame.extend_from_slice(&offset.to_ne_bytes());
        }
        debug_assert_eq!(frame.len(), FRAME_BYTES);
        state.levels = levels;
        state.tag = state.tag % 255 + 1;
        state.stats = VirtualShadowStats {
            levels: levels as u32,
            finest_level: finest,
            invalidated_levels: invalidated[..levels].iter().filter(|&&b| b).count() as u32,
            moved_rects: rects.len() as u32,
            templates: state.templates.len() as u32,
            bytes: state.stats.bytes,
            page_work: true,
        };
        let created = state.ensure_resources(
            &self.device,
            &self.ledger,
            levels,
            [size.width, size.height],
            self.submissions,
        );
        if state.debug && state.pipelines.debug.is_none() || state.constants != state.built {
            let layout = &state.lookup_layout;
            let lookup = |label, debug: bool| {
                let mut constants = state.constants.clone();
                constants.push(("VSM_DEBUG", f64::from(u8::from(debug))));
                self.ledger.own(
                    self.device
                        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(label),
                            layout: Some(layout),
                            module: &state.module,
                            entry_point: Some("vsm_lookup"),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants: &constants,
                                ..Default::default()
                            },
                            cache: None,
                        }),
                    ResourceKind::ComputePipeline,
                )
            };
            let debug = state.debug.then(|| lookup("vsm_lookup debug", true));
            if state.constants != state.built {
                state.pipelines.lookup = lookup("vsm_lookup", false);
                state.built = state.constants.clone();
            }
            state.pipelines.debug = debug;
        }
        let mut key: Vec<u64> = camera
            .view()
            .to_cols_array()
            .iter()
            .chain(&[camera.fov_y()])
            .map(|v| u64::from(v.to_bits()))
            .collect();
        key.extend([
            u64::from(size.width),
            u64::from(size.height),
            finest as u64,
            levels as u64,
        ]);
        key.extend(light_bits.map(u64::from));
        for i in 0..levels {
            let o = state.cache.origins[i].unwrap_or_default();
            key.extend([o[0] as u64, o[1] as u64, u64::from(state.cache.epochs[i])]);
        }
        for t in &state.templates {
            let draw = &draws[t.draw];
            key.extend([
                t.draw as u64,
                t.level as u64,
                Arc::as_ptr(&draw.mesh.vertex) as u64,
                draw.instances.state.models.len() as u64,
            ]);
        }
        state.page_work = created || !rects.is_empty() || key != state.key;
        state.stats.page_work = state.page_work;
        state.key = key;
        let state = self.virtual_shadows.as_mut().unwrap();
        let r = state.resources.as_mut().unwrap();
        self.queue.write_buffer(&r.frame, 0, &frame);
        if !template_bytes.is_empty() {
            self.queue.write_buffer(&r.templates, 0, &template_bytes);
        }
        let mut pass = vec![0u8; MAX_LEVELS * PASS_STRIDE as usize];
        for i in 0..MAX_LEVELS {
            pass[i * PASS_STRIDE as usize..i * PASS_STRIDE as usize + 4]
                .copy_from_slice(&(i as u32).to_ne_bytes());
        }
        self.queue.write_buffer(&r.pass, 0, &pass);
        self.uploads.shadows += (FRAME_BYTES + template_bytes.len() + pass.len()) as u64;

        let primary = self.primary.as_ref().unwrap();
        let targets = primary.targets.as_ref().unwrap();
        let fresh = self.fresh.as_ref().unwrap().targets.as_ref().unwrap();
        let (arrays_models, _) = primary.virtual_geometry().unwrap();
        let key = [
            (*targets.hit.texture).clone(),
            (*targets.tiles.texture).clone(),
            (*fresh.world.texture).clone(),
            (*fresh.visibility.texture).clone(),
        ];
        let horizons = (*state.horizons.as_ref().unwrap().buffer).clone();
        if r.groups
            .as_ref()
            .is_none_or(|g| g.key != key || &g.models != arrays_models || g.horizons != horizons)
        {
            let view = |t: &wgpu::Texture| t.create_view(&Default::default());
            let hit = view(&targets.hit.texture);
            let tiles = view(&targets.tiles.texture);
            let group =
                |label, layout: &wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry<'_>]| {
                    self.ledger.own(
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some(label),
                            layout,
                            entries,
                        }),
                        ResourceKind::BindGroup,
                    )
                };
            let l = &state.layouts;
            let groups = Groups {
                key,
                models: arrays_models.clone(),
                horizons: horizons.clone(),
                mark: group(
                    "N8 page requests",
                    &l.mark,
                    &[
                        buffer(0, &r.frame),
                        buffer(1, &r.requests),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(14, &r.level_view),
                    ],
                ),
                compact: group(
                    "N8 page cache",
                    &l.compact,
                    &[
                        buffer(0, &r.frame),
                        buffer(1, &r.requests),
                        buffer(2, &r.state),
                        buffer(3, &r.dirty),
                        buffer(4, &r.quad_args),
                        buffer(5, &r.caster_args),
                        buffer(6, &r.templates),
                    ],
                ),
                raster: group(
                    "N8 page raster",
                    &l.raster,
                    &[
                        buffer(0, &r.frame),
                        buffer(11, &r.dirty),
                        buffer(12, arrays_models),
                        texture(13, &r.peel_ids_sampled),
                    ],
                ),
                raster_peel: group(
                    "N8 page peel raster",
                    &l.raster,
                    &[
                        buffer(0, &r.frame),
                        buffer(11, &r.dirty),
                        buffer(12, arrays_models),
                        texture(13, &r.ids_sampled),
                    ],
                ),
                pass: group(
                    "N8 page raster level",
                    &l.pass,
                    &[wgpu::BindGroupEntry {
                        binding: 30,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &r.pass,
                            offset: 0,
                            size: wgpu::BufferSize::new(16),
                        }),
                    }],
                ),
                lookup: group(
                    "N8 visibility",
                    &l.lookup,
                    &[
                        buffer(0, &r.frame),
                        texture(7, &hit),
                        texture(8, &tiles),
                        texture(9, &fresh.world.view),
                        texture(10, &fresh.normal.view),
                        texture(15, &fresh.visibility.view),
                        texture(16, &r.depth_sampled),
                        texture(17, &r.ids_sampled),
                        texture(18, &r.level_view),
                        texture(19, &r.peel_sampled),
                        buffer(20, &r.debug),
                        texture(21, &r.peel_ids_sampled),
                        buffer(24, &horizons),
                    ],
                ),
            };
            r.groups = Some(groups);
        }
        Ok(())
    }

    /// Page requests, cache compaction, stale-page raster and the 8-direction lookup into
    /// the fresh visibility target. `timestamps` brackets all of it.
    pub(super) fn encode_virtual(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[PreparedDraw<'_>],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.virtual_shadows.as_ref().unwrap();
        let r = state.resources.as_ref().unwrap();
        let g = r.groups.as_ref().unwrap();
        let p = &state.pipelines;
        let slots = state.levels as u64 * u64::from(r.pages * r.pages);
        let [width, height] = r.size;
        if state.debug {
            encoder.clear_buffer(&r.debug, 0, None);
        }
        if state.page_work {
            encoder.clear_buffer(&r.requests, 0, Some(slots * 4));
            encoder.clear_buffer(&r.dirty, 0, Some(MAX_LEVELS as u64 * 4));
            self.encode_virtual_pages(encoder, draws, timestamps.as_ref());
        }
        let primary = self.primary.as_ref().unwrap();
        let (_, arrays) = primary.virtual_geometry().unwrap();
        if let Some(h) = state.horizons.as_ref().filter(|h| h.dispatch) {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8 ring horizons"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&p.horizon);
            pass.set_bind_group(1, arrays, &[]);
            for (d, &(_, triangles)) in h.key.iter().enumerate() {
                pass.set_bind_group(0, &*h.group, &[d as u32 * HORIZON_STRIDE as u32]);
                pass.dispatch_workgroups(triangles.div_ceil(64), 1, 1);
            }
            let _ = &h.params;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("N8 ring and map visibility"),
            timestamp_writes: timestamps.map(|t| wgpu::ComputePassTimestampWrites {
                query_set: t.query_set,
                beginning_of_pass_write_index: if state.page_work {
                    None
                } else {
                    t.beginning_of_pass_write_index
                },
                end_of_pass_write_index: t.end_of_pass_write_index,
            }),
        });
        match (&p.debug, state.debug) {
            (Some(debug), true) => pass.set_pipeline(debug),
            _ => pass.set_pipeline(&p.lookup),
        }
        pass.set_bind_group(0, &*g.lookup, &[]);
        pass.set_bind_group(1, arrays, &[]);
        pass.set_bind_group(3, self.lighting.bind(), &[]);
        pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(4), 1);
        let _ = &state.lookup_layout;
    }

    /// Requests, compaction and the stale-page raster of every level.
    fn encode_virtual_pages(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[PreparedDraw<'_>],
        timestamps: Option<&wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let state = self.virtual_shadows.as_ref().unwrap();
        let r = state.resources.as_ref().unwrap();
        let g = r.groups.as_ref().unwrap();
        let p = &state.pipelines;
        let slots = state.levels as u64 * u64::from(r.pages * r.pages);
        let [width, height] = r.size;
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("N8 page requests and cache"),
                timestamp_writes: timestamps.map(|t| wgpu::ComputePassTimestampWrites {
                    query_set: t.query_set,
                    beginning_of_pass_write_index: t.beginning_of_pass_write_index,
                    end_of_pass_write_index: None,
                }),
            });
            pass.set_pipeline(&p.mark);
            pass.set_bind_group(0, &*g.mark, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(4), 1);
            pass.set_pipeline(&p.compact);
            pass.set_bind_group(0, &*g.compact, &[]);
            pass.dispatch_workgroups(slots.div_ceil(64) as u32, 1, 1);
            pass.set_pipeline(&p.finalize);
            pass.dispatch_workgroups(
                (state.templates.len().max(state.levels) as u32).div_ceil(64),
                1,
                1,
            );
        }
        for level in 0..state.levels {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("N8 stale page raster"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &r.id_layers[level],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &r.depth_layers[level],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                ..Default::default()
            });
            self.encode_virtual_layer(&mut pass, draws, level, [&p.clear, &p.caster], &g.raster);
            drop(pass);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("N8 stale page peel"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &r.peel_id_layers[level],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &r.peel_layers[level],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                }),
                ..Default::default()
            });
            self.encode_virtual_layer(
                &mut pass,
                draws,
                level,
                [&p.clear, &p.caster_peel],
                &g.raster_peel,
            );
        }
    }

    /// One layer of one level: stale pages cleared and tagged, then every caster draw of
    /// the level, stencil-tested against the tag (zero instances when nothing is stale).
    fn encode_virtual_layer(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draws: &[PreparedDraw<'_>],
        level: usize,
        [clear, caster]: [&wgpu::RenderPipeline; 2],
        raster: &wgpu::BindGroup,
    ) {
        let state = self.virtual_shadows.as_ref().unwrap();
        let r = state.resources.as_ref().unwrap();
        let g = r.groups.as_ref().unwrap();
        pass.set_stencil_reference(state.tag);
        pass.set_bind_group(0, raster, &[]);
        pass.set_bind_group(1, &*g.pass, &[level as u32 * PASS_STRIDE as u32]);
        pass.set_pipeline(clear);
        pass.draw_indirect(&r.quad_args, level as u64 * 16);
        pass.set_pipeline(caster);
        for (t, template) in state.templates.iter().enumerate() {
            if template.level != level {
                continue;
            }
            let mesh = draws[template.draw].mesh;
            pass.set_vertex_buffer(0, mesh.vertex.slice(..));
            pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed_indirect(&r.caster_args, t as u64 * 20);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn virtual_shadow_module_validates_without_ray_queries() {
        let source = shader_source();
        assert!(!source.contains("ray_query") && !source.contains("acceleration_structure"));
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        assert_eq!(FRAME_BYTES, 896);
    }
    #[test]
    fn sun_basis_is_orthonormal_and_settings_validate() {
        for toward in [
            glam::DVec3::new(0.4, 0.8, 0.3),
            glam::DVec3::Y,
            -glam::DVec3::Y,
        ] {
            let [u, v, l] = basis(toward);
            for (a, b) in [(u, v), (v, l), (u, l)] {
                assert!(a.dot(b).abs() < 1e-12);
            }
            assert!((u.cross(v) - l).length() < 1e-12);
        }
        assert!(VirtualShadowSettings::default().validate().is_ok());
        let bad = VirtualShadowSettings {
            pages: 12,
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }
}
