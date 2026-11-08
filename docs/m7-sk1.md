# M7 SK1: animated meshes

Implementation branch: `t3/m7-sk1`, based on `984a18b`, updated with main's
`f159fcd` (default GI/bounce, TR1b, R1 and AT1), then `c7855a5` before the final
checks. This is the 2026-11-08 overnight base, with the remaining limits listed
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

One `SkinnedMeshInstance` owns one independently posed primitive. Multiple
placements may share a pose; independent poses require independent instances.
Joint palettes are accepted only as finite, nonsingular affine matrices. Each
admitted render uploads the palette and dispatch parameters. `UploadBytes::skinning`
counts those bytes separately from static geometry edits.

There is no opt-in renderer switch: uploading a skinned mesh enables skinning
automatically on RT and non-RT devices.

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

## Verification record

The first guarded run (`sk1-a1`, 16:27:57-16:28:22 UTC) passed all available lib
tests: asset **4**, cooker **16** (2 existing ignored), renderer **149** (146
ignored). The GPU current/previous-position oracle passed on a non-RT device and
an RT device with zero Vulkan validation errors. The example stopped before
rendering because its initial GI direction count was unsupported; it is corrected
to 64. The retained image/timing run is recorded below once completed.

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
- The example uses the renderer's existing shadow-method contracts, including
  double-sided opaque hardware shadow casters.
- No new Unreal/reference campaign is part of this overnight unit; no Unreal
  equivalence or engine-wide performance bar is asserted.
