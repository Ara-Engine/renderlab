//! Opt-in P202 diagnostic. No cache policy or production representation changes.
use super::*;
use std::sync::OnceLock;
pub(crate) const SLOTS: u32 = 1 << 20;
pub(crate) const CAP: u32 = 1 << 21;
pub(crate) const WORDS: u64 = 16 + SLOTS as u64 * 4 + CAP as u64 * 16;
pub(crate) fn manifest() -> Option<&'static std::path::PathBuf> {
    static P: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    P.get_or_init(|| std::env::var_os("RENDERLAB_GI_PROVENANCE").map(Into::into))
        .as_ref()
}
pub(crate) fn on() -> bool {
    manifest().is_some()
}
pub(crate) struct State {
    pub buffer: Owned<wgpu::Buffer>,
    _allocation: Allocation,
}
impl State {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        let size = WORDS * 4;
        assert!(
            size <= u64::from(device.limits().max_storage_buffer_binding_size),
            "provenance buffer exceeds binding limit"
        );
        Self {
            buffer: ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("GI provenance diagnostic"),
                    size,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            _allocation: ledger.allocate(AllocationKind::Light, size),
        }
    }
}
fn replace(source: &mut String, a: &str, b: &str) {
    assert_eq!(source.matches(a).count(), 1, "provenance seam: {a}");
    *source = source.replacen(a, b, 1);
}
pub(crate) fn kernels(s: String) -> String {
    kernels_for(s, on())
}
fn kernels_for(mut s: String, enabled: bool) -> String {
    if !enabled {
        return s;
    }
    s.push_str(include_str!("../shaders/gi_provenance_cells.wgsl"));
    replace(
        &mut s,
        "        atomicAdd(&gi_work[GI_W_INVALIDATED],1u);",
        "        atomicAdd(&gi_work[GI_W_INVALIDATED],1u); gp_invalidate(slot);",
    );
    // Both reset sites end the old value's generation, including slot reuse.
    s = s.replace(
        "gi_table[gi_hot(slot)+GI_HOT_BOUNCE]=0u;",
        "gi_table[gi_hot(slot)+GI_HOT_BOUNCE]=0u; gp_reset(slot);",
    );
    s = s.replace(
        "gi_table[hot+GI_HOT_BOUNCE]=0u;",
        "gi_table[hot+GI_HOT_BOUNCE]=0u; gp_reset(slot);",
    );
    replace(
        &mut s,
        "    let ph=gi_hot(parent);",
        "    let ph=gi_hot(parent);",
    );
    replace(
        &mut s,
        "    for(var w=GI_HOT_BOUNCE;w<GI_HOT_BOUNCE+7u;w+=1u) {\n        gi_table[hot+w]=gi_table[ph+w];",
        "    gp_parent(slot,parent);\n    for(var w=GI_HOT_BOUNCE;w<GI_HOT_BOUNCE+7u;w+=1u) {\n        gi_table[hot+w]=gi_table[ph+w];",
    );
    // RT and software each commit the raw bounce at lane 0, before filtering.
    let seam = "// The unfiltered value, the filter's source (G3).";
    assert!((1..=2).contains(&s.matches(seam).count()));
    s = s.replace(seam, &format!("gp_gather(slot); {seam}"));
    s
}
pub(crate) fn shading(s: String, eligible: bool) -> String {
    shading_for(s, eligible, on())
}
fn shading_for(mut s: String, eligible: bool, enabled: bool) -> String {
    if !enabled || !eligible {
        return s;
    }
    s.push_str(include_str!("../shaders/gi_provenance_lookup.wgsl"));
    s = s.replace(
        "    rl_indirect_prepare(surface_input);",
        "    gp_begin(surface_input); rl_indirect_prepare(surface_input); gp_prepared();",
    );
    replace(
        &mut s,
        "    let u=gi_lattice(world,gi_edge(level),bin);",
        "    gp_level=level; gp_bin=bin; gp_record(6u,vec4<u32>(level,bin,0u,0u));\n    let u=gi_lattice(world,gi_edge(level),bin);",
    );
    replace(
        &mut s,
        "    let s=w*bitcast<f32>(h.r0.y);",
        "    gp_record(1u,vec4<u32>(gp_slot,gp_level,gp_bin,bitcast<u32>(w)));\n    let s=w*bitcast<f32>(h.r0.y);",
    );
    replace(
        &mut s,
        "fn gi_rx_add(w:f32, b:f32) {",
        "fn gi_rx_add(w:f32, b:f32) { gp_record(2u,vec4<u32>(bitcast<u32>(w),bitcast<u32>(b),0u,0u));",
    );
    replace(
        &mut s,
        "fn gi_keep_level() {",
        "fn gi_keep_level() { gp_record(5u,vec4<u32>(0u));",
    );
    replace(
        &mut s,
        "fn gi_restore_level() {",
        "fn gi_restore_level() { gp_record(4u,vec4<u32>(0u));",
    );
    // gi_hot_load is used by every successful find; retained slot is consumed by hot_add.
    replace(
        &mut s,
        "fn gi_hot_load(slot:u32) -> GiHot {",
        "fn gi_hot_load(slot:u32) -> GiHot { gp_slot=slot;",
    );
    // first is loaded before next/loop; explicitly restore its slot at return.
    replace(
        &mut s,
        "if (first.r0.x&GI_KEY_BITS)==key.x { return first; }",
        "if (first.r0.x&GI_KEY_BITS)==key.x { gp_slot=bucket+(gi_start(key)&(GI_BUCKET-1u)); return first; }",
    );
    s
}

fn frame_list() -> Vec<u64> {
    let text = std::fs::read_to_string(manifest().expect("provenance manifest"))
        .expect("read provenance manifest");
    text.lines()
        .find_map(|l| l.strip_prefix("frames="))
        .expect("manifest frames=")
        .split(',')
        .map(|v| v.trim().parse().expect("renderer frame"))
        .collect()
}
impl State {
    pub fn prepare(&self, queue: &wgpu::Queue, frame: u64, slots: u32, parent: u32) {
        assert!(slots <= SLOTS, "provenance slot capacity");
        let mut h = [0u32; 16];
        h[0] = u32::try_from(frame).expect("frame fits u32");
        h[1] = u32::from(frame_list().contains(&frame));
        h[2] = slots;
        h[3] = CAP;
        h[6] = parent;
        let bytes: Vec<u8> = h.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&self.buffer, 0, &bytes);
    }
}
fn read(r: &Renderer, b: &wgpu::Buffer, size: u64) -> Result<Vec<u8>, Error> {
    let dst = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("GI provenance readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut e = r.device.create_command_encoder(&Default::default());
    e.copy_buffer_to_buffer(b, 0, &dst, 0, size);
    r.queue.submit([e.finish()]);
    let slice = dst.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    r.device.poll(WAIT).map_err(Error::Poll)?;
    let mapped = slice
        .get_mapped_range()
        .map_err(|_| Error::InvalidInput("GI provenance map"))?;
    let out = mapped.to_vec();
    drop(mapped);
    dst.unmap();
    Ok(out)
}
impl Renderer {
    pub(crate) fn dump_gi_provenance(&self) -> Result<(), Error> {
        let Some(p) = &self.gi.provenance else {
            return Ok(());
        };
        if !frame_list().contains(&self.frames) {
            return Ok(());
        }
        let r = self
            .gi
            .resources
            .as_ref()
            .ok_or(Error::InvalidInput("provenance cache missing"))?;
        let h = read(self, &p.buffer, 64)?;
        let word = |i: usize| u32::from_le_bytes(h[4 * i..4 * i + 4].try_into().unwrap());
        if word(5) != 0 || word(4) > CAP {
            return Err(Error::InvalidInput("GI provenance output overflow"));
        }
        if word(0) as u64 != self.frames {
            return Err(Error::InvalidInput("GI provenance frame mismatch"));
        }
        let output = manifest()
            .unwrap()
            .with_extension("")
            .join(format!("frame-{}", self.frames));
        std::fs::create_dir_all(&output).expect("provenance output directory");
        let bytes = read(
            self,
            &p.buffer,
            (16 + SLOTS as u64 * 4 + u64::from(word(4)) * 16) * 4,
        )?;
        std::fs::write(output.join("provenance.u32"), bytes).expect("provenance output");
        for (name, buffer) in [("table.u32", &*r.table), ("raw.u32", &*r.raw)] {
            std::fs::write(output.join(name), read(self, buffer, buffer.size())?)
                .expect("provenance snapshot");
        }
        std::fs::write(output.join("metadata.txt"),format!("renderer_frame={}\nrecords={}\nsidecar_slots={}\ntable_slots={}\nparent_standin={}\n",self.frames,word(4),SLOTS,r.slots,word(6))).expect("provenance metadata");
        eprintln!(
            "GI_PROVENANCE frame={} records={} parent_standin={} output={}",
            self.frames,
            word(4),
            word(6),
            output.display()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flag_off_leaves_kernel_bytes_unchanged() {
        for soft in [false, true] {
            let source = super::super::kernel_source_base(soft, 64);
            assert_eq!(kernels_for(source.clone(), false), source);
            assert_eq!(shading_for(source.clone(), true, false), source);
        }
    }
    #[test]
    fn diagnostic_gather_stamps_parse() {
        for soft in [false, true] {
            let source =
                super::super::priority::source(super::super::kernel_source_base(soft, 64), soft);
            let source = kernels_for(source, true);
            naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        }
    }
    #[test]
    fn diagnostic_lookup_validates_and_flag_off_is_identical() {
        use crate::program::{self, Stage};
        use crate::renderer::primary_ray_gpu;
        let prelude = include_str!("../shaders/scene.wgsl");
        let hw = primary_ray_gpu::shading_sources(prelude, program::STANDARD_PROGRAM).unwrap();
        let (_, heads) = primary_ray_gpu::non_rt_sources(prelude).unwrap();
        let sw = heads
            .map(|h| program::compose(&h, program::STANDARD_PROGRAM, Stage::Traced, false).source);
        for source in hw.into_iter().chain(sw) {
            assert_eq!(shading_for(source.clone(), true, false), source);
            let source = shading_for(source, true, true);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }
}
