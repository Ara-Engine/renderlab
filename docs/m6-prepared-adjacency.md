# Prepared triangle adjacency

2026-11-06, based on `61131af`. Regular asset preparation now computes the exact
receiver-edge table after final vertex/tangent remapping. Schema 3 stores its
little-endian words per primitive; geometry sidecars and new manifests also use
version 3. The cache recipe changes so old entries cannot masquerade as prepared
adjacency. [The format](asset-format.md) specifies admission and validation.

`Renderer::try_upload_primitive` uploads the stored table into the existing index
suffix. No shader, triangle ordering, raster/BLAS source index count, or shading
ABI changes. Missing/ambiguous edges keep their existing ray-query fallback.
Legacy assets and procedural meshes reuse the moved builder from the asset
crate. Keeping this small fallback avoids rejecting existing prepared files and
procedural fixtures; supplied adjacency never enters the welding/sort branch.
Insufficient suffix capacity keeps the existing original-buffer/probe path.

`renderlab-cook prepare-adjacency SOURCE.rla OUTPUT.rla --limits FILE` upgrades a
composed asset, refuses its input as output, checks the added decoded-byte budget,
verifies the encoded result and publishes it atomically. The registered quarry
original remains intact. Its new sibling is
`E:/renderlab-work/cook-triangle-adjacency/full-adjacency-v3.rla`.
An independent streaming comparison checked all 1,548,588,296 original bytes
outside the version word, including vertices, indices, placements, materials and
textures. All match. Original SHA-256:
`1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`;
prepared: `f03cbe0807e414dd0c7e407e8de6231d114391ec908f5251e8548bfbeba1ef80`.

The payload grows from **1,548,588,300 to 1,705,007,376 bytes**: +156,419,076
bytes (**149.17 MiB, 10.10%**), including 240 bytes of format metadata. The table
also retains 149.17 MiB in decoded host storage. GPU geometry uploads are exactly
unchanged at 631,261,656 bytes, including the same padded suffix.

## Measured load and first image

One original/cooked pair in the same optimized executable, each loaded once on
the same E: drive. RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7 7800X3D, Windows/Vulkan,
wgpu 30.0.1; 1x MSAA, 1920x1080, the same quarry/environment/lights and Interactive
first frame. Startup begins before instance/device/renderer creation and ends at
the first completed offscreen image; it excludes process launch and preceding
fixtures. Load includes environment, prepared I/O/decode, setup, upload and drain.

| Interval | Legacy (ms) | Prepared (ms) | Observed reduction (ms) |
| --- | ---: | ---: | ---: |
| Runtime adjacency build, summed | 2631.216 | 0.001 | 2631.216 |
| Geometry/material/texture upload wall | 3751.821 | 1207.117 | 2544.704 |
| Total load | 7567.954 | 7514.900 | 53.053 |
| Startup to completed first image | 8030.707 | 7920.451 | 110.256 |

These are **single contended observations**, not a statistically established
end-to-end speedup. Prepared file read time was 5161.770 ms versus 2388.997 ms for
the original, absorbing most of the upload saving. OS file caches were not
controlled. Rust compiler count ranged 0–10 during the complete verification
lease; the live wallpaper renderer remained active. The earlier 1.669 s build in
the receiver report belongs to its different run. The retained quality and frame
cost conclusions are unchanged.

## Verification and latency

Evidence: `E:/renderlab-work/cook-triangle-adjacency/`. The single GPU batch passes
all receiver fixtures, all nine 1024-square micro-grids, and three poses ×
hard/Exact/Interactive × eight repeats for both original and prepared inputs.
Full HDR/hit/count SHA-256 values agree across assets; subsequent repeats use
exact GPU pixel comparisons with only mismatch-count readback. All 21 analytic
fixture oracles pass, including front-side grazing, thin and stress controls.
Validation records zero errors. All three complete quarry images and selected
fixture oracle sheets were inspected (`inspected-quarry.png`,
`inspected-fixtures.png`). No software-BVH or contact-fold-benchmark files changed.

181 CPU tests/doctests pass; 95 GPU/optional tests remain ignored by that CPU
command. Workspace all-target release Clippy with warnings denied and formatting
pass. Independent cold/warm fixture cache inspection passes on schema 3.
Commands, source/binary SHA-256 identities, startup values and all image hashes
are retained with the run JSONs. The test entry is
`adjacency_preparation_tests::gpu_prepared_adjacency_batch`.

The shared GPU lock and `gpu-schedule.md` protocol were used through builds and
verification, with 4 GiB available-memory preflight and a 2 GiB abort. Successful
GPU lease: 00:18:45–00:20:41 UTC; minimum available memory 2782.50 MiB. Earlier
resource-aborted attempts are retained and are not acceptance evidence.

Implementation and harness work took approximately 30 minutes, interleaved with
verification and resource queues. Retained CPU command: 13.06 s including its
incremental compile; Clippy: 64.74 s. Re-cooking: 38.47 s (3.19 s preparation);
independent payload comparison: 25.39 s. The single GPU test: 55.41 s; CPU fixture
replay: 57.83 s, together 116.50 s guarded wall. Waiting, interrupted compilation
and initial helper corrections are separate from these retained timings. The
worker's E: Cargo target is removed at handoff.
