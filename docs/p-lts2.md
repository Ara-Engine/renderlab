# P-LTS2: exact local-light dispatch cuts

2026-11-09, `t3/p-lts2`, baseline `7f4f75d5999c58fd90541900a935bcd88d5ef470`.
Checkpoint A changes single-program deferred shading from 4x4 to 8x4 invocations.
**The 3.5 ms target remains unmet.** All retained A image gates, including full
quarry, pass byte identity. Unshadowed performance needs a clean follow-up after
foreign compiler contention; D remains an unretained prototype.

## Retained implementation

- `primary_shade_tiles` and `SHADE_TILE` agree on 8x4 groups. Region dimensions
  use ceiling division and the pixel guard covers the region's final partial group.
- A clear block still has exactly sixteen column writers; the other lanes return.
  Clear and shaded pixels retain their original computations and coverage.
- Mixed-program binning, list capacity and dispatch keep independent 4x4 `LIST_TILE`
  dimensions. User programs retain the same inputs, functions and clustered addressing.

This adds no GPU resource, binding or quality setting. Every pixel retains its own
light evaluation and accumulation order. The 128-light map budget, all contributing
RT local shadows, LC2 softness, source samples, atlas residency and cache keys are
unchanged. No shadow/ray kernel, shading model, receiver or hit record is changed.
No patented transport or sampling method is introduced by this dispatch change.

## Profile and source study

Own warm Nsight GPU Trace: `profile-screen-1-nonrt`, merged baseline, 1080p,
unshadowed ARA1 game-lights camera, four complete frames after 1200 submits.
The primary shading pipeline averages **4.165 ms**; `vsc_claim` and `vsc_missed`
average **1.064 / 1.008 ms**. Pipeline timings and sampled shader states are
different measurements; sample percentages are not additive function times.

| Function | Sample share |
|---|---:|
| `light_sample` | 22.7% |
| `shade` | 15.8% |
| `unit_normal` | 12.2% |
| `rl_light_index` | 7.8% |
| `exposed_product` | 7.2% |
| `brdf` | 3.0% |

The older P-NRT unshadowed trace attributes 42.2% to `exposed_product`; it predates
the fast path already in this baseline. It is attribution history, not a current
timing baseline. The first own 45-second-delay trace captured startup work and
unattributed shader samples; it is excluded. The submit-count trigger and explicit
debug environment follow `tools/p-nrt/plan.py`. No Nsight UI, screenshot or clock
change was used. A foreign compiler appeared near the warm trace's completion;
these timings select an experiment rather than establish a quiet performance claim.

[NVIDIA documents 32-thread warps](https://docs.nvidia.com/cuda/cuda-programming-guide/03-advanced/advanced-kernel-programming.html).
The previous shading group supplies sixteen invocations. The retained group supplies
thirty-two; its practical benefit is measured below, not inferred from warp width.
No achieved occupancy counter was recovered.

Read-only Ara study: `shaders/baseline.slang::tileMain` and
`shaders/look-shading.slangh` in
`C:/Users/snowy/.t3/worktrees/snengine/t3-9c4a1f92`. Read-only Unreal 5.7 study:
`LightGridCommon.ush` and `ClusteredDeferredShadingPixelShader.usf`, including its
register-pressure motivation for separating shading-model work. See Epic's public
[frustum-grid description](https://dev.epicgames.com/documentation/en-us/unreal-engine/forward-shading-renderer-in-unreal-engine).
No Epic or Ara source was copied. Renderlab already caches static local maps and
uses conservative clustered lists; those implementations remain intact.

## Initial paired screen

RTX 4070 SUPER, Windows/Vulkan, optimized release, native 1920x1080. Synchronous
submission-to-idle frame service excludes capture/hash/PNG work. Original 88-frame
ARA1 history is followed by a cut back to the original camera, 128 warm frames and
eight measured still frames. These are frame-service medians, not isolated light costs.

`runs/screen-2-a/summary.json` records the complete pairs. This is a **contended
diagnostic screen**; compiler activity overlaps the lease, so its numbers are not
a final quiet performance certification.

| Steady frame service, ms | Baseline | A |
|---|---:|---:|
| RT shadowed | 10.020 | 6.417 |
| RT unshadowed | 8.626 | 4.502 |
| Non-RT shadowed | 15.612 | 13.453 |
| Non-RT unshadowed | 9.886 | 9.321 |

All four pairs' original still/motion/cut witnesses and final tail are HDR-byte and
PNG-byte identical. These paired baselines replace neither P-LTS's older measured
baseline nor Ara's measured values. All four remain above 3.5 ms.

The final `a-final-1` lease supplies two uncontended pairs. Per-process intervals
in `a-final-timing-context.json` contain zero observed rustc processes and at least
10.3 GiB available throughout these four runs:

| Steady frame service, ms | Baseline | A |
|---|---:|---:|
| RT shadowed | 10.098 | 6.632 |
| Non-RT shadowed | 15.864 | 13.685 |

Both final unshadowed pairs are excluded from performance claims: foreign compiler
counts rise to 1–2 on RT and sixteen on non-RT. The non-RT baseline's eight samples
range from 10.68 to 56.42 ms. Their images still match. Only those unresolved timing
pairs need repeating; the completed quality gates do not.

Quarry timings are also contended and do **not** establish a quarry speedup. Its
soft-default probes are 13.858 to 14.272 ms RT and 19.869 to 20.943 ms non-RT;
hard controls are 13.284 to 11.555 and 15.813 to 14.353 ms. These fixture probes
include readback and differ from ARA1 service. Whether the soft result reflects
group-size cost or contention remains unresolved; do not hide that possible regression.

## Quality ledger

- **198 lib tests passed**, zero failed, including all shading compositions and
  the historical TP1 digest. `a-lib-result.json` records the binary identity.
- 1080p ARA1 game-lights shadowed/unshadowed, both paths: all HDR and PNG witnesses
  identical in `runs/screen-2-a`.
- Original 960x540 ARA1 witnesses, both cases/paths: all HDR and PNG witnesses
  identical in `runs/a-checkpoint-1-quality`.
- Additional 961x541 image/clear-boundary fixture, both paths: all HDR and PNG
  witnesses identical in `runs/a-checkpoint-1-edge`.
- LC1/LC2 point/spot hard/soft and LC3 leakage/fade controls: all **24 candidate
  PNGs** identical, `runs/a-checkpoint-1-fixtures`.
- MP1 standard/toon/custom, eight programs and reload controls: all **fourteen
  candidate PNGs** match across both paths.
- Night courtyard with the Fox, default effects and unchanged lights, sixteen
  still and thirty-two moving frames per path: all **sixteen candidate PNGs** match.
- Full registered quarry, 13,034,903 source triangles and 261 lights: **28 float-HDR
  and 56 candidate PNG witnesses** match across both paths. This includes the
  sixteen-frame camera walk, moving lights, hard/soft and legacy controls, GI on/off,
  day and dusk. Cluster mean 3.7766, longest 99, zero CPU mismatches/overflow and
  zero unclustered differing channels are unchanged; visible-light GI populations
  also match exactly. Evidence: `runs/a-final-1-quarry/summary.json`.

All completed short image pairs share lease `a-checkpoint-1`. Its completed image
prefix has no failed assertion or mismatch. The runner stopped at its bounded
command deadline (512.36 s) just as the separate final timing step began; that
incomplete timing attempt supplies no performance evidence.

Inspected contact sheets: `inspection/a-game-lights.png`, `a-lc.png`, `a-mixed.png`,
`a-night.png`, and both `a-quarry-*.png` sheets, plus full frame 031 courtyard images
on both paths, RT quarry night and non-RT final walk presentation. Silhouettes,
shadows, highlights, soft penumbrae and transparency retain the baseline image.
Existing grain and the courtyard glass rectangle remain baseline defects. Byte
identity preserves reference-error populations; this establishes no new Unreal or
film-quality claim.

## Other trials

- B copies at most 128 exact final atmosphere-scaled light records into spare space
  in the existing cluster uniform, falling back to storage for larger sets. This is
  a cache capacity, not a light-count or shadow budget. Prototype frame size is
  61,552 bytes and the API signatures remain unchanged.
- C adds an immutable last-word cache to avoid fetching the same packed cluster
  word for consecutive even/odd light indices. Its key includes cluster and word.
- Initial B/C binaries failed during composition before rendering: changing the
  `hard_shadow(lights[s.index],...)` return text broke the reflection injection seam.
  Restoring that return line fixes the source seam without editing reflection code.
  Neither initial binary provides quality or timing evidence.
- A receiver-constant BRDF preparation override passed one RT shadowed image pair,
  but its timing baseline was contended and the run stopped on a new root order.
  It is unretained; the current profile attributes only 3% to `brdf`.

The fixed combined D prototype is saved in the ignored evidence directory. The
checkpoint production diff contains only A. Any later promotion requires its own
image gates and the 128/129-light uniform/storage boundary checks.
Its manual reference-probe composition also needs a storage-backed `rl_light_record`
stub; that source fix passes the full **199-test** prototype suite. No D GPU result
is claimed yet.

## Reproduction and resource policy

All own output, staged binaries and the per-tree target are under
`D:/Coding/renderlab-work/p-lts2/`. Cargo uses `-j 2`. Each GPU command runs through
`scripts/p-lts2-run.py` and `scripts/n8-run.py`, at most fifteen minutes per lease.
The latest anchored root order is read before admission and handoff gates match
actual owner-prefixed release lines. Compiler-quiet holds are honored.

Root explicitly amended admission to **7 GiB for short non-quarry leases**;
full-quarry runs retain **8 GiB**. The standard **2 GiB runtime guard** is unchanged.
The fixture driver rejects a full-quarry run under a smaller admission lease.

The first quiet screen was stopped by foreign compiler contention after its warm
profile, before any scored A/B pair. Remaining diagnostic screens allow compiler
contention and record it. A root-order parser mismatch in P-FLAT's queue gate was
identified with a CPU-only reproducer; the corrected order unblocked the queue.
`a-final-1` completes in **272.48 s**, with 8 GiB admission and minimum available
memory **2447.6 MiB**, above the unchanged 2 GiB runtime guard. Foreign compiler
count reaches nineteen later in that lease; only the clean shadowed timing prefix
is retained as uncontended evidence. Root's hold starts at the runner's actual
owner-prefixed admitted line. Commands honor later orders explicitly allowing
running leases to continue.

Automatic approval review rejected deletion of the two summarized raw GPU traces
with **"blocked by policy"**; nothing was deleted. `summarized-traces.json` records
their exact paths, sizes and SHA-256 hashes for root cleanup. Target cleanup is pending
completion of the unit. Inspected PNGs, JSON/CSV and logs are retained separately.

Development and verification time are separate from frame measurements: A's warm
example build took **52.08 s**, and its retained lib-test build took **1m57s**.
The lib run took **3.82 s**. GPU profiling, interrupted screens and queue waits are
recorded by the lease JSONs; no claim about general development-loop latency is made.
