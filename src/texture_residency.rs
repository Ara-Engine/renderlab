//! Texture mip residency (M3, docs/memory-m3.md).
//!
//! A streamed texture (`Renderer::try_upload_streamed_texture`) keeps its prepared mip
//! chain on the CPU and only source levels `top..` on the GPU, as a texture whose level 0
//! is source level `top`. Sampling it with the same normalized coordinates, gradients
//! and sampler selects hardware LOD `λ - top`, because its level-0 extent is the source's
//! divided by 2^top, so every sample whose finest touched level is at least `top` is
//! bit-identical to sampling the full chain (measured by `gpu_residency_probe`).
//!
//! Requests come from the programmable sampling helpers themselves
//! (`shaders/program_traced.wgsl`): each `sample_*_grad` call of a streamed texture also
//! samples the shared level probe (level j holds 2^-j) with the material's own sampler
//! and the footprint scaled to the probe, so the hardware selects the same trilinear pair
//! and anisotropic footprint; the decoded finest level is recorded per draw slot and
//! role, as is any sample that needed a level below the resident top (a miss). User
//! programs reach textures only through these helpers, so they are covered.
//!
//! Anisotropic filtering needs one level of margin. On NVIDIA 617.14 the probe reports
//! anisotropic levels exactly too, but a tail whose base is the finest touched level
//! samples differently from the full chain for about half of all footprints, at every
//! level; tails that keep one level more are bit-identical (`gpu_residency_probe`). The
//! helpers of anisotropic slots therefore request one level finer than they touch.
//! `survey` records the touched levels themselves: what extent-preserving sparse
//! residency (not exposed by wgpu 30) would keep (docs/memory-m3.md).
use super::*;
use crate::material::{TextureInner, TextureStorage};
use std::collections::BTreeMap;
use std::sync::Weak;

/// log2 of the shared level probe's extent.
pub(crate) const PROBE_LOG2: u32 = 6;
/// Source levels at or beyond this one (64 x 64 for a 4K texture) are always resident:
/// the probe has no finer distinction there, and they hold ~0.02% of a 4K chain.
pub(crate) const MAX_TOP: u32 = PROBE_LOG2;
/// Draw slots times the three material roles.
pub(crate) const SLOTS: usize = 3 * TRACED_DRAWS as usize;
/// Counter copies, chosen per 8x8 screen tile: every sample of a frame updates one
/// counter word, and same-address L2 traffic from every warp would serialize on one
/// slice. Each bucket's counters are 2,304 bytes apart.
pub(crate) const BUCKETS: usize = 32;
/// GPU words: per slot and role the residency word, then per bucket the request codes,
/// the misses and the surveyed probe levels.
pub(crate) const WORDS: usize = SLOTS + BUCKETS * 3 * SLOTS;
/// Byte offset and size of the per-frame request, miss and survey counters.
pub(crate) const REQUEST_OFFSET: u64 = (SLOTS * 4) as u64;
pub(crate) const REQUEST_BYTES: u64 = (BUCKETS * 3 * SLOTS * 4) as u64;
/// Residency word bits: a streamed texture; probe its samples and request the touched
/// level less the word's margin (bits 14-15); record the touched level in the survey.
pub(crate) const STREAMED: u32 = 1 << 31;
pub(crate) const PROBED: u32 = 1 << 30;
pub(crate) const SURVEY: u32 = 1 << 29;
/// Levels an anisotropic slot keeps above its touched level (measured, see above).
pub(crate) const ANISOTROPIC_MARGIN: u32 = 1;
/// Frames a level stays resident after the last frame whose finest request reached it.
pub(crate) const HOLD_FRAMES: u64 = 120;
/// Automatic camera-cut thresholds: a larger view change verifies the frame.
pub(crate) const CUT_RADIANS: f32 = 5.0 * std::f32::consts::PI / 180.0;
pub(crate) const CUT_METRES: f32 = 0.25;
/// Level bytes one servicing step queues from the CPU; verification waits for each step,
/// so staging memory stays bounded however many levels a cut needs.
pub(crate) const STEP_BYTES: u64 = 256 << 20;

/// How streamed textures stay resident. Textures from `try_upload_texture` are always
/// fully resident.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextureResidency {
    /// Every level of every streamed texture.
    Full,
    /// Every streamed frame is verified before it is returned: the renderer waits for the
    /// frame, and if any sample requested a level that was not resident, uploads it and
    /// shades and presents again. No returned frame samples a missing level; the cost is
    /// that the CPU waits for each streamed frame, so its next frame's CPU work no longer
    /// overlaps this frame's GPU work.
    #[default]
    Exact,
    /// Requests from completed frames decide which levels stay, without waiting. Only
    /// frames the renderer cannot vouch for are verified: a texture's first use, a
    /// target-size change, `mark_camera_cut`, and view changes beyond 5 degrees, 0.25 m
    /// or 1% zoom. Elsewhere a sample may need a level that feedback has not made
    /// resident yet (on the quarry's motion phases, 6-15 samples in about 300 frames):
    /// it filters from the finest resident level and is counted as a miss.
    Feedback,
}

/// Residency counters since the renderer was created.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResidencyStats {
    pub streamed_textures: u64,
    /// Current GPU bytes of streamed textures, and what their full chains would take.
    pub resident_bytes: u64,
    pub full_bytes: u64,
    /// Samples of returned frames that needed a non-resident level (the fallback sampled
    /// the finest resident one), and the frames containing any.
    pub misses: u64,
    pub missed_frames: u64,
    pub verified_frames: u64,
    /// Verified frames whose first shading missed: serviced, then shaded again.
    pub reshaded_frames: u64,
    /// Misses found and serviced by verification before the frame was returned.
    pub serviced_misses: u64,
    pub transitions: u64,
    /// Level bytes uploaded after each texture's initial upload.
    pub uploaded_bytes: u64,
}

/// Source levels `top..` of one streamed texture on the GPU.
pub(crate) struct Resident {
    pub texture: Owned<wgpu::Texture>,
    pub view: Owned<wgpu::TextureView>,
    pub top: u32,
    pub bytes: u64,
    _allocation: Allocation,
}

pub(crate) struct Entry {
    owner: Weak<TextureInner>,
    source: Arc<asset::Texture>,
    format: wgpu::TextureFormat,
    max_top: u32,
    /// log2(extent / probe extent) per axis: the exact probe gradient scales.
    scale: [i32; 2],
    pub resident: Resident,
    pub generation: u64,
    /// Per finest requested level (clamped to `max_top`), the last frame requesting it.
    last: [u64; MAX_TOP as usize + 1],
    /// Whether any completed frame sampled it; unobserved textures never shrink.
    observed: bool,
    /// The finest probe level any surveyed sample needed (`Residency::survey`).
    pub surveyed: Option<u32>,
}
impl Entry {
    fn full_bytes(&self) -> u64 {
        self.source.mips.iter().map(|m| m.rgba8.len() as u64).sum()
    }
    /// The slot word the sampling helpers decode: streamed, probed with this margin,
    /// resident top, probe scales, survey.
    pub fn word(&self, margin: u32, survey: bool) -> u32 {
        word(self.resident.top, self.scale)
            | PROBED
            | margin << 14
            | if survey { SURVEY } else { 0 }
    }
}

pub(crate) fn word(top: u32, scale: [i32; 2]) -> u32 {
    let field = |s: i32| (s + 16) as u32 & 31;
    STREAMED | top | field(scale[0]) << 4 | field(scale[1]) << 9
}

/// The coarsest top keeping every level requested within the last `hold` frames.
pub(crate) fn desired_top(last: &[u64], now: u64, hold: u64, max_top: u32) -> u32 {
    (0..=max_top)
        .find(|&level| last[level as usize] != 0 && last[level as usize] + hold >= now)
        .unwrap_or(max_top)
}

/// Source levels a move from `old` to `new` resident tops writes from the CPU, then the
/// levels it copies from the old texture (at `level - old`) to the new (at `level - new`).
pub(crate) fn transition_levels(
    old: u32,
    new: u32,
    levels: u32,
) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
    (new..old.max(new), old.max(new)..levels)
}

/// The finest level a streamed texture may drop to: power-of-two extents only (the probe
/// scales and the tail's level-0 extent must be exact), at most `MAX_TOP`.
pub(crate) fn max_top(width: u32, height: u32, levels: u32) -> Option<u32> {
    (width.is_power_of_two() && height.is_power_of_two()).then(|| {
        MAX_TOP
            .min(width.trailing_zeros())
            .min(height.trailing_zeros())
            .min(levels - 1)
    })
}

#[derive(Default)]
pub(crate) struct Residency {
    pub mode: TextureResidency,
    entries: BTreeMap<u64, Entry>,
    next_id: u64,
    /// The next frame is verified (`mark_camera_cut`).
    pub cut: bool,
    /// The last streamed frame's view.
    pub view: Option<ViewKey>,
    /// Probe anisotropic samples too, recording the levels sparse residency would need.
    pub survey: bool,
    pub stats: ResidencyStats,
}
impl Residency {
    pub fn entry(&self, id: u64) -> &Entry {
        &self.entries[&id]
    }
    /// Records one completed frame's finest requested level of a texture.
    pub fn observe(&mut self, id: u64, frame: u64, level: u32) {
        if let Some(e) = self.entries.get_mut(&id) {
            let level = level.min(e.max_top) as usize;
            e.last[level] = e.last[level].max(frame);
            e.observed = true;
        }
    }
    pub fn observed(&self, id: u64) -> bool {
        self.entries.get(&id).is_some_and(|e| e.observed)
    }
    /// Drops entries whose texture handles are gone; the GPU copy outlives queued work.
    pub fn prune(&mut self) {
        self.entries.retain(|_, e| e.owner.strong_count() > 0);
    }
    fn target(&self, id: u64, e: &Entry, now: u64, full: &[u64]) -> u32 {
        if self.mode == TextureResidency::Full || full.contains(&id) {
            0
        } else if !e.observed {
            e.resident.top
        } else {
            desired_top(&e.last, now, HOLD_FRAMES, e.max_top)
        }
    }
}

/// What camera-cut detection compares: eye, unit forward direction, projection scale
/// (zoom) and target size.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewKey {
    pub eye: Vec3,
    pub forward: Vec3,
    pub scale: [f32; 2],
    pub size: [u32; 2],
}
/// Whether a view change from `last` to `next` is large enough to verify the frame.
pub(crate) fn camera_cut(last: Option<ViewKey>, next: ViewKey) -> bool {
    let Some(last) = last else {
        return true;
    };
    last.size != next.size
        || last.eye.distance(next.eye) > CUT_METRES
        || last.forward.dot(next.forward).clamp(-1.0, 1.0).acos() > CUT_RADIANS
        || (0..2).any(|i| (last.scale[i] - next.scale[i]).abs() > 0.01 * last.scale[i].abs())
}

impl Renderer {
    /// Uploads a prepared chain whose levels then stay resident only while the
    /// renderer's `TextureResidency` needs them; the CPU levels are retained for later
    /// uploads. Extents that are not powers of two upload fully resident.
    pub fn try_upload_streamed_texture(
        &mut self,
        data: Arc<asset::Texture>,
    ) -> Result<Option<Texture>, Error> {
        self.check_fault()?;
        let started = Instant::now();
        crate::material::validate_texture(
            &data,
            self.device.limits().max_texture_dimension_2d,
            self.limits.max_texture_bytes,
        )?;
        let preparation = started.elapsed();
        let first = &data.mips[0];
        let Some(max_top) = max_top(first.width, first.height, data.mips.len() as u32) else {
            return self.try_upload_texture(&data);
        };
        let poll = self.collect()?;
        if self.occupied() >= FLIGHTS - 1 {
            self.busy_uploads += 1;
            return Ok(None);
        }
        let slot = self.free_slot().unwrap();
        let started = Instant::now();
        let scopes = OperationScopes::new(&self.device);
        let top = if self.residency.mode == TextureResidency::Full {
            0
        } else {
            max_top
        };
        let format = texture_format(data.role);
        let resident = self.create_resident(&data, format, top, top..data.mips.len() as u32);
        let id = self.residency.next_id;
        self.residency.next_id += 1;
        let texture = Texture(Arc::new(TextureInner {
            renderer: self.identity.clone(),
            role: data.role,
            storage: TextureStorage::Streamed(id),
        }));
        let scale =
            [first.width, first.height].map(|v| v.trailing_zeros() as i32 - PROBE_LOG2 as i32);
        self.residency.entries.insert(
            id,
            Entry {
                owner: Arc::downgrade(&texture.0),
                source: data,
                format,
                max_top,
                scale,
                resident,
                generation: 0,
                last: [0; MAX_TOP as usize + 1],
                observed: false,
                surveyed: None,
            },
        );
        self.residency.stats.streamed_textures += 1;
        if let Some(error) = scopes.finish() {
            return Err(self.fail(error.to_string()));
        }
        self.submit_upload(slot);
        self.upload_cpu = CpuFrame {
            preparation,
            poll,
            upload: started.elapsed(),
            ..Default::default()
        };
        self.check_fault()?;
        Ok(Some(texture))
    }

    /// Selects how streamed textures stay resident; takes effect at the next frame.
    pub fn set_texture_residency(&mut self, mode: TextureResidency) {
        self.residency.mode = mode;
    }
    pub fn texture_residency(&self) -> TextureResidency {
        self.residency.mode
    }
    /// The next frame follows a camera cut: verify it before it is returned.
    pub fn mark_camera_cut(&mut self) {
        self.residency.cut = true;
    }
    pub fn residency_stats(&self) -> ResidencyStats {
        let mut stats = self.residency.stats;
        stats.resident_bytes = self
            .residency
            .entries
            .values()
            .map(|e| e.resident.bytes)
            .sum();
        stats.full_bytes = self.residency.entries.values().map(Entry::full_bytes).sum();
        stats
    }
    /// The resident source top of a streamed texture; 0 for fully resident ones.
    pub fn resident_top(&self, texture: &Texture) -> u32 {
        match texture.0.storage {
            TextureStorage::Full(_) => 0,
            TextureStorage::Streamed(id) => self.residency.entry(id).resident.top,
        }
    }

    /// The shared level probe: 2^PROBE_LOG2 square R16Float, level j filled with 2^-j
    /// (binary16 exponent field 15 - j, zero mantissa), which every filter blends exactly.
    pub(crate) fn level_probe(
        &mut self,
    ) -> (Owned<wgpu::Texture>, Owned<wgpu::TextureView>, Allocation) {
        let size = 1u32 << PROBE_LOG2;
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("texture residency level probe"),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
                mip_level_count: PROBE_LOG2 + 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        let mut bytes = 0;
        for level in 0..=PROBE_LOG2 {
            let extent = size >> level;
            let texels = (((15 - level) as u16) << 10)
                .to_le_bytes()
                .repeat((extent * extent) as usize);
            bytes += texels.len() as u64;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    mip_level: level,
                    ..texture.as_image_copy()
                },
                &texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(extent * 2),
                    rows_per_image: Some(extent),
                },
                wgpu::Extent3d {
                    width: extent,
                    height: extent,
                    depth_or_array_layers: 1,
                },
            );
        }
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let allocation = self.ledger.allocate(AllocationKind::Lut, bytes);
        (texture, view, allocation)
    }

    /// Per streamed texture: extent, resident top, the finest droppable top, generation,
    /// whether a frame sampled it, and the finest surveyed probe level.
    #[cfg(test)]
    pub(crate) fn test_residency_entries(&self) -> serde_json::Value {
        self.residency
            .entries
            .iter()
            .map(|(id, e)| {
                serde_json::json!({
                    "id": id, "width": e.source.mips[0].width, "height": e.source.mips[0].height,
                    "top": e.resident.top, "max_top": e.max_top, "generation": e.generation,
                    "observed": e.observed, "surveyed": e.surveyed,
                    "resident_bytes": e.resident.bytes, "full_bytes": e.full_bytes(),
                })
            })
            .collect()
    }

    /// The texture's current view: its own, or its resident levels.
    pub(crate) fn texture_view<'a>(&'a self, texture: &'a Texture) -> &'a wgpu::TextureView {
        match &texture.0.storage {
            TextureStorage::Full(full) => &full.view,
            TextureStorage::Streamed(id) => &self.residency.entry(*id).resident.view,
        }
    }

    /// A texture holding source levels `top..`, with levels `written` queued from the CPU.
    fn create_resident(
        &mut self,
        data: &asset::Texture,
        format: wgpu::TextureFormat,
        top: u32,
        written: std::ops::Range<u32>,
    ) -> Resident {
        let base = &data.mips[top as usize];
        let texture = self.ledger.own(
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("streamed material texture levels"),
                size: wgpu::Extent3d {
                    width: base.width,
                    height: base.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: data.mips.len() as u32 - top,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            }),
            ResourceKind::Texture,
        );
        for level in written {
            let mip = &data.mips[level as usize];
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    mip_level: level - top,
                    ..texture.as_image_copy()
                },
                &mip.rgba8,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(mip.width * 4),
                    rows_per_image: Some(mip.height),
                },
                wgpu::Extent3d {
                    width: mip.width,
                    height: mip.height,
                    depth_or_array_layers: 1,
                },
            );
            self.uploads.textures += mip.rgba8.len() as u64;
            self.uploads.texture_writes += 1;
            self.uploads.write_calls += 1;
        }
        let view = self.ledger.own(
            texture.create_view(&Default::default()),
            ResourceKind::TextureView,
        );
        let bytes = data.mips[top as usize..]
            .iter()
            .map(|m| m.rgba8.len() as u64)
            .sum();
        Resident {
            texture,
            view,
            top,
            bytes,
            _allocation: self.ledger.allocate(AllocationKind::Texture, bytes),
        }
    }

    /// Moves streamed textures to the ranges their requests need (`full` ids to all
    /// levels). New levels are queued from the CPU and retained levels copied in one
    /// submission now, so any later pass samples complete textures; the old copies retire
    /// when it completes (wgpu keeps them until then). Stops once `budget` CPU bytes are
    /// queued (after at least one move) and returns whether moves remain.
    pub(crate) fn apply_residency(&mut self, full: &[u64], budget: u64) -> bool {
        let now = self.frames + 1;
        let moves: Vec<(u64, u32)> = self
            .residency
            .entries
            .iter()
            .filter_map(|(&id, e)| {
                let target = self.residency.target(id, e, now, full);
                (target != e.resident.top).then_some((id, target))
            })
            .collect();
        if moves.is_empty() {
            return false;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("texture residency transitions"),
            });
        let mut queued = 0;
        let mut more = false;
        for (id, target) in moves {
            if queued >= budget {
                more = true;
                break;
            }
            let e = &self.residency.entries[&id];
            let (source, format, old) = (e.source.clone(), e.format, e.resident.top);
            let levels = source.mips.len() as u32;
            let (written, copied) = transition_levels(old, target, levels);
            let uploaded: u64 = written
                .clone()
                .map(|l| source.mips[l as usize].rgba8.len() as u64)
                .sum();
            let resident = self.create_resident(&source, format, target, written);
            for level in copied {
                let m = &source.mips[level as usize];
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        mip_level: level - old,
                        ..self.residency.entries[&id].resident.texture.as_image_copy()
                    },
                    wgpu::TexelCopyTextureInfo {
                        mip_level: level - target,
                        ..resident.texture.as_image_copy()
                    },
                    wgpu::Extent3d {
                        width: m.width,
                        height: m.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            let e = self.residency.entries.get_mut(&id).unwrap();
            e.resident = resident;
            e.generation += 1;
            self.residency.stats.transitions += 1;
            self.residency.stats.uploaded_bytes += uploaded;
            queued += uploaded;
        }
        self.queue.submit([encoder.finish()]);
        more
    }

    /// Streamed texture ids the draws' materials bind, in draw order without repeats.
    pub(crate) fn streamed_ids<'a>(
        &self,
        materials: impl Iterator<Item = Option<&'a Material>>,
    ) -> Vec<u64> {
        let mut ids = vec![];
        for m in materials {
            for t in &m
                .unwrap_or(self.default_material.as_ref().unwrap())
                ._textures
            {
                if let TextureStorage::Streamed(id) = t.0.storage
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        }
        ids
    }

    /// Whether this streamed frame must be verified before it is returned.
    pub(crate) fn residency_verify(
        &mut self,
        ids: &[u64],
        camera: &Camera,
        size: [u32; 2],
    ) -> bool {
        let view = camera.view().inverse();
        let projection = camera.matrix(size[0], size[1]) * camera.view().inverse();
        let next = ViewKey {
            eye: view.w_axis.truncate(),
            forward: -view.z_axis.truncate().normalize_or_zero(),
            scale: [projection.x_axis.x, projection.y_axis.y],
            size,
        };
        let cut = std::mem::take(&mut self.residency.cut) | camera_cut(self.residency.view, next);
        self.residency.view = Some(next);
        !ids.is_empty()
            && match self.residency.mode {
                // Full residency cannot miss.
                TextureResidency::Full => false,
                TextureResidency::Exact => true,
                TextureResidency::Feedback => {
                    cut || ids.iter().any(|&id| !self.residency.observed(id))
                }
            }
    }

    /// Copies the frame's counters into the slot's readback, mapped with the submission;
    /// `collect` observes them with the residency ids the slot words described.
    pub(crate) fn encode_residency_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        slot: usize,
    ) -> (MapResult, Vec<Option<u64>>) {
        if self.slots[slot].residency_readback.is_none() {
            self.slots[slot].residency_readback = Some(self.residency_buffer());
        }
        let readback = self.slots[slot].residency_readback.as_ref().unwrap();
        let state = self.primary.as_ref().unwrap().residency.as_ref().unwrap();
        encoder.copy_buffer_to_buffer(
            &state.buffer.value,
            REQUEST_OFFSET,
            readback,
            0,
            REQUEST_BYTES,
        );
        let mapped = Arc::new(OnceLock::new());
        let callback = mapped.clone();
        encoder.map_buffer_on_submit(readback, wgpu::MapMode::Read, .., move |result| {
            let _ = callback.set(result);
        });
        (mapped, state.ids.clone())
    }
    fn residency_buffer(&self) -> Owned<wgpu::Buffer> {
        self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("texture residency readback"),
                size: REQUEST_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        )
    }

    /// Verification of a submitted streamed frame: waits for it and observes its
    /// counters; while any sample missed, uploads the requested levels and shades and
    /// presents again into `view`. Shading reads only the hit records, visibility counts
    /// and materials, and the present pass only the HDR target, so repeating them is
    /// exact. Every other pass, timestamps included, ran once in the frame itself.
    pub(crate) fn verify_frame(
        &mut self,
        slot: usize,
        index: wgpu::SubmissionIndex,
        draws: &[PreparedDraw<'_>],
        soft: bool,
        view: &wgpu::TextureView,
    ) -> Result<(), Error> {
        let wait = |r: &Self, index: Option<wgpu::SubmissionIndex>| {
            r.device
                .poll(wgpu::PollType::Wait {
                    submission_index: index,
                    timeout: Some(Duration::from_secs(30)),
                })
                .map_err(Error::Poll)
        };
        self.residency.stats.verified_frames += 1;
        wait(self, Some(index))?;
        self.check_fault()?;
        // Observed here, so `collect` does not count this pass's misses as shown.
        let Some((mapped, mut ids)) = self.slots[slot].residency.take() else {
            return Ok(());
        };
        let Some(Ok(())) = mapped.get() else {
            return Err(self.fail("texture residency verification readback failed".into()));
        };
        let mut words = read_words(self.slots[slot].residency_readback.as_ref().unwrap());
        let shading = if soft {
            primary_ray_gpu::Shading::Soft
        } else {
            primary_ray_gpu::Shading::Hard
        };
        let mut readback = None;
        // Each round grows some texture by at least one level.
        for round in 0..=MAX_TOP + 1 {
            let misses = self.observe_residency(&words, &ids, self.frames);
            if misses == 0 {
                return Ok(());
            }
            self.residency.stats.reshaded_frames += u64::from(round == 0);
            self.residency.stats.serviced_misses += misses;
            // Bounded steps keep staging memory to one step's levels.
            while self.apply_residency(&[], STEP_BYTES) {
                wait(self, None)?;
            }
            let mut uploads = UploadBytes::default();
            self.prepare_primary_materials(draws, &mut uploads);
            self.uploads.add(uploads);
            let readback: &Owned<wgpu::Buffer> =
                readback.get_or_insert_with(|| self.residency_buffer());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            self.encode_request_clear(&mut encoder);
            self.encode_primary_shading(&mut encoder, shading, None);
            self.encode_hdr_present(&mut encoder, view, None);
            let state = self.primary.as_ref().unwrap().residency.as_ref().unwrap();
            encoder.copy_buffer_to_buffer(
                &state.buffer.value,
                REQUEST_OFFSET,
                readback,
                0,
                REQUEST_BYTES,
            );
            ids = state.ids.clone();
            let mapped: MapResult = Arc::new(OnceLock::new());
            let callback = mapped.clone();
            encoder.map_buffer_on_submit(readback, wgpu::MapMode::Read, .., move |result| {
                let _ = callback.set(result);
            });
            let index = self.queue.submit([encoder.finish()]);
            wait(self, Some(index))?;
            self.check_fault()?;
            let Some(Ok(())) = mapped.get() else {
                return Err(self.fail("texture residency verification readback failed".into()));
            };
            words = read_words(readback);
        }
        Err(self.fail("texture residency verification did not converge".into()))
    }

    /// Feeds one completed frame's counters (per bucket: request codes, misses, survey
    /// codes, each in slot order) into the residency table. Returns the samples that
    /// needed a non-resident level.
    pub(crate) fn observe_residency(
        &mut self,
        words: &[u32],
        ids: &[Option<u64>],
        frame: u64,
    ) -> u64 {
        let mut misses = 0;
        if words.len() < BUCKETS * 3 * SLOTS {
            return 0;
        }
        let buckets = || words.chunks_exact(3 * SLOTS);
        for (slot, id) in ids.iter().enumerate() {
            let Some(id) = *id else { continue };
            let code = buckets().map(|b| b[slot]).max().unwrap();
            if code != 0 {
                self.residency.observe(id, frame, 16 - code.min(16));
            }
            misses += buckets().map(|b| u64::from(b[SLOTS + slot])).sum::<u64>();
            let survey = buckets().map(|b| b[2 * SLOTS + slot]).max().unwrap();
            if survey != 0
                && let Some(e) = self.residency.entries.get_mut(&id)
            {
                let level = 16 - survey.min(16);
                e.surveyed = Some(e.surveyed.map_or(level, |l| l.min(level)));
            }
        }
        misses
    }
}

/// A mapped counter readback's words, then unmapped for the next frame.
pub(crate) fn read_words(readback: &wgpu::Buffer) -> Vec<u32> {
    let words = readback
        .get_mapped_range(..)
        .map(|view| {
            view.chunks_exact(4)
                .map(|c| u32::from_ne_bytes(c.try_into().unwrap()))
                .collect()
        })
        .unwrap_or_default();
    readback.unmap();
    words
}

/// The GPU format of a texture role: base colour decodes sRGB when sampled.
pub(crate) fn texture_format(role: asset::TextureRole) -> wgpu::TextureFormat {
    match role {
        asset::TextureRole::BaseColorSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        asset::TextureRole::MetallicRoughnessLinear | asset::TextureRole::NormalLinear => {
            wgpu::TextureFormat::Rgba8Unorm
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residency_word_packs_top_and_exact_probe_scales() {
        // 4096 x 1024 against the 64 x 64 probe: scales 2^6 and 2^4.
        let w = word(3, [6, 4]);
        assert_eq!(w & STREAMED, STREAMED);
        assert_eq!(w & 15, 3);
        assert_eq!((w >> 4) & 31, 22);
        assert_eq!((w >> 9) & 31, 20);
        // Smaller than the probe: 2^-2, still nonzero and streamed.
        assert_eq!((word(0, [-2, -2]) >> 4) & 31, 14);
    }

    #[test]
    fn levels_stay_for_the_hold_window_and_grow_at_once() {
        let mut last = [0u64; MAX_TOP as usize + 1];
        // Never requested: only the always-resident tail.
        assert_eq!(desired_top(&last, 10, 120, 6), 6);
        last[2] = 10;
        assert_eq!(desired_top(&last, 11, 120, 6), 2);
        // A finer request at a later frame wins immediately.
        last[1] = 20;
        assert_eq!(desired_top(&last, 21, 120, 6), 1);
        // Level 1 expires after the hold; level 2 was not requested again either.
        assert_eq!(desired_top(&last, 141, 120, 6), 6);
        last[2] = 140;
        assert_eq!(desired_top(&last, 141, 120, 6), 2);
        // Clamped to the texture's own finest droppable level.
        assert_eq!(desired_top(&[0; 7], 5, 120, 3), 3);
    }

    #[test]
    fn transitions_write_missing_levels_and_copy_shared_ones() {
        // Growing from top 3 to 1 in a 13-level chain: write 1..3, copy 3..13.
        assert_eq!(transition_levels(3, 1, 13), (1..3, 3..13));
        // Shrinking from 0 to 2: nothing written, copy 2..13 (old level l, new level l-2).
        assert_eq!(transition_levels(0, 2, 13), (2..2, 2..13));
        assert_eq!(transition_levels(2, 2, 13), (2..2, 2..13));
    }

    #[test]
    fn only_power_of_two_extents_stream() {
        assert_eq!(max_top(4096, 4096, 13), Some(6));
        assert_eq!(max_top(4096, 16, 13), Some(4));
        assert_eq!(max_top(2, 2, 2), Some(1));
        assert_eq!(max_top(1, 1, 1), Some(0));
        assert_eq!(max_top(1000, 1024, 11), None);
    }

    #[test]
    fn camera_cuts_need_a_large_view_change() {
        let view = ViewKey {
            eye: Vec3::ZERO,
            forward: Vec3::NEG_Z,
            scale: [1.0, 1.5],
            size: [1920, 1080],
        };
        let at = |eye| ViewKey { eye, ..view };
        assert!(camera_cut(None, view));
        // The quarry bench's 1 mm steps and 1 cm pose change stay feedback frames.
        assert!(!camera_cut(Some(view), at(Vec3::new(0.01, 0.0, 0.0))));
        assert!(camera_cut(Some(view), at(Vec3::new(0.3, 0.0, 0.0))));
        let forward = Vec3::new(0.1f32.sin(), 0.0, -0.1f32.cos());
        assert!(camera_cut(Some(view), ViewKey { forward, ..view }));
        let size = [1280, 720];
        assert!(camera_cut(Some(view), ViewKey { size, ..view }));
        let scale = [1.1, 1.65];
        assert!(camera_cut(Some(view), ViewKey { scale, ..view }));
    }
}
