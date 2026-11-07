# N8l: the non-RT exact walk under camera motion, divergence and latency

**Follow-up: [N8m](non-rt-n8m.md)** found the residency limiter left open below: the one-workgroup-per-tile launch, not a resource. Registers and spilled shared memory both allow 12 warps per SM; the launch reached 8.5-9.3 and ended in a tail of late heavy tiles. Persistent workgroups taking tiles heaviest first hold 12.00 warps and cut camera-moving frames 22-23% (1.93-1.96 → 1.50-1.51 ms), byte-identically.

Follow-up unit to [N8i](non-rt-n8i.md), 2026-11-07 UTC, base `d798200`, merged with main `f09d2d0` (M1/M2 memory work), branch `t3/n8l-resumable-walk`. Brief (Islam, 2026-11-07): remove the wave divergence in the virtual-shadow exact walk, which N8i's profile put at 8.8 of 32 active lanes on camera-moving frames, byte-identically; profile before and after. Mid-unit, Islam changed the time-box: keep trying designs and profile each, rather than stop at the first 25% miss. A later addendum asked for an A/B of M2's packed vertex-position stream ([memory-m2.md](memory-m2.md), "Walk-record proposal"). Labels: **M** measured here, **I** inference.

## Result

**Kept: a byte-identical lookup restructure. Camera-moving frames fall 17.5%: 2.38-2.40 → 1.96-1.98 ms on the no-RT quarry, 2.43-2.46 → 2.02-2.03 ms on the traced quarry. Static and turning frames fall 16-17%, caster-moving frames 9%. Camera motion still misses VSM's estimated ~0.5 ms by about 4x.**

- **What paid was latency, not divergence.** Three exact changes, measured one at a time:
  - **Per-lane workgroup tables (S1/S2, −10%).** The lookup's per-receiver direction and shear tables are indexed by a runtime direction, so the driver kept them in local memory. They now live in per-lane workgroup slots, laid out [component][lane] so a warp's reads are conflict-free. The receiver triangle and the repeat filter are now read only by constant indices, so they stay in registers.
  - **Neighbour prefetch (P1, a further −6%).** A walk or fan step loaded its triangle, chose the exit edge from it, then loaded that edge's neighbour: two dependent round trips per step. All three neighbour words now load beside the triangle's record, and the edge picks one by select.
  - **Per-walk hoists (P2/P3, a further −2.5%).** The adjacency suffix's base used to be a buffer-length (descriptor) load on every neighbour read, and the record range a runtime-indexed uniform read on every record load. Each is now read once per walk or ring.
- **The divergence designs the brief listed were built, profiled and rejected** (table below):
  - a resumable per-lane state machine with pixel refill: 2.2-8x slower;
  - a cooperative ring: +3%;
  - subgroup-cooperative walks: 2.1x slower. They doubled active lanes, from 8.3 to 16.0, but the group reduction and the warp-uniform march cost more than they saved.

  Breadth-first expansion was not built: the walk cap makes visit order part of the output (below).
- **M2's packed vertex stream works but regresses.** It is byte-identical and cuts walk storage 447.5 → 75.9 MiB (allocator reserve −368 MiB). Camera motion slows 11% (2.19-2.21 against 1.96-1.98 ms), and with an index prefetch 19%. **Not kept**, per the addendum's condition; its code is kept as a diff.
- **Output is main's, byte for byte**, on every hash and score this unit checks (Quality, below). Memory is unchanged.
- **Subgroups on this device:**
  - Vulkan reports subgroup size 32 with basic, vote, arithmetic, ballot, shuffle, relative shuffle, clustered, quad, rotate and partitioned operations.
  - wgpu 30 exposes them as `Features::SUBGROUP`. naga 30's WGSL front end takes `subgroupBallot`, `subgroupAll`/`subgroupAny`, reductions and scans, `subgroupBroadcast(First)`, `subgroupShuffle(Up/Down/Xor)`, `quadBroadcast`/`quadSwap*` and the `subgroup_*` builtins. There is no clustered reduction, and `enable subgroups;` is unimplemented.
  - `subgroup_invocation_id` needs a one-dimensional workgroup.
  - The shipped lookup uses none of them.

## Timing (M)

RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile, validation off, 1920x1080, poses 0 and 1. Main is main's shader on the same binary (`RENDERLAB_VSM_SHADER`), in separate processes interleaved in one lease (final, main, final, main). A visit is clean when its primaries read at most 0.45 ms (no-RT) or 0.30 ms (traced). Ranges are clean visits' medians. The traced primaries ran in their own lease (final-b).

**No-RT quarry at LOD 0.35 px, shadow pass, ms**

| Configuration | Static | Sun turning | Camera moving | Caster moving | No-RT frame (static / camera / caster) |
|---|---:|---:|---:|---:|---|
| Main (base) | 0.674-0.697 | 0.676-0.697 | 2.382-2.402 | 2.058-2.136 | 1.42-1.43 / 3.11-3.16 / 2.78-2.89 |
| **N8l (shipped)** | **0.563-0.572** | **0.570-0.579** | **1.957-1.980** | **1.861-1.943** | **1.28-1.31 / 2.68-2.73 / 2.57-2.68** |
| Unreal VSM (recorded) | 0.50 | about 0.95 | not recorded (about 0.5, I) | not recorded | 1.61 / — / — |

**Traced quarry, RT device, validation off, ms**

| Primaries | Main: static / turning / camera / caster | N8l: static / turning / camera / caster |
|---|---|---|
| Traced | 0.630-0.638 / 0.631-0.640 / 2.427-2.463 / 2.135-2.190 | **0.522-0.534 / 0.538-0.544 / 2.021-2.027 / 1.936-1.985** |
| LOD 0.35 px (RT device) | 0.672-0.690 / 0.682-0.695 / contended | 0.564-0.583 / 0.580-0.586 / contended |

- **The validation-on traced screen read N8l slower** than main (static 0.701 against 0.631, camera 2.638 against 2.456). With validation off on the same device, N8l is 17% faster at every column above. The validation layer weighs on the lookup's extra loads (I); N8i already found validation-on screens unreliable for timing. Its hashes and agreement are used, not its times.
- **S2 and S3** (validation on, as scored). Shadow p50 is in ms, against N8h's last recorded run (renderer output identical since); the frame column is GPU p50 against M2's run on main.

| Bench / phase | N8h shadow p50 | N8l shadow p50 | VSM | Frame GPU p50, M2 → N8l |
|---|---:|---:|---:|---|
| S2 camera, out / hold / return | 0.68 / 0.22 / 0.68 | **0.48 / 0.18 / 0.48** | 0.35 | 0.86 / 0.40 / 0.86 → 0.66 / 0.37 / 0.66 |
| S2 shadow step, out / hold / return | 0.40 / 0.17 / 0.39 | **0.36 / 0.14 / 0.36** | 0.35-0.36 | 0.63 / 0.39 / 0.63 → 0.53 / 0.32 / 0.52 |
| S3, six phases | 0.26-0.27 | **0.23-0.24** | 0.37 static, 0.59 rotating | 0.49-0.51 → 0.46-0.47 |

## Every design measured (M)

No-RT quarry, camera-moving shadow pass, clean-visit medians over both poses and all visits. All variants run on main's binary layout with shader files (`E:/renderlab-work/n8l/variants/`). Every row is byte-identical to main at both poses unless noted.

| Step | Change | Camera ms | Verdict |
|---|---|---:|---|
| Main | — | 2.38-2.41 | base |
| c1/c2, pool 1x1 | Resumable per-lane program: the ring fans and the march's level, step, slot, layer, attempt, phase and walk loops become 21 resume points; one shared triangle visit; no refill | 5.21-6.24 | rejected; pose 0 differs (`cd9b0c90…`), the others equal |
| c2, pool 2x2 | The same, plus lanes refill from a 4-tile pool, waiting until 8 lanes are idle | 9.31-9.44 | rejected |
| c2 variants | Refill 1 / 24, 8 control steps per round, pools 4x1 / 4x4, pool 1x1 with 1 control step | 9.91 / 9.49-9.64, 8.94-8.99, 13.74 / 19.09-19.25, 5.91 | rejected |
| R1 | Constant-index `vsm_own` in the ring | 2.38-2.39 | no gain |
| R2 | `vsm_shear` in per-lane workgroup slots (vec3) | 2.21-2.22 | superseded by S1 |
| R3 / R4 | R2 plus `vsm_own`; plus `vsm_dirs` | 2.26 / 2.22 | no further gain |
| S1 | Shear in packed f32 slots [component][lane] | 2.19 | kept (part of S2) |
| **S2** | S1, plus packed `vsm_dirs` slots, plus constant-index `vsm_own` and `vsm_last` | **2.15-2.17** | **kept** |
| S3 | S2 with the shear recomputed from the directions per test | 2.90-2.91 | rejected |
| B1 / R5 / R5B1 | Branch-free tests / the walk's model matrix in workgroup slots / both (on R2) | 2.44-2.47 / 2.27 / 2.57-2.61 | rejected |
| W2 / W4 | 2 / 4 trace tiles (warps) per workgroup | 2.13 / 2.23 | noise / rejected |
| H1 | Non-thin per-direction slots reuse the march origin | 2.15 | noise |
| D1 / D2 | Set-bit direction loops in walk tests / plus ring tests | 2.48 / 2.47-2.50 | rejected |
| G | Cooperative ring: corner fans as workgroup jobs (atomic OR/max, two barriers) | 2.22 | rejected (caster frames −3.5%) |
| **P1** | S2 plus all three neighbour words loaded beside the record (walk and fan steps) | **2.03-2.05** | **kept** |
| PG | P1 plus the cooperative ring | 2.13 | rejected |
| **P2** | P1 with the adjacency base found once per walk or ring | **2.00-2.02** | **kept** |
| **P3** | P2 with the record range read once per walk or ring | **1.97-2.03** | **kept (shipped, with final comments)** |
| P4 | P3 plus the three neighbours' records loaded speculatively | 2.67-2.71 | rejected |
| P5 / P6 | Shear as one 16-byte slot per direction / plus the peel word loaded beside the top walk | 2.01-2.08 / 2.00-2.01 | noise |
| CW | Subgroup-cooperative walks on P3 (warp-uniform march, 8-lane groups split the directions) | 4.18-4.19 | rejected |
| Vertices | P3 with packed 12-byte vertex positions through the index buffer (M2's proposal) | 2.19-2.21 | rejected (−368 MiB reserved) |
| V2 | Vertices plus the neighbours' index triples loaded speculatively | 2.35 | rejected |

The fixture (960x540, two suns) was byte-identical for c1 and CW; c1 was 2.5-4x slower there too.

## Profile, camera-moving frames, before and after (M)

Nsight Graphics 2026.4.0 GPU Trace (Top-Level Triage), N8g's recipe: no-RT quarry pose 0, `move_wrap` 48 mm, four submits from 45 s. Every trace landed in camera motion: `vsm_raster` read 13 µs (empty indirect dispatches). Clocks unaltered, no screenshot, no window.
- The real-time shader profiler gives the per-line samples.
- Multi-pass traces give register and shared-memory allocation. Registers per thread are CS register allocation ÷ (resident warps × 32).

| Lookup, per camera-moving frame | Main | c2 (2x2) | S2 | P1 | CW | **N8l shipped** |
|---|---:|---:|---:|---:|---:|---:|
| Duration | 2.31-2.34 ms | 9.12-9.37 | 2.04-2.08 | 1.86-1.95 | 4.08-5.38 | **1.83-1.93** |
| Warp instructions issued (longest dispatch) | 432 M | 801 M | — | — | 578 M | **373 M** |
| IPC (of 4) / SM throughput | 1.17 / 29.4% | 0.54 / 13.6% | — / 28.8% | 1.24 / 30.9% | 0.68 / 17.1% | **1.23 / 30.9%** |
| Active threads per warp | 8.8 | 5.85 | — | 8.3 | 16.0 | **8.5** |
| Resident compute warps per SM | 7.7 | 3.0 | 7.4 | 7.4 | 5.6 | **7.35** |
| Stalls: long scoreboard / wait / selected | 30.8 / 30.3 / 15.7% | 13.1 / 37.6 / 18.7% | 26.2 / 32.5 / 16.1% | 22.0 / 33.9 / 17.2% | 28.1 / 28.8 / 12.9% | **22.9 / 33.4 / 17.2%** |
| Short scoreboard / branch / no instruction | 6.8 / 5.8 / 4.1% | 5.8 / 7.8 / 11.1% | 7.2 / 7.0 / 4.3% | 7.3 / 7.2 / 4.7% | 12.6 / 10.3 / 4.0% | **7.0 / 7.2 / 4.7%** |
| L2 / VRAM throughput | 12.8 / 10.0% | 5.3 / 3.6% | — | — | — | **8.7 / 12.3%** |

**Where the samples are, by function (self time, every profiled line).**
- **Main:**
  - march control 20.4%, walk control 11.8%, ring 7.2%, ring tests 6.6%, texel candidates 6.4%;
  - records 6.1%, walk tests 5.2%, `vsm_same` 4.6%, watertight test 4.4% plus 4.0% in `vsm_difference`;
  - adjacency 3.4%, appended-LOD chains 3.3%.

  The march's hottest lines stall on long scoreboard where they first use `vsm_step(j)` (`vsm_dirs[j]`) and `vsm_last[min(j,7)]`. Those are runtime-indexed private arrays, which sit in local memory.
- **S2:** march control falls to 9.9%. The remaining long-scoreboard lines are the walk's dependent loads.
- **P1:** the adjacency function holds 11.2%. Of that, 4.3% is `arrayLength` (a descriptor load per neighbour read), which P2 hoists.
- **N8l shipped:** march 12.6%, records 8.7%, walk control 8.1%, tests about 25% (`vsm_hits_sheared` 7.3, `vsm_hits_mask` 6.9, `vsm_difference` 4.8, `vsm_test` 5.4), neighbour words 4.9%, corners 4.7%, ring 3.5%, chains 2.7%.
- **c2:** the loop head (14.4%) and the 21-way switch (9.2%), mostly wait and no-instruction stalls; the shared visit gets only 4.7%.
- **CW:** the group OR (8 ballots per test) 24.1%, the warp-uniform march 15.7%, the warp-uniform texel 12.0%, the group walk 7.2%.

**Residency** (multi-pass; M):

| | Main | R2 | S2 | S3 | W2 | CW |
|---|---|---|---|---|---|---|
| Registers per thread | 168 | 168 | 168 | 168 | 168 | 168 |
| Resident warps per SM | 7.62 | 7.28 | 7.39 | 6.69 | 7.43 | 7.38 |
| Shared memory allocated per warp | 8,453 B | 8,454 B | 8,453 B | 6,148 B | 9,903 B | 8,451 B |
| L1TEX hit rate | 66% | 70% | 71% | 72% | 72% | — |

- **The limiter is not registers.** 168 registers would allow 12 warps per SM, and the register file is 61-63% allocated.
- **Nor is it the explicit tables:** S3 has less shared memory and fewer warps.
- **The limiter was not found.** Residency stays at about 7.5 warps whatever the tables, workgroup size or design. The kept changes raised IPC and cut issued instructions rather than raising residency.

## Where the time goes (M)

**Stage split** (R2, camera-moving, clean visits). The figures are the differences between full and `VSM_SKIP` / `VSM_FAR` visits:

| Stage | ms |
|---|---:|
| Page passes and dispatch | about 0.17 |
| Receiver setup | about 0.08 |
| Ring | about 0.45 |
| March and walks | about 1.51 (68%) |

The other split visits were contended.

**Per-wave work** (`VSM_DEBUG` trace of the traced quarry, pose 0, 346,517 receivers; `scripts/waves2.py`):

| Work | Total | Per covered pixel | Wave divergence (32 x slowest lane ÷ lane total) |
|---|---:|---:|---:|
| Ring triangle tests (own triangle included) | 1.22 M | 3.5 | 3.9 |
| Walk visits | 0.62 M | 1.8 | 5.2 |
| March candidates | 0.12 M | 0.33 | 5.1 |
| Ring and walk | 1.84 M | 5.3 | 3.8 |

- **Ring:** 19.7% of receivers walk their ring, 13.8 tests each.
- **Walks:** 12% of receivers walk, 15.0 visits on average (p90 27, p99 103).
- **By class:** lit 89.6%, umbra 8.7%, partial 1.7%. Umbra receivers make 72% of the walk visits.
- **Per working warp:** 6.15 lanes walk the ring and 3.77 walk in the march.
- **A cooperative ring** (each receiver's three corner fans as jobs for any lane of the wave) models at 0.43x the ring's wave cost; measured, it gained nothing on camera frames.

## Why the divergence designs lost (I, from the profiles above)

- **The visit is a minority of the work.** Fetch and tests are about 30% of main's samples; control, map reads and the march are the rest. Converging the visit alone can save at most about a fifth.
- **The state machine lost the nested loops' overlap.** In nested loops, lanes reconverge at every loop exit, so a lane's cheap slot iterations run alongside another lane's walk iterations in the same body. Resumable rounds serialize them: the round's control lasts as long as its longest lane's path to its next visit, and each sub-step pays for every resume point present. Refill desynchronizes lanes further, so bigger pools were slower. Register pressure cut residency from 7.7 to 3.0 warps per SM.
- **Cooperative walks did raise lane activity, from 8.3 to 16.0.** Every lane still marching is present at a walk only if the march's loops are warp-uniform. That scaffolding, plus one ballot per direction to OR a group's hits, cost 52% of CW's samples, more than the eight-way test split saved. WGSL has no clustered reduction; a one-ballot OR for 8-lane groups would remove most of the 24%, but the scaffolding alone (about 1.2 ms) exceeds P3's whole march-and-texel cost.
- **Breadth-first expansion was not built.** The 16-visit walk cap stops a receiver once a block is proven, so visit order is part of the output, and later candidates' masks, early exits and repeat filters depend on earlier results. Parallel candidates would need speculative walks plus an exact replay of each direction's first-hit visit. Umbra receivers, which do 72% of walk visits, would then walk candidates the sequential march never reaches.
- **The kept changes all shorten dependent chains:** local-memory tables, edge → neighbour → record, and descriptor and uniform reads per step. Changes that add loads or ALU to hide latency lost: speculative records +35%, set-bit loops +16% (they lose the unrolled tests' ILP), branch-free tests +10%, recomputed shears +21%.

## Walk storage A/B: M2's packed vertex stream (M)

`WalkRecords::Vertices` (built and removed; `variants/vertices.diff`) replaces the 36-byte per-source-triangle records with a 12-byte copy of each vertex position:
- read through the existing index buffer;
- packed into the same segment array (2^21 vertices, 24 MiB, per segment);
- built by a `vsm_vertex_record` pass;
- selected in the lookup by an override (`VSM_VERTEX_RECORDS`).

It covers appended LOD triangles too, which `Source` records do not.

| No-RT quarry, one process, interleaved | Camera | Static | Turning | Caster | Walk storage | Allocator reserved |
|---|---:|---:|---:|---:|---:|---:|
| P3, `Source` records | 1.962-1.979 | 0.569-0.576 | 0.570-0.588 | 1.879-1.919 | 447.5 MiB | 3,439.7-3,503.7 MiB |
| P3, packed vertices | 2.193-2.209 | 0.674-0.675 | 0.681-0.682 | 2.008-2.071 | **75.9 MiB** | **3,071.7 MiB** |
| P3, packed vertices + neighbour index prefetch (V2) | 2.349-2.353 | 0.704-0.737 | 0.726-0.728 | 2.094-2.161 | 75.9 MiB | — |

- **Byte-identical:** counts equal main's at both poses in every visit.
- **Slower, as the proposal warned.** A source triangle's step goes from one 36-byte record load to an index load, then three 12-byte gathers: one more dependent round trip. Divergence does not hide it. V2's speculative index loads cost more than they saved, as P4's did.
- **Not kept.** Even with it, camera motion would read 2.19-2.21 ms, still 8% under main's 2.38-2.41, at 368 MiB less. That trade is open for a future decision; this unit's condition was no regression.

## What was built and shipped (M)

`shaders/virtual_shadow.wgsl` only.
- **Per-lane workgroup tables.** `vsm_wg_dirs` and `vsm_wg_shear` are `array<array<f32,32>,24>`, [component][lane], read by `vsm_dir(j)` and `vsm_shear_at(j)`. They replace the private `vsm_dirs` and `vsm_shear` (6 KB of workgroup memory per one-warp workgroup). `vsm_lane` is set at the lookup's entry.
- **Constant-index reads.** `vsm_own_at(i)` and `vsm_last_at(j)` read `vsm_own` and `vsm_last` by constant indices only (select chains), so neither is runtime-indexed.
- **Neighbour prefetch.** `vsm_nbs` loads a triangle's three adjacency words together and `vsm_pick` selects one:
  - walk steps load them beside the step's record and choose their exit, including the fold fallback, from them;
  - ring fans load the next triangle's words with its record.
- **Hoists.** `vsm_adjacency(batch)` (the suffix base, or none) and `vsm_records_of(batch)` (first record, count) are read once per walk and per ring; `vsm_nb_at` and `vsm_tri_in` take them. `vsm_nb` is removed; `vsm_tri` and `vsm_neighbour` remain for setup, ring membership and the horizon pass.
- **Unchanged:** dispatch, bindings, uniforms, workgroup size and Rust. Main's shader file still runs on this binary.
- **Programmability is unchanged.** The tables hold directions and shears, geometry-only. No lighting-model data entered hit records, clusters, maps or the lookup, which still outputs the visibility counts that user shading programs consume.

## Quality and identity (M)

The shipped lookup is byte-identical to main everywhere this unit checked:
- **No-RT quarry at LOD 0.35 px:** counts `05e50a1d8ad17426…` (pose 0) and `bed48bf7099343bb…` (pose 1) in both final visits; hits `01dff017…`. First-frame hits equal settled hits, and the caster identity gate reports 0 differing pixels at both poses in both visits.
- **Traced quarry screen:** static `2d6e4fa6ca32…` and turned `88888216c1d9…`, with agreement against Interactive unchanged (2,157 flips of at least ½, 1 lit receiver shadowed, 1,438 umbra receivers lit).
- **RT device:** counts equal main's for traced primaries (`95ea47e97211bc02…` / `5b9d5d6b136b233b…`) and LOD primaries at both poses, in both visits.
- **Fixture:** counts `5917aada…` (15°) and `c5004c93…` (40°), as main.
- **S1, S2, S3:** rescored on the final binary with the unchanged scorers. 0 non-timing differences against M2's scores on main: S1's 792 fields, and 72 each in S2's and S3's `comparison.json` (partial and HDR RMS, contacts, leaks, flips, missing).

So the quality table is N8h's. Against Unreal VSM it is above on every measure except N8h's carried misses (below). Not film quality, and never presented as meeting the film gate.

## Memory against Unreal (M)

Unchanged: the shipped change adds 6 KB of workgroup memory per lookup workgroup and no buffers. The no-RT total stays M2's 3,439.7 MiB reserved (3,512 WDDM) against Unreal VSM's 3,912; RT 2,641 against 5,410. The rejected vertex stream would have cut no-RT to 3,071.7 MiB reserved.

## Misses, honestly

- **Camera motion:** 1.96-1.98 ms (no-RT) and 2.02-2.03 (traced) against VSM's estimated ~0.5 ms, about 4x. The no-RT frame with the camera moving is 2.68-2.73 ms against VSM's 1.61.
- **No divergence design gained.** Lanes active per warp stay at 8.5 of 32. The residency limiter at about 7.5 warps per SM was not identified.
- **Static** 0.56-0.57 ms against VSM's 0.50; turning is met (0.57-0.58 against about 0.95). The no-RT static frame is 1.28-1.31 ms, under 1.61.
- **S2's camera phases** are 0.48 ms against VSM's 0.35.
- **Caster motion:** 1.86-1.94 ms; VSM has no recorded row.
- **The resumable prototype at pool 1x1 with 256 control steps differs from main at pose 0** (counts `cd9b0c90…`, hits equal; pose 1 equal). Every other configuration of the same program was identical: pools 2x2, 4x1 and 4x4, pool 1x1 with one control step. Raising its round guard to 2^22 did not change it. Not isolated, since that design is rejected.
- **Carried from N8e-N8i:**
  - S1 pose 1 has 2 umbra leaks against VSM's 0.
  - The quarry's lit-darkened margin stays thin (55-60 against 87-88).

## Evidence and time

- **Evidence** (`E:/renderlab-work/n8l/`):
  - `p*-t-*`, `final-t-*`, `final-tt-*`: timing, hashes and memory reports per lease. Summarize them with `scripts/summ.py`.
  - `p*-fix-*`, `final-fix`: fixture counts. `final-screen-*`: traced screens. `final-scores`: S1-S3 runs and scores.
  - `p2-*-rt`, `p4-*-mp`, `p5-*-mp`, `p6-*`, `p11-*`, `p16-*`, `final-fin-rt`: trace exports (`analysis/`, `lines.json`, `mp.json`); trace files deleted after export.
  - `p7-diag/waves2-pose0.json`: the work model.
  - `variants/`: every screened shader; `candidate.diff` (the resumable lookup); `vertices.diff` (the vertex stream, Rust and WGSL).
  - `edits/`: the generator scripts. `scripts/`: summ, waves2, lines, and N8i's trace scripts.
  - `counts-hashes.txt`, `final-binaries.sha256`, `runs/`: every lease's log and preflight.

  Deleted after hashing, scoring and export, and listed in `deleted-raw.txt`: raw dumps over 5 MB (trace files, the debug trace, fixture hit buffers) and this run's scored S1-S3 frames, 1,489 files in all. The S2/S3 references hardlinked from N7a, JSON/CSV summaries and PNGs are kept.
- **Build and environment.** Development/test profile, RTX 4070 SUPER, NVIDIA 617.14, Vulkan. Validation was off for timing and traces, and on for the traced screen and S1-S3.
- **Gate.** `cargo test -p renderlab --lib -j 2` exited 0 inside the final lease, on the committed source: 106 passed, 0 failed, 110 ignored. Clippy (`-D warnings -A clippy::nonminimal_bool -A clippy::items_after_test_module`) passes on renderlab and renderlab-viewer, and fmt passes.
- **Contention.** `mypaper-wallpaper` and N8k's leases interleaved, so contended visits (primaries 0.46-1.1 ms) are excluded. No game ran, and no window, focus change or capture was used.
- **Leases.** 21 under the shared lock with the 4/2 GiB guard, 3,248 s in total, each taken after the other worker's release lines; at least 12.8 GiB was available at every start.
  - p1's candidate process failed on the screen-constant whitelist.
  - final-a (200 s) and final-b (198 s) ran the final batch.
- **Time (UTC).**
  - 01:00-01:20: reading.
  - 01:20-01:37: the resumable lookup and its leases.
  - 01:37-02:25: profiles, stage split, register and shared-memory cuts.
  - 02:25-03:15: work model, cooperative ring, set-bit loops and the prefetch series.
  - 03:15-03:35: cooperative walks.
  - 03:35-03:50: the vertex-stream A/B.
  - 03:50-04:10: final leases, scoring and this record.

  GPU leases total 54 minutes; implementation and analysis took the rest.
