# Ara first-cut pipeline inventory

Use **bin-P201**, `D:/Coding/renderlab-work/ara-pin/bin-P201`, from Ara #201,
pinning Renderlab `faccf9b6` (merged #119 log and #120 prewarm, default off).
Ara #201 is merged at 2f9eff09; P201's Showcase stills are byte-equal to P200 on both paths. P200 does not contain this log. Run only through PC integration's normal
GPU admission; this unit does not launch this recipe.

Use integration's freshly imported Ara-main projects, not an older sun scene:
Outpost `samples-scratch/outpost/content/levels/outpost.scene` (10-degree sun),
Showcase `samples-scratch/showcase/content/levels/showcase.scene`. Record their
absolute physical paths and SHA-256 plus Ara head, Renderlab pin, executable
hash and companion identity in the row manifest. Do not write into another
unit's project; PC integration runs its own copies. Keep every PNG/crop/sheet.

Verified byte-identical to Ara `origin/main` (`eb8cc457`) on 14 November:

| Physical scene file | SHA-256 |
| --- | --- |
| `D:/Coding/renderlab-work/ara-integration/samples-scratch/outpost/content/levels/outpost.scene` | `f3583f2f9cb02a5fdae641214db4e9e89792f8f3185200708d92a5b760d2e386` |
| `D:/Coding/renderlab-work/ara-integration/samples-scratch/showcase/content/levels/showcase.scene` | `ac84a12aa4a3dd4c8979140fab169db080b4077e4f83c4fb1fecb33019921165` |

Recheck these physical hashes before running; if Ara main changes the scene,
refresh from main and record the replacement hash. Do not use `sun2_auth.scene`
or an older 4-degree-sun Outpost variant.

Before each fresh player process, in PowerShell (replace BIN, OUT and TAG):

```powershell
$bin='D:/Coding/renderlab-work/ara-pin/bin-P201'
$env:ARA_BIN=$bin
$env:ARA_DOTNET_ARTIFACTS='D:/Coding/renderlab-work/ara-pr169/tree/dotnet/artifacts/bin'
Remove-Item Env:RENDERLAB_PREWARM_PIPELINES -ErrorAction SilentlyContinue
$env:RENDERLAB_PIPELINE_LOG="$out/$tag/pipelines.jsonl"
$env:RENDERLAB_PASS_LOG="$out/$tag/passes.jsonl"
$env:ARA_RENDERLAB_CPU_LOG="$out/$tag/cpu.jsonl"
$env:ARA_VIDEO_KEEP='0,59,60,61,119'
Remove-Item Env:VK_INSTANCE_LAYERS,Env:VK_LAYER_PATH,Env:RENDERLAB_PASS_LOG_DETAIL,Env:RENDERLAB_STARTUP_PROFILE,Env:ARA_SCENE_RENDERER -ErrorAction SilentlyContinue
# RT row: remove ARA_RENDERER. Non-RT row: set it to renderlab-nonrt.
Remove-Item Env:ARA_RENDERER -ErrorAction SilentlyContinue
```

Exact switch rows, each in a fresh player process and output directory:

```powershell
# Off control: logging ON, prewarm OFF.
$env:RENDERLAB_PIPELINE_LOG="$out/$tag/pipelines.jsonl"
Remove-Item Env:RENDERLAB_PREWARM_PIPELINES -ErrorAction SilentlyContinue
# On treatment: logging ON, prewarm ON.
$env:RENDERLAB_PIPELINE_LOG="$out/$tag/pipelines.jsonl"
$env:RENDERLAB_PREWARM_PIPELINES='1'
# Non-RT path, for either treatment (RT instead removes this variable).
$env:ARA_RENDERER='renderlab-nonrt'
```

Set the C# companion to the exact pin's build if that pin changes it. Preserve
the scenes' own shaders, lighting, GI and reconstruction. Use a new output
directory for every row because the pipeline log appends; create it beforehand.

Outpost, run from its project directory:

```powershell
& "$bin/ara.exe" capture --scene levels/outpost.scene --camera 'Follow camera' --path "$out/$tag/frame.png" --width 1920 --height 1080 --ev100 7.8 --warm 590 --stability 400 --video 120 --video-move 4,0,-4 --video-yaw 12 --video-cut-at 60 --video-cut-move -20,0,15 --video-cut-yaw 140
```

Showcase, run from its project directory:

```powershell
& "$bin/ara.exe" capture --scene levels/showcase.scene --camera 'Camera' --path "$out/$tag/frame.png" --width 1920 --height 1080 --ev100 10.17 --stability 400 --video 120 --video-move 2,0,0 --video-yaw 20 --video-cut-at 60 --video-cut-move -3,0,3 --video-cut-yaw 140
```

Ordinary inventory: each scene on both paths. Fresh player per row, one round
first. Collect pipeline JSONL, pass JSONL, CPU JSONL, full player stdout/stderr,
capture metadata, sparse PNGs and the manifest. Join pipeline frame markers to
pass/CPU renderer IDs, then to video metadata. Assign compile events preceding
a frame marker since the previous marker to that frame. The first marker also
contains load-time compilation. Report every new render/compute key at the cut
and +1, and resource/build deltas; distinguish actual compile events from cloned
resource handles. No GPU query/readback is added by the pipeline log itself.

P201 contains prewarm; repeat each row with
`RENDERLAB_PREWARM_PIPELINES=1`, interleaved off/on then on/off. Require the
pipeline keys to move to load; do not infer GPU spike removal from a flat count.
Compare cut-frame GPU passes and CPU encode time separately. Take clean final
performance rows with `--timing` leases under five minutes each, split by scene
and round if needed; omit retained video PNGs there (`ARA_VIDEO_KEEP=999999`).
The compilation log is diagnostic CPU I/O: clean final timings may leave it off
after the ordinary run establishes which pipelines moved.

Require zero native render/compute compilation events at the cut and +1 with
prewarm on. Compare the +1 picture off/on for identity on both paths. The default
flip waits for these Ara rows: renderer-fixture timing alone cannot establish
Outpost or Showcase's worst cut frame, which also carries bridge work.

The game-facing explicit call after resources/settings are loaded is
`engine.prepare_pipelines()?`. It performs CPU compilation only, without a
camera, rendering, or advancing histories/particles. Its return value reports
CPU duration, pipeline counts and additional owned buffer/texture bytes. Driver
pipeline memory is not included. Call again after loading another level or
replacing programs. The environment switch auto-calls it before the first view
on the wgpu engine; the Apple port is not changed by this prototype.

## Outpost P200 input and conditional inventory

Source: `D:/Coding/renderlab-work/measure/p200-b/table.md`, Ara eb8cc457,
Renderlab 50fc872a / bin-P200, main Outpost scene hash
`f3583f2f9cb02a5fdae641214db4e9e89792f8f3185200708d92a5b760d2e386`.
The RT cut window (clip rows 60-99) reports wall median 16.91 ms,
p95 20.52 ms and worst 45.63 ms across the recorded three rounds.
This is baseline context, not evidence that a pipeline caused the worst frame;
GI refill and host work are also possible contributors. P200 lacks the new log.

PC integration's cut-back-cut replay on P201 should keep the first arrival at B
separate from its later arrival. Record its actual replay arguments and frame
mapping; the single-cut commands above do not themselves provide that revisit.
For each arrival, join the exact CPU frame to the pipeline frame marker, compile
labels/keys and call durations, resource/AS deltas, and GPU passes. The source
table uses pass frame = CPU frame + 1; verify that relationship in the new row
rather than assigning by line position or ignoring the warm/stability frames.

Add Outpost to the confirmed pipeline-variant inventory only if the first
arrival has native render/compute creation events. For each such key, record
creation order, CPU compile ms, why load did not reach it, and whether it is
reused at the second arrival. With prewarm on, require the same reachable keys
to be prepared at load and zero new creations at the cut and +1. If both
arrivals have zero creations, report that result and retain the worst-frame
investigation as GPU/host/cache work rather than extending the pipeline list.
