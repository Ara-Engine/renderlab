"""Pass switches in pairs. The engine reads each pass switch once per process, so one test process
cannot compose two settings; this runs the composition tests once with none, once per switch, per
pair, per triple and with every switch at once, and optionally one game_scene frame per set on both
paths. SWITCH_PAIRS_SETS=a,b,... runs only the named sets.

    python switch_pairs.py TESTS_EXE                       # composition tests only (CPU)
    python switch_pairs.py TESTS_EXE GAME_SCENE_EXE OUTDIR  # plus one frame per pair and path

TESTS_EXE is the renderlab lib test binary (on the PC, a renamed copy outside target/, run
outside a lease only for these CPU tests). Exit status 1 if any run fails."""
import itertools
import os
import subprocess
import sys

SWITCHES = {
    "gi": {"RENDERLAB_GI_PASS": "1"},
    "material": {"RENDERLAB_MATERIAL_PASS": "1"},
    "sun": {"RENDERLAB_SUN_PASS": "1"},
    "lvis": {"RENDERLAB_LIGHT_VIS_PASS": "1"},
    "rl": {"RENDERLAB_RL_PASS": "1", "RENDERLAB_RL_DEFERRED": "1"},
    "rl-diag": {"RENDERLAB_RL_PASS": "1", "RENDERLAB_RL_DEFERRED": "diag"},
    "rl-all": {"RENDERLAB_RL_PASS": "1", "RENDERLAB_RL_DEFERRED": "diag",
               "RENDERLAB_RL_HIT_OWN_PROGRAM": "1", "RENDERLAB_RL_HIT": "missblack,hitblack,lone"},
}
TESTS = [
    "every_list_head_module_validates",
    "material_pass_variants_compose_and_validate",
    "hit_modules_validate_on_both_heads",
    "pass_modules_validate_on_both_heads",
    "pass_switch_seams_hold",
    "the_primary_shading_layout_has_no_clashing_bindings",
    "receiver_and_shading_validate_both_paths",
]


def sets():
    """No switches, each switch, every pair, every triple and every switch at once. The
    reflection settings (rl, rl-diag, rl-all) are one switch's values, so a set takes at most one."""
    yield "none", {}
    for n in (1, 2, 3):
        for names in itertools.combinations(SWITCHES, n):
            if sum(x.startswith("rl") for x in names) > 1:
                continue
            yield "+".join(names), {k: v for x in names for k, v in SWITCHES[x].items()}
    others = [x for x in SWITCHES if not x.startswith("rl")]
    yield "all-but-rl", {k: v for x in others for k, v in SWITCHES[x].items()}
    for rl in [x for x in SWITCHES if x.startswith("rl")]:
        yield f"all+{rl}", {k: v for x in [*others, rl] for k, v in SWITCHES[x].items()}


def main():
    tests = sys.argv[1]
    game = sys.argv[2] if len(sys.argv) > 3 else None
    out = sys.argv[3] if len(sys.argv) > 3 else None
    base = {k: v for k, v in os.environ.items()
            if not any(k in env for env in SWITCHES.values())}
    failed = 0
    exact = os.environ.get("SWITCH_PAIRS_SETS")
    for name, env in sets():
        if exact and name not in exact.split(","):
            continue
        r = subprocess.run([tests, *TESTS], env={**base, **env}, capture_output=True, text=True)
        summary = [line for line in r.stdout.splitlines() if line.startswith("test result")]
        ok = r.returncode == 0
        failed += not ok
        print(f"tests {name}: {'ok' if ok else 'FAIL'} {summary[-1] if summary else r.stderr[-300:]}")
        if not ok:
            for line in r.stdout.splitlines():
                if "panicked" in line or "FAILED" in line:
                    print("    " + line[:300])
        if game:
            for path in ("nonrt", "rt"):
                d = os.path.join(out, f"{name}-{path}")
                os.makedirs(d, exist_ok=True)
                g = subprocess.run([game, "--path", path, "--frames", "2", "--out", d],
                                   env={**base, **env}, capture_output=True, text=True, timeout=300)
                bad = g.returncode != 0 or any(w in g.stderr for w in ("panicked", "Validation Error", "Error:"))
                failed += bad
                tail = "" if not bad else " " + g.stderr.strip().splitlines()[-1][:300]
                print(f"frame {name} {path}: {'FAIL' if bad else 'ok'}{tail}")
    print(f"{'all passed' if failed == 0 else f'{failed} failed'}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
