# M7-TR1: identity-validated temporal reconstruction (T1b revived)

2026-11-08, branch `t3/m7-tr1` from main `ca80f85`, main `7971991` (M7-U0b) merged. Contract: [M7](milestone-7.md), [design §6–9](m7-design.md), [P0](m7-p0-claims.md), the bar in [TR0](m7-tr0.md) (`E:/renderlab-work/m7-tr0/bar/tr1-bar.json`).

<!-- STATUS -->

## 1. What TR1 is

`Renderer::set_reconstruction(Reconstruction)` (default off). On traced and visibility-buffer lit frames at 1x, on RT and non-RT devices:

1. **Jitter through the projection.** Frame *k* samples every pixel at one offset of T1's corner-free 256-point rank-1 lattice (one symmetry for the whole image), carried by `Camera::jittered` in the view-projection. Traced rays, the N1c visibility raster and resolve, the receivers and every program's shading take it from the same matrix, so no primary, shading or shadow shader is edited. Frame 0 of a sequence is the pixel centre. A jitter of zero keeps every existing matrix bit-identical.
2. **Interactive's eight disk directions rotate** by 8·(*k* mod 32) through each pixel's own 256-direction sequence (RT), through the unused `PrimaryFrame.sided.y` word. Only TR1's own compiled copy of the Interactive kernels reads it (`ROTATION_SEAM`); the near-edge check, which traces no direction, stays production's.
3. **The program shades normally.** Its output is copied, and one compute pass (`shaders/temporal_resolve.wgsl`, compiled from the no-ray-query head, so it runs on no-RT devices) writes the HDR target and the next history.
4. **Reprojection by surface identity.** Each pixel's current sample point comes from its hit record (placement, primitive, barycentrics, the same receiver point R1 uses); its history is fetched where that point was in the previous frame. A history tap is continued only when its anchor (the latest sample's placement, viewed side and view distance) shows the same surface: the tap's own sample point, rebuilt from its previous-frame ray, must lie within four pixel footprints of the current point's tangent plane. No colour, luminance or neighbourhood statistic decides acceptance or weight.
5. **Silhouette bit.** A pixel whose 3×3 samples show more than one surface writes a "mixed" anchor. Interior and open-background pixels never continue mixed history, so nothing trails into interiors or sky. Open background (every 3×3 sample a miss) is the background exactly.
6. **Still frames** (bit-equal unjittered view-projection, decided on the CPU): each pixel continues its own history while the tap's surface is still among its 3×3 samples, so edges converge to the footprint integral as T1a does.
7. **Windows.** Static 256; under bilinear motion T1b's blur budget W = 1 + v_max / Σf(1−f); Catmull-Rom over 4×4 when all 16 taps are accepted (window 16), limited to its own inner taps' range (the undershoot fix: history only, not a current-frame box); moving silhouettes cap the window.
8. **Revealed and restarted pixels start from this frame's samples only:** a tent over the 3×3 samples of the same surface (radius 2 inside a surface, effective count ≤ 4).
9. **Engine events.** A key over programs, prelude, geometry, bindings, materials, textures, transforms, light colours and kinds, environment and every shadow/visibility setting restarts every pixel. Exposure changes rescale the exposed history exactly. A sun turn caps the window at `turn_angle / turn` (0.004 rad; larger turns restart). `cut_reconstruction()` restarts on a camera cut the engine cannot infer.

<!-- SETTINGS -->

### 1.1 Files

| File | Content |
|---|---|
| `src/temporal_reconstruction.rs` | `Reconstruction`, history state, frame words, pipelines, prepare/encode, CPU tests |
| `shaders/temporal_resolve.wgsl` | the resolve pass |
| `src/temporal_reconstruction_tests.rs` | `gpu_tr1_sequence` (both paths, variants, frames, visits, replay) |
| `src/mesh.rs` | `Camera::jittered` (projection plumbing) |
| `src/renderer.rs`, `src/primary_ray_gpu.rs`, `src/observe.rs`, `src/lib.rs` | settings, jittered camera of reconstructed frames, rotation word, rotated Interactive set on production passes, timestamps 4/5 (`GpuSample::reconstruction_pass`), Virtual still-key fix (§7.2) |
| `scripts/m7-tr1-*.py` | runner/waiter/quiet poller, plan, scorer, compare, table, settle, cost, inspect |

No shadow, GI/cache, reference, shader-reload or pipeline file and no shader of primary rays, shadows or programs is edited. `film_accumulation.rs` (T1a) is unchanged and stays a separate opt-in mode.

## 2. Claim-avoidance notes (engineering reading, not clearance)

Against P0's element reading ([m7-p0-claims.md](m7-p0-claims.md) §3.C–D, §4.3–4.6):

| Claim | TR1 has | TR1 lacks, and why |
|---|---|---|
| US 10,116,916 cl. 1, 11, 18 (variance clip) | (a) current frame, (b) previous colour, (d) blend | (c) no neighbourhood mean, variance or σ box: acceptance and weights come from surface identity, geometry and engine state. The optional min/max box (`box_clamp`, off) has no statistic |
| US 10,362,289 (box excluding a sample) | (a), (b), (e) | (c)–(d): no box by default; the optional box spans **all** 3×3 samples, so no neighbourhood sample lies outside it |
| History-clamp family (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) | a full history | no fast or short history and no clamp of one history by another: one value per pixel |
| US 11,847,737 cl. 1 | (a), (b) | (c) windows come from resampling geometry and engine events (sun turn, exposure, key), never from comparing pixel values; (d) one blended value per pixel, no retained list. P0's ambiguity on W-capped blends stands |
| US 12,536,735 | one list-like value | a second list and second algorithm |
| US 12,437,371 / pending US 2026/0162231 | (a) lighting data of two frames exist | (c)–(e): no lighting difference, gradient or confidence is computed; own-pixel moments (`Reconstruction::OwnMoments`) are not built |
| US 10,600,167 cl. 1 | (a1) back-projection through the hit record, (a2) the history sample, (a3) placement/side/plane check, (a4) colour plus the anchor and silhouette bit | (b)–(c): **W5** — the weights, anchors, silhouette bits and history are read only by `tr1_resolve` itself, next frame. No later stage (present, GI, reflections, any filter) binds them; the bind group exists only in `reconstruction_groups` |
| US 11,113,792 cl. 1, 11, 15 | the revealed-pixel and silhouette spatial estimates are spatial filters | cl. 1/11 (a)–(c): fixed 3×3 taps and fixed tent radii, not derived from render data or jittered; cl. 15 (c): the spatial estimate filters **this frame's samples**, never the temporally filtered values |
| US 12,423,782 | the camera sample jitters on a lattice | (a) filter taps are fixed: the 3×3 tent, bilinear and Catmull-Rom taps never move with the jitter; P0 already reads camera jitter as outside (a) |
| US 11,688,042 | — | no separable multi-pass kernel |

Not used: variance clipping, SVGF, temporal gradients, own-pixel moments, fast/slow histories, colour or luminance rejection. The plain min/max box was built as the design's last option and measured (§5); it changed nothing measurable and stays off.

## 3. Programmability

- Shading is untouched: programs shade the jittered hit as any traced frame; TR1 reads only their output colour, through an engine copy.
- Acceptance and weights never read colour, so toon, flat and custom programs converge to the footprint integral of their own output. Two places read output colour: the spatial estimates (a weighted mean of the program's own samples, not a decision), and the optional `box_clamp` (off).
- Constraint recorded: a program whose output is not a function of the surface point (screen-space patterns, dithering keyed to the pixel, output that depends on the frame) is integrated over the jitter like any other signal; screen-locked patterns blur by the jitter footprint. A program that wants per-frame stochastic effects should key them on the surface, not the screen.
- Constraint recorded: TR1 needs each pixel's hit record and placement, so it applies to traced and visibility-buffer frames only. Raster frames render unjittered without history.
