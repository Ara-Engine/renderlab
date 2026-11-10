# P-VIA: US 8,471,853 claim mapping

2026-11-10. Documentation only; no code changes, builds, Cargo, GPU work, or image tests.
Code baseline: `41bf406781906163f2d85df976984d863bd15283`. Branch: `t3/p-via`.

**Verdict: independent claim 1 is AT RISK for the shipping Surface lookup's cache-miss march.**
It explicitly reads triangle vertices, calculates signed barycentric subareas at a light-space aim,
uses their signs to locate a triangle, reconstructs its height by weighted interpolation, and
compares that height with the ray height. There is no dependable missing computational element.
The unresolved issue is whether a march sample and its relationship to the original testing pixel
satisfy the claim's light-view pixel/occluder relationship. This is an engineering claim chart,
not an adjudication of infringement or a freedom-to-operate opinion.

## Sources, status, ownership and maintenance

Primary granted text: [Google Patents US8471853B2, claims 1–9](https://patents.google.com/patent/US8471853B2/en#C1).
Read the actual claims, including the nested consistency-test limitations, rather than the abstract.
There are **nine claims and exactly one independent claim (1)**; claims 2–9 each depend directly on 1.
The design note's four-element summary is incomplete without those nested limitations.

The same page's Info, timeline and Legal Events record:

| Item | Evidence / result |
|---|---|
| Title | Reconstructable geometry shadow mapping method |
| Application / priority | US12/287,546; provisional priority 2007-10-26; nonprovisional filed 2008-10-10 |
| Grant | 2013-06-25 |
| Status / adjusted expiry | **Active; 2032-04-25**, as reported by Google Patents |
| Original and current listed assignee | VIA Technologies Inc. |
| Recorded assignment | Recorded 2008-10-10, effective 2008-10-08; Dai and Yang to VIA; reel/frame 021749/0440 |
| 4th-year fee | FPAY, 2016-12-08 |
| 8th-year fee | MAFP/M1552, 2020-09-21; large entity |
| 12th-year fee | MAFP/M1553, **2024-12-06**; large entity |
| Lapse | No lapse or reinstatement event appears in the retrieved Legal Events table. All three maintenance installments are recorded paid. **Do not treat this patent as abandoned for unpaid fees.** |

These are database-reported facts, not a certified USPTO status/ownership search. Google expressly
qualifies its status, assignee and expiry fields. The recorded payments support treating the patent
as maintained; this work did not independently inspect USPTO payment receipts, prosecution history,
terminal disclaimers, assignment instruments, or later unindexed events.

Family check: the parent page identifies one US divisional, application US13/559,223,
[US8605088B2](https://patents.google.com/patent/US8605088B2/en), titled Method for reconstructing
geometry mapping, granted 2013-12-10 and listed Active. Attempts to fetch its own granted page
failed (web tool Internal Error; shell fetch returned Google's automated-query rejection).
**Its claims and maintenance remain unreviewed: clearing 8,471,853 alone would not clear this family.**
The parent also links provisional US60/982,727 and Taiwan application 97118693/TWI417808B;
neither is treated as an additional granted US claim here.

The parent was successfully fetched through the web tool during this unit. Direct shell fetching
was rejected; for machine-readable claim inspection, reused the same-day primary HTML already saved
by P-S4. Raw HTML and extracted independent claim are preserved outside the repo at
`D:/Coding/renderlab-work/p-via/{raw,claims}/US8471853B2.*`.
The failed child fetch is not usable claim evidence. No third-party patent commentary was used.

## Granted independent claim 1

Transcription of its words, with mathematical notation compacted below; the original linked claim
and saved HTML remain authoritative:

> 1. A reconstructable geometry mapping method comprising:
>
> extracting geometry information of a plurality of occluding geometry shapes of an object's front-face with respect to a light source's point of view;
>
> performing a consistency test on a testing pixel so as to determine an occluding geometry shape corresponding to the testing pixel from the object's front-face among the plurality of occluding geometry shapes;
>
> reconstructing a depth value of an occluding point corresponding to the testing pixel; and
>
> performing a shadow determination of the testing pixel,
>
> wherein coordinates of the testing pixel are (p.x, p.y, p.z), and the consistency test comprises:
>
> selecting one of the geometry shapes;
>
> reading geometry information of the selected geometry shape, the geometry information including vertex coordinates (v0.x, v0.y, v0.z), (v1.x, v1.y, v1.z), and (v2.x, v2.y, v2.z) of the selected geometry shape;
>
> obtaining coordinate weight values (w1, w2, w3) of the occluding point by solving an equation of

```text
[p.x p.y 1] = [w1 w2 w3] M
M = [[v0.x v0.y 1], [v1.x v1.y 1], [v2.x v2.y 1]]
```

> determining whether the selected geometry shape is consistent according to the coordinate weight values (w1, w2, w3) of the occluding point; and
>
> determining the selected geometry shape as the occluding geometry shape-when the selected geometry shape is determined to be consistent.

The raw claim MathML contains v1.y and v2.y in those rows; the web tool's flattened equation
incorrectly repeats v0.y. This chart uses the claim-section MathML, not that flattened rendering
or the description's similar equation. A matrix-inverse instruction is **not** required by claim 1.
Unnormalized subareas whose common denominator is applied later solve the same barycentric relation.

## Which code actually ships

`VirtualLookup::Surface` is the default (`src/virtual_shadow_gpu.rs:218`). Surface is **not** the
map-only gradient lookup in the similarly named `virtual_shadow_lookup.wgsl`:

1. `src/virtual_shadow_cache.rs:32`–44 appends the surface cache shader to `shader_source()`.
2. `src/virtual_shadow_gpu.rs:587`–600 supplies `virtual_shadow.wgsl` as that base module.
3. Cache queue processing calls `vsm_prepare` and **`vsm_walk_batches`**
   (`shaders/virtual_shadow_cache.wgsl:422`–427).
4. `vsm_walk_batches` calls `vsm_march` for remaining directions
   (`shaders/virtual_shadow.wgsl:1429`–1437); the march's texel-candidate routine calls
   `vsm_top_hits` (`:1170`–1223).

Cached counts can skip this work on later frames; they do not remove the computation used to produce
those counts. A warm-cache frame alone is therefore not a design-around.

Rust prepares the light basis (`src/virtual_shadow_gpu.rs:688`–697, `:1554`), transforms placement
bounds into light-space xy and height ranges (`:1556`–1607), and writes the basis, range, texel,
march settings and primitive-ID bits into the frame uniform (`:1727`–1756). Rust does **not**
construct a CPU per-texel triangle map. GPU cluster rasterization transforms geometry into that
basis (`shaders/virtual_shadow.wgsl:341`–347), rasterizes triangles (`:368`–388), and writes
top/peel `(height, triangle ID)` maps (`:318`–338).

## Claim 1 element chart

“Present” means the operation exists in a reachable shipping path, not that every sample executes
every step. “At risk” marks a claim-construction/relationship question rather than a missing opcode.

| Required element | Code has it? | File:line evidence and qualification |
|---|---|---|
| Reconstructable geometry mapping method | Present | Light map generation at `shaders/virtual_shadow.wgsl:318`–388; lookup at `:996`–1085. |
| Extract geometry information of plural occluding shapes from an object's front-face relative to a light | Present for light-visible surfaces; terminology at risk | `:341`–347 transforms triangle vertices; `:324`–331 retains maximum sunward height and geometry ID. SW raster accepts both windings (`:371`–374); HW culling is disabled (`src/virtual_shadow_gpu.rs:1046`). Processing additional back-facing surfaces does not exclude processing the claimed front-facing/visible surfaces. |
| Consistency test on a testing pixel to determine the corresponding front-face occluder | Computation present; pixel relationship at risk | `:1005`–1006 sets aim and ray height from march sample q; `:1030`–1044 locates the shape under aim.xy. q is displaced along a sampled sun-disk ray, rather than necessarily the original receiver's central-light xy. |
| Reconstruct depth of occluding point corresponding to testing pixel | Present at the aim; same relationship at risk | `:1039` reconstructs height at aim.xy from the triangle's light-space vertex heights. Height toward sun is an affine/reversed depth convention, not an absent depth. |
| Perform shadow determination of testing pixel | Present | Exact hits at `:1028`; interpolated height influences refinement and continuity at `:1075`–1084; result becomes blocked directions at `:1223`–1235 and a pixel count at `:1437`, `:1444`–1446. |
| Testing pixel has coordinates (p.x,p.y,p.z) | Present, with identification caveat | Receiver preparation feeds the march; `:1005`–1006 uses q.xy/q.z. `vsm_light`, `:87`–90, provides 3-D light coordinates. Identifying claim p with q is the disputed step. |
| Select one geometry shape | Present | Candidate ID selected from top/peel map at `:1175`, `:1181`–1196; `:1002` decodes primitive, `:1048`–1058 walks to neighbors. |
| Read selected shape's three vertex coordinates | Present | `:1025`–1027 reads/transforms three vertices; `vsm_tri`/record access at `:473`–480 and index/vertex fallback at `:442`–446. Storing vertices in packed walk records still reads vertex coordinates. |
| Obtain coordinate weights by solving the stated relation | Present algebraically | `:1031` computes projected area; `:1034`–1036 computes three signed subareas. Normalized weights are `w/(w.x+w.y+w.z)`, applied in `:1039`. Separate inverse-matrix evaluation and named normalized-weight variables are unnecessary. |
| Decide consistency according to the weights | Present | `:1037` tests all three signed weights nonnegative. For a nondegenerate triangle they sum to its positive projected area, so normalization gives weights in [0,1]. |
| Determine consistent selected shape as occluder | Present as located candidate; actual blocking conditional | `:1038` sets located, `:1039` reconstructs its height, `:1044` stops the point-location walk. Own-surface/interior exclusions at `:1075` limit which candidates affect the result; they do not eliminate the operation for other surfaces. |

**Independent claim 1 verdict: AT RISK**, not clear. Its arithmetic and data-access elements are
all present. The best scope distinction is that this point-location works on displaced march samples,
and exact ray/triangle hits often decide the shadow before height reconstruction; however, some
reachable cases use reconstructed height for the continuity verdict. The method is open-ended
(“comprising”): extra exact tests, caching, soft-shadow batches and additional backfaces are not
reliable exclusions. No prosecution-history analysis was performed to resolve “testing pixel”,
“front-face”, indirect contribution to the determination, or equivalence. Accordingly this chart
does not escalate the engineering finding to a definitive “reads on” judgment.

## All dependent claims that matter (2–9)

These are the complete set of dependents, not additional independent claims. Each inherits **all**
of claim 1, including the vertex read and weight-based consistency test. The following records their
additional language/requirements and code mapping; equations use M above and Z = [v0.z v1.z v2.z]^T.
See the linked primary claims for original mathematical typography.

| Claim | Additional granted requirement | Mapping and result (subject to claim 1) |
|---|---|---|
| 2 | “the occluding geometry shapes are triangles” | Present: `virtual_shadow.wgsl:1025`–1036. At risk with 1. |
| 3 | Geometry information comprises vertex coordinates or geometry indices of the occluding shapes | Present: map IDs `:338`, vertex reads `:1025`–1027. At risk with 1. |
| 4 | Reconstruct T.z by `T.z = [w1,w2,w3] Z` | Present algebraically: `:1039` divides weighted height by the subarea sum. At risk with 1. |
| 5 | Reconstruct by `[w1,w2,w3] M M^-1 Z`, rewritten as `[p.x p.y 1] M^-1 Z` | Same affine interpolation is present mathematically at `:1031`–1039; no explicit matrix inversion. At risk under an algebraic reading; no clearance based solely on avoiding an inverse instruction. |
| 6 | Reconstruct by `v0.z + (dz/dx)(p.x-v0.x) + (dz/dy)(p.y-v0.y)`, with an equivalent 2x2 inverse edge-matrix expression for the gradient | Exact march uses equivalent affine interpolation `:1039`. Map-only path directly uses gradient reconstruction: `virtual_shadow_lookup.wgsl:85`–101, `:469`. That map-only resemblance alone does not satisfy inherited claim 1. At risk for march; not a separate broad plane-only independent claim. |
| 7 | Occluding shape comprises the occluding point and testing pixel overlaps it when viewed from the light | Inside test/located point at `virtual_shadow.wgsl:1037`–1039. Present for aim; unresolved for original receiver versus displaced aim. At risk. |
| 8 | Reconstruct depth according to shape geometry and testing-pixel position data | Triangle light vertices and aim.xy at `:1030`–1039. Present with same pixel caveat. At risk. |
| 9 | Shadow determination achieved by comparing occluding-point depth with testing-pixel depth | `:1076`–1084 compares height-ray and derives continuity; `:1227`, `:1232` turns continuity into blocking. Exact hits also contribute. At risk: indirect/refinement use is not a reliable missing element. |

The map-only lookup reads per-texel gradients/heights (`virtual_shadow_lookup.wgsl:454`–469)
and compares them (`:475`–480). Its runtime cover test does not read three occluder vertices or
solve triangle barycentric consistency. Its bilinear weights (`:452`) are weights of four texels,
not the claim's three triangle-coordinate weights. The preprocessing pass does read vertices
(`:92`–98); that preprocessing is not itself the receiver's consistency test. This is a useful
literal distinction, but **switching the default to this approximation is not authorized by this
research and would require the existing final-image/motion quality gates**.

## Concrete design-arounds and costs

No implementation or measured timing/identity claim is made here. The following costs are structural
estimates. Patent avoidance must preserve the Unreal quality floor and programmable shading under
AGENTS.md; candidates must fail the image gates measurably before invoking the policy's patented-method
fallback. That fallback would need the patent number, failed alternatives and a named setting.

| Candidate | Required element deliberately dropped | Byte identity and cost |
|---|---|---|
| **Replace the complete march with S4 BVH gather plus boolean 3-D half-space segment/triangle tests** | Remove light-view point-location consistency and its coordinate-weight solve; compute only segment occupancy using a triangle plane and oriented edge planes, no reconstructed light-space occluder depth. Keep candidate records as planes, not vertex triplets in the receiver query. | Preserve receiver origins, direction sequence, epsilon, active masks, count and cache interface. Should agree with exact triangle visibility apart from numerical boundaries, but **cannot promise march-byte identity**: truncation/continuity/hidden layers are different by design. BVH and plane-record memory, traversal, ALU, and robust-boundary engineering cost. S4's quarry full-detail memory issue still applies. A generic barycentric watertight routine instead of this boolean test weakens the claimed weight-element break. Re-chart final implementation and the divisional. |
| **Precompute planar region records and use a partition/BSP query** | Receiver consistency never reads the selected triangle's three vertex coordinates or obtains three coordinate weights. Build a light-plane partition offline/on caster or sun changes; leaves contain height planes, ownership and adjacency, query performs region/edge predicates only. | Could retain the march schedule, candidate order, own/interior rules and continuity behavior. This is the closest candidate to preserving existing decisions, but precomputed f32 plane coefficients change rounding: strict byte identity is **unproven**. Preserve ambiguous boundaries via robust predicates, not by falling back to the old vertex/barycentric query. Extra build/update work and storage; awkward for moving geometry. Equivalence risk remains because edge predicates implement the same geometric inclusion; moving the same barycentric test to another buffer is insufficient. Whole-method preprocessing must also be re-charted. |
| **Per-direction geometric shadow volumes / stencil occupancy** | No per-pixel geometry-shadow-map consistency test, no barycentric weights, no reconstructed occluder depth. Extrude caster silhouettes for each sampled light direction and count volume occupancy using raster/stencil operations. | Same ideal binary visibility for opaque closed casters and same sampling/outputs; not byte-identical to current heuristic march. Costs scale with direction count, silhouette generation, fill/overdraw, caps and robust near-plane handling; thin/open/alpha geometry needs explicit treatment. This drops more of the combination, but needs its own patent-family search and quality/performance screen. |
| **Map-only depth/gradient cover lookup with no exact-march fallback** | Remove runtime reading of selected triangle vertices and the triangle-weight consistency solve (literal claim 1 limitations). Existing `vmap_cover` is a concrete starting point. | Lowest implementation scope, but **not** a proven same-image replacement. Plane filtering and map resolution already change coverage/occluder decisions. Costs become map resolution, storage and quality recovery; retain no old-march fallback in purportedly cleared configurations. Not acceptable as a quality reduction. |

Do not count the following as design-arounds: renaming w to edge areas; postponing their common
denominator; replacing matrix inversion with cross products; changing buffers/CPU versus GPU; storing
indices rather than vertices in the map; changing height sign; disabling just the continuity bit;
or moving only cut frames to S4 while retaining the march on moving/still frames. They respectively
retain claimed operations, leave reconstructed height driving traversal, or leave the exposed path
reachable. Merely removing `height=...` without reviewing `vsm_hits_sheared` (`:671`–689) is also weak:
that helper computes three edge quantities and a weighted intersection parameter.

**Recommendation:** retain the flag as open/at risk, pursue S4's full replacement with an explicit
non-barycentric boolean intersection contract, and inspect US8605088B2 before claiming family clearance.
For minimum image disruption, screen the planar-region option first, while recording its weaker
equivalence position and its unproven bit identity. No current evidence establishes a free, universally
byte-identical design-around.

## Verification and scope

Read AGENTS.md's patent policy, both requested memory notes, the S4 patent section, and the shader/Rust
call chain above. Confirmed one independent claim and all eight dependents from primary HTML and
cross-checked status, assignment and fee events with the freshly retrieved Google Patents page.
Source inspected at the baseline above; P-S3Q's concurrent lookup changes require re-charting if
they alter these operations. Only this report and one P-S4 gap-status line are changed.
No build, Cargo command, GPU/window work, benchmark or image capture was performed.
