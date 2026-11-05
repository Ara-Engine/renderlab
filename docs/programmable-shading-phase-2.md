# Programmable shading, phase 2: toon

2026-11-05, based on main `c22e6a6`. **The phase 2 gate passes on the registered
workloads.** This is a shading-program checkpoint, not film-quality or matched
Unreal acceptance. Mixed frames and outlines remain phases 3 and 4.

## Implementation

[`shaders/programs/toon.wgsl`](../shaders/programs/toon.wgsl) is a real alternative
to the standard program, using `surface()` / `shade()` and engine services only.
It keeps textured/vertex-coloured albedo, quantises normal-to-light incidence,
applies continuous engine shadow visibility to the bands, blends a shadow tint,
adds environment fill and a view-angle rim. There are no bindings, screen reads,
neighbour reads, custom shadow rays or engine lighting-model changes. The rim is
a surface term, not an outline post effect.

The seven reflected `Parameters` members are:

| Member | Type | Example value |
|---|---|---|
| `shadow_tint` | vec3<f32> | (0.12, 0.08, 0.24) |
| `bands` | f32 | 4 (integer-valued, clamped to 2–16) |
| `rim_color` | vec3<f32> | (0.12, 0.2, 0.3) |
| `rim_power` | f32 | 4 |
| `thresholds` | vec2<f32> | (0.12, 0.9) |
| `fill` | f32 | 0.25 |
| `flat_shaded` | u32 | 0 |

`thresholds` maps the dark/light endpoints before evenly spaced quantisation.
Shadow tint also supplies the unlit side's colour. Rim colour is exposed HDR,
intentionally independent of scene exposure. The style uses interpolated or
geometric normals; it deliberately ignores normal maps and metallic/roughness
response. Standard materials and their defaults are unchanged.

No production Rust API addition was needed. `src/program_gpu.rs` only registers
the new test module under `cfg(test)`.
[`src/toon_tests.rs`](../src/toon_tests.rs) demonstrates public program creation,
named material parameters and buffer updates. Private test access reads engine
counts, supplies the existing seeded visibility kernels and compares pipeline
handles; it does not implement another visibility or material API.

## Gate evidence

Windows/Vulkan, RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D,
wgpu/naga 30.0.1, release library tests, `-j 2`, 1x sampling, 16x anisotropy.
Quarry: the registered twelve full-resident draws / 13,034,903 source triangles,
1920×1080, prepared RLE2 environment, registered lighting and camera pair.
Material fixture: the authored M5 spheres, floor and mirrored/source-back panels,
960×640, one directional light and constant environment. Its materials are all
double-sided to meet the current soft-visibility admission contract; point and
spot lights from the original fixture are omitted for that same reason.

Both scenes exercise all-standard and all-toon frames at poses **0 → 1 → return**:
raster Exact, traced Exact (256 directions), and traced Interactive (8 directions)
with seeds 0–3. The programs use the same geometry, material factors, textures,
camera, lights and receiver policy. Interactive is explicitly selected for this
gate; this does not promote its known grain to a quality default.

Count readback follows the existing acceptance convention: untraced pixels in
unused scratch tiles are canonicalised to 256; every covered count byte is read
unchanged. Standard/toon comparison uses the complete resulting images.

| Gate | Result |
|---|---|
| Full images and native-resolution crops | Inspected for both scenes and all paths; six-path motion sheets include every seed and pose |
| Standard/toon shadow counts | 36/36 full count-image pairs byte-identical (18 per scene) |
| Static toon repeats | 1,152/1,152 image **and count** comparisons byte-identical: 32 × 6 paths/seeds × 3 poses × 2 scenes |
| Return frames | Both programs' HDR and counts byte-identical to pose 0 in all 12 scene/path/seed combinations |
| Motion | Pose 1 changes the complete toon image in both scenes |
| Standard primary gate | 285/285 files match the immutable current-main reference hashes |
| Standard Interactive gate | 36/36 files match; its separate 32-repeat loop also passes (384 comparisons) |
| Parameter update | Changing bands 4 → 2 changes the image on both paths; zero compiles, identical counts; restore reproduces the original bytes |
| Toon source edit | Exactly two active toon pipelines compiled per edit; standard raster/traced pipeline handles and images preserved |
| Invalid toon edit | Rejected at `program toon line 13:40`; no compiles; both programs keep rendering their accepted complete images |
| Source revert | Original toon HDR bytes restored on both paths |
| Validation | Core + Synchronization active, zero errors in all four completed GPU processes |

The reference hashes are
`E:/renderlab-work/env-dedup/runs/candidate-primary-hashes.json` and
`candidate-static-hashes.json`. That reference's production source is the source
at `543a296`; `543a296..c22e6a6` changes only documentation. The new scene loader's
18 standard images independently match those references as well.

The old external replay count files have been compacted. For the standard
Interactive test, the replay inputs were the new paired harness's standard count
captures, and Exact inputs were the newly regenerated primary gate. The separate
comparison of **all 36 resulting files against the immutable reference hashes**
is the regression oracle; the newly generated inputs do not replace it.

### Visual observations and raster/traced differences

Verified images are under `E:/renderlab-work/toon-phase-2/`:

- `fixture-overview.png`, `fixture-crops.png`, `fixture-motion.png`;
- `quarry-overview.png`, `quarry-crops.png`, `quarry-motion.png`;
- `fixture-1/pose-*-*-toon.png` and `quarry-2/pose-*-*-toon.png`: original full
  presentation captures, including all four Interactive seeds and return poses.

The fixture clearly separates four bands, coloured shadow/contact regions and a
blue rim from the standard glossy response. Its metal/roughness rows intentionally
share the same toon response. Silhouettes, floor contacts, the mirrored panel and
the source-back panel remain present. The quarry retains its scanned texture and
fine geometry, so its bands are less graphic than the smooth fixture; blue rim
accents and darker, tinted unlit areas are visible. No missing region or changed
shadow footprint was observed. At 1x there is visible edge/band aliasing, and
Interactive retains its inherited penumbra grain. The small registered camera
motion is checked, not continuous-motion film-quality stability.

Raster and traced toon frames are **not** required to match each other: they use
their existing raster-derivative versus traced-differential footprints and their
own receivers. Pose 0 complete exposed-HDR RGB differences:

| Scene/style | Mean absolute | RMS | Maximum | Pixels with any RGB difference >0.01 |
|---|---:|---:|---:|---:|
| Fixture standard | 0.00000602 | 0.001772 | 1.500 | 16 |
| Fixture toon | 0.00000355 | 0.001107 | 0.625 | 3 |
| Quarry standard | 0.00001901 | 0.000539 | 0.3870 | 294 |
| Quarry toon | 0.00001064 | 0.000657 | 0.3012 | 142 |

The crop inspection and receiver comparison put the differences in the existing
coverage/receiver/texture-footprint class, with toon band and rim discontinuities
able to amplify small input differences. This is not a cross-path bit-equivalence
or quality-improvement claim. Of the quarry's 142 toon pixels above 0.01 at pose
0, 3 differ in coverage, 2 neighbour a silhouette, 12 have different engine
visibility counts and 85 have different geometric receiver normals; 40 remaining
shared-coverage shading differences are not uniquely attributed. The analogous
standard populations are 2 / 10 / 16 / 233 / 33. These are ordered diagnostic
buckets, not a proof of the cause of every pixel. Pose 1 and all values are in
`raster-traced.json` and `difference-classes.json`.

### Edit latency

One process-unique edit per path per scene, Core + Synchronization validation on.
The clock starts before writing a scratch toon file, immediately reads it into
`reload_program`, and ends after the first frame completes on the GPU. This is
**save → API reload → completed offscreen image**; it excludes file-watcher
polling, presentation and scanout. Repeated-source cache hits are not reported as
new compiles. Other-project CPU compilation was observed during the session;
these are single-sample development-loop observations, not controlled frame-time
benchmarks or a comparison with standard reload cost.

| Scene/path | Compile (ms) | Save to image (ms) | Pipelines |
|---|---:|---:|---:|
| Fixture raster | 213.9 | 232.4 | 2 toon |
| Fixture traced | 384.5 | 396.7 | 2 toon |
| Quarry raster | 205.7 | 236.6 | 2 toon |
| Quarry traced | 378.5 | 391.3 | 2 toon |

The design's 100 ms target remains unmet for these uncached edits.

## API gaps encountered

Gaps 1-3 below are the historical phase-2 observations, now addressed by the
[API follow-through](programmable-shading-api-gaps.md). Visibility admission in
item 4 remains unchanged.

1. **Custom program watching is a caller concern.** The public reload transaction
   works, but the viewer's `ShaderWatch` names the standard program only. There is
   no turnkey toon watcher/selection UI. This gate calls the public reload API;
   it does not claim viewer polling coverage for toon.
2. **Parameter defaults and schema metadata are absent.** Omitted fields are
   zero, so the application must provide meaningful values when creating each
   material. The example parameter list lives in the test and is shown above.
   Basic scalar/vector members suffice here; arbitrary threshold arrays are not
   supported, so this shader exposes two endpoints plus evenly spaced bands.
3. **Normal-map reconstruction is not a public helper.** Sampling and frame
   inputs exist, but a style wanting the standard perturbed normal would copy its
   frame construction. This style intentionally uses smooth/geometric normals;
   it does not require an engine addition.
4. **Existing visibility admission still applies.** Soft visibility accepts one
   directional light and opaque double-sided regular geometry. Toon cannot
   select another visibility budget or change receiver origins. This preserves
   engine-owned shadows and is not a new lighting-model constraint in hit data.

No missing input blocked this style. There is no dispatch, receiver or hit-record
specialisation for toon, and no phase 3/4 implementation.

## Reproduction, checks and time

Build the library test executable with your own E: target and two jobs:

```powershell
cargo test -p renderlab --lib --release --no-run -j 2 --target-dir E:/renderlab-work/toon-phase-2/target
py -3 scripts/toon-run.py --binary E:/path/to/renderlab-test.exe --manifest E:/path/to/manifest.json --output E:/new-fixture-evidence --test fixture
py -3 scripts/toon-run.py --binary E:/path/to/renderlab-test.exe --manifest E:/path/to/manifest.json --output E:/new-quarry-evidence --test quarry
```

The manifest uses the asset, limits, cameras, lighting and environment paths from
the standard primary tests, plus `static_repeats: 32`; fixture needs only output
and repeat count. `--test primary` and `--test interactive` run the original gates;
`--reference-hashes` compares their entire capture sets with a saved hash map.

The runner creates `D:/Coding/renderlab-work/gpu-slot.lock` atomically and writes
`owner.json`, checks for games, Unreal and foreign renderlab GPU jobs, requires
6 GiB available RAM **before launching**, monitors for process conflicts and
sets a maximum 44-minute child timeout. Every lease records acquire/release UTC.
All work here was offscreen/background; no window, input automation or desktop
capture was used.

CPU verification: 79 non-GPU library tests pass, including the real toon shader's
naga/SPIR-V composition on all five stage/visibility variants and reflected
parameter checks. Affected-crate release Clippy `--lib --tests -D warnings`,
`cargo fmt --all -- --check`, Python syntax and `git diff --check` pass.

Implementation/read/analysis and verification were interleaved. The first cold
compile ran about 100 s and found a test-only array/Option mismatch, corrected
before any GPU test; the successful incremental build took 44.34 s. Clippy took
26.48 s; CPU tests took 0.57 s. Completed GPU test children took about 217.4 s:
fixture 19.0, quarry 32.8, standard primary 84.0 and standard Interactive 81.7.
Image analysis/inspection is separate from those execution times; active editing
time was not separately instrumented.

One additional 7 s quarry child was stopped by the first runner's overly strict
in-run memory floor during asset upload (initial admission had 8,070 MiB free).
The runner was corrected to apply the required floor before launch, matching the
existing determinism runner, while continuing to watch process conflicts. That
interrupted run is excluded from acceptance. The clean quarry rerun admitted
with 8,958 MiB; the other successful preflights admitted with 7,384–9,543 MiB.
No game, Unreal or foreign renderer appeared in any preflight/monitor sample.

Five owned GPU leases ran between **14:06:07.093467 and 14:13:20.209803 UTC**,
totalling 235.07 s; the longest was 85.97 s. All five were released, including the
interrupted run. Their individual acquire/release times are in `gpu-windows.json`.

All evidence, manifests, build identity, inspected PNGs and JSON/hash/log summaries
are in `E:/renderlab-work/toon-phase-2/`. Bulk raw captures are compacted after
analysis; the worker's Cargo target directory is removed at handoff. See
`summary.json`, `gpu-windows.json` and `cleanup.json` for final timing/cleanup
records. No private scan data or generated capture is committed.
