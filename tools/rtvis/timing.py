"""U-RTVIS timing of the RT visibility pass (docs/rtvis.md): one lease, one game_scene binary.

    py -3 -B tools/rtvis/timing.py run GAME_EXE OUT [PLAN]   # inside a lease
    py -3 -B tools/rtvis/timing.py report OUT                # CPU

Each lease opens with the reference run: the default game scene on RT at 1080p (120 frames);
its median frame span is recorded and compared with REFERENCE_MS once that is set (more than
10 % off: the lease is not a measurement). PLAN `repro` (step 1) then walks `--diag forest=1`
(and the field walk) with a camera cut at frame 24, interleaving the default shadow budget
with an 8-ray one, the sun off, a high sun and 720p. Every run logs RENDERLAB_PASS_LOG with
its `visibility.*` parts. PLAN `cold` (step 3) times the forest walk's cut with the cold probe
stop on and off (RENDERLAB_COLD_PROBES), twice, interleaved. PLAN `pictures` writes every
frame of short single-cut clips with the stop on and off: the forest held still (cut at 24,
settled before and after), the forest walk, and the game scene's orbit jump on both paths.
`report` prints per run the median frame span, `visibility` and its parts over the moving
frames (8-22 and 40-119), and the frames from 22 to 30 around the cut; `hashes` compares the
picture runs' per-frame hashes, on against off.
"""
import json
import os
import statistics as st
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
WORK = Path("D:/Coding/renderlab-work/unit-rtvis")
# The default game scene on RT at 1080p, frame span median (ms): this branch's references of
# 2026-11-13 read 61.6-61.8 (root's bar; the span is CPU-bound, cpu.wait about 0.03 ms).
REFERENCE_MS = 61.7
FOX = "D:/Coding/renderlab-work/m7-sk1/Fox.glb"
# Held-budget runs (step 5): main's cold probes on, the held stop on (1) or off (0).
HELD = lambda v: {"RENDERLAB_COLD_PROBES": "1", "RENDERLAB_HELD_PROBES": v}
LOW = ["--hour", "17.651"]
CUT = ["--diag", "cut=1"]
FOREST = ["--diag", "forest=1", *CUT]
EIGHT = ["--diag", "shadow-budget=256,64,8,8,16"]
PLANS = {
    "repro": [
        ("forest-low-1", FOREST + LOW, {}),
        ("forest-low-b8-1", FOREST + LOW + EIGHT, {}),
        ("forest-low-nosun", FOREST + LOW + ["--diag", "sun=0"], {}),
        ("forest-noon", FOREST + ["--time", "noon"], {}),
        ("forest-low-720", FOREST + LOW + ["--size", "1280x720"], {}),
        ("field-low", ["--diag", "field=1", *CUT, *LOW], {}),
        ("field-low-opaque", ["--diag", "field=1", *CUT, *LOW], {"RENDERLAB_DIAG_ALL_OPAQUE": "1"}),
        ("forest-low-2", FOREST + LOW, {}),
        ("forest-low-b8-2", FOREST + LOW + EIGHT, {}),
    ],
    "cold": [
        (f"forest-low-cold{v}-{k}", FOREST + LOW, {"RENDERLAB_COLD_PROBES": v})
        for k in (1, 2) for v in ("1", "0")
    ],
    "pictures": [
        (f"{name}-cold{v}", extra, {"RENDERLAB_COLD_PROBES": v})
        for name, extra in [
            ("p-forest-still", FOREST + LOW + ["--still", "--frames", "60", "--every", "1"]),
            ("p-forest-walk", FOREST + LOW + ["--frames", "40", "--every", "1"]),
            ("p-game-rt", CUT + ["--frames", "40", "--every", "1"]),
            ("p-game-nonrt", CUT + ["--path", "nonrt", "--frames", "40", "--every", "1"]),
        ]
        for v in ("1", "0")
    ],
    # The forest walk and the game flight (the Fox walking, the crate spinning), moving frames.
    "held": [
        (f"{name}-held{v}-{k}", extra, HELD(v))
        for k in (1, 2)
        for name, extra in [
            ("forest", ["--diag", "forest=1", *LOW]),
            ("game", ["--fox", FOX]),
        ]
        for v in ("1", "0")
    ],
    # Every frame of: the forest walk; the game flight; the game scene with the camera held at
    # its first pose while the Fox and the crate move (their shadows sweep ground whose history
    # has saturated); and the game flight on non-RT (byte identity).
    "heldpics": [
        (f"{name}-held{v}", extra, HELD(v))
        for name, extra in [
            ("p-forest-walk", ["--diag", "forest=1", *LOW, "--frames", "60", "--every", "1"]),
            ("p-game-flight", ["--fox", FOX, "--frames", "60", "--every", "1"]),
            ("p-game-hold", ["--fox", FOX, "--diag", "hold=1", "--frames", "60", "--every", "1"]),
            ("p-game-nonrt", ["--fox", FOX, "--path", "nonrt", "--frames", "60", "--every", "1"]),
        ]
        for v in ("1", "0")
    ],
}
# GI cut refill (docs/design-gi-cut-refill.md): RENDERLAB_GI cut_refill off (0) or on (1), with
# main's cold and held stops. The game flight cuts with an orbit jump, the forest walk by
# turning 150 degrees in place (`cut=turn`); both call Engine::cut_view at frame 24.
REFILL = lambda v: {"RENDERLAB_GI": f"cut_refill={v}", "RENDERLAB_COLD_PROBES": "1", "RENDERLAB_HELD_PROBES": "1"}
REFILL_CLIPS = [
    ("nonrt-game", ["--path", "nonrt", "--diag", "cut=1"]),
    ("nonrt-forest", ["--path", "nonrt", "--diag", "forest=1", "--diag", "cut=turn", *LOW]),
    ("rt-game", ["--diag", "cut=1"]),
]
PLANS["refill"] = [
    (f"{name}-refill{v}-{k}", extra, REFILL(v))
    for k in (1, 2)
    for name, extra in REFILL_CLIPS
    if k == 1 or name.startswith("nonrt")
    for v in ("1", "0")
]
# The non-RT GI gather study (docs/rtvis.md): the forest walk's moving frames (no cut) at the
# defaults, with fewer rays per gather, with a quarter of the gather budget, and without the
# foliage, against the game flight; `-n` runs also log the GI cache's counters (a blocking
# readback, so they are not timed).
NONRT_FOREST = ["--path", "nonrt", "--diag", "forest=1", *LOW, "--frames", "60"]
PLANS["gather"] = [
    ("g-forest", NONRT_FOREST, {}),
    ("g-forest-stride4", NONRT_FOREST, {"RENDERLAB_GI": "gather_stride=4"}),
    ("g-forest-cells8k", NONRT_FOREST, {"RENDERLAB_GI": "gather_cells=8192"}),
    ("g-bare", ["--path", "nonrt", "--diag", "forest=bare", *LOW, "--frames", "60"], {}),
    ("g-game", ["--path", "nonrt", "--frames", "60"], {}),
    ("g-forest-n", NONRT_FOREST, {"RTVIS_GI_LOG": "1"}),
    ("g-bare-n", ["--path", "nonrt", "--diag", "forest=bare", *LOW, "--frames", "60"], {"RTVIS_GI_LOG": "1"}),
    ("g-game-n", ["--path", "nonrt", "--frames", "60"], {"RTVIS_GI_LOG": "1"}),
]
# The software rays' top-level BVH (src/instance_bvh.rs): RENDERLAB_INSTANCE_BVH on (1) or off
# (0, the flat instance loop), non-RT, interleaved twice. The forest and bare forest turn 150
# degrees at frame 24 (`cut=turn`), so a run has moving frames and a cut.
TOP = lambda v: {"RENDERLAB_INSTANCE_BVH": v}
TOP_CLIPS = [
    ("forest", ["--path", "nonrt", "--diag", "forest=1", "--diag", "cut=turn", *LOW, "--frames", "60"]),
    ("bare", ["--path", "nonrt", "--diag", "forest=bare", "--diag", "cut=turn", *LOW, "--frames", "60"]),
    ("game", ["--path", "nonrt", "--frames", "60"]),
]
PLANS["top"] = [
    (f"{name}-top{v}-{k}", extra, TOP(v)) for k in (1, 2) for name, extra in TOP_CLIPS for v in ("1", "0")
]
# Byte identity: every frame's hash, on against off (`hashes OUT top`).
PLANS["toppics"] = [
    (f"p-{name}-top{v}", extra + ["--every", "1"], TOP(v))
    for name, extra in [
        ("game", ["--path", "nonrt", "--frames", "60"]),
        ("forest", ["--path", "nonrt", "--diag", "forest=1", "--diag", "cut=turn", *LOW, "--frames", "30"]),
    ]
    for v in ("1", "0")
]
QUARRY = "D:/Coding/renderlab-work/mq1/quarry-manifest.json"
PLANS["topquarry"] = [
    (f"p-quarry-top{v}", ["--path", "nonrt", "--quarry", QUARRY, "--frames", "20", "--every", "1"], TOP(v))
    for v in ("1", "0")
]
# Run-to-run repeatability of one binary (`main` against itself): the forest walk and the quarry, twice.
PLANS["repeat"] = [
    (f"p-{name}-run{k}", extra + ["--every", "1"], {})
    for k in (1, 2)
    for name, extra in [
        ("forest", ["--path", "nonrt", "--diag", "forest=1", "--diag", "cut=turn", *LOW, "--frames", "30"]),
        ("quarry", ["--path", "nonrt", "--quarry", QUARRY, "--frames", "20"]),
    ]
]
# MIN_INSTANCES: the default (unset) against the flat loop on the small scenes, and the instance
# counts the GI log reports (`-n` runs).
PLANS["topdefault"] = [
    (f"{name}-default{v}-{k}", extra, {} if v == "1" else TOP("0"))
    for k in (1, 2) for name, extra in TOP_CLIPS[1:] for v in ("1", "0")
] + [(f"{name}-n", extra[:-2] + ["--frames", "12"], {"RTVIS_GI_LOG": "1"}) for name, extra in TOP_CLIPS]
# Non-RT `atmosphere` (U-RTVIS): the forest walk and the game flight, frames pipelined as
# game_scene runs them, then synchronous (`--every 1`: each frame's capture waits for the GPU,
# as a capturing embedder does), and the forest synchronous without the media. `cpu.wait` is the
# GPU's wait for the CPU's GI and post preparation between the early and late submits.
NONRT_GAME = ["--path", "nonrt", "--frames", "40"]
NONRT_WALK = ["--path", "nonrt", "--diag", "forest=1", *LOW, "--frames", "40"]
PLANS["atmos"] = [
    ("a-forest-piped", NONRT_WALK, {}),
    ("a-forest-sync", NONRT_WALK + ["--every", "1"], {}),
    ("a-forest-sync-nomedia", NONRT_WALK + ["--every", "1"], {"RENDERLAB_ABLATE": "media"}),
    ("a-game-piped", NONRT_GAME, {}),
    ("a-game-sync", NONRT_GAME + ["--every", "1"], {}),
]
# Shading ablations (RENDERLAB_ABLATE), both paths: the forest walk and the game flight.
PLANS["ablate"] = [
    (f"s-{scene}-{path}-{what or 'base'}", ["--path", path, *extra, "--frames", "40"],
     {"RENDERLAB_ABLATE": what} if what else {})
    for scene, extra in [("forest", ["--diag", "forest=1", *LOW]), ("game", [])]
    for path in ("rt", "nonrt")
    for what in (None, "reflections", "local_shadows", "gi")
]
# Byte identity of a measurement-only change: the non-RT game flight, every frame hashed.
PLANS["ident"] = [("p-game-ident", ["--path", "nonrt", "--frames", "30", "--every", "1"], {})]
# Reflection-hit shading split (RENDERLAB_RL_HIT, reflection hits only): the game flight.
RL_VARIANTS = [("base", {}), ("local", {"RENDERLAB_RL_HIT": "local"}), ("sun", {"RENDERLAB_RL_HIT": "sun"}),
               ("nolights", {"RENDERLAB_RL_HIT": "nolights"}), ("black", {"RENDERLAB_RL_HIT": "black"}),
               ("noreflections", {"RENDERLAB_ABLATE": "reflections"})]
PLANS["rlhit"] = [
    (f"r-{path}-{name}-{k}", ["--path", path, "--fox", FOX, "--frames", "60"], env)
    for path, visits in (("nonrt", (1, 2)), ("rt", (1,)))
    for k in visits
    for name, env in RL_VARIANTS
    if k in visits
]
# The stratum split of reflection hits' local-light visibility (RENDERLAB_RL_SPLIT).
SPLIT = lambda v: {"RENDERLAB_RL_SPLIT": v}
PLANS["rlsplit"] = [
    (f"q-{path}-split{v}-{k}", ["--path", path, "--fox", FOX, "--frames", "60"], SPLIT(v))
    for k in (1, 2) for path in ("nonrt", "rt") for v in ("1", "0")
]
PLANS["splitpics"] = [
    (f"p-{path}-{kind}-split{v}", ["--path", path, "--fox", FOX, *extra, "--frames", "48", "--every", "1"], SPLIT(v))
    for path in ("nonrt", "rt") for kind, extra in (("flight", []), ("hold", ["--diag", "hold=1"])) for v in ("1", "0")
]
# Local soft shadows (light_cluster_api.wgsl): the gather fast path on (1) or off (0), and the
# filter skipped (measurement), non-RT, the game flight with the Fox and the night lanterns.
GATHER = lambda v: {"RENDERLAB_LOCAL_GATHER": v}
PCF_CLIPS = [("game", ["--path", "nonrt", "--fox", FOX]), ("night", ["--path", "nonrt", "--fox", FOX, "--night-lanterns"])]
PLANS["pcf"] = [
    (f"g-{name}-{v}-{k}", extra + ["--frames", "60"], GATHER(v) if v != "skip" else {"RENDERLAB_LOCAL_PCF": "skip"})
    for k in (1, 2) for name, extra in PCF_CLIPS for v in ("1", "0", "skip") if k == 1 or v != "skip"
]
PLANS["pcfpics"] = [
    (f"p-{name}-gather{v}", extra + ["--frames", "40", "--every", "1"], GATHER(v))
    for name, extra in PCF_CLIPS for v in ("1", "0")
]
# Item 2 (2026-11-12): the forest cut on non-RT with the instance top level, cut_refill off/on.
PLANS["refill2"] = [(n, e, m) for n, e, m in PLANS["refill"] if n.startswith("nonrt-forest")]
# The cut refill rework (cap until drained, phases while deferring, no coarse read): forest turn
# and game jump, both paths, timing and pictures (still: frame 59 is the settled reference).
REFILL4_CLIPS = [
    ("nonrt-forest", ["--path", "nonrt", "--diag", "forest=1", "--diag", "cut=turn", *LOW]),
    ("rt-forest", ["--diag", "forest=1", "--diag", "cut=turn", *LOW]),
    ("nonrt-game", ["--path", "nonrt", "--diag", "cut=1"]),
    ("rt-game", ["--diag", "cut=1"]),
]
PLANS["refill4"] = [
    (f"{name}-refill{v}-{k}", extra + ["--frames", "60"], REFILL(v))
    for k in (1, 2) for name, extra in REFILL4_CLIPS for v in ("1", "0")
]
PLANS["refill4still"] = [
    (f"p-{name}-still-refill{v}", extra + ["--still", "--frames", "60", "--every", "1"], REFILL(v))
    for name, extra in REFILL4_CLIPS for v in ("1", "0")
]
PLANS["refill4move"] = [
    (f"p-{name}-move-refill{v}", extra + ["--frames", "36", "--every", "1"], REFILL(v))
    for name, extra in REFILL4_CLIPS for v in ("1", "0")
]
# Reflections (2026-11-12): where their cost goes by band, the game flight and night lanterns.
REFL_VARIANTS = [("base", {}), ("strata8", {"RENDERLAB_RL_HIT": "strata=8"}), ("strata4", {"RENDERLAB_RL_HIT": "strata=4"}),
                 ("rough03", {"RENDERLAB_REFLECTION_ROUGH": "0.3"}), ("rough01", {"RENDERLAB_REFLECTION_ROUGH": "0.1"}),
                 ("off", {"RENDERLAB_ABLATE": "reflections"})]
REFL_CLIPS = [("game", ["--fox", FOX]), ("night", ["--fox", FOX, "--night-lanterns"])]
PLANS["refl"] = [
    (f"x-{path}-{name}-{v}", ["--path", path, *extra, "--frames", "60"], env)
    for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v, env in REFL_VARIANTS
]
# Half the glossy strata a frame (RENDERLAB_RL_HALF): timing, and the game's pictures.
HALF = lambda v: {"RENDERLAB_RL_HALF": v}
PLANS["half"] = [
    (f"h-{path}-{name}-half{v}-{k}", ["--path", path, *extra, "--frames", "60"], HALF(v))
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v in ("1", "0")
]
PLANS["halfpics"] = [
    (f"p-{path}-{kind}-half{v}", ["--path", path, "--fox", FOX, *extra, "--frames", "48", "--every", "1"], HALF(v))
    for path in ("nonrt", "rt") for kind, extra in (("flight", []), ("hold", ["--diag", "hold=1"])) for v in ("1", "0")
]
# Wide-lobe strata subsets (RENDERLAB_RL_SUBSET=D@R; "1" is off): timing, and the game's pictures.
SUBSET = lambda v: {"RENDERLAB_RL_SUBSET": v}
SUBSET_ON = "4@0.3"
PLANS["subset"] = [
    (f"s-{path}-{name}-sub{v.replace('@', 'r')}-{k}", ["--path", path, *extra, "--frames", "60"], SUBSET(v))
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v in (SUBSET_ON, "1")
]
PLANS["subsetpics"] = [
    (f"p-{path}-{kind}-sub{v.replace('@', 'r')}", ["--path", path, "--fox", FOX, *extra, "--frames", "48", "--every", "1"], SUBSET(v))
    for path in ("nonrt", "rt") for kind, extra in (("flight", []), ("hold", ["--diag", "hold=1"])) for v in (SUBSET_ON, "1")
]
# The traced band's end (RENDERLAB_REFLECTION_ROUGH replaces max_roughness, default 0.5): timing.
ROUGH_VALUES = ("0.5", "0.35", "0.25")
PLANS["rough"] = [
    (f"r-{path}-{name}-r{v}-{k}", ["--path", path, *extra, "--frames", "60"], {"RENDERLAB_REFLECTION_ROUGH": v})
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v in ROUGH_VALUES
]
# Reflection history (RENDERLAB_RL_HISTORY=1, off by default; reflections_history.wgsl): timing on
# the game flight and night lanterns, and the game's flight and cut pictures, both paths.
HIST = lambda v: {"RENDERLAB_RL_HISTORY": v}
PLANS["histsmoke"] = [(f"q-{path}-smoke", ["--path", path, "--frames", "6"], HIST("1")) for path in ("rt", "nonrt")]
PLANS["hist"] = [
    (f"h-{path}-{name}-hist{v}-{k}", ["--path", path, *extra, "--frames", "60"], HIST(v))
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v in ("1", "0")
]
PLANS["histpics"] = [
    (f"p-{path}-{kind}-hist{v}", ["--path", path, "--fox", FOX, *extra, "--every", "1"], HIST(v))
    for path in ("nonrt", "rt")
    for kind, extra in (("flight", ["--frames", "48"]), ("cut", ["--diag", "cut=1", "--frames", "36"]))
    for v in ("1", "0")
]
ON = {"RENDERLAB_RL_HISTORY": "1"}
HIST_TUNES = [("off", {"RENDERLAB_RL_HISTORY": "0"}), ("s1", ON),
              ("s4c1", ON | {"RENDERLAB_RL_HISTORY_TUNE": "strata=4,cap=1,moving=1"}),
              ("s4c4", ON | {"RENDERLAB_RL_HISTORY_TUNE": "strata=4,cap=4,moving=4"})]
PLANS["histtune"] = [
    (f"t-{path}-{name}-{v}-{k}", ["--path", path, *extra, "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in REFL_CLIPS for v, env in HIST_TUNES
]
# Diagnosis: never reject (sigma huge), against off and the default, game flight both paths;
# and the game held still (hold=1), where every glossy pixel's history should be trusted.
HIST_DIAG = [("off", {"RENDERLAB_RL_HISTORY": "0"}), ("s1", ON),
             ("norej", ON | {"RENDERLAB_RL_HISTORY_TUNE": "sigma=1000000000"})]
PLANS["histdiag"] = [
    (f"d-{path}-{name}-{v}-{k}", ["--path", path, "--fox", FOX, *extra, "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in (("game", []), ("hold", ["--diag", "hold=1"]))
    for v, env in HIST_DIAG
]
# The Showcase-like glossy band (reflection_matched --stage --metal-balls, 1280x720, moving):
# shading with the history off, on, and four strata a frame. Run with the reflection_matched exe.
# Timing only: the one written frame is frame 0, outside the measured frames.
SHOW = ["--matched", "--lamp", "1", "--stage", "--metal-balls", "--write-frames", "0"]
PLANS["histshow"] = [
    (f"s-{path}-{v}-{k}", ["--path", path, *SHOW], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in HIST_TUNES[:3]
]
# History-only-while-still gate: moving game frames (flight, night lanterns) for a byte
# check against main's binary (same plan, run with each exe), and still timing.
PLANS["gatemoving"] = [
    (f"p-{path}-{name}", ["--path", path, *extra, "--every", "1"], {})
    for path in ("nonrt", "rt")
    for name, extra in (("flight", ["--fox", FOX, "--frames", "48"]), ("night", ["--fox", FOX, "--night-lanterns", "--frames", "30"]))
]
GATE_VARIANTS = (("off", {"RENDERLAB_RL_HISTORY": "0"}), ("on", {}), ("cap4", {"RENDERLAB_RL_HISTORY_TUNE": "cap=4"}))
# Run with reflection_matched (the Showcase-like band held at frame 32) and game_scene (hold=1).
PLANS["gateshow"] = [
    (f"s-{path}-show-{v}-{k}", ["--path", path, *SHOW, "--freeze", "32"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in GATE_VARIANTS
]
PLANS["gatehold"] = [
    (f"s-{path}-hold-{v}-{k}", ["--path", path, "--fox", FOX, "--diag", "hold=1", "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in GATE_VARIANTS
]
# The history's spatial resolve under motion (RENDERLAB_RL_RESOLVE=1): the Showcase-like band
# moving (run with reflection_matched) and the game flight (game_scene), against the default.
RES_VARIANTS = (("base", {}), ("res", {"RENDERLAB_RL_RESOLVE": "1"}),
                ("res4", {"RENDERLAB_RL_RESOLVE": "1", "RENDERLAB_RL_HISTORY_TUNE": "strata=4"}))
PLANS["resshow"] = [
    (f"m-{path}-show-{v}-{k}", ["--path", path, *SHOW], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in RES_VARIANTS
]
PLANS["resgame"] = [
    (f"g-{path}-flight-{v}-{k}", ["--path", path, "--fox", FOX, "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in RES_VARIANTS
]
# The reflection pass's first stage (RENDERLAB_RL_PASS=1: lobes and rl_trace, the picture
# still inline): byte identity on against off (`hashes OUT rlpass`) and the trace's cost.
PASS = lambda v: {"RENDERLAB_RL_PASS": v}
# The game flight shades two programs (game, toon); the quarry is the single-program case.
PASS_CLIPS = (("flight", ["--fox", FOX, "--frames", "40"]), ("quarry", ["--quarry", QUARRY, "--frames", "20"]))
PLANS["rlpass"] = [
    (f"p-{path}-{name}-rlpass{v}", ["--path", path, *extra, "--every", "1"], PASS(v))
    for path in ("nonrt", "rt") for name, extra in PASS_CLIPS for v in ("1", "0")
]
PLANS["rlpasstime"] = [
    (f"t-{path}-{name}-pass{v}", ["--path", path, *extra], PASS(v))
    for path in ("nonrt", "rt") for name, extra in PASS_CLIPS for v in ("1", "0")
]
PLANS["rlpassshow"] = [
    (f"t-{path}-show-pass{v}", ["--path", path, *SHOW], PASS(v)) for path in ("nonrt", "rt") for v in ("1", "0")
]
# rl_resolve as a pass (RENDERLAB_RL_HISTORY=1 RENDERLAB_RL_RESOLVE_PASS=1) against the base.
RP = {"RENDERLAB_RL_HISTORY": "1", "RENDERLAB_RL_RESOLVE_PASS": "1"}
RP_VARIANTS = (("base", {}), ("rp", RP))
PLANS["rpshow"] = [
    (f"m-{path}-show-{v}-{k}", ["--path", path, *SHOW], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in RP_VARIANTS
]
PLANS["rpgame"] = [
    (f"g-{path}-flight-{v}-{k}", ["--path", path, "--fox", FOX, "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in RP_VARIANTS
]
PLANS["rppics"] = [
    (f"p-{path}-cut-rp{int(v == 'rp')}", ["--path", path, "--fox", FOX, "--diag", "cut=1", "--frames", "36", "--every", "1"], env)
    for path in ("nonrt", "rt") for v, env in RP_VARIANTS
]
# The pass's probe (RENDERLAB_RL_PROBE, measurement only): frame 30's H count and R/D slot counts.
PROBE_DIR = WORK / "probe"
PLANS["rlprobe"] = [
    (f"q-{path}-{name}-probe", ["--path", path, *extra, "--frames", "40"],
     PASS("1") | {"RENDERLAB_RL_PROBE": str(PROBE_DIR / f"{path}-{name}.json")})
    for path in ("nonrt", "rt") for name, extra in (("flight", ["--fox", FOX]), ("quarry", ["--quarry", QUARRY]))
]
# The no-lag resolve (step 1) and four strata a frame on rough lobes (step 2), Showcase-like timing.
NL_VARIANTS = (("base", {}), ("nl", RP), ("nl4", RP | {"RENDERLAB_RL_HISTORY_TUNE": "rough_strata=4"}))
PLANS["nlshow"] = [
    (f"m-{path}-show-{v}-{k}", ["--path", path, *SHOW], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in NL_VARIANTS
]
# The resolve's history length and clamp (RENDERLAB_RL_RESOLVE_TUNE) with four rough strata.
NL4 = RP | {"RENDERLAB_RL_HISTORY_TUNE": "rough_strata=4"}
PLANS["nltune"] = [
    (f"m-{path}-show-{v}", ["--path", path, *SHOW], env)
    for path in ("nonrt", "rt")
    for v, env in (("c4k1", NL4 | {"RENDERLAB_RL_RESOLVE_TUNE": "cap=4,clamp=1.0"}),
                   ("c2k075", NL4 | {"RENDERLAB_RL_RESOLVE_TUNE": "cap=2,clamp=0.75"}))
]
# The resolve candidate (history, resolve pass, footprint 0.03, 4 strata a frame on every glossy lobe).
CAND = RP | {"RENDERLAB_RL_HISTORY_TUNE": "strata=4,rough_strata=4", "RENDERLAB_RL_RESOLVE_TUNE": "footprint=0.03"}
PLANS["candshow"] = [
    (f"m-{path}-show-{v}-{k}", ["--path", path, *SHOW], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in (("base", {}), ("cand", CAND))
]
PLANS["candgame"] = [
    (f"g-{path}-flight-{v}-{k}", ["--path", path, "--fox", FOX, "--frames", "60"], env)
    for k in (1, 2) for path in ("nonrt", "rt") for v, env in (("base", {}), ("cand", CAND))
]
# The candidate's merge gate: the game flight and a cut, the default (history + resolve) against
# RENDERLAB_RL_HISTORY=0 (today's inline full set), both paths (`compare.py PICS CROPS gate`).
PLANS["gatepics2"] = [
    (f"p-{path}-{name}-gate{v}", ["--path", path, "--fox", FOX, *extra, "--every", "1"], {"RENDERLAB_RL_HISTORY": v})
    for path in ("nonrt", "rt")
    for name, extra in (("flight", ["--frames", "48"]), ("cut", ["--diag", "cut=1", "--frames", "36"]))
    for v in ("1", "0")
]
# Byte identity with main on moving frames (run with each binary; `bin_compare.py A B`).
PLANS["identmain"] = [
    (f"p-{path}-{name}", ["--path", path, *extra, "--every", "1"], {})
    for path in ("nonrt", "rt")
    for name, extra in (("flight", ["--fox", FOX, "--frames", "48"]), ("night", ["--fox", FOX, "--night-lanterns", "--frames", "30"]))
]
# Where a cut's frame time goes (pass log detail and the GI log): the game cut and the forest
# turn, both paths, twice (`timing.py gi OUT`).
PLANS["cutsplit"] = [
    (f"{name}-{path}-{k}", ["--path", path, *extra, "--frames", "40"], {"RTVIS_GI_LOG": "1"})
    for k in (1, 2) for path in ("nonrt", "rt")
    for name, extra in (("game", ["--diag", "cut=1"]), ("forest", ["--diag", "forest=1", "--diag", "cut=turn", *LOW]))
]
# Item (2), the post-cut gather time budget with parent stand-ins (`RENDERLAB_GI`) against the
# default (hit_gathers=1 already): frame times (`timing.py gi OUT`) and every frame's picture
# around the cut (`compare.py PICS CROPS cutgi`).
CUTGI = lambda v: {"RENDERLAB_GI": "cut_gather_ms=3;parent_standin=1"} if v == "1" else {}
CUTGI_CLIPS = (("game", ["--diag", "cut=1"]), ("forest", ["--diag", "forest=1", "--diag", "cut=turn", *LOW]),
               ("respawn", ["--fox", FOX, "--diag", "hold=cut"]))
PLANS["cutgi"] = [
    (f"{name}-{path}-cutgi{v}", ["--path", path, *extra, "--frames", "40"], CUTGI(v) | {"RTVIS_GI_LOG": "1"})
    for path in ("nonrt", "rt") for name, extra in CUTGI_CLIPS for v in ("1", "0")
]
PLANS["cutgipics"] = [
    (f"p-{path}-{name}-cutgi{v}", ["--path", path, *extra, "--frames", "60" if name == "respawn" else "34", "--every", "1"], CUTGI(v))
    for path in ("nonrt", "rt") for name, extra in CUTGI_CLIPS for v in ("1", "0")
]
# The deferred reflection composite (RENDERLAB_RL_PASS=1 RENDERLAB_RL_DEFERRED=1) against the
# default: pictures (`compare.py PICS CROPS comp`), the strata counter (probe) and timing.
COMP = lambda v: {"RENDERLAB_RL_PASS": "1", "RENDERLAB_RL_DEFERRED": "1"} if v == "1" else {}
COMP_CLIPS = (("flight", ["--fox", FOX]), ("night", ["--fox", FOX, "--night-lanterns"]))
PLANS["comppics"] = [
    (f"p-{path}-{name}-comp{v}", ["--path", path, *extra, "--frames", "40", "--every", "1"], COMP(v))
    for path in ("nonrt", "rt") for name, extra in COMP_CLIPS for v in ("1", "0")
]
PLANS["compprobe"] = [
    (f"q-{path}-{name}-comp", ["--path", path, *extra, "--frames", "40"],
     COMP("1") | {"RENDERLAB_RL_PROBE": str(PROBE_DIR / f"comp-{path}-{name}.json")})
    for path in ("nonrt", "rt") for name, extra in COMP_CLIPS
]
PLANS["comptime"] = [
    (f"t-{path}-{name}-comp{v}-{k}", ["--path", path, *extra, "--frames", "60"], COMP(v))
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in COMP_CLIPS for v in ("1", "0")
]
# One lease, one reference run: the composite's probe, pictures and timing in a single call.
PLANS["comp4"] = PLANS["compprobe"] + PLANS["comppics"] + PLANS["comptime"]
# comp4d: non-deferred programs untouched (default scene, switches on vs off, byte-identical),
# the composite on the standard program (`--diag program=standard`) against inline, moving and
# still, its probe and timing, then the reference again (a lease's timing counts only if both
# references are within 10 %).
STD = ["--diag", "program=standard"]
PLANS["comp4d"] = (
    [(f"p-{path}-ident-comp{v}", ["--path", path, "--fox", FOX, "--frames", "30", "--every", "1"], COMP(v))
     for path in ("nonrt", "rt") for v in ("1", "0")]
    + [(f"p-{path}-std{kind}-comp{v}", ["--path", path, "--fox", FOX, *STD, *still, "--frames", "40", "--every", "1"], COMP(v))
       for path in ("nonrt", "rt") for kind, still in (("flight", []), ("still", ["--still"])) for v in ("1", "0")]
    + [(f"q-{path}-std-comp", ["--path", path, "--fox", FOX, *STD, "--frames", "40"],
        COMP("1") | {"RENDERLAB_RL_PROBE": str(PROBE_DIR / f"comp-{path}-std.json")}) for path in ("nonrt", "rt")]
    + [(f"t-{path}-{name}-comp{v}-{k}", ["--path", path, "--fox", FOX, *STD, *extra, "--frames", "60"], COMP(v))
       for k in (1, 2) for path in ("nonrt", "rt") for name, extra in (("flight", []), ("night", ["--night-lanterns"]))
       for v in ("1", "0") if k == 1 or name == "flight"]
    + [("reference-end", [], {})]
)
# comp4d split in two leases: pictures (identity, standard composite vs inline, probe), then
# timing with the reference again at the end.
PLANS["comppicsd"] = [x for x in PLANS["comp4d"] if x[0][0] in "pq"]
PLANS["comptimed"] = [x for x in PLANS["comp4d"] if x[0][0] == "t" or x[0] == "reference-end"]
# comp4g: the RT standard-program pictures (moving and still) first, then the timing.
PLANS["comp4g"] = [x for x in PLANS["comp4d"] if x[0].startswith("p-rt-std")] + PLANS["comptimed"]
# The RT speck bisect (root, 2026-11-13): the standard-program flight on RT, composite against
# inline under each hit switch (RENDERLAB_RL_HIT, composed into both), frames 0/13/26/39.
BISECT = (("base", ""), ("black", "black"), ("nolights", "nolights"), ("sun", "sun"),
          ("local", "local"), ("s4", "strata=4"), ("s4black", "strata=4,black"))
PLANS["rtbisect"] = [
    (f"p-rt-{name}-comp{v}", ["--path", "rt", "--fox", FOX, *STD, "--frames", "40", "--every", "13"],
     COMP(v) | ({"RENDERLAB_RL_HIT": hit} if hit else {}))
    for name, hit in BISECT for v in ("1", "0")
]
# specks: RT, the pass forced on against off, under the pass's own switches (program.rs
# rl_pass_switches; `black` above only blackened inline hits). The lamp cap at frames 0 and 13 of
# the flight, and root's row: night lanterns' moving gold sphere, every frame of 34.
SPECK_SWITCHES = (("plain", ""), ("missblack", "missblack"), ("lone", "lone"), ("hitblack", "hitblack"))
PLANS["specks"] = (
    [(f"s-rt-cap-off", ["--path", "rt", "--fox", FOX, *STD, "--frames", "14", "--every", "13"], {})]
    + [(f"s-rt-cap-{name}", ["--path", "rt", "--fox", FOX, *STD, "--frames", "14", "--every", "13"],
        COMP("1") | ({"RENDERLAB_RL_HIT": hit} if hit else {}))
       for name, hit in SPECK_SWITCHES]
    + [(f"s-rt-night-off", ["--path", "rt", "--fox", FOX, *STD, "--night-lanterns", "--frames", "34", "--every", "1"], {})]
    + [(f"s-rt-night-{name}", ["--path", "rt", "--fox", FOX, *STD, "--night-lanterns", "--frames", "34", "--every", "1"],
        COMP("1") | ({"RENDERLAB_RL_HIT": hit} if hit else {}))
       for name, hit in SPECK_SWITCHES]
)
# comp5: the non-RT default (no switches) against RENDERLAB_RL_PASS=0 (main's path): bytes and
# the probe (60 frames, flight and lanterns), pictures moving, with planner cuts (cut=1) and a
# respawn under a still camera (hold=cut; the event at frame 24), and RT unset vs off (identity).
OFF = {"RENDERLAB_RL_PASS": "0"}
PLANS["comp5"] = (
    [(f"q-nonrt-{name}-{tag}", ["--path", "nonrt", "--fox", FOX, *STD, *extra, "--frames", "120"],
      env | ({"RENDERLAB_RL_PROBE": str(PROBE_DIR / f"comp5-{name}.json"),
              "RENDERLAB_RL_ROOM_LOG": str(PROBE_DIR / f"comp5-{name}-memory.jsonl")} if tag == "default" else {}))
     for name, extra in (("flight", []), ("night", ["--night-lanterns"])) for tag, env in (("default", {}), ("off", OFF))]
    + [(f"p-nonrt-{name}-comp{v}", ["--path", "nonrt", "--fox", FOX, *STD, *extra, "--frames", "40", "--every", "1"],
        {} if v == "1" else OFF)
       for name, extra in (("flight", []), ("cut", ["--diag", "cut=1"]), ("holdcut", ["--diag", "hold=cut"]))
       for v in ("1", "0")]
    + [(f"p-rt-ident-comp{v}", ["--path", "rt", "--fox", FOX, *STD, "--frames", "30", "--every", "1"],
        {} if v == "1" else OFF) for v in ("1", "0")]
    # Forced overflow (RENDERLAB_RL_ROOM=fixed:16384, no growth): a mixed composite/inline estimate
    # every frame, with each frame's overflow count logged, against inline.
    + [(f"p-nonrt-fix{name}-comp{v}", ["--path", "nonrt", "--fox", FOX, *STD, *extra, "--frames", "40", "--every", "1"],
        {"RENDERLAB_RL_ROOM": "fixed:16384", "RENDERLAB_RL_ROOM_LOG": str(PROBE_DIR / f"comp5-fix{name}.jsonl")}
        if v == "1" else OFF)
       for name, extra in (("flight", []), ("night", ["--night-lanterns"])) for v in ("1", "0")]
)
# comp6 on the glossy-pixel list, in two leases.
# comp6a, pictures: the limit (a floor at roughness 0.5, 0.499 and 0.501 under the night lanterns)
# and a cross-program hit (the toon hero in a 0.2 glossy floor), each the non-RT default against
# RENDERLAB_RL_PASS=0 (identical within noise: the w floor and inline's hit fallback), the same two
# with RENDERLAB_RL_HIT_OWN_PROGRAM=1 (the evidence for Islam), then comp5's pictures.
OWN = {"RENDERLAB_RL_HIT_OWN_PROGRAM": "1"}
LIMIT_ROWS = [(f"floor{r.replace('.', '')}", ["--diag", f"floor={r}", "--night-lanterns"]) for r in ("0.5", "0.499", "0.501")]
CROSS_ROWS = [("toon", ["--diag", "floor=0.2"])]
PLANS["comp6a"] = (
    [(f"p-nonrt-{name}-comp{v}", ["--path", "nonrt", "--fox", FOX, *STD, *extra, "--frames", "24", "--every", "1"],
      {} if v == "1" else OFF)
     for name, extra in LIMIT_ROWS + CROSS_ROWS for v in ("1", "0")]
    + [(f"p-nonrt-{name}own-comp{v}", ["--path", "nonrt", "--fox", FOX, *STD, *extra, "--frames", "24", "--every", "1"],
        OWN if v == "1" else OFF)
       for name, extra in (LIMIT_ROWS[1], CROSS_ROWS[0]) for v in ("1", "0")]
    + [x for x in PLANS["comp5"] if x[0].startswith("p-") and "fix" not in x[0]]
)
# comp6b, bytes and timing: comp5's probe and memory rows and its forced-overflow rows, plus the
# coordinator's finding 2 (no glossy lobes: the game program's flight; few: the standard program's),
# default against RENDERLAB_RL_PASS=0, 60 frames, two rounds, the reflection.* stages in the log.
PLANS["comp6b"] = (
    [x for x in PLANS["comp5"] if x[0].startswith("q-") or "fix" in x[0]]
    + [(f"t-nonrt-{name}-{tag}-{k}", ["--path", "nonrt", "--fox", FOX, *extra, "--frames", "60"], env)
       for k in (1, 2) for name, extra in (("zero", []), ("few", STD)) for tag, env in (("default", {}), ("off", OFF))]
    # The own-program switch's cost: the toon hero in a 0.2 glossy floor (a mixed frame).
    + [(f"t-nonrt-toon-{tag}-{k}", ["--path", "nonrt", "--fox", FOX, *STD, "--diag", "floor=0.2", "--frames", "60"], env)
       for k in (1, 2) for tag, env in (("default", {}), ("own", OWN))]
)
# The glossy strata slope (RL_RAYS_PER_ROUGHNESS 48, 32, 24: one binary each, same plans): the
# flight over a 0.2 floor (`--diag floor=0.2`, grazing ground) and the default flight (gold 0.25,
# paint 0.35), moving and still; `compare.py` crops the reflected edges, binary against binary.
SLOPE_CLIPS = (("floor", ["--fox", FOX, "--diag", "floor=0.2"]), ("flight", ["--fox", FOX]))
PLANS["slopepics"] = [
    (f"p-{path}-{name}-{kind}", ["--path", path, *extra, *still, "--frames", "40", "--every", "1"], {})
    for path in ("nonrt", "rt") for name, extra in SLOPE_CLIPS
    for kind, still in (("moving", []), ("still", ["--still"]))
]
PLANS["slopetime"] = [
    (f"t-{path}-{name}-{k}", ["--path", path, *extra, "--frames", "60"], {})
    for k in (1, 2) for path in ("nonrt", "rt") for name, extra in SLOPE_CLIPS
]
PLANS["refillpics"] = [
    (f"p-{name}-refill{v}", extra + ["--frames", "40", "--every", "1"], REFILL(v))
    for name, extra in REFILL_CLIPS
    for v in ("1", "0")
]
MATCHED_FRAMES = "16,32,47"


def matched(exe, out, spp="4096"):
    """reflection_matched's flight: the RT matched frames with the held stop on and off, then
    the path tracer at the same poses (the measurement scene's moving crate and hero)."""
    out = Path(out)
    env = environment()
    code = 0
    for name, extra, more in [
        ("m-held1", ["--path", "rt", "--matched"], HELD("1")),
        ("m-held0", ["--path", "rt", "--matched"], HELD("0")),
        ("m-reference", ["--path", "rt", "--reference", spp], {}),
    ]:
        d = out / name
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--write-frames", MATCHED_FRAMES, "--out", str(d)], cwd=REPO,
                           capture_output=True, text=True, timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(name, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def rows(log):
    return [json.loads(line) for line in Path(log).read_text().splitlines() if line.strip()]


def environment():
    drop = ("VK_INSTANCE_LAYERS", "VK_LAYER_PATH", "RENDERLAB_GI", "RENDERLAB_DIAG_ALL_OPAQUE")
    env = {k: v for k, v in os.environ.items() if k not in drop}
    # The pre-step-3 cold budget and no held stop unless a run sets them itself.
    return env | {"RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK": "I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK",
                  "TMP": str(WORK / "tmp"), "TEMP": str(WORK / "tmp"), "RENDERLAB_COLD_PROBES": "0",
                  "RENDERLAB_HELD_PROBES": "0"}


def fly(game, out, extra, env, frames=120):
    out.mkdir(parents=True, exist_ok=True)
    log = out / "passes.jsonl"
    log.unlink(missing_ok=True)
    # Later flags win in game_scene's parser: `extra` may override the path, size and frames.
    args = [game, "--path", "rt", "--size", "1920x1080", "--frames", str(frames), "--every", "1000",
            *extra, "--out", str(out), "--pipeline-cache", str(WORK / "cache")]
    extra_env = {"RENDERLAB_PASS_LOG": str(log), "RENDERLAB_PASS_LOG_DETAIL": "1"}
    if env.get("RTVIS_GI_LOG") == "1":
        (out / "gi.jsonl").unlink(missing_ok=True)
        extra_env["RENDERLAB_GI_LOG"] = str(out / "gi.jsonl")
    r = subprocess.run(args, cwd=REPO, capture_output=True, text=True, timeout=600,
                       env=env | extra_env)
    (out / "stderr.txt").write_text(r.stderr[-20000:])
    print(out.name, "exit", r.returncode, flush=True)
    return r.returncode


def run(game, out, plan="repro"):
    out = Path(out)
    env = environment()
    code = fly(game, out / "reference", [], env)
    frame = st.median(r["span"] for r in rows(out / "reference/passes.jsonl")[8:])
    ok = REFERENCE_MS is None or 0.9 * REFERENCE_MS <= frame <= 1.1 * REFERENCE_MS
    (out / "reference.json").write_text(json.dumps(dict(frame_ms=frame, expected_ms=REFERENCE_MS, within_10_percent=ok)))
    print(f"reference frame {frame:.2f} ms (expected {REFERENCE_MS}): {'ok' if ok else 'MORE THAN 10% OFF'}", flush=True)
    for name, extra, more in PLANS[plan]:
        code = max(code, fly(game, out / name, extra, env | more))
    return code


def gi(out):
    """Per run: frame span and the GI stages from the frame before the cut (game frame 23) to
    ten frames after it (34); the pass log counts frames from 1."""
    out = Path(out)
    keys = ("span", "gi_update", "gi_gather", "gi_claim", "gi_integrate", "gi_cell_shading", "shading")
    for d in sorted(p for p in out.iterdir() if (p / "passes.jsonl").exists() and p.name != "reference"):
        r = {x["frame"]: x for x in rows(d / "passes.jsonl")}
        print(f"== {d.name}  (frames 23..34; moving median before the cut: "
              f"{st.median(r[f]['span'] for f in range(9, 24) if f in r):.1f} ms)")
        for k in keys:
            vals = [(r[f + 1]["span"] if k == "span" else r[f + 1]["passes"].get(k, 0.0)) if f + 1 in r else float("nan")
                    for f in range(23, 35)]
            print(f"  {k:16}" + " ".join(f"{v:6.1f}" for v in vals))


def switch_matched(exe, out, key, name, spp="4096"):
    """`split_matched` for any on/off switch: environment `key` 1 or 0, runs named `name`1/0."""
    out = Path(out)
    env = environment()
    code = 0
    runs = [(f"m-{path}-{kind}-{name}{v}", ["--path", path, "--matched", "--lamp", "1", *extra], {key: v})
            for kind, extra in [("moving", ["--write-frames", "16,32,47"]), ("still", ["--freeze", "32", "--write-frames", "47"])]
            for path in ("rt", "nonrt") for v in ("1", "0")]
    runs.append(("m-reference", ["--path", "rt", "--lamp", "1", "--reference", spp, "--write-frames", "16,32,47"], {}))
    for run, extra, more in runs:
        d = out / run
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--out", str(d)], cwd=REPO, capture_output=True, text=True,
                           timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(run, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def pair_matched(exe, out, name, key, value, spp="4096"):
    """reflection_matched with `key`=`value` (runs `name`1) against main (`name`0), both paths,
    moving frames 16/32/47 and the held pose of frame 32, then the path tracer at those poses."""
    out = Path(out)
    env = environment()
    code = 0
    runs = [(f"m-{path}-{kind}-{name}{v}", ["--path", path, "--matched", "--lamp", "1", *extra],
             {key: value} if v == "1" else {})
            for kind, extra in [("moving", ["--write-frames", "16,32,47"]), ("still", ["--freeze", "32", "--write-frames", "47"])]
            for path in ("rt", "nonrt") for v in ("1", "0")]
    if not (out / "m-reference" / "frame-047.png").exists():
        runs.append(("m-reference", ["--path", "rt", "--lamp", "1", "--reference", spp, "--write-frames", "16,32,47"], {}))
    for run, extra, more in runs:
        d = out / run
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--out", str(d)], cwd=REPO, capture_output=True, text=True,
                           timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(run, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def multi_matched(exe, out, key, *values, spp="4096"):
    """reflection_matched with `key` at each value (runs m-PATH-KIND-vVALUE; "-" leaves it unset),
    both paths, moving frames 16/32/47 and the held pose of frame 32, then the path tracer.
    RTVIS_MATCHED_EXTRA (space-separated) adds scene options to every run, the reference too."""
    out = Path(out)
    env = environment()
    scene = os.environ.get("RTVIS_MATCHED_EXTRA", "").split()
    code = 0
    runs = [(f"m-{path}-{kind}-v{v}", ["--path", path, "--matched", "--lamp", "1", *scene, *extra], {} if v == "-" else {key: v})
            for kind, extra in [("moving", ["--write-frames", "16,32,47"]), ("still", ["--freeze", "32", "--write-frames", "47"])]
            for path in ("rt", "nonrt") for v in values]
    if not (out / "m-reference" / "frame-047.png").exists():
        runs.append(("m-reference", ["--path", "rt", "--lamp", "1", *scene, "--reference", spp, "--write-frames", "16,32,47"], {}))
    for run, extra, more in runs:
        d = out / run
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--out", str(d)], cwd=REPO, capture_output=True, text=True,
                           timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(run, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def split_matched(exe, out, spp="4096"):
    """reflection_matched with its arch lamp (a shadowed local light), RENDERLAB_RL_SPLIT on and
    off, both paths: the moving flight's frames 16, 32 and 47, and the pose of frame 32 held still
    for 48 frames (`--freeze 32`, frame 47 written); then the path tracer at those poses."""
    out = Path(out)
    env = environment()
    code = 0
    runs = [(f"m-{path}-{kind}-split{v}", ["--path", path, "--matched", "--lamp", "1", *extra], SPLIT(v))
            for kind, extra in [("moving", ["--write-frames", "16,32,47"]), ("still", ["--freeze", "32", "--write-frames", "47"])]
            for path in ("rt", "nonrt") for v in ("1", "0")]
    runs.append(("m-reference", ["--path", "rt", "--lamp", "1", "--reference", spp, "--write-frames", "16,32,47"], {}))
    for name, extra, more in runs:
        d = out / name
        d.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([exe, *extra, "--out", str(d)], cwd=REPO, capture_output=True, text=True,
                           timeout=900, env=env | more)
        (d / "stderr.txt").write_text(r.stderr[-20000:])
        print(name, "exit", r.returncode, flush=True)
        code = max(code, r.returncode)
    return code


def hashes(out, switch="cold"):
    """Per picture clip: the frames whose hashes differ between the stop on and off."""
    out = Path(out)
    for on in sorted(out.glob(f"p-*-{switch}1")):
        off = on.with_name(on.name[:-1] + "0")
        a, b = (json.loads((d / "run.json").read_text())["frames"] for d in (on, off))
        differ = [x["frame"] for x, y in zip(a, b) if x["rgba8_sha256"] != y["rgba8_sha256"]]
        print(f"{on.name[:-6]:16} frames {len(a)}  differing {len(differ)}: {differ[:12]}{' ...' if len(differ) > 12 else ''}")


def report(out):
    out = Path(out)
    print(json.loads((out / "reference.json").read_text()))
    for d in sorted(p for p in out.iterdir() if (p / "passes.jsonl").exists()):
        r = rows(d / "passes.jsonl")
        moving = [x for x in r if 8 <= x["frame"] <= 22 or x["frame"] >= 40]
        if not moving:
            print(f"{d.name:22} n={len(r):3} (no moving frames; see stderr.txt)")
            continue
        parts = sorted({k for x in r for k in x["passes"] if k.startswith("visibility")})
        med = lambda key, xs: st.median(x["passes"].get(key, 0.0) for x in xs)
        line = f"{d.name:22} n={len(r):3} frame {st.median(x['span'] for x in moving):6.2f}"
        line += "".join(f"  {k} {med(k, moving):6.2f}" for k in parts)
        print(line)
        around = [x for x in r if 22 <= x["frame"] <= 30]
        print(" " * 22, "cut frames 22-30 visibility:", " ".join(f"{sum(x['passes'].get(k, 0) for k in parts):.1f}" for x in around),
              "| span:", " ".join(f"{x['span']:.1f}" for x in around))


if __name__ == "__main__":
    if sys.argv[1] == "run":
        raise SystemExit(run(*sys.argv[2:5]))
    if sys.argv[1] == "report":
        report(sys.argv[2])
    if sys.argv[1] == "hashes":
        hashes(*sys.argv[2:4])
    if sys.argv[1] == "gi":
        gi(sys.argv[2])
    if sys.argv[1] == "multi_matched":
        raise SystemExit(multi_matched(*sys.argv[2:]))
    if sys.argv[1] == "pair_matched":
        raise SystemExit(pair_matched(*sys.argv[2:7]))
    if sys.argv[1] == "switch_matched":
        raise SystemExit(switch_matched(*sys.argv[2:6]))
    if sys.argv[1] == "split_matched":
        raise SystemExit(split_matched(*sys.argv[2:5]))
    if sys.argv[1] == "matched":
        raise SystemExit(matched(*sys.argv[2:5]))
