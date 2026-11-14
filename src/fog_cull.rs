//! Opt-in current-frame opaque-depth culling for wgpu RT fog queries.
//! No per-pixel record: visibility reduces its already decoded points to 8x4 depths.
//! RENDERLAB_FOG_CULL=1 adds binding 81 on the wgpu producer only. Off retains
//! the original modules, bindings, allocations and early fog schedule. On moves
//! only inject/temporal/integrate after visibility and repair; LUT work stays early.
//! Skipped cells hold zero RGBA, with G=0 marking invalid history. Filled cells
//! have G=1; an incomplete raw neighbourhood marks its filtered result invalid.
//! History is reused only when every contributing trilinear tap is valid, so a
//! newly visible cell uses its current rays rather than old placeholder values.
use super::*;
use primary_ray_gpu::Buffer;

pub(crate) const BINDING: u32 = 81;
const PRODUCER: &str = include_str!("../shaders/fog_depth_producer.wgsl");
const OCCLUSION: &str = include_str!("../shaders/fog_occlusion.wgsl");

pub(crate) fn enabled() -> bool {
    std::env::var("RENDERLAB_FOG_CULL").as_deref() == Ok("1")
}
pub(super) fn entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: BINDING,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(48),
        },
        count: None,
    }
}
pub(super) fn extend_entries(entries: &mut Vec<wgpu::BindGroupLayoutEntry>, on: bool) {
    if on {
        entries.push(entry());
    }
}

// The canonical functions have balanced braces. Apply edits to entries only: no
// receiver, repair, light or material helper changes, and no port factory changes.
fn entries(source: &str) -> Vec<(std::ops::Range<usize>, String)> {
    source
        .match_indices("@compute")
        .map(|(at, _)| {
            let start = at + source[at..].find("fn ").unwrap();
            let name_end = start + source[start..].find('(').unwrap();
            let name = source[start + 3..name_end].to_owned();
            let brace = start + source[start..].find('{').unwrap();
            let mut depth = 1;
            let mut end = brace + 1;
            while depth != 0 {
                depth += i32::from(source.as_bytes()[end] == b'{')
                    - i32::from(source.as_bytes()[end] == b'}');
                end += 1;
            }
            (start..end, name)
        })
        .collect()
}
fn edit(source: &mut String, name: &str, before: &str, after: &str) {
    let (range, _) = entries(source)
        .into_iter()
        .find(|(_, n)| n == name)
        .unwrap();
    let body = &source[range.clone()];
    assert_eq!(body.matches(before).count(), 1, "fog cull seam in {name}");
    let body = body.replacen(before, after, 1);
    source.replace_range(range, &body);
}

pub(super) fn producer_source(mut source: String, on: bool) -> String {
    if !on {
        return source;
    }
    const INITIAL: [&str; 4] = [
        "primary_trace",
        "primary_trace_receivers",
        "primary_trace_interactive",
        "primary_trace_interactive_deferred",
    ];
    for (range, name) in entries(&source).into_iter().rev() {
        let mut body = source[range.clone()].to_owned();
        if INITIAL.contains(&name.as_str()) {
            let seam = if name.starts_with("primary_trace_interactive") {
                "    workgroupBarrier();"
            } else {
                "    if !primary_tile_covered("
            };
            assert!(body.contains(seam), "fog cull entry {name}");
            body = body.replacen(
                seam,
                &format!(
                    "    rl_fog_depth_group(id.xy,group.xy,trace.record,trace.point);\n{seam}"
                ),
                1,
            );
        } else {
            // A sparse correction invalidates the whole half-tile conservatively.
            // This also covers future retry/fallback entries that publish a hit.
            let mut cursor = 0;
            let mut writes = vec![];
            while let Some(at) = body[cursor..].find("textureStore(primary_hit_output,") {
                let begin = cursor + at + "textureStore(primary_hit_output,".len();
                let mut end = begin;
                let mut nesting = 0;
                while body.as_bytes()[end] != b',' || nesting != 0 {
                    nesting += i32::from(body.as_bytes()[end] == b'(')
                        - i32::from(body.as_bytes()[end] == b')');
                    end += 1;
                }
                let pixel = body[begin..end].trim().to_owned();
                let after = end + body[end..].find(';').unwrap() + 1;
                writes.push((after, pixel));
                cursor = after;
            }
            for (at, pixel) in writes.into_iter().rev() {
                body.insert_str(
                    at,
                    &format!("\n    rl_fog_depth_changed(vec2<u32>({pixel}));"),
                );
            }
        }
        source.replace_range(range, &body);
    }
    source + PRODUCER
}

pub(super) fn atmosphere_source(mut source: String, on: bool) -> String {
    if !on {
        return source;
    }
    let guard = "    if any(id>=grid) { return; }";
    edit(
        &mut source,
        "at_fog_inject",
        guard,
        &format!(
            "{guard}\n    if at_fog_cell_hidden(id) {{textureStore(at_scatter_out,vec3<i32>(id),vec4<f32>(0.0));return;}}"
        ),
    );
    edit(
        &mut source,
        "at_fog_inject",
        "lit_density+=extinction*at_sun_visible(world);",
        "if extinction>0.0 {lit_density+=extinction*at_sun_visible(world);}",
    );
    edit(
        &mut source,
        "at_fog_inject",
        "lit_density/max(total_density,1e-20),0.0,0.0,",
        "lit_density/max(total_density,1e-20),select(0.0,1.0,at_fog_cull_enabled()),0.0,",
    );
    edit(
        &mut source,
        "at_fog_inject",
        "    textureStore(at_scatter_out,vec3<i32>(id),",
        "    if total_density==0.0 && at_fog_cull_enabled() {atomicAdd(&at_fog_bounds.zero_density,1u);}\n    textureStore(at_scatter_out,vec3<i32>(id),",
    );
    edit(
        &mut source,
        "at_fog_temporal",
        "    let extinction=value.a;",
        "    let extinction=value.a;\n    if at_fog_cull_enabled() && value.g==0.0 {textureStore(at_scatter_out,vec3<i32>(id),vec4<f32>(0.0));return;}\n    var complete=true;",
    );
    edit(
        &mut source,
        "at_fog_temporal",
        "                let sample=textureLoad(at_scatter,cell,0).r;",
        "                let neighbor=textureLoad(at_scatter,cell,0);\n                if at_fog_cull_enabled() && neighbor.g==0.0 {complete=false;continue;}\n                let sample=neighbor.r;",
    );
    edit(
        &mut source,
        "at_fog_temporal",
        "if all(previous>=border) && all(previous<=vec3<f32>(1.0)-border) {",
        "if all(previous>=border) && all(previous<=vec3<f32>(1.0)-border) && (!at_fog_cull_enabled() || at_fog_history_filled(previous,grid)) {",
    );
    edit(
        &mut source,
        "at_fog_temporal",
        "mix(value.r,old,weight),0.0,0.0,extinction",
        "mix(value.r,old,weight),value.g,0.0,extinction",
    );
    edit(
        &mut source,
        "at_fog_temporal",
        "    textureStore(at_scatter_out,vec3<i32>(id),value);",
        "    if at_fog_cull_enabled() {value.g=select(0.0,1.0,complete);}\n    textureStore(at_scatter_out,vec3<i32>(id),value);",
    );
    source + OCCLUSION
}

pub(super) struct Depth {
    pub buffer: Buffer,
    size: [u32; 4],
}
impl Depth {
    pub fn new(r: &Renderer, size: wgpu::Extent3d) -> Self {
        let groups = [
            size.width.div_ceil(8),
            size.height.div_ceil(4),
            size.width,
            size.height,
        ];
        Self {
            buffer: Buffer::new(
                r,
                "fog opaque half-tile depths",
                (32 + u64::from(groups[0]) * u64::from(groups[1]) * 4).max(48),
                wgpu::BufferUsages::STORAGE,
            ),
            size: groups,
        }
    }
    pub fn prepare(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, eye: Vec3) {
        let mut bytes = vec![];
        for x in [eye.x, eye.y, eye.z, 0.0] {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        for x in self.size {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        queue.write_buffer(&self.buffer.value, 0, &bytes);
        // Zero is unknown, never a culling depth. A path without a producer stays full.
        encoder.clear_buffer(&self.buffer.value, 32, None);
    }
}

pub(super) struct Bounds {
    pub buffer: Owned<wgpu::Buffer>,
    _allocation: Allocation,
    group: Option<(wgpu::Buffer, Owned<wgpu::BindGroup>)>,
    grid: [u32; 2],
}
impl Bounds {
    pub fn new(device: &PipelineDevice, ledger: &Ledger, grid: [u32; 2]) -> Self {
        let bytes = 16 + u64::from(grid[0]) * u64::from(grid[1]) * 4;
        Self {
            buffer: ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("fog first-hidden slices and counters"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            _allocation: ledger.allocate(AllocationKind::Target, bytes),
            group: None,
            grid,
        }
    }
    pub fn reset(&self, queue: &wgpu::Queue) {
        // Disabled until a same-frame visibility producer has completed. This also
        // makes unsupported primary modes a full-query fallback, never empty fog.
        queue.write_buffer(&self.buffer, 0, &[0; 16]);
    }
    pub fn encode(
        &mut self,
        device: &PipelineDevice,
        ledger: &Ledger,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        uniform: &wgpu::Buffer,
        raw: &wgpu::Buffer,
    ) {
        if self.group.as_ref().is_none_or(|(key, _)| key != raw) {
            self.group = Some((
                raw.clone(),
                ledger.own(
                    device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("fog current opaque-depth bounds"),
                        layout: &pipeline.get_bind_group_layout(0),
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: uniform.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 21,
                                resource: raw.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 22,
                                resource: self.buffer.as_entire_binding(),
                            },
                        ],
                    }),
                    ResourceKind::BindGroup,
                ),
            ));
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fog opaque-depth halo"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &*self.group.as_ref().unwrap().1, &[]);
        pass.dispatch_workgroups(self.grid[0].div_ceil(8), self.grid[1].div_ceil(8), 1);
    }
    pub fn probe(&self, device: &PipelineDevice, encoder: &mut wgpu::CommandEncoder, frame: u64) {
        if std::env::var("RENDERLAB_FOG_CULL_PROBE_FRAME")
            .ok()
            .and_then(|s| s.parse().ok())
            != Some(frame)
        {
            return;
        }
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog cull counters"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.buffer, 0, &readback, 0, 16);
        let mapped = readback.clone();
        let grid = self.grid;
        encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
            if result.is_ok() && let Ok(bytes) = mapped.slice(..).get_mapped_range() {
                let words: Vec<_> = bytes.chunks_exact(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect();
                eprintln!("fog_cull frame={frame} active={} skipped={} zero_density_in_queried_cells={} froxels={}",words[0],words[1],words[2],grid[0]*grid[1]*64);
            }
            mapped.unmap();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
    }
    #[test]
    fn fog_cull_off_keeps_source_and_layout() {
        for source in
            primary_ray_gpu::shader_sources(include_str!("../shaders/scene.wgsl")).unwrap()
        {
            assert_eq!(producer_source(source.clone(), false), source);
        }
        let source = include_str!("../shaders/atmosphere.wgsl").to_owned();
        assert_eq!(atmosphere_source(source.clone(), false), source);
        let non_rt = super::super::atmosphere_gpu::compile_structure_sources(false);
        let expected = format!(
            "{}{}{}",
            include_str!("../shaders/atmosphere_common.wgsl"),
            include_str!("../shaders/atmosphere.wgsl"),
            include_str!("../shaders/atmosphere_shadow_map.wgsl")
        );
        assert_eq!(
            non_rt
                .iter()
                .find(|(name, _)| *name == "kernels")
                .unwrap()
                .1,
            expected
        );
        let old = vec![wgpu::BindGroupLayoutEntry {
            binding: 10,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(16),
            },
            count: None,
        }];
        let mut off = old.clone();
        extend_entries(&mut off, false);
        assert_eq!(off, old);
        extend_entries(&mut off, true);
        assert_eq!(off.last().unwrap().binding, BINDING);
        assert_eq!(off.len(), old.len() + 1);
    }
    #[test]
    fn fog_cull_producers_and_atmosphere_validate() {
        for source in
            primary_ray_gpu::shader_sources(include_str!("../shaders/scene.wgsl")).unwrap()
        {
            let on = producer_source(source, true);
            validate(&on);
            let module = naga::front::wgsl::parse_str(&on).unwrap();
            let raw = module
                .global_variables
                .iter()
                .find(|(_, v)| {
                    v.binding
                        .as_ref()
                        .is_some_and(|b| b.group == 0 && b.binding == BINDING)
                })
                .unwrap()
                .1;
            let naga::TypeInner::Struct { span, .. } = &module.types[raw.ty].inner else {
                panic!("fog depth header must be a struct")
            };
            let wgpu::BindingType::Buffer {
                min_binding_size, ..
            } = entry().ty
            else {
                unreachable!()
            };
            assert!(min_binding_size.unwrap().get() >= u64::from(*span));
            if on.contains("fn primary_trace_interactive(") {
                validate(&primary_ray_gpu::film_accumulation::kernel_source(&on).unwrap());
            }
            // Every entry that can write a hit either publishes a group depth or
            // invalidates its half-tile; sparse fallback writes cannot escape it.
            for (range, name) in entries(&on) {
                let body = &on[range];
                if body.contains("textureStore(primary_hit_output,") {
                    assert!(
                        body.contains("rl_fog_depth_group(")
                            || body.contains("rl_fog_depth_changed("),
                        "{name}"
                    );
                }
            }
        }
        let source = format!(
            "enable wgpu_ray_query;\n{}{}{}{}",
            crate::shadows::SHADOW_CASTER_MASK_WGSL,
            include_str!("../shaders/atmosphere_common.wgsl"),
            atmosphere_source(include_str!("../shaders/atmosphere.wgsl").to_owned(), true),
            super::super::atmosphere_gpu::HARDWARE_VISIBILITY
        );
        validate(&source);
        assert!(source.contains("history_filled(previous,grid)"));
        assert!(source.contains("neighbor.g==0.0 {complete=false;continue;}"));
    }
    #[test]
    fn measured_halo_keeps_visible_prefix_and_six_neighbours_filled() {
        let size = 9usize;
        let range = 60.0f32;
        let coordinate = |d: f32| (1.0 + d / range * 31.0).log2() / 5.0 * 64.0;
        let mut depths = vec![0.0f32; size * size];
        let mut seed = 17u32;
        for d in &mut depths {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *d = 2.0 + (seed % 5800) as f32 / 100.0;
        }
        depths[0] = f32::INFINITY;
        let at = |x: isize, y: isize| {
            y.clamp(0, size as isize - 1) as usize * size + x.clamp(0, size as isize - 1) as usize
        };
        let mut hidden = vec![64u32; size * size];
        for y in 0..size as isize {
            for x in 0..size as isize {
                let mut far = 0.0f32;
                for dy in -2..=2 {
                    for dx in -2..=2 {
                        far = far.max(depths[at(x + dx, y + dy)]);
                    }
                }
                if far < range {
                    hidden[at(x, y)] =
                        (coordinate(far + 0.001f32.max(far * 1e-5)).ceil() as u32 + 1).min(64);
                }
            }
        }
        for y in 0..size as isize {
            for x in 0..size as isize {
                let upper = (coordinate(depths[at(x, y)]).min(64.0) - 1.0)
                    .max(0.0)
                    .ceil() as u32;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        for (nx, ny, nz) in [
                            (0, 0, 0),
                            (1, 0, 0),
                            (-1, 0, 0),
                            (0, 1, 0),
                            (0, -1, 0),
                            (0, 0, 1),
                            (0, 0, -1),
                        ] {
                            let needed = (upper as i32 + nz).clamp(0, 63) as u32;
                            assert!(needed < hidden[at(x + dx + nx, y + dy + ny)]);
                        }
                    }
                }
            }
        }
    }
}
