//! View modes on the traced and visibility-buffer paths (docs/design-id-output.md): a frame
//! in any mode but `Lit` runs as a lit frame up to shading, then shades with its programs'
//! view-mode modules (`program::view_variant`), which store the mode's value instead of
//! `shade`. Data modes also run without temporal reconstruction, transparency or tone
//! mapping: their HDR values reach the output unchanged. `LightingOnly` is lit (reconstructed
//! and tone-mapped). Lit frames never build or bind anything here.
use super::{
    Renderer,
    primary_ray_gpu::{SHADE_TILE, Shading},
};
use crate::{
    Error, ViewMode,
    observe::{Owned, ResourceKind},
    program::{self, Stage},
};

/// A view's mode and the modules it shades with.
pub(crate) struct ViewModes {
    /// The frame's mode; `Lit` on every ordinary frame.
    pub mode: ViewMode,
    pipelines: Vec<ViewPipeline>,
    present: Option<(wgpu::TextureFormat, Owned<wgpu::BindGroupLayout>, wgpu::RenderPipeline)>,
}
impl Default for ViewModes {
    fn default() -> Self {
        Self {
            mode: ViewMode::Lit,
            pipelines: Vec::new(),
            present: None,
        }
    }
}

/// One program's view-mode shading pipeline, keyed by everything it was composed from.
struct ViewPipeline {
    program: u64,
    source: String,
    head: String,
    list: bool,
    streamed: bool,
    shading: usize,
    mode: ViewMode,
    pipeline: Owned<wgpu::ComputePipeline>,
}

impl ViewModes {
    /// Shades with the mode's modules (every mode but `Lit` on a hit-record path).
    pub fn shades(&self) -> bool {
        self.mode != ViewMode::Lit
    }
    /// Data modes: no reconstruction, transparency, fog or tone mapping.
    pub fn data(&self) -> bool {
        self.mode != ViewMode::Lit && self.mode.is_data()
    }
}

impl Renderer {
    /// The mode the frame's passes see for a requested `mode`: hit-record frames in any
    /// mode run as lit frames (and remember the mode for shading); raster frames keep it.
    pub(super) fn frame_view_mode(&mut self, mode: ViewMode) -> ViewMode {
        if self.primary_mode == crate::PrimaryVisibility::Raster {
            self.outputs.modes.mode = ViewMode::Lit;
            return mode;
        }
        self.outputs.modes.mode = mode;
        ViewMode::Lit
    }

    /// The frame's shading in its view mode, in place of `encode_primary_shading`.
    pub(super) fn encode_view_shading(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        shading: Shading,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<(), Error> {
        let state = self.primary.as_ref().unwrap();
        let (list, streamed) = (state.mixed(), state.requests);
        let programs = if list {
            state.programs.clone()
        } else {
            vec![state.program]
        };
        let mode = self.outputs.modes.mode;
        let mut indices = Vec::with_capacity(programs.len());
        for &id in &programs {
            indices.push(self.view_pipeline(id, list, streamed, shading as usize, mode)?);
        }
        let pipelines = &self.outputs.modes.pipelines;
        if list {
            self.encode_program_lists_with(encoder, shading, timestamp_writes, |k| {
                &pipelines[indices[k]].pipeline
            });
        } else {
            let pipeline = &pipelines[indices[0]].pipeline;
            self.encode_primary_shade(encoder, pipeline, SHADE_TILE, shading, timestamp_writes);
        }
        Ok(())
    }

    /// The index of program `id`'s pipeline for these choices, composed and compiled the
    /// first time a frame needs it (and again after the program or the head changes).
    fn view_pipeline(
        &mut self,
        id: u64,
        list: bool,
        streamed: bool,
        shading: usize,
        mode: ViewMode,
    ) -> Result<usize, Error> {
        let head = &self.primary_sources.as_ref().unwrap().1[shading];
        let slot = self.program_slot(id);
        let found = self.outputs.modes.pipelines.iter().position(|p| {
            (p.program, p.list, p.streamed, p.shading, p.mode) == (id, list, streamed, shading, mode)
                && p.source == slot.source
                && p.head == *head
        });
        if let Some(index) = found {
            return Ok(index);
        }
        let stage = if streamed {
            Stage::TracedStreamed
        } else {
            Stage::Traced
        };
        let parameters = program::declares_parameters(&slot.source);
        let composed = if list {
            program::compose_list(head, &slot.source, stage, parameters)
        } else {
            program::compose(head, &slot.source, stage, parameters)
        };
        let source = program::view_variant(&composed.source, &slot.source, mode)?;
        let scopes = super::OperationScopes::new(&self.device);
        let descriptor = wgpu::ShaderModuleDescriptor {
            label: Some("view mode shading"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        };
        // User programs keep their runtime checks, as in every other composition.
        let module = if slot.source == crate::STANDARD_PROGRAM {
            self.device.create_engine_shader_module(descriptor)
        } else {
            self.device.create_shader_module(descriptor)
        };
        let primary = self.primary.as_ref().unwrap();
        let pipeline = match (list, streamed) {
            (true, _) => primary.list_pipeline(&self.device, &self.ledger, &module, shading, streamed, None),
            (false, true) => primary.streamed_pipeline(&self.device, &self.ledger, &module, shading, None),
            (false, false) => primary.shade_pipeline(&self.device, &self.ledger, &module, shading, None),
        };
        if let Some(error) = scopes.finish() {
            return Err(Error::Shader(format!(
                "view mode {mode:?} of program `{}`: {error}",
                slot.label
            )));
        }
        let entry = ViewPipeline {
            program: id,
            source: slot.source.clone(),
            head: head.clone(),
            list,
            streamed,
            shading,
            mode,
            pipeline,
        };
        // Pipelines of removed programs or old sources are never found again; drop them.
        let modes = &mut self.outputs.modes;
        modes.pipelines.retain(|p| {
            !((p.program, p.list, p.streamed, p.shading, p.mode) == (id, list, streamed, shading, mode))
        });
        modes.pipelines.push(entry);
        Ok(modes.pipelines.len() - 1)
    }

    /// A data-mode frame's present: the HDR values copied to `view` as they are.
    pub(super) fn encode_view_present(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        let format = self.format;
        if self.outputs.modes.present.as_ref().is_none_or(|p| p.0 != format) {
            self.outputs.modes.present = Some(self.view_present(format));
        }
        let (_, layout, pipeline) = self.outputs.modes.present.as_ref().unwrap();
        let hdr = &self.lighting.hdr.as_ref().unwrap().resolved;
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view mode present"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&hdr.view),
            }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("view mode present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn view_present(
        &self,
        format: wgpu::TextureFormat,
    ) -> (wgpu::TextureFormat, Owned<wgpu::BindGroupLayout>, wgpu::RenderPipeline) {
        let layout = self.ledger.own(
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("view mode present"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    }],
                }),
            ResourceKind::BindGroupLayout,
        );
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("view mode present"),
                bind_group_layouts: &[Some(&*layout)],
                immediate_size: 0,
            });
        let module = self
            .device
            .create_engine_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("view mode present"),
                source: wgpu::ShaderSource::Wgsl(PRESENT.into()),
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("view mode present"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("view_present_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("view_present_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        (format, layout, pipeline)
    }
}

/// The data-mode present: each output pixel's HDR value, unchanged (an sRGB output encodes it).
pub(crate) const PRESENT: &str = "@group(0) @binding(0) var view_hdr:texture_2d<f32>;
@vertex fn view_present_vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let p=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
    return vec4<f32>(p[i],0.0,1.0);
}
@fragment fn view_present_fs(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32> {
    return vec4<f32>(textureLoad(view_hdr,vec2<i32>(p.xy),0).rgb,1.0);
}
";
