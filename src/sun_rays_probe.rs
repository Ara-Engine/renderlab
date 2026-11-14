//! One-frame query census. No per-pixel storage or persistent renderer state.
use super::*;

const WORDS: u32 = 289;
/// The census frame without `RENDERLAB_SUN_RAYS_PROBE_FRAME`.
const DEFAULT_FRAME: u64 = 32;

/// The renderer frame (zero-based `frames` before submission) whose sun kernel the census
/// replaces: `RENDERLAB_SUN_RAYS_PROBE_FRAME`, else `DEFAULT_FRAME`. An unparsable value warns
/// once and keeps the default.
fn frame() -> u64 {
    static FRAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *FRAME.get_or_init(|| {
        frame_from(
            std::env::var("RENDERLAB_SUN_RAYS_PROBE_FRAME")
                .ok()
                .as_deref(),
        )
    })
}

fn frame_from(value: Option<&str>) -> u64 {
    let Some(value) = value else {
        return DEFAULT_FRAME;
    };
    value.trim().parse().unwrap_or_else(|_| {
        eprintln!("RENDERLAB_SUN_RAYS_PROBE_FRAME={value:?} is not a frame number; the census stays at frame {DEFAULT_FRAME}");
        DEFAULT_FRAME
    })
}

fn source(soft: &str) -> String {
    let mut source = schedule_source(temporal_reconstruction::vb_sun_source(soft).unwrap());
    let header =
        "fn fresh_estimate(receiver:Receiver, pixel:vec2<u32>, width:u32, lane:u32) -> u32 {";
    assert_eq!(source.matches(header).count(), 1);
    source = source.replacen(header, &format!("{header}\n    sunprobe_receiver=1u;"), 1);
    let budget = "tr1_rays=tr1_zone_rays(world,primary_frame.sampling.x);";
    source = source.replacen(budget, &format!("{budget}sunprobe_budget=tr1_rays;"), 1);
    source = source.replace(
        "rayQueryInitialize(&query,fresh_scene,",
        "sunprobe_rays+=1u;rayQueryInitialize(&query,fresh_scene,",
    );
    source = source.replacen("if j==tr1_held_probes", "if j==8u {sunprobe_prefix_agrees=u32(visible==select(0u,j,tr1_held_lit));}\n        if j==tr1_held_probes", 1);
    let at = source.rfind("    primary_sun_store(p,visible);").unwrap();
    source.insert_str(at, "    sunprobe_write(visible);\n");
    source.push_str(include_str!("../shaders/sun_rays_probe.wgsl"));
    source
}

pub(super) fn encode(
    renderer: &Renderer,
    encoder: &mut wgpu::CommandEncoder,
    stamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
) -> bool {
    let Some(path) = std::env::var_os("RENDERLAB_SUN_RAYS_PROBE") else {
        return false;
    };
    let frame = frame();
    if renderer.frames != frame {
        return false;
    }
    let state = renderer.view.primary.as_ref().unwrap();
    let targets = state.targets.as_ref().unwrap();
    let fresh = renderer
        .view
        .fresh
        .as_ref()
        .unwrap()
        .targets
        .as_ref()
        .unwrap();
    let held = targets.held.as_ref().unwrap();
    let parity = renderer
        .view
        .reconstruction_state
        .as_ref()
        .unwrap()
        .plan
        .parity;
    let device = &renderer.device;
    let buffer = |label, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: u64::from(WORDS) * 4,
            usage,
            mapped_at_creation: false,
        })
    };
    let counts = buffer(
        "sun ray counters",
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    );
    let readback = buffer(
        "sun ray census readback",
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let texture = |sample_type| wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let image = wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: wgpu::TextureFormat::R32Uint,
        view_dimension: wgpu::TextureViewDimension::D2,
    };
    let uint = texture(wgpu::TextureSampleType::Uint);
    let float = texture(wgpu::TextureSampleType::Float { filterable: false });
    let entries: Vec<_> = [
        (
            2,
            wgpu::BindingType::AccelerationStructure {
                vertex_return: false,
            },
        ),
        (3, float),
        (4, float),
        (5, image),
        (7, uint),
        (10, uniform),
        (21, uint),
        (50, uniform),
        (51, uint),
        (52, image),
        (
            80,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
    ]
    .into_iter()
    .map(|(binding, ty)| wgpu::BindGroupLayoutEntry {
        binding,
        ty,
        visibility: wgpu::ShaderStages::COMPUTE,
        count: None,
    })
    .collect();
    let bindings = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sun census inputs"),
        entries: &entries,
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sun census ABI"),
        bind_group_layouts: &[
            Some(&bindings),
            Some(&state.array_bindings),
            None,
            Some(&renderer.lighting.bindings),
        ],
        immediate_size: 0,
    });
    let source = source(&renderer.primary_sources.as_ref().unwrap().0[1]);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sun ray census"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("sun ray census"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("primary_vb_sun"),
        compilation_options: Default::default(),
        cache: None,
    });
    use wgpu::BindingResource as R;
    let resources = [
        (
            2,
            renderer
                .hardware_shadows
                .as_ref()
                .unwrap()
                .scene()
                .as_binding(),
        ),
        (3, R::TextureView(&fresh.world.view)),
        (4, R::TextureView(&fresh.normal.view)),
        (5, R::TextureView(&fresh.visibility.view)),
        (7, R::TextureView(&targets.hit.view)),
        (
            10,
            state.frame.as_ref().unwrap().0.value.as_entire_binding(),
        ),
        (21, R::TextureView(&targets.tiles.view)),
        (50, held.frame.value.as_entire_binding()),
        (51, R::TextureView(&held.history[1 - parity].view)),
        (52, R::TextureView(&held.history[parity].view)),
        (80, counts.as_entire_binding()),
    ];
    let resources: Vec<_> = resources
        .into_iter()
        .map(|(binding, resource)| wgpu::BindGroupEntry { binding, resource })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sun census inputs"),
        layout: &bindings,
        entries: &resources,
    });
    let size = targets.hit.texture.size();
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sun ray census (measurement only)"),
            timestamp_writes: stamps,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_bind_group(1, &*state.geometry.as_ref().unwrap().arrays, &[]);
        pass.set_bind_group(3, renderer.lighting.bind(), &[]);
        pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(4), 1);
    }
    encoder.copy_buffer_to_buffer(&counts, 0, &readback, 0, u64::from(WORDS) * 4);
    let mapped = readback.clone();
    encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
        if result.is_err() {
            return;
        }
        let Ok(data) = mapped.get_mapped_range(..) else {
            return;
        };
        let words: Vec<_> = data
            .chunks_exact(4)
            .map(|x| u32::from_ne_bytes(x.try_into().unwrap()))
            .collect();
        let list = |values: &[u32]| {
            values
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        };
        let json = format!(
            "{{\"frame\":{frame},\"counters\":[{}],\"ray_histogram\":[{}]}}\n",
            list(&words[..32]),
            list(&words[32..])
        );
        let _ = std::fs::write(path, json);
        drop(data);
        mapped.unmap();
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn census_counts_actual_queries_without_changing_visibility() {
        let kernels = shader_sources(include_str!("../shaders/scene.wgsl")).unwrap();
        let source = source(&kernels[1]);
        let module = naga::front::wgsl::parse_str(&source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        assert!(source.contains("sunprobe_rays+=1u;rayQueryInitialize(&query,fresh_scene,"));
        assert!(source.contains("sunprobe_budget=tr1_rays;"));
    }
    #[test]
    fn census_frame_defaults_to_32_and_takes_a_frame_number() {
        assert_eq!(frame_from(None), 32);
        assert_eq!(frame_from(Some("26")), 26);
        assert_eq!(frame_from(Some(" 0 ")), 0);
        assert_eq!(frame_from(Some("cut+1")), 32);
        assert_eq!(frame_from(Some("-1")), 32);
    }
}
