# MV1: per-object motion

2026-11-08 overnight unit, branch `t3/m7-mv1`, based on main `fcb1630`
(PP1 merged), with main through `bccf730` merged at `79f8354` before the final screen
(RTDEF2, G2c, TP1 and VF1 included). Scope: submitted instance-transform history and the motion source read
by TR1 and PP1. MQ1 retains ownership of TR1's filtering/history policy.

2026-11-09 integration follow-up: merged main `622db4d` (MQ1) and then
`1ad5e29` (PC1/GC1) and `4c8ef7d` (R1n). The acceptance results below initially describe the earlier
delivery; the integration results are recorded separately below.
Final main sync: `dc3fa11` (ARA1 prep), which adds docs/new examples/tools and
changes none of the tested library, shaders or `game_scene` sources.

**Core motion verified on both paths; full quarry follow-through is incomplete.**
The quarry load hit the shared memory guard. Retrying with more launch headroom
was blocked by available RAM through the unit's two-hour box; no quarry result
or quarry timing is claimed. The unchanged real-quarry test remains in the branch.

## Implementation

The baseline did **not** reproject rigid motion. SK1 supplied previous mesh-local
vertices, but TR1 transformed them with the current instance placement. Transform
uploads also restarted all TR1 history, hiding the missing rigid reprojection by
discarding accumulation. PP1 reconstructed both clip positions from the current
world point, so its blur covered camera motion only and missed SK1 deformation too.

`Instances` now keeps its last successfully submitted transforms. Pending edits,
coalesced edits, failed admission and CPU buffer uploads do not advance that history.
The first submitted pose initializes both transforms identically. A successful
frame commits the current transforms; stopping therefore produces zero object
velocity on the next frame. Replacing a placement collection changes geometry-table
identity even when the mesh and instance count match, invalidating old surface IDs.
A collection omitted from the previous rendered frame initializes motion from its
current transform when it returns. Frame IDs exclude uploads and rejected admission.

The placement-ordered model buffer stores current transforms followed by previous
transforms. Its ordinary model indices and programmable material/vertex interfaces
are preserved. This adds 64 CPU bytes and 64 GPU bytes per instance, plus 64 bytes
in the CPU publication shadow; moving collections copy their submitted transform
snapshot. Deferred transform publication grows from 128 to 192 bytes per placement
(current/previous models and normals). Static frames retain the existing no-upload
comparison. The storage-binding size is checked before allocation. No new binding
or full-resolution texture is required.

`shaders/object_motion.wgsl` is the shared geometry source. Both hardware closest-hit
primaries and the non-RT visibility buffer already publish placement, original
triangle and barycentrics. TR1 and PP1 interpolate the same point on that triangle
in the previous submitted vertex stream, then apply the previous instance transform.
Static meshes alias their current vertex stream; SK1 supplies the previous pose.
Translation, rotation, scale, deformation and their combination therefore use the
same material-independent convention.

TR1 projects the previous point through the previous unjittered camera, and the
current point through the current camera, retaining its footprint offset and
existing history validation. Its previous tangent plane also uses the previous
placement. Rigid movers enter SK1's moving-silhouette source classification.
Transform/derived-normal uploads no longer restart every history. A moving rigid
caster initially extended SK1's swept world-space shadow-refresh region. The MQ1
integration removes that rigid-only extension: MQ1's nine-sample min/max clamp
handles moved-placement lighting changes and static receivers keep their history.
SK1's existing deformation refresh remains. These edits are marked MV1; MQ1 owns
resolve weights, windows, clamping, coverage storage and filtering policy.
RTDEF2's budget also sees rigid/deforming caster motion as motion, rather than
entering its settled-camera shadow budget while objects move. Its budget values
are unchanged. PP1 binds the primary-frame byte count from the owning module, so
RTDEF2's new sampling word is included instead of retaining a 112-byte literal.

PP1 writes `current_pixel - previous_pixel` into its existing depth/velocity texture,
in **output pixels**, including when its input is reconstructed/upscaled. Its shutter
fraction and maximum extent still bound the filter. The first frame and explicit
post-camera cuts suppress motion. TR1 reads the same previous-surface source directly;
PP1 does not read or modify TR1's private history.
Both consumers also stamp their previous camera with the rendered frame ID. A
diagnostic/non-reconstructed frame can advance instance history without advancing
their camera history; returning to lit output now restarts TR1 and suppresses post
motion for that frame. This keeps the camera and object snapshots aligned.

## Verification

The first screen (`screen-a-3`) stopped after the lib suite: 161 passed and ten
shader-composition tests failed on a duplicate `query` declaration inherited from
main's PP1/SK1 merge. Both units had hoisted R1's query to avoid Naga's invalid opaque
null initialization, leaving declarations at function entry and immediately before
the loop. MV1 removes the latter and reuses the entry declaration. No reflection
algorithm, settings or history behavior is changed by that integration repair.

`screen-b-9` passed all 171 lib tests and the SK1 GPU regression on both paths
(deformed versus baked images: maximum byte error 0; zero Vulkan errors). The
non-RT moving fixture passed all motion, continued-history, stopped-motion and
cleared-background assertions; its blur-off/on frames were inspected. During the
following quarry load the live memory guard stopped the batch below its unchanged
2 GiB floor. Preflight had 8,644 MiB available and concurrent foreign compilers.
This is a resource-aborted screen, not a quarry/RT motion acceptance result.

All test binaries and GPU commands run through
`scripts/n8-run.py` with `D:/Coding/renderlab-work/gpu-slot.lock`; the MV1 wrapper
checks shared QUIET/game-hold state. Compiles use two jobs and an owned D: target.
The full-scene retries raised launch admission first to 12 GiB, then 10 GiB; both
leave the shared 2 GiB runtime floor and all image-quality settings unchanged.

The focused GPU fixture has a stationary camera and a translating, bobbing, rotating
striped object. It checks per-pixel velocity against CPU barycentric projection,
zero first-frame/stopped velocity, continued TR1 history after transform edits, and
the cleared HDR region behind the moving object. Both paths retain blur-off/on
frames. The full-scene test adds a moving/rotating cube to the quarry; the unmodified
`examples/game_scene.rs` supplies its bobbing hero/spinning crate/camera flight.
Each full-scene path loads the quarry once; the final eight frames provide diagnostic GPU
timestamps. The existing SK1 stream/history GPU regression checks source integration.

Retained evidence:

- `lib-h-1`: **182 lib tests passed** on the final source, inside the n8 lease.
- `fixtures-d-5`: both-path motion fixtures and 24-frame, 960×540 game-example
  flights passed. SK1's deformed-versus-baked images had maximum byte error 0 on
  both paths, with zero Vulkan errors. These example frames use consecutive lit
  renders; the later diagnostic-resume guard does not change that sequence.
- `history-e-8`: corrected the history statistic to exclude uncovered stale hits.
  The earlier raw maximum of 256 included background and is **not** moving-surface
  evidence. All covered moving samples retained history with maximum weight 16.
- `resume-f-7`: final source, both-path fixtures passed, including the stricter
  coverage/region mask and the diagnostic-view → lit resumption regression.
  Both runs reported zero Vulkan errors and the guard reported no failures.

`resume-f` velocity compares every covered object pixel against CPU barycentric
projection at frames 0, 8 and 23. Maximum half-extent error was **0.00001296 output
pixels** on non-RT and **0.00001151** on RT. Initial velocity was exactly zero;
stopped velocity was at most 0.00000139 px. At frame 23, all 6,187 non-RT and 6,189
RT moving samples had history weight greater than 2 (maximum 16). The expanded
cleared region behind the object had **zero HDR colour error** against the clear
colour on both paths. Returning from a moved diagnostic camera restarted TR1 and
gave exactly zero post velocity. These are bounded fixture checks, not an
engine-wide no-ghosting certification.

Final test binary SHA-256:
`0F2C236A0AFE7B70FE34A4106A9CB732C80C084F730B5577DE1DEF71181371BC`.

### Inspected frames

The full four-frame blur-off/on fixture sequences and the game-example contact
sheets were inspected. The translating/rotating stripes remain distinct from the
cleared background, with short object-aligned blur rather than a long retained
silhouette. The example's rotating/bobbing crate, hero, shadows and scene geometry
remain present on both paths. No full-quarry frame was produced by the failed load.

- [Non-RT fixture, blur off](D:/Coding/renderlab-work/m7-mv1/resume-f/nonrt/inspect-fixture-off.png)
- [RT fixture, blur on](D:/Coding/renderlab-work/m7-mv1/resume-f/rt/inspect-fixture-on.png)
- [Non-RT game flight](D:/Coding/renderlab-work/m7-mv1/fixtures-d/example-nonrt/inspect.png)
- [RT game flight](D:/Coding/renderlab-work/m7-mv1/fixtures-d/example-rt/inspect.png)
- [Final motion data and timestamps](D:/Coding/renderlab-work/m7-mv1/resume-f/summary.json)

### Diagnostic timing

RTX 4070 SUPER, NVIDIA 617.14, Vulkan; Cargo test profile with validation and warm
driver caches. Fixture: 320×192, TR1 native, no GI, programmable striped material,
stationary camera, translating/rotating object; neutral post with blur off/on,
180° shutter and 16 px maximum half extent. Eight moving-frame timestamp samples
per cell, no timing readbacks between configurations beyond the existing collector.
Foreign CPU builds were allowed by the functional lease. Medians below are
diagnostic; there is no optimized, quiet or comparative speed claim.

| Path / post | Renderer `render_span` ms | Post and tone interval ms |
|---|---:|---:|
| Non-RT / blur off | 0.199 | 0.004 |
| Non-RT / blur on | 0.278 | 0.076 |
| RT / blur off | 0.075 | 0.005 |
| RT / blur on | 0.147 | 0.078 |

The earlier cold/contended `fixtures-d` medians were substantially higher (raw
samples are retained). Do not treat that difference as an MV1 speed improvement.
Quarry moving-caster timings and full-image acceptance remain missing.

Implementation/integration ran from approximately 19:15 UTC through the final
incremental compiles at about 20:53; GPU and memory queues overlapped it. The final test
compile took 87 s and example compile 31.4 s. Owned verification leases, separate
from queue time and frame time, total about 6.6 minutes through `lib-h` (including
the failed early screens). The 12 GiB full-scene request waited 75 minutes without
running. The two-hour unit box is closed with the quarry gate explicitly unmet.

Evidence root: `D:/Coding/renderlab-work/m7-mv1/`. No private asset or generated bulk
output is committed.

At the initial delivery, all owned build/test processes finished and the unstarted `screen-c`/`quarry-g`
requests were withdrawn. **Target cleanup was blocked:** automatic execution
approval rejected `Remove-Item -LiteralPath D:/Coding/renderlab-work/m7-mv1/target
-Recurse -Force`, stating only "blocked by policy". The resolved path was verified
as this unit's own D: target before the attempt; no deletion workaround was used.
The coordinator subsequently removed that target before this integration follow-up.

To finish the missing gate when RAM permits, rebuild with `scripts/m7-mv1-run.py
build` if needed, then run `scripts/m7-mv1-batch.py` **through the MV1 `gpu` wrapper**,
using the retained `screen-b/nonrt.json` as the quarry template, a new output
directory and the staged example executable. The batch includes lib/SK1 checks,
both motion/quarry paths and both example flights. Do not call it accepted on the
quarry merely because the small-scene checks passed.

## MQ1 integration (2026-11-09)

MQ1 independently fixed rigid reprojection while MV1 was being implemented. The
merged renderer has **one transform history**, owned by `InstanceState` and
published in the shared primary placement buffer. TR1 no longer caches another
copy of transforms or appends it to its frame-state buffer. Both TR1 and PP1 read
`object_motion.wgsl`; it combines the previous submitted placement and SK1's
previous vertex stream on traced and visibility-buffer hits.

MQ1's moved-surface classification, still-silhouette guards and ordinary min/max
clamp over all nine current samples remain. The `motion_box` setting remains on
by default. Its event bit comes from the same bitwise current/previous transform
comparison as the shared shader source. MQ1's original
`gpu_mq1_moving_placement` test is unchanged byte for byte. The rigid-only swept
receiver reset from the first MV1 delivery is removed so MQ1's static-ground
history continues; SK1's deformation reset still applies. Diagnostic-camera
resumption and the shared primary-frame ABI size remain fixed by MV1. That ABI is
now 144 bytes after R1n, so PP1 follows the owning constant automatically.

R1n introduced another reset source: its transform-only software-reflection record
writes were charged to `uploads.geometry`, which TR1's topology key hashes. MQ2
reported the regression while this integration was queued. MV1 changes that one
charge to `uploads.models`; real geometry repacks still count as geometry. RQ1
also carries that accounting correction. This keeps non-RT rigid motion from
restarting all histories after the R1n merge.

`reflections.wgsl` has exactly one `var query:ray_query` in `rl_hit_shadow` and one
in `rl_reflection_prepare`, both outside repeated execution scopes. Lib shader
validation also exercises the composed modules and SPIR-V opaque-query check.

The focused static-identity control reads the immutable resolve shader from main
`622db4d`, SHA-256
`3ac3d5e68ef9fc3f7c5a67de2319e0cf09b9073dbb3e0b6782a014f0343986be`.
For each path it swaps only that resolve versus the merged resolve, resets TR1's
sequence, and compares HDR bytes after 32 frames, with both a stationary and a
moving camera viewing a stationary translated/rotated striped object. Test-only
frame storage includes one unused matrix so the old shader's runtime-array ABI
can compile; production's fixed frame words contain no duplicate transform history.
This complements MQ1's recorded RF/quarry identity evidence; it does not repeat
those large-scene campaigns.

Integration evidence and build identities live in
`D:/Coding/renderlab-work/m7-mv1/integration-n/` and `integration-build.json`.
The target was freshly created under this unit's D: directory, with two Cargo jobs.
The wrapper acknowledges the exact latest `[root order]` and checks its required
subsequent releases before requesting a lease. It checks again inside the lease
and before every test/example process. An initial attempt yielded before lib tests
at 8,187 MiB available, 5 MiB below the original floor; no test ran. The coordinator
then explicitly authorized **6 GiB admission for this quarry-free batch**, with a
bounded 45-second recovery wait. The shared **2 GiB runtime guard** remains intact.
Simulations run serially and completed steps persist across resource deferrals.
Each step records its actual available memory and admission floor in `starts.json`.
Quarry manifests still require at least 8 GiB; none is loaded by this batch.
Leases are capped at 15 minutes. Attempt releases do not end the coordinator's hold;
the unit writes an explicit `m7-mv1 FINAL release` after verification.

### Final integration verification

`integration-n-1` passed the complete batch in **209.123 seconds**, with no guard
failures. All eight processes ran serially inside the owned n8 lease. Actual
available memory at their starts was **9,613–10,395 MiB**. Its results supersede
the earlier `integration-i` harness stops (a missing manifest environment variable,
then reconstruction not explicitly enabled in the new static control).

- **192 lib tests passed**, plus `cargo fmt --all --check` and `git diff --check`.
- MQ1's unchanged moving-placement test passed both paths: moved card weight **16**,
  static ground weight **24**. Transform changes did not restart history.
- The immutable MQ1 resolve and merged resolve produced **byte-identical HDR** on
  each path, after 32 frames for each still/moving-camera case. Full hashes are in
  `static.log`; comparisons are within each path, not between RT and non-RT.
- Both MV1 motion tests passed. Maximum half-extent error was **0.00001296 px**
  (non-RT) and **0.00001151 px** (RT). First-frame velocity and cleared-background
  HDR trail error were exactly zero. All 6,187 non-RT and 6,189 RT covered movers
  continued history at frame 23 (maximum weight 16). Stopping left at most
  0.00000139 px velocity; diagnostic-view resumption reset TR1 and post motion.
- Both **48-frame, 960×540** example flights passed. All seven retained frames per
  path were inspected, along with the four-frame blur-off/on fixture sequences.
  Moving stripes have short motion-aligned blur without a retained silhouette;
  the flight retains the bobbing hero, rotating crate, shadows and scene geometry.
- SK1 passed both paths, including deformed-versus-baked maximum byte error **0**.
  The strict MV1 and SK1 validation guards reported **zero Vulkan errors** and
  confirmed the active Khronos layer. Both example logs contain no VUID messages.

MQ1's legacy `renderer(false)` startup, unchanged by this merge, prints
`VUID-StandaloneSpirv-None-10684` diagnostics with the externally forced validation
layer (Function `Offset` / Workgroup `ArrayStride` decorations). That test does not
install the strict diagnostic guard. These messages are retained in `mq1.log` and
reported to the coordinator; they are not presented as a clean-validation result.
The strict MV1/SK1 constructors and tests above are clean.

Inspected integration evidence:

- [Non-RT blur off](D:/Coding/renderlab-work/m7-mv1/integration-n/nonrt/inspect-fixture-off.png)
- [Non-RT blur on](D:/Coding/renderlab-work/m7-mv1/integration-n/nonrt/inspect-fixture-on.png)
- [RT blur off](D:/Coding/renderlab-work/m7-mv1/integration-n/rt/inspect-fixture-off.png)
- [RT blur on](D:/Coding/renderlab-work/m7-mv1/integration-n/rt/inspect-fixture-on.png)
- [Non-RT flight](D:/Coding/renderlab-work/m7-mv1/integration-n/example-nonrt/inspect.png)
- [RT flight](D:/Coding/renderlab-work/m7-mv1/integration-n/example-rt/inspect.png)
- [Motion data and timing samples](D:/Coding/renderlab-work/m7-mv1/integration-n/summary.json)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan; non-release Cargo test/dev profiles
(`renderlab` opt-level 1, configured dependencies opt-level 2), validation on.
Fixture settings match the earlier diagnostic protocol: 320×192, native TR1,
programmable stripes, no GI, neutral post, 180° shutter and 16 px maximum radius.
Medians of eight moving frames per cell, **diagnostic functional-lease timings**:

| Path / post | Renderer `render_span` ms | Post/present interval ms |
|---|---:|---:|
| Non-RT / blur off | 1.243 | 0.031 |
| Non-RT / blur on | 1.852 | 0.643 |
| RT / blur off | 0.713 | 0.033 |
| RT / blur on | 1.335 | 0.652 |

No quiet window or clock control was used. These are not an overhead comparison
against the earlier, differently merged/contended screen, and no speed superiority
is claimed. The real-quarry moving-caster screen remains unrun.

The follow-up began around 21:24 UTC. Fresh compilation/integration reached the
first ready binaries at 21:45; queue/memory admission delayed GPU work until 22:45.
The later R1n accounting fix compiled in 46.58 s (tests) / 18.31 s (example); the
static test setup correction took a 53.24 s test-only rebuild. Admitted integration
leases, including the resource deferral and harness stops, totaled **5.51 minutes**;
the final successful screen ended at 22:58:01 UTC. The exact
`m7-mv1 FINAL release` marker is written; no owned build, GPU job or waiter remains.

Verified test binary SHA-256:
`dffda2081897f419afc6a8d12d061265620cf398d1612770af2bdbcc7c358f32`.
Example binary SHA-256:
`462682be324c6541cb69333485fec95fb97d41f0353489e112eef6bbaf191eae`.
Production source is `c9ffda5`; `8bbbd36` adds only the static-control test setup fix.

**Fresh-target cleanup remains blocked.** The resolved target was verified as the
ordinary directory `D:/Coding/renderlab-work/m7-mv1/target` (not a reparse point).
Automatic execution approval again rejected the native `Remove-Item -LiteralPath
D:/Coding/renderlab-work/m7-mv1/target -Recurse -Force` request, stating only
"blocked by policy". No alternate deletion was attempted. The new target remains;
staged executables and inspected PNG/JSON/log evidence remain under the unit's D:
directory. No new raw image dump larger than 5 MB was produced.

## Gaps

- Real-quarry moving-caster images and timings did not complete: the original load
  was memory-aborted, and higher-headroom retries could not launch. This unit has
  not met the overnight quarry acceptance gate; run the retained full-scene batch
  when RAM headroom is available before calling it accepted on the quarry.
- Forward raster frames still use PP1's camera-only depth unprojection. MV1 covers
  the shipping traced and visibility-buffer paths; it adds no raster velocity pass.
- Transparency and particle-layer velocities are not represented by the opaque hit.
  Hidden surfaces and shutter visibility changes remain PP1's single-layer limits.
- Removing whole-scene transform resets exposes changing-lighting/material history
  problems to TR1's validation. MQ1's retained box clamp limits stale colours but
  cannot supply a newly disoccluded surface's missing lighting samples.
- Motion evidence is native-resolution. The same source projects in output-pixel
  units when upscaled, but Quality/Performance object-motion sweeps were not run.
- No ground-truth shutter, Unreal comparison, film-quality certification or speed
  superiority is claimed by this bounded overnight unit.
