import os,sys,json,subprocess,time,hashlib
from pathlib import Path
root=Path("D:/Coding/renderlab-work/p-preview")
binary=max((root/"target/release/deps").glob("renderlab-*.exe"),key=lambda p:p.stat().st_mtime)
binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest()
mode=sys.argv[1]
cases={
 "screen":[("rt","before",None),("nonrt","before",None),("rt","opt","-O"),("nonrt","opt","-O"),("rt","ssa","--ssa-rewrite"),("rt","local","--eliminate-local-single-block --eliminate-local-single-store --ssa-rewrite --ccp --simplify-instructions --eliminate-dead-code-aggressive"),("rt","inline","--eliminate-dead-branches --merge-return --inline-entry-points-exhaustive --eliminate-dead-functions --eliminate-local-single-block --eliminate-local-single-store --ssa-rewrite --eliminate-dead-code-aggressive"),("rt","scalar","--scalar-replacement=0 --convert-local-access-chains --eliminate-local-single-block --eliminate-local-single-store --ssa-rewrite --eliminate-dead-code-aggressive")],
 "lib":[("rt","lib",None)]
}[mode]
for path,label,passes in cases:
 out=root/"runs"/mode/(path+"-"+label);out.mkdir(parents=True,exist_ok=True)
 marker=out/"process.json"
 if marker.exists():
  prior=json.loads(marker.read_text())
  if prior.get("binary_sha256")==binary_sha256 and prior["exit"]==0:
   print("reusing completed",path,label,flush=True);continue
 env=os.environ|{"RENDERLAB_ES1_PATH":path,"RENDERLAB_SHC2_OUT":str(out),"RENDERLAB_CC1_PROFILE":"1","RENDERLAB_RELOAD_PROFILE":"1","RENDERLAB_SPIRV_OPT":"C:/VulkanSDK/1.4.363.0/Bin/spirv-opt.exe","RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK":"I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK","WGPU_DEBUG":"0","WGPU_VALIDATION":"0"}
 env.pop("RENDERLAB_PREVIEW",None);env.pop("RENDERLAB_PREVIEW_PASSES",None)
 if passes:env.update(RENDERLAB_PREVIEW="1",RENDERLAB_PREVIEW_PASSES=passes)
 for key in ("VK_INSTANCE_LAYERS","VK_LAYER_ENABLES"):env.pop(key,None)
 args=[str(binary),"gpu_shc_mixed_reload","--ignored","--nocapture","--test-threads=1"] if label!="lib" else [str(binary),"--test-threads=2"]
 start=time.monotonic()
 with (out/"run.log").open("w") as log:
  child=subprocess.Popen(args,env=env,stdout=log,stderr=subprocess.STDOUT,creationflags=0x08000000)
  try: code=child.wait(timeout=300)
  except subprocess.TimeoutExpired:
   subprocess.run(["taskkill","/PID",str(child.pid),"/T","/F"],capture_output=True);code=124
 (out/"process.json").write_text(json.dumps(dict(exit=code,seconds=time.monotonic()-start,binary_sha256=binary_sha256)))
 print(path,label,code,round(time.monotonic()-start,3),flush=True)
 if code:sys.exit(code)
