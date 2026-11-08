# M7-MQ1: TR1 in an animated scene (reflection grain and moving edges)

2026-11-08, branch `t3/m7-mq1` from main `f870cb4`. Islam, looking at the example scene (`examples/game_scene.rs`, frames `D:/Coding/renderlab-work/api1/phase2c/{rt,nonrt}/frame-047.png`): "the reflections in the rt one seems good but noisy as fuck.. also no good anti aliasing yet it seems". Rules: AGENTS.md's overnight push (ship when clearly better and not visibly worse in inspected images; gaps go to [m7-gaps.md](m7-gaps.md)), [P0](m7-p0-claims.md). Labels: **M** measured here, **I** inference.

RESULTS_STATUS

## 1. Diagnosis: TR1 restarted every pixel on every frame of the example (M)

The example animates three placements on every frame (the hero's body and head walk and bob; the blue crate spins). `Engine::set_transform` republishes the primary tables' transforms, which bumps `UploadBytes::models` and `normals`. TR1's restart key (`reconstruction_key`) hashed both counters, so **every reconstructed frame of the flight was a restart**: each pixel's output was its radius-1 spatial start, the current frame's jittered samples interpolated to the pixel centre. That has two visible effects:

- **No anti-aliasing.** Edges show one jittered sample per frame, which crawls as the jitter changes. Box and arch edges stair-step through the whole flight.
- **No integration of any stochastic signal.** On RT, the four glossy reflection strata (R1) and Interactive's eight shadow directions get no history, so the gold sphere shows raw four-ray grain every frame. TR1's direction rotation only changes which grain appears.

The same restart applies to **any scene with any moving object**, on both paths. TR1 [§10](m7-tr1.md) recorded it ("moving instances restart every pixel"); the RT-shadow gap list records its consequence for moving casters. Every TR1 measurement so far (Q-walk, Q-disocclusion, Q-sun, S2G, the visible-sky walk, RF fixtures) used static placements, so none of them saw it.

## 2. What changed

### 2.1 Rigid reprojection of moved placements (`src/temporal_reconstruction.rs`, `shaders/temporal_resolve.wgsl`)

- **The key no longer hashes transform edits** (`models`, `normals`). New placements still restart through `instances` and the primary tables' `geometry_generation`, which also keep placement slots stable while only transforms change.
- **RT: the TLAS rebuild no longer restarts.** On RT renderers a transform edit rebuilds the TLAS (`scene_generation`). That recreates the primary frame bindings and bumped `bindings_generation`, which the key also hashed, so RT restarted even with the transform counters gone. MQ1's own GPU test found this (RT frame 1 restarted). The key now compares the bound targets themselves (hit record, HDR and fresh-world textures, `Previous::bound`) instead of the rebinding counter. Target reallocations still restart; TLAS rebuilds do not.
- **Previous transforms.** On each reconstructed frame where any placement's transform bytes changed, the CPU writes the previous frame's placement transforms after the 304 fixed frame words of TR1's frame buffer (binding 40 gains a runtime array `previous_models`, so there is no new binding or storage-buffer slot). The flag `TR1_MOVED` tells the resolve to read them.
- **Per-surface motion.** `tr1_surface` compares a placement's current and previous transforms bit for bit. A moved surface's sample point and plane are carried to the previous frame through its previous transform (`previous`, `previous_normal`), so:
  - the history fetch position follows the object, not only the camera;
  - the tangent-plane acceptance test compares the tap's rebuilt previous-frame point with the surface's previous-frame plane;
  - a moved surface is never "still": it skips the still-camera shortcuts (own-pixel fetch, kept silhouette pairs) and gets the hidden-surface cap, as camera motion does.
- **Static scenes are unchanged.** With no transform edit, `TR1_MOVED` is off and every expression reduces to main's (byte-identity checked in §4).

### 2.2 The motion box (`Reconstruction::motion_box`, default on)

Continuing history across object motion exposes the case the restart had hidden: **light that a moving object changes on other, static surfaces** (its shadow, its reflection, its bounce) would be carried by surface identity as a trail. On frames where `TR1_MOVED` is set (`TR1_BOX`), every continued history is clamped to the **plain min/max box of this frame's 3x3 samples, all nine**, before blending. Frames where nothing moved never clamp, so static scenes keep TR1b's behaviour and evidence.

### 2.3 Test and tooling

- `gpu_mq1_moving_placement` (ignored, GPU, both paths): a card slides 1 cm per frame over a static ground under a still camera for 24 frames. No frame after the first may restart; the card's interior history weight must exceed the start window, and the ground's must keep accumulating.
- `scripts/m7-mq1-run.py`: lease poller (shared lock, QUIET windows, game holds) around `scripts/n8-run.py`.
- `scripts/m7-mq1-score.py`: the flight's lighting-matched supersampled control, per-frame object regions from the example's camera path, display RMS, edge RMS, motion-compensated flicker, contact sheets, strips and GIFs.

## 3. Claim-avoidance notes (engineering reading, not clearance)

TR1/TR1b's readings ([m7-tr1.md](m7-tr1.md) §2, [m7-tr1b.md](m7-tr1b.md) §5) hold. What MQ1 adds:

| Claim | MQ1 has | MQ1 lacks, and why |
|---|---|---|
| US 10,116,916 cl. 1, 11, 18 | (a) current frame, (b) previous colour, (d) blend, and a box | (c) the box's planes are the **min and max** of the nine samples, with no mean, variance or standard deviation. Claims 8/15 need layered transmittance composites; 10/16 need stereo |
| US 10,362,289 cl. 1, 12, 20 | a box from neighbourhood samples | (c) "at least one color value … outside of the bounding box": the box spans **all nine** 3x3 samples, so none lies outside it (P0 §B: "a plain min/max box avoids it") |
| History-clamp family (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) | a clamp of the history | no fast or short history: the range is this frame's samples only, and there is one history value per surface per pixel |
| US 11,847,737 / 12,536,735 | — | windows still come from resampling geometry and engine events; the box changes the history value, not the window, and there is no retained list |
| US 10,600,167 (W5) | (a1) back-projection now also through previous placement transforms | (b)-(c): previous transforms, slots and weights are still read only by `tr1_resolve`, next frame |
| US 12,437,371 / 2026/0162231 | — | no lighting difference, gradient or confidence: the box is a clamp, not a change detector; `TR1_MOVED` comes from transform bytes on the CPU and GPU, never from colour |

Not used: variance clipping, SVGF, temporal gradients, own-pixel moments, fast histories, colour-based rejection or weights.

## 4. Results

RESULTS

## 5. Programmability

- Acceptance and windows still never read colour. Moved surfaces are matched by placement, side and plane through their own transforms, whatever program shades them.
- **Constraint added:** on frames where a placement moved, the box reads the program's output colour, a plain range of the program's own nine samples. A program whose output varies sharply within a pixel's 3x3 for a reason that does not move with the surface (a screen-space pattern, pixel-keyed dithering) is clamped to that range on such frames. Its history then carries less than on static frames. Surface-keyed patterns are unaffected except where they alias at the pixel scale.
- **Constraint kept:** the reprojection is rigid (affine per placement). Rewritten meshes change the geometry uploads and still restart every pixel. Per-vertex motion for deforming geometry (skinning, vertex animation) is not built.

## 6. Gaps

GAPS

## 7. Evidence, reproduction and time

EVIDENCE
