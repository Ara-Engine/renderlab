"""Materialize the historical P-RTS screen, with process-local variant selection.

No production checkout files are changed. Run from the repository root after
git archive --format=zip --output=D:/Coding/renderlab-work/p-rts2/source.zip db326de.
"""
from pathlib import Path
import zipfile

OUT = Path('D:/Coding/renderlab-work/p-rts2')
SOURCE = OUT / 'source'
with zipfile.ZipFile(OUT / 'source.zip') as archive:
    archive.extractall(SOURCE)
p = SOURCE / 'src/temporal_reconstruction.rs'
s = p.read_text()
s = s.replace('const PROBE_DISK: u32 = 8;', '''fn probe_count() -> u32 {
    std::env::var("P_RTS2_COUNT").unwrap_or("8".into()).parse().unwrap()
}
fn probe_disk() -> u32 {
    if probe_cone() > 1.0 { 0 } else { probe_count() }
}''')
s = s.replace('const PROBE_CONE: f32 = 2.0;', '''fn probe_cone() -> f32 {
    std::env::var("P_RTS2_CONE").unwrap_or("1".into()).parse().unwrap()
}''')
s = s.replace('pub(super) const PENUMBRA_LISTING: bool = false;', '''pub(super) fn penumbra_listing() -> bool {
    std::env::var("P_RTS2_VARIANT").is_ok_and(|v| v != "base")
}''')
s = s.replace('trace_source_with(soft_kernels, PENUMBRA_LISTING)', 'trace_source_with(soft_kernels, penumbra_listing())')
s = s.replace('&& PENUMBRA_LISTING', '&& penumbra_listing()')
s = s.replace('2 * PROBE_DISK', 'probe_count()').replace('(PROBE_DISK, PROBE_CONE)', '(probe_disk(), probe_cone())')
s = s.replace('primary_frame.sampling.z!=0u && j>=primary_frame.sampling.z', 'primary_frame.sampling.y<primary_frame.sampling.x && j>=primary_frame.sampling.z')
p.write_text(s)
p = SOURCE / 'src/primary_ray_gpu.rs'
s = p.read_text().replace('temporal_reconstruction::PENUMBRA_LISTING', 'temporal_reconstruction::penumbra_listing()')
s = s.replace('temporal_reconstruction::PENUMBRA_LIST.into()', '''temporal_reconstruction::PENUMBRA_LIST.replace(
                    "const PENUMBRA_RADIUS:i32=1;",
                    &format!("const PENUMBRA_RADIUS:i32={};", std::env::var("P_RTS2_RADIUS").unwrap_or("2".into()))
                ).into()''')
p.write_text(s)
assert 'const PENUMBRA_LISTING:' not in (SOURCE / 'src/temporal_reconstruction.rs').read_text()
print(SOURCE)


