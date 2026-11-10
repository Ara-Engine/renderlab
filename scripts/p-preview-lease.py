import importlib.util,os,sys,time,subprocess,shutil
from pathlib import Path
root=Path("D:/Coding/renderlab-work/p-preview")
spec=importlib.util.spec_from_file_location("guard",Path(__file__).with_name("p-preview-build.py"));g=importlib.util.module_from_spec(spec);spec.loader.exec_module(g)
label=sys.argv[1]; command=sys.argv[2:]
g.post(f"[p-preview request] owner=p-preview label={label} <=14 min: quiet ARA1/lib verification, >=8 GiB/5 GB, no compiler; Ara first {g.utc()}")
while True:
 pending=g.pending_requests()
 order=g.order()
 if "HOLD:" in (order or ""):raise RuntimeError(order)
 if not pending and not g.SLOT.exists() and not g.GPU_LOCK.exists() and not g.compilers() and g.available_mib()>=8192 and shutil.disk_usage("D:/").free>=5e9:break
 time.sleep(5)
print(order,flush=True)
env=os.environ|{"N8_OWNER":"p-preview","N8_OUT":str(root/"lease")}
code=subprocess.call([sys.executable,"-B","scripts/n8-run.py","--label",label,"--minutes","14","--no-build-contention","--record-resources","--runtime-memory-mbytes","8192","--",*command],env=env)
sys.exit(code)
