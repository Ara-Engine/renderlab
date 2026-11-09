# P-NRT2: non-RT frame cost, second pass (zero quality loss)

Unit P-NRT2, 2026-11-09, branch `t3/p-nrt2` from main `a5ae72a` (after P-NRT, P-GI and P-LTS2 A).
Islam's bar: "without compromising even a bit in quality". A pure optimisation is byte-identical;
anything else is equal or better on every crop and population; no quality setting is lowered.

Scope: the non-RT frame, excluding three areas owned by other running units:
- the sun-shadow files (P-NSQ);
- the local-light loop, light_sample and light_cluster* (P-LTS2);
- the TR1 history keys (P-FLAT).

Owned here: the GI cache, fog and atmosphere, post, R1n reflections, frame scheduling and the
rest of shading. Root added one quality item: camera cuts did not reset volumetric fog history.

Labels: **M** measured here, **A** author-reported elsewhere, **I** inference.

## 1. Status

- **Speed:** two exact optimisations land (environment lookups, GI software-ray loads). Both are
  byte-identical, and neither moves the non-RT frame measurably.
  - Under a physical sky, the shading pass's minimum and first quartile fall about 1-2%.
  - The GI software-ray change shows no measurable change in ARA1's quarry span.
  - The non-RT frame is still 4-5x Ara raster's. The target is **missed** (§5 ranks what remains).
- **Quality:** root's fog-cut fix lands. A camera cut no longer reprojects the previous view's
  volumetric fog. On the pixels it changes, the cut frame moves toward a fresh render of the
  same view on both paths, and every non-cut frame is byte-identical.
- No quality setting changed.

## 2. Attributed profile of main `a5ae72a` (M)

Method: Nsight Graphics 2026.4.0 GPU Trace, P-NRT's recipe and scenes unchanged:
- `tools/p-nrt/plan.py` traces with `--start-after-submits 1200`, four submits and the
  real-time shader profiler, on P-NRT's long single-phase scenes;
- the binary is the release `ara1_capture` of main;
- `tools/p-nrt/attribute.py` assigns every microsecond of each frame.

Environment: RTX 4070 SUPER, NVIDIA 617.14, 1920x1080 non-RT, inside lease p1 (the only GPU job).
Values are medians of four frames, in ms.

| Scene / phase | GPU frame | CPU gap | In-frame gaps | Shading | Sun visibility (P-NSQ) | TR1 | VB | GI cache passes | Sun pages | Atmosphere | Post |
|---|---|---|---|---|---|---|---|---|---|---|---|
| game-base still | 6.08 | 2.11 | 0.18 | 2.03 | 2.54 | 0.44 | 0.37 | 0.26 | 0.19 | — | 0.03 |
| ara-pbr still | 5.10 | 1.85 | 0.20 | 2.53 | 1.38 | 0.35 | 0.28 | 0.17 | 0.11 | — | 0.03 |
| quarry, every frame a cut | 3.60 | 1.82 | 0.27 | 0.66 | 0.69 | 0.46 | 0.69 | 0.53 | 0.11 | — | 0.03 |
| game-atmosphere still | 8.56 | 1.83 | 0.19 | 3.95 | 2.55 | 0.48 | 0.62 | 0.38 | 0.22 | 0.06 | 0.03 |

Light clusters and wgpu's internal passes are each 0.01 ms or less.

- The game-base move trace was dropped to fit the lease, so moving frames use ARA1's per-pass
  timestamps (§4). P-NRT's base traces put moving frames 0.3-0.6 ms above still frames, in
  shading and TR1.
- **What this steady state leaves out.** The traces sit at about frame 1200, when the GI cache
  has settled. ARA1 times frames 1-87, while it is still filling. ARA1's spans run 1.7 ms
  (game-base) to about 7 ms (quarry) above these frames, and P-NRT §1 attributes that excess to
  GI fill and the gather drain.

**Inside the shading pass** (shader-profiler sample shares, sums of the hot lines):

| Trace | R1n traversal | Environment lookups | GI lookup | Other |
|---|---|---|---|---|
| game-base still | about 31% | — (uniform sky) | about 10% | material, normal and local-depth lines |
| game-atmosphere still | about 16% | about 26% | about 4% | `atmosphere_surface`, material |

- **R1n traversal** lines: box slabs, the triangle test, stack push and pop, `rs_q`, the leaf
  and instance loops.
- **Environment lookup** lines: `environment_tap`, `environment_remap`, `environment_face_uv`,
  and `environment_radiance`'s level loop.
- **Hot instructions are loads.** In the environment lookups that is `textureLoad` itself, as
  well as branch bookkeeping. In the traversal, the box and triangle lines sit under long
  scoreboard stalls, as P-NRT found.
- **The CPU gap is 1.8-2.1 ms of the synchronous frame.** Of that, `render()` takes about
  0.85 ms (encode 0.67-0.89, submit 0.08, upload 0.04) and the rest is completion and submit
  latency. For comparison, Ara raster's whole service is 1.7-1.9 ms.

Traces and hotspot JSON: `D:/Coding/renderlab-work/p-nrt2/traces/base-*/`.

## Changes

1. **Environment lookups, straight-line loads** (`shaders/scene.wgsl`). This was P-NRT's main cost
   under a physical sky, where about half of shading's samples were environment lookups.
   - `environment_bilinear`: a footprint whose four texels lie inside its cube face (almost every
     lookup) now loads them in straight-line code, then blends them with the same expression.
     Before, each of the four taps ran `environment_tap` and `environment_remap`, whose in-face
     branch performs exactly these loads.
   - Footprints that touch a face edge or corner keep the old path.
   - `environment_radiance` now loads both blend levels without the local `values` array, so the
     second level's loads can issue alongside the first's.
   - A level that is skipped (zero blend weight) stays the zero the array held, so the
     arithmetic is unchanged.
2. **GI software rays read whole vec4 words** (`shaders/gi_rays_software.wgsl`,
   `src/gi_bvh.rs`, `src/gi_cache.rs`). These rays feed the GI cache's setup, rays, light and
   gather on non-RT, and the capped gather drain sets ARA1's early-frame cost (P-NRT §1).
   - Each mesh's record block is padded to 16 bytes, and the packed buffer to a whole 16-byte
     word, so every instance record, BVH4 node, leaf and position starts on one.
   - `gi_geometry` is bound as `array<vec4<u32>>`. The loads per record drop as follows:
     - a node: 16 to 4;
     - an instance: 23 to 6;
     - a leaf header: 2 to 1;
     - a position: 3 to 1.
   - Index and mask words are read singly through `gi_gw`.
   - The words and the arithmetic are the same, as P-NRT's R1n change.
3. **Fog history resets on camera cuts** (`src/engine/mod.rs`, `src/atmosphere_gpu.rs`).
   - `Engine::cut()` (the camera cut/teleport API, which ARA1's cut frames call) now also calls
     `Renderer::reset_atmosphere_history()`. That function is P-POST's: same text and place as
     on `t3/p-post`, so the branches merge cleanly.
   - Fog froxels no longer reproject the old view's history into the first frame after a cut.
   - The reset is deliberately not in `cut_reconstruction()`. `transparency_shadow.rs` calls that
     on every transmission-hash change, so moving glass would drop fog history every frame.
   - This is a quality fix. Frames without a cut are unchanged by construction.
4. **Tools.**
   - `tools/p-nrt/{lease,abplan,build}` take the unit's owner, root and binary directory from the
     environment.
   - `build.sh` takes its kill pattern from the environment. With `PNRT_QUIET_LEASES` set, it
     also refuses to start while a named quiet lease is admitted (see Housekeeping).

## Quality proof (M)

Binaries:
- `base`: main `a5ae72a`, sha256 `6c756295...`;
- `cand2`: changes 1 and 2, `d5103522...`;
- `cand3`: all three changes, `405e9ece...`.

All are release `ara1_capture` builds in their own processes. Lib-test binaries are `base-lib`
(separate base worktree and target) and `cand3-lib`. Every row ran in lease p1 except where
marked p2.

| Check | Result |
|---|---|
| ARA1 quality witnesses, non-RT 960x540 (game-base, ara-pbr, quarry, game-atmosphere: still, moving, the cut and its recovery), cand2 vs base | byte-identical |
| ARA1 1080p timing captures (frames 39, 71 and 87; every scene and repetition), cand2 vs base | byte-identical (100/100 frames, both rows) |
| cand3 vs base: game-base, ara-pbr, game-atmosphere and game-fog quality witnesses | 69/76 identical; the 7 that differ are exactly game-fog frames 72-87, the cut and its recovery (the fog fix) |
| Q-walk (quarry walk, 93 frames, TR1 on), non-RT, cand3 vs base | 93/93 frame hashes identical |
| RF1-RF10 non-RT after 32 TR1 frames, RT witnesses, lib suite | see p2 below |

**Fog-cut fix** (fog on, GI off so the cache's fill does not mask it):
- `fogcut`: 40 frames at ARA1's still pose, then `Engine::cut()` to ARA1's cut pose.
- `fogfresh`: the cut pose rendered from a fresh start.
- The fresh renders are byte-identical between base and cand3, and so is frame 39, before the
  cut.
- A cut frame cannot equal frame 0 of a fresh start exactly: the TR1 jitter phase and the shadow
  caches differ. The comparison below is therefore against the fresh render, on the pixels the
  fix changes.

| Path | Frame after cut | Pixels changed | Closer to fresh | Farther | Summed abs error on them, base → cand3 | Whole-image mean abs error vs fresh, base → cand3 |
|---|---|---|---|---|---|---|
| non-RT | +0 | 3.4% | 91.6% | 8.3% | 35,844 → 20,633 | 0.175 → 0.165 |
| non-RT | +2 | 3.1% | 88.8% | 11.0% | 31,434 → 18,859 | 0.154 → 0.146 |
| RT | +0 | 3.4% | 91.7% | 8.1% | 36,096 → 20,963 | 0.173 → 0.164 |
| RT | +2 | 3.0% | 89.6% | 10.2% | 31,197 → 18,421 | 0.153 → 0.145 |

The cut frame is closer to the fresh view on both paths: error on the affected pixels falls by
about 42%. The images are in `runs/p1/fog-*`.

P2_PLACEHOLDER

## Timings (M)

Setup:
- ARA1 timing scenes at 1920x1080, synchronous service (`render()` plus GPU drain); still is
  frames 1-39, moving 40-71, cut 72 and recovered 73-87;
- medians pooled over two repetitions per binary, interleaved A,B / B,A in lease p1;
- `tools/p-nrt/summarize.py runs/p1`.

These are functional leases on the shared machine, not quiet-certified.

| Scene | Phase | Base svc ms | cand2 svc ms | Base span | cand2 span | Base shading | cand2 shading |
|---|---|---|---|---|---|---|---|
| game-base | still | 8.83 | 9.26 | 7.77 | 7.96 | 2.28 | 2.25 |
| game-base | moving | 9.47 | 9.91 | 8.33 | 8.63 | 2.00 | 2.03 |
| game-base | cut | 10.69 | 11.07 | 9.31 | 9.29 | 2.27 | 2.14 |
| ara-pbr-oriented | still | 7.95 | 7.86 | 6.91 | 6.85 | 2.60 | 2.55 |
| ara-pbr-oriented | moving | 6.50 | 6.37 | 5.43 | 5.40 | 2.49 | 2.40 |
| ara-pbr-oriented | cut | 8.18 | 7.12 | 6.62 | 5.66 | 3.50 | 2.72 |
| quarry-daylight | still | 11.94 | 11.91 | 10.61 | 10.70 | 0.59 | 0.59 |
| quarry-daylight | moving | 11.50 | 11.83 | 10.26 | 10.54 | 0.56 | 0.57 |
| quarry-daylight | cut | 17.55 | 18.46 | 15.75 | 16.34 | 0.64 | 0.66 |
| game-atmosphere | still | 11.02 | 11.09 | 9.88 | 9.99 | 4.23 | 4.30 |
| game-atmosphere | moving | 13.21 | 12.21 | 11.93 | 10.73 | 4.45 | 3.84 |
| game-atmosphere | cut | 13.80 | 13.08 | 12.23 | 11.48 | 4.82 | 4.51 |

- **Reading.** Images are identical, so every difference is timing spread.
  - The cut is one sample per run.
  - The game-base still/moving rises come from the frame span outside the shading pass, which
    neither change touches.
- **Shading pass, frame by frame** (min / first quartile / median, ms, base → cand2):

  | Scene | Phase | Min | First quartile | Median |
  |---|---|---|---|---|
  | game-atmosphere | still | 3.83 → 3.77 | 4.09 → 4.08 | 4.23 → 4.30 |
  | game-atmosphere | moving | 3.69 → 3.64 | 3.86 → 3.77 | 4.45 → 3.84 |
  | ara-pbr | still | 2.32 → 2.26 | 2.52 → 2.46 | 2.60 → 2.55 |
  | game-base | still | 1.99 → 1.96 | 2.22 → 2.21 | 2.28 → 2.25 |

  The environment change is worth about 1-2% of shading under a physical sky (about
  0.05-0.1 ms). It is not the 0.6 ms the moving median suggests. The lookup lines' cost is the
  texel loads themselves.
- **GI software rays (vec4):** no change in ARA1's quarry span beyond spread. The gather drain is
  bound by something other than the record loads (I; the profile of a drain frame is the next
  step).

## Gaps

Recorded in [m7-gaps.md](m7-gaps.md) under "Non-RT frame cost" (P-NRT2 update) and
"Atmosphere". Ranked by size:
1. **GI fill in ARA1's frames 1-87:** 1.7-7 ms of span. Profile a drain frame next.
2. **The synchronous CPU gap:** 1.8-2.1 ms, of which `render()` is about 0.85 ms. Try a
   two-submit split, after an audit of `write_buffer` calls.
3. **R1n traversal:** about 31% of game-base shading. An exact BVH change first needs a
   traversal-independent tie rule.
4. **Environment texel loads under a physical sky:** about 26% of shading. An exact further cut
   needs fewer lookups.

## How Ara raster gets about 1.8 ms (read-only study of Ara main 1d2d647)

From the source at `D:/Coding/renderlab-work/ara1/ara-main` (code reading, **I**, no profile):

- **No rays at all in native raster.**
  - Indirect diffuse is a probe volume. Its surfaces come from a one-time CPU (or second-device)
    "acquisition" bake, cached on disk; ARA1 charges it separately from frame time.
  - Each frame `GI.Relight` relights a fixed budget of probes (1024 Medium, 2048 High). It uses
    no rays, and `GI.Gather` reads them at half resolution (`ara-render/src/look/gi.rs`).
- **Specular** is an analytic sky prefilter plus probe lobes, with half-resolution SSR blended on
  top from Medium up. There are no traced reflections. A glossy sphere therefore reflects only
  what is on screen (SSR) or the probes and sky.
- **Sun shadows are cached cascades.** Four 2048 cascades plus a far map on High; each redraws
  only when its matrix or the casters in its volume change, so a still static scene draws none.
  Filtering is gather PCF.
- **Stock presets render at 0.45-0.7 scale** with TAAU or TSR. ARA1 forced native resolution for
  both engines, so its 1.8 ms is native, but every other choice above holds.
- **What this means here.**
  - Ara's frame is cheap because its non-RT image omits what costs most here: per-pixel traced
    glossy reflections (16 rays), exact sun penumbrae and an online world-space GI cache.
  - Matching its speed by copying those choices would lower quality, so it is ruled out (M8).
  - The exact wins left are cost per ray and per lookup, and frame scheduling.

## Housekeeping

- Bulk output: `D:/Coding/renderlab-work/p-nrt2/` (`bin/`, `plans/`, `runs/`, `traces/`, `scenes/`).
- Cargo targets: `D:/Coding/pnrt2-target` and `D:/Coding/pnrt2-target-base` (base worktree
  `D:/Coding/pnrt2-base-tree` at `a5ae72a`), deleted at the end.
- **Quiet-window incident (13:00 UTC).**
  - A root order had queued p-lts2's D screen with a compiler-quiet prefix. That window opens at
    the owner's `admitted` line, but the build watcher only reacted to schedule lines written
    after it started.
  - Build b3 (incremental, `-j 2`, 13:01:11-13:01:45) therefore ran inside p-lts2
    d-screen-1's prefix. It was reported on the schedule and root stopped further builds.
  - Later builds go through a gate (`p-nrt2/quietgate.py`) that treats an admitted quiet lease
    without its release line as an open window.
