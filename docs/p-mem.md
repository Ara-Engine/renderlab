# P-MEM: exact GPU memory reductions

2026-11-09. Branch `t3/p-mem`, baseline `7f4f75d5999c58fd90541900a935bcd88d5ef470`
from `origin/main`. The ARA1 report's `ae85750` totals are historical comparators,
not the before side of this unit's measurements.

## Contract and measurement

Keep every quality-carrying budget: GI slots/directions, local-light count and
atlas tile resolution, source geometry, texture mips, reflection rays, shadow
resolution and reconstruction settings. A retained storage change must produce
the same complete HDR and display bytes, including motion and cuts.

`ARA1_MEMORY=1` in `examples/ara1_capture.rs` records every named live allocator
allocation, its block and offset, total allocated/reserved bytes and the renderer's
ownership counters. It records an upload snapshot and a final warm snapshot.
`ARA1_HDR_HASH=1` hashes full HDR readbacks at the ARA1 capture frames; PNG files
are hashed separately. The scene, camera track, 88-frame history and all effects
remain the registered ARA1 quality inputs at 1920x1080 and 16x anisotropy.

The adapter also queries this process's DXGI local and non-local usage, on every
enumerated adapter, at capture boundaries and at the allocation snapshots.
This is the API behind Unreal's `LocalUsedMB` counter, rather than a board-wide
memory delta. [Microsoft documents the process-local query](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo).
Allocator reservation includes free block tails and host-visible blocks; it is
reported separately from local usage. These are sampled values, not a certified
instantaneous startup peak.

Device policy remains `renderlab::MEMORY_HINTS` (`MemoryUsage`, 8–64 MiB blocks).
M1 already implemented immutable BLAS compaction and retired upload/build scratch
after completion. Its smaller-block experiment regressed frame time; neither is
claimed as a new P-MEM saving.

All output and the two-job Cargo target are under
`D:/Coding/renderlab-work/p-mem/`. GPU jobs use `scripts/n8-run.py`, leases at
most 15 minutes, owner-matched schedule gates and the 2 GiB runtime guard.
Root's 11:00:47 UTC amendment permits 7 GiB admission for short non-quarry jobs;
quarry jobs still require 8 GiB. No interactive windows are needed.

## Before and after

Fresh baseline below, MiB (2^20 bytes), RTX 4070 SUPER, NVIDIA 617.14, Vulkan,
release. The timing input's 1920x1080 size and quality input's 19 capture points
are combined; all 88 camera/phase/effect frames are otherwise asserted equal.
The first 960x540 pilot is excluded. Every baseline uses the frozen executable,
including captures made while implementation source was being edited.

| Scene | Path | Live allocations | Allocator reserved | DXGI local, warm |
|---|---|---:|---:|---:|
| game-base | RT | 879.303 | 1,015.442 | 1,131.047 |
| game-base | non-RT | 1,276.706 | 1,392.250 | 1,435.797 |
| PBR oriented | RT | 944.606 | 1,083.442 | 1,199.047 |
| PBR oriented | non-RT | 1,205.693 | 1,332.250 | 1,375.797 |
| 64 lights | RT | 895.309 | 1,023.192 | 1,138.734 |
| 64 lights | non-RT | 1,676.899 | 1,784.000 | 1,827.484 |
| quarry | RT | 2,840.193 | 3,323.461 | 3,439.547 |
| quarry | non-RT | 3,907.909 | 4,091.344 | 4,135.422 |

The [per-resource CSV](p-mem-resources.csv) accounts for every live named
allocation on every row; after columns are filled when candidates are verified.
The full local evidence includes upload snapshots, allocations, block membership
and offsets in `D:/Coding/renderlab-work/p-mem/baseline1080/`.

Baseline checks: **198 passed, 0 failed, 163 ignored** (release, two test threads,
3.27 seconds). RF1–RF10 produce 13 final-radiance captures per path (26 total).
Q-walk records all 161 display hashes per path (322 total). The optional existing
PP1 sweep fails its neutral HDR-invariance assertion on this untouched baseline;
P-POST is investigating temporal fog history. Its ordinary assertion stays strict;
`P_MEM_POST_RECORD=1` records a complete comparison without asserting that separate
invariance property and records the result as `hdr_matches_disabled`.

## Audited candidates

- Post auxiliary, motion, depth-of-field and bloom targets are allocated even
  when their passes are off. Allocate full-size storage when a pass uses it;
  retain same-format 1x1 bindings otherwise. Preserve temporal state on toggles.
- GI cold records reserve four words for each of four directional bitsets even
  with 64 directions (two words used). Pack those unused words without changing
  slots or directions. Cell-shading pipelines and the light-list reduction test
  must use the matching layout, including 128-direction mode.
- Software-GI BVH uploads include four unread node error words, six unread leaf
  words and a position padding word. A runtime-only packed representation can
  preserve every field the shader reads, the same child/triangle order and every
  float bit. The cooked asset format and source data remain unchanged.
- An all-spot local atlas never samples cube faces 1–5. Allocate those faces
  when a point light actually occupies a slot, retaining all 128 slots and the
  existing tile allocator, coordinates, resolutions and cache keys.

## Remaining gaps

The at-or-under-Ara target is not established. The four implementation candidates
are not retained or claimed as savings until the after tables and complete-image
identity checks pass. The non-RT quarry's 407.074 MiB software-GI geometry buffer
is the largest audited exact packing candidate after immutable textures and walk
records. Existing fixed GI, clipmap and reconstruction budgets remain substantial.
