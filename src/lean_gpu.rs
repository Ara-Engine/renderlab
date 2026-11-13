//! Lean shading on wgpu (`program::lean_variant`, `program::FrameInputs`): a frame that
//! projects no decals and has no media shades with each program's lean module, which is
//! byte-identical in its picture and compiles the idle decal and atmosphere code out.
//!
//! The lean pipelines compile on a worker thread, one at a time, the first time a lean frame
//! shades with a program that lacks one. Until it lands, frames shade with the program's full
//! module, which is the same picture: a frame rendered while the lean module compiles is the
//! full module's frame, byte for byte. Streamed-texture frames and frames with decals or media
//! always shade full. `RENDERLAB_LEAN=0` keeps every frame full. A child of primary_ray_gpu.
use super::*;
use crate::program::{self, Stage};

/// The lean pipeline being compiled: its program (and the digests of that program's source
/// and of the shading head, so a reload or a visibility change while it compiles discards
/// it), tiles or list, `Shading` index, and the worker.
pub(super) struct LeanCompile {
    id: u64,
    digest: [u8; 32],
    head: [u8; 32],
    list: bool,
    shading: usize,
    worker: std::thread::JoinHandle<Option<Owned<wgpu::ComputePipeline>>>,
}

/// The view's lean shading state: the compile in flight, and the pipelines that failed to
/// compile (never retried; those frames shade full).
#[derive(Default)]
pub(super) struct LeanState {
    running: Option<LeanCompile>,
    failed: Vec<(u64, bool, usize)>,
}

impl Renderer {
    /// Whether this frame may shade lean: no decal projects, the atmosphere has no media, and
    /// it does not stream textures.
    pub(super) fn frame_lean(&self) -> bool {
        let Some(state) = self.view.primary.as_ref() else {
            return false;
        };
        let a = &self.atmosphere.settings;
        program::lean_on()
            && !state.requests
            && program::FrameInputs {
                decals: self.decal_stats().decals > 0,
                media: a.aerial_perspective || a.height_fog.is_some() || a.volumetric_fog.is_some(),
            }
            .lean()
    }

    /// Once a frame before shading: moves a finished lean compile into its program's
    /// pipelines (unless the program reloaded meanwhile) and, on a lean frame, starts the one
    /// compile a program of `ids` (the frame's programs; `list` when it shades by tile list)
    /// lacks. Never more than one compile runs.
    pub(in crate::renderer) fn prepare_lean(&mut self, ids: &[u64], list: bool, shading: usize) {
        let finished = self
            .view
            .primary
            .as_ref()
            .and_then(|s| s.lean.running.as_ref())
            .is_some_and(|c| c.worker.is_finished());
        if finished {
            let Some(done) = self
                .view
                .primary
                .as_mut()
                .and_then(|s| s.lean.running.take())
            else {
                return;
            };
            let key = (done.id, done.list, done.shading);
            let pipeline = done.worker.join().ok().flatten();
            let head = self
                .primary_sources
                .as_ref()
                .map(|(_, heads)| <[u8; 32]>::from(Sha256::digest(heads[done.shading].as_bytes())));
            let current = self
                .programs
                .iter()
                .position(|s| s.id == done.id && s.digest == done.digest)
                .filter(|_| head == Some(done.head));
            match (pipeline, current) {
                (Some(pipeline), Some(i)) => {
                    self.programs[i].pipelines.lean[usize::from(done.list)][done.shading] =
                        Some(pipeline);
                }
                (None, Some(_)) => {
                    if let Some(state) = self.view.primary.as_mut() {
                        state.lean.failed.push(key);
                    }
                }
                // The program reloaded or left: its new source gets its own compile.
                _ => {}
            }
        }
        if !self.frame_lean()
            || self
                .view
                .primary
                .as_ref()
                .is_none_or(|s| s.lean.running.is_some())
        {
            return;
        }
        let failed = |id: u64| {
            self.view
                .primary
                .as_ref()
                .is_some_and(|s| s.lean.failed.contains(&(id, list, shading)))
        };
        let Some(&id) = ids.iter().find(|&&id| {
            let slot = self.program_slot(id);
            slot.pipelines.lean[usize::from(list)][shading].is_none() && !failed(id)
        }) else {
            return;
        };
        let slot = self.program_slot(id);
        // The full pipeline this lean one stands in for: its layout, from its own groups.
        let full = if list {
            slot.pipelines.list[0][shading].as_ref()
        } else {
            slot.pipelines.traced[shading].as_ref()
        };
        let Some(full) = full else {
            return;
        };
        let groups: [_; 4] = std::array::from_fn(|group| full.get_bind_group_layout(group as u32));
        let layout = self.ledger.own(
            self.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("lean shading ABI"),
                    bind_group_layouts: &groups.each_ref().map(Some),
                    immediate_size: 0,
                }),
            ResourceKind::PipelineLayout,
        );
        let Some((_, heads)) = self.primary_sources.as_ref() else {
            return;
        };
        let head = heads[shading].clone();
        let source = slot.source.clone();
        let digest = slot.digest;
        let head_digest: [u8; 32] = Sha256::digest(head.as_bytes()).into();
        let device = self.device.clone();
        let ledger = self.ledger.clone();
        let cache = self.program_cache.get().cloned();
        let worker = std::thread::spawn(move || {
            let parameters = program::declares_parameters(&source);
            // Exactly the full module's composition (ensure_lists, compile_program), made lean.
            let composed = if list {
                program::compose_list(&head, &source, Stage::Traced, parameters)
            } else {
                program::compose(&head, &source, Stage::Traced, parameters)
            };
            let lean = program::lean_variant(program::with_reflection_history(composed));
            let scopes = OperationScopes::new(&device);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("lean shading"),
                source: wgpu::ShaderSource::Wgsl(lean.source.into()),
            });
            let pipeline = ledger.own(
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("lean program shading"),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(if list {
                        "primary_shade_list"
                    } else {
                        "primary_shade_tiles"
                    }),
                    compilation_options: Default::default(),
                    cache: cache.as_ref(),
                }),
                ResourceKind::ComputePipeline,
            );
            scopes.finish().is_none().then_some(pipeline)
        });
        if let Some(state) = self.view.primary.as_mut() {
            state.lean.running = Some(LeanCompile {
                id,
                digest,
                head: head_digest,
                list,
                shading,
                worker,
            });
        }
    }

    /// Program `id`'s lean pipeline (tiles or list, `Shading` index `shading`) when this
    /// frame shades lean and it has landed; the frame's full pipeline otherwise.
    pub(super) fn lean_pipeline(
        &self,
        id: u64,
        list: bool,
        shading: usize,
    ) -> Option<&wgpu::ComputePipeline> {
        if !self.frame_lean() {
            return None;
        }
        self.program_slot(id).pipelines.lean[usize::from(list)][shading].as_deref()
    }
}
