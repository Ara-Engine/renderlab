"""Run only inside a root-approved ordinary GPU lease. Identity MUST precede dump mode."""
from pathlib import Path
import argparse, hashlib, json, os, subprocess, time

unit = Path('D:/Coding/renderlab-work/u-sunrt')
base = unit/'outpost-pass-cost'
binaries = base/'ara-joint'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('mode', choices=('identity','dump'))
parser.add_argument('--out', type=Path, required=True)
parser.add_argument('--candidate-bin', type=Path, required=True, help='Owned copy of the supplied shared binary directory')
parser.add_argument('--control-bin', type=Path, required=True, help='Owned copy of bin-P203')
parser.add_argument('--provenance', type=Path, required=True, help='Shared binary source heads and default-off memory-switch manifest')
args = parser.parse_args()
args.out.mkdir(parents=True)
provenance = json.loads(args.provenance.read_text())
source_label = 'main d9388afa plus two default-off memory measurement switches (t3/fixed-memreport-m)'
candidate_sha = hashlib.sha256((args.candidate_bin/'ara-player.exe').read_bytes()).hexdigest()
expected_scenes = dict(outpost='f3583f2f9cb02a5fdae641214db4e9e89792f8f3185200708d92a5b760d2e386',
                       showcase='ac84a12aa4a3dd4c8979140fab169db080b4077e4f83c4fb1fecb33019921165')
if args.mode == 'dump':
    gate = json.loads((binaries/'identity-pass.json').read_text())
    assert gate['candidate_player_sha256'] == candidate_sha

def digest(path):
    with path.open('rb') as source: return hashlib.file_digest(source,'sha256').hexdigest()

rows = []
for scene in ('outpost','showcase'):
    for mode in (('control','candidate') if args.mode == 'identity' else ('candidate',)):
        binary = args.control_bin if mode == 'control' else args.candidate_bin
        project = base/'projects'/scene
        dest = args.out/(scene+'-'+mode)
        dest.mkdir()
        scene_file = project/'content/levels'/f'{scene}.scene'
        assert digest(scene_file) == expected_scenes[scene]
        # Same existing C# companion and scene setup on both sides; no managed build.
        managed = 'D:/Coding/renderlab-work/ara-pin/tree/dotnet/artifacts/bin' if scene=='outpost' else 'D:/Coding/renderlab-work/ara-pr169/tree/dotnet/artifacts/bin'
        env = {k:v for k,v in os.environ.items() if not k.startswith(('ARA_','RENDERLAB_')) and k not in ('VK_INSTANCE_LAYERS','VK_LAYER_PATH')}
        env.update(PATH=str(binary)+os.pathsep+env['PATH'], ARA_BIN=str(binary),
                   ARA_DOTNET_ARTIFACTS=managed, ARA_ALLOW_STALE_COMPANIONS='1',
                   ARA_RENDERER='renderlab-nonrt', ARA_CONFIG_DIR=str(dest/'config'),
                   ARA_CRASH_DIR=str(dest/'crashes'), ARA_CAPTURE_OUTPUT_DIR=str(dest/'capture-output'))
        if args.mode == 'dump':
            env.update(RENDERLAB_RL_PASS='1', RENDERLAB_RL_DEFERRED='1',
                       RENDERLAB_REFLECTION_GEOMETRY_DUMP=str(dest/'dumps'),
                       RENDERLAB_REFLECTION_GEOMETRY_DUMP_FRAME='30',
                       RENDERLAB_RL_PROBE=str(dest/'probe30.json'),
                       RENDERLAB_RL_ROOM_LOG=str(dest/'room.jsonl'))
        cmd = [str(binary/'ara.exe'),'--root',str(project),'capture','--scene',f'levels/{scene}.scene',
               '--camera','Follow camera' if scene=='outpost' else 'Camera',
               '--path',str(dest/'frame.png'),'--width','1920','--height','1080','--marks','0',
               '--ev100','7.8' if scene=='outpost' else '10.17','--stability','40']
        row = dict(scene=scene, scene_file=str(scene_file), scene_sha256=digest(scene_file),
                   shared_binary_provenance=provenance, candidate_source_label=source_label,
                   mode=mode, command=cmd, player_sha256=digest(binary/'ara-player.exe'),
                   environment={k:v for k,v in env.items() if k.startswith(('ARA_','RENDERLAB_'))})
        (dest/'manifest.json').write_text(json.dumps(row,indent=2)+'\n')
        start = time.monotonic()
        with (dest/'capture.log').open('w',encoding='utf-8') as log:
            child = subprocess.Popen(cmd,cwd=project,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
            try: code = child.wait(timeout=150)
            except subprocess.TimeoutExpired:
                subprocess.run(['taskkill','/PID',str(child.pid),'/T','/F'],capture_output=True)
                code = 124
        row.update(exit=code,seconds=time.monotonic()-start)
        if code == 0: row['png_sha256'] = digest(dest/'frame.png')
        rows.append(row)
        (args.out/'rows.json').write_text(json.dumps(rows,indent=2)+'\n')
        print(scene,mode,code,row['seconds'],flush=True)
        if code: raise SystemExit(code)
        if args.mode == 'identity' and mode == 'candidate':
            if rows[-1]['png_sha256'] != rows[-2]['png_sha256']:
                raise SystemExit('IDENTITY MISMATCH: stop; no dump turn may follow')
        if args.mode == 'dump':
            lists = list((dest/'dumps').glob('*.glossy.json'))
            if len(lists) != 1: raise SystemExit('expected exactly one glossy snapshot')
            stem = str(lists[0]).removesuffix('.glossy.json')
            geometry = Path(stem+'.json')
            gm, lm = json.loads(geometry.read_text()), json.loads(lists[0].read_text())
            assert gm['frame'] == lm['frame'] == 30
            assert gm['view'] == lm['view'] and gm['generation'] == lm['generation']
            assert json.loads((dest/'probe30.json').read_text())['frame'] == 30
if args.mode == 'identity':
    gate = dict(candidate_player_sha256=candidate_sha,
                comparisons=rows, result='both scene PNG hashes equal')
    (binaries/'identity-pass.json').write_text(json.dumps(gate,indent=2)+'\n')
print(args.mode.upper()+'_PASS',flush=True)
