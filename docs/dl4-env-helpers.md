# DL4: environment helper compilation

Base `2b931d3`, branch `t3/dl4-env-helpers`, 2026-11-07. Retained variant **D**.

Optimized native compilation falls to **318.355 ms graphics / 375.444 ms traced
compute** in the two-repeat screen. Output is byte-identical on both quarry poses
and all three configurations, and on all 43 image-fixture readbacks. Paired runtime
changes stay within observed variation. **The fresh-preview target is still missed:**
353.822 ms standard / 363.307 ms user. Preview is above DL3's historical measurements;
this unit establishes no preview-latency improvement.

## Retained implementation

Only environment lookup helper bodies in `shaders/scene.wgsl` change:

- Choose mip and bank before the texture branch, sharing legacy and independent
  first-bank filtering. Two bilinear sites remain because the textures are separate bindings.
- Filter the two roughness levels through one loop/call site, including legacy LOD.
  Preserve the fully rough unblended return and all interpolation arithmetic.
- Share the three corner-remap samples in a loop, preserving their summation order.
  Ordinary taps return directly through the original remap helper.
- Retain the four original bilinear tap expressions. Looping those taps was slower
  on the quarry. Retain original floating-point face selection/direction expressions.

The syntactic expansion under `environment_radiance` falls from **96 to 16 remap
sites**: one level call ? two texture branches ? four bilinear calls ? two remap
sites. This counts all branches, not executed samples or final ISA. The first
corner value initializes the sum instead of adding zero. The only new arrays are
local two-element arrays; there are no dynamic vector-component stores or arrays
passed by value.

Helper signatures, the standard user program, material/shading contracts, shadow
and primary-ray files, pipeline/reload code and `milestones-5-6-decisions.tsv` are
unchanged. There is no added restriction on programmable shading.

## Compiler measurements

Unmodified DL2 tests `gpu_vulkan_compile_screen` and `gpu_vulkan_helper_size`,
Vulkan interop, fresh unique sources, application pipeline caches disabled. Each
cell has two samples; native duration surrounds the driver call. The initial
baseline is reused from this unit's first screen, rather than a new interleaved
baseline campaign. Baseline and retained screens record no foreign compiler activity.
These are compiler screens, distinct from the five-repeat viewer endpoints below.

Driver milliseconds, median (range):

| Pipeline / policy | Baseline | D |
|---|---:|---:|
| Graphics optimized | 1,200.580 (1,192.738?1,208.423) | 318.355 (313.441?323.269) |
| Traced compute optimized | 1,313.174 (1,243.984?1,382.363) | 375.444 (361.939?388.950) |
| Graphics helper preview | 290.879 (289.072?292.685) | 235.769 (232.787?238.752) |
| Traced compute helper preview | 407.199 (397.841?416.557) | 328.551 (311.719?345.383) |

Optimized helper-size medians in ms; distinct calls vary direction and roughness:

| User reflection calls | Baseline graphics | D graphics | Baseline compute | D compute |
|---:|---:|---:|---:|---:|
| 0 | 26.128 | 18.645 | 50.503 | 42.282 |
| 1 | 735.378 | 134.960 | 893.867 | 253.036 |
| 2 | 1,631.757 | 253.077 | 2,101.128 | 296.473 |
| 4 | 5,227.828 | 789.478 | 5,723.930 | 893.411 |

DL3's unmodified `gpu_vulkan_helper_boundaries` also runs on D. Two-repeat native
medians, graphics / compute: radiance boundary alone **1,378.113 / 1,428.530 ms**;
bilinear alone **535.258 / 606.520**; tap alone **359.727 / 396.743**; bilinear plus
radiance **569.015 / 590.457**; the environment set **269.312 / 336.107**. DL3's
historical radiance-only graphics result was about 8.4 s; it is not a paired control
for this screen. All raw boundary choices remain in `screen-candidate-d/`.
No compiler policy is changed here.

## Exact images and runtime

Full-resident quarry, 13,034,903 triangles, 1920?1080, 1? MSAA, 16? anisotropy,
registered camera poses 0 and 1, original lighting and `sun128.rle2`. Configurations:
raster primary/Virtual shadows (the software path), traced primary/Interactive
shadows, and traced primary/Virtual shadows. The quarry harness uses an RT-capable
device for switching configurations; the software configuration uses no traced
primary or hardware shadow estimator. Featureless-device image fixtures run separately.

A disposable E: source copy adds one measurement test alongside the reload tests;
no repository test files are edited. It uploads the quarry once, drains and reloads
the exact baseline/D WGSL before each visit, warms four frames, then samples eight
GPU frames. Eight paired visits alternate baseline/candidate order. All **96 full
RGBA16F hash comparisons** match, with 64 timing frames per variant/configuration/pose.
Two earlier validated visits also match all six identities. Final timing visits
have validation disabled; image fixtures and preview checks enable validation.

Complete HDR SHA-256, shared by baseline and D:

| Configuration / pose | SHA-256 |
|---|---|
| no-rt-0 | `fa1384a4d26c39fe5f0e9849d7c597db6dffd226f4929d2742e5009cfa06ece5` |
| no-rt-1 | `4c8e58c3053d54cca50b9ae2644d5a3b7d3f6171669948a8d142843619ef040b` |
| rt-0 | `91f355f82393c43762750db808e5859d34e4a5b2b44aae295afb36c1779d55e7` |
| rt-1 | `e504718739744f4e08b7fc63acdbe4f0a1925e1c06705ab25625192a9c9a1017` |
| traced-virtual-0 | `83e2d493cc4355391ac6f779a033b4699a4f047051ba0cc967a909da08915074` |
| traced-virtual-1 | `b1c45eaf5e02fd9d02b64c164c3ddf92a59e4b4c4f898df503948bf1ab3bc17e` |

Seven environment image tests pass, including RLE1 axes/yaw/poles/seams/mips,
independent RLE2 banks, the 538-reference reconstruction and 289-direction sun,
extreme-energy photometry, and normal/shadow regular?virtual images at 1?/4?.
All **43 captures / 1,889,608 bytes** have identical lengths and SHA-256 values.
The identical baseline/D capture manifest SHA-256 is
`9508879c59ea9e060f01f875a99b62159949eecbfda51d1439d0b691f281e18a`.
Per-capture hashes are in `fixture-baseline-hashes.json` and
`fixture-candidate-hashes.json` outside Git.

GPU mesh/shading-stage cost: median of the eight **paired visit differences**,
D minus baseline. Each visit first takes the median of its eight frames. Readback
and shader compilation are outside the GPU shading interval.

| Configuration / pose | Paired delta, ms | Paired change |
|---|---:|---:|
| no-rt / 0 | -0.00932 | -0.285% |
| no-rt / 1 | -0.00822 | -0.252% |
| rt / 0 | -0.01287 | -2.190% |
| rt / 1 | +0.00449 | +0.135% |
| traced-virtual / 0 | -0.00256 | -0.149% |
| traced-virtual / 1 | -0.07853 | -2.076% |

No-RT shading medians are 3.270/3.266 ms baseline and 3.260/3.257 ms D, poses 0/1.
RT visits vary substantially, roughly 0.34?3.7 ms, with isolated pair transitions
exceeding 100%. Independently taking the two unpaired medians is misleading for
that distribution. The paired medians show no regression beyond this noise; **no
RT frame-time speedup or fixed-clock equivalence bound is claimed**. Every visit,
raw timestamp and pair difference remains in `pair-retained/` and `paired-summary.json`.

## Fresh edit to image

DL3's method: actual inactive development viewer, same full quarry and settings,
five unique standard and five unique user edits, each followed by restore, invalid
save and recovery. Default helper preview, no policy command. All forty actions
pass: edits change the complete HDR hash, invalid saves retain the last working
image, and restores/recoveries recover the exact baseline. Cached restores are not
fresh-edit evidence. The 100 ms observer records **0/994 compiler-busy samples**;
the lease guard records **0/35 foreign-build samples**. No game/foreign renderer is
detected; ordinary desktop activity remains.

Milliseconds, median (range), five fresh edits each:

| Program | Save to preview GPU completion | Save to preview present call | Save to optimized GPU completion |
|---|---:|---:|---:|
| standard | 353.822 (348.240?604.547) | 342.514 (331.850?595.602) | 805.979 (795.957?1054.240) |
| user | 363.307 (352.716?368.441) | 347.945 (334.225?351.087) | 810.355 (799.875?837.564) |

Preview misses 100 ms by **253.822 / 263.307 ms**. DL3's historical completed-preview
medians were 293.791 / 319.932 ms; D is higher, without a same-session paired viewer
baseline to attribute the difference causally. Optimized completion is below DL3's
historical 1,379.408 / 1,426.534 ms. These historical comparisons are not a fresh
paired causal speedup measurement. Present is a present-call proxy, not scanout;
GPU completion is the viewer's first observed completed revision. PNG/readback work
happens after the timed endpoints.

Actual viewer hot-pair creation: preview **301.450 / 307.037 ms**, optimized
**400.214 / 411.992 ms**. Preview module/reflection preparation is
**13.431 / 11.659 ms**, optimized **6.899 / 5.822 ms**. Component medians do not add.
This is why the smaller sequential native calls cannot be substituted for the
viewer result. Further preview-policy work remains outside this helper-only unit.

The untimed quality probe captures the **actual first-preview revision**, then
compares it with optimized promotion and the exact-source ordinary reference.
Standard and custom user previews and replacements have **0/2,073,600 differing HDR
pixels, maximum/mean absolute error 0**. Complete preview PNGs and the six-pose/path
contact sheet were inspected, including silhouettes and shadowed regions. Original
viewer baseline hashes remain:

- Standard: `1c4462b43466188d58f0e6e47ef03060e8adb64a1c0e8f3aff88e470172c12f1`.
- User: `10ee30f19e0f50ecde34c3cca32c3bc806e539aa9eba73f2d0cd1c259ae33690`.

## Rejected work and setup limits

A loops all taps: native graphics 191?244 ms and all 43 fixtures/96 quarry hashes
match, but its repeated no-RT shading is about 14% slower. Rejected. C restores the
ordinary-tap fast path but retains looped bilinear sampling; the short screen is
still about 4?8% slower on no-RT. D restores the four original bilinear expressions.
Short rejection screens precede the broader retained checks.

B additionally uses constant axis/sign tables for face mapping. A CPU f32 replay
matches 65,536 directions, but the first GPU quarry comparison changes no-RT pose
0 from `fa1384a4d26c39fe5f0e9849d7c597db6dffd226f4929d2742e5009cfa06ece5`
to `4e4e1ea7916ac626f2d236a359f8b7a236ad8d80da24938ee95c7c4fe6a50db8`.
Rejected immediately. Its optimized driver ranges, 188?246 ms graphics and 282?289 ms
compute, do not justify relaxing exact identity. No face-table change ships.

The first fixture setup selected a smaller preparation artifact instead of the
registered `m5-environment-runtime/sun64.rle`; that failed on the unchanged baseline
and was corrected. The unchanged resource-lifetime test also fails its timing-sensitive
`no observed incomplete prior environment uses` assertion. It is recorded as a
baseline miss, excluded from the seven image fixtures, and is not weakened or edited.
No resource-lifetime fix is claimed.

One final diagnostic invocation selected zero custom runtime tests: Cargo reused a
static D library binary across the disposable source copy. Its native/fixture data
is valid, but that invocation supplies no runtime evidence. The retained 96-row
runtime run instead uses the saved custom harness binary with baseline/D files
loaded explicitly by `reload`; it asserts every reload succeeds. Static D compiler
and fixture binaries are separately confirmed to embed the exact D source.

## Checks, identity and resource accounting

`cargo test -p renderlab --lib`: **123 passed, 122 ignored, exit 0** on final D.
`cargo fmt --all -- --check` and `git diff --check` pass. Source/body SHA-256:
`bb75cf5eab677461edd894ec35a6624e7e55ca1f548b239b9cf1d0608b2359d1`.
Measured viewer SHA-256:
`3e487ac5c2be8c85ef99464d26908d351cd07f50120aa3c75491702580ceb252`.

Host: Windows 11, RTX 4070 SUPER/NVIDIA 617.14, Ryzen 7 7800X3D, Rust 1.96,
wgpu 30.0.1, development profile. This is one driver/workload, not all user shaders
or vendor certification. No Unreal, startup or Rust-edit latency claim is made.

Every GPU/window run uses `D:/Coding/renderlab-work/gpu-slot.lock`, reservation and
release lines in `E:/renderlab-work/gpu-schedule.md`, game checks, at least 4 GiB
available RAM at launch and a 2 GiB runtime abort floor. No Cargo runs in an owned
GPU lease. Game hold remains released throughout. CPU preparation stops when another
GPU lease appears; one short build abort is caused by that guard, despite its old
failure string saying a game appeared. A 3,821 MiB preflight starts no renderer.
Foreign-build preflights also defer without starting GPU work. Foreign processes
are not paused or terminated. Owned viewers start inactive and close; foreground
handle/PID match at the final lease endpoints. No focus/input automation or desktop
capture is used.

Implementation/analysis time was not independently stopwatched and interleaves with
shared queue waits. **Recorded CPU preparation/checks: 376.761 s**, including initial
dependency compilation and the aborted preparation. Final library/fmt/viewer/diagnostic
preparation takes **76.679 s**. **Owned GPU lease holds: 631.934 s**, including driver
CPU work, initialization, readbacks, failed setup and deferred preflights. Final
compiler/fixtures take **64.477 s**, the retained runtime **69.648 s**, and the final
viewer timing/quality lease **147.912 s**. These verification intervals are not GPU
execution time. Final minimum available RAM is 14,582 / 10,969 / 11,207 MiB for those
three leases; each records zero foreign compiler samples. Shared-resource waiting
is outside those active intervals.

Evidence: **E:/renderlab-work/dl4/**, including `screen-baseline/`,
`screen-candidate-d/`, `pair-retained/`, `paired-summary.json`, fixture manifests,
`window-final/results.json`, `final-summary.json`, `quality-final/quality.json`,
source/binary identities and all guarded lease/check logs. Large raw readbacks,
uninspected PNGs, compiled binary copies and the owned Cargo target are removed
after summarization. Compact evidence, measurement source/scripts and inspected PNGs
remain. All owned GPU/window/build work is finished before handoff.
