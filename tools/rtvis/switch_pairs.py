"""Pass switches in pairs. The engine reads each pass switch once per process, so one test process
cannot compose two settings; this runs the composition tests once per switch and once per pair
(and once with none), and optionally one game_scene frame per pair on both paths.

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
    "sun-zone": {"RENDERLAB_SUN_ZONE_STOP": "1"},
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
    yield "none", {}
    for name, env in SWITCHES.items():
        yield name, env
    for a, b in itertools.combinations(SWITCHES, 2):
        if a.startswith("rl") and b.startswith("rl"):
            continue  # the reflection variants are one switch's settings, not a pair
        yield f"{a}+{b}", {**SWITCHES[a], **SWITCHES[b]}


def main():
    tests = sys.argv[1]
    game = sys.argv[2] if len(sys.argv) > 3 else None
    out = sys.argv[3] if len(sys.argv) > 3 else None
    base = {k: v for k, v in os.environ.items()
            if not any(k in env for env in SWITCHES.values())}
    failed = 0
    for name, env in sets():
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
