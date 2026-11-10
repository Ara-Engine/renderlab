# P-CUT: spread the cut's work over the first frames

Unit P-CUT, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-10, branch `t3/p-cut` on main `9e343a0`. Labels: **M** measured, **C** read from code, **I** inference.

## Status: stopped before the first build (host memory exhausted)

The code and tools are written and committed. Nothing has been compiled, timed or inspected yet. The build waiter queued twice behind Ara, p-show2 and p-still, following the 15:02 UTC rules: admission first, the mutex taken only after it passes, and a 6 GiB bar. Before it was admitted it died with `WinError 1455` ("the paging file is too small") while sampling resources. At that moment the whole machine had about 80 MB of free commit (FreeVirtualMemory) and 123 MB of free physical memory. Under the host rules ("if anything dies with an allocation or write error, stop and report it"), the unit stopped and did not requeue. It held no build slot, mutex or lease. A release line is posted in the schedule.

**Verdict: DON'T MERGE yet.** The change is unverified: no compile, lib tests, timing or images.

## Baseline (M, from P-YARD2's saved per-pass timings, main `71ea355`)

The quarry cut spike is almost entirely sun-shadow directions. The other passes are flat across the cut. GPU ms at 1080p, timing visit 0:

| Path | Pass | Frame 319 (moving) | Cut 320 | 321 (after) | 336+ (held, 8 rays) |
|---|---|---:|---:|---:|---:|
| RT | `primary` (Interactive rays) | 1.44 | **9.79** | 1.43 | 0.49 |
| RT | span | 4.21 | **12.56** | 4.26 | 3.42 |
| non-RT | `visibility` (sun march) | 6.46 | **39.78** | 6.19 | 3.74 |
| non-RT | span | 10.67 | **44.43** | 11.02 | 8.16 |

- The rest of the frame (span minus the shadow pass) is 2.77 ms on the RT cut against 2.79 ms on a moving frame. The GI refill adds nothing visible to the quarry's cut frame (**M**), so on the quarry the cut is the 256-direction budget alone.
- Frames 321-335 run the active 32 directions and frames 336 onward the held 8 (`settle_frames` 16).
- Unreal's per-frame post-cut errors for every preset, frames 320-351, are in P-YARD2's `quality.json` and remain valid (same package and manifest). Unreal's post-cut HDR frames were deleted after scoring, so the side-by-side strips need one Unreal capture lease (`ue.py capture quarry ue-hw-e ue-sw-e` with `P_YARD_WORK`).

**Owed:** the rerun on current main (after P-DEFECT and P-SHOW), for the quarry and the game on both paths, plus Showcase.

## Design (C)

**Cut ramp.** `ReconstructionShadowBudget` gains `cut_rays` (default 32).
- With direction rotation on (the default), a cold start traces `cut_rays` directions in the cut frame and in each of the next `cold_rays / cut_rays - 1` frames: 8 frames of 32.
- Ramp frame *p* rotates by `base + 32p`, where `base` is the cut frame's normal rotation. Each pixel's R2 sequence index is unwrapped on both paths: RT `fresh_direction(j + sided.y)` and non-RT `vsm_direction(nsq.y + 8b + j)`. So the eight frames trace exactly the window `[base, base + 256)` that the one-frame cut used to trace, each direction once.
- TR1 accumulates with equal weights while young, so by frame 7 after the cut each pixel's history holds the same 256 directions. Before that, frame *n* holds 32(n + 1). This is the noisy-but-plausible path: a 32-direction cut frame is what every moving frame already shows, with TR1's 3×3 spatial start on revealed pixels.
- After the ramp the normal rotation (`8 (k mod 32)`) and budgets resume.
- A full restart during the ramp ends it. A partial restart (P-FLAT's moved-caster or relit pixels) keeps at least `reset_rays`.
- `cut_rays = cold_rays` restores the one-frame cut exactly: the rotation helper then always returns `rotation(k)`.

**Implementation:**
- `src/temporal_reconstruction.rs`: the `cut_rays` field, `CutRamp`, `Renderer::shadow_rotation` and the budget choice in `prepare_reconstruction`. A CPU unit test checks that the ramp's windows cover the cold window exactly once.
- `src/primary_ray_gpu.rs` and `src/virtual_shadow_gpu.rs`: one line each, so the rotation word comes from `shadow_rotation`.
- No shader, GI-cache or VSM data change.

**Expected cost (I, from the baseline):**

| Path | Cut frame today | Cut frame with the ramp | Moving frame | Unreal Epic cut |
|---|---:|---:|---:|---:|
| RT | 12.6 ms (2.8x slower than Epic) | about 4.3 ms (about 1.05x faster) | 4.26 | 4.52 (HW) |
| non-RT | 43.1 ms (10.5x slower than Epic) | about 11 ms (about 2.7x slower) | 10.57 | 4.11 (SW) |

**Side effects (C):**
- S4 (`sunvis_s4::chosen`, 8 or more batches) no longer runs on cut frames: a ramp frame has 4 batches. It still runs on 64-ray reset frames where it is chosen.
- RT glossy reflections use the same frame word as their stratum seed. Ramp frames give distinct seeds, but one seed repeats about six frames later (I: negligible).
- **GI fill.** Nothing changes. The quarry shows no GI cut cost. The game and Showcase cut costs still need measuring before any GI spreading is designed. The GI files belong to P-SHOW2.

**Rebase note.** P-STILL's branch edits the same `virtual_rotation` line (a `norotate` filter). The merge keeps their filter and calls `shadow_rotation`. P-STILL's `p_yard_capture.rs` count hooks merge beside P-CUT's two env switches.

## Harness (committed, untested)

- `examples/p_yard_capture.rs`: `P_CUT_RAMP=0` gives the one-frame cut; `P_YARD_CAPTURE_FROM=N`; per-frame `shadow_rays` and the budget go into `engine.json`.
- `examples/p_nsq_screen.rs`: `budget=c/r/a/h[/cut]`, where cut defaults to cold, as before.
- `tools/p-yard/{host,rl,score,ue}.py`:
  - `P_YARD_OWNER`, `P_YARD_WORK`, `P_YARD_AFTER` and `P_YARD_MIN_MIB` settings;
  - `rt-base`/`nonrt-base` paths;
  - `P_YARD_BUILD=lib-tests`.
- `tools/p-cut/plan.py`: `quarry`, `game` and `lib` lease plans.
- `tools/p-cut/game.py`: ARA1 game-base, one process per variant and visit.
- `tools/p-cut/cut.py`: `curves` (per-frame error and ms after the cut against P-YARD2's Unreal rows, CSV and PNG), `strips` (crops plus 60 fps APNGs) and `game` (P-NSQ3's ARA1 scorer, copied to `D:/Coding/renderlab-work/p-cut/tools/`).

## Resume

All env vars: `P_YARD_OWNER=p-cut P_YARD_WORK=D:/Coding/renderlab-work/p-cut P_YARD_AFTER=p-show2,p-still P_YARD_MIN_MIB=6144`, with TMP/TEMP under the work directory.

1. `py -3 -B tools/p-yard/host.py build` (release examples), then the same with `P_YARD_BUILD=lib-tests`.
2. `host.py lease pcut-q 15 -- py -3 -B tools/p-cut/plan.py quarry` (about 9-10 min estimated). Then `pcut-g ... plan.py game`, then `pcut-l ... plan.py lib`.
3. One Unreal capture lease for HW and SW Epic, then `ue.py decode quarry`.
4. On the CPU: `score.py quality`, `score.py timing`, `cut.py curves`, `cut.py strips X0,Y0,X1,Y1`, `cut.py game`. Inspect the strips for banding, holes and stale-view ghosts.
5. Showcase needs an Ara build patched to this branch (`tools/showcase`, owned by P-SHOW2); it was not attempted.
