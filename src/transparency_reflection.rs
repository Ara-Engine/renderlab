//! TP2 adapter to R1: rasterize the glass lobe, then trace it with the opaque
//! frame's geometry/material tables and the opaque program's hit shading.
use super::*;

pub(super) struct GlassReflections {
    pub position: Depth,
    pub normal: Depth,
    pub geometric: Depth,
    pub radiance: Depth,
    pub program_slots: Owned<wgpu::Buffer>,
    pub layout: Owned<wgpu::BindGroupLayout>,
    pub pipeline: Owned<wgpu::ComputePipeline>,
    pub key: [u8; 32],
}
// P-SHC: the glass hit shader depends on the first opaque program. Compile that
// dependent pipeline in the same reload transaction, retaining its existing targets.
pub(crate) struct ReflectionReload {
    pub(crate) source: String,
    pub(crate) layout: Owned<wgpu::PipelineLayout>,
    pub(crate) key: [u8; 32],
}
pub(super) fn key(head: &str, program: &str, parameters: bool) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(head.as_bytes());
    hash.update(program.as_bytes());
    hash.update([u8::from(parameters)]);
    hash.finalize().into()
}
pub(super) fn source(head: &str, program: &str, parameters: bool) -> String {
    let program = crate::program::glass_hit_program(program);
    let mut s = crate::program::compose(head, &program, crate::program::Stage::Traced, parameters)
        .source
        .replace("\r\n", "\n");
    // Only the hit reconstruction helpers from the generated camera entry are needed.
    s.truncate(s.find("// primary_frame.scene widened").unwrap());
    s = s
        .replace(
            "fn rl_reflection_prepare(s:Surface,",
            "fn rl_reflection_prepare(lobe:SpecularLobe,",
        )
        .replace("    let lobe=reflection_lobe(s,input);\n", "")
        // MP1 integration: a foreign program's parameter block has another layout.
        // Match MP1's response fallback, with slot zero's own parameter block.
        .replace(
            "rl_load_parameters(h.batch);",
            "rl_load_parameters(rl_glass_parameter_batch(h.batch));",
        )
        // Glass draws after TR1: fixed glossy strata avoid unfiltered frame rotation.
        .replace("let frame=primary_frame.sided.y;", "let frame=0u;");
    s.push_str(r#"
@group(0) @binding(31) var tp_reflection_position:texture_2d<f32>;
@group(0) @binding(32) var tp_reflection_normal:texture_2d<f32>;
@group(0) @binding(33) var tp_reflection_geometric:texture_2d<f32>;
@group(0) @binding(38) var<uniform> rl_glass_programs:array<vec4<u32>,PRIMARY_BATCHES/4>;
fn rl_glass_parameter_batch(batch:u32)->u32 {
    return select(0u,batch,rl_glass_programs[batch>>2u][batch&3u]==0u);
}
@compute @workgroup_size(8,8) fn tp_reflect(@builtin(global_invocation_id) id:vec3<u32>) {
    let size=textureDimensions(primary_hdr);
    if any(id.xy>=size) {return;}
    let p=textureLoad(tp_reflection_position,vec2<i32>(id.xy),0);
    if p.w==0.0 {textureStore(primary_hdr,id.xy,vec4<f32>(0.0));return;}
    let lobe=textureLoad(tp_reflection_normal,vec2<i32>(id.xy),0);
    let n=unit_normal(lobe.xyz);
    let geometric=unit_normal(textureLoad(tp_reflection_geometric,vec2<i32>(id.xy),0).xyz);
    let receiver=Receiver(p.xyz,geometric,vec2<i32>(id.xy),0u,p.w,true);
    let input=SurfaceInput(p.xyz,vec3<f32>(0.0),vec3<f32>(0.0),geometric,n,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),0.0,
        vec2<f32>(0.0),vec2<f32>(0.0),vec2<f32>(0.0),vec4<f32>(1.0),true,lighting.eye,id.xy,primary_materials[0].value,receiver,decal_none());
    rl_indirect_open();
    rl_reflection_prepare(SpecularLobe(n,lobe.w,1.0),input,0u,id.xy,size);
    textureStore(primary_hdr,id.xy,rl_specular);
}
"#);
    s
}
impl Renderer {
    pub(crate) fn tp_reload_pipeline(&self, program: u64) -> Option<&wgpu::ComputePipeline> {
        self.primary.as_ref().filter(|p| p.program == program)?;
        self.transparency.reflections.as_ref().map(|r| &*r.pipeline)
    }

    pub(crate) fn tp_reload_recipe(
        &self,
        program: u64,
        text: &str,
    ) -> Result<Option<ReflectionReload>, Error> {
        let Some(pipeline) = self.tp_reload_pipeline(program) else {
            return Ok(None);
        };
        let groups: [_; 4] = std::array::from_fn(|i| pipeline.get_bind_group_layout(i as u32));
        let parameters = crate::program::declares_parameters(text);
        let head = primary_ray_gpu::transparent_reflection_head(&self.source)?;
        Ok(Some(ReflectionReload {
            source: source(&head, text, parameters),
            layout: self.tp_pipeline_layout("background glass reflection ABI", &groups.each_ref()),
            key: key(&self.source, text, parameters),
        }))
    }

    pub(crate) fn tp_install_reload(
        &mut self,
        previous: &wgpu::ComputePipeline,
        key: [u8; 32],
        pipeline: Owned<wgpu::ComputePipeline>,
    ) {
        if let Some(reflections) = self
            .transparency
            .reflections
            .as_mut()
            .filter(|r| *r.pipeline == *previous)
        {
            reflections.pipeline = pipeline;
            reflections.key = key;
        }
    }

    pub(super) fn tp_reflections(&self, size: wgpu::Extent3d) -> Result<GlassReflections, Error> {
        let limits = self.device.limits();
        if limits.max_color_attachments < 3 || limits.max_color_attachment_bytes_per_sample < 32 {
            return Err(Error::InvalidInput(
                "glass reflections require three colour attachments and 32 bytes per sample",
            ));
        }
        let primary = self.primary.as_ref().unwrap();
        let program = self.program_slot(primary.program);
        let head = primary_ray_gpu::transparent_reflection_head(&self.source)?;
        let source = source(&head, &program.source, program.layout.is_some());
        let key = key(&self.source, &program.source, program.layout.is_some());
        let program_slots = self.ledger.own(
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("TP2 reflected hit program slots"),
                size: 4 * u64::from(crate::TRACED_DRAWS),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            ResourceKind::Buffer,
        );
        let position = self.tp_target(
            "TP2 glass world position",
            size,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            16,
        );
        let normal = self.tp_target(
            "TP2 glass reflection lobe",
            size,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            8,
        );
        let geometric = self.tp_target(
            "TP2 glass geometric plane",
            size,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            8,
        );
        let radiance = self.tp_target(
            "TP2 glass reflection radiance",
            size,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            8,
        );
        let layout = self.tp_layout(
            "TP2 glass reflection",
            &[
                entry(0, COMPUTE, uniform(false, 80)),
                entry(
                    2,
                    COMPUTE,
                    wgpu::BindingType::AccelerationStructure {
                        vertex_return: false,
                    },
                ),
                entry(10, COMPUTE, uniform(false, primary.frame_buffer().size())),
                entry(
                    18,
                    COMPUTE,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(31, COMPUTE, texture(UNFILTERED)),
                entry(32, COMPUTE, texture(UNFILTERED)),
                entry(33, COMPUTE, texture(UNFILTERED)),
                entry(
                    38,
                    COMPUTE,
                    uniform(false, 4 * u64::from(crate::TRACED_DRAWS)),
                ),
            ],
        );
        let pipeline_layout = self.tp_pipeline_layout(
            "TP2 glass reflection",
            &[
                &layout,
                primary.array_layout(),
                primary.material_layout(),
                &self.lighting.bindings,
            ],
        );
        // Hits run program text: checked, as every program module (P-SPLIT).
        let module = self.ledger.own(
            self.device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("TP2 glass reflection"),
                    source: wgpu::ShaderSource::Wgsl(source.as_str().into()),
                }),
            ResourceKind::ShaderModule,
        );
        let pipeline = self.tp_compute(
            "TP2 glass reflection",
            &pipeline_layout,
            &module,
            "tp_reflect",
        );
        Ok(GlassReflections {
            position,
            normal,
            geometric,
            radiance,
            program_slots,
            layout,
            pipeline,
            key,
        })
    }
}
