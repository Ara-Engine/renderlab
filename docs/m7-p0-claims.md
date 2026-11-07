# M7-P0: patent claim mapping for the M7 design

Unit P0 of [m7-design.md](m7-design.md) (draft 2.1), 2026-11-07, base main `1be068d`. CPU, web and docs only: no GPU work, no source changes.

> **Not legal advice and not a clearance.** This is an engineering reading of granted and published claim text, written by an agent. It maps M7's planned parts onto claim elements so that the design can drop elements on purpose. It does not construe claims the way a court would, assess validity or prior art, or address the doctrine of equivalents. "Clear" below means only that a component appears to lack at least one element of every independent claim read. Islam can commission a legal review at any time ([milestone-7.md](milestone-7.md), patent policy).

## Summary

**53 US documents read** (granted patents and published applications). That covers every patent the research notes flagged, plus their continuations, divisionals and CIP parents, plus nine NVIDIA hash-cache filings that the notes did not list. Section 2 has the inventory.

**The main finding is that the WRC's real exposure is not the one the design names.**
- **US 9,754,407** is about per-triangle object-space *shading grids*. Neither the WRC nor the surface shadow cache has its grid elements.
- **US 10,614,613** (NVIDIA path-space filtering by hashing, in force to about 2038) has an independent claim that reads on a hashed cell cache which accumulates samples with a counter and averages them.
- **US 11,790,594** (NVIDIA irradiance caches, to about 2041) reads on casting cache-update rays from locations ranked by characteristics such as age, under a ray budget.

As drafted, the WRC appears to practise every element of both. Each has a simple design-around (section 6, W1-W2).

| Component | Verdict | Binding claim(s) | What keeps it clear, or the change needed |
|---|---|---|---|
| World radiance cache (WRC), as drafted | **At risk** | US 10,614,613 cl. 1; US 11,790,594 cl. 1 | **W1:** no per-cell sample counter or running mean; replace or fixed-weight values instead. **W2:** no ranking of cells or query points to decide which rays are cast |
| WRC with W1-W3 | Clear with constraints | US 11,315,310 cl. 1; US 12,423,782 cl. 1; US 11,756,254 cl. 1 | No reservoir or RIS anywhere that reads the WRC. Deterministic lookups (W3). Cubic cells |
| Fixed-direction fill | Clear | US 11,315,310; US 2025/0182387 (pending) | Stratified fixed directions; no resampling; no sampling of points by light value |
| TR1 (revived T1b) | Clear with constraint | US 10,600,167 cl. 1 | Practises (a1)-(a4). Clear only while no later filter consumes TR1's per-pixel auxiliary data (W5) |
| Own-pixel temporal moments | **At risk** | pending US 2026/0162231 cl. 1 (every element as published); thin against US 12,437,371 cl. 1 and US 11,847,737 cl. 1 | **W4:** drop it, or keep it behind a named setting, off by default, until the continuation's claims settle |
| Edge-aware à-trous | Clear with constraints; one thin point | US 10,600,167; US 11,113,792; US 12,423,782; US 11,688,042 cl. 9 (thin) | Non-separable 2D taps, fixed size and iteration count, fixed tap pattern, no inputs from any reprojecting stage (W6) |
| Single history | Clear (one ambiguity) | US 11,847,737 cl. 1 (d); history-clamp family | One value per pixel or cell, with no fast history and no retained list of previous values |
| Recompute on light change | Clear | US 12,437,371; US 2026/0162231; US 11,847,737 | Driven by engine events; no lighting-difference estimate |
| Traced-primary RT reflections | Clear with constraint | US 12,051,145 cl. 1; US 11,315,310 cl. 1 | Per-pixel coupled rays, no common projection, no matching step. No RIS, because hits read the WRC |
| Planar mirrors | **Thin** | US 10,565,776 cl. 1; US 10,789,759 cl. 1 | Lacks only step (g), repeated deviated projections. **W7:** one render per plane per frame, glossy only by prefiltering, planar geometry only |
| Fixed sub-cone glossy rays | Clear | US 12,051,145; US 11,315,310 | Rays start at their own pixel's hit; deterministic; no resampling |
| R1n LOD-BVH tier | Clear | US 12,051,145; US 10,380,785 | Per-pixel independent rays, global-first traversal, no per-cell hit registration. A cook-time BVH is *not* the operative avoidance (section 7, item 4) |
| SDF-brick tier | Clear with constraint | US 10,380,785 cl. 1 | Never register hits per brick or launch rays per brick (W8) |
| Existing surface shadow cache (N8o) | Clear | US 10,614,613 cl. 1 (e)-(g); US 9,754,407 | Stores a consensus-certified verdict, not an accumulated average. Cube cells, not triangle grids |

**Patented fallbacks confirmed.** Each fallback in m7-design §9.1 practises the claim it was flagged for (section 5): `GiSampling::Restir`, `Reconstruction::VarianceClip`, `Denoise::Svgf`, `Denoise::FastHistory`, `Reconstruction::GradientReshade`, `NonRtReflections::CoherentHybrid` and `ReflectionSampling::Restir`.

**Corrections to m7-design §9.1** are listed in section 7. The largest are:
- US 10,600,167's claim 1 has no variance element.
- No Adshir/Snap independent claim recites a dynamic acceleration structure.
- US 11,847,737's claim 1 has no dual-history element.
- A plain min/max colour box avoids US 10,362,289.

Labels used: **verbatim** text is in quotation marks. Element letters are this note's own. Every mapping is an inference (I).

## 1. Method and limits

**Retrieval.**
- Each document's Google Patents page (`patents.google.com/patent/<number>/en`) was fetched on 2026-11-07 into `E:/renderlab-work/p0-patents/raw`.
- `scripts/extract_claims.py` (stdlib Python, run with `-I`) split the claims section at each claim id. A claim counts as independent when its text has no `<claim-ref>`.
- For every document, the independent-claim count equals the claim total minus the `claim-dependent` blocks. Claim text in `out/` was read from that extraction.
- Specs were searched with `scripts/dump_claims.py` only to read ambiguous terms ("temporal lag", "auxiliary information", "filter direction", "DAS", "phantom camera", "ranking").

**Leads.**
- The numbers came from [docs/research/](research/) and m7-design §9.1.
- Each page's "Priority and related applications" table gave continuations, divisionals and CIP parents, one level out from each flagged patent.
- Three web searches turned up the hash-cache patents in section 3.A, which no research note had listed.

**Status and expiry** are Google Patents' legal-status estimate ("Active", "Pending", "Anticipated/Adjusted expiration"). Nothing was checked against USPTO Patent Center: no maintenance-fee or terminal-disclaimer check was done. Pending applications' claims can change before grant.

**Scope.**
- US documents only. Foreign counterparts (DE, CN, EP, JP) were not read.
- Not searched: Epic, AMD (Brixelizer, GI-1.0 hash grid), Unity, Microsoft or Intel filings, apart from the two Intel/NVIDIA items in section 8.

## 2. Documents read

"Exp." is Google's estimate. Ind. = independent claims.

| Area | Document | Assignee | Status | Priority | Exp. | Ind. | Title (short) |
|---|---|---|---|---|---|---|---|
| GI cache | **US 10,614,613 B2** | NVIDIA | Active | 2017-07-28 | 2038-07-25 | 1, 22, 23 | Parallel path space filtering utilizing hashing |
| GI cache | **US 11,790,594 B2** | NVIDIA | Active | 2019-08-18 | 2041-10-17 | 1, 10, 16 | Ray-tracing with irradiance caches |
| GI cache | US 2024/0029338 A1 | NVIDIA | Pending | 2019-08-18 | — | 1, 10, 15 | Ray-tracing with irradiance caches (continuation) |
| GI cache | **US 11,315,310 B2** | NVIDIA | Active | 2020-03-12 | 2041-01-19 | 1, 12, 21 | Reservoir-based ... resampling utilizing a GI data structure |
| GI cache | US 2025/0182387 A1 | NVIDIA | Pending | 2020-03-12 | — | 1 | Same title (continuation; US 2022/0198746 abandoned) |
| GI cache | **US 9,754,407 B2** | NVIDIA | Active | 2014-08-12 | 2035-04-30 | 1, 17, 20 | Shading using a dynamic object-space grid |
| GI cache | US 9,747,718 B2 | NVIDIA | Active | 2014-08-12 | 2035-04-17 | 1, 19, 20 | Performing object-space shading |
| GI cache | US 11,925,860 B2 | NVIDIA | Active | 2021-06-09 | 2042-09-08 | 1, 15, 22 | Projective hash maps |
| GI cache | US 11,756,254 B2; US 12,374,024 B2 | NVIDIA | Active | 2020-12-08 | 2040-12-08 | 1, 10, 18; 1, 10, 16 | Light importance caching using spatial hashing |
| GI cache | US 12,456,261 B2 | NVIDIA | Active | 2022-03-21 | 2044-01-04 | 1, 8, 15 | Hash cell boundary shifting |
| GI cache | US 11,609,899 B2; US 12,013,844 B2; US 12,360,985 B2 | NVIDIA | Active | 2020-07-16 | 2040 | 1, 11, 16 each | Concurrent hash map / dataset updates |
| Light sampling | US 11,663,773 B2; US 12,361,633 B2; US 2025/0259377 A1 (pending) | NVIDIA | Active / Pending | 2020-08-21 | 2041-02-24 | 1, 9, 17; 1, 7, 14; 1, 8, 14 | Importance resampling for light-sampling memory incoherence |
| Light sampling | US 11,948,246 B2; US 12,646,247 B2 | NVIDIA | Active | 2021-04-05 | 2042 | 1, 8, 16; 1, 8, 15 | Spatiotemporal resampling with decoupled shading and reuse |
| Light sampling | US 12,361,635 B1; US 2025/0299434 A1 (pending) | NVIDIA | Active / Pending | 2022-07-22 | 2043-11-01 | 1, 11, 17 each | Initial candidates in spatiotemporal resampling |
| Light sampling | US 12,299,801 B2; US 2025/0278884 A1 (pending) | NVIDIA | Active / Pending | 2020-08-21 | 2040-09-24 | 1, 9, 17; 1, 8, 15 | Grid-based light sampling |
| Temporal | **US 10,116,916 B2** | NVIDIA | Active | 2016-03-17 | 2037-04-04 | 1, 8, 10, 11, 15, 16, 18 | Data reuse ... spatio-temporal supersampling (variance clip) |
| Temporal | US 10,362,289 B2 | NVIDIA | Active | 2016-03-17 | 2037-03-07 | 1, 12, 20 | Same (continuation) |
| Temporal | US 11,216,916 B1; US 11,232,544 B1; US 11,783,455 B2; US 12,299,855 B2; US 2026/0051032 A1 (pending) | NVIDIA | Active / Pending | 2020-10-09 | 2040-10-09 | see 3.C | History clamping with temporal accumulation |
| Temporal | US 11,847,737 B2; US 12,536,735 B2 (divisional) | NVIDIA | Active | 2021-04-09 | 2041-04-09 | 1, 11, 14; 1, 12, 19 | Temporal denoiser quality in dynamic scenes |
| Temporal | **US 12,437,371 B2**; US 2026/0162231 A1 (pending) | NVIDIA | Active / Pending | 2021-10-29 | 2043-07-21 | 1, 11, 17; 1 | Illumination resampling using temporal gradients |
| Denoise | **US 10,600,167 B2** | NVIDIA | Active | 2017-01-18 | 2038-07-07 | 1, 15, 18 | Performing spatiotemporal filtering (SVGF) |
| Denoise | US 11,113,792 B2; US 11,688,042 B2; US 12,423,782 B2 | NVIDIA | Active | 2018-08-14 | 2039-08-14; 2040-01-14; 2040-03-11 | 1, 11, 15; 1, 9, 15; 1, 10, 16 | Temporal-spatial denoising; multi-iteration filter direction; jittered filter taps |
| Reflections | US 10,699,468 B2; **US 10,950,030 B2**; US 11,302,058 B2; **US 12,051,145 B2**; US 12,573,127 B2; US 2025/0363721 A1 (pending) | Snap (orig. Adshir) | Active / Pending | 2018-06-09 | 2038 | see 3.E | Non-planar specular reflections in hybrid ray tracing |
| Reflections | US 10,565,776 B2; US 10,789,759 B2 | Snap | Active | 2015-12-12 | 2036-12-12 | 1 | Path traced reflections on a semi-reflective surface (CIP parent) |
| Reflections | US 10,614,612 B2; US 10,930,053 B2 | Snap | Active | 2018-06-09; 2016-01-28 | 2038-06-09; 2036-02-09 | 1 | Reflections for augmented reality (CIP parent) |
| Reflections | US 10,614,614 B2; US 10,380,785 B2; US 10,818,072 B2 | Snap | Active | 2015-09-29 | 2035-2036 | 1 | Distributed accelerating structures (CIP parent) |
| Reflections | US 10,410,401 B1; US 10,332,304 B1 | Snap | Active | 2017-07-26; 2015-12-12 | 2037; 2036 | 1, 8; 1 | Spawning secondary rays from non-primary rays; fast intersections |

## 3. Independent claim elements

The system and storage-medium claims recite the same elements as the method claim unless noted.

### 3.A GI caches and world-space structures

**US 10,614,613, claim 1** (22 and 23 the same). Verbatim:
- (a) "selecting a vertex of a light transport path;"
- (b) "quantizing one or more features of the selected vertex;"
- (c) "calculating a first hash based on the one or more quantized features of the selected vertex;"
- (d) "performing a collision resolution within a hash table;"
- (e) "accumulating a contribution of the light transport path at the selected vertex to the hash table;"
- (f) "incrementing a counter in response to adding the contribution of the light transport path at the selected vertex to the hash table; and"
- (g) "calculating an average contribution of the light transport path, utilizing the counter and the hash table."

Dependents of note:
- 2: the vertex is jittered before quantizing.
- 5: quantization by distance from the camera.
- 10-11: an exponential moving average kept with the sum and counter, or made cumulative.
- 19: contributions with the same hash are accumulated together.

The spec notes the counter "may include an atomic counter".

**US 11,790,594, claim 1.** Close paraphrase:
- (a) ranking a first location against a second location based on characteristics of both. Each location "being associated with one or more irradiance caches to store irradiance that corresponds to the location".
- (b) "casting a ray from the at least one first location based at least on the at least one first location having a higher ranking";
- (c) determining an irradiance value for the ray's interaction;
- (d) rendering a frame using the first location's cached irradiance and that value.

Claim 10 does the same with groups of geometry: rank, cast rays from the higher-ranked group, update its caches, render. Dependents of note:
- 3 and 11: a ray budget.
- 4: locations are mesh vertices.
- 7-9 and 12-14: characteristics are camera distance, camera visibility, or "a number of frames since a prior update".
- 15: cache values used for disoccluded pixels in a denoiser.

**US 2024/0029338 A1 (pending), claim 1.**
- (a) samples of lighting for a first pixel;
- (b) cache values for a second pixel, chosen "based at least on a quantity of samples of the lighting conditions available for the at least one second pixel";
- (c) a denoising filter over both;
- (d) render.

Claim 10 replaces (b) with detecting a disocclusion at the second pixel.

**US 11,315,310, claim 1** (12 and 21 the same). Verbatim, one sentence: "implementing reservoir importance resampling to compute global illumination on visible surfaces in a scene, utilizing a world-space data structure." Elements:
- (a) reservoir importance resampling;
- (b) GI on visible surfaces;
- (c) utilizing a world-space data structure.

The spec's list for (c) includes "a photon map, a light map, a radiance cache, an irradiance cache, a radiosity value data structure, a light probe data structure".

**US 2025/0182387 A1 (pending), claim 1.**
- (a) light values for points on both light sources and geometry;
- (b) "in a single step", sampling a point with probability proportional to its light value;
- (c) shading by a shadow ray to the sampled point.

There is no reservoir element and no world-space structure element.

**US 9,754,407, claim 1** (17 and 20 the same).
- (a) an object of triangle primitives in an object-specific 3D space;
- (b) an object-space shading grid for at least one triangle, "based on coordinates at vertices of the at least one triangle primitive";
- (c) "a shape of an object-space shading element corresponds to a shape of the at least one triangle primitive";
- (d) a shader program computes "a shaded value at a point on the object-space shading grid".

**US 9,747,718, claim 1** (19 and 20 the same).
- (a) a primitive in object-specific space;
- (b) "computing a shading sample rate for the primitive";
- (c) a primitive-specific grid with triangle-shaped elements, its points set by the rate;
- (d) a shader program samples the grid for shaded attributes.

**US 11,925,860, claim 1** (15 and 22 the same).
- (a) points in an N-dimensional space;
- (b) "projecting the point onto a projection surface to generate projected coordinates ... less than N dimensions";
- (c) a hash from a key including the projected coordinates;
- (d) insert by hash.

**US 11,756,254, claim 1** (10 and 18 the same).
- (a) light information from a first ray set cast for "two or more light sources", including average contribution;
- (b) per-light values per spatial region, where "a spatial hashing algorithm segments the virtual environment into the plurality spatial regions corresponding to a plurality of non-cubic voxels";
- (c) a second ray set with more samples for higher-valued lights;
- (d) updated illumination per region;
- (e) render.

US 12,374,024 claim 1 has the same structure, with "at least one spatial region ... corresponding to a non-cubic voxel".

**US 12,456,261, claim 8** (1 and 15 similar).
- (a) partition a space into cells;
- (b) shift cell boundary points "using a set of one or more periodic functions";
- (c) light transport using the updated cells.

**US 11,609,899, claim 1** (11 the same).
- (a) change requests from parallel threads;
- (b) request information stored to a buffer;
- (c) per hash-map entry, the requests that correspond to it, several allowed per entry;
- (d) fetch their information from the buffer;
- (e) "iterating through the change requests corresponding to the individual entries in the hash map using single threads of execution for the individual entries".

US 12,013,844 claim 1 has (a), plus a buffer holding request indices and values representing other attempts, plus (e). US 12,360,985 claim 1 aggregates requests through a hash map and updates "by serially iterating through the two or more change requests for individual entries".

### 3.B NVIDIA ReSTIR light sampling

- **US 11,663,773, claim 9** (1 and 17 similar):
  - (a) select a set of lights "based at least in part on a first one or more random factors";
  - (b) select a subset based on second random factors;
  - (c) render a pixel from the subset.

  Claim 1 adds that the subset's data must fit "memory predicted to be available in a processor cache". **US 12,361,633** claim 7 is a set, then a proper subset sized for the cache, then render. **US 2025/0259377 A1** (pending) claim 8 is: sample a first subset of lights, store it, sample a second subset from memory, render.
- **US 11,948,246, claim 1** (8 and 16 similar):
  - (a) select first lights from the scene;
  - (b) "select a second one or more lights from among lights associated with one or more pixels in at least one of the frame or a prior frame";
  - (c) choose from both a light for a later frame;
  - (d) render from both.

  **US 12,646,247** claim 1 is (a), (b), and rendering later-frame pixels with a light chosen from both.
- **US 12,361,635, claim 1** (11 and 17 similar):
  - (a) initial light candidates for a pixel;
  - (b) add "at least one light sample identified as an important sample for a prior image";
  - (c) new weights and sample counts, discarding the reused candidates' prior ones;
  - (d) importance resampling over all;
  - (e) shade.

  **US 2025/0299434 A1** (pending) recites (c) and (d).
- **US 12,299,801, claim 1** (9 and 17 similar):
  - (a) per subdivision of the scene, sample a first light set by its contribution there;
  - (b) store each set in a cell, one cell per subdivision;
  - (c) sample a second set from each cell's first set;
  - (d) render with it.

  **US 2025/0278884 A1** (pending) does the same with 3D subdivisions, previous-frame contribution and proximity selection.

### 3.C Temporal history

**US 10,116,916, claim 1** (11 and 18 the same).
- (a) generate the current frame;
- (b) per pixel, sample the previous frame's resolved colour;
- (c) adjust it "based on a statistical distribution of color values for a plurality of samples in the neighborhood of the pixel in the current frame". This comprises a mean, a per-component variance, and "an axis-aligned bounding box (AABB) based on the mean color value and a standard deviation";
- (d) blend.

Claims 8 and 15 drop the mean/σ limitation but require "a composite image that comprises a plurality of layers", filtering only the layer "associated with a maximum relative difference in transmittance value". Claims 10 and 16 require a stereoscopic left/right image with cross-blends.

**US 10,362,289, claim 1** (12 and 20 similar).
- (a)-(b) as above;
- (c) a colour-space bounding box whose planes come from neighbourhood statistics, "and wherein at least one color value for the plurality of samples in the neighborhood of the pixel is outside of the bounding box";
- (d) adjust history by the box;
- (e) blend.

**History-clamp family.** Every member requires a fast history:
- **US 11,216,916, claim 1** (10 and 16): "generating a fast historical frame", an expected range from its values, a full-history value outside the range clamped, then blend.
- **US 11,232,544, claim 1** (10): the same, with the spatial-filter amount set by the clamp difference.
- **US 11,783,455, claim 1** (9 and 16): "using data accumulated over a first number of previous frames for clamping data accumulated over a second number of previous frames" chooses a different spatial filtering at a pixel affected by a temporal change.
- **US 12,299,855, claim 1** (9 and 16) and pending **US 2026/0051032 A1**: an expected range from the first-number frames, "a subset of a second number"; clamp the full history; blend.

**US 11,847,737, claim 1** (11 and 14 the same).
- (a) a display value from "a first pixel value for the pixel location and a plurality of previous pixel values";
- (b) the next frame's pixel information, including a second value;
- (c) "determining, based at least in part on the pixel information, a temporal lag between the first frame and the second frame";
- (d) a second display value "based at least in part upon a subset of the plurality of previous pixel values corresponding to the temporal lag".

The spec detects lag "based on the first pixel value and the second pixel value". The dynamic event appears only in dependent claim 9. **There is no dual-history element** in claim 1.

**US 12,536,735 (divisional), claim 12** (1 and 19 similar).
- (a) a first value from "a first list of previous pixel values" by a first algorithm;
- (b) a second value from a second list by a second algorithm;
- (c) a display value from both;
- (d) update from the first list.

Claims 1 and 19 add that the oldest value is removed from the first list.

**US 12,437,371, claim 1.** Verbatim:
- (a) "obtaining lighting data for a plurality of pixel locations of a current frame and for the plurality of pixel locations of a previous frame";
- (b) "selecting pixel locations ... based at least in part on the lighting data for the plurality of pixel locations of the current frame";
- (c) "determining, using the selected pixel locations, gradient information ... indicating a difference in the lighting data between the current frame and the previous frame";
- (d) "determining one or more confidence values ... based at least in part on the gradient information";
- (e) "determining a weighting of the lighting data, to be used for shading the pixel locations of the current frame based at least in part on the one or more confidence values."

Claims 11 and 17 add spatial blurring of the gradients, upscaling the blurred gradient image to the target resolution, and conversion to confidences. The spec's selection example is the brightest sample per 3x3 stratum (A-SVGF).

**US 2026/0162231 A1 (pending continuation), claim 1:** (a), (c) without "using the selected pixel locations", (d) and (e). **The selection element (b) is gone.**

### 3.D Denoisers

**US 10,600,167, claim 1** (15 and 18 the same).
- (a) a temporal filter of a filtering pipeline produces "a color and auxiliary information for each pixel", by:
  - (a1) "performing back-projection of a first sample of the pixel in the current image frame to a screen space location in a prior frame";
  - (a2) the second sample at that location;
  - (a3) "verifying that the first sample and the second sample are located on a same surface";
  - (a4) computing colour and auxiliary information from both, on that verification;
- (b) "providing the auxiliary information for each pixel ... to one or more subsequent filters of the filtering pipeline";
- (c) creating the reconstructed image with those filters.

Variance appears **only in dependents** 5 and 6 (colour or luminance variance). Dependent 8 is a confidence metric, 10 says spatial subsequent filters, and 20 is a depth, object-space normal and mesh-ID surface check.

**US 11,113,792.**
- **Claim 1** (11 similar):
  - (a) frames rendered with spatial filters whose tap locations are jittered;
  - (b) each filter's size "based at least in part on render data representative of ray-traced samples";
  - (c) taps for a subset of the filter's positions, chosen by jittering a selection parameter between filters;
  - (d) sparse sampling of ray-traced lighting.
- **Claim 15:**
  - (a) a temporal filter on pixels' ray-traced lighting values;
  - (b) "determining a size of a spatial filter based at least in part on a count of historical data values of the temporal filter";
  - (c) the spatial filter applied to the temporally filtered values.

**US 11,688,042.**
- Claim 1: two sub-kernels "that corresponds to the filter direction of the filter kernel", in two passes over ray-traced render data.
- Claim 9: "computing a first filter radius and a second filter radius; applying a filter in a filter direction ... over a plurality of iterations" using both radii.
- Claim 15: a filter determined from ray-traced render data, applied "in at least a filter direction as a separable filter over a plurality of passes".

The spec uses "filter direction" for one axis of a separable filter.

**US 12,423,782, claim 1** (10 and 16 the same). Verbatim:
- (a) "selecting, for a scene to be rendered, locations of one or more filter taps based at least on jittering at least one parameter corresponding to the locations; and"
- (b) "rendering one or more frames of the scene, the rendering including applying one or more spatial filters at the selected locations."

Dependents add random or pseudo-random jitter (3), jitter between frames (4), separable filters with reducing widths (7), and use on ray-traced shadows, reflections, AO or GI (9).

### 3.E Reflections (Adshir Ltd, now Snap Inc.)

**Non-planar family, priority 2018-06-09.** Every claim recites a "non-planar (specular) surface". The specs state that "the present invention deals only with reflections from non-planar objects".
- **US 10,699,468, claim 1:**
  - (a) primary rays projected on the non-planar surface;
  - (b) secondary rays shot from each surface triangle;
  - (c) "transforming to each surface triangle all its visible object triangles or a visible part thereof, and then shading the transformed object triangles";
  - (d) per surface triangle, sampling the transformed triangles' light;
  - (e) "per each surface triangle, matching the secondary rays and the primary rays";
  - (f) composite;
  - (g) transfer to pixels.

  Claim 12 adds a per-triangle point of projection and a per-triangle visibility test (drop, clip or transform).
- **US 10,950,030, claim 1**, and **US 11,302,058, claims 1, 11 and 20:** rays per surface triangle are "first projected to that triangle and further, as secondary rays, to the objects' triangles". Then the transform-and-shade, sampling, primary projection, per-triangle matching, compositing and transfer steps.
- **US 12,051,145, claim 1** (14 and 20 the same). Verbatim:
  - (a) "causing projection of one or more primary rays on a non-planar surface";
  - (b) secondary rays generated in response, "bouncing, from surface triangles of the non-planar surface, being decoupled from the one or more primary rays";
  - (c) "collectively handling the plurality of secondary rays as a common projection";
  - (d) "matching the plurality of secondary rays to a given one of the one or more primary rays to deliver light values to image pixels."
- **US 12,573,127, claim 1** (16 and 20 the same):
  - (a) primary projection on a non-planar surface;
  - (b) "computing a location of a phantom camera". The spec says "a reflected location of the real camera in a specific virtual plane", fitted per receiving triangle;
  - (c) secondary rays from the phantom camera;
  - (d) visibility of each object triangle from the phantom camera as "not visible, partly visible, and fully visible".

  **US 2025/0363721 A1** (pending) recites (b)-(d), still on a non-planar surface.

**Semi-reflective family, priority 2015-12-12.**
- **US 10,565,776, claim 1:**
  - (a) identify the semi-reflective surface;
  - (b) project primary rays from pixels onto it, giving "a cluster of primary hit points";
  - (c) "shooting a projection of carrier rays at a principal direction onto the object through the cluster of the generated primary hit points, whereas only those carrier rays that intersect the primary hit points are utilized for generating secondary rays";
  - (d) a render target texture of the secondary rays' intersections;
  - (e) sample light values in it;
  - (f) "feeding the sampled light values at the intersection points into their respective primary hit points";
  - (g) "repeating steps c-g multiple times while the direction is randomly deviated from the principal direction";
  - (h) reconstruct the reflection.

  Dependent 13 covers a perspective-shaped secondary projection. The spec defines "DAS", a "dynamically aligned structure", as "a projection of parallel rays, used to carry secondary rays".
- **US 10,789,759, claim 1:** the same, except (c) "collectively generating secondary rays by use of carrier rays" and (g) "repeating steps c-f multiple times while the direction of carrier rays is deviated from the principal direction" (no "randomly").
- **US 10,614,612, claim 1**, and **US 10,930,053, claim 1:** the same carrier-ray structure, for virtual objects "augmented into a real world image", on a reflective area identified in that image.
- **US 10,410,401, claim 1** (8 similar):
  - (a) "shooting additional rays at a three dimensional scene after primary rays were shot" at a cluster of existing hit points;
  - (b) secondary rays spawned at the intersections of the additional rays with existing hit points. Each spawned ray "originates at an existing hit point", "becomes associated with said existing hit point", "always keeps its original shooting direction" and seeks geometry onward.
- **US 10,332,304, claim 1:**
  - (I) a first projection of rays at the cluster of existing hit points "for acquiring a depth map of the existing hit points";
  - (II) a second projection through the cluster "utilizing the acquired depth map for creating secondary rays";
  - (III) render with them into a render target;
  - (IV) match hit points with the intersections;
  - (V) identify the objects there.

**Distributed accelerating structures, priority 2015-09-29.**
- **US 10,380,785, claim 1:**
  - (a1) "generating data structure of cells, wherein cell is a basic unit of process and data locality";
  - (a2) map scene objects onto cells;
  - (a3) "a single global acceleration structure";
  - (a4) "multiple local acceleration structures, one per each cell";
  - (a5) primary rays "by rendering the 3D scene by raster pipeline";
  - (a6) per primary ray, traverse the global structure to find its cell;
  - (a7) "registering a hit point in the cell of intersection";
  - (b1) "locally at each cell, shooting secondary rays originating at each primary hit point, or at each incoming ray";
  - (b2) traverse the local structure;
  - (b3) on a hit, sample reflected light "for calculation of a diffuse radiance at a point of the secondary ray's origin" and register a new hit point; on a miss, traverse the global structure to the next cell.
- **US 10,614,614** and **US 10,818,072:** system forms of the same, with "in run time" construction (10,818,072 adds multiple GPUs and autonomous per-cell rendering).

**No independent claim in any Adshir/Snap document read recites a "dynamic acceleration structure".** The phrase appears only in the summary of the non-planar family's specs.

## 4. Component mapping, element by element

✓ = the component has the element; ✗ = it lacks it; ~ = ambiguous. Each table lists only claims with some overlap. The other claims read are named in the "no overlap" line.

### 4.1 World radiance cache (WRC, m7-design §3.1)

As drafted:
- N8o-style keys (power-of-two world cube, level by distance, placement, side, plus the geometric normal), with claim/resolve inserts;
- a payload of sky visibility and irradiance SH, plus "Count and epochs: 8 B";
- per-cell blending with engine-event W caps;
- a fixed ray budget, with priority "new cells, then epoch-changed cells, then aged cells";
- Kajiya-style voting and bounce-rank to choose each cell's query point;
- pixels read their cell "with sub-cell jitter".

| Claim | Present | Absent | Reading |
|---|---|---|---|
| **US 10,614,613 cl. 1** | (a) a cell's query point is a path vertex; (b) position quantized to a cube at a distance level, plus side and normal; (c) hashed; (d) fingerprint plus claim/resolve; (e) cell rays' radiance accumulated into the cell; (f) a count field; (g) blended average with W caps | — | **Appears to practise every element as drafted**, if the count is incremented per accumulated update and used for the average (D0 variant "(a) world-cell accumulation with event W"). Dependents 2 (jitter before quantizing) and 5 (distance-based quantization) also match. **W1** |
| **US 11,790,594 cl. 1** | (a) cells are locations with irradiance caches, ranked by state and age (dependent 9: "frames since a prior update") under a budget (dependent 3); (b) rays cast from higher-ranked cells; (c) irradiance from the ray hit; (d) frame rendered from the cache | — | **Appears to practise every element.** Query-point voting is a second reading: candidate points sharing one cell are ranked by bounce-rank, and the cell's rays are cast from the winner. **W2** |
| US 11,315,310 cl. 1 | (b) GI on visible surfaces; (c) the WRC is a radiance/irradiance cache, as listed in the spec | (a) no reservoir or RIS | **Clear on (a) only.** Any resampling in GI or reflections that reads the WRC completes the claim, including cell-update selection and reflection hit shading (constraint C1) |
| US 2025/0182387 (pending) cl. 1 | — | (b) stratified directions, not points sampled by light value | Clear; watch |
| US 12,423,782 cl. 1 | (b) a cache read is a spatial lookup; ~ whether it counts as a "spatial filter" | ~ (a) the sub-cell jitter is a jittered tap location | **Thin.** A jittered lookup reads as a one-tap stochastic spatial filter. **W3** |
| US 9,754,407 cl. 1 / US 9,747,718 cl. 1 | (a) triangle meshes; (d) partly: `scatter` integrated at cell centres runs a user program | (b) cells come from `floor(world/size)`, not from a triangle's vertex coordinates; (c) cube cells, not triangle-shaped elements; 9,747,718 (b) sizing is per cell, not per primitive | Clear on (b) and (c) |
| US 11,925,860 cl. 1 | (a), (c), (d) | (b) no projection to fewer dimensions: the position stays 3D | Clear. If the normal joins the key, quantize it as a small discrete bin; don't project the *position* (constraint C2) |
| US 11,756,254 / US 12,374,024 cl. 1 | spatial hashing into regions | "non-cubic voxel(s)" (cells are cubes); "two or more light sources" with per-light values and per-light ray allocation | Clear. Keep cells cubic (constraint C3) |
| US 12,456,261 cl. 8 | cells | (b) no periodic boundary shifting | Clear |
| US 11,609,899 / US 12,013,844 / US 12,360,985 cl. 1 | (a) parallel requests; (b) per-request results in a buffer; (c) requests map to slots | (e) no per-entry single thread iterating through that entry's requests: claim/resolve takes an atomicMax per slot, then each requester checks whether it won (`vsc_resolve`) | Clear on (e) (constraint C4) |
| US 2024/0029338 (pending) cl. 1 | — | (b) every pixel reads its cell; the source is not chosen per pixel by sample count or disocclusion | Clear (constraint C5) |

No overlap: US 11,663,773, US 12,361,633, US 2025/0259377, US 11,948,246, US 12,646,247, US 12,361,635, US 2025/0299434, US 12,299,801 and US 2025/0278884. The WRC stores irradiance, not light sets, and selects no lights. Constraint C6 covers future local lights.

**Cook-time near field (G-N1, part of the WRC on non-RT).**
- Against 9,754,407 and 9,747,718: it stores a per-vertex visibility mask with no grid defined per triangle (~, low). The value is visibility from cook rays, not a shader program's shaded value.
- Against US 11,790,594: every vertex is traced at cook, so no ranking decides where rays are cast; and it stores visibility, not irradiance.
- Clear.

### 4.2 Fixed-direction fill

K deterministic, cosine-stratified directions per cell, rotated per cell, with deterministic queues. Cache-terminated rays and an RLE2 read on escape.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 11,315,310 cl. 1 | (b), (c) | (a) | Clear (C1) |
| US 2025/0182387 cl. 1 | (c) partly: shadow rays at cell hits toward the sun | (a)-(b) no joint light-plus-geometry point set; no sampling proportional to light value | Clear |
| US 10,380,785 cl. 1 | ~ (b3) radiance at the ray origin | (a1)-(a4) no spatial cells with per-cell local acceleration structures (rays traverse per-instance BVHs or the hardware TLAS from the top); (a6)-(a7); (b1) | Clear |

No overlap: the light-sampling family (3.B). The scheduling of the fill belongs to 4.1 (W2).

### 4.3 TR1: revived T1b

Resampled history through the exact hit record; instance, primitive and reverse-visibility validation; anchor bit; engine-event W caps; Catmull-Rom undershoot fix; revealed-pixel boost; jittered visibility buffer.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| **US 10,600,167 cl. 1** | (a1) back-projection via the hit record and previous transforms; (a2) the history sample there; (a3) identity and reverse-visibility check (dependent 20 names mesh ID); (a4) colour, plus any per-pixel W, age or anchor bit as "auxiliary information" | (b), (c): only while no later filter in the pipeline reads TR1's auxiliary data | **Clear with constraint**: the gap is entirely downstream of TR1. **W5.** Forward projection (T2's world-anchored scatter) would also drop (a1) |
| US 10,116,916 cl. 1, 11, 18 | (a), (b), (d) | (c) no neighbourhood colour statistics | Clear. Claims 8, 10, 15 and 16 are limited to layered or stereoscopic images |
| US 10,362,289 cl. 1 | (a), (b), (e) | (c), (d) | Clear |
| US 11,847,737 cl. 1 | (a), (b) | (c) W caps come from engine state, not from pixel values (the spec detects lag from the two pixel values); ~ "pixel information" is broad. (d) one blended value per pixel, so there is no retained list of previous values to take a subset of | Clear; ambiguous on (c) and (d) |
| US 12,536,735 cl. 12 | (a) | (b), (c) one history, one algorithm | Clear |
| US 12,437,371 cl. 1 / US 2026/0162231 | (a) | (c) no lighting difference is computed; validation uses identity and geometry | Clear |
| History-clamp family | — | the fast history and the clamp | Clear |
| US 12,423,782 cl. 1 | — | (a) the jitter moves camera samples on a lattice; filter taps (the Catmull-Rom history taps) are not jittered | Clear (low ambiguity) |

### 4.4 Own-pixel temporal moments

The mean and σ of a pixel's own accepted samples over time, used to shorten W or force a reset.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| **US 2026/0162231 A1 (pending) cl. 1** | (a); (c) comparing the current sample with the pixel's temporal mean and σ is a current-versus-previous lighting difference; (d) confidence; (e) the W or reset weighting | — | **Every element as published.** It lacks nothing if the claim is granted as filed. **W4** |
| US 12,437,371 cl. 1 | (a), (c), (d), (e) | (b) only, if moments are evaluated on every identity-validated pixel with no lighting-based selection | **Thin** |
| US 11,847,737 cl. 1 | (a), (b), (c) lag judged from the current value against history, as in the spec | (d) only if the response is a full reset to current-frame samples; a shortened-W blend is ambiguous | **Thin** |
| US 10,600,167 cl. 1 | moments are auxiliary information (dependents 5-6) | (b), (c) as in 4.3 | Same constraint as TR1 (W5) |
| US 10,116,916 / US 10,362,289 | (a), (b) | the spatial neighbourhood statistics | Clear (the design's note is right) |

### 4.5 Edge-aware à-trous (Dammertz 2010)

Fixed geometric edge stops (depth, normal, instance), no variance guidance, D0's residual clean-up.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 10,600,167 cl. 1 | it is a "subsequent filter" | (a1)-(a4) when its input comes from WRC accumulation, which is world-space with no screen back-projection; (b) when it reads no auxiliary data from TR1 or any reprojecting stage. Edge stops from the current G-buffer are not data computed by a temporal filter from two samples | Clear with constraint. **"No variance guidance" alone does not avoid this claim.** W6 |
| US 11,113,792 cl. 1, 11 | (d) sparse taps on ray-traced data | (b) fixed size, not from render data; (c) fixed tap pattern | Clear |
| US 11,113,792 cl. 15 | (c) | (a) the preceding accumulation is in world cells, not on pixels; (b) size not set by any history count | Clear (constraint in W6) |
| US 11,688,042 cl. 1, 15 | multiple passes over ray-traced render data | the separable form, with sub-kernels per filter direction | Clear with a non-separable 2D kernel |
| US 11,688,042 cl. 9 | the step size doubles each iteration (radii 1, 2, 4 ...) | ~ "in a filter direction": the spec means one axis of a separable filter, and a 2D kernel is not applied along one direction | **Thin.** W6 |
| US 12,423,782 cl. 1 | (b) spatial filter | (a) the taps are not jittered | Clear (W6: no per-frame or per-pixel pattern rotation) |
| US 11,783,455 / US 11,232,544 | per-pixel variation possible | the fast history and the clamp | Clear |

### 4.6 Single history

One exponential history: the WRC per cell, and TR1 per pixel.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| History-clamp family (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) | a full history | the fast or short history used to clamp it | Clear |
| US 12,536,735 | one list-like value | a second list and second algorithm | Clear |
| US 11,847,737 cl. 1 | (a), (b) | (c) when W comes from engine events; (d) no retained per-frame values | Clear; ~ (d) for a W-capped blend |
| US 10,614,613 cl. 1 | in the WRC, see 4.1 | — | Its counter-average is covered by W1 |

### 4.7 Recompute on light change

Caster, sun and program epochs; exact relight from transport records; event-driven W caps.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 12,437,371 / US 2026/0162231 | — | (c) no gradient: a cell is recomputed because an engine input changed | Clear |
| US 11,847,737 cl. 1 | — | (a) WRC cells are not pixel locations; (c) the event comes from engine state | Clear; ~ for per-pixel W caps that test a pixel's position against moved bounds |
| US 11,783,455 cl. 1 | ~ "identifying a temporal change" at a pixel | the dual-history clamp | Clear |
| US 11,790,594 cl. 1 | — | the ranking, as long as every invalidated cell is re-traced, not ranked (W2) | Clear with W2 |

### 4.8 Traced-primary RT reflections

Hardware primary rays; per-pixel lobe rays from each pixel's own hit; hit shaded by its own program; hardware TLAS/BLAS.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 12,051,145 cl. 1 | ~ (a) (whether a traced primary is a "projection"); (b) rays bounce from surface triangles | (b) "decoupled from the primary rays": each ray starts at its own pixel's hit; (c) no common projection; (d) no matching step: results go to the ray's own pixel | Clear |
| US 10,699,468; 10,950,030; 11,302,058 | (a) partly | per-reflector-triangle projection; transforming and shading object triangles onto each reflector triangle; per-triangle matching | Clear |
| US 12,573,127; US 2025/0363721 | — | no phantom camera; no per-triangle visibility classification | Clear |
| US 10,565,776; 10,789,759; 10,614,612; 10,930,053 | — | no carrier-ray projection; no render target of secondary intersections | Clear |
| US 10,410,401 | rays originate at hit points | no "additional rays" spawning them; directions come from the lobe, not an earlier ray | Clear |
| US 10,380,785 family | — | (a5) primaries are traced; (a1)-(a4), (a7), (b1) | Clear |
| US 11,315,310 cl. 1 | (b), (c): hit shading reads `indirect_irradiance` from the WRC, and B2 reads the WRC | (a) | Clear only without RIS (C1). See section 7, item 1 on "plain per-pixel weighted sampling" |

Correction: the operative avoidance of 12,051,145 is coupled per-pixel secondary rays. Traced versus rasterized primaries is not what matters (section 7, item 4).

### 4.9 Planar mirrors (non-RT)

An N1c raster view from the mirrored camera for each declared plane; glossy by lobe prefiltering of that view.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| Non-planar family (10,699,468; 10,950,030; 11,302,058; 12,051,145; 12,573,127; 2025/0363721) | the mirrored camera resembles 12,573,127's "phantom camera" | the "non-planar" surface in every claim | Clear, while planar views are used only for geometrically planar reflectors (W7) |
| **US 10,565,776 cl. 1** | (a) declared plane; (b) raster primaries; ~ (c) the mirrored camera's rays pass through the plane at the primary hit points, and dependent 13 covers perspective projections; (d) render target; (e) sampled; (f) looked up at each primary hit; (h) | (g) one render per frame, with no repeats at randomly deviated directions | **Thin**: only (g) is clearly absent. W7 |
| **US 10,789,759 cl. 1** | (a), (b), ~ (c) "collectively generating secondary rays by use of carrier rays", (d)-(f), (h) | (g) no repeated deviated projections | **Thin**, as above |
| US 10,410,401 cl. 1 | ~ the mirrored rays continue through the plane in their own direction | no ray is spawned at, or associated with, an existing hit point; the reflection is a projective lookup after rendering | Clear on a literal reading; ambiguous on a functional one |
| US 10,332,304 cl. 1 | (III) render target | (I), (II) no depth map of existing hit points: the oblique clip is the analytic plane | Clear (W7: never clip with a reflector depth map) |
| US 10,614,612 / US 10,930,053 | — | no real-world image | Clear |

Mirrored-camera planar reflection was published long before the 2015 priority date, for example stencil-buffer planar reflections in the late 1990s. That bears on how broadly a court would read (c), but this note doesn't assess validity.

### 4.10 Fixed sub-cone glossy rays

K deterministic sub-cones of `reflection_lobe`, rotated per pixel, from each pixel's hit.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 12,051,145 cl. 1 | (b) bounce | decoupling; (c) one pixel's K rays share a lobe but no projection is shared across a reflector triangle's rays; (d) | Clear |
| US 10,565,776 / 10,789,759 | — | (c), (d), (g): per-pixel rays, not repeated projections | Clear |
| US 11,315,310 cl. 1 | (b), (c) | (a) deterministic, no resampling | Clear (C1) |
| US 12,423,782 | — | rays are not filter taps | Clear |

### 4.11 R1n LOD-BVH tier (non-RT)

N1c raster primaries; independent per-pixel rays; cone-sized cluster-LOD BVHs cooked once, never refit; movers as static per-instance BVHs in a flat per-frame list.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 12,051,145 cl. 1 | (a) raster primaries on a possibly non-planar surface; (b) bounce | decoupling; (c) no grouping, packets, beams or shared frustum; (d) | Clear (the design's "no ray projection" rule is the right constraint) |
| US 10,380,785 cl. 1 (and 10,614,614; 10,818,072) | (a5) raster primaries; ~ (a3) a flat instance list | (a1)-(a2) no spatial cells with objects mapped onto them; (a4) per-mesh BVHs are per object, not per cell; (a6)-(a7) primary hits come from the visibility buffer and are never located in, or registered to, a cell; (b1)-(b2) rays are per-pixel work items traversing from the top, not launched "locally at each cell" | Clear |
| Other Adshir/Snap claims | — | as in 4.8 | Clear |
| US 11,315,310 | (b), (c) | (a) | Clear (C1) |

**Note.** A rebuilt top-level tree is not an element of any claim read. If R1n needs one for speed, the operative constraints are only those in W8 and C7: global-first traversal, and no per-cell hit registration or per-cell launches.

### 4.12 SDF-brick tier (non-RT)

Sphere tracing static bricks; movers' own static SDFs; B2 visibility rays and G-N2 far-field GI rays.

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 10,380,785 cl. 1 | (a1)-(a3) bricks are cells, with geometry voxelized into a global brick map; ~ (a4) each brick's SDF as a local structure; (a5) raster primaries; ~ (b2)-(b3) marching inside a brick, then moving to the next brick through the global map; (b3) diffuse radiance at the origin, for G-N2 only | (a7) no hit registration in bricks; (b1) rays are launched per pixel or per WRC cell, not per brick as the unit of processing; (b3) for B2: a visibility result, not diffuse radiance | **Clear with constraint**, on (a7) and (b1). Thinner than the LOD-BVH tier. W8 |
| US 10,614,614 / US 10,818,072 | as above | as above; also "in run time" construction, which static bricks avoid | Clear |
| Adshir/Snap reflection claims | — | no projections, phantom camera or render target | Clear |

### 4.13 Existing surface shadow cache (N8o)

Code read: `shaders/virtual_shadow_cache.wgsl:93-136` (cell from `floor(world/size)` at a distance level; key from slot, side and level) and `:539-625` (`vsc_claim` takes an atomicMax per slot; in `vsc_resolve` each request checks whether it won and writes).

| Claim | Present | Absent | Reading |
|---|---|---|---|
| US 10,614,613 cl. 1 | (a) the receiver point; (b) quantized position, side and level; (c) hash; (d) fingerprint and claim/resolve | (e) a shown cell stores a consensus-certified solid verdict, not accumulated contributions; (f) its "count" is the number of lit directions in one walk, not a counter of additions; (g) no average | Clear on (e)-(g) (constraint C8) |
| US 11,790,594 cl. 1 | ~ (a) the queues order work heaviest-first | it is not an irradiance cache; every pixel that needs a walk walks, so ordering doesn't decide casting; refresh phases are a fixed per-block schedule | Clear |
| US 9,754,407 / US 9,747,718 | — | (b), (c) cube cells with no triangle in the key (N8n P5) | Clear |
| US 11,925,860 | (a), (c), (d) | (b) | Clear |
| US 11,609,899 family | (a)-(c) | (e) | Clear (C4) |
| US 12,423,782 | — | the jitter is per-pixel ray-direction jitter for walks; lookups read the pixel's own cell | Clear |

This settles the engineering side of the open "US 9,754,407 review" for both caches. That patent's claims don't reach them. The hash-cache patents in 4.1 are the ones that matter.

## 5. Patented fallbacks: confirmation

| Setting | Claim it practises | Notes |
|---|---|---|
| `GiSampling::Restir` | US 11,315,310 cl. 1, all of (a)-(c) | Also US 12,361,635 if prior-frame candidates are reused, and US 11,948,246 / US 12,646,247 for light reuse |
| `Reconstruction::VarianceClip` | US 10,116,916 cl. 1, all elements | A mean ± γσ AABB per component, then blend |
| `Denoise::Svgf` (Falcor port) | US 10,600,167 cl. 1, all elements | Back-projection; depth/normal/mesh-ID check (dependent 20); moments-derived variance (dependents 5-6) passed to à-trous |
| `Denoise::FastHistory` (ReBLUR-like) | US 11,216,916 cl. 1; US 12,299,855 cl. 1; US 11,232,544 / US 11,783,455 if it drives spatial filtering | Plus US 12,536,735 if it keeps two value lists |
| `Reconstruction::GradientReshade` (A-SVGF) | US 12,437,371 cl. 1 (brightest-per-stratum selection is (b)); cl. 11 if the gradients are blurred and upscaled | Also the pending US 2026/0162231 |
| `Denoise::ValidationRays` (kajiya-style) | Probably US 2026/0162231 if it issues; US 12,437,371 cl. 1 only if the validated pixels are chosen by lighting data | Unchanged: "possibly within" |
| `NonRtReflections::CoherentHybrid` | US 12,051,145 cl. 1, if a triangle's rays are decoupled, handled as a common projection and matched | Also the 10,699,468 / 10,950,030 / 11,302,058 forms if object triangles are transformed onto reflector triangles |
| `ReflectionSampling::Restir` | US 11,315,310 cl. 1 (hits read the WRC) | Plus 11,948,246 / 12,646,247 / 12,361,635 depending on reuse |

## 6. Suggested design changes

Each change drops an element named in section 4. W1, W2 and W4 address components that appear to practise every element as drafted. The others keep thin margins from closing.

**W1. WRC value without counter-and-average (drops US 10,614,613 (f)-(g), and (e) in the first form).** Choose one in G1/D0 by measurement:
1. **Replace on update.** A cell update traces its full fixed direction set in one work item (a workgroup reduction), forms the K-direction quadrature, and *replaces* the cell value. There is no blend across updates and no counter.
   - Fixed per-cell direction sets mean an unchanged cell re-traces to the same value, so stability needs no running mean.
   - Cost: K rays per update, so fewer cells are updated per frame.
2. **Per-direction slots.** Each direction's latest result is stored in its own slot and replaced when re-traced. The cell value is a fixed-weight quadrature over the slots.
   - This is the G2 transport-record variant, and also relights on sun turns.
   - Memory: about 6-8 B x K per cell. At K = 8 and 2^21 cells that is about 96-128 MiB on top of the 80 MiB payload, which exceeds the 160 MiB stop, so it fits only at fewer cells or a smaller K.
3. **Fixed-α blend without a count.** Drops (f) and the counter in (g). It is weaker: dependents 10-11 show the patentee treats an EMA as the claimed average, though always with a counter. It also converges slowly after a reset.

Whatever is chosen, the "count" field in the payload must not count accumulated samples. Epochs and age are fine.

**W2. WRC scheduling without ranking (drops US 11,790,594 (a), and the "higher ranking" in (b)).**
- Event-driven work is unconditional: every new or epoch-changed cell is re-traced, with no comparison between cells. Budget the frame to cover it, and record spikes as measured cost.
- Aged refresh is a fixed rotation: a cell refreshes when (slot + frame) mod P = 0, as N8o's per-block refresh phases already do. No age ranking.
- Heaviest-first ordering stays allowed *within* the set that runs this frame, because there it orders work and doesn't decide which rays are cast.
- Query points: use a canonical point fixed by the key (for example, the cube centre moved to the surface by a deterministic probe) instead of Kajiya-style voting or bounce-rank among candidates. N8n's P1 canonical point lost on shadow contacts, so measure crevice error against R_GI.
- Ambiguity remains if overflow cells are deferred in slot order: that order derives from the key, though it is not a "characteristic" in the patent's sense (distance, visibility, age).

**W3. Deterministic WRC lookups (drops US 12,423,782 (a)).** Replace "sub-cell jitter" with fixed-weight interpolation over neighbouring cells, such as trilinear within a level, or a plain nearest-cell read. The parent-on-miss fallback is deterministic and fine. This also removes the US 10,614,613 dependent-2 match.

**W4. Own-pixel moments (drops the gradient element in US 12,437,371 and US 2026/0162231).**
- Rely on engine events and identity tests, which is TR1's design-around already.
- If undeclared shading change still needs a detector, put the moments behind a named setting such as `Reconstruction::OwnMoments`, off by default, until the continuation's claims settle. Even then:
  - evaluate every validated pixel, with no luminance-based selection (keeps 12,437,371 (b) absent);
  - respond only with a full reset to the current frame's samples (keeps 11,847,737 (d) absent).
- In m7-design §6 and §9.1 B, reclassify it from "unclaimed part" to "watch-listed variant".

**W5. TR1's auxiliary outputs stay internal (keeps US 10,600,167 (b)-(c) absent).**
- TR1's per-pixel W, age, anchor bit, moments and rejection mask are read only by TR1 itself, next frame.
- No later stage takes them as parameters: no sharpen, GI or reflection denoiser, upscaler or other filter.
- The revealed-pixel boost (a sampler inside TR1) may use TR1's rejection mask.
- Record this as a constraint for the later 4K upscaling unit.

**W6. À-trous form.**
- A non-separable 2D 5x5 kernel at each iteration (11,688,042 cl. 1, 15, and the spec sense of cl. 9).
- Fixed kernel size and iteration count, never derived from hit distance, ray-traced data or any history count (11,113,792 cl. 1, 11, 15).
- A fixed tap pattern with no per-frame or per-pixel rotation (12,423,782).
- No inputs from TR1 or from any screen-reprojecting stage (10,600,167).
- If 11,688,042 cl. 9 must be fully avoided, use a single-radius 2D filter or the WRC's cell-space filter in one pass.

**W7. Planar mirrors.**
- One raster view per plane per frame. Glossy only by prefiltering that view by the lobe, never by repeated deviated renders (keeps 10,565,776 / 10,789,759 (g) absent).
- Jitter the planar view only by the main view's mirrored sub-pixel lattice offset, for anti-aliasing. Never deviate its direction to sample a glossy lobe across frames.
- Planar views only for reflectors whose triangles lie in the plane within a tolerance. Never fit a mirrored camera to curved geometry or per triangle (the non-planar family's phantom camera).
- Clip with the analytic plane, never with a reflector depth map (10,332,304).
- Stronger design-around, if wanted: shade declared mirrors with R1n's per-pixel rays, at their measured cost.

**W8. Software-ray tiers.**
- Rays are per-pixel or per-cell work items that traverse from the top-level structure.
- Never collect primary hits into per-brick or per-cell lists and launch secondary rays per brick or cell, local structure first (10,380,785 (a7), (b1)).
- No shared frustum or projection per ray group (12,051,145 (c)).

**Constraints, already true in the design, that must stay true:**
- **C1:** no reservoir or RIS anywhere in GI or reflections that reads or writes the WRC, including cell-update selection and glossy sampling. Plain importance sampling (VNDF, BRDF or environment PDF) without resampling is fine.
- **C2:** the key position stays unprojected 3D.
- **C3:** cells stay cubic.
- **C4:** inserts keep atomicMax-and-winner-writes, never per-entry serial request lists.
- **C5:** a pixel's GI source is never switched between per-pixel samples and cache values by sample count or disocclusion.
- **C6:** future local lights get no random set-then-subset presampling (11,663,773), no per-cell sampled light lists (12,299,801), and no reuse of prior-frame or neighbour light choices (11,948,246 / 12,646,247 / 12,361,635).
- **C7:** RT and LOD-BVH reflection rays start at their own pixel's hit.
- **C8:** the shadow cache never accumulates walk results with a sample counter and averages them.

**Copy-with-notice list (m7-design §9.3).** Bevy Solari's world-cache files, kajiya's ircache and Capsaicin's hash cells are licence-clean, but check each before copying:
- for an accumulate-plus-counter average (10,614,613), likely in any hash radiance cache;
- for age- or visibility-ranked updates under a budget (11,790,594);
- for any reservoir use (11,315,310);
- for light-tile presampling (11,663,773).

Strip whatever W1-W3 and C1/C6 rule out. Not verified here: none of their code was read for this note.

## 7. Corrections to m7-design §9.1

This note doesn't edit m7-design.md. The design owner can apply these.

1. **A (GI cache).**
   - Add **US 10,614,613** and **US 11,790,594** as the WRC's primary exposures, with W1-W2 as the design-arounds.
   - US 11,315,310's claim is exactly three elements: reservoir importance resampling, GI on visible surfaces, a world-space data structure. The "WRC alone" reading holds, but the WRC is plainly element (c), so C1 must cover reflections too.
   - The §9.1 D worry that US 12,361,635 reads on "plain per-pixel weighted sampling" is unfounded: it needs prior-image candidates whose old weights are discarded. But if that sampling is RIS, **US 11,315,310 reads on it**, because hits read the WRC. Use plain importance sampling.
   - **US 9,754,407** claims per-triangle object-space shading grids. Both caches lack (b) and (c), so the "incident, not shaded" argument is not the operative one.
   - The US 11,925,860 note is right.
2. **B (temporal).**
   - The US 10,116,916 summary is right.
   - **US 10,362,289** needs a box that excludes at least one neighbourhood sample, so a plain min/max box avoids it; a box shrunk toward the mean does not.
   - Every history-clamp member (11,216,916; 11,232,544; 11,783,455; 12,299,855; 2026/0051032) needs a fast or short history. A plain min/max neighbourhood clamp over a single history therefore avoids the independent claims read, so the §9.2 "history clamping" caution can narrow to fast-history designs. The min/max option still reads output colour, which is a programmability problem.
   - US 12,437,371's granted claims need lighting-based pixel selection, but **US 2026/0162231** drops it, so own-pixel moments are at risk (W4).
3. **C (denoisers).**
   - **US 10,600,167's claim 1 has no variance element.** It is screen back-projection, a same-surface check, and auxiliary data passed to subsequent filters. "Non-variance à-trous" is not the operative avoidance; W5-W6 are.
   - **US 11,847,737's claim 1 has no dual-history element.** Temporal lag from pixel information plus a subset of previous values; dual lists are in its divisional, US 12,536,735.
   - Add US 11,113,792, US 11,688,042 and US 12,423,782 as à-trous constraints (W6).
4. **D (reflections).**
   - **No Adshir/Snap independent claim recites "dynamic acceleration structures".** The phrase is from the non-planar specs' summary. "DAS" in the 2015 family means a "dynamically aligned structure", a parallel-ray projection.
   - "A static cook-time BVH avoids the dynamic-structure element" is therefore not a claim-avoidance note. The operative avoidances are:
     - per-pixel coupled secondary rays with no common projection or matching (12,051,145);
     - no per-reflector-triangle transform-and-shade (10,699,468 / 10,950,030 / 11,302,058);
     - no phantom camera on a non-planar surface (12,573,127);
     - no carrier-ray projections with repeated deviated renders (10,565,776 / 10,789,759);
     - no per-cell local structures with registered hits (10,380,785).
   - Raster primaries are an element in several claims but are not decisive. The RT tier's "traced, not rasterized, primaries" is not its operative avoidance either; coupled per-pixel rays are.
   - The family is larger than §9.1 lists. Add the CIP parents 10,565,776, 10,789,759, 10,614,612, 10,930,053, 10,614,614, 10,380,785, 10,818,072, 10,410,401 and 10,332,304.
   - Planar views are thin against the 2015 semi-reflective family (W7). They are no longer "no secondary rays at all, so clear".
5. **Ownership.** m7-design §7 says P0 owns `docs/research/patent-claims.md`. This brief named `docs/m7-p0-claims.md`, so the record is here.

## 8. Uncertainty and what was not covered

**Ambiguous terms that decide a verdict.**
- "Projection" (12,051,145 (a); 10,565,776 (c)).
- "Auxiliary information" and "subsequent filters" (10,600,167).
- "Pixel information" and "subset of the plurality of previous pixel values" (11,847,737).
- "Spatial filter" applied to a cache lookup (12,423,782).
- "In a filter direction" (11,688,042 cl. 9).
- "Ranking ... based on characteristics", if overflow is deferred in slot order (11,790,594).
- Whether a counter-less fixed-α blend is "an average contribution" (10,614,613).
- Whether per-brick SDF marching is "local acceleration structure" traversal (10,380,785).

**Pending applications** whose claims can change: US 2024/0029338, 2025/0182387, 2025/0259377, 2025/0278884, 2025/0299434, 2025/0363721, 2026/0051032 and 2026/0162231. Two later continuations were seen but not read: US 2026/0162355 (light importance caching) and US 2026/0162376 (boundary shifting).

**Not read.**
- Foreign counterparts.
- Intel US 12,374,006 (neural TAA validation), not relevant unless a neural validator is used.
- NVIDIA US 11,113,800 (neural image filtering) and US 11,631,210 (neural radiance cache).
- Further Adshir/Snap ancestors beyond one level from the flagged members.
- RTXGI/DDGI, SHaRC-specific or NRC filings, beyond the hash-cache items found by three web searches.
- Epic, AMD, Unity and Microsoft filings, which were not searched.

**The WRC needs a broader search** before any commercial claim. Its hits came from three web queries, not from a structured search.

**Not assessed:** validity, prior art (beyond the one planar-mirror remark), equivalents, prosecution history, and whether any patent has lapsed for unpaid fees.

## Evidence and time

**Sources.**
- Google Patents full-text pages for the 53 documents in section 2, fetched 2026-11-07.
- Leads came from [gi.md](research/gi.md), [reflections.md](research/reflections.md), [reuse-temporal-denoise.md](research/reuse-temporal-denoise.md), [open-source-reuse.md](research/open-source-reuse.md), [m7-design.md](m7-design.md) §9 and [milestone-7.md](milestone-7.md).
- Web searches found US 10,614,613 and the hash-cache filings (search results, then their Google Patents pages).
- Local code read: `shaders/virtual_shadow_cache.wgsl` (key, claim and resolve). Docs read: [non-rt-n8n.md](non-rt-n8n.md) and [non-rt-n8o.md](non-rt-n8o.md).

**Artifacts.** In `E:/renderlab-work/p0-patents/`, not committed:
- `raw/`: 53 pages, 29 MB, each below 5 MB;
- `out/`: extracted metadata and independent claims, 0.4 MB;
- `scripts/`: `extract_claims.py`, `dump_claims.py`.

Every claim quoted here can be regenerated from `raw/` with the scripts.

**Time.** Reading, retrieval and mapping only. No implementation, build, GPU, Unreal or verification run. No source files were changed.
