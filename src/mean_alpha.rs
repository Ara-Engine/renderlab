//! A see-through material's mean texture alpha (docs/design-see-through-occlusion.md).
//!
//! Rays that carry diffuse light (the indirect cache's, the final gather's probes) pass a
//! see-through surface by `1 - opacity * base alpha * mean texture alpha`. The mean is the base
//! texture's smallest level. Base textures are uploaded as texels, as block-compressed levels or
//! streamed, so the CPU cannot read them in general. This module samples that level once on the
//! GPU when a see-through material is created and reads the result back asynchronously. Until
//! it arrives (a frame or two), `Material::see_through_coverage` uses the material factors alone.
use crate::observe::{Ledger, Owned, ResourceKind};
use std::sync::{Arc, OnceLock};

const SHADER: &str = "
@group(0) @binding(0) var base:texture_2d<f32>;
@group(0) @binding(1) var linear:sampler;
@group(0) @binding(2) var<storage,read_write> mean:array<f32,4>;
// The smallest level's alpha, over a 2x2 grid of its texture coordinates (a 2x1 or 1x2 tail
// is covered too).
@compute @workgroup_size(1) fn mean_alpha() {
    let level=f32(textureNumLevels(base)-1u);
    var a=0.0;
    for(var y=0u;y<2u;y+=1u) {
        for(var x=0u;x<2u;x+=1u) {
            a+=textureSampleLevel(base,linear,(vec2<f32>(f32(x),f32(y))+0.5)*0.5,level).a;
        }
    }
    mean[0]=a*0.25;
}
";

/// One probe in flight: the readback, whether its map finished, and where its value goes.
struct Pending {
    readback: wgpu::Buffer,
    _result: wgpu::Buffer,
    mapped: Arc<OnceLock<bool>>,
    target: Arc<OnceLock<f32>>,
}

/// The probe's pipeline and the probes in flight.
pub(crate) struct MeanAlpha {
    pipeline: Owned<wgpu::ComputePipeline>,
    sampler: Owned<wgpu::Sampler>,
    pending: Vec<Pending>,
}

impl MeanAlpha {
    pub(crate) fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("see-through mean alpha"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = ledger.own(
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("see-through mean alpha"),
                layout: None,
                module: &module,
                entry_point: Some("mean_alpha"),
                compilation_options: Default::default(),
                cache: None,
            }),
            ResourceKind::ComputePipeline,
        );
        let sampler = ledger.own(
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("see-through mean alpha"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            ResourceKind::Sampler,
        );
        Self { pipeline, sampler, pending: Vec::new() }
    }

    /// Samples `view`'s smallest level on the GPU; `target` receives the mean alpha once the
    /// readback maps (`collect`).
    pub(crate) fn probe(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &wgpu::TextureView,
        target: Arc<OnceLock<f32>>,
    ) {
        let result = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("see-through mean alpha"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("see-through mean alpha readback"),
            size: 16,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("see-through mean alpha"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: result.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("see-through mean alpha"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("see-through mean alpha"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result, 0, &readback, 0, 16);
        queue.submit([encoder.finish()]);
        let mapped = Arc::new(OnceLock::new());
        let done = mapped.clone();
        readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = done.set(r.is_ok());
        });
        self.pending.push(Pending { readback, _result: result, mapped, target });
    }

    /// Hands every finished probe's value to its material (after the renderer's device poll).
    pub(crate) fn collect(&mut self) {
        self.pending.retain(|p| {
            let Some(&ok) = p.mapped.get() else { return true };
            if ok {
                let alpha = p
                    .readback
                    .slice(..)
                    .get_mapped_range()
                    .map(|v| f32::from_le_bytes(v[..4].try_into().unwrap()))
                    .unwrap_or(1.0);
                p.readback.unmap();
                let _ = p.target.set(if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 });
            }
            false
        });
    }
}
