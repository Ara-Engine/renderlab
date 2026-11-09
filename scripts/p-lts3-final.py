"""Only unresolved C gates, inside the 8 GiB compiler-quiet P-LTS3 lease."""
from pathlib import Path
import subprocess
import sys
ROOT=Path("D:/Coding/renderlab-work/p-lts3")
label=sys.argv[1]
commands=[
 ["scripts/p-lts3-batch.py",label+"-timing","--binary",str(ROOT/"bin/c-capture.exe"),"--baseline-binary",str(ROOT/"bin/base-capture.exe"),"--steady-warmup","128"],
 ["scripts/p-lts3-batch.py",label+"-boundary","--binary",str(ROOT/"bin/c-capture.exe"),"--baseline-binary",str(ROOT/"bin/base-capture.exe"),"--scene-dir",str(ROOT/"boundary-scenes"),"--cases","lights-128,lights-129"],
 ["scripts/p-lts3-fixtures.py",label+"-fixtures","--candidate",str(ROOT/"bin/c-tests.exe"),"--game",str(ROOT/"bin/c-game.exe"),"--steps","mixed,game-still,game-move,quarry"],
]
for i,command in enumerate(commands):
 print("STEP",i,command,flush=True)
 with (ROOT/f"{label}-step-{i}.log").open("w") as log:
  result=subprocess.run([sys.executable,"-B",*command],stdout=log,stderr=subprocess.STDOUT,creationflags=subprocess.CREATE_NO_WINDOW)
 print((ROOT/f"{label}-step-{i}.log").read_text()[-1800:],flush=True)
 if result.returncode: raise SystemExit(result.returncode)
