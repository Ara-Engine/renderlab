# M7-AT2: continuous time of day and a filtered sun disk

AT2 builds on VF1 (merged before implementation at `10a40db`), on branch
`t3/m7-at2`. Overnight complete-base rules apply; no new Unreal or path-traced
reference campaign. The sky and VF1 volumetric haze remain on by default on
both paths; the existing sun disk now has analytic pixel coverage by default.

## Implementation

The procedural environment keeps its texture storage and kernel groups when
the sun direction, lux, medium or camera altitude changes. A typed input key
marks the pixels dirty. GPU writes run before camera/GI shading on the same
queue, so previous submissions finish reading the previous contents first.
Unchanged inputs run no environment convolution. A changed yaw still replaces
the map because its immutable source-to-world transform changes.

`set_environment(None, ..)` means automatic sky selection and retains an already
selected procedural sky. This matters for `Engine::set_sun`, which republishes
lighting, the sky selection and atmosphere together on every call. Turning the
atmosphere off releases that automatic map on the next admitted lit frame.
Captured maps still take priority and remain immutable.

The GI production change is one line in `src/gi_cache.rs`: include the
procedural `content_revision` in the existing environment key. There is no
change to cache storage, geometric visibility, tracing, scheduling or quality.
The existing eight-slot-group environment integration and bounce relighting
handle continuous changes. A test-only age accessor verifies the table is not
cleared. `lighting_gpu.rs` includes the revision in the film lighting hash too.

Correction to the old gap wording: in the VF1-merged source a new map requested
reintegration/relighting and lighting rebinding, but there was no explicit
GI-table clear on an environment-generation change. AT2 removes texture and
binding churn and verifies retained GI rather than claiming a measured prior
blackout that was not reproduced.

The sun uses chord distance (avoids `acos(dot)` cancellation at its small angular
radius) and explicit adjacent camera rays to find a pixel footprint on every
entry path. The rim is locally a straight line: coverage is the analytic area
under that line in a square pixel, the CDF of two centred uniform distributions.
The limb-darkened physical-radius disk remains. No derivatives, screen-space
lighting, material ABI changes or new patent-exposed reuse method are added.
GI continues using its existing fixed-rotation design-around; there is no
resampling, ranking or variance clipping. This does not change the existing
world-cache patent-policy record.

`examples/game_scene.rs` gains `--sun-speed DEGREES_PER_SECOND`,
`--sun-after FRAME`, `--still` and `--timing-frames COUNT`, so time of day can be exercised through the
engine-facing API without moving the camera or objects.

## Verification and measurements

**Passed:** 170 lib tests, zero failures, 152 ignored, inside n8 lease `c-1`.
The non-RT quarry completed in `c-1`; RT traced, RT raster and all four
`game_scene` runs completed in `d-1`. Production rendering sources did not
change between these runs. The non-RT directory was moved unchanged into
`D:/Coding/renderlab-work/at2/images/d/nonrt`; provenance and all numbers are
in `images/d/summary.json`, with build hashes in `build-identity-c.json` and
`build-identity.json`.

Two rejected attempts are kept in the logs. The first lib run found the
inherited duplicate reflection query; main already fixed it in `9f98869`,
which AT2 merged (base `431186a`). `c-1` then exposed a harness error: RT's
`since_clear` diagnostic starts at `u32::MAX` until a reset, so it saturated
and falsely failed the age assertion. The harness now resets once before
warm-up and asserts that GI actually ran. No GI production change was needed.
The passing non-RT evidence was reused rather than repeated.

**GI retention:** after 96 warm-up frames, a 32-frame sun turn at
0.05 degrees/frame retained the same map identity and generation (1) on all
three paths. Its content revision advanced once per changed frame. On RT
traced and actual non-RT, GI age advanced 96 to 128 with no clear. Whole-frame
mean exposed RGB changed smoothly from 0.6445 to 0.6428 (RT), and 0.6443 to
0.6426 (non-RT). Those means are supporting data; the age/identity assertions
and inspected images establish the absence of a flush/blackout. Raster still
has no GI; its sentinel age is excluded from this claim.

**Images inspected:** the whole quarry at rest and moving-sun frames 0, 15 and
31 on non-RT, RT traced and RT raster; `game_scene` still frame 96 and moving
frames 96, 112 and 143 on both paths; native-size quarry and game moving-frame
images; every disk crop at eight subpixel positions, with TR1 off. Silhouettes,
rock cavities, cast shadows, the arch, reflective spheres and sky stayed
continuous, with no observed blackout or new trail. Existing flat-ground GI
speckle and the RT/non-RT reflection differences remain; this is not a scored
lighting-quality or broad-motion certification. No glass entry was added.

![Whole quarry, still and moving sun on all paths](D:/Coding/renderlab-work/at2/images/d/quarry-sheet.png)

![Game scene, still and moving sun on both paths](D:/Coding/renderlab-work/at2/images/d/game-sheet.png)

**Disk:** true radius 0.00465 rad, 1080p, -28 stops, eight 0.01-degree steps.
The exposed-HDR 48x48 crops match bit for bit between RT and non-RT deferred
misses. Their summed energy varies by 0.410% across positions; raster varies
by 0.396%. The filtered rim moves smoothly in the inspected crops. Raster
and deferred rays still differ numerically: crop HDR RMS 0.00276-0.00346,
maximum 0.0977, with 97-104 pixels above a 0.001 HDR difference. This replaces
the old hard-rim result; it does not claim exact raster/deferred equality.
No high-sample curved-disk/limb reference was generated.

![Eight subpixel sun positions; nearest-neighbour magnification](D:/Coding/renderlab-work/at2/images/d/disk-sheet.png)

**Host/build:** Windows/Vulkan, RTX 4070 SUPER, NVIDIA driver 617.14,
Ryzen 7 7800X3D. Cargo release profile, two jobs, base `431186a` plus the AT2
source hashes in the build identities. Quarry: 13,034,903 source triangles,
1920x1080, physical procedural sky, 120 klux sun, default GI/TR1/volumetric
fog, 0.04 rad scene-shadow disk. The non-RT harness uses the registered
cluster-BVH's first level per mesh. Game: 1280x720, native TR1, -14.5 stops,
Realtime quality, default GI and volumetric fog, stationary camera/objects,
1.5 degrees/second sun starting after frame 96.

Wall throughput, one stationary/moving pair, 32 frames each:

| Workload/path | Stationary | Moving sun | Difference |
|---|---:|---:|---:|
| Quarry, non-RT VB + Virtual | 15.452 ms | 43.355 ms | +27.903 ms |
| Quarry, RT traced + Interactive | 5.948 ms | 10.157 ms | +4.209 ms |
| Quarry, RT raster + Exact | 18.666 ms | 18.538 ms | -0.128 ms |
| Game scene, non-RT | 6.016 ms | 6.592 ms | +0.575 ms |
| Game scene, RT traced | 3.055 ms | 3.577 ms | +0.523 ms |

These are **contended functional measurements**, not quiet performance claims.
Foreign rustc processes were present in every one of c-1's 744 resource samples
(maximum 4), and 822 of d-1's 872 samples (maximum 6). Vulkan validation was
enabled. GPU span samples are retained, but RT's spans exclude atmosphere
before primary visibility; they must not be used as whole-frame sky cost.
Game timing drains before and after its separate segment, includes the final
colour copy/readback and excludes PNG encoding. Median moving-sun setter cost
was 2.25 us non-RT and 2.05 us RT. All new measurement output is on D:.

**Performance miss:** this is not a cheap moving-sun quarry frame. The +4.21 ms
RT and +27.90 ms non-RT increments include existing GI relighting, gathers,
shadow updates and atmosphere work. No per-stage attribution or speedup over
the old reallocating implementation was measured. AT2 removes environment
allocation/rebinding churn and preserves GI; it does not solve that complete
frame cost. This branch predates main's later G2c/GC1 work. The small game
fixture's approximately +0.5-0.6 ms does not establish quarry scalability.
Quality was not reduced to improve these numbers.

![Diagnostic throughput and disk energy](D:/Coding/renderlab-work/at2/images/d/timing-chart.png)

**Validation miss:** every quarry/game GPU process emitted ten inherited
`VUID-StandaloneSpirv-None-10684` SPIR-V Function/Workgroup explicit-layout
diagnostics, as VF1 recorded. Lib composition tests emitted none. Rendering
and assertions pass, but this is not validation-clean. Both retained leases
record the same foreground window before/after; there were no window launches,
input automation or desktop captures.

Compilation: first clean lib/example 304.24/43.11 s; after the upstream query
fix 115.27/36.48 s; test-only age correction 115.40/3.15 s. Leased verification
was 7.03 s for the first lib failure, 95.04 s for c-1, and 111.08 s for d-1.
Implementation/integration was about 20 minutes of active work (estimate);
GPU queue waiting exceeded 50 minutes and is separate from execution.

The original VF1 runner adapter advertised a 20-minute lease and missed short
queue gaps. d-1 actually finished in 111 s and released at 20:47:14 UTC. Root
then required order-aware leases of at most 15 minutes. The retained runner now
rereads `[root order]` before every acquisition and subprocess step, requires
reviewed `gpu-after.json` release gates, and hard-caps n8 leases at 15 minutes.
No GPU driver remains queued. Subsequent work was CPU-only.

Reproduce with `py -3 -B scripts/m7-at2.py build`, then
`py -3 -B scripts/m7-at2.py gpu LABEL`, after reviewing the current root order
and recording its exact line and release gates in the ignored evidence root's
`gpu-after.json`. `AT2_LIB=0` and `AT2_PATHS=traced,raster` select a focused
continuation; the default runs the complete batch. Actual tests/renders require
AT2's owned n8 lease even when invoking `batch` directly.

The disposable target was deleted at completion. No raw HDR dumps were written;
only the measured small disk crops are retained in JSON. Uninspected intermediate
PNGs were removed; summaries, logs, build identities, staged binaries and the
inspected PNGs remain on D:.

## Remaining scope

- Quarry moving-sun cost above is a miss. GI scheduling/geometry and shadow
  optimization remain with GC1/G2c and the shadow worker, respectively.
- Continuous changes relight GI progressively. Fast day cycles can lag the sky
  by the existing eight-group/gather schedule; no extreme day/night campaign.
- The disk uses a locally straight rim and centre-sampled limb intensity.
  Subpixel custom radii, HDR-ceiling exposures, horizon crossings, reduced
  reconstruction scales and broad camera motion remain unscored. Small
  raster/deferred numeric differences remain as quantified above.
- A second moon and transparent-entry atmosphere integration were not attempted
  while the lighting/glass units owned those files. Other Atmosphere gaps remain
  in [m7-gaps.md](m7-gaps.md), including fog history resets and validation errors.
- No matched Unreal or participating-media path-traced comparison was run.
  No Unreal-equivalence or 2x performance claim is made.
