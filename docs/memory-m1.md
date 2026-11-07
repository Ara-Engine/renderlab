# M1: GPU memory, allocation and lifetime

Work in progress, base `d798200`, branch `t3/m1-memory`.

## Measurement before implementation

The starting inventory is the measured N8h allocation report, not an estimate
from source geometry sizes. N8j changed configuration resets, not these resource
layouts. [The baseline CSV](memory-m1-baseline.csv) lists **every allocator label**,
its exact allocated bytes in all three paths, usage and owning lifetime. Original
reports: `E:/renderlab-work/n8h/final-mem/{nort-rsource-mu,traced-interactive-mu,traced-virtual-rsource-mu}/memory-*.json`.
Current-base replays will accompany the retained changes.

MiB means 2^20 bytes. In this starting evidence, no-RT reserves 3,511 MiB,
RT 3,175 MiB, and traced primaries plus Virtual shadows 3,927 MiB. WDDM maxima
are 3,662 / 3,241 / 4,001 MiB, respectively. These include driver allocations;
allocator reserved bytes include free block tails. They are different quantities.
The matching recorded Unreal DXGI totals are 3,912 / 5,410 / 3,946 MiB.
The hybrid has no exact Unreal counterpart: ours traces camera visibility and
retains 870 MiB of BLAS; Unreal's VSM configuration rasterizes Nanite.
See [N8h](non-rt-n8h.md) for the matched workload and measurement limits.

The largest safe candidates identified by the inventory are:

- 870 MiB of uncompressed immutable BLAS on the two traced paths. Test wgpu's
  asynchronous compaction, preserving primitive order and hit-vertex return.
- 79.1 MiB of deferred Interactive edge queues allocated even on Virtual-shadow
  paths, where they are never dispatched. Preserve full capacity when used.
- Allocator block slack: 74 MiB no-RT, 132 MiB RT, 149 MiB hybrid. Inspect actual
  block packing before changing allocation hints.

Vertices already share their raster/storage/BLAS allocation. Upload staging and
build scratch are absent from the warm reports; CPU upload vectors are not GPU
memory. Texture mips account for 1,024 MiB, but reducing them or using lossy formats
would change output. Cluster position copies and source/LOD adjacency are real
costs; changing those shader layouts is outside this allocation/lifetime unit.
Shadow walk records, horizons and clipmap resources are deliberately untouched.

## Reusable harness

`scripts/memory-m1.py` runs `gpu_visibility_quarry` once per path, loading the
quarry once per process and replaying both poses, static, turning-sun, camera and
caster motion. It saves allocator reports **after upload and after warm-up**,
including every allocation's name, bytes, offset and block membership. Each
report has an adjacent CSV adding usage and lifetime. The catalogue rejects
unknown names. `scripts/memory-wddm.ps1` samples per-process WDDM Dedicated Usage.
Complete hit/count/HDR/world snapshots are hashed and raw buffers deleted.

Run through the existing headless shared lease and its 4 GiB start / 2 GiB abort
guard (with no game running):

```powershell
$env:N8_OWNER='m1-memory'
$env:N8_OUT='E:/renderlab-work/m1-memory'
py -3 -B scripts/n8-run.py --label memory --minutes 8 --fast-check -- py -3 -B scripts/memory-m1.py <test-binary> E:/renderlab-work/m1-memory/run
```

`--memory-only` records one warm pose without the motion replay. The runner owns
`D:/Coding/renderlab-work/gpu-slot.lock`, reservations/releases in
`E:/renderlab-work/gpu-schedule.md`, game detection and resource guards. No window
is opened. CSV lifetime descriptions are ownership contracts, not proof that
resources with overlapping ownership can safely alias.

## Retained changes and verification

Pending implementation and measurement. No performance or image-identity claim
is made for an unmeasured variant.
