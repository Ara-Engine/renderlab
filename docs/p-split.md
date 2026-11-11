# P-SPLIT: program kernel compile cost, trust policy, crack flashes

Unit P-SPLIT, milestone 8 ([milestone-8.md](milestone-8.md)), 2026-11-11, branch `t3/p-split` from main `c8c7443`. Labels: **M** measured here, **C** read from code, **A** another unit's measurement, **I** inference.

Scope after root's steers: (A) first, the quarry crack flashes P-FLICKER left; (B) the split was time-boxed by P-SKY. P-SKY (#16/#17) cut the game scene's cold first image from about 183 s to about 11 s on both paths (**A**, root), so this unit keeps to the trust-policy fan-out, the two unchecked user-text paths and the census tool. The full kernel split goes ahead only if Showcase through Ara, cold, after P-SKY, still takes over a minute.

## Result

(filled in below as steps land)

## A. Crack flashes

### Measurement switch

`RENDERLAB_P_STILL=tr1-mask|tr1-raw|tr1-window` (src/temporal_reconstruction.rs `mask_resolve`) composes a measurement copy of TR1's resolve. Production compiles `RESOLVE` unchanged; the switch only edits the copy's text. The output's alpha carries how TR1 continued each pixel:

| alpha | meaning |
|---:|---|
| 1 | restart (cut, first frame) |
| 2 | still pixel, own history |
| 3 | moving, Catmull-Rom over 16 accepted taps |
| 4 | moving, bilinear over 4 accepted taps (an outer Catmull-Rom tap failed) |
| 5 | moving, bilinear over 1-3 accepted taps |
| 6 | no accepted tap: spatial start from this frame's 3x3 |
| 7 | rejected by an engine event (moving caster, relit light) |
| +16 | silhouette pixel (two-slot path) |
| 32 | third-surface pixel (state carried) |

`tr1-raw` writes the frame's fresh sample as the colour, `tr1-window` the history weight and window before the sample. `tools/p-flicker/mask.py` finds one-frame spikes (frame t departs from the mean of t-1 and t+1 by more than 20 codes while those two agree within 10) and tabulates the class, history and fresh sample at them.

### Capture (M)

Lease `crack-l1` (65 s), RT, P-FLICKER's clip manifest (walk frames 160-223, every frame captured) with each switch, and the standard yardstick manifest for the score. Binary: release `p_yard_capture` at `1375b18` (c8c7443 plus the switches), SHA-256 `1467b02b…`. The mask switch changes only the output's alpha and adds private writes (C); history targets are production's. The base run scores exactly main's 4.323.

**Where the >20-code steps fall** (P-FLICKER's own residual: frame step minus the reference's step, on the 15 clip pairs that have references; 1,840 such pixels a pair, 25% of the flicker energy, as P-FLICKER measured):

| TR1 class at frame t | share of the quarry's pixels | share of >20-code pixels | share of their energy |
|---|---:|---:|---:|
| Catmull-Rom, all 16 taps accepted | 88% | 45% | 35% |
| bilinear, 4 taps (an outer tap rejected) | 9.5% | 16% | 13% |
| bilinear, 1-3 taps | 3.3% | 34% | 44% |
| spatial start (no tap accepted) | 0.2% | 5% | 7% |

The strict one-frame spikes (86 a frame on the clip) are 75% bilinear 1-3, 20% spatial start, 2% Catmull-Rom. The frame before a spike is also mostly bilinear 1-3 (68%): these pixels sit in partial rejection frame after frame, not once.

**Rejection, not bright samples.**
- The class map (below) lines every crack, ledge and fold of the scan with rejected taps (blue: 4 taps, orange: 1-3, red: none), and the white spike pixels sit on those lines.
- Only 15% of the spikes' fresh samples lie more than 10 codes outside their 8 neighbours' range (0.13% of all pixels do), and the spike's sign follows the sample in 99% of cases. The sample is ordinary texture; what makes it jump is its weight.
- A rejected tap shortens the window: bilinear over the remaining taps keeps T1b's `1 + blur_budget / sum f(1-f)` (about 3-5 frames) and a spatial start at most 4, so each frame's jittered sample, landing on the crack's dark floor or its lit rim, moves the pixel by a third to a quarter of the difference.
- The fresh-sample clamp screen confirms it: clamping the sample to its 8 same-surface neighbours' range scores 4.325 (hard clamp) and 4.319 (range widened by half), against 4.323 unclamped. Neither moves the flicker, and the hard clamp raises still error (6.630 → 6.703) by flattening fine texture.

![Rock crop, frame 206: image, step x4, TR1 class with spike pixels white](p-split/crack-mask-206.jpg)

**Verdict: history rejection, a reconstruction change; not fixed here (root's steer).** The geometric tap test (`tr1_tap`: the previous sample point within four pixel footprints of the current tangent plane) rejects taps across the scan's crack folds, which belong to the same placement and side. That is the next flicker unit. Directions, unmeasured: a tap test that tolerates near-coplanar folds of one mesh (distance along the view ray, or the tap's own plane); or keeping the window when only outer taps fail, renormalising over the accepted ones. Either must keep real disocclusions rejecting, so the S4 door and the silhouette tests gate it.

## B. Trust policy and user-text paths

**C:** before, `program_gpu::module` compiled the exact bundled standard program with the engine API (unchecked) and any other text checked, so the first edit of the standard program changed the check mode of all 24 of its kernels (P-PIPE, census PR #11). GI cell shading (`gi_cache.rs`) and the glass reflected-hit kernel (`transparency_reflection.rs`) compose program text but used the engine API.

Change: every module composed from program text compiles checked: the standard and toon programs, user programs, TP1 glass, GI cell shading and glass reflected hits. An edit now recompiles only the kernels whose text it changes.

## Census

## Cold start

## Hardware, build, process

- RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, 32 GB, Windows 11 Pro 26300.
