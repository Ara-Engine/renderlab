"""Prepare a measurement-only 8x4 GI probe, never a production G-E variant.

Uses the existing GI_F_STATS counters at shading-tile dimensions. Build/run only under
this unit's build guards/GPU lease. The frozen source lives outside the checkout.
"""
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path('D:/Coding/renderlab-work/p-gie')
SOURCE = ROOT / 'probe-source'
BASE = '21088f37f5159385e69d3b7f5691bf185cb33359'


def replace(path, before, after):
    source = path.read_text(encoding='utf-8')
    if source.count(before) != 1:
        raise RuntimeError(f'expected one instrumentation seam in {path}: {before}')
    path.write_text(source.replace(before, after), encoding='utf-8')


def prepare():
    ROOT.mkdir(parents=True, exist_ok=True)
    SOURCE.mkdir(exist_ok=False)
    archive = ROOT / 'probe-source.zip'
    with archive.open('wb') as output:
        subprocess.run(['git', 'archive', '--format=zip', BASE], stdout=output, check=True)
    with zipfile.ZipFile(archive) as bundle:
        bundle.extractall(SOURCE)
    archive.unlink()
    replace(SOURCE/'shaders/gi_cache.wgsl',
            'const GI_PROBE_LANES:u32=GI_PROBE_TILE*GI_PROBE_TILE;',
            'const GI_PROBE_LANES:u32=GI_PROBE_TILE*4u;')
    replace(SOURCE/'shaders/gi_cache.wgsl', '@workgroup_size(8,8) fn gi_probe',
            '@workgroup_size(8,4) fn gi_probe')
    replace(SOURCE/'src/gi_cache.rs', '(region[3] - region[1]).div_ceil(probe_tile()),',
            '(region[3] - region[1]).div_ceil(4),')
    example = SOURCE/'examples/ara1_capture.rs'
    replace(example, 'let mut gpu = Vec::new();', '''let mut gpu = Vec::new();
    let mut probes = Vec::new();
    engine.renderer_mut().set_indirect_profiling(true);
    fs::write(out.join("device-limits.json"), serde_json::to_vec_pretty(&json!({
        "max_compute_workgroup_storage_size":engine.device().limits().max_compute_workgroup_storage_size
    }))?)?;''')
    replace(example, 'let service_ms = start.elapsed().as_secs_f64() * 1e3;', '''let service_ms = start.elapsed().as_secs_f64() * 1e3;
        if let Some(c) = engine.renderer_mut().indirect_counters()? {
            let p = c.probe;
            probes.push(json!({"frame":index,"phase":frame.phase,"tiles":p.tiles,
                "pixels":p.pixels,"cells":p.cells,"parents":p.parents,"refined":p.refined,
                "finds":p.finds,"max_cells":p.max_cells,"tiles_by_cells":p.tiles_by_cells,
                "local_overflow":c.local_overflow}));
            fs::write(out.join("probe.json"), serde_json::to_vec_pretty(&probes)?)?;
        }''')
    scenes = Path('D:/Coding/renderlab-work/ara1/phase2-scenes')
    for name in ('game-base', 'quarry-base'):
        original = scenes / f'{name}-timing.json'
        spec = json.loads(original.read_text())
        asset = Path(spec['asset'])
        spec['asset'] = str((original.parent/asset).resolve())
        # Retain the registered 1080p still/motion/cut plan; omit PNG encoding.
        for frame in spec['frames']:
            frame['capture'] = False
        (ROOT/f'{name}.json').write_text(json.dumps(spec))
    (ROOT/'source.json').write_text(json.dumps(dict(base=BASE, tile=[8,4],
        note='Existing probe request union, including conservative bin/level ties and fallback rounds; not shading-only keys.'), indent=2))


if __name__ == '__main__':
    prepare()
