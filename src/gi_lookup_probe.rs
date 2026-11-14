//! Measurement only: receiver-walk flags in the prepared record's unused padding byte.
use super::*;

pub(super) fn on() -> bool {
    std::env::var_os("RENDERLAB_GI_LOOKUP_DIAG").is_some()
}
pub(super) fn source(mut source: String, enabled: bool) -> String {
    if !enabled {
        return source;
    }
    let edits = [
        (
            "fn gi_prepare(input:SurfaceInput) {",
            "var<private> gi_lookup_flags:u32;\nfn gi_prepare(input:SurfaceInput) {\n    gi_lookup_flags=0u;",
        ),
        (
            "sample_level=level-1u;",
            "gi_lookup_flags|=2u;\n            sample_level=level-1u;",
        ),
        (
            "let bw=gi_lv_bounce_weight(s);",
            "let bw=gi_lv_bounce_weight(s);\n        if bw<0.999 { gi_lookup_flags|=4u; }",
        ),
        (
            "let c=gi_sample(world,level+1u,bin);",
            "gi_lookup_flags|=1u;\n        let c=gi_sample(world,level+1u,bin);",
        ),
        (
            "let s2=gi_sample_refined(world,pick.lo,gi_second_bin(source));",
            "gi_lookup_flags|=8u;\n    let s2=gi_sample_refined(world,pick.lo,gi_second_bin(source));",
        ),
        (
            "gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9);",
            "gi_prepared[at].r2=vec4<u32>(q6,q7,q8,q9|((gi_lookup_flags|128u|select(0u,16u,gi_rx_valid))<<24u));",
        ),
    ];
    for (before, after) in edits {
        assert_eq!(
            source.matches(before).count(),
            1,
            "GI lookup diagnostic seam: {before}"
        );
        source = source.replacen(before, after, 1);
    }
    source
}

impl Renderer {
    /// Blocking, measurement-only walk counts: receivers, coarser fill, refinement,
    /// incomplete HOT bounce coverage, fallback walk and invalid cache. Requires
    /// RENDERLAB_GI_PASS=1 and RENDERLAB_GI_LOOKUP_DIAG before target allocation.
    pub fn indirect_receiver_lookup_counts(&mut self) -> Result<[u64; 6], Error> {
        if !on() {
            return Err(Error::InvalidInput("GI lookup diagnostics are not enabled"));
        }
        let target = self
            .view
            .primary
            .as_ref()
            .and_then(|p| p.targets.as_ref())
            .and_then(|t| t.gi_receiver.as_ref())
            .ok_or(Error::InvalidInput(
                "GI lookup diagnostics require the receiver pass",
            ))?;
        let size = target.values.value.size();
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GI lookup diagnostic readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&target.values.value, 0, &readback, 0, size);
        self.queue.submit([encoder.finish()]);
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(Error::Poll)?;
        let data = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("GI lookup diagnostic map"))?;
        let mut counts = [0; 6];
        for record in data.chunks_exact(BYTES as usize) {
            let flags = record[47];
            if flags & 128 == 0 {
                continue;
            }
            counts[0] += 1;
            for (index, mask) in [(1, 1), (2, 2), (3, 4), (4, 8)] {
                counts[index] += u64::from(flags & mask != 0);
            }
            counts[5] += u64::from(flags & 16 == 0);
        }
        drop(data);
        readback.unmap();
        Ok(counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walk_diagnostic_preserves_disabled_modules_and_validates_when_enabled() {
        let prelude = include_str!("../shaders/scene.wgsl");
        for head in super::super::super::non_rt_sources(prelude).unwrap().1 {
            let composed = program::compose(&head, program::STANDARD_PROGRAM, Stage::Traced, false);
            let mut module = if composed.source.contains("struct GiPrepared") {
                composed.source
            } else {
                shade_source_for(composed.source, true, true, true)
            };
            module.push_str(SOURCE.split_once(ENTRY_SEAM).unwrap().1);
            assert_eq!(source(module.clone(), false), module);
            let module = source(module, true);
            let parsed = naga::front::wgsl::parse_str(&module)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&module)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&parsed)
            .unwrap();
        }
    }
}
