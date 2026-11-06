# N1b: raster hit records and a complete frame without RT

2026-11-06, base `aee5007`. **Implemented and device-tested, but not accepted against the N1b cost/film gates.** Regular full-detail geometry now produces the primary 16-byte hit ABI by rasterization, and an entire quarry frame runs on a device created without either experimental ray feature. Quarry primaries cost about 2.6 ms, missing 0.40 ms; the complete no-RT frame costs about 4.7 ms, missing recorded Unreal VSM's 1.61 ms. Virtual shadows retain N8's quality limitations. This is an experimental fallback, not a shipping-quality or performance win.

## Implementation and selection

`PrimaryVisibility::VisibilityBuffer` selects the new producer. It uses `VISIBILITY_FEATURES` and `visibility_limits`: descriptor arrays, nonuniform indexing, and five resolve storage targets. `RECEIVER_FEATURES` remains a renderer requirement. On a 1x device with these capabilities and `VIRTUAL_FEATURES`, but without hardware-shadow options, construction selects VisibilityBuffer and `SoftShadows::Virtual`. Ordinary devices retain the existing forward-raster default; RT renderer defaults are unchanged.

The no-RT test requests **neither `EXPERIMENTAL_RAY_QUERY` nor `EXPERIMENTAL_RAY_HIT_VERTEX_RETURN`**, raises no AS limits, asserts both features are absent, and asserts there is no `HardwareState`. Its frames use the production lighting, environment, Virtual shadows, program shading and tone mapping. Shader tests parse and validate every no-RT composition and reject any traversal declaration/instruction. An RTX 4070 SUPER with these device features disabled proves this execution path; a physically non-RT GPU remains N3 work.

The producer:

1. Builds 128-triangle spatial clusters from Morton-sorted source triangle centroids. A separate source-ID indirection preserves every original primitive and vertex. No LOD or position quantization is used.
2. Projects conservative cluster bounds on the GPU and culls the frustum and previous HZB. Previous depth is used only for an unchanged camera and transforms; changed frames start with frustum culling alone.
3. Compacts visible clusters into one indirect hardware raster draw. The native rasterizer clips at the near plane and writes placement/triangle IDs plus full reversed-Z depth.
4. Builds a min-depth pyramid, retests previous-frame rejects against current-frame depth, and rasterizes recovered clusters in a second indirect draw. The final HZB is retained. Bounds use an absolute-product floating-point error bound, including the different matrix association orders in projection and vertex shading.
5. Reintersects the selected source triangle with the pixel-centre ray, reconstructs barycentrics and the point with the existing contraction order, and applies the production shared-edge ordering/fold inset. The ray construction matches traced primaries. Missing adjacency uses an exact-endpoint software search; proven open boundaries need no search.
6. Writes `(placement + 1, source primitive, f32 u, f32 v)`, receiver precision, coverage and the existing receiver attachments. The same deferred program API, texture gradients, receiver policy and N8 lookup consume them.

The frame remains programmable. No BRDF, sun-facing decision or lighting model was put in the hit ABI, cluster table or repair. Standard and toon programs, reload, instance movement and return were exercised. The inherited deferred-path restrictions remain: 1x regular opaque affine geometry, 64 draw slots and one shading program per frame. Virtual-geometry/diagnostic submissions retain forward rasterization. Virtual and RT Exact shadows are supported with this producer; Interactive/Temporal still require traced primaries and are rejected rather than silently running the wrong estimator.

Cluster metadata is prepared at upload for primary-eligible meshes, including RT renderers that may later select this mode. This adds CPU preparation/storage even while traced primaries are selected. The quarry indirection alone is 52,139,612 bytes on the GPU, alongside retained CPU cluster/ID vectors; it is not a startup or memory saving. The recorded load logs include this work, but its isolated development-latency cost has not been measured.

This follows the two-phase visibility-buffer design surveyed in [reuse-non-rt](research/reuse-non-rt.md). **No Bevy or other third-party source was copied**, and no private asset was imported into the repository. This checkpoint uses hardware rasterization and a flat spatial cluster list; Bevy's meshlet BVH and software/hardware microtriangle split are not implemented. Its software rasterizer's inclusive edge tests lack a top-left rule; adopting it would still require explicit tie classification and clipping. The current native rasterizer supplies both clipping and its top-left rule.

## Hit agreement and ties

1920x1080 quarry, 13,034,903 source triangles, registered pose 0, translated pose 1 and return. Results below compare no-RT raster primaries with traced primaries using Virtual shadows in both cases.

| Measure | Pose 0 | Pose 1 |
|---|---:|---:|
| Covered union | 346,520 | 345,884 |
| Placement/primitive agreement on that union | **99.396283%** | **99.409340%** |
| Coverage disagreements | 3 | 3 |
| Exact-position shared-edge winners | 1,997 | 1,954 |
| Shared-vertex winners | 43 | 51 |
| Other winners within 0.01 screen pixel of a source edge | 48 | 35 |
| Near-coincident instance winner | 1 | 0 |
| Remaining unclassified identity differences | 0 | 0 |
| Same-triangle world displacement p99 | 2.320 um | 2.327 um |
| Same-triangle world displacement maximum | 129.814 um | 39.689 um |

These are **identity and position comparisons, not byte equality of the whole record**. Full 16-byte covered-hit equality is **0%** on the quarry: independently reconstructed f32 barycentrics differ from the hardware intersection's arithmetic. The ABI is shared; numerical bit patterns are not. A requirement for byte-identical traced hit records is therefore unmet.

Native subpixel snapping/top-left coverage and opaque hardware traversal choose different facets at edges; the shared repair does not erase all raster coverage choices. Hardware-raster depth ties also need not follow hardware ray traversal order. The exception at pixel `(844,622)` names two instances whose reconstructed points are 10.57 um apart, within the sum of their existing 10 um receiver precision radii; their projected depths differ by about 12 f32 spacings. It is recorded separately, not called a shared edge.

Pixel `(1301,643)` is the one same-triangle result above 0.1 mm. Its absolute view/normal dot is 0.00281, amplifying small arithmetic differences into 129.8 um along the grazing facet. It remains a precision limitation, not a passed 0.1 mm maximum-position gate. This run does not establish which producer is nearer the exact intersection of the GPU's actual f32 ray. No comparator or reference was changed to hide it.

Pose-0/return no-RT hits, visibility counts and HDR are byte-identical. On S1/S2/S3, covered identity agreement is respectively 99.994-99.997%, 99.998-99.9996%, and 99.668%; all identity differences are classified. These selected motion/return poses are not a replay of the entire motion-film suite.

## GPU cost

Windows/Vulkan, RTX 4070 SUPER, NVIDIA 617.14, wgpu/naga 30.0.1; Cargo development/test profile, renderer opt-level 1, `-j 2`. GPU shaders are the production pipelines. Final batch disables Vulkan validation, uses 32 pipelined warm frames and eight measured frames per configuration. Each device loads each scene once. The final binary hash and source hashes are in the E:-local evidence manifest.

Median milliseconds at pose 0; all rows are 1920x1080, 1x with Virtual shadows:

| Scene | Traced primary control | No-RT raster primary | Complete no-RT frame |
|---|---:|---:|---:|
| Quarry, 13.03 M triangles | 0.302 | **2.528** | **4.718** |
| S1, contact/fold | 0.095 | 0.306 | 0.847 |
| S2, slivers | 0.081 | 0.301 | 0.792 |
| S3, dense slab/pillars | 0.106 | 0.401 | 1.169 |

Quarry medians over the original/translated/return poses are 2.528-2.605 ms primary and 4.696-4.727 ms complete frame. That frame includes culling, both rasters/HZBs, reconstruction/repair, N8 visibility, programmable shading and tone mapping. N8 itself costs about 1.71-1.80 ms here. Against the recorded matched-quarry VSM denominator of 1.61 ms, this is about **2.9x the cost**, not a win or the required 2x performance. The 0.40 ms primary budget is missed by about 6.3x. S3 is at the budget boundary in one pose and slower on return; no unconditional S3 budget pass is claimed.

The run used the shared GPU lock, with no game or foreign Renderlab/Unreal job. Unrelated wallpaper-project compilers and desktop GPU activity remained present. Strict no-compiler attempts were rejected/aborted and their samples were discarded. These are paired development measurements with recorded contention, not a final isolated performance certification. The traced quarry control reproduces its approximately 0.30 ms baseline.

There are 101,840 quarry clusters. Static pose 0 draws 60,397, still approximately 7.7 M triangles; the recovery pass draws zero in that static frame. Replacing source-order clusters with spatial clusters reduced the observed primary cost from about 5 ms to 2.8 ms; the error-scaled depth bound reduced the admitted set from 71,967 to 60,397 and the measured primary to about 2.6 ms. Those screens had different contention and do not establish isolated speedup ratios. Hardware microtriangle work and coarse cluster visibility remain unresolved cost levers. The RT path retains traced primaries.

## Quality against references

The current receiver's high-sample control is the mean of 64 independently seeded 256-ray Exact frames (16,384 rays/pixel), generated once and reused unchanged. On common same-triangle receivers:

| Quarry measure | Raster + Virtual | Traced + Virtual |
|---|---:|---:|
| Pose 0 partial visibility RMS | 0.297581 | 0.297514 |
| Pose 1 partial visibility RMS | 0.295199 | 0.295203 |
| Pose 0 solid flips | 6,418 | 6,419 |
| Pose 0 umbra leaks | 2,523 | 2,523 |

Full-image HDR raster-minus-traced RMS is 0.000564 / 0.000620 at poses 0/1. Against the archived native-resolution 1024-primary-sample x 32-shadow-ray film radiance PNGs, 8-bit code RMS is 2.57687 / 2.61562 for raster, versus 2.57133 / 2.61463 for traced. These published PNGs permit radiance-image comparison, not a fresh HDR Level-F certification; their original raw arrays have been deleted. The high-sample visibility rows also exclude classified identity changes, while full-image HDR/PNG comparisons include them.

**Film quality is not met.** N8's leaks and false shadows remain, and one same-triangle precision residual remains above 0.1 mm. N1b does not fix N8b's shadow work and none of its files were edited. Full quarry, +4-stop images, sparse amplified differences, the film image, S1 contacts/folds, S2's moved view and S3's slab/pillars were inspected. Geometry, silhouettes, rubble and crevices remain complete; visibility differences are sparse. This is not acceptance of complete motion, transparency or film-quality rendering.

## Validation, evidence and handoff

- Final `cargo test -p renderlab --lib`: **103 passed, 0 failed, 102 ignored, exit 0**. Shader/ABI checks and spatial-cluster partition/bounds tests are included.
- Formatting passes. Clippy passes with `-D warnings -A clippy::nonminimal_bool`; strict Clippy first reported the pre-existing boolean-style diagnostics at `virtual_shadow_gpu.rs:71`, which belongs to N8b and was left untouched.
- GPU fixture passes with validation active: a device without RT, Virtual shadows, Standard/toon programs, reload, moved instance and byte-identical return.
- The retained eight-step quarry/S1/S2/S3 batch passes by exit status, including separate devices without RT. It took 79.50 s including loading. No window, focus/input automation or desktop/game capture was used.
- Every run used `scripts/n1b-run.py` and `gpu-schedule.md`: 4 GiB start, 2 GiB runtime abort, owned descendants only. One early quarry load hit the memory abort. Other final-timing attempts stopped on foreign compiler contention; no foreign process was closed.
- Evidence: `E:/renderlab-work/n1b/retained-batch/` (summaries, counts, identity witnesses, PNGs, timings), `runs/` (lease/resource/exit logs), `evidence.json` (build/source identity and verification time), and `fixture-first/`, `final-checks/fixture/`. Raw arrays over 5 MB and this worker's E:-local Cargo target are removed after summarization; inspected PNGs and summaries remain.
- Runtime implementation/diagnosis ran approximately 09:15-10:02 UTC, interleaved with checks. Registered verification-process wall time totals **457.25 s CPU checks** and **331.75 s GPU batches**, including failed attempts; CPU scoring/inspection is additional. These overlapping windows are not isolated active-editing time and must not be added as sequential phases. Exact run intervals and identities are in `evidence.json`. No viewer startup or shader edit-to-image performance claim was made.

Reproduce with `scripts/n1b-check.py`, `n1b-run.py`, `n1b-batch.py` and `n1b-score.py`. The batch reads the existing local quarry/bench manifests; it never redistributes the source assets. A future acceptance needs faster microtriangle/cluster submission, the grazing precision witness resolved or explicitly accepted, fresh film/motion gates, and N8b's quality fixes. No shipping-floor or 2x claim is made by this checkpoint.
