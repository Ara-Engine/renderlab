"""Per-pass table from the Unreal ProfileGPU dumps (CPU only). Rows are the children of `Scene`
plus the children of RenderDeferredLighting's two groups; columns are still (frame 64, mean of
the three launches), walking (190), the cut frame (320) and the frame after (321)."""
import json
import sys
from pathlib import Path

ROOT = Path("D:/Coding/renderlab-work/p-yard2/ue/quarry")
OUT = ROOT / "passes-table.json"


def tree(rows):
    depths = sorted({r["depth"] for r in rows})
    out, stack = {}, []
    for r in rows:
        level = depths.index(r["depth"])
        del stack[level:]
        name = r["name"].split("(")[0].split(":")[0].strip() if r["name"].startswith(("TemporalSuperResolution", "LumenSceneUpdate")) else r["name"]
        stack.append(name)
        out.setdefault(tuple(stack), 0.0)
        out[tuple(stack)] += r["inclusive_ms"]
    return out


def under(t, parent_tail, extra=0):
    """Inclusive ms of each child of the first path ending in parent_tail."""
    parents = [k for k in t if k[-len(parent_tail):] == parent_tail]
    if not parents:
        return {}
    p = min(parents, key=len)
    return {k[len(p):][0]: v for k, v in t.items() if k[:len(p)] == p and len(k) == len(p) + 1}


result = {}
for config in ("ue-sw-e", "ue-hw-e"):
    cols = {}
    stills = []
    for frame, label in ((190, "walk"), (320, "cut"), (321, "cut+1")):
        dumps = json.loads((ROOT / f"profile-{frame}" / config / "passes.json").read_text())["dumps"]
        full = [d for d in dumps if len(d) > 50]
        stills.append(tree(full[0]))
        cols[label] = tree(full[1])
    keys = set().union(*stills)
    cols["still"] = {k: sum(s.get(k, 0.0) for s in stills) / len(stills) for k in keys}
    cols["still_range"] = {k: [min(s.get(k, 0.0) for s in stills), max(s.get(k, 0.0) for s in stills)] for k in keys}
    table = {}
    for label in ("still", "walk", "cut", "cut+1"):
        t = cols[label]
        rows = {"frame total": next(v for k, v in t.items() if len(k) == 1)}
        for name, v in under(t, ("Scene",)).items():
            rows[name] = v
        for group in ("DiffuseIndirectAndAO", "Lights"):
            for name, v in under(t, ("RenderDeferredLighting", group)).items():
                rows[f"  {group} / {name}"] = v
        for name, v in under(t, ("PostProcessing",)).items():
            rows[f"  PostProcessing / {name}"] = v
        table[label] = rows
    root = [k for k in cols["still_range"] if len(k) == 1][0]
    result[config] = dict(table=table, still_total_range=cols["still_range"][root])
    names = list(dict.fromkeys(n for label in table for n in table[label]))
    print(f"== {config}   still total range over three launches {cols['still_range'][root]}")
    print(f"{'pass':58s} {'still':>7s} {'walk':>7s} {'cut':>7s} {'cut+1':>7s}")
    for n in names:
        v = [table[l].get(n, 0.0) for l in ("still", "walk", "cut", "cut+1")]
        if max(v) >= 0.04:
            print(f"{n[:58]:58s} " + " ".join(f"{x:7.3f}" for x in v))
OUT.write_text(json.dumps(result, indent=1))
