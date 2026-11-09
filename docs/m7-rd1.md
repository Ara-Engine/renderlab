# M7-RD1: non-RT courtyard reflection blotches

Overnight unit (Islam's complete-base push, 2026-11-08/09; rules in AGENTS.md). Branch `t3/m7-rd1`, from origin/main `08aa1ba`, rebased onto `ff56b79` (LC3 and VF2 merged). Labels: **M** measured here, **I** inference.

## Status

| Item | Result |
|---|---|
| Defect on current main `ff56b79` (M) | **Does not reproduce.** In the courtyard (`examples/game_scene.rs`, `--fox`, 48 frames at 1280x720), the non-RT chrome and gold spheres reflect the wall, the cube, each other, the Fox and the ground. They match RT, still and moving, from frame 000. No blotches, and frame 000 is not black. |
| Cause of LC3's sheet (M) | **LC3's binary predated R1n.** Every frame in `D:/Coding/renderlab-work/lc3/final-inspected-sheet.png` came from runs `lc3-e`, `lc3-h` and `lc3-i`, built from `1324c4a`. That is LC3's commit on main `622db4d`, before the R1n merge (`562a74f`); `git merge-base --is-ancestor 562a74f 1324c4a` fails and `src/reflection_software.rs` is absent there. Its non-RT path traced no reflections, so the spheres showed their untraced estimate, and that estimate is what looks blotchy (below). RT had R1's hardware rays, so it was clean. |
| Fix | **None needed in the renderer.** The branch adds the example's `--no-reflections` switch, which reproduces the sheet's look on demand, and the RD1 lease runner. With default flags its frames are byte-identical to main's on both paths (13 of 13 written frames each). |
| RT | Unchanged: byte-identical to main (the same frames as above). |
| Lib tests | Not run: the branch changes no library code. Its only code change is the example's argument parsing. Root's merge gates cover main's library. |

## 1. Bisect

All runs use one `game_scene` binary per row, `--fox D:/Coding/renderlab-work/m7-sk1/Fox.glb`, 48 frames at 1280x720 with every 4th frame written, and the example's defaults otherwise: GI, TR1, local shadows, particles, glass, post and fog. They are headless, inside RD1 leases (`scripts/m7-rd1-run.py`), on an RTX 4070 SUPER with release builds.

### 1.1 Main `08aa1ba` (run `a1`, binary `a`: main plus a temporary coverage log)

The log was an env-gated `eprintln` in `prepare_primary` and `prepare_software_reflections`, and it is not in the branch. Per frame, it showed:
- 11 draws and 28 placements every frame;
- **covered = true** every frame, with bands `[0.5, 0.35]`;
- 4 distinct meshes and 27 traced placements. The 28th placement is the skinned Fox, which is skipped and does not break coverage;
- one pack on frame 0 (generation 0 → 1). After that, record-only rewrites (the spinning crate and the Fox walk) count as transform edits.

| Variant | Chrome and gold, non-RT |
|---|---|
| default | clean; matches RT |
| `--no-temporal` | clean |
| `--no-gi` | clean; the spinning crate's blotches (both paths) also go away (section 3) |
| no `--fox` | clean |
| `--no-particles --no-glass` | clean |
| `--still` | clean |
| **`--no-reflections`** | **LC3's look**: chrome dark at 000, blocky dark patches at 008-016, matte grey later |

### 1.2 Suspects

| Suspect (brief) | Verdict (M unless marked) |
|---|---|
| 1. R1n software geometry or frame-0 preparation | Frame 0 packs, binds and traces; coverage holds every frame; the chrome reflects at 000. |
| 2. History and restarts: the transform-accounting line, MQ2's view cap, MQ1's box | Records are rewritten every animated frame and counted as transform edits; moving non-RT matches RT. The line was not reverted: with no defect to bisect, a revert only restores MQ2's every-frame TR1 restart. |
| 3. RQ1's 4-16 glossy rays | On main and clean on both paths; not toggled, for the same reason. |
| 4. Skinned Fox and particles | The Fox is skipped without zeroing coverage. With or without the Fox, particles and glass, the result is the same. |
| 5. LC3 hit shadows | LC3's re-merge `938ee90` (run `b2`) and main `ff56b79` (run `c1`), which has LC3: clean. |

VF2's main integration `7cb8379` (run `b1`, code-identical to main `2aea7ed`) is clean as well.

### 1.3 Binary provenance

These builds shared one Cargo target directory across worktrees, which can return another tree's artifacts. Each executable was checked for WGSL text unique to each unit:
- `let visibility=rl_local_tap(light,receiv` for LC3;
- `seed=id.x*73856093u` for VF2;
- the `--no-reflections` flag string.

Binary `a` has neither unit's text. `lc3` has LC3's only, `vf2` has VF2's only, and `main-ff56b79` and `rd1` have both. `rd1` alone also has the flag.

## 2. Final check on main `ff56b79` (runs `c1`, `c2`)

- `D:/Coding/renderlab-work/rd1/rd1-final-spheres.png` (sphere crops, frames 000, 008, 016, 024, 032 and 047). Its rows:
  - **before**: LC3's `lc3-i` non-RT frames from the pre-R1n base;
  - main non-RT moving;
  - main RT moving;
  - main non-RT still;
  - main RT still;
  - main non-RT `--no-reflections`, the fallback, which reproduces the before row.
- `D:/Coding/renderlab-work/rd1/rd1-final-full.png`: full frames 000, 024 and 047, non-RT above RT.

**Inspected:**
- The chrome sphere reflects the sky, the wall, the cube, the gold sphere and the Fox on both paths in every frame, still and moving, at the same places and sharpness.
- The gold sphere's glossy reflection carries the same colours on both paths.
- The lantern bulbs are lit on both paths at frame 000.
- Nothing on the spheres is visibly worse on non-RT.

## 3. What the sheet did show (recorded in m7-gaps.md)

- **The untraced estimate is blotchy in motion** (M, `--no-reflections` on main). The estimate is the environment plus the world radiance cache's specular. A non-RT frame shows it whenever the coverage rule switches tracing off:
  - any drawn mesh over 2^19 triangles, or a virtual mesh;
  - `ReflectionSettings::off()`.

  In that case the chrome is near black at a cut and shows blocky dark patches for roughly the first 8-24 frames of motion. That the patches are the cache's cells is inference (I): the `--no-reflections --no-gi` screen was queued but did not run.
- **The spinning crate is blotchy on both paths** while GI is on (frames 008-032), and clean with `--no-gi` (M, `a1`). This is a GI-on-moving-object issue, not a reflection one.

## 4. Process

- **Leases:** `rd1-a` ran nothing (a batch-script argument bug, fixed). `rd1-b` ran the bisect, `rd1-c` the VF2 and LC3-interim screens, and `rd1-d` the final check. Each took 1-4 minutes of GPU time. RD1 held priority under the 23:53 root order; G3 took the slot at TR2's release ahead of `rd1-d` and released it early.
- **Time:** about 45 minutes of investigation and verification, about 10 minutes of code and docs, and about 10 minutes of lock waits. Builds took 5 minutes cold, then about 20 seconds incremental.
- **Evidence** in `D:/Coding/renderlab-work/rd1/runs/`:
  - `a1`: the bisect, with the coverage logs in each variant's `stderr.log`;
  - `b1`: VF2 `7cb8379`;
  - `b2`: LC3 interim `938ee90`;
  - `c1`: main `ff56b79`, both paths, moving and still;
  - `c2`: this branch, default and `--no-reflections`, with `run.json` frame hashes.

```powershell
py -3 -B scripts/m7-rd1-run.py LABEL 15 25 -- powershell -NoProfile -ExecutionPolicy Bypass -File D:/Coding/renderlab-work/rd1/batch-c.ps1
```
