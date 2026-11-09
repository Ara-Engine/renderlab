# P-MEM: exact GPU memory reductions

2026-11-09. Branch `t3/p-mem`, baseline
`7f4f75d5999c58fd90541900a935bcd88d5ef470` from `origin/main`.
The measurement-only checkpoint was pushed first as `fbf2cbf19e5da5e019c6040f83da0620d439aae1`.

Four retained cuts preserve every recorded HDR/display byte. Live allocations
fall about **113 MiB on the small rows**, **208 MiB on non-RT quarry**, and
**433 MiB on the all-spot witness**. **The at-or-under-Ara target is still unmet
overall.** No GI capacity, directions, shadow resolution, 128-light capacity,
texture detail, reconstruction history or programmable shading budget is reduced.

## Before and after

MiB means 2^20 bytes. Each cell is before -> after. Primary quarry is ARA1's
`quarry-daylight`, exposure -8.5; the original -14 quarry row is supplementary.

| Scene | Path | Live allocated MiB | Reserved MiB | DXGI local, warm MiB |
|---|---|---:|---:|---:|
| game-base | RT | 879.303 -> 766.265 | 1,015.442 -> 919.442 | 1,131.047 -> 1,035.047 |
| game-base | non-RT | 1,276.706 -> 1,163.600 | 1,392.250 -> 1,296.250 | 1,435.797 -> 1,339.797 |
| PBR oriented | RT | 944.606 -> 831.567 | 1,083.442 -> 987.442 | 1,199.047 -> 1,103.047 |
| PBR oriented | non-RT | 1,205.693 -> 1,092.486 | 1,332.250 -> 1,236.250 | 1,375.797 -> 1,279.797 |
| 64 point lights | RT | 895.309 -> 782.270 | 1,023.192 -> 927.192 | 1,138.734 -> 1,042.734 |
| 64 point lights | non-RT | 1,676.899 -> 1,563.791 | 1,784.000 -> 1,688.000 | 1,827.484 -> 1,731.484 |
| quarry daylight | RT | 2,840.192 -> 2,727.154 | 3,259.461 -> 3,227.461 | 3,375.547 -> 3,343.547 |
| quarry daylight | non-RT | 3,907.909 -> 3,699.730 | 4,091.344 -> 3,900.203 | 4,177.422 -> 3,944.234 |

Small rows save 96 MiB of reserved and warm local memory. Quarry non-RT saves
191.140 MiB reserved and 233.188 MiB warm local usage in this pair. Quarry RT
saves 113.039 MiB live, but only 32 MiB reserved/local in the daylight pair.
The supplementary dark quarry has the same live saving and 160 MiB less
reservation. These 64 MiB block-packing differences are why live, reserved and
DXGI usage must remain separate; no universal driver-reservation saving follows
from a single pair.

Maximum captured DXGI local usage includes upload and the nineteen capture
boundaries, rather than just the final warm sample. This exposes the temporary
second GI generation in the 64-light case; it still does not certify an
instantaneous startup peak.

| Scene | RT before -> after MiB | non-RT before -> after MiB |
|---|---:|---:|
| game-base | 1131.047 -> 1035.047 | 1435.797 -> 1339.797 |
| PBR oriented | 1199.047 -> 1103.047 | 1375.797 -> 1279.797 |
| 64 point lights | 1362.797 -> 1234.797 | 2051.547 -> 1923.547 |
| quarry daylight | 3375.547 -> 3343.547 | 4177.422 -> 3944.234 |

ARA1's historical Ara local/dedicated values were game 633.9/618.7 MiB,
PBR 749.1/651.2 MiB, quarry daylight 3388.2/1965.8 MiB (hybrid/raster).
Only the fresh RT quarry snapshot falls below that historical counterpart.
There is no new Ara replay, no memory superiority claim for the other rows,
and no matched Ara memory result for the 64-light case. See
[m7-ara1-report.md](m7-ara1-report.md); its `ae85750` Renderlab values are not
this unit's before side.

## Itemized inventory

[p-mem-resources.csv](p-mem-resources.csv) contains every named allocation,
resource kind, count and exact byte total, **both upload and warm snapshots**,
before and after, on all twelve rows. Each snapshot's grouped sum is asserted
equal to the allocator's total. Exact CSV zero means absent in that snapshot, not an unmeasured estimate;
rounded table entries may show 0.000 for a small placeholder. [p-mem-memory.json](p-mem-memory.json) retains category
totals, reservation slack, DXGI local/non-local and sampled local maxima.
Grouped warm allocations below are MiB; unchanged groups show one value.

### game-base

| Resource group | RT before -> after | non-RT before -> after |
|---|---:|---:|
| Geometry buffers | 0.461 | 0.611 |
| Material/environment/LUT textures | 0.563 | 0.563 |
| Other frame targets/textures | 164.253 | 178.028 |
| TR1 history textures | 168.750 | 168.750 |
| GI cache buffers | 384.255 -> 352.255 | 384.255 -> 352.255 |
| Software GI geometry buffer | 0.000 | 0.298 -> 0.230 |
| Sun-shadow buffers | 0.000 | 427.415 |
| Sun-shadow textures | 0.000 | 16.438 |
| Local shadow atlas | 0.000 | 0.000 |
| Post textures | 81.056 -> 0.017 | 81.056 -> 0.018 |
| Post uniforms | 0.000 -> 0.001 | 0.000 -> 0.001 |
| BLAS/TLAS | 0.302 | 0.000 |
| Other buffers | 79.662 -> 79.663 | 19.291 |
| Upload/build staging | 0.000 | 0.000 |

### PBR oriented

| Resource group | RT before -> after | non-RT before -> after |
|---|---:|---:|
| Geometry buffers | 1.322 | 1.702 |
| Material/environment/LUT textures | 64.622 | 64.622 |
| Other frame targets/textures | 164.254 | 178.028 |
| TR1 history textures | 168.750 | 168.750 |
| GI cache buffers | 384.255 -> 352.255 | 384.255 -> 352.255 |
| Software GI geometry buffer | 0.000 | 0.741 -> 0.572 |
| Sun-shadow buffers | 0.000 | 289.293 |
| Sun-shadow textures | 0.000 | 16.438 |
| Local shadow atlas | 0.000 | 0.000 |
| Post textures | 81.056 -> 0.017 | 81.056 -> 0.017 |
| Post uniforms | 0.000 -> 0.001 | 0.000 -> 0.001 |
| BLAS/TLAS | 0.685 | 0.000 |
| Other buffers | 79.663 | 20.808 |
| Upload/build staging | 0.000 | 0.000 |

### 64 point lights

| Resource group | RT before -> after | non-RT before -> after |
|---|---:|---:|
| Geometry buffers | 0.461 -> 0.461 | 0.611 |
| Material/environment/LUT textures | 0.563 | 0.563 |
| Other frame targets/textures | 164.253 -> 164.253 | 178.028 |
| TR1 history textures | 168.750 | 168.750 |
| GI cache buffers | 400.255 -> 368.255 | 400.255 -> 368.255 |
| Software GI geometry buffer | 0.000 | 0.298 -> 0.230 |
| Sun-shadow buffers | 0.000 | 427.415 |
| Sun-shadow textures | 0.000 | 16.438 |
| Local shadow atlas | 0.000 | 384.188 |
| Post textures | 81.057 -> 0.017 | 81.058 -> 0.017 |
| Post uniforms | 0.000 -> 0.001 | 0.000 -> 0.001 |
| BLAS/TLAS | 0.302 | 0.000 |
| Other buffers | 79.667 -> 79.668 | 19.295 |
| Upload/build staging | 0.000 | 0.000 |

### quarry daylight

| Resource group | RT before -> after | non-RT before -> after |
|---|---:|---:|
| Geometry buffers | 602.023 | 835.328 |
| Material/environment/LUT textures | 1,024.582 | 1,024.582 |
| Other frame targets/textures | 164.254 -> 164.254 | 178.027 |
| TR1 history textures | 168.750 | 168.750 |
| GI cache buffers | 384.255 -> 352.255 | 384.255 -> 352.255 |
| Software GI geometry buffer | 0.000 | 407.074 -> 311.933 |
| Sun-shadow buffers | 0.000 | 792.015 |
| Sun-shadow textures | 0.000 | 16.439 |
| Local shadow atlas | 0.000 | 0.000 |
| Post textures | 81.056 -> 0.017 | 81.056 -> 0.017 |
| Post uniforms | 0.000 -> 0.001 | 0.000 -> 0.001 |
| BLAS/TLAS | 335.608 -> 335.608 | 0.000 |
| Other buffers | 79.663 | 20.382 |
| Upload/build staging | 0.000 | 0.000 |

Upload/build staging at the post-upload snapshot is unchanged. Existing upload
collection and M1's asynchronous immutable-BLAS compaction already retire these
allocations; the warm staging total is zero on every main row.

| Scene | RT upload staging MiB | non-RT upload staging MiB |
|---|---:|---:|
| game-base | 0.158 | 0.181 |
| PBR oriented | 0.050 | 0.088 |
| 64 point lights | 0.158 | 0.181 |
| quarry daylight | 66.295 | 79.174 |

### Supplementary exact witnesses

| Scene | Path | Live allocated MiB | Reserved MiB | DXGI local, warm MiB |
|---|---|---:|---:|---:|
| 64 spots | RT | 895.309 -> 782.269 | 1,023.192 -> 927.192 | 1,138.734 -> 1,042.734 |
| 64 spots | non-RT | 1,676.899 -> 1,243.791 | 1,784.000 -> 1,368.000 | 1,827.484 -> 1,411.484 |
| quarry exposure -14 | RT | 2,840.193 -> 2,727.154 | 3,323.461 -> 3,163.461 | 3,439.547 -> 3,279.547 |
| quarry exposure -14 | non-RT | 3,907.909 -> 3,699.730 | 4,091.344 -> 3,900.203 | 4,135.422 -> 3,944.234 |

The all-spot non-RT local depth texture is 384 -> 64 MiB at the unchanged
1024 map setting. Six faces return before any selected point-light slot needs
them, including transitions. RT does not allocate that mapped-shadow pool.

## Retained implementation

1. **GI cold-record packing: 32 MiB.** Four directional bitsets contain two
   words each for 64 directions, rather than four. Cold records become 24 words
   instead of 32 at the default slot/light counts. Every direction and light bit
   remains; 128 directions retain the original 32-word record. The original
   slot-admission policy stays unchanged even on smaller binding-limit devices.
   Kernel and cell-shading layouts share the chosen direction count; the latter's
   cache key includes it. The light-reduction test derives its offset from the
   production layout. Conditional workgroup loads avoid reading past two-word
   scratch entries. No GI quality setting or allocation limit is lowered.

2. **Lazy post targets: about 81.039 MiB at 1080p with inactive effects.**
   Auxiliary, velocity, motion, DOF and bloom targets use same-format 1x1 valid
   bindings while their passes are unused. Active effects restore full dimensions
   before encoding. Camera history and frame serial survive effect toggles;
   bind groups rebuild on topology changes, and replaced targets remain owned
   until their last submission completes. No post shader or sampling math changes.
   ARA1 explicitly sets its post profile (motion/DOF off and optional bloom);
   the engine settings debug string also contains constructor defaults, so the
   canonical scene-spec hash is part of the identity proof.

3. **Packed runtime software-GI geometry: about 95.140 MiB on quarry.**
   Nodes retain their 16 consumed words, leaves their index offset/count, and
   positions every XYZ float bit. Unread node-error words, leaf metadata and W
   padding are omitted from the GPU upload. Child order, triangle order, local
   indices, near-field masks and traversal arithmetic remain identical. Cooked
   assets and the general cluster-BVH representation do not change. The quarry's
   uploaded software-GI geometry falls from about 407.074 to 311.933 MiB.

4. **Unreachable spot cube faces: 320 MiB in the all-spot witness.**
   The atlas uses one D2Array layer when no selected slot needs a cube; any point
   slot restores six. Tile coordinates, map resolution, all 128 slots, view
   matrices, cache keys and lighting algorithms stay unchanged. A backing change
   triggers the existing generation invalidation/redraw and normal retirement.
   The point-light ARA1 rows correctly keep all six layers.

Transient target aliasing was not added: inactive post allocation was a simpler,
measured exact cut. BLAS compaction and staging retirement are existing behavior,
not new savings. No source geometry or material copy was deleted without proving
its programmable consumers unreachable.

## Identity and tests

**All recorded comparisons pass.** [p-mem-identity.json](p-mem-identity.json)
contains the complete SHA-256 manifest and `all_identical: true`.

| Suite | Paths | Equal complete-image evidence |
|---|---|---|
| ARA1 main + supplementary | RT/non-RT | 12 rows x 19 HDR hashes and 19 PNG hashes = 228 HDR + 228 PNG |
| RF1-RF10, native/TR1 | RT/non-RT | 13 final-radiance float captures each = 26 |
| Q-walk | RT/non-RT | all frames 0-160 = 322 display hashes; camera/frame arrays also equal |
| Post sweep, 13 variants | RT/non-RT | 52 PNGs plus 26 HDR/display hash pairs |
| LC2/LC3 point, spot, fade | RT/non-RT | 10 RT + 14 non-RT PNGs, including 64-light visibility |

The library run is **199 passed, 0 failed, 163 ignored**, release, two test
threads, 3.10 seconds (baseline 198/0/163, 3.27 seconds). The added boundary test
checks non-overlap and capacity of the 64/128-direction cold layouts across
light counts 0, 1, 32, 64, 128, 300 and 65,535. Existing shader validation covers
both ray sources and both direction counts. The software-geometry packing test
checks every consumed field's original bits and near-field mask placement.
GPU RF, Q-walk, LC2 and the record-mode post suite completed on both paths.

The ordinary pre-existing PP1 neutral HDR-invariance assertion fails on the
untouched baseline. P-POST owns the reset/strict-assertion fix and is investigating
temporal fog history. `P_MEM_POST_RECORD=1` bypasses only that assertion to record
all thirteen variants for cross-build identity; the JSON retains
`hdr_matches_disabled`. The ordinary assertion stays strict. This unit claims
cross-build identity, **not** that the separate PP1 invariance gate passed.

Verified visually: full quarry daylight RT/non-RT frame 87, Q-walk non-RT
frame 64, point/spot penumbra fixture, post effects, all-spot game and RF
inspection views under `D:/Coding/renderlab-work/p-mem/`. Their source captures
match the frozen baseline hashes; no new reference-quality superiority is claimed.

Canonical hash-evidence digest: `6d1bcc2e1f0ac35cdf3dfa33b63b3c2e448abc93a3565a81c37b0f934cb3b0d0`.

## Measurement and reproducibility

`ARA1_MEMORY=1` in `examples/ara1_capture.rs` writes every allocator label,
individual offset and block, allocated/reserved totals, ownership/HAL counters
and `MEMORY_HINTS` after upload and after the 88-frame replay.
`ARA1_HDR_HASH=1` hashes the full HDR readback at every captured frame.

The Windows-only SDK DXGI adapter queries this process's local and non-local
`QueryVideoMemoryInfo` usage, the API behind Unreal's `LocalUsedMB`. See
[Microsoft's API contract](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo).
Repeated DXGI aliases of the NVIDIA adapter are not summed; take the maximum.
This is process-local usage, not a board-wide delta. Reservation includes host
blocks and free tails. Values are samples, not an instantaneous startup-peak
certificate.

Feature/topology transitions retain old and new backing until GPU completion.
Their transient peak can exceed the warm table; these savings do not certify a
worst-case transition or startup peak.

Hardware: RTX 4070 SUPER, NVIDIA 617.14, Ryzen 7800X3D, Vulkan, release,
1920x1080, 16x anisotropy. The harness combines the registered timing size with
the quality spec's nineteen capture points and asserts every other frame field
equal across the two specs. All 88 poses, histories, cuts and effects are retained.
The initial 960x540 pilot is excluded. Both sides use frozen executables from
one source base. The candidate and baseline binary identities are in
[p-mem-builds.json](p-mem-builds.json).

Memory policy stays `MemoryUsage` with 8-64 MiB blocks. M1 already rejected a
smaller-block policy for its frame-time cost. Peak capture-boundary local usage
for 64 point lights is RT 1362.797 -> 1234.797 MiB, non-RT 2051.547 -> 1923.547 MiB;
the warm table is lower. Initial light-bitset growth temporarily retains another
GI resource generation. Simply deferring the initial GI allocation would alter
first-frame bindings because lighting upload precedes GI preparation; it was not
implemented.

All GPU work went through `scripts/n8-run.py`, <=15-minute leases, latest root
order checks and owner-prefixed release gates. Short non-quarry jobs admitted at
>=7168 MiB per root's amendment; quarry at >=8192, runtime guard always 2048.
No owned compiler ran through another unit's compiler-quiet hold. Output and the
two-job Cargo target stayed under `D:/Coding/renderlab-work/p-mem/`.
Raw RF and Q-walk dumps above 5 MB were summarized and removed. Sixty-six
representative PNGs were inspected, individually and in three contact sheets;
their retained-image manifest is under the local evidence's `report/` directory.
Automatic approval review blocked the subsequent deletion of unretained PNGs
with "blocked by policy", so those additional PNGs remain. The Cargo target's
resolved path was verified as exactly `D:/Coding/renderlab-work/p-mem/target`
(ordinary directory, 1353 files, 816564472 bytes). The requested literal-path
recursive deletion was also rejected by automatic approval review with
"blocked by policy"; a subsequent existence check confirmed it remains. No
alternate deletion mechanism was attempted. Frozen proof binaries remain in
the separate `bin/` directory.

Public replay helpers:

```powershell
# Read the latest root order and wait for the exact owner-prefixed release first.
$env:P_MEM_CAPTURE_ONLY = '1'
py -3 -B scripts/p-mem-lease.py --label quarry-proof --minutes 10 -- `
  py -3 -B scripts/p-mem-capture.py candidate quarry-daylight
# RF/post/LC2 are short suites; Q-walk loads quarry and needs the 8 GiB floor.
py -3 -B scripts/p-mem-identity.py BEFORE AFTER REPORT.json
py -3 -B scripts/p-mem-inventory.py BEFORE AFTER OUTPUT
```

Frozen binaries are expected in `P_MEM_OUT/bin`; scene specs default to the
registered ARA1 phase2 directory, overridden with `P_MEM_SCENES`. The fixture
helper reuses the registered RF and Q-walk inputs and emits per-suite logs.
Record-mode post capture is explicit. Build separately with `-j 2` and the
worker target on D:, obeying any active compiler hold.

Schedule readers stream line by line so the growing shared history is never
copied wholesale. The final retime's first preflight (13:06 UTC) deferred at
6935 MiB with foreign compilers active, before any GPU execution. Its rejected
preflight is retained separately; it contributes no timing result.

The second attempt admitted at 13:41:04 UTC (12672 MiB, no compiler). At
13:41:22 the runtime sample detected new foreign `cargo.exe`/`rustc.exe`
processes, with 10647 MiB still available, and the strict no-build guard aborted
the job. It released at 13:41:23. Only the first baseline game process completed;
there is **no complete A/B pair or quiet quarry retime**, and no no-regression
clearance. [p-mem-retime.json](p-mem-retime.json) records the rejected and aborted
attempts. No preflight/runtime guard was reduced, and no foreign job was stopped.

## Timing screen and limits

One GPU timestamp screen is retained from these same replays. Cells below are
before -> after mean GPU span in milliseconds, eight samples at indices 32-39
(still) and 64-71 (moving). Quarry and some small captures overlapped other units'
CPU compilation. Capture/readback scheduling also changes power/thermal behavior.
These numbers are diagnostic, **not a quiet-window performance claim**.

| Scene | Path | Still GPU ms | Moving GPU ms |
|---|---|---:|---:|
| game-base | RT | 3.279 -> 3.269 | 5.204 -> 5.242 |
| game-base | non-RT | 7.393 -> 8.435 | 8.805 -> 10.340 |
| PBR oriented | RT | 2.369 -> 2.159 | 3.474 -> 3.211 |
| PBR oriented | non-RT | 4.675 -> 4.725 | 4.821 -> 4.826 |
| 64 point lights | RT | 9.311 -> 9.610 | 11.048 -> 11.657 |
| 64 point lights | non-RT | 16.084 -> 17.288 | 17.234 -> 17.526 |
| quarry daylight | RT | 2.528 -> 2.651 | 3.550 -> 3.506 |
| quarry daylight | non-RT | 12.017 -> 14.294 | 12.153 -> 10.701 |

No frame-time superiority is inferred. Implementation occupied about one hour
of interleaved allocation/shader work and retained-build preparation. GPU proof
used short leases; final candidate small and quarry leases took 389.5 and 270.6
seconds. Baseline quarry took 136.7 seconds; the baseline small job's 274.0
seconds includes its recorded PP1 assertion failure. Build, queue, coding and
verification overlap; wall time is not attributed entirely to either algorithm
implementation or GPU verification.

The unit closes within its three-hour box with the timing effect still open.
Resolve the non-RT deltas with the frozen ABBA replay in a genuinely quiet window
before treating these storage cuts as cleared for frame-time regression.

## Gaps and next exact candidates

The remaining non-RT quarry live allocation is about 3699.730 MiB. Its largest
resources are immutable material mips (1024.022), walk records (447.518),
software-GI geometry (about 311.933), mesh vertices (303.673), mesh indices
(298.345), raster cluster geometry (226.911), GI table (192), sun clipmap layers
(192), GI filter sources (88), TR1 histories (168.750), and GI work (56.250).
They carry quality or serve distinct consumers; deleting them is not justified.

Further work must prove address-compatible geometry sharing/encoding, sparse
physical GI backing with unchanged logical capacity, reusable GI ancillary
buffers during light-count growth, and overlap-safe transient aliasing. Mixed
point/spot pools still reserve six whole layers; eliminating unused faces there
requires address/layout work and a full cache/transition proof. CPU geometry
deduplication helps host RAM but does not establish GPU savings by itself.
No lower GI budget, smaller shadows or mips are proposed. The focused backlog
is appended to [m7-gaps.md](m7-gaps.md).
