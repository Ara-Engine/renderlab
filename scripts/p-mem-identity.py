"""Compare complete ARA1/RF/Q-walk captures; exits nonzero on a missing or changed byte hash.

py -3 -B scripts/p-mem-identity.py BEFORE_DIR AFTER_DIR REPORT.json
"""
import hashlib
import json
from pathlib import Path
import sys


def read(path):
    return json.loads(path.read_text())


def main():
    before, after, report = map(Path, sys.argv[1:])
    checks, evidence = [], {}

    def compare(scope, a, b):
        same = a == b and bool(a)
        checks.append(dict(scope=scope, identical=same, entries=len(a)))
        evidence[scope] = a if same else dict(before=a, after=b)
        print(scope, "IDENTICAL" if same else "FAILED", len(a))

    required = {f"{scene}-{mode}" for scene in
                ["game-base", "ara-pbr-oriented", "game-lights", "quarry-daylight"]
                for mode in ["rt", "nonrt"]}
    cases = required | {p.parent.name for root in [before, after]
                        for p in root.glob("*/memory.json")}
    for case in sorted(cases):
        a, b = before / case, after / case
        if not all((p / "hashes.json").exists() for p in [a, b]):
            checks.append(dict(scope=case, identical=False, reason="capture missing"))
            continue
        x, y = read(a / "hashes.json"), read(b / "hashes.json")
        compare(case + "/png", {k:v for k,v in x.items() if k.endswith(".png")},
                {k:v for k,v in y.items() if k.endswith(".png")})
        compare(case + "/hdr", x.get("hdr", {}), y.get("hdr", {}))
        x, y = read(a / "engine.json"), read(b / "engine.json")
        def configuration(root, engine):
            result = {k:engine[k] for k in ["size", "path", "settings", "effective_indirect"]}
            spec = json.dumps(read(root / "scene.json"), sort_keys=True, separators=(",", ":"))
            result["capture_spec_sha256"] = hashlib.sha256(spec.encode()).hexdigest()
            return result
        compare(case + "/configuration", configuration(a, x), configuration(b, y))
    for mode in ["rt", "nonrt"]:
        for suite in ["rf", "post", "lc2"]:
            case = f"{suite}-{mode}"
            a, b = before / case / "hashes.json", after / case / "hashes.json"
            if a.exists() and b.exists(): compare(case, read(a), read(b))
            else: checks.append(dict(scope=case, identical=False, reason="required capture missing"))
        case = "qwalk-" + mode
        a, b = before / case / "tr1.json", after / case / "tr1.json"
        if a.exists() and b.exists():
            x, y = read(a), read(b)
            compare(case, x["variants"]["native"]["hashes"], y["variants"]["native"]["hashes"])
            compare(case + "/frames", x["frames"], y["frames"])
        else: checks.append(dict(scope=case, identical=False, reason="required capture missing"))
        for suite in ["post"]:
            a, b = before / f"{suite}-{mode}" / "bright-sky/timing.json", after / f"{suite}-{mode}" / "bright-sky/timing.json"
            if a.exists() and b.exists():
                compact = lambda p: {k: {h:v[h] for h in ["hdr_sha256", "display_sha256"]} for k,v in read(p).items()}
                compare(f"{suite}-{mode}/hdr-display", compact(a), compact(b))
            else: checks.append(dict(scope=f"{suite}-{mode}/hdr-display", identical=False,
                                     reason="required capture missing"))

    passed = bool(checks) and all(c["identical"] for c in checks)
    encoded = json.dumps(evidence, sort_keys=True, separators=(",", ":")).encode()
    report.write_text(json.dumps(dict(all_identical=passed, checks=checks,
        evidence_sha256=hashlib.sha256(encoded).hexdigest(), hashes=evidence), indent=2))
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
