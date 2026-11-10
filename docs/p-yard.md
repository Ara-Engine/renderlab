# P-YARD: full-frame yardstick against Unreal (Lumen + TSR) — PARKED

Milestone 8 measurement unit, branch `t3/p-yard` on main `a807b01`, 2026-11-10. No renderer code changes.

**Status: parked by root order before any GPU or build work** ("Showcase profiling for Ara comes first, so PARK P-YARD now"). The unit stopped during the reading phase. No build slot, GPU lease, Cargo target, capture or dump was taken or created, so nothing needed cleaning up. Nothing below is a new measurement: every number is quoted from the earlier docs named next to it.

## Goal (unchanged, for the resume)

One matched scene; still, fixed-path motion and a camera cut; Renderlab RT and non-RT against Unreal at Low/Medium/High/Epic/Cinematic on Lumen HW and Lumen SW (D3D12 SM6); the same resolution and GPU frame-time method for both engines; a high-sample path-traced reference; full-frame error per configuration and state; an error-vs-ms chart (PNG + CSV); eye-judged crop sheets of Unreal's known flaws (ghosting, disocclusion trails); and a per-path, per-state position against "Unreal's image at about 3x its speed", with a ranked gap list for `docs/m7-gaps.md`.

## Stopped at

The reading phase was done: AGENTS.md, `docs/milestone-8.md`, `docs/m7-ara1-plan.md`, `docs/m7-u0.md` in full, `docs/m7-u0b.md` (summary, references, floors) and the six named memories. Not yet read: `docs/m7-u1*.md` beyond their summaries, `docs/ue-sky.md`, `docs/quarry-end-to-end-comparison.md`, `docs/unreal-comparison-setup.md`, `docs/unreal-quarry-full-detail-comparison.md`, `docs/m7-ara1-report.md`, `docs/m7-gaps.md`.

## What exists and can be reused

**Scene choice: use the quarry.** It is the only scene with a matched Unreal project, Lumen-capable cooked packages and 1080p path-traced references. The ARA1 scenes (game-base, Ara PBR) have **no Unreal port**. Porting one would cost an import, a cook and a lighting match, which is most of the time box.

| Asset | Location | Notes |
|---|---|---|
| Matched UE quarry project, e2e launches | `D:/Coding/renderlab-work/ue-m5/Quarry` (`launches/e2e-matched-s3-*/process.json` hold the command lines) | Original matched package. It has **no mesh distance fields**, so software Lumen cannot run on it. |
| Lumen-capable quarry package (DF + path tracer) | `E:/renderlab-work/m7-u0/ue-package`, project copy `E:/renderlab-work/m7-u0/ue-project` | Cooked by U0 with `r.GenerateMeshDistanceFields=True`, `r.DistanceFields.SupportEvenIfHardwareRayTracingSupported=1`, `r.PathTracing=True`. UE 5.7.4 CL51494982, packaged Development, D3D12 SM6, `-RenderOffscreen`, hidden. |
| UE bench package (SV, S4, S5, S2G) | `E:/renderlab-work/m7-u0b/ue-package` | `/Game/M7U0b/{SV,S4,S5,S2G}/Bench` |
| UE configuration study driver | `scripts/m7-u0-ue.py` (`study`, `summarize`; `BASE_COMMANDS`) | Proves every run's settings by `DumpCVars` against `BaseScalability.ini`, and reads the tracing mode from RDG pass names. It already has static, camera-alternation, pan (2 cm/frame, captures at 97/112/128/159), sun-step and sky-yaw workloads. |
| U0b quiet timing and floor runs | `scripts/m7-u0b.py`, `m7-u0b-score.py`, `m7-u0b-report.py`; evidence under `E:/renderlab-work/m7-u0b` | 80-process static/camera/sun quiet campaign. The cut clears TSR history through the camera-cut flag and keeps Lumen's world state. |
| Shadow-only UE timing | `scripts/ue-shadow-motion.py` | See the memory `unreal-shadow-cost-measured`. |
| UE memory | memory `memory-budget-vs-unreal` | `GPUMem/LocalUsedMB` CSV column. |
| 1080p quarry path-traced references | `E:/renderlab-work/m7-u0b/reference/quarry-final` (`reference-coverage.json`) | `renderlab::reference` with full 13.0M triangles, textures and the RLE2 environment. quarry-p0/p1 (± sky-only) at 4,096 spp have a median relative noise of about 0.29% and take about 22 s per call. There are 123 archives across 162 rows, including motion/disocclusion states. **Their content identity must be rechecked against the current base before scoring** (memory `check-evidence-binary-base`). |
| Reference tracer validation | `docs/m7-u0.md` §2, `docs/m7-u0b.md` | Analytic gates and Mitsuba 3.9.1 cross-checks pass. **Recorded choice for this unit: Renderlab's reference tracer.** It already renders the matched quarry; UE's path tracer was only timed in U0 (about 7.6 ms per 1-spp frame). |
| Renderlab-side scoreboard | `tools/ara1/` (`images.py compose/score`, `lease.py`) and `scripts/n8-quarry-score.py` | `images.py score` reuses `scan-quality-compare.py` linear/statistics. Swap the Ara column for UE. |

**Unreal numbers already measured** (quarry, 1080p, RTX 4070 SUPER, NVIDIA 617.14, `docs/m7-u0.md` study/c, contended block; U0b's quiet campaign refines them):

| Config | Static GPU p50 (ms) | LocalUsed MiB |
|---|---:|---:|
| TSR only, VSM | 3.12 | 4,293 |
| SW Lumen High | 3.77 | 5,038 |
| SW Lumen Epic (non-RT floor) | 4.04 | 5,142 |
| SW Lumen Cinematic | 6.46 | 5,409 |
| HW Lumen High | 4.13 | 6,211 |
| HW Lumen Epic (RT floor) | 4.69 | 6,444 |
| HW Lumen Cinematic (+ hit lighting) | 6.72 (6.68) | 6,578 |

U0b's clean HW Epic visits range from 4.45 to 4.55 ms. Camera motion showed no systematic Unreal cost. On a 2 cm/frame pan, U0 saw no visible trails or ghosting at display scale on this scene. **So the quarry's slow pans may not show Unreal's ghosting at all.** The ghosting crops probably need a mover or a fast-disocclusion path; the S4 door and S5 cube benches already exist in the UE bench package.

**Error against reference already measured for UE** (`docs/m7-u0b.md`, frame population, G1 sky-visibility study): SW Epic with global DF 35.9%/68.3%, SW Epic with mesh SDF 38.9%/87.2%, HW Epic 10.7%/11.2%. This is a population metric for one effect, not the full-frame displayed error this unit needs.

## Gaps in existing coverage (this is the unit's actual work)

1. **Low and Medium presets were never run.** At those GI levels UE turns Lumen off (no GI, or SSGI and screen-space reflections), so "Lumen HW vs SW" does not apply there. Record them as single rows marked "Lumen not active", read back with `DumpCVars`.
2. **No full-frame displayed-error score** for any UE preset against the quarry reference, in any state.
3. **No current-base Renderlab row** on the same quarry poses, path, cut and timing method. GPU frame time for both engines: UE CSV `GPUTime`; Renderlab `render_span` from the ARA1 adapter.
4. **No error-vs-ms chart or CSV.**
5. **No eye-judged ghosting/disocclusion crop sheets** for UE versus Renderlab.

## Next step (resume plan)

1. Read the remaining docs listed in "Stopped at", especially `m7-ara1-report.md` and `m7-gaps.md`, so known gaps are not duplicated.
2. On CPU: check that the U0 package's quarry poses match `quarry-final`'s scene identity. Write `tools/p-yard/` with a manifest of poses, the pan path and the cut frame shared by both engines; a UE wrapper over `m7-u0-ue.py study` for the preset × {HW, SW} × {static, pan, cut} matrix; and a scorer and plotter over `tools/ara1/images.py score`.
3. GPU, through `scripts/n8-run.py` in leases of at most 15 minutes, after the Ara PR gate, the Ara integration thread and P-PREVIEW: (a) a 2-launch UE smoke checking frame brightness (pre-exposure override 0); (b) the UE matrix (about 10 configs × 3 states, at about 10 s per run, fits in 1-2 leases); (c) Renderlab RT and non-RT on the same poses with a release binary; (d) references for any pan or cut frame not covered by `quarry-final`, at 4,096 spp and about 22 s each.
4. Score, plot, inspect crops (white pixels, noise, aliasing and trails, in motion and on the cut), write the per-path, per-state verdict and add the ranked gaps to `docs/m7-gaps.md`.

**Estimated remaining effort:** about 3 hours, including roughly 30-40 minutes of GPU lease time.
