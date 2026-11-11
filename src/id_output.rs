//! Entity-ID output on the wgpu renderer (docs/design-id-output.md): one compute pass per
//! frame that turns the frame's settled surfaces into the embedder's ids, an R32Uint target
//! with two layers (0: opaque, 1: front), and rectangle readbacks that never wait on the
//! frame thread. Off by default: an engine without ids encodes nothing here.
//!
//! All of a view's output state is `ViewOutputs`, one renderer field (a view's, once views
//! exist): its ids here and its view mode (view_modes.rs).
use super::{Depth, PreparedDraw, Renderer};
use crate::{
    Draw, Error, FrameId,
    engine::ViewId,
    mesh::matrix_bytes,
    object_ids::{
        ID_SLOTS, ID_TICKS, IdPixel, IdReadback, IdRect, IdRequest, IdsLost, TagTables,
    },
    observe::{AllocationKind, Owned, ResourceKind},
};
use std::sync::{Arc, OnceLock};

const SHADER: &str = include_str!("../shaders/id_output.wgsl");
const HANDOFF_MARK: &str = "// ---- Handoff:";
/// The ID pass module: the shader up to its handoff section.
pub(super) fn id_source() -> String {
    SHADER.split_once(HANDOFF_MARK).unwrap().0.to_owned()
}
/// The handoff module: the shader's handoff section alone.
pub(super) fn handoff_source() -> String {
    format!("{HANDOFF_MARK}{}", SHADER.split_once(HANDOFF_MARK).unwrap().1)
}
const GLASS_SHADER: &str = include_str!("../shaders/id_glass.wgsl");
/// Per-draw uniform stride of the glass raster (the device's offset alignment, at most).
const DRAW_STRIDE: u64 = 256;
const FRAME_BYTES: u64 = 32;
const RECONSTRUCTED: u32 = 1;
const STILL_CAMERA: u32 = 2;
const PREVIOUS: u32 = 4;
const GLASS: u32 = 8;

/// A view's optional outputs: entity ids and the view mode's shading (view_modes.rs).
#[derive(Default)]
pub(crate) struct ViewOutputs {
    pub ids: IdOutput,
    pub modes: super::view_modes::ViewModes,
}

/// One view's ID output: its switch, its tables, its targets and its readbacks.
#[derive(Default)]
pub(crate) struct IdOutput {
    enabled: bool,
    /// The tables the next frame uses, and whether the GPU copy is behind them.
    tables: TagTables,
    tables_dirty: bool,
    /// Moved placements of the last encoded frame, one bit each.
    moved: Vec<u32>,
    gpu: Option<Gpu>,
    glass: Option<Glass>,
    targets: Option<Targets>,
    /// The last encoded frame's unjittered view-projection, for the still-camera test.
    camera: Option<[u8; 64]>,
    next_request: u64,
    pending: Vec<Pending>,
}

/// Created by the first frame with ids.
struct Gpu {
    layout: Owned<wgpu::BindGroupLayout>,
    pipeline: wgpu::ComputePipeline,
    frame: Owned<wgpu::Buffer>,
    /// One id layer into a caller's Rgba8Unorm target (`write_ids_to`).
    handoff: Option<(Owned<wgpu::BindGroupLayout>, wgpu::RenderPipeline, Owned<wgpu::Buffer>)>,
    /// Bound for absent inputs: 1x1 zero textures of each format.
    zero_uint: Depth,
    zero_rg: Depth,
    zero_rgba: Depth,
    entries: Option<(Owned<wgpu::Buffer>, u64)>,
    ranges: Option<(Owned<wgpu::Buffer>, u64)>,
    moved: Option<(Owned<wgpu::Buffer>, u64)>,
}

/// The glass raster: created by the first frame with visible transparent draws.
struct Glass {
    /// The prelude it was composed from; a prelude reload recomposes it.
    prelude: String,
    layout: Owned<wgpu::BindGroupLayout>,
    pipeline: wgpu::RenderPipeline,
    /// Camera block (80 bytes: the prelude's `Camera`), then at 256 eye and forward.
    view: Owned<wgpu::Buffer>,
    /// One `DRAW_STRIDE` slot per transparent draw: its first transparent instance.
    draws: Option<(Owned<wgpu::Buffer>, u64)>,
    /// Transparent instance + 1 (R32Uint) and reversed-Z depth, at the output size.
    target: Option<(Depth, Depth)>,
}

struct Targets {
    /// Two layers of R32Uint: opaque ids, front ids.
    ids: Owned<wgpu::Texture>,
    ids_view: Owned<wgpu::TextureView>,
    /// Settled (placement + 1, primitive), ping-ponged: the previous frame's is read.
    settled: [Depth; 2],
    parity: usize,
    /// The frame that last wrote `ids` (and `settled[parity]`); None before one.
    frame: Option<FrameId>,
    _allocation: crate::observe::Allocation,
}

/// A readback in flight: one of `ID_SLOTS`.
struct Pending {
    request: IdRequest,
    view: ViewId,
    frame: FrameId,
    rect: IdRect,
    buffer: Owned<wgpu::Buffer>,
    row: u32,
    mapped: Arc<OnceLock<Result<(), wgpu::BufferAsyncError>>>,
    /// The renderer's frame count at the request: `ID_TICKS` frames later an unmapped copy
    /// is reported lost and only waits for its map to free the slot.
    issued: u64,
    lost: bool,
}

impl IdOutput {
    pub fn enabled(&self) -> bool {
        self.enabled
    }
}

impl Renderer {
    /// Turns the per-frame ID pass on or off. Off frees its targets and encodes nothing.
    pub fn set_id_output(&mut self, enabled: bool) {
        self.outputs.ids.enabled = enabled;
        if !enabled {
            self.outputs.ids.targets = None;
            self.outputs.ids.camera = None;
        }
    }
    pub fn id_output(&self) -> bool {
        self.outputs.ids.enabled
    }
    /// The id tables of the next frames' placements (`TagTables`): `opaque` per instance of
    /// the frame's opaque draws in draw order (the hit record's placements), `transparent` per
    /// instance of its transparent draws in draw order. Missing entries read as untagged.
    pub fn set_object_tags(&mut self, tables: &TagTables) {
        if self.outputs.ids.tables != *tables {
            self.outputs.ids.tables.clone_from(tables);
            self.outputs.ids.tables_dirty = true;
        }
    }
    /// The ID target of the last frame with ids: R32Uint, the output's size, layer 0 the
    /// opaque ids and layer 1 the front ids. None before such a frame.
    pub fn id_texture(&self) -> Option<&wgpu::Texture> {
        self.outputs
            .ids
            .targets
            .as_ref()
            .filter(|t| t.frame.is_some())
            .map(|t| &*t.ids)
    }
    /// Queues a copy of `rect` of the last frame's ids behind that frame and returns at
    /// once; the result arrives from `take_ids` (the engine's `poll`) when the copy lands.
    /// Refused while `ID_SLOTS` readbacks are in flight; `view` names the result.
    pub fn request_ids(&mut self, view: ViewId, rect: IdRect) -> Result<IdRequest, Error> {
        self.check_fault()?;
        let targets = self.outputs.ids.targets.as_ref();
        let (targets, frame) = targets
            .and_then(|t| Some((t, t.frame?)))
            .ok_or(Error::InvalidInput("no frame with ids to read"))?;
        let size = targets.ids.size();
        rect.check(size.width, size.height)?;
        if self.outputs.ids.pending.len() >= ID_SLOTS {
            return Err(Error::InvalidInput(
                "every id readback slot is busy; retry after the next poll",
            ));
        }
        let row = (rect.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("id readback"),
                size: u64::from(row) * u64::from(rect.height) * 2,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.ids,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(rect.height),
                },
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 2,
            },
        );
        let mapped = Arc::new(OnceLock::new());
        let callback = mapped.clone();
        encoder.map_buffer_on_submit(&buffer, wgpu::MapMode::Read, .., move |result| {
            let _ = callback.set(result);
        });
        // Queue order puts the copy behind the last frame and ahead of the next one; the
        // submitted copy keeps the texture alive through a resize or cut.
        self.queue.submit([encoder.finish()]);
        let request = IdRequest(self.outputs.ids.next_request);
        self.outputs.ids.next_request += 1;
        self.outputs.ids.pending.push(Pending {
            request,
            view,
            frame,
            rect,
            buffer,
            row,
            mapped,
            issued: self.frames,
            lost: false,
        });
        Ok(request)
    }
    /// The readbacks' progress: finished ones in request order, and ones still unmapped
    /// `ID_TICKS` rendered frames after their request, reported lost once. Polls the device
    /// without waiting.
    pub fn take_ids(&mut self) -> Result<(Vec<IdReadback>, Vec<IdsLost>), Error> {
        let (mut done, mut lost) = (Vec::new(), Vec::new());
        if self.outputs.ids.pending.is_empty() {
            return Ok((done, lost));
        }
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(Error::Poll)?;
        let mut kept = Vec::with_capacity(self.outputs.ids.pending.len());
        for mut p in std::mem::take(&mut self.outputs.ids.pending) {
            let Some(result) = p.mapped.get() else {
                if self.frames >= p.issued + u64::from(ID_TICKS) && !p.lost {
                    p.lost = true;
                    lost.push(IdsLost {
                        request: p.request,
                        view: p.view,
                    });
                }
                kept.push(p);
                continue;
            };
            if p.lost || result.is_err() {
                // The slot frees; a lost request was already reported, a failed map is.
                if !p.lost {
                    lost.push(IdsLost {
                        request: p.request,
                        view: p.view,
                    });
                }
                continue;
            }
            done.push(read(&p)?);
        }
        self.outputs.ids.pending = kept;
        Ok((done, lost))
    }

    /// The frame's ID pass, after reconstruction and the transparent pass: called only on
    /// frames with a hit record (traced or visibility-buffer primaries) while ids are on.
    pub(super) fn encode_ids(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &[PreparedDraw<'_>],
        transparent: &[Draw<'_>],
        reconstructed: bool,
    ) {
        let hit_size = self.primary.as_ref().unwrap().targets.as_ref().unwrap().hit.texture.size();
        // Reconstructed frames resolve at the output size (upscaled or not); others at the
        // hit record's, which is then the output's.
        let size = match (&self.upscale, reconstructed) {
            (Some(u), true) => u.output.size(),
            _ => hit_size,
        };
        if self.outputs.ids.gpu.is_none() {
            self.outputs.ids.gpu = Some(self.id_gpu());
        }
        if self
            .outputs
            .ids
            .targets
            .as_ref()
            .is_none_or(|t| t.ids.size().width != size.width || t.ids.size().height != size.height)
        {
            self.outputs.ids.targets = Some(self.id_targets(size));
            self.outputs.ids.camera = None;
        }
        // Moved placements: a transform changed since the last submitted frame, or skinned.
        let placements: usize = draws.iter().map(|d| d.instances.len()).sum();
        let mut moved = vec![0u32; placements.div_ceil(32).max(1)];
        let mut base = 0;
        for d in draws {
            let models = &d.instances.state.models;
            let previous = d.instances.state.previous_models(self.frames);
            for (slot, model) in models.iter().enumerate() {
                let changed = d.mesh.skin.is_some()
                    || model.to_cols_array().map(f32::to_bits)
                        != previous[slot].to_cols_array().map(f32::to_bits);
                if changed {
                    moved[(base + slot) / 32] |= 1 << ((base + slot) % 32);
                }
            }
            base += models.len();
        }
        // The entry table as this frame's placements need it: untagged past the tables.
        let tables = &self.outputs.ids.tables;
        let mut entries = tables.opaque.clone();
        entries.resize(placements, [0, 0]);
        entries.extend_from_slice(&tables.transparent);
        entries.resize(placements + transparent.iter().map(|d| d.instances.len()).sum::<usize>(), [0, 0]);
        if entries.is_empty() {
            entries.push([0, 0]);
        }
        let mut ranges = tables.ranges.clone();
        if ranges.is_empty() {
            ranges.push([0; 4]);
        }
        let camera = self
            .transparency
            .camera
            .map(|c| matrix_bytes(c.matrix(size.width, size.height)));
        let still = camera.is_some() && camera == self.outputs.ids.camera;
        self.outputs.ids.camera = camera;
        let frame = FrameId(self.frames + 1);
        // The previous frame's settled surfaces name its placements, which spawning or
        // despawning renumbers: they continue only with reconstruction's history, which
        // restarts on every such change (its key holds the traced geometry generation, which
        // a batch's new instance count or buffers bump). Unreconstructed frames need none.
        let continued = reconstructed
            && self
                .reconstruction_state
                .as_ref()
                .is_some_and(|s| !s.plan.reset);
        let targets = self.outputs.ids.targets.as_mut().unwrap();
        let previous = targets.frame.is_some() && continued;
        targets.parity = 1 - targets.parity;
        targets.frame = Some(frame);

        let dirty = std::mem::take(&mut self.outputs.ids.tables_dirty);
        let gpu = self.outputs.ids.gpu.as_mut().unwrap();
        let (device, queue, ledger) = (&self.device, &self.queue, &self.ledger);
        upload(device, queue, ledger, &mut gpu.entries, "id entries", words(entries.as_flattened()), dirty);
        upload(device, queue, ledger, &mut gpu.ranges, "id ranges", words(ranges.as_flattened()), dirty);
        let moved_changed = moved != self.outputs.ids.moved;
        upload(device, queue, ledger, &mut gpu.moved, "id moved placements", words(&moved), moved_changed);
        self.outputs.ids.moved = moved;
        // Glass: only where the transparent pass ran this frame (native lit frames with
        // visible transparent draws, not data modes), which leaves its opaque depth; its
        // target outlives frames that skip it.
        let glass = self.upscale.is_none()
            && !self.outputs.modes.data()
            && transparent.iter().any(|d| !d.instances.state.ids.is_empty())
            && self.transparency.opaque_depth().is_some();
        if glass {
            self.encode_glass(encoder, transparent, size);
        }
        let flags = if reconstructed { RECONSTRUCTED } else { 0 }
            | if still { STILL_CAMERA } else { 0 }
            | if previous { PREVIOUS } else { 0 }
            | if glass { GLASS } else { 0 };
        let tile = super::primary_ray_gpu::TRACE_TILE;
        let frame_words = [size.width, size.height, flags, placements as u32, tile[0], tile[1], 0, 0];
        let gpu = self.outputs.ids.gpu.as_ref().unwrap();
        self.queue.write_buffer(&gpu.frame, 0, &words(&frame_words));

        let targets = self.outputs.ids.targets.as_ref().unwrap();
        let primary_targets = self.primary.as_ref().unwrap().targets.as_ref().unwrap();
        let (anchors, layers) = match (reconstructed, &self.reconstruction_state) {
            (true, Some(state)) => state.written_slots(),
            _ => (&*gpu.zero_rg.view, &*gpu.zero_rgba.view),
        };
        use wgpu::BindingResource as R;
        let entries = [
            (0, gpu.frame.as_entire_binding()),
            (1, R::TextureView(&primary_targets.hit.view)),
            (2, R::TextureView(&primary_targets.tiles.view)),
            (3, R::TextureView(anchors)),
            (4, R::TextureView(layers)),
            (5, gpu.entries.as_ref().unwrap().0.as_entire_binding()),
            (6, gpu.moved.as_ref().unwrap().0.as_entire_binding()),
            (
                7,
                R::TextureView(match (&self.outputs.ids.glass, glass) {
                    (Some(Glass { target: Some((ids, _)), .. }), true) => &ids.view,
                    _ => &gpu.zero_uint.view,
                }),
            ),
            (8, R::TextureView(&targets.settled[1 - targets.parity].view)),
            (9, R::TextureView(&targets.settled[targets.parity].view)),
            (10, R::TextureView(&targets.ids_view)),
            (11, gpu.ranges.as_ref().unwrap().0.as_entire_binding()),
        ]
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("id output"),
            layout: &gpu.layout,
            entries: &entries,
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("id output"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
    }

    /// Writes the last frame's ids of `layer` (0 opaque, 1 front) into `target`: an
    /// Rgba8Unorm render attachment of the ID target's size, each 32-bit id packed bit-exact
    /// (r = bits 0-7 ... a = bits 24-31). Queued behind that frame; nothing waits.
    pub fn write_ids_to(&mut self, target: &wgpu::Texture, layer: u32) -> Result<(), Error> {
        self.check_fault()?;
        let targets = self.outputs.ids.targets.as_ref().filter(|t| t.frame.is_some());
        let size = targets
            .ok_or(Error::InvalidInput("no frame with ids to write"))?
            .ids
            .size();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        if target.format() != wgpu::TextureFormat::Rgba8Unorm
            || !target.usage().contains(usage)
            || (target.width(), target.height()) != (size.width, size.height)
            || target.sample_count() != 1
            || target.mip_level_count() != 1
            || target.depth_or_array_layers() != 1
        {
            return Err(Error::InvalidInput(
                "id target must be a single-sample, single-mip 2D Rgba8Unorm texture of the view's size with RENDER_ATTACHMENT | COPY_SRC usage",
            ));
        }
        if self.outputs.ids.gpu.as_ref().unwrap().handoff.is_none() {
            let handoff = self.id_handoff();
            self.outputs.ids.gpu.as_mut().unwrap().handoff = Some(handoff);
        }
        let gpu = self.outputs.ids.gpu.as_ref().unwrap();
        let targets = self.outputs.ids.targets.as_ref().unwrap();
        let (layout, pipeline, uniform) = gpu.handoff.as_ref().unwrap();
        self.queue.write_buffer(uniform, 0, &words(&[layer, 0, 0, 0]));
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("id handoff"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&targets.ids_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("id handoff"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        Ok(())
    }

    fn id_handoff(
        &self,
    ) -> (Owned<wgpu::BindGroupLayout>, wgpu::RenderPipeline, Owned<wgpu::Buffer>) {
        let layout = self.ledger.own(
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("id handoff"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Uint,
                                view_dimension: wgpu::TextureViewDimension::D2Array,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(16),
                            },
                            count: None,
                        },
                    ],
                }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("id handoff"),
                bind_group_layouts: &[Some(&*layout)],
                immediate_size: 0,
            });
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("id handoff"),
                source: wgpu::ShaderSource::Wgsl(handoff_source().into()),
            });
        let pipeline = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("id handoff"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("handoff_vs"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("handoff_ids_fs"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: Default::default(),
                    multiview_mask: None,
                    cache: None,
                });
        let uniform = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("id handoff layer"),
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        (layout, pipeline, uniform)
    }

    /// Rasterizes the visible transparent draws' nearest surfaces into the glass target.
    fn encode_glass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        transparent: &[Draw<'_>],
        size: wgpu::Extent3d,
    ) {
        if self
            .outputs
            .ids
            .glass
            .as_ref()
            .is_none_or(|g| g.prelude != self.source)
        {
            self.outputs.ids.glass = Some(self.id_glass());
        }
        let resized = self.outputs.ids.glass.as_ref().unwrap().target.as_ref().is_none_or(|(t, _)| {
            t.texture.size().width != size.width || t.texture.size().height != size.height
        });
        if resized {
            use wgpu::{TextureFormat as F, TextureUsages as U};
            let ids = self.id_target("id glass", size, F::R32Uint, U::RENDER_ATTACHMENT);
            let depth = self.id_target("id glass depth", size, F::Depth32Float, U::RENDER_ATTACHMENT);
            self.outputs.ids.glass.as_mut().unwrap().target = Some((ids, depth));
        }
        // The unjittered camera block, eye and forward, as transparency_gpu.rs computes them.
        let camera = self.transparency.camera.unwrap();
        let inverse_view = camera.view().inverse();
        let eye = inverse_view.w_axis.truncate();
        let forward = -inverse_view.z_axis.truncate().normalize();
        let mut view = [0u8; 288];
        view[..64].copy_from_slice(&matrix_bytes(camera.matrix(size.width, size.height)));
        view[64..68].copy_from_slice(&(crate::ViewMode::Lit as u32).to_ne_bytes());
        let words = [eye.x, eye.y, eye.z, 0.0, forward.x, forward.y, forward.z, 0.0];
        for (i, v) in words.into_iter().enumerate() {
            view[256 + i * 4..260 + i * 4].copy_from_slice(&v.to_ne_bytes());
        }
        // Each draw's first transparent instance in the tag table's transparent part.
        let mut draws = vec![0u8; DRAW_STRIDE as usize * transparent.len().max(1)];
        let mut first = 0u32;
        for (i, d) in transparent.iter().enumerate() {
            let at = i * DRAW_STRIDE as usize;
            draws[at..at + 4].copy_from_slice(&first.to_ne_bytes());
            first += d.instances.len() as u32;
        }
        let glass = self.outputs.ids.glass.as_mut().unwrap();
        self.queue.write_buffer(&glass.view, 0, &view);
        upload_with(
            &self.device,
            &self.queue,
            &self.ledger,
            &mut glass.draws,
            "id glass draws",
            draws,
            wgpu::BufferUsages::UNIFORM,
        );
        let glass = self.outputs.ids.glass.as_ref().unwrap();
        let (target, depth) = glass.target.as_ref().unwrap();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("id glass"),
            layout: &glass.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &glass.view,
                        offset: 0,
                        size: wgpu::BufferSize::new(80),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 40,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &glass.view,
                        offset: 256,
                        size: wgpu::BufferSize::new(32),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 41,
                    resource: wgpu::BindingResource::TextureView(
                        self.transparency.opaque_depth().unwrap(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 42,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &glass.draws.as_ref().unwrap().0,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
            ],
        });
        let sampling: Vec<_> = transparent
            .iter()
            .map(|d| self.sampling_group(d.material))
            .collect();
        let default = self.default_material.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("id glass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_pipeline(&glass.pipeline);
        for (i, (d, sampling)) in transparent.iter().zip(&sampling).enumerate() {
            let visible = d.instances.state.ids.len() as u32;
            if visible == 0 {
                continue;
            }
            pass.set_bind_group(0, &group, &[(i as u64 * DRAW_STRIDE) as u32]);
            pass.set_bind_group(1, &*d.instances.bind, &[]);
            let material: &wgpu::BindGroup = match sampling {
                Some(group) => group,
                None => &d.material.unwrap_or(default).bind,
            };
            pass.set_bind_group(2, material, &[]);
            pass.set_vertex_buffer(0, d.mesh.vertex.slice(..));
            pass.set_vertex_buffer(1, d.instances.id_buffer.slice(..));
            pass.set_index_buffer(d.mesh.index.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..d.mesh.index_count, 0, 0..visible);
        }
    }

    fn id_glass(&self) -> Glass {
        use wgpu::ShaderStages as S;
        let uniform = |binding, size, dynamic| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: wgpu::BufferSize::new(size),
            },
            count: None,
        };
        let layout = self.ledger.own(
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("id glass"),
                    entries: &[
                        uniform(0, 80, false),
                        uniform(40, 32, false),
                        wgpu::BindGroupLayoutEntry {
                            binding: 41,
                            visibility: S::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        uniform(42, 16, true),
                    ],
                }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("id glass"),
                bind_group_layouts: &[
                    Some(&*layout),
                    Some(&*self.instance_bindings),
                    Some(&*self.material_bindings),
                ],
                immediate_size: 0,
            });
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("id glass"),
                source: wgpu::ShaderSource::Wgsl(format!("{}\n{GLASS_SHADER}", self.source).into()),
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("id glass"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("id_glass_vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 48,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &super::VERTEX_ATTRIBUTES,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: 4,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &super::MODEL_ATTRIBUTES,
                        }),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("id_glass_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::TextureFormat::R32Uint.into())],
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let view = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("id glass view"),
                size: 288,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        Glass {
            prelude: self.source.clone(),
            layout,
            pipeline,
            view,
            draws: None,
            target: None,
        }
    }

    fn id_gpu(&self) -> Gpu {
        use wgpu::{ShaderStages as S, TextureSampleType as T};
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let written = |binding, format, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: S::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format,
                view_dimension,
            },
            count: None,
        };
        let uint = T::Uint;
        let layout = self.ledger.own(
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("id output"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: S::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(FRAME_BYTES),
                            },
                            count: None,
                        },
                        texture(1, uint),
                        texture(2, uint),
                        texture(3, uint),
                        texture(4, uint),
                        storage(5),
                        storage(6),
                        texture(7, uint),
                        texture(8, uint),
                        written(9, wgpu::TextureFormat::Rg32Uint, wgpu::TextureViewDimension::D2),
                        written(10, wgpu::TextureFormat::R32Uint, wgpu::TextureViewDimension::D2Array),
                        storage(11),
                    ],
                }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("id output"),
                bind_group_layouts: &[Some(&*layout)],
                immediate_size: 0,
            });
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("id output"),
                source: wgpu::ShaderSource::Wgsl(id_source().into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("id output"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("id_resolve"),
                compilation_options: Default::default(),
                cache: None,
            });
        let frame = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("id output frame"),
                size: FRAME_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let one = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        use wgpu::TextureFormat as F;
        Gpu {
            layout,
            pipeline,
            frame,
            zero_uint: self.id_target("id zero", one, F::R32Uint, wgpu::TextureUsages::empty()),
            zero_rg: self.id_target("id zero", one, F::Rg32Uint, wgpu::TextureUsages::empty()),
            zero_rgba: self.id_target("id zero", one, F::Rgba32Uint, wgpu::TextureUsages::empty()),
            entries: None,
            ranges: None,
            moved: None,
            handoff: None,
        }
    }

    /// A 2D target; new textures are zero-initialised by wgpu.
    fn id_target(
        &self,
        label: &str,
        size: wgpu::Extent3d,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
    ) -> Depth {
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | usage,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let bytes = u64::from(size.width)
            * u64::from(size.height)
            * u64::from(format.block_copy_size(None).unwrap());
        Depth {
            texture,
            view,
            _allocation: self.ledger.allocate(AllocationKind::Target, bytes),
        }
    }

    fn id_targets(&self, size: wgpu::Extent3d) -> Targets {
        let size = wgpu::Extent3d {
            depth_or_array_layers: 1,
            ..size
        };
        let ids = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("object ids"),
                size: wgpu::Extent3d {
                    depth_or_array_layers: 2,
                    ..size
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Uint,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let ids_view = self.ledger.own(
            ids.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            }),
            ResourceKind::TextureView,
        );
        let settled = || {
            self.id_target(
                "settled id surfaces",
                size,
                wgpu::TextureFormat::Rg32Uint,
                wgpu::TextureUsages::STORAGE_BINDING,
            )
        };
        Targets {
            ids,
            ids_view,
            settled: [settled(), settled()],
            parity: 0,
            frame: None,
            _allocation: self.ledger.allocate(
                AllocationKind::Target,
                u64::from(size.width) * u64::from(size.height) * 8,
            ),
        }
    }
}

/// Writes `bytes` to a buffer of `usage`, recreating it at a new size.
fn upload_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ledger: &crate::observe::Ledger,
    slot: &mut Option<(Owned<wgpu::Buffer>, u64)>,
    label: &str,
    bytes: Vec<u8>,
    usage: wgpu::BufferUsages,
) {
    let size = bytes.len() as u64;
    if slot.as_ref().is_none_or(|(_, s)| *s != size) {
        *slot = Some((
            ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            size,
        ));
    }
    queue.write_buffer(&slot.as_ref().unwrap().0, 0, &bytes);
}

/// Writes `bytes` to a storage buffer, growing it to fit; `changed` false skips the write
/// when the buffer already holds them.
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ledger: &crate::observe::Ledger,
    slot: &mut Option<(Owned<wgpu::Buffer>, u64)>,
    label: &str,
    bytes: Vec<u8>,
    changed: bool,
) {
    let size = bytes.len() as u64;
    if slot.as_ref().is_none_or(|(_, s)| *s != size) {
        *slot = Some((
            ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            size,
        ));
    } else if !changed {
        return;
    }
    queue.write_buffer(&slot.as_ref().unwrap().0, 0, &bytes);
}

fn words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_ne_bytes()).collect()
}

/// A mapped readback's pixels.
fn read(p: &Pending) -> Result<IdReadback, Error> {
    let bytes = p
        .buffer
        .get_mapped_range(..)
        .map_err(|e| Error::Shader(e.to_string()))?;
    let layer = p.row as usize * p.rect.height as usize;
    let word = |offset: usize| u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let mut ids = Vec::with_capacity(p.rect.width as usize * p.rect.height as usize);
    for y in 0..p.rect.height as usize {
        for x in 0..p.rect.width as usize {
            let at = y * p.row as usize + x * 4;
            ids.push(IdPixel {
                opaque: word(at),
                front: word(layer + at),
            });
        }
    }
    drop(bytes);
    p.buffer.unmap();
    Ok(IdReadback {
        request: p.request,
        view: p.view,
        frame: p.frame,
        rect: p.rect,
        ids,
    })
}

#[cfg(test)]
#[path = "id_output_tests.rs"]
mod tests;
