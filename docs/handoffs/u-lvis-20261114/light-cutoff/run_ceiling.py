import csv,hashlib,json,os,statistics as st,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parent;OUT=ROOT/'ceiling-runs';OUT.mkdir(exist_ok=False)
EXE=ROOT.parent/'game-facing/after-0428d73c.exe';FOX=Path('D:/Coding/renderlab-work/m7-sk1/Fox.glb')
base={k:v for k,v in os.environ.items() if not k.startswith(('RENDERLAB_','ARA_')) and k not in ('VK_INSTANCE_LAYERS','VK_LAYER_PATH')}
base.update(RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK='I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK',RENDERLAB_LIST_LANES='32',RENDERLAB_LIGHT_VIS_PASS='0',RENDERLAB_SCENE_KEEP_FRAMES='4294967295')
meta={'head':'0428d73c9d4ef35fa847b28fcc3068a030dd3f59','exe_sha256':hashlib.sha256(EXE.read_bytes()).hexdigest(),'fox_sha256':hashlib.sha256(FOX.read_bytes()).hexdigest(),'program':'standard control via existing --diag program=standard; toon hero retained','size':[1920,1080],'motion':'moving','claim':'Hard black bound feasibility screen; not an Ara measurement, not an exact mathematical smooth ray-superset theorem.'}
rows=[];started=time.monotonic()
for round_id in [1,2]:
 for path in ['rt','nonrt']:
  for scene in ['flight','night']:
   for state in (['off','black'] if round_id==1 else ['black','off']):
    assert time.monotonic()-started<177,'CHILD BUDGET STOP'
    name=f'r{round_id}-{path}-{scene}-{state}';out=OUT/name;out.mkdir()
    env={**base,'RENDERLAB_PASS_LOG':str(out/'passes.jsonl')}
    if state=='black':env.update(RENDERLAB_LIGHT_BOUND='black',RENDERLAB_LIGHT_BOUND_CODES='0.5',RENDERLAB_LIGHT_BOUND_SKIP='unshadowed')
    cmd=[str(EXE),'--path',path,'--fox',str(FOX),'--frames','120','--every','1000','--size','1920x1080','--diag','program=standard','--pipeline-cache',str(ROOT/'pipeline-cache'),'--out',str(out)]
    if scene=='night':cmd+=['--night-lanterns']
    (out/'manifest.json').write_text(json.dumps({**meta,'round':round_id,'state':state,'path':path,'scene':scene,'command':cmd,'environment':{k:v for k,v in env.items() if k.startswith(('RENDERLAB_','ARA_'))}},indent=2)+'\n')
    start=time.monotonic();clocks=[];observations=[];stop=threading.Event()
    clock=subprocess.Popen(['nvidia-smi','--query-gpu=timestamp,clocks.gr,clocks.mem,pstate,power.draw,utilization.gpu','--format=csv,noheader,nounits','-lms','200'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,creationflags=subprocess.CREATE_NO_WINDOW)
    def read_clocks():
     for line in clock.stdout:clocks.append({'seconds':time.monotonic()-start,'values':next(csv.reader([line.strip()]))})
    def observe():
     while not stop.wait(.15):
      try:
       lines=(out/'passes.jsonl').read_text().splitlines()
       if lines:observations.append({'seconds':time.monotonic()-start,'frame':json.loads(lines[-1])['frame']})
      except (OSError,ValueError,KeyError):pass
    ct=threading.Thread(target=read_clocks,daemon=True);ct.start();ft=threading.Thread(target=observe,daemon=True);ft.start()
    print('START',name,flush=True)
    try:
     with (out/'run.log').open('w') as log:
      proc=subprocess.Popen(cmd,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
      try:rc=proc.wait(timeout=min(95,max(1,193-(time.monotonic()-started))))
      except subprocess.TimeoutExpired:subprocess.run(['taskkill','/PID',str(proc.pid),'/T','/F'],capture_output=True);raise
    finally:
     stop.set();ft.join(timeout=1);clock.terminate();ct.join(timeout=1)
     (out/'clocks.json').write_text(json.dumps({'columns':['timestamp','graphics_MHz','memory_MHz','pstate','power_W','gpu_percent'],'samples':clocks,'observed_frames':observations})+'\n')
    assert rc==0,(name,rc)
    text=(out/'run.log').read_text(errors='replace').lower();assert not any(s in text for s in ['panicked','validation error','wgpu error']),(name,'start validation failure')
    assert not list(out.glob('*.png')),name
    records=[json.loads(l) for l in (out/'passes.jsonl').read_text().splitlines() if l]
    samples=[r for r in records if 41<=r['frame']<=100];assert len(samples)==60 and len({r['frame'] for r in samples})==60,(name,len(samples))
    assert not any('light_visibility' in r['passes'] for r in samples),name
    row={**meta,'case':name,'round':round_id,'path':path,'scene':scene,'state':state,'samples':60,'renderer_frames':[41,100],'seconds':time.monotonic()-start,'span_ms':st.median(r['span'] for r in samples),'shading_ms':st.median(r['passes']['shading'] for r in samples),'passes_ms':{k:st.median(r['passes'].get(k,0) for r in samples) for k in sorted({k for r in samples for k in r['passes']})}}
    rows.append(row);(OUT/'rows.json').write_text(json.dumps(rows,indent=2)+'\n');print('END',name,'seconds',round(row['seconds'],2),'shading',row['shading_ms'],'span',row['span_ms'],flush=True)
print('COMPLETE',time.monotonic()-started,flush=True)
