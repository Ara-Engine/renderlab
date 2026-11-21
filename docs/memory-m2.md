# Memory M2: dedicated memory by owner, and the gap to Unreal without ray hardware (21 Nov)

Unreal's dedicated memory for Outpost with software Lumen is 2,932 MiB (frozen 2026-11-14). Without ray hardware, Renderlab read 3,439 MiB. That gap blocks memory-costing speed-ups such as the mover-page static copy (Islam's rule: merge a speed-up that costs memory only while our total is under Unreal's for the matched scene and path).

## Method

- **Allocator report:** `RENDERLAB_MEMORY_REPORT=<file>[;<frame>]` (src/memory_report.rs) writes wgpu's allocator report once, after renderer frame 240 by default: reserved and allocated totals, then every live allocation's size and label. `main-thread/clipmap/memory/owners.py` groups the labels by owner.
- **WDDM counter:** the player's dedicated usage is sampled every 0.5 s through the run (n8h `gpumem.ps1`). The table takes its median within 2 s of the report.
- **Run:** drive.py's still phase (the first view), 1920x1080, editor play mode on a hidden window.
- **Memory hint:** `MemoryUsage` (8-64 MiB device blocks), as `MEMORY_HINTS` sets it.
- **Binary:** the yardstick's tree (main e78702ac) plus the measurement hook.

## Owners at the first view, MiB

| owner | Showcase RT | Showcase no-RT | Outpost RT | Outpost no-RT | Glint RT | Glint no-RT |
|---|---:|---:|---:|---:|---:|---:|
| virtual shadow clipmap | 0 | 449 | 0 | 592 | 0 | 589 |
| targets and history | 457 | 361 | 532 | 434 | 529 | 432 |
| lighting cache | 456 | 467 | 457 | 503 | 456 | 461 |
| textures | 394 | 394 | 345 | 345 | 26 | 26 |
| local light shadow maps | 0 | 8 | 0 | 384 | 0 | 48 |
| reflections | 282 | 267 | 255 | 315 | 255 | 228 |
| transparency | 248 | 197 | 248 | 197 | 240 | 189 |
| allocator slack (reserved - allocated) | 181 | 151 | 178 | 140 | 160 | 90 |
| geometry | 33 | 42 | 119 | 127 | 14 | 23 |
| software trace structures | 1 | 42 | 0 | 75 | 0 | 38 |
| other | 18 | 4 | 56 | 3 | 6 | 2 |
| **allocator reserved** | **2,069** | **2,382** | **2,189** | **3,116** | **1,685** | **2,126** |
| **WDDM dedicated** | **2,287** | **2,655** | **2,421** | **3,439** | **1,934** | **2,412** |
| Unreal (frozen) | 2,492 | 2,643 | 2,781 | 2,932 | – | – |

**Why Outpost without ray hardware is about 1,000 MiB heavier than with it** (allocator reserve 3,116 against 2,189 MiB):

| owner | MiB | detail |
|---|---:|---|
| virtual shadow clipmap | +592 | dense top and peel layers, 256 MiB each: 8 levels of 2048² texels at 8 bytes; max-mips 43 |
| local light shadow maps | +384 | a 4096² × 6-layer Depth32 atlas. Outpost's 17 point lights are cube maps, so all six layers are in use |
| software trace structures | +75 | the visibility buffer's cluster tables |
| software reflection geometry | +94 | |
| lighting cache's software geometry | +47 | |
| RT only, absent here | -71, -67, -17 | primary edge worklist; three glass targets; held shadow histories |

The WDDM counter reads 220-320 MiB above the allocator's reserve. That memory is outside the allocator: driver-side memory such as pipelines.

## Cuts tried

**Sparse storage with the dense windows** (`RENDERLAB_VSM_SPARSE=1 RENDERLAB_VSM_SPARSE_DENSE=1 RENDERLAB_VSM_POOL=384,100000`, default off):

- **Mechanism:** the sparse page pool holds the dense clipmap's own windows (16 pages, kept until the eye moves a page) with an unlimited raster budget. Receivers take the same levels, and only the requested pages are stored.
- **Memory:** the clipmap falls from 449-592 to 137-141 MiB. WDDM dedicated:

  | scene, no ray hardware | before (MiB) | after (MiB) |
  |---|---:|---:|
  | Outpost | 3,439 | 2,909 |
  | Showcase | 2,655 | 2,335 |
  | Glint | 2,412 | 1,964 |

- **Picture:** near the dense one but not byte-identical. At frame 40, Outpost has 34 pixels and Showcase 65 at 1 code. The yardstick's own reads differ by 4 pixels, the candidate's by 20.
- **Cost:** speed. The clip gate (`bin-smoke-ec829a2e` against `bin-smoke-e78702ac`, without ray hardware) read:

  | row | change |
  |---|---:|
  | Glint goal | +0.81 ms (worse) |
  | Outpost, fastest of 3 | +0.25 ms |
  | Glint course | -0.15 ms |
  | Showcase | +0.02 ms |

  That is the page-table indirection in the lookup. Not a default.

**Smaller allocator blocks:** rejected before (docs/memory-m1.md). 4-8 MiB blocks saved memory with identical images but cost 2-3% frame time.

**Local light atlas:** a split into one-layer spot and six-layer cube atlases saves almost nothing on Outpost, where the lights are points.

**Peel layer as height only:** not possible. The lookup reads the peel's triangle id (the own-mesh test and the edge-triangle masks).

## What would close the gap without costing picture or speed

None of the following is in place:

| option | MiB | what it needs |
|---|---:|---|
| sparse storage with dense windows, plus a lookup that pays no indirection | 530 | caching each pixel's few pages ahead of the walk, as the dense path's direct indexing does |
| shared software geometry for reflections and the lighting cache | about 47-94 | one structure instead of two copies of the same triangles |
| local light maps sized to the lights' screen size | up to 384 | changes the picture as resolution falls |
