//! Per-pixel pane shading on the wgpu renderer (the default; `RENDERLAB_PANES=raster` keeps the
//! glass raster), the design the ara-gpu port ships (RL#48): instead of rasterizing a glass
//! draw's triangles with the full lit fragment shader, which runs once per covered quad of
//! every sub-pixel triangle, each pane instance gets a four-sample raster of its triangle
//! ids, then one compute dispatch over its screen rectangle shades each distinct triangle
//! under a pixel's samples once at the pixel centre and averages the samples against the
//! colour behind, as the glass target's resolve does. The shading body is `fs_transparent`'s
//! own, rewritten with explicit derivatives (src/pane_pixels.rs), so it follows the shared
//! entry.
//!
//! Instances of one draw shade one at a time, in the order the raster draws them, each over
//! what the previous left in the HDR target.
use super::*;
use glam::Vec2;

/// Whether panes shade per pixel: yes unless `RENDERLAB_PANES=raster`, as on the ara-gpu port
/// (`RENDERLAB_ARA_PANES`). Traced frames on a device with `Features::PRIMITIVE_INDEX` only;
/// the raster draws them otherwise. On Ara's Glint start view the port's glass rails cost
/// 9.6 ms through the raster and 1.3 ms per pixel (M5).
pub(super) fn wanted() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("RENDERLAB_PANES").map_or(true, |v| v != "raster"))
}

/// Instances above which a glass draw keeps the raster: per-pixel panes encode an id pass, a
/// dispatch and a region copy per instance, so a fence or foliage draw of hundreds would cost
/// what RL#72's unbounded spans did (Ara Outpost RT, 2026-11-13). Window panes stay well under.
pub(super) const MAX_INSTANCES: usize = 64;

/// One pane record (shaders/pane_pixels.wgsl `PanePixels`): the instance's pixel rectangle
/// and its model slot, at a dynamic offset of this stride.
pub(super) const RECORD: u64 = 256;
pub(super) const RECORD_BYTES: u64 = 32;

/// A program's pane pipelines: the id raster by tangent stream, and the shading.
pub(super) struct PanePipelines {
    pub ids: [Owned<wgpu::RenderPipeline>; 2],
    pub shade: Owned<wgpu::ComputePipeline>,
}

/// The pane id target's format: wgpu guarantees four-sample `Rg16Uint` on every adapter but
/// not `R32Uint` (the port's), so ids are stored as two u16 halves (`ID_SEAMS`).
pub(super) const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Uint;
/// shaders/pane_pixels.wgsl's id encoding and its wgpu replacement.
const ID_SEAMS: [(&str, &str); 3] = [
    ("alias PaneId=u32;\n", "alias PaneId=vec2<u32>;\n"),
    (
        "fn tp_pane_id_out(id:u32)->PaneId { return id; }\n",
        "fn tp_pane_id_out(id:u32)->PaneId { return vec2<u32>(id&0xffffu,id>>16u); }\n",
    ),
    (
        "fn tp_pane_id(pixel:vec2<i32>,k:i32)->u32 { return textureLoad(pane_ids,pixel,k).x; }\n",
        "fn tp_pane_id(pixel:vec2<i32>,k:i32)->u32 { let t=textureLoad(pane_ids,pixel,k); return t.x|(t.y<<16u); }\n",
    ),
];
/// `section` with the wgpu id encoding.
fn wgpu_section(name: &str) -> Result<String, String> {
    let mut text = crate::pane_pixels::section(name)?.to_owned();
    for (from, to) in ID_SEAMS {
        text = text.replace(from, to);
    }
    Ok(text)
}

/// The pane id raster's and the pane shading's module sources, from a program's glass
/// module (`glass_source`).
pub(super) fn sources(glass: &str) -> Result<(String, String), String> {
    let ids = format!(
        "enable primitive_index;\n{glass}\n{}\n",
        wgpu_section("PANE_RASTER")?
    );
    let explicit = crate::pane_pixels::explicit_sampling(glass)?;
    let shade = format!(
        "{explicit}\n{}\n{}\n{}\n",
        crate::pane_pixels::fragment(&explicit)?,
        crate::pane_pixels::section("PANE_COMMON")?,
        wgpu_section("PANE_COMPUTE")?
    );
    Ok((ids, shade))
}

/// The pane group 0 layout entries: the glass group's, visible to compute too, then the
/// pane's vertices (70), indices (71), id raster (72), output (76) and record (77).
pub(super) fn layout_entries(
    glass: Vec<wgpu::BindGroupLayoutEntry>,
) -> Vec<wgpu::BindGroupLayoutEntry> {
    let compute = wgpu::ShaderStages::COMPUTE;
    let storage = |binding: u32, dynamic: bool| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: compute,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    };
    let mut entries: Vec<_> = glass
        .into_iter()
        .map(|mut e| {
            e.visibility |= compute;
            e
        })
        .collect();
    entries.extend([
        storage(70, false),
        storage(71, false),
        wgpu::BindGroupLayoutEntry {
            binding: 72,
            visibility: compute,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: true,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 76,
            visibility: compute,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: wgpu::TextureFormat::Rgba16Float,
                view_dimension: wgpu::TextureViewDimension::D2,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 77,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(RECORD_BYTES),
            },
            count: None,
        },
    ]);
    entries
}

/// The pixel rectangle [x0, y0, x1, y1) of `bounds` placed by `model` under `matrix` (y
/// down), widened by a pixel and clamped to `size`; the whole view when a corner is at or
/// behind the eye.
pub(super) fn rect(
    bounds: &crate::visibility::Aabb,
    model: Mat4,
    matrix: Mat4,
    size: [u32; 2],
) -> [u32; 4] {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let mut lo = Vec2::splat(f32::INFINITY);
    let mut hi = Vec2::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let p = Vec3::new(
            if corner & 1 == 0 {
                bounds.min[0]
            } else {
                bounds.max[0]
            },
            if corner & 2 == 0 {
                bounds.min[1]
            } else {
                bounds.max[1]
            },
            if corner & 4 == 0 {
                bounds.min[2]
            } else {
                bounds.max[2]
            },
        );
        let clip = matrix * model * p.extend(1.0);
        if clip.w.is_nan() || clip.w <= 1e-6 {
            return [0, 0, size[0], size[1]];
        }
        let ndc = clip.truncate().truncate() / clip.w;
        let px = Vec2::new((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h);
        lo = lo.min(px);
        hi = hi.max(px);
    }
    let x0 = (lo.x - 1.0).floor().clamp(0.0, w) as u32;
    let y0 = (lo.y - 1.0).floor().clamp(0.0, h) as u32;
    let x1 = (hi.x + 1.0).ceil().clamp(0.0, w) as u32;
    let y1 = (hi.y + 1.0).ceil().clamp(0.0, h) as u32;
    [x0, y0, x1.max(x0), y1.max(y0)]
}

/// One pane record's bytes: the rectangle and the instance's model slot.
pub(super) fn record(rect: [u32; 4], slot: u32) -> [u8; RECORD_BYTES as usize] {
    let mut bytes = [0u8; RECORD_BYTES as usize];
    for (i, word) in rect.into_iter().chain([slot, 0, 0, 0]).enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_ne_bytes());
    }
    bytes
}

impl Renderer {
    /// Whether this renderer shades panes per pixel (`wanted`, or a test's choice).
    pub(super) fn tp_pane_pixels(&self) -> bool {
        #[cfg(test)]
        if let Some(on) = self.transparency.pane_pixels {
            return on;
        }
        wanted()
    }
    /// `program`'s pane pipelines from its glass module `glass` on path `rt`: the id raster
    /// (the glass entry's vertex stages into four-sample `ID_FORMAT`, no blending) and the shading.
    pub(super) fn tp_pane_pipelines(
        &self,
        program: u64,
        rt: bool,
        glass: &str,
    ) -> Result<PanePipelines, Error> {
        let slot = self.program_slot(program);
        let (ids_source, shade_source) = sources(glass).map_err(Error::Shader)?;
        let gpu = self.transparency.gpu.as_ref().ok_or_else(|| {
            Error::Fault(Fault(
                "pane pipelines before the transparency pass's resources".into(),
            ))
        })?;
        let scopes = OperationScopes::new(&self.device);
        let module = |label: &str, source: String| {
            let label = format!("{label} {}", slot.label);
            self.ledger.own(
                self.device
                    .create_program_module(wgpu::ShaderModuleDescriptor {
                        label: Some(&label),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    }),
                ResourceKind::ShaderModule,
            )
        };
        let ids_module = module("TP pane ids", ids_source);
        let shade_module = module("TP pane shading", shade_source);
        let key = self.tp_glass_key(program, rt);
        let ids = [false, true].map(|tangent| {
            let label = format!(
                "TP pane ids program={} rt={rt} key={key:02x?} tangent={tangent}", slot.label,
            );
            self.ledger.own(
                self.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some(&label),
                        layout: Some(&gpu.glass_layouts[usize::from(rt)]),
                        vertex: wgpu::VertexState {
                            module: &ids_module,
                            entry_point: Some(if tangent { "vs_tangent" } else { "vs" }),
                            compilation_options: Default::default(),
                            buffers: &[
                                Some(wgpu::VertexBufferLayout {
                                    array_stride: 48,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &VERTEX_ATTRIBUTES,
                                }),
                                Some(wgpu::VertexBufferLayout {
                                    array_stride: 4,
                                    step_mode: wgpu::VertexStepMode::Instance,
                                    attributes: &MODEL_ATTRIBUTES,
                                }),
                                tangent.then_some(wgpu::VertexBufferLayout {
                                    array_stride: 16,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &TANGENT_ATTRIBUTES,
                                }),
                            ],
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &ids_module,
                            entry_point: Some("tp_pane_id_fs"),
                            compilation_options: Default::default(),
                            targets: &[Some(ID_FORMAT.into())],
                        }),
                        primitive: wgpu::PrimitiveState {
                            cull_mode: None,
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState {
                            count: 4,
                            ..Default::default()
                        },
                        multiview_mask: None,
                        cache: None,
                    }),
                ResourceKind::RenderPipeline,
            )
        });
        let label = format!("TP pane shading program={} rt={rt} key={key:02x?}", slot.label);
        let shade = self.ledger.own(
            self.device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(&label),
                    layout: Some(&gpu.pane_layouts[usize::from(rt)]),
                    module: &shade_module,
                    entry_point: Some("tp_pane_shade"),
                    compilation_options: Default::default(),
                    cache: None,
                }),
            ResourceKind::ComputePipeline,
        );
        if let Some(error) = scopes.finish() {
            return Err(match error {
                wgpu::Error::Validation { .. } => {
                    Error::Shader(format!("program `{}`, pane pipelines: {error}", slot.label))
                }
                _ => Error::Fault(Fault(error.to_string())),
            });
        }
        Ok(PanePipelines { ids, shade })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The standard program's pane modules compose and validate on both paths' heads.
    #[test]
    fn pane_modules_validate_on_both_heads() {
        const SCENE: &str = include_str!("../shaders/scene.wgsl");
        let rt = crate::hardware_shadows::shader_source(SCENE, crate::SampleCount::One).unwrap();
        for (head, is_rt) in [(rt.as_str(), true), (SCENE, false)] {
            let glass = glass_source(
                head,
                is_rt,
                crate::STANDARD_PROGRAM,
                crate::program::declares_parameters(crate::STANDARD_PROGRAM),
            );
            let (ids, shade) = sources(&glass).unwrap();
            // wgpu's two-halves id encoding replaced the port's in both modules.
            assert!(ids.contains("alias PaneId=vec2<u32>;") && !ids.contains("alias PaneId=u32;"));
            assert!(shade.contains("return t.x|(t.y<<16u);"));
            for (label, source) in [("ids", ids), ("shade", shade)] {
                let module = naga::front::wgsl::parse_str(&source)
                    .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(&source)));
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .unwrap_or_else(|e| panic!("{label} rt={is_rt}: {}", e.emit_to_string(&source)));
            }
        }
    }

    /// A box in front of the camera maps inside the view; one straddling the eye covers it.
    #[test]
    fn rects_bound_the_projected_box() {
        let matrix = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0)
            * Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let bounds = crate::visibility::Aabb {
            min: [-0.5; 3],
            max: [0.5; 3],
        };
        let r = rect(&bounds, Mat4::IDENTITY, matrix, [100, 100]);
        assert!(r[0] > 30 && r[2] < 70 && r[1] > 30 && r[3] < 70, "{r:?}");
        let near = rect(
            &bounds,
            Mat4::from_translation(Vec3::new(0.0, 0.0, 5.0)),
            matrix,
            [100, 100],
        );
        assert_eq!(near, [0, 0, 100, 100]);
    }
}
