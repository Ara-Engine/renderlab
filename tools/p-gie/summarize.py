"""Summarize the existing probe counters; no GPU work or timing inference."""
import hashlib
import json
from pathlib import Path

ROOT = Path('D:/Coding/renderlab-work/p-gie')


def summarize():
    expected = {f'{scene}-{mode}' for scene in ('game-base','quarry-base') for mode in ('rt','nonrt')}
    paths = sorted((ROOT/'runs').glob('*/probe.json'))
    if {path.parent.name for path in paths} != expected:
        raise RuntimeError('need all four scene/path configurations')
    rows = []
    for path in paths:
        samples = json.loads(path.read_text())
        if len(samples) != 88 or {row['frame'] for row in samples} != set(range(88)):
            raise RuntimeError(f'incomplete registered frame plan: {path}')
        if any(row['local_overflow'] for row in samples):
            raise RuntimeError(f'probe table overflow: {path}')
        for phase in sorted({row['phase'] for row in samples}):
            group = [row for row in samples if row['phase'] == phase and row['tiles']]
            if not group:
                continue
            pixels = sum(row['pixels'] for row in group)
            finds = sum(row['finds'] for row in group)
            maximum = max(row['max_cells'] for row in group)
            rows.append(dict(configuration=path.parent.name, phase=phase, frames=len(group),
                max_unique_per_32_pixel_tile=maximum,
                mean_unique_per_nonempty_tile=finds/sum(row['tiles'] for row in group),
                unique_per_32_covered_pixels=32*finds/pixels if pixels else None,
                local_overflow=sum(row['local_overflow'] for row in group),
                reject=maximum>40,
                counters_sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    if not rows:
        raise RuntimeError('no probe statistics')
    result = dict(tile=[8,4], threshold=40, rows=rows,
        reject=any(row['reject'] for row in rows),
        limitation='Probe request union, including bin/level ties and parent/refine rounds; not the exact shading-only set.')
    (ROOT/'gate-summary.json').write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    summarize()
