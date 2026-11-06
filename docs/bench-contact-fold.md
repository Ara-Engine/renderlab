# S1: contact, fold and stacked occluders

2026-11-06. Static competitor fixture from [competitor weaknesses](research/competitor-weaknesses.md#first-three-scenes).
Based on `d262e4651610f297d523d873981821fd915f3a45`; branch `t3/contact-fold-bench`.
Only generated scene code, test scripts and this report are public. The private UE project
and all measurements stay outside Git under `E:/renderlab-work/contact-fold/`.

## Scene and registered measurements

The shared generator authors **38 triangles**: a 2 x 2 m two-triangle floor; nine 1 m
strips spanning all combinations of 1/5/20 mm widths and 0.2/1/5 mm gaps; tilted
20 x 20 cm plates at nominal 2/10 cm heights overlapping in sun projection; a
60-degree concave crease, its convex control, and an open double-sided three-face
shell. Separate labeled faces/lanes retain flat geometric normals. No textures,
normal maps, GI or scene reflections. Grey base color 0.5 linear, roughness 0.6,
metallic zero. All faces are double-sided; strips, plates and shell declare thin
shadow receivers in Renderlab. Fold/control/floor retain opaque viewed-side receivers.
UE's native receiver policy is retained and is not identical to that declaration.

Metres, Y up. Camera `(1.8,1.2,2.4)` targets `(0,0.05,0)`, vertical FOV 45 degrees,
1920 x 1080. Pose 1 translates eye and target 0.5 m backward along the view direction.
The generator also records the complete 224-frame, 60 Hz hold/dolly/hold/return/hold
track; **only the two static distances are measured here**, not contiguous motion.
Sun elevation 20 degrees, azimuth 35 degrees measured from +Z toward +X, angular
radius 0.04 rad, white 1000 lux. UE diameter is 4.583662361 degrees. Exposure is
2^-8. Both cameras run first with black world, then constant environment radiance 1.

One JSON mesh/camera/light manifest drives both engines. As in the retained quarry
transport, glTF bakes `(-Z,Y,X)` before the importer maps to UE `(-Z,X,Y)` in cm.
The private map is `/Game/BenchContactFold/Bench` in
`D:/Coding/renderlab-work/ue-m5/M5Calibration.uproject`. These tiny meshes use
ordinary full-detail static meshes with Nanite disabled. Preparation verifies all
38 imported triangles and all 19 lane bounds; maximum bounds error is
0.00000458 cm. No quarry asset or existing quarry map is modified.

Population/defect definitions were recorded in `scene.json` before candidate rendering:

- Populations are the unchanged `fresh_level_b_metrics.populations` construction
  from film visibility, with physical view depth: partial, lit, umbra, narrow,
  faint, isolated, silhouette, wide, wide6, contact and solid.
- Contact loss: reference contact-band pixel with `candidate - film > 0.5`.
  The denominator is explicitly retained; it is not all shadowed pixels.
- Leak: reference umbra with `candidate - film > 0.1`.
- Solid flip: reference lit with error below -0.5, or reference umbra above +0.5.
  Separately count all solid errors above the frozen **0.002** maximum gate.
- Report each population's RMS, signed mean, p99 and max, missing receivers,
  missed/false shadows, and 9-pixel contour error. Common-domain scores accompany
  native availability scores. Missing pixels never become lit values or zero error.
- Final RGB reports include linear HDR RMS and common Reinhard/sRGB display error
  at registered exposure and +4 stops, including background and silhouettes.

Near/far reference populations are **16,132 / 12,246 partials**, **419 / 335 contacts**,
and **274,108 / 194,077 solids**. No covered reference pixel lacks accepted samples.

## Film reference and independent check

The existing F0/R1 generator is reused: 256 stratified jittered primary samples per
pixel, each with 256 independently salted sun directions, with full radiance
accumulation including background/silhouettes. Visibility is conditioned on the
center placement and its 2%-depth neighborhood, as in the existing generator.
This is not the 256-direction pixel-center image. Odd/even split halves estimate
reference uncertainty: partial visibility sigma RMS is **0.003241 / 0.003300**.

An independent CPU integrator uses double-precision Moller-Trumbore primary and
shadow intersections and an independent 1,024-point scrambled Sobol source disk.
At twelve film-partial witnesses per camera, maximum visibility disagreement is
**0.000916 / 0.000851**. All witnesses pass the recorded conservative uncertainty
bound. `oracle.json` retains every coordinate, estimate, accepted count and bound.
This validates sampled visibility, not a full-frame independent radiance/path-tracer
certificate. The CPU zero-origin-offset diagnostic differs from the registered
10-um policy by up to **0.01622 / 0.00334** at these witnesses; the policy-conditioned
reference must not be described as physically unbiased for every contact.

## Results

Raw visibility errors against the footprint film reference. Contact loss is a count/denominator; leaks use >0.1 visibility, solid flips >0.5. GPU time is a **contended diagnostic median [min,max] in ms**, with 8 Renderlab frames versus 64 UE frames per configuration. These timings do not establish a speed ratio.

**Near camera · black world**

| Mode | Contact loss | Leaks | Partial RMS | Solid flips | Missing | GPU ms [min,max] |
|---|---:|---:|---:|---:|---:|---:|
| RL hard | 1/419 | 0 | 0.243140 | 0 | 0 | 5.427 [2.301,14.387] |
| RL Exact | 0/419 | 0 | 0.051992 | 0 | 0 | 14.086 [4.450,17.985] |
| RL Interactive | 0/419 | 0 | 0.087840 | 0 | 0 | 1.544 [1.538,2.294] |
| RL Temporal | 0/419 | 0 | 0.005975 | 0 | 0 | 0.424 [0.423,0.424] |
| UE VSM | 5/419 | 0 | 0.164184 | 0 | 1 | 0.768 [0.688,1.156] |
| UE HRT1 | 34/419 | 431 | 0.165304 | 16 | 1 | 1.006 [0.929,1.415] |
| UE HRT256 | 0/419 | 0 | 0.084120 | 4 | 1 | 8.959 [8.873,11.796] |

**Near camera · constant environment**

| Mode | Contact loss | Leaks | Partial RMS | Solid flips | Missing | GPU ms [min,max] |
|---|---:|---:|---:|---:|---:|---:|
| RL hard | 1/419 | 0 | 0.243140 | 0 | 0 | 12.420 [2.409,18.624] |
| RL Exact | 0/419 | 0 | 0.051992 | 0 | 0 | 10.757 [9.974,15.513] |
| RL Interactive | 0/419 | 0 | 0.087840 | 0 | 0 | 1.633 [1.608,5.196] |
| RL Temporal | 0/419 | 0 | 0.005975 | 0 | 0 | 0.421 [0.420,0.431] |
| UE VSM | 6/419 | 0 | 0.164364 | 1 | 1 | 0.760 [0.699,1.200] |
| UE HRT1 | 29/419 | 448 | 0.161410 | 15 | 1 | 1.028 [0.928,1.683] |
| UE HRT256 | 0/419 | 0 | 0.084215 | 4 | 1 | 8.946 [8.876,12.069] |

**Far camera · black world**

| Mode | Contact loss | Leaks | Partial RMS | Solid flips | Missing | GPU ms [min,max] |
|---|---:|---:|---:|---:|---:|---:|
| RL hard | 1/335 | 0 | 0.239744 | 0 | 0 | 4.967 [1.563,11.469] |
| RL Exact | 0/335 | 0 | 0.052182 | 0 | 0 | 9.851 [9.267,14.329] |
| RL Interactive | 0/335 | 0 | 0.086842 | 0 | 0 | 1.352 [1.347,6.071] |
| RL Temporal | 0/335 | 0 | 0.005968 | 0 | 0 | 0.339 [0.339,0.340] |
| UE VSM | 2/335 | 0 | 0.162777 | 0 | 0 | 0.743 [0.672,1.216] |
| UE HRT1 | 26/335 | 348 | 0.177903 | 6 | 0 | 0.954 [0.881,1.684] |
| UE HRT256 | 0/335 | 0 | 0.086285 | 1 | 0 | 6.642 [6.565,8.274] |

**Far camera · constant environment**

| Mode | Contact loss | Leaks | Partial RMS | Solid flips | Missing | GPU ms [min,max] |
|---|---:|---:|---:|---:|---:|---:|
| RL hard | 1/335 | 0 | 0.239744 | 0 | 0 | 1.885 [1.816,7.310] |
| RL Exact | 0/335 | 0 | 0.052182 | 0 | 0 | 12.494 [11.872,16.275] |
| RL Interactive | 0/335 | 0 | 0.086842 | 0 | 0 | 1.452 [1.445,4.150] |
| RL Temporal | 0/335 | 0 | 0.005968 | 0 | 0 | 0.758 [0.743,1.215] |
| UE VSM | 3/335 | 0 | 0.161977 | 0 | 0 | 0.744 [0.684,1.398] |
| UE HRT1 | 26/335 | 352 | 0.175632 | 6 | 0 | 0.968 [0.882,1.778] |
| UE HRT256 | 0/335 | 0 | 0.086263 | 1 | 0 | 6.655 [6.590,8.542] |

All source partial receivers are available in UE at both distances; near UE lacks one other covered receiver, so its complete covered-domain gate remains undefined. Common-domain scores are preserved alongside these values in `scores.json`.

**Whole-image radiance, black world; RMS at registered exposure**

| Mode | Near HDR | Near display codes | Far HDR | Far display codes |
|---|---:|---:|---:|---:|
| RL hard | 0.005791 | 3.887 | 0.005069 | 3.356 |
| RL Exact | 0.003204 | 1.898 | 0.002925 | 1.718 |
| RL Interactive | 0.003513 | 2.084 | 0.003176 | 1.855 |
| RL Temporal | 0.000161 | 0.107 | 0.000144 | 0.092 |
| UE VSM | 0.004533 | 3.004 | 0.003998 | 2.607 |
| UE HRT1 | 0.004606 | 2.735 | 0.004280 | 2.475 |
| UE HRT256 | 0.003493 | 2.030 | 0.003172 | 1.825 |

The environment rows reveal a **remaining capture mismatch**: UE's constant skylight illuminates geometry but its visible background stays black, while Renderlab displays constant environment radiance. The unchanged whole-image UE display RMS therefore rises to 12.22–12.58 codes. No background is fitted, replaced or excluded. These final-radiance rows are an explicit miss, not a fair isolated BRDF/engine ranking. Full HDR, registered/+4 display statistics and every population are retained in `scores.json`; `comparison.csv` includes frame times.

**Verdict:** S1 reproduces shortened/missing VSM strip shadows and low-spp HRT leakage/noise; 256 spp recovers contacts but retains receiver/aliasing errors and flips. Static Renderlab Temporal has the lowest partial error here, while its boundary-conditioned solid gate fails. AA-off controls, the unmatched environment background, contended timings, missing motion evidence and missing software execution prevent film-quality/2x acceptance or an engine-wide superiority claim.

Static Temporal reduces partial error substantially but fails the frozen full-population solid
maximum: **29 / 28** solid pixels exceed 0.002, with maximum errors
**0.06607 / 0.04706**. Zero flips does not imply exact solids or film acceptance.
All these failures have fewer than 256 accepted reference primaries: mixed-surface
footprints. On solids with all 256 primaries accepted, maximum Temporal error is
0.000977 at either distance, below 0.002. The independent CPU audit confirms fully
lit reference visibility at the two largest failures per pose (including the
zero-offset diagnostic); coordinates and accepted counts are in `oracle-solids.json`
and `solid-witnesses.json`. This separates a boundary/receiver-conditioned failure
from a claim of arbitrary interior light leakage. No mode is promoted, and no
quality gate is loosened.

## Capture and timing limits

Renderlab uses one renderer/device and one scene upload for both cameras, lighting
setups, four production modes and all film references. Temporal captures ages
0/1/3/7/31/127; the summary uses 128 static frames. Its visibility diagnostic
integrates Temporal's own samples conditioned on the same center surface; it does
not replace those rays with Exact. Eight production frames per configuration are
timed after warmup, excluding captures and diagnostic passes. Timings are direct
first-rendered-pass through final-tone-pass GPU spans, not summed subtotals.

Unreal uses NullRHI preparation followed by an offscreen editor-game batch with
resume support. **The requested single Unreal session was not achieved:** a
callback/CSV-path failure and two enforced RAM-floor stops required resumes.
The retained matrix contains five completed configurations from `unreal-batch-3`
and seven from `unreal-batch-4`; completed cases were not rerun after the second
memory stop. Earlier/full-tree attempts are superseded. Native VSM uses default receiver/SMRT
policies with 8 rays x 8 samples and adaptive rays; HRT uses 1 spp with native
denoising/temporal accumulation, or diagnostic 256 spp with denoising disabled.
AA/reconstruction are disabled to isolate these shadow policies. The native
deferred-light attenuation mask decodes `R^2 * B^2`, resolving the resource's last
writer before its consumer, as in [native shadow masks](unreal-native-shadow-mask.md).
The diagnostic HRT-256 result is not the film reference.

This session must initialize RT support to switch modes. VSM uses no RT shadows;
it is not a demonstration on an RT-feature-disabled device. Unreal's editor-game
cost is not a newly cooked packaged-game measurement. Final linear RGB is taken
before native tonemapping with explicit pre-exposure 1, then scaled by 2^-8 for
the common display/error transform; native main screenshots are also retained.
Compact diagnostic dumps select `Lights` for the native mask and `Tonemap*` for
the actual HDR input. That input is native `PF_FloatR11G11B10`, decoded explicitly;
it includes native intermediate quantization. Main-view camera cache, effective
CVars and actual HRT shader radius/SPP are checked, not inferred from the recipe.

Hardware: Ryzen 7 7800X3D, RTX 4070 SUPER 12 GB, NVIDIA 617.14; UE
5.7.4 CL51494982, D3D12 SM6. Renderlab is Vulkan/wgpu 30.0.1, a development/test
binary with active validation-layer environment. Renderlab's CPU timings are
development-build service/admission spans, not release CPU performance or a
dependency-aware CPU critical path. The binary/source/scene identities and per-run
resource context are retained in JSON.

**Timing is diagnostic, not a 2x acceptance claim.** Renderlab's batch began with
no compiler or known game/renderer in preflight, but live samples subsequently
recorded up to 16 rustc processes and large GPU-span variance. No quiet paired
performance campaign is substituted for these results. Full film radiance parity,
motion gates and no-RT-device measurements remain separate acceptance work.

## Inspection, non-RT applicability and reproduction

Inspected the two-camera black/environment lane sheets, whole-image registered/+4
radiance panels, visibility/error-x8 sheets, and native-main-view montage sheets.
All five lane columns were checked. Hard shadows erase soft structure;
Exact/Interactive alias faint strip contacts; static Temporal restores much of that
structure but its solid-gate failure remains reportable. VSM removes most faint
strip detail, shrinks the plate penumbra and hardens the fold/shell edges; HRT 1 spp
retains patterned noise and leakage. HRT 256 smooths penumbrae but keeps pixel-center
aliasing and isolated false shadows. Crop sheets preserve nearest-neighbor detail.

Private inspection artifacts: `images/p{0,1}-e{0,1}-crops.png`, matching
`*-visibility.png` and `*-radiance.png`, and `native-main-p{0,1}.png`.
`comparison.html` provides a switchable chart and crop sheets.

The geometry, camera/light manifests, film targets, population definitions and
Unreal VSM witnesses apply unchanged to a future non-RT path. None of the measured
Renderlab hardware-ray times or quality results automatically transfers to it.
Run the same fixture/estimator/program on a device created without RT features and
repeat every gate; until then the software result is **missing**, not a win.

```powershell
py -3 -B scripts/bench-contact-fold-scene.py --output E:/renderlab-work/contact-fold/fixture
py -3 -B scripts/bench-contact-fold-test.py
# Use bench-contact-fold-run.py for each owned build/GPU lease, with -j 2 and the
# per-worker E:/renderlab-work/contact-fold/target. Guarded commands are in runs/*.json.
# Renderlab manifest: {"scene":".../fixture/scene.json","output":".../renderlab"}
# Ignored test filter: gpu_contact_fold_batch
# UE phase environment: RENDERLAB_S1_ROOT and RENDERLAB_S1_PHASE=prepare or render.
# prepare: UnrealEditor-Cmd -run=pythonscript -script=.../bench-contact-fold-ue.py -NullRHI
# render: same executable/map with -game -EnablePython -ExecCmds="py .../bench-contact-fold-ue.py"
# plus -dx12 -sm6 -RenderOffscreen -ResX=1920 -ResY=1080 -ForceRes -csvGpuStats.
py -3 -B scripts/bench-contact-fold-score.py --root E:/renderlab-work/contact-fold
py -3 -B scripts/bench-contact-fold-oracle.py --root E:/renderlab-work/contact-fold
```

The shared atomic lock is `D:/Coding/renderlab-work/gpu-slot.lock`; reservations
and releases are appended to `E:/renderlab-work/gpu-schedule.md`. All launches
preflight known games/renderers and at least 4 GiB available RAM; a live 10 Hz
sampler aborts below 2 GiB. Only owned process trees are terminated. Engine helper
processes are classified separately from games. No desktop/game capture, input
automation or focus activation is used. Read-only foreground endpoints accompany
the Unreal records; they are not proof of uninterrupted foreground state.

Preparation resumed from saved assets after the installed engine's known
post-save Slate assertion on commandlet imports. A separate commandlet-only
subsystem fallback was corrected. The first preparation attempt was stopped by
the guard's false classification of UE's own CrashReportClient as a game; the
exact installed engine helpers are now distinguished. The first Cargo build was
stopped when another renderer started during the held lease; the warm build then
passed. These are preparation/coordination failures, not scored candidate frames.

Verification: the affected Rust test binary compiled with `-j 2` and its own E:
target; `gpu_contact_fold_batch` passed in **101.89 s** (guarded process **105.82 s**),
including all four film references and every Renderlab configuration. Four focused
Python tests cover geometry, population/missing accounting and native packed-HDR
decoding. Python AST parsing, affected Rust formatting and Git whitespace checks pass.
Independent CPU visibility checks and all retained native camera, CVar, shader
radius/SPP readbacks pass. The final resume-completion filter also checks PNG/HDR
presence; that final launcher-only tightening was CPU-checked after capture.

Implementation time was not separately stopwatched. Verification/preparation time
is recorded separately from frame time: the successful warm Cargo build was
**79.08 s** after a 91.02 s interrupted initial build; UE NullRHI attempts totaled
**91.85 s**; the two retained/partly retained capture processes were **105.62 s**
and **215.97 s**, excluding the earlier superseded 119.58 s harness failure and
86.04 s memory abort. Those process spans include startup/loading/dumps and are
never substituted for frame costs. CPU scoring/oracle work is additional.

The two memory-aborted captures reached **1863.26 / 2017.41 MiB** available RAM.
The final resume stayed above **3634.93 MiB** and had no known game/foreign-renderer
sample. Concurrent compilers still appeared (up to eight rustc processes in that
resume), and the live wallpaper renderer remained; no foreign process was stopped.
Unreal's earlier foreground endpoints were equal; the last resume's endpoints
differed, ending at Windows ShellExperienceHost. No activation/input operation was
issued, and endpoint snapshots cannot establish continuous focus stability.

Cleanup removed **2,699 raw/auxiliary dump files, 19,531,791,494 bytes (18.19 GiB)**,
including the large GPU arrays and superseded attempts. `cleanup.json` retains the
inventory and Renderlab raw-readback hashes. Scene inputs, JSON/CSV/log summaries,
native readbacks/metadata, inspected PNGs and the HTML comparison remain on E:.
Raw arrays must be regenerated to rescore a different metric. Cargo's own
`cargo clean --target-dir E:/renderlab-work/contact-fold/target` removed **748.8 MiB**;
the owned target directory is gone. No foreign target or application was removed.
