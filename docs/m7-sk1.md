# M7 SK1: animated meshes

Implementation branch: `t3/m7-sk1`, based on `984a18b`, updated with main's
`f159fcd` (default GI/bounce, TR1b, R1 and AT1), then `f870cb4` (API1) before the
final checks. This is the 2026-11-08 overnight base, with the remaining limits listed
below. It makes no Unreal quality or performance claim.

## Asset and cache contract

`renderlab_asset::skin::SkinnedAsset` owns the ordinary prepared geometry, a node
hierarchy, skins with joint-node indices and inverse bind matrices, mesh/node/skin
bindings, four joint indices and weights per vertex, and animation clips.

The standalone **RLS v1** format has `RLS\0` magic and schema version 1, wrapping
an ordinary RLA payload. Existing RLA v1-v6 formats, their meaning and the static
cook recipe are unchanged. The old cooker still rejects skins and animations.
`renderlab_cook::skin::{prepare,cook_file}` opts into them. Content-addressed
entries live under **`skin-v1/`**, keyed by the source, versioned recipe, settings
and limits. Decoding validates spans, aggregate limits, hierarchy cycles, joint
indices, weights and animation keys before runtime use.

Import is embedded GLB 2.0, with the existing material/texture subset. It imports
JOINTS_0, WEIGHTS_0 (float or normalized integer), inverse bind matrices (identity
when omitted), and translation/rotation/scale channels. Influence remapping
follows indexed triangle corners through flat-normal and tangent duplication.
It never matches vertices by position. Weights are normalized at import.

Nodes must use TRS. Matrix nodes, morph targets and JOINTS_1/WEIGHTS_1 are rejected
explicitly. The sample importer does not resolve external glTF resource URIs.

## Runtime API

```rust
use renderlab::{animation::ClipPlayer, Draw};

// Load once with renderlab::asset::skin::decode, after cooking with the skin recipe.
let binding = &asset.bindings[0];
let primitive = &asset.geometry.meshes[binding.mesh as usize].primitives[0];
let influences = asset.influences[binding.mesh as usize][0].as_ref().unwrap();
let mut mesh = renderer.try_upload_skinned_mesh(
    primitive, influences, asset.skins[binding.skin as usize].joints.len() as u32,
)?.expect("retry after polling when the upload queue is busy");

let mut player = ClipPlayer::new(1); // Fox's Walk clip in the pinned source.
player.advance(delta_seconds)?;
let pose = player.sample(&asset)?;
mesh.set_joint_palette(&pose.joint_palette(&asset, 0)?)?;
let draw = Draw::new(mesh.mesh(), &mut placements);
```

The clip sampler supports STEP, LINEAR (quaternion shortest-arc slerp), and
CUBICSPLINE (Hermite tangents scaled by key interval, rotations normalized).
It supports looping, seek and playback speed. A sampled mesh-local palette is
`inverse(mesh global) * joint global * inverse bind`.
Apply `pose.node_transform(binding.node)` as the draw's model, or premultiply it
into those palette matrices while keeping a fixed model. This preserves animated
mesh-node transforms as well as joint animation. Fox's mesh node is identity.

One `SkinnedMeshInstance` owns one independently posed primitive. Multiple
placements may share a pose; independent poses require independent instances.
Joint palettes are accepted only as finite, nonsingular affine matrices. Each
admitted render uploads the palette and dispatch parameters. `UploadBytes::skinning`
counts those bytes separately from static geometry edits.

There is no opt-in renderer switch: uploading a skinned mesh enables skinning
automatically on RT and non-RT devices.

API1's `Engine` facade currently lists SK1 as an extension point. This unit
provides the typed renderer API above; storing independent poses behind
`MeshId`/`ObjectId` and replacing API1's example hero remain facade integration work.

## GPU and integration

The material-independent compute pass runs before every consumer. It writes the
ordinary 48-byte vertex stream: skinned positions, inverse-transpose normals and
skinned/orthogonalized tangents. The previously submitted vertex stream is kept
on the GPU. The first pose initializes current and previous positions identically.
Busy/rejected frame admission does not advance pose history.

- **Programmable shading:** the vertex/material and hit-record ABIs retain their
  ordinary semantics. Skinning does not assume a lighting model.
- **TR1:** a previous-vertex storage array supplies the same hit triangle and
  barycentrics in the previous pose. Reprojection uses its previous point and
  tangent plane. A still camera is not treated as a still deforming scene.
  Static draws alias their current stream. Geometry descriptor-array admission
  increases from 11 to 12 arrays per draw; the public limit helpers include it.
  A padding vec4 in a previous-pose stream identifies skinned geometry by buffer
  length without stealing placement or material bits. Only pixels touching skinned
  geometry use the moving-surface history rule; the static quarry keeps its still
  accumulation. Moving-character neighbourhoods bypass the still-silhouette cache;
  TR1b's two history slots retain their individual surface identities.
  Static receivers reject lighting history inside a swept caster shadow volume.
  With dynamic GI enabled this rejection is conservatively scene-wide; refining
  that region and retaining more stable shadow samples remain performance/quality work.
  TLAS-only rebinding preserves reconstruction history; target texture changes
  still invalidate it. Both devices test continuity after the first frame.
- **Non-RT visibility:** full-detail original clusters retain their topology and
  primitive IDs. A second compute dispatch updates their duplicated positions.
  Conservative mesh bounds cover every skinned cluster; LOD attachment is rejected.
  This uses terminal clusters rather than a separate plain-draw implementation.
  Only the skinned draw's bounds records are republished in the shared table;
  existing visibility/shadow consumers keep their buffers and pipelines.
- **Shadows:** raster shadow passes read the same deformed stream. Virtual shadow
  pages/receiver regions are invalidated for deforming casters even if their model
  matrix and aggregate box stay unchanged; horizon records include pose revision.
  Pose changes refresh only the animated draw's horizon and walk records; the
  static quarry's records remain resident.
- **RT:** animated BLASes disable compaction, request ALLOW_UPDATE/PreferUpdate and
  fast-build behavior. Skinning precedes the BLAS update and TLAS build. Pose scratch
  is admitted/accounted and retained through its flight. `blas_updates` counts it.
  **wgpu 30 currently implements rebuilds for both PreferUpdate calls**, not actual
  BLAS refit or TLAS refit. This remains an explicit cost gap.
- **GI:** swept caster boxes include deforming meshes, and pose revisions prevent
  idle-cache reuse. Non-RT software geometry is refreshed from the admitted pose
  when its revision changes. This CPU BVH rebuild is a correctness base, not the
  final scalable dynamic-GI implementation. The current software-scene packer
  also recopies static scene geometry on each replacement: this dominates the
  quarry CPU frame cost and needs a persistent dynamic geometry range.

## Example, attribution and reproduction

`examples/skinned-mesh.rs` is a headless clip-player example. It cooks the source,
renders 32 walk frames with the product's default GI/bounce, TR1, reflection and
atmosphere settings, and writes five poses per path and scene. Both scenes check
GI caster invalidation. An optional prepared quarry argument
adds the same walk to the real quarry. The quarry asset is local and is not bundled.

The redistributable Fox source is unchanged, pinned by commit and SHA-256; the
example scales it, moves it and uses double-sided materials for the current RT
shadow contract. Model, rig, animation and conversion credits and CC0/CC BY 4.0
links are preserved in [the asset notice](../third-party/Fox.md).

```powershell
py -3 -B scripts/m7-sk1-asset.py D:/Coding/renderlab-work/m7-sk1
$env:CARGO_TARGET_DIR = 'D:/Coding/renderlab-work/m7-sk1/target'
cargo test -p renderlab -p renderlab-cook -p renderlab-asset --lib --no-run -j 2
$env:N8_OWNER = 'm7-sk1'
$env:N8_OUT = 'D:/Coding/renderlab-work/m7-sk1'
py -3 -B scripts/n8-run.py --label sk1-check --minutes 25 --fast-check -- `
  py -3 -B scripts/m7-sk1-check.py
py -3 -B scripts/m7-sk1-report.py
```

Every renderer test binary and GPU run is inside the shared `gpu-slot.lock` lease.
Check `E:/renderlab-work/gpu-schedule.md` for QUIET/game holds first. Bulk outputs
and the worker target are on D:. The worker target is deleted when the unit finishes.
The retained run's target was cleaned with
`cargo clean --target-dir D:/Coding/renderlab-work/m7-sk1/target` (2.0 GiB removed).
Its oversized cooked RLS cache entry was also removed; the pinned source, inspected
PNGs and measurement/log summaries remain. The skin recipe regenerates the cache.

## Verification record

Retained run **`sk1-api1-15`**, 2026-11-08 **18:22:39–18:24:56 UTC**, verified
runtime source **`e77a4cdd648d595709cbf362a1224bd422143c64`**, including merged main
`f870cb4`. The later report-only commit does not change that runtime source.

- Asset **4**, cooker **16** (2 existing ignored), renderer **163** (149 ignored)
  lib tests passed. Both explicit SK1 GPU tests passed. Exit 0, no guard failures,
  **zero Vulkan validation errors**; existing unused-vertex-attribute warnings remain.
- GPU current/previous positions match the CPU deformation oracle; inverse-transpose
  normals survive rotation and nonuniform scale. A skinned pose matches an ordinary
  baked-geometry image **exactly (maximum byte error 0) on both paths**. This isolated
  direct-shadow oracle disables GI; the separate default walk keeps GI/bounce on.
- Covered shadows: non-RT raster directional/point/spot and Virtual; RT Exact,
  Virtual and Interactive. Both devices assert TR1 history continues after frame 0.
- The full default walk ran **32 frames per scene/path at 960×540**, saving frames
  0/8/16/24/31. I inspected the entire 20-frame contact sheet and full-resolution
  frame 16 for every scene/path: coherent silhouettes and moving shadows, without
  the earlier retained-silhouette and shadow trails. This does not certify an
  Unreal quality floor or all possible animation/material combinations.
- GI ran for all 32 frames in each configuration. Sums of invalidation counter
  snapshots at 8/16/24/31 were **456,064 / 283,301** for non-RT fixture/quarry and
  **554,116 / 642,170** for RT fixture/quarry. These are sampled counts, not unique
  cache cells. Non-RT explicitly reports software GI; RT reports hardware GI.

The retained source also fixes an integration defect exposed by the first merged
R1 run: Naga 30 emitted illegal opaque `OpConstantNull` instructions for conditional
ray-query declarations. Function-scope queries preserve behavior and compile cleanly;
the existing primary SPIR-V test now checks for opaque nulls in backend output.
An intervening run was stopped by the unchanged 2 GiB runtime memory guard while
13 external Rust compilers were active; its partial results are not retained timings.

Local evidence (ignored bulk output on D:):

- [Inspected walk contact sheet](D:/Coding/renderlab-work/m7-sk1/images/contact-sheet.png)
- Quarry frame 16: [non-RT](D:/Coding/renderlab-work/m7-sk1/images/nonrt-quarry-16.png),
  [RT](D:/Coding/renderlab-work/m7-sk1/images/rt-quarry-16.png)
- Fixture frame 16: [non-RT](D:/Coding/renderlab-work/m7-sk1/images/nonrt-fixture-16.png),
  [RT](D:/Coding/renderlab-work/m7-sk1/images/rt-fixture-16.png)
- [Timing chart](D:/Coding/renderlab-work/m7-sk1/images/timings.png),
  [measurement JSON](D:/Coding/renderlab-work/m7-sk1/images/summary.json),
  [guard/lease JSON](D:/Coding/renderlab-work/m7-sk1/runs/sk1-api1-15.json),
  [complete test log](D:/Coding/renderlab-work/m7-sk1/runs/sk1-api1-15.log),
  [compiler, hardware and executable hash](D:/Coding/renderlab-work/m7-sk1/build.json)

### Diagnostic timing

Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14, Vulkan, Rust 1.96.0. Development
test build with renderer opt-level 1 and debug assertions. Product GI/bounce,
TR1, reflection and atmosphere defaults remain selected. Non-RT uses visibility
primaries + Virtual shadows; RT uses traced primaries + Interactive shadows.
The lease saw external CPU builds (up to 5 build processes in the coarse samples),
no foreign GPU process or game, and no foreground change. This is functional timing,
not a quiet-window optimized benchmark or an RT/non-RT superiority comparison.

| Path / scene | CPU prepare/encode/submit ms | GPU render span ms | Synchronized frame wall ms | Skin compute + cluster copies µs |
|---|---:|---:|---:|---:|
| non-RT fixture | 7.56 | 11.36 | 19.80 | 8.89 |
| non-RT quarry | 242.10 | 41.15 | **309.84** | 8.37 |
| RT fixture | 6.70 | 5.77 | 14.64 | 8.10 |
| RT quarry | 3.05 | 10.72 | 14.60 | 8.27 |

Frame values average frames 24–31. Synchronized wall time includes submission,
AS updates and the explicit GPU drain, but excludes clip sampling/palette construction
before the timer. The renderer's existing GPU span starts after skinning/AS work;
it is **not total GPU frame cost**. The final column is eight warmed, separately
timestamped skin/cluster dispatches, excluding uploads, CPU bounds/BVH work and AS
updates. Palette/parameter uploads are 1,552 bytes/frame; non-RT additionally counts
896 bytes of cluster-bound metadata (2,448 total).

The non-RT quarry is a clear performance miss: the changed Fox pose triggers a
software-scene repack that copies static geometry too. The small skin kernel does
not explain or conceal that 242 ms CPU cost. No quality setting was reduced to
improve it.

Implementation/integration ran approximately **16:03–18:09 UTC**, including compile
and queue interruptions; exclusive active editing time was not instrumented.
The final no-run compile took **93 s**. The retained verification lease took
**137.01 s** (GPU-test suite 121.25 s); cumulative acquired verification leases,
including rejected/aborted iterations, took **613.19 s**, excluding lease queue time.

## Remaining gaps

- Actual incremental BLAS/TLAS refit requires backend support beyond wgpu 30's
  current rebuild implementation.
- Non-RT dynamic GI rebuilds the software BVH on the CPU for every changed pose.
  Its scene packer also recopies static geometry; full-quarry pose preparation is
  currently too expensive for game frame rates. Cluster bounds use the mesh-wide
  box, and shadow invalidation is conservative.
- Dynamic-GI receiver-history rejection is scene-wide, and independent skins
  currently own their compute pipelines. Crowds and dynamic-GI temporal stability
  need further work; this is a correctness base rather than a crowd renderer.
- No skinned cluster simplification/LOD, blend shapes, dual-quaternion skinning,
  clip blending, animation events or automatic root-motion extraction.
- Matrix nodes and additional influence sets are rejected, rather than silently
  approximated. Clip selection is currently by index.
- Existing TR1 resets on instance model-transform edits. Deforming vertex history
  works with a fixed placement; the example includes translation in the palette.
- API1's engine handle facade still needs its planned SK1 adapter. The renderer
  API and clip player are public and usable directly.
- The example uses the renderer's existing shadow-method contracts, including
  double-sided opaque hardware shadow casters.
- Normal-mapped skins, large crowds and reflection-specific skinned-hit scenes
  have not had a separate visual campaign. The shared current geometry stream
  and acceleration updates are covered by the focused geometry/shadow oracle.
- No new Unreal/reference campaign is part of this overnight unit; no Unreal
  equivalence or engine-wide performance bar is asserted.
