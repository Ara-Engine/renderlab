"""CPU-only: reuse P-NRT's planner/summarizer for P-RETIME's batched 88-pose tracks.

Split saved 352-frame reports into four 88-frame reports, retaining frame IDs and
hard-linking PNG witnesses. Existing summaries are reused when source hashes match.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys

REPO = Path(__file__).resolve().parents[1]
ROOT = Path('D:/Coding/renderlab-work/p-retime')
SCENES = {'quarry':'quarry-daylight','lights':'game-lights-unshadowed','game':'game-base'}


def sha(path):
    with path.open('rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()


summaries = {}
for prefix,case,mode in [('a','quarry','rt'),('a','quarry','nonrt'),('a','lights','rt'),
    ('a','lights','nonrt'),('mem','game','nonrt'),('mem','quarry','nonrt')]:
    sources = [ROOT/'runs'/case/mode/f'{i}-{prefix}-{side}'
        for i,side in enumerate(['before','after','after','before'])]
    if not all((p/'result.json').exists() for p in sources):
        if '--partial' in sys.argv: continue
        raise RuntimeError(f'incomplete pair: {prefix} {case} {mode}')
    key = f'{prefix}-{case}-{mode}'
    out = ROOT/'pnrt'/key
    out.mkdir(parents=True,exist_ok=True)
    hashes = {p.name:sha(p/'engine.json') for p in sources}
    manifest = out/'source-hashes.json'
    if manifest.exists():
        assert json.loads(manifest.read_text())==hashes,'completed source reports changed'
    else:
        command = [sys.executable,'-B',str(REPO/'tools/p-nrt/abplan.py'),f'p-retime-{key}',
            '--bin',f'base={ROOT}/bin/{prefix}-before.exe','--bin',f'candidate={ROOT}/bin/{prefix}-after.exe',
            '--reps','8','--cases',SCENES[case],'--no-quality']
        with (out/'abplan.json').open('w') as f:
            subprocess.run(command,cwd=REPO,stdout=f,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
        for index,source in enumerate(sources):
            binary = 'base' if index in [0,3] else 'candidate'
            block = 0 if index in [0,1] else 1
            engine = json.loads((source/'engine.json').read_text())
            assert len(engine['frames'])==352
            for track in range(4):
                destination = out/binary/f'{SCENES[case]}-timing-{block*4+track}'
                destination.mkdir(parents=True)
                frames = [f | dict(index=f['index']-88*track) for f in engine['frames'][88*track:88*(track+1)]]
                ids = {f['frame_id'] for f in frames}
                split = engine | dict(frames=frames,gpu=[g for g in engine['gpu'] if g['frame_id'] in ids])
                (destination/'engine.json').write_text(json.dumps(split))
                for frame in frames:
                    png = source/f"frame-{frame['index']+88*track:04}.png"
                    if png.exists(): (destination/f"frame-{frame['index']:04}.png").hardlink_to(png)
        manifest.write_text(json.dumps(hashes,indent=2))
    if not (out/'summary.json').exists():
        with (out/'summary.txt').open('w') as f:
            subprocess.run([sys.executable,'-B',str(REPO/'tools/p-nrt/summarize.py'),str(out)],
                cwd=REPO,stdout=f,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
    summary = json.loads((out/'summary.json').read_text())
    assert summary['identity']['candidate']['compared']==152
    assert summary['identity']['candidate']['identical']==152
    assert summary['timing'][SCENES[case]]['base']['cut']['span_ms']['n']==8
    summaries[key] = summary
    print(key,'P-NRT: 152/152 PNG witnesses identical; eight complete tracks per binary',flush=True)
(ROOT/'pnrt/summary-index.json').write_text(json.dumps(summaries,indent=2))
