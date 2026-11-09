# P-NSQ: non-RT sun penumbrae at RT's quality

Unit P-NSQ, 2026-11-09, branch `t3/p-nsq` on main `bdec7b9`. Milestone 8 ([milestone-8.md](milestone-8.md)):
beat Ara on performance with zero quality loss. This unit fixes a quality shortfall recorded in
[m7-gaps.md](m7-gaps.md) ("Non-RT sun shadows"): on non-RT, sun penumbrae were grainy and harder
than RT's. Labels: **M** measured here, **I** inference.

## Result

**Fixed on the courtyard, the wide-sun game scene and the S2 disk fixture; one open item on
the quarry; a large cost on moving and cut frames.**

- **Quality.** Non-RT penumbrae are now soft and clean, as wide as RT's, still and in
  motion. Against the path-traced reference the wide-sun game crop goes from .02669 to
  .02398 (RT .02374), and the original-sun crop from .02463 to .02394 (RT .02358).
  Against RT, courtyard flight error falls from .01744 to .01143 and still error from .02062
  to .01079. S2's penumbra grain at its held frames falls from 13.4-14.5 to 1.4-2.3 (RT 1.3-1.7).
- **Open item.** The quarry contact crop is 0.6% worse than main (.01505 vs .01496; RT
  .01485): about 100 single-pixel speckles a frame in gravel and along rock contacts, which
  main's walk cap hid. No variant fixed them without costing wide penumbrae (see "What decides
  an undecided direction"), so they are recorded as a gap, not traded.
- **RT is byte-identical** to main: 25/25 courtyard frames and 12/12 wide-sun frames.
- **Cost (1080p, sun-visibility pass).** Held frames are unchanged. Moving frames rise from
  2.9 to 5.5-6.2 ms (game) and from 0.76 to 3.7 ms (quarry), from 32 directions a frame. Cut
  frames rise from 3.4 to 31-33 ms (game) and from 1.1 to 31 ms (quarry), from 256 directions,
  RT's cold budget. P-NRT and P-RTS (the cold-ray spike) are where this cost gets paid back.

## Cause (M)

The v0.1.0 courtyard (`game_scene`, 0.04 rad sun at dusk) shows it plainly. RT's penumbrae from
the gate and boxes are about as wide as the shadows. Non-RT's are a few centimetres wide,
grainy, and hard (crops `crops/v010-f16-{nonrt,rt}.png` in the evidence). Two causes, bisected
on the ARA1 game scene with the courtyard's 0.04 rad sun against a 16,384-sample path-traced
reference ([Evidence](#evidence)):

1. **The walk cap collapsed penumbrae (the larger cause).** N8e's cap (`walk_cap: 16`):
   once a receiver's walks had visited 16 triangles and one direction was proven blocked,
   every direction still open was taken as blocked. A penumbra receiver of a distant caster
   always passes 16 visits: its open rays pass near the caster's silhouette in the map, and
   each candidate there costs a walk. So most of the penumbra became umbra, and only receivers
   whose every direction stayed clear kept light. The result was a hard edge where the first
   direction was blocked.
2. **Fixed directions, so TR1 could not integrate them.** On RT, TR1 rotates each pixel's
   sun-disk directions every frame (`temporal_reconstruction::rotation`) and samples
   `ReconstructionShadowBudget`'s 256/64/32/8 directions. The non-RT lookup seeded its 8
   directions from the pixel alone, the same every frame. TR1's history then averaged one
   8-sample estimate per pixel forever: grain that never converged and crawled in motion.

| Wide-sun game, `sun_shadow` crop, 11 frames | RMS vs reference | Grain | RMS vs RT |
|---|---:|---:|---:|
| main (8 fixed directions, cap 16) | .02669 | .01375 | .01298 |
| cap 0 only | .02492 | .01224 | .00930 |
| rotation only (8 a frame) | .02601 | .01275 | .00984 |
| rotation + RT's budget, cap 16 | .02585 | .01245 | .00943 |
| rotation + budget + cap 0 | .02393 | .01059 | .00235 |
| RT (traced Interactive, TR1) | .02374 | .01058 | |

RMS is linear display RGB (ARA1's metric). Grain is the RMS of the residual minus its 5x5 box
mean, which isolates pixel-scale noise. Neither fix alone reaches RT; together they do.

The 16-triangle cap had one job: on the quarry's scans, receivers in umbra whose walks ran out
of steps (8 triangles, doubled per coarser level) before reaching the triangle under the ray
were left open. Those leaks are what the cap hid. Removing the cap alone on the quarry
(narrow 0.00465 rad sun, `shadow_contact` crop) raises error from .01496 to .01549, with a
brighter mean: umbra leaks.

### What decides an undecided direction (M)

The cap hid two kinds of miss on the quarry. Each was measured with RT's sampling and no
cap, on the quarry's `shadow_contact` crop against its path-traced reference (main .01496,
RT .01485):

| Variant (rotation + budget, cap 0) | Walk steps | Quarry RMS | Mean | Wide-sun RMS | Original-sun RMS |
|---|---:|---:|---:|---:|---:|
| truncated walks stay open (N8b) | 8 | .01549 | +.00006 | .02393 | |
| truncated walks stay open | 128 | .01508 | -.00004 | .02393 | .02387 |
| truncated walks block (`truncated_walks_block`) | 8 | .01512 | -.00039 | .02399 | |
| truncated walks block | 32 / 64 / 128 | .01505 / .01506 / .01506 | -.00011 | .02398 | .02394 |
| cap 16 kept, 128-step walks | 128 | .01486 | -.00018 | .02591 | .02395 |
| main | 8 | .01496 | -.00013 | .02669 | .02463 |
| RT | | .01485 | +.00001 | .02374 | .02358 |

- **Truncated walks.** A walk that runs out of steps before reaching the triangle under its
  aim left its direction open. That is the leak the cap was masking. Blocking exactly
  those directions (the map texel's verdict: its surface is above the ray) removes it,
  and 32 steps make truncation rare. Beyond 32 nothing changes.
- **Near-field contacts.** The rest of the quarry gap, about 70-120 pixels a frame against
  main, is single-pixel speckle in the gravel and along rock-ground contact lines, slightly
  brighter. The cap blocked those receivers' remaining directions after their first
  proof. The exact march does not present those occluders as candidates. Neither testing
  the first step's 2x2 block for every surface (`first_block: 1`: .01523, darker) nor more
  march samples (16: .01504) fixes them. A cap applied only within the first texels of the
  march does (.01491 at 1, 2, 4, 16 and 64 texels). But it also costs the wide-sun crop
  .02407 / .02408 / .02414 against .02398 uncapped, and the courtyard .01146 against .01143,
  so it caps some real penumbra. Per the equal-or-better rule it was rejected and removed.

Keeping the cap with long walks is the quarry's best (.01486, RT's level). It still
collapses the wide-sun penumbrae (.02591), so it is not a fix: a receiver's visit count
cannot tell a crevice from a penumbra.

## Fix

Changes to the Exact and Surface lookups (shaders/virtual_shadow.wgsl,
shaders/virtual_shadow_cache.wgsl, src/virtual_shadow_gpu.rs, one renderer hook):

1. **RT's sampling on TR1 frames.** Each pixel's directions continue through its 256-direction
   R2 sequence by RT Interactive's rotation (`temporal_reconstruction::rotation(k)`). The count
   follows the engine's `ReconstructionShadowBudget`, as on RT: 256 on a new target or cut, 64 on
   a scene-history restart, 32 while the camera, sun or casters move or history is young, and 8
   after 16 held frames. The lookup evaluates them in batches of eight (`vsm_walk_batches`); each
   batch is N8e's lookup of eight directions. The renderer writes the batch count after TR1 picks
   the budget (`prepare_virtual_sampling`); `VsmFrame` gains one uvec4 (1,680 -> 1,696 bytes).
   The surface cache still shows only consensus-certified solid cells, now solid over every
   direction of the frame. Frames without TR1 keep one unrotated batch; Map and Hybrid are
   unchanged.
2. **No full-march walk cap.** A direction a candidate walk could not decide (it ran out of
   steps before locating the triangle under its aim) is blocked alone at the march's end
   (`truncated_walks_block`, default on), and walks get 32 steps (`walk_steps`, was 8).
   `walk_cap` defaults to 0; N8e's 16 remains available.
3. P-NRT's CPU find: `prepare_virtual` collects the skin flags once per frame instead of
   rescanning them per placement (O(placements^2) -> O(placements)), identical values.

Shading stays programmable: programs read the same 0-256 visibility count, and nothing was added
to hit records, material inputs or the program ABI.

## How others get soft, stable penumbrae without RT

Unreal's virtual shadow maps use shadow-map ray tracing (SMRT). Each pixel shoots a few rays
toward the light's disk and tests samples along each against the virtual shadow map. The rays
are random per pixel and per frame, and TSR/TAA integrates their noise
([Epic: Virtual Shadow Maps](https://dev.epicgames.com/documentation/unreal-engine/virtual-shadow-maps-in-unreal-engine)).
Its adaptive ray count spends fewer rays where the first ones agree. Epic's forums trace
moving-shadow pixel artifacts to that adaptivity
([forum](https://forums.unrealengine.com/t/virtual-shadow-maps-for-high-quality-rendering-cinematics/1922741)).
More rays is the documented quality lever.

Renderlab's non-RT lookup already decides each direction exactly, with watertight triangle
tests rather than depth samples. It lacked two things: per-frame variation for TR1 to
integrate, and a correct verdict for directions whose walks could not finish. This unit adds
both. Patent policy: no filter, reservoir or history count is added. The rotation and budget
are RTDEF2's existing sampling scheduler, covered by TR1's W5 design-around record
([m7-p0-claims.md](m7-p0-claims.md)). The surface cache still stores consensus-certified
verdicts and never averages walks (constraint C8).

## Quality, before and after (M)

ARA1 game scene and quarry: 960x540, crops from `tools/ara1` regions (`sun_shadow`,
`shadow_contact`), linear display RMS against the 16,384-sample path-traced references (ARA1's
for the 0.00465 rad sun, rendered here with ARA1's frozen tracer for the 0.04 rad wide sun).
Frames cover still (39), moving (40-71), cut (72-74) and recovered (75-87).

| Crop | main | final | RT |
|---|---:|---:|---:|
| Wide-sun game `sun_shadow` (11 frames) | .02669 | .02398 | .02374 |
| - still / moving / cut / recovered | .02825 / .02220 / .02986 / .02900 | .02550 / .01997 / .02698 / .02586 | .02455 / .01962 / .02692 / .02580 |
| Original-sun game `sun_shadow` (10) | .02463 | .02394 | .02358 |
| Quarry `shadow_contact` (10) | .01496 | .01505 (open item) | .01485 |

Courtyard (`game_scene`, 1280x720, 0.04 rad dusk sun, every second frame of the 48-frame flight
and of a 48-frame still hold). No reference exists for its atmosphere, fog and particles, so it
is scored against RT, which the gap entry calls clean: RMS / grain / mean of the penumbra crop
around the boxes and gate shadows (`crops/court-f16-main-cand-rt.png`: main, final, RT).

| Courtyard vs RT | main | final |
|---|---|---|
| flight | .01744 / .00430 / -.00477 | .01143 / .00238 / -.00065 |
| still | .02062 / .00752 / -.00525 | .01079 / .00216 / +.00050 |

S2 disk fixture (0.04 rad, registered films, `scripts/rtdef2-score.py`; Virtual on traced
receivers under TR1; +4-stop codes, penumbra grain / whole image):

| Track, frame | main sampling (fixed 8, cap 16) | final | RT (RTDEF2 budget) |
|---|---|---|---|
| camera 0.25 px, 0 (cut) | 22.98 / 14.25 | 6.48 / 12.60 | 6.46 / 12.60 |
| camera 0.25 px, 63 (moving) | 9.56 / 4.17 | 3.91 / 3.11 | 3.51 / 3.08 |
| camera 0.25 px, 95 (held) | 14.50 / 4.17 | 2.30 / 1.39 | 1.72 / 1.35 |
| camera 1 px, 63 / 95 | 9.16 / 3.82, 13.41 / 4.05 | 3.38 / 2.83, 2.05 / 1.51 | 2.98 / 2.80, 1.50 / 1.44 |
| shadow step, 31 / 63 / 95 | 13.67 / 4.86, 13.61 / 4.95, 13.44 / 4.63 | 2.37 / 1.63, 1.84 / 1.65, 1.38 / 1.21 | 2.35 / 1.61, 1.79 / 1.61, 1.30 / 1.17 |

On the camera tracks the final penumbra grain stays 0.4-0.6 codes above RT's at the held
frames. Whole-image error is within 3% of RT's.

## Cost (M)

1080p ARA1 timing scenes, release build, one frame in flight, synchronous GPU spans, RTX 4070
SUPER (NVIDIA 617.14), shared machine with other units' idle processes. Medians per phase, ms,
two interleaved game visits (main then candidate, twice) and one quarry visit each:

| Scene | Sun-visibility pass: still / moving / cut / recovered | Frame span: still / moving / cut / recovered |
|---|---|---|
| game, main | 2.81-2.98 / 2.90-2.91 / 3.38-3.43 / 2.80-2.88 | 10.1-10.5 / 10.3-10.4 / 11.5-13.2 / 9.0-9.4 |
| game, final | 2.85-3.45 / 5.52-6.17 / 31.3-32.6 / 5.47-5.55 | 9.6-11.7 / 12.6-15.2 / 39.4-43.7 / 11.2-11.6 |
| quarry, main | 0.79 / 0.76 / 1.07 / 0.93 | 9.95 / 10.26 / 17.96 / 13.54 |
| quarry, final | 1.52 / 3.73 / 30.83 / 4.25 | 13.44 / 12.87 / 46.36 / 14.84 |

Held frames (8 directions, as before, now rotated) are about unchanged on the game scene. The
quarry's held frames rise by 0.7 ms (no cap, longer walks). Moving and young-history frames
evaluate 32 directions (4 batches) and pay +2.6-3.3 ms. A cut evaluates 256 (32 batches) and pays
about +28-30 ms on that one frame, the same cold budget RT spends (RT's 21 ms first cut is
P-RTS's target). Memory is unchanged apart from 16 bytes of frame uniform. Ways to pay this back
without changing the image, for P-NRT: share the march's map reads across batches (each batch now
repeats the ring and the map march), keep a pixel's batch loop in one warp-coherent order, and
take whatever P-RTS finds for the cold spike.

## Evidence

`D:/Coding/renderlab-work/p-nsq/` (external): `runs/ref-wide` (wide-sun references),
`runs/s4-*` (ARA1 game/quarry screens, every variant named above), `runs/s*-*.json` (scores),
`runs/court/` (courtyard runs), `runs/S2*` (S2 bench and `scores.json`), `runs/t-*`, `runs/t2-*`
(timing `run.json`), `runs/*.log` (lease logs), `runs/sheets/` and `crops/` (inspected crops),
`plans/` and `tools/` (lease plans, scorer `score.py`, `timing.py`, `identity.py`). The screen
harness is `examples/p_nsq_screen.rs`; S2's `TR1-Virtual-fixed8` and `TR1-Virtual-Auto32` modes
are in `src/motion_bench_tests.rs`. Binaries: candidate = this branch, main = `bdec7b9`, each
built release in its own target outside the repository and deleted at the end.

Leases: l1b (wide-sun references; its candidate steps failed on a WGSL name clash, fixed),
l4b, l6k, l7, l8 and l9 under the shared lock, each at most 12 minutes. The lib suite passed
inside l7 and l9: 197 passed, 0 failed, 163 ignored. The final code is l7's binary
(`ec7db0c`'s source).
