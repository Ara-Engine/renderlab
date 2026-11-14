"""Existing P202 switch split; measurement only, called inside n8 timing lease."""
import csv,hashlib,json,os,shutil,subprocess,threading,time
from pathlib import Path
import joinlib as J
ROOT=Path(__file__).resolve().parent
META=json.loads((ROOT/'copy-manifest.json').read_text())
BIN=ROOT/'bin-P202';RUN=ROOT/'split-runs';RUN.mkdir(exist_ok=False)
base={k:v for k,v in os.environ.items() if not k.startswith(('RENDERLAB_','ARA_')) and k not in ('VK_INSTANCE_LAYERS','VK_LAYER_PATH')}
base.update(PATH=str(BIN)+os.pathsep+base['PATH'],ARA_CONFIG_DIR=str(ROOT/'profile/config'),ARA_DOTNET_ARTIFACTS=str(ROOT/'managed'),ARA_ALLOW_STALE_COMPANIONS='1',ARA_VIDEO_KEEP='999999',RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK='I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK',RENDERLAB_LIST_LANES='32')
rows=[];start=time.monotonic()
for round_id in [1,2]:
 for scene in ['showcase','outpost']:
  for state in (['A','B'] if round_id==1 else ['B','A']):
   assert time.monotonic()-start<177,'CHILD BUDGET STOP'
   name=f'r{round_id}-{scene}-{state}';out=RUN/name;out.mkdir()
   project=ROOT/'projects'/scene
   assert hashlib.sha256((project/'content/levels'/f'{scene}.scene').read_bytes()).hexdigest()==META['scenes'][scene]['sha256']
   env={**base,'RENDERLAB_PASS_LOG':str(out/'passes.jsonl'),'ARA_RENDERLAB_CPU_LOG':str(out/'cpu.jsonl'),'ARA_CRASH_DIR':str(out/'crashes')}
   if state=='B':env['RENDERLAB_SUN_PASS']='1'
   assert 'RENDERLAB_SUN_ZONE_STOP' not in env and 'RENDERLAB_PASS_LOG_DETAIL' not in env
   cmd=[str(BIN/'ara.exe'),'capture','--scene',f'levels/{scene}.scene','--camera','Follow camera' if scene=='outpost' else 'Camera','--path',str(out/'capture.png'),'--width','1920','--height','1080','--ev100','7.8' if scene=='outpost' else '10.17','--stability','590' if scene=='outpost' else '400','--video','120','--video-move','4,0,-4' if scene=='outpost' else '2,0,0','--video-yaw','12' if scene=='outpost' else '20']
   (out/'manifest.json').write_text(json.dumps(dict(**META,command=cmd,environment={k:v for k,v in env.items() if k.startswith(('ARA_','RENDERLAB_'))},no_cut=True),indent=2)+'\n')
   clocks=[];beg=time.monotonic()
   clock=subprocess.Popen(['nvidia-smi','--query-gpu=timestamp,clocks.gr,clocks.mem,pstate,power.draw,utilization.gpu','--format=csv,noheader,nounits','-lms','200'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,creationflags=subprocess.CREATE_NO_WINDOW)
   def read_clock():
    for line in clock.stdout:clocks.append({'seconds':time.monotonic()-beg,'values':next(csv.reader([line.strip()]))})
   ct=threading.Thread(target=read_clock,daemon=True);ct.start()
   print('START',name,flush=True)
   try:
    with (out/'run.log').open('w') as log:
     p=subprocess.Popen(cmd,cwd=project,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
     try:rc=p.wait(timeout=min(60,max(1,193-(time.monotonic()-start))))
     except subprocess.TimeoutExpired:
      subprocess.run(['taskkill','/PID',str(p.pid),'/T','/F'],capture_output=True);raise
   finally:
    clock.terminate();ct.join(timeout=1);(out/'clocks.json').write_text(json.dumps(clocks)+'\n')
   for f in (project/'.ara/play').glob('*.log'):shutil.copy2(f,out/('play-'+f.name))
   assert rc==0,(name,rc)
   errorlines=[]
   for f in out.glob('*.log'):
    errorlines += [l for l in f.read_text(errors='replace').splitlines() if any(x in l.lower() for x in ['panicked at','validation error','wgpu error'])]
   assert not errorlines,(name,errorlines)
   j=J.load(name,cpu=J.jl(out/'cpu.jsonl'),passes=J.jl(out/'passes.jsonl'));assert j and J.anchor_ok(j),(name,'join anchor failed')
   cpu=j['cpu'];end=len(cpu)-200
   for motion,window in [('held',cpu[end-60:end]),('moving',cpu[-120:][20:60])]:
    found=[j['passes'][x['frame']+j['offset']] for x in window if x['frame']+j['offset'] in j['passes']]
    assert len(found)==len(window) and len(found)>0,(name,motion,len(found),len(window))
    assert all(('sun_shadow' in r['passes'])==(state=='B') for r in found),(name,'sun split missing')
    med=__import__('statistics').median
    row=dict(case=name,scene=scene,scene_sha256=META['scenes'][scene]['sha256'],player_sha256=META['player_sha256'],state=state,round=round_id,motion=motion,offset=j['offset'],anchor_ok=J.anchor_ok(j),coverage=f'{len(found)}/{len(window)}',cpu_frames=[x['frame'] for x in window],renderer_frames=[r['frame'] for r in found],span_ms=med(r['span'] for r in found),visibility_ms=J.pstat(found,'visibility'),sun_ms=J.pstat(found,'sun_shadow'),visibility_plus_sun_ms=med(r['passes'].get('visibility',0)+r['passes'].get('sun_shadow',0) for r in found),tlas_ms=J.pstat(found,'tlas'),passes_ms={k:J.pstat(found,k) for k in sorted({k for r in found for k in r['passes']})})
    rows.append(row)
   (RUN/'rows.json').write_text(json.dumps(rows,indent=2)+'\n')
   assert not list((out/'capture.video').glob('*.png')),'VIDEO PNG READBACK'
   print('END',name,'seconds',round(time.monotonic()-beg,2),'cpu',len(cpu),'rows',json.dumps(rows[-2:]),flush=True)
print('COMPLETE',time.monotonic()-start,flush=True)
