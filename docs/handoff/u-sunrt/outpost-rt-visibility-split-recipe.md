# Outpost RT visibility split: PC integration recipe

CPU handoff, corrected for the PC backend. Earlier Apple-port instructions
are withdrawn. No renderer implementation is authorized before this split.

## Backend and dispatch gate

On Windows, Ara's `crates/ara-renderlab/Cargo.toml` selects Renderlab without
`backend-ara`; `bridge.rs` calls the wgpu `Engine::render_view` on Renderlab's
own Vulkan device. The Apple `backend_ara/forward.rs` route is irrelevant.
P198 exists at `D:/Coding/renderlab-work/ara-pin/bin-P198` (Renderlab
`fdf2fb05`). Use the SAME P198 executable for every off/on row.

The wgpu split branch requires traced primary visibility, Interactive
shadows, active reconstruction with its trace pipeline, and
`rotate_directions=true`. SUN_PASS then selects `encode_sun_split`.
`ensure_reconstruction` builds `vb_sun_source` for these split traced frames;
this uses `trace_source_held`, matching the wgpu fused TR1 path. The former
Apple-only `trace_source` versus `trace_source_held` warning does not apply.

Before timing, require positive `visibility` and `sun_shadow` labels in the
SAME logged frame from P198 with SUN_PASS=1. An accepted environment variable
is insufficient. The authorized four-video-frame ordinary gate is
`D:/Coding/renderlab-work/u-sunrt/outpost_label_gate.py`; it writes
`outpost-label-gate/result.json`, `start.log`, `cpu.jsonl` and `pass.jsonl`.
Gate result: **PASSED** on P198, ordinary lease
`u-sunrt-outpost-label-02`, exit 0 in 24.41 seconds, no watchdog failures.
The player log confirms `Renderlab (HardwareRt)` on RTX 4070 SUPER. All
84 logged renderer frames (IDs 1..84, complete/in order) contain positive
`visibility` and `sun_shadow` entries. The public API reaches the split;
no renderer fix is needed. This startup/capture gate is not a performance
measurement and does not establish the partition of the prior 3.01 ms.
`ara.exe` SHA-256:
`a823cce2a60dffa370167e1d2048e838f41137c79ca4cd0af713857eafb78bad`.

The first admission exited before rendering because the legacy
`ara-wscene` C# artifacts failed ABI validation. The successful visit uses
P198's `ara-pr169/tree/dotnet/artifacts/bin` companions, as specified below;
keep that pairing. The failure log is retained as `start-abi-failure.log`.

## Common environment and scene

Run from `D:/Coding/renderlab-work/ara-integration/samples-scratch/outpost`.
Scene `levels/sun2_auth.scene`, camera `Follow camera`, 1920x1080, EV100 7.8.
Preserve the current cooked scene, day cycle, custom programs, GI and
reflection settings. Do not force the standard-program diagnostic.

Exact PowerShell common environment, in a fresh process per row:

```powershell
$env:ARA_BIN='D:/Coding/renderlab-work/ara-pin/bin-P198'
$env:ARA_CONFIG_DIR='D:/Coding/renderlab-work/ara-integration/profile/config'
$env:ARA_CRASH_DIR='D:/Coding/renderlab-work/u-sunrt/outpost-split/crashes'
$env:ARA_DOTNET_ARTIFACTS='D:/Coding/renderlab-work/ara-pr169/tree/dotnet/artifacts/bin'
$env:ARA_ALLOW_STALE_COMPANIONS='1'
$env:RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK='I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK'
$env:RENDERLAB_SUN_ZONE_STOP='0'
Remove-Item Env:ARA_RENDERER,Env:ARA_SCENE_RENDERER,Env:VK_INSTANCE_LAYERS,Env:VK_LAYER_PATH,Env:RENDERLAB_ABLATE,Env:RENDERLAB_SUN_RAYS_PROBE -ErrorAction SilentlyContinue
```

No ARA_RENDERER override matches integration's `op-rt` cell. Verify its path
and adapter in the log. Create the output root and each TAG directory first.
Use a distinct TAG for each round/view/row, for example `r1-held-A`.

```powershell
$env:RENDERLAB_PASS_LOG="D:/Coding/renderlab-work/u-sunrt/outpost-split/$tag/pass.jsonl"
$env:ARA_RENDERLAB_CPU_LOG="D:/Coding/renderlab-work/u-sunrt/outpost-split/$tag/cpu.jsonl"
```

Exact row-specific environment:

```powershell
# A: clean fused control
$env:RENDERLAB_SUN_PASS='0'
Remove-Item Env:RENDERLAB_PASS_LOG_DETAIL -ErrorAction SilentlyContinue
# B: clean split (a separate process)
$env:RENDERLAB_SUN_PASS='1'
Remove-Item Env:RENDERLAB_PASS_LOG_DETAIL -ErrorAction SilentlyContinue
# C: separate attribution process; never use its inflated span as frame cost
$env:RENDERLAB_SUN_PASS='1'
$env:RENDERLAB_PASS_LOG_DETAIL='1'
```

## Clips and frames

Held view: 400 stability frames precede the video; the video has the
player's existing 30-frame warmup plus 120 held frames. Analyze video IDs
40..119 (80 requested samples), joined to actual renderer IDs. Clean A/B
rows keep no video PNGs. A separate C picture visit may retain frame 119.

```powershell
$env:ARA_VIDEO_KEEP='999999' # clean A/B; use '119' for the separate C picture visit
& 'D:/Coding/renderlab-work/ara-pin/bin-P198/ara.exe' capture --scene levels/sun2_auth.scene --camera 'Follow camera' --path "D:/Coding/renderlab-work/u-sunrt/outpost-split/$tag/frame.png" --width 1920 --height 1080 --ev100 7.8 --stability 400 --video 120 --video-move 0,0,0 --video-yaw 0
```

Motion/cut: integration's established 120-frame move and frame-60 cut.
Analyze pre-cut IDs 40..59 (20), recovery 60..99 (40), recovered 100..119
(20) separately. Clean A/B rows keep no video PNGs. Only C's diagnostic
visit retains the sparse pictures; do not use its span as frame cost.
Do not combine these windows.

```powershell
$env:ARA_VIDEO_KEEP='999999' # clean A/B; C picture visit uses '0,40,59,60,61,70,119'
& 'D:/Coding/renderlab-work/ara-pin/bin-P198/ara.exe' capture --scene levels/sun2_auth.scene --camera 'Follow camera' --path "D:/Coding/renderlab-work/u-sunrt/outpost-split/$tag/frame.png" --width 1920 --height 1080 --ev100 7.8 --stability 400 --video 120 --video-move 4,0,-4 --video-yaw 12 --video-cut-at 60 --video-cut-move -20,0,15 --video-cut-yaw 140
```

Run A,B then B,A as two interleaved rounds, same executable and settings.
Each `n8-run.py --timing` lease stays under five minutes; split by round or
view if necessary. C is a separate diagnostic visit after clean A/B rows.
Do not nest a wrapper that takes a second GPU lock inside a lease. PC
integration schedules this after its current chain; this unit queues only
the explicitly authorized short dispatch gate, not a timing matrix.

For an explicit child deadline, use one child per round: round 1 runs A,B;
round 2 runs B,A. Give each child a 240-second total deadline inside its
five-minute timing lease, with at most 110 seconds per capture and the
remaining time for logging/cleanup. If a visit cannot finish, preserve its
completed predecessor and report the incomplete row rather than starting a
third visit or silently reusing an earlier result. The required clip is the
120-frame Outpost motion/cut command above with `--stability 400`; its args
match integration's `fpsrun.sh` Outpost CUTS. A held-view pair is a separate
view/lease if needed to match the earlier held 3.01 ms observation. C runs
separately with its own deadline; never append its replays to a clean pair.

## Logs, attribution and required output

Per TAG collect complete stdout/stderr and exit status, pass/CPU JSONL,
sparse PNGs, replay metadata, executable SHA-256, Ara/Renderlab SHAs, flags,
raw SM/memory clocks and contention. Keep raw per-frame JSONL summaries.

Join CPU, pass and images by renderer ID plus the explicit application-frame
mapping. Sort, reject duplicates, state actual/requested coverage per
window, and list dropped/out-of-order samples. Never infer cut boundaries
from `passes[-120:]` or timestamp-array position. Missing cut samples leave
attribution unverified until reconciled.

DETAIL replays seven extra shading variants. Its GPU span is inflated;
report shading marginals separately. `shading - shading.without_shadows`
also contains reflection-hit shadow work. For primary-local shadows only,
use paired plain/detail diagnostics with reflections consistently ablated,
or a validated direct-local-shadow term. Do not alter the production A/B
rows or sum overlapping attribution terms into frame time.

Fused visibility contains camera traversal/repair and directional sun rays.
Local-light rays execute in shading, not as a third part of visibility;
local-light restart zones can nevertheless increase the sun ray budget.

Return per round/view/window: camera `visibility` ms, `sun_shadow` ms,
their sum, fused A visibility ms, split overhead (B-total minus A), clean
GPU span, CPU wall ms, unlabelled time/CPU wait, and coverage. Return C's
shading/local-shadow attribution separately.

Do not extrapolate game_scene's 1.12/0.13 ms partition to Outpost's 3.01 ms.
If sun dominates, count Outpost's actual query classes and preserved
reprojection/mover/caster eligibility before changing budgets. If camera
dominates, investigate primary traversal/repair with U-RTVIS. If local-light
shading dominates the full frame, route that separate term to U-LVIS.
Cause and measured milliseconds precede code or a default flip. ZONE_STOP
does not change CPU-held eligibility: settled eight-ray frames may not win.
