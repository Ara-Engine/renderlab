//! TP2 adapter to R1: rasterize the glass lobe, then trace it with the opaque
//! frame's geometry/material tables and the opaque program's hit shading.
use super::*;

pub(in crate::renderer) struct GlassReflections {
    pub position: Depth,
    pub normal: Depth,
    pub geometric: Depth,
    pub radiance: Depth,
    pub program_slots: Owned<wgpu::Buffer>,
    pub layout: Arc<Owned<wgpu::BindGroupLayout>>,
    pub pipeline: Arc<Owned<wgpu::ComputePipeline>>,
    pub key: [u8; 32],
    pub sun_map: bool,
}
/// Pipeline state is independent of a view's lobe target size.
#[derive(Clone)]
pub(super) struct ReflectionPipeline {
    pub program: u64,
    pub key: [u8; 32],
    pub sun_map: bool,
    layout: Arc<Owned<wgpu::BindGroupLayout>>,
    pipeline: Arc<Owned<wgpu::ComputePipeline>>,
}
// P-SHC: the glass hit shader depends on the first opaque program. Compile that
// dependent pipeline in the same reload transaction, retaining its existing targets.
pub(crate) struct ReflectionReload {
    pub(crate) source: String,
    pub(crate) layout: Owned<wgpu::PipelineLayout>,
    pub(crate) key: [u8; 32],
}
/// The identity of the reflection pipeline built from a prelude and a program, from their
/// SHA-256 digests (kept with the sources: this runs every frame).
pub(super) fn key(
    head: &[u8; 32],
    program: &[u8; 32],
    parameters: bool,
    rt: bool,
    sun_map: bool,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(head);
    hash.update(program);
    hash.update([u8::from(parameters), u8::from(rt), u8::from(sun_map)]);
    hash.finalize().into()
}
/// The `tp_reflect` kernel's module: `program`'s traced hit shading composed on `head`
/// (`transparent_reflection_head`), with the glass lobe read from the lobe pass's targets
/// instead of a camera hit. The wgpu renderer and the ara-gpu backend both compile it.
pub(crate) fn source(head: &str, program: &str, parameters: bool) -> String {
    let deferred = crate::program::rl_deferred_for(head);
    let program = crate::program::glass_hit_program(program, deferred);
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
        .replace("let frame=primary_frame.sided.y;", "let frame=0u;")
        // Smooth architectural window glass (0.05) and wine bottles (0.06) trace one clean mirror
        // ray instead of unfiltered static multi-stratum glossy noise after TR1.
        .replace("let mirror=rough<RL_MIRROR;", "let mirror=rough<=0.065;")
        // Curved glass silhouettes (e.g. wine bottles) can reflect slightly below the triangle
        // geometric plane; lift the ray just above the horizon so it still traces real environment
        // / scene geometry rather than leaving valid==0 (which fell back to bright unoccluded sky).
        .replace(
            "if dot(d,side)<=0.0 {continue;}",
            "if dot(d,side)<=0.0 { d=unit_normal(d+side*(0.02-dot(d,side))); }",
        )
        // Glass Fresnel reflections (~4% dielectric at normal incidence) use a single center
        // shadow ray per shadowed local light instead of 4 soft-disk rays per hit, avoiding
        // 12+ shadow rays per pixel inside multi-light glass corridors.
        .replace(
            "let samples=select(1u,max(rl_local_rays,4u),radius>0.0);",
            "let samples=1u;",
        );
    let software = head.contains(crate::program::SOFTWARE_REFLECTION_GEOMETRY);
    // Non-RT glass reflections run inline in software BVH: trace 1 ray per 2x1 checkerboard
    // pixel pair (odd pixels reconstructed from their 4 traced cardinal neighbours in TP_GLASS,
    // keeping full H+V resolution on curved wine bottles and windows with 2x fewer rays) and
    // allow settled world radiance-cache hits for distant cones.
    if software {
        s = s
            .replace(
                "var count=select(clamp(u32(ceil(RL_RAYS_PER_ROUGHNESS*rough)),RL_GLOSSY_RAYS,RL_GLOSSY_MAX),1u,mirror);",
                "var count=1u;",
            )
            .replace(
                "if rough<RL_CACHE_ROUGH { return vec4<f32>(0.0); }",
                "if rough<0.0 { return vec4<f32>(0.0); }",
            );
    }
    s.push_str(r#"
@group(0) @binding(31) var tp_reflection_position:texture_2d<f32>;
@group(0) @binding(32) var tp_reflection_normal:texture_2d<f32>;
@group(0) @binding(33) var tp_reflection_geometric:texture_2d<f32>;
@group(0) @binding(38) var<uniform> rl_glass_programs:array<vec4<u32>,PRIMARY_BATCHES/4>;
// The dispatch's pixel rectangle [x0, y0, x1, y1): the glass run's conservative screen bounds
// (transparency_bounds.rs), or the whole view. Outside it no lobe of this run was drawn.
@group(0) @binding(39) var<uniform> tp_reflect_rect:vec4<u32>;
fn rl_glass_parameter_batch(batch:u32)->u32 {
    return select(0u,batch,rl_glass_programs[batch>>2u][batch&3u]==0u);
}
"#);
    if software {
        s.push_str(r#"
@compute @workgroup_size(8,8) fn tp_reflect(@builtin(global_invocation_id) grid:vec3<u32>) {
    let x0=(tp_reflect_rect.x&0xfffffffeu)+grid.x*2u;
    let y=tp_reflect_rect.y+grid.y;
    let size=textureDimensions(primary_hdr);
    if x0>=size.x || y>=size.y || x0>=tp_reflect_rect.z || y>=tp_reflect_rect.w {return;}
    let parity=y&1u;
    let p0_xy=vec2<u32>(x0+parity,y);
    let p1_xy=vec2<u32>(x0+(parity^1u),y);
    if p0_xy.x<size.x { textureStore(primary_hdr,p0_xy,vec4<f32>(0.0)); }
    if p1_xy.x<size.x { textureStore(primary_hdr,p1_xy,vec4<f32>(0.0)); }
    var p=vec4<f32>(0.0);
    var sample_xy=p0_xy;
    if p0_xy.x<size.x && p0_xy.x<tp_reflect_rect.z {
        p=textureLoad(tp_reflection_position,vec2<i32>(p0_xy),0);
    }
    if p.w==0.0 && p1_xy.x<size.x && p1_xy.x<tp_reflect_rect.z {
        let alt=textureLoad(tp_reflection_position,vec2<i32>(p1_xy),0);
        if alt.w!=0.0 {
            p=alt;
            sample_xy=p1_xy;
        }
    }
    if p.w==0.0 {return;}
    let lobe=textureLoad(tp_reflection_normal,vec2<i32>(sample_xy),0);
    let v=unit_normal(lighting.eye-p.xyz);
    var n=unit_normal(lobe.xyz);
    if dot(n,v)<0.02 { n=unit_normal(n+v*(0.05-dot(n,v))); }
    let geometric=unit_normal(textureLoad(tp_reflection_geometric,vec2<i32>(sample_xy),0).xyz);
    let receiver=Receiver(p.xyz,geometric,vec2<i32>(sample_xy),0u,p.w,true);
    let input=SurfaceInput(p.xyz,vec3<f32>(0.0),vec3<f32>(0.0),geometric,n,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),0.0,
        vec2<f32>(0.0),vec2<f32>(0.0),vec2<f32>(0.0),vec4<f32>(1.0),true,lighting.eye,sample_xy,primary_materials[0].value,receiver,decal_none());
    rl_indirect_open();
    rl_reflection_prepare(SpecularLobe(n,lobe.w,1.0),input,0u,sample_xy,size);
    textureStore(primary_hdr,sample_xy,rl_specular);
}
"#);
    } else {
        s.push_str(r#"
@compute @workgroup_size(8,8) fn tp_reflect(@builtin(global_invocation_id) grid:vec3<u32>) {
    let id=vec3<u32>(grid.xy+tp_reflect_rect.xy,0u);
    let size=textureDimensions(primary_hdr);
    if any(id.xy>=size) || any(id.xy>=tp_reflect_rect.zw) {return;}
    let p=textureLoad(tp_reflection_position,vec2<i32>(id.xy),0);
    if p.w==0.0 {textureStore(primary_hdr,id.xy,vec4<f32>(0.0));return;}
    let lobe=textureLoad(tp_reflection_normal,vec2<i32>(id.xy),0);
    let v=unit_normal(lighting.eye-p.xyz);
    var n=unit_normal(lobe.xyz);
    if dot(n,v)<0.02 { n=unit_normal(n+v*(0.05-dot(n,v))); }
    let geometric=unit_normal(textureLoad(tp_reflection_geometric,vec2<i32>(id.xy),0).xyz);
    let receiver=Receiver(p.xyz,geometric,vec2<i32>(id.xy),0u,p.w,true);
    let input=SurfaceInput(p.xyz,vec3<f32>(0.0),vec3<f32>(0.0),geometric,n,vec3<f32>(0.0),vec3<f32>(0.0),vec3<f32>(0.0),0.0,
        vec2<f32>(0.0),vec2<f32>(0.0),vec2<f32>(0.0),vec4<f32>(1.0),true,lighting.eye,id.xy,primary_materials[0].value,receiver,decal_none());
    rl_indirect_open();
    rl_reflection_prepare(SpecularLobe(n,lobe.w,1.0),input,0u,id.xy,size);
    textureStore(primary_hdr,id.xy,rl_specular);
}
"#);
    }
    s
}
impl Renderer {
    pub(crate) fn tp_reload_pipeline(&self, program: u64) -> Option<&wgpu::ComputePipeline> {
        self.view
            .primary
            .as_ref()
            .filter(|p| p.program == program)?;
        self.view
            .transparency_reflections
            .as_ref()
            .map(|r| &**r.pipeline)
    }

    pub(crate) fn tp_reload_recipe(
        &self,
        program: u64,
        text: &str,
    ) -> Result<Option<ReflectionReload>, Error> {
        let Some(pipeline) = self.tp_reload_pipeline(program) else {
            return Ok(None);
        };
        let sun_map = self
            .view
            .transparency_reflections
            .as_ref()
            .is_some_and(|r| r.sun_map);
        let groups: [_; 4] = std::array::from_fn(|i| pipeline.get_bind_group_layout(i as u32));
        let parameters = crate::program::declares_parameters(text);
        let rt = self.hardware_shadows.is_some();
        let head = if rt {
            primary_ray_gpu::transparent_reflection_head(&self.source)?
        } else {
            primary_ray_gpu::transparent_reflection_head_non_rt(&self.source)?
        };
        let mut src = source(&head, text, parameters);
        let sun_group = if !rt && sun_map {
            if let Some(with_sun) =
                primary_ray_gpu::reflection_hits_gpu::hit_sun::compose(src.clone())
            {
                src = with_sun;
                Some(pipeline.get_bind_group_layout(
                    primary_ray_gpu::reflection_hits_gpu::hit_sun::GROUP,
                ))
            } else {
                None
            }
        } else {
            None
        };
        let layout = if let Some(sun) = sun_group.as_ref() {
            self.tp_pipeline_layout(
                "background glass reflection ABI",
                &[&groups[0], &groups[1], &groups[2], &groups[3], sun],
            )
        } else {
            self.tp_pipeline_layout("background glass reflection ABI", &groups.each_ref())
        };
        Ok(Some(ReflectionReload {
            source: src,
            layout,
            key: key(
                &self.source_digest,
                &Sha256::digest(text.as_bytes()).into(),
                parameters,
                rt,
                sun_group.is_some(),
            ),
        }))
    }

    pub(crate) fn tp_install_reload(
        &mut self,
        previous: &wgpu::ComputePipeline,
        key: [u8; 32],
        pipeline: Owned<wgpu::ComputePipeline>,
    ) {
        if let Some(reflections) = self
            .view
            .transparency_reflections
            .as_mut()
            .filter(|r| **r.pipeline == *previous)
        {
            reflections.pipeline = Arc::new(pipeline);
            reflections.key = key;
        }
    }

    pub(super) fn tp_reflections(
        &mut self,
        size: wgpu::Extent3d,
        rt: bool,
        sun_map: bool,
    ) -> Result<GlassReflections, Error> {
        let limits = self.device.limits();
        if limits.max_color_attachments < 3 || limits.max_color_attachment_bytes_per_sample < 32 {
            return Err(Error::InvalidInput(
                "glass reflections require three colour attachments and 32 bytes per sample",
            ));
        }
        let primary_program = self.view.primary.as_ref().unwrap().program;
        let program = self.program_slot(primary_program);
        let key = key(
            &self.source_digest,
            &program.digest,
            program.layout.is_some(),
            rt,
            sun_map,
        );
        // The pipeline for this primary program, built once and kept: a frame whose first
        // opaque draw changes program switches pipelines without compiling (Glint rebuilt it
        // 16 times in 30 s of play when only the frame's targets kept it).
        let prepared = match self.transparency.reflection_pipelines.iter().find(|p| p.key == key) {
            Some(prepared) => prepared.clone(),
            None => {
                let built = self.tp_reflection_pipeline(primary_program, rt, sun_map)?;
                self.transparency.reflection_pipelines.retain(|p| p.program != primary_program);
                self.transparency.reflection_pipelines.push(built.clone());
                built
            }
        };
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
        Ok(GlassReflections {
            position,
            normal,
            geometric,
            radiance,
            program_slots,
            layout: prepared.layout,
            pipeline: prepared.pipeline,
            key,
            sun_map: prepared.sun_map,
        })
    }
    pub(super) fn tp_reflection_pipeline(
        &self,
        program_id: u64,
        rt: bool,
        sun_map: bool,
    ) -> Result<ReflectionPipeline, Error> {
        let primary = self.view.primary.as_ref().unwrap();
        let program = self.program_slot(program_id);
        let head = if rt {
            primary_ray_gpu::transparent_reflection_head(&self.source)?
        } else {
            primary_ray_gpu::transparent_reflection_head_non_rt(&self.source)?
        };
        let mut source = source(&head, &program.source, program.layout.is_some());
        let mut used_sun_map = false;
        if !rt
            && sun_map
            && let Some(with_sun) =
                primary_ray_gpu::reflection_hits_gpu::hit_sun::compose(source.clone())
        {
            source = with_sun;
            used_sun_map = true;
        }
        let key = key(
            &self.source_digest,
            &program.digest,
            program.layout.is_some(),
            rt,
            sun_map,
        );
        let scene_entry = if rt {
            entry(
                2,
                COMPUTE,
                wgpu::BindingType::AccelerationStructure {
                    vertex_return: false,
                },
            )
        } else {
            entry(
                primary_ray_gpu::SOFTWARE_REFLECTIONS,
                COMPUTE,
                wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
            )
        };
        let layout = self.tp_layout(
            "TP2 glass reflection",
            &[
                entry(0, COMPUTE, uniform(false, 80)),
                scene_entry,
                entry(10, COMPUTE, uniform(false, primary_ray_gpu::FRAME_BYTES as u64)),
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
                entry(39, COMPUTE, uniform(true, 16)),
            ],
        );
        let sun_layout = used_sun_map
            .then(|| primary_ray_gpu::reflection_hits_gpu::hit_sun::layout(&self.device));
        let pipeline_layout = if let Some(sun) = sun_layout.as_ref() {
            self.tp_pipeline_layout(
                "TP2 glass reflection",
                &[
                    &layout,
                    primary.array_layout(),
                    primary.material_layout(),
                    &self.lighting.bindings,
                    sun,
                ],
            )
        } else {
            self.tp_pipeline_layout(
                "TP2 glass reflection",
                &[
                    &layout,
                    primary.array_layout(),
                    primary.material_layout(),
                    &self.lighting.bindings,
                ],
            )
        };
        // Hits run program text: checked, as every program module (P-SPLIT).
        let module = self.ledger.own(
            self.device
                .create_program_module(wgpu::ShaderModuleDescriptor {
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
        Ok(ReflectionPipeline {
            program: program_id,
            key,
            sun_map: used_sun_map,
            layout: Arc::new(layout),
            pipeline: Arc::new(pipeline),
        })
    }

}
