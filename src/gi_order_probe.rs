//! P202 first-divergence diagnostic. GPU-only fingerprints; one final readback.
use super::*;
use std::sync::OnceLock;
pub(crate) const ENTRIES: [&str; 8] = [
    "go_stage0",
    "go_stage1",
    "go_stage2",
    "go_stage3",
    "go_stage4",
    "go_stage5",
    "go_stage6",
    "go_stage7",
];
const BYTES: u64 = (16 + 2048 * 8 * 48) * 4;
fn manifest() -> Option<&'static std::path::PathBuf> {
    static P: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    P.get_or_init(|| std::env::var_os("RENDERLAB_GI_ORDER_PROBE").map(Into::into))
        .as_ref()
}
pub(crate) fn on() -> bool {
    manifest().is_some()
}
pub(crate) fn source(mut s: String) -> String {
    if on() {
        s.push_str(include_str!("../shaders/gi_order_probe.wgsl"));
    }
    s
}
pub(super) struct State {
    pub buffer: Owned<wgpu::Buffer>,
    _allocation: Allocation,
}
impl State {
    pub fn new(device: &wgpu::Device, ledger: &Ledger) -> Self {
        Self {
            buffer: ledger.own(
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("GI order fingerprints"),
                    size: BYTES,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                ResourceKind::Buffer,
            ),
            _allocation: ledger.allocate(AllocationKind::Light, BYTES),
        }
    }
    pub fn prepare(&self, queue: &wgpu::Queue, frame: u64) {
        assert!(frame < 2048, "GI order diagnostic frame limit");
        queue.write_buffer(
            &self.buffer,
            0,
            &u32::try_from(frame).unwrap().to_le_bytes(),
        );
    }
}
impl Renderer {
    pub(crate) fn dump_gi_order_probe(&self) -> Result<(), Error> {
        let Some(p) = &self.gi.order_probe else {
            return Ok(());
        };
        let path = manifest().unwrap();
        let text = std::fs::read_to_string(path).expect("order probe manifest");
        let frame: u64 = text
            .lines()
            .find_map(|l| l.strip_prefix("dump_frame="))
            .expect("dump_frame=")
            .parse()
            .expect("dump_frame integer");
        if self.frames != frame {
            return Ok(());
        }
        let dst = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GI order final readback"),
            size: BYTES,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut e = self.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&p.buffer, 0, &dst, 0, BYTES);
        self.queue.submit([e.finish()]);
        let slice = dst.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(WAIT).map_err(Error::Poll)?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|_| Error::InvalidInput("order probe map"))?;
        let out = path.with_extension("");
        std::fs::create_dir_all(&out).expect("order probe directory");
        std::fs::write(out.join("fingerprints.u32"), &*mapped).expect("order fingerprints write");
        drop(mapped);
        dst.unmap();
        std::fs::write(out.join("layout.txt"),"frames=2048\nstages=8\nrow_words=48\nheader_words=16\nstages_order=before_probe,after_probe,after_gather,after_resolve,after_update,after_filter,after_light,after_cell_shading\ndomains=primary_hit_ordered,raw_misses_ordered,raw_misses_set,chosen_keys_set,cell_keys_set,cold_state,hot_sky,raw_bounce,outgoing,hot_bounce\ntriplet=count,xor_mix,sum_salted_mix\nno_per_frame_cpu_readback=1\n").expect("order layout write");
        eprintln!(
            "GI_ORDER_PROBE dumped renderer_frame={} output={}",
            frame,
            out.display()
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order_fingerprints_validate_both_paths() {
        for soft in [false, true] {
            let mut s =
                super::super::priority::source(super::super::kernel_source_base(soft, 64), soft);
            s.push_str(include_str!("../shaders/gi_order_probe.wgsl"));
            let m = naga::front::wgsl::parse_str(&s)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&s)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&m)
            .unwrap();
        }
    }
    #[test]
    fn flag_off_adds_no_source() {
        if !on() {
            for soft in [false, true] {
                let s = super::super::kernel_source_base(soft, 64);
                assert_eq!(source(s.clone()), s);
            }
        }
    }
    #[test]
    fn order_probe_device_request_includes_its_storage_binding() {
        let limits = crate::visibility_limits(wgpu::Limits::default());
        let baseline = 13
            + u32::from(crate::program::cache_hits_rough().is_some())
            + crate::renderer::primary_ray_gpu::reflection_pass::STORAGE_BUFFERS
                * u32::from(crate::program::rl_pass_env() == Some(true));
        assert!(limits.max_storage_buffers_per_shader_stage >= baseline + u32::from(on()));
    }
}
