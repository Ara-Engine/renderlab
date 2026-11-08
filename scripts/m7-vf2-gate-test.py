"""CPU-only checks of VF2's actual lease gate; never reads the live schedule or locks."""
import importlib.util
import json
from pathlib import Path

spec=importlib.util.spec_from_file_location("vf2",Path(__file__).with_name("m7-vf2.py"))
vf2=importlib.util.module_from_spec(spec)
spec.loader.exec_module(vf2)
root=Path("D:/Coding/renderlab-work/vf2/gate-test")
root.mkdir(parents=True,exist_ok=True)
vf2.SCHEDULE=root/"schedule.md"
vf2.g.OUT=root
order="[root order] MV1 first; hold for FINAL release"
config=root/"gpu-after.json"
config.write_text(json.dumps({"acknowledged_order":order,"after":[{"line_prefix":"m7-mv1 FINAL release"}]}))
try:
    for text,expected in [
        ("",False),
        ("m7-mv1 FINAL release without a root directive",False),
        (order+"\nm7-mv1 integration-k2-1 window released",False),
        (order+"\nnote: m7-mv1 FINAL release",False),
        ("m7-mv1 FINAL release old\n"+order,False),
        (order+"\nm7-mv1 FINAL release 2026-11-08T23:00:00Z",True),
        (order+"\nm7-mv1 FINAL release\n[root order] new order",False),
    ]:
        vf2.SCHEDULE.write_text(text)
        assert vf2.order_ready()==expected,(text,expected)
    print("7 lease-gate cases passed; no GPU or live schedule access")
finally:
    vf2.SCHEDULE.unlink()
    config.unlink()
    root.rmdir()
