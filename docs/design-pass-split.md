# Pass split: materials, lighting and reflections as separate passes

U-RTVIS design note, 2026-11-13. Direction: Islam, approved as the main line (AGENTS.md on main
e84d1a15). Docs only; nothing here is built. Measured ms are the Mac's M5 floor split (Showcase /
Glint goal, 32 lanes) and PC integration's RTX 4070 SUPER split; estimates are marked (E).

## Today

One kernel per program and path, `primary_shade_pixel` (`shaders/program_traced_entry.wgsl`):

1. decode the hit;
2. `decal_prepare`;
3. the program's `surface`;
4. `rl_reflection_prepare` (the inline strata, or S1's list entry);
5. `rl_indirect_prepare` (the GI cache lookup, ES1's blocked sky specular, the environment);
6. the program's `shade` (the light loop with shadow visibility);
7. `atmosphere_surface`;
8. the HDR store and TR1's view hint.

Every program compiles all of it: 69-96 s per shading kernel on NVIDIA, with about 5 GB of
driver memory each (memory: nvidia-compile-memory-per-kernel). The GI lookup and the shadow
lookups run at the whole kernel's register count and occupancy.

## 1. The passes

| # | Pass | Dispatch | Reads | Writes |
|---|------|----------|-------|--------|
| P1 | **Material** (per program, MP1 lists at 32 lanes) | covered pixels | hit record, mesh attributes, material parameters and textures, decals | material record (group 0, bindings 75 and 78); S1's glossy-list entry; emission |
| P2 | **GI receiver lookup** | indirect over covered pixels, small register footprint | position (hit record + camera ray), the vertex normal for the cell bin, cache tables | prepared SH (binding 68, 48 B/px), evaluated by shading about the shading normal |
| P3a | **Local-light visibility** | coherent screen tiles | position, light clusters, the local atlas or ray query | unorm8 visibility, 16 slots a pixel (binding 73) |
| P3b | **RT sun visibility**, split from the fused camera visibility | covered pixels | position, TLAS | sun visibility (binding 74) |
| P3 | **Lights** (the standard model) | coherent screen tiles | record, P2's SH, P3a/P3b's visibility, light clusters | HDR (direct + indirect diffuse + untraced specular + emission, through fog); binding 62 (reflectance x fog transmittance, w) |
| P4 | **Reflections** (S1) | the glossy list, then hits, then the composite | list, record (lobe), TLAS or software BVH, binding 62 | HDR += w x reflectance x mean |
| - | Unchanged | | | TR1, post |

Units running tonight (2026-11-13), each behind a switch, default off:

| Unit | Pass | Switch | Output |
|------|------|--------|--------|
| the Mac (`t3/material-pass`) | P1 material record | `RENDERLAB_MATERIAL_PASS=1` | bindings 75, 78 |
| U-GIPASS | P2 GI receiver lookup | `RENDERLAB_GI_PASS` | binding 68, prepared SH, 48 B/px |
| U-LVIS | P3a local-light visibility | `RENDERLAB_LIGHT_VIS_PASS` | binding 73, unorm8 x 16 slots |
| U-SUNRT | P3b RT sun visibility | `RENDERLAB_SUN_PASS` | binding 74 |
| U-RTVIS | P4 reflections (S1) | `RENDERLAB_RL_PASS`, `RENDERLAB_RL_DEFERRED` (non-RT default on the list head) | glossy list 56, room counters 69, receiver epsilon 79 |

A program with its own `shade` keeps today's single kernel as the fallback: it runs in its own
MP1 list, writes HDR directly, and sets a record flag so P2 and P3 skip its pixels.

## 2. The material record (the Mac's `t3/material-pass`, shared by port and wgpu)

The seam exists: the Mac's branch `t3/material-pass`, switch `RENDERLAB_MATERIAL_PASS=1`
(default off). A material entry runs the decode and the program's `surface` once a pixel and
writes the record; the shading entry rebuilds `Surface` from it. Programs opt in through a
declared mapping (the standard program does); the others keep the single kernel. It is also the
seam a material graph plugs into later.

24 bytes a pixel in group 0, bindings 75 and 78: 50 MB at 1080p, 199 MB at 4K.

| Word | Field | Encoding |
|------|-------|----------|
| rgba32uint .x | shading normal | octahedral, 2 x 16 bits |
| rgba32uint .y | base colour + coverage | rgb8 + u8 |
| rgba32uint .z | metallic, roughness + flags / slot / model | 2 x u8 + u16 |
| rgba32uint .w | emission | rgb9e5 |
| rg32uint .x | geometric normal | octahedral, 2 x 16 bits |
| rg32uint .y | MQ2 view share, lobe alpha | 2 x u8 (16 bits spare) |

The flags carry double-sided/front, thin, the transmission/IOR class, normal-mapped and the
lighting model, so later passes can skip pixels whose program keeps its own `shade`.

Position is not stored. It comes from the hit record's distance along the camera ray (traced
primaries), or from depth (raster or the visibility buffer).

## 3. Where things go

| Item | Pass |
|------|------|
| Decals, normal mapping, `surface` | P1 |
| Emission | P1 (in the record); P3 adds it |
| MQ2 view share, lobe alpha, S1's list entry | P1 |
| GI cache lookup (diffuse) | P2 (prepared SH) |
| ES1 blocked sky specular, environment specular for the untraced band | P2 (the same cell reads as the lookup, which is why they move together) |
| Lights: cluster walk, standard model (Lambert + GGX from the record) | P3 |
| Local shadows (atlas, ray query) | P3a |
| Sun: RT ray query | P3b; non-RT VSM stays in P3 until it is split too |
| Fog and aerial perspective | P3 (writes transmittance into binding 62 for P4) |
| Traced reflections | P4 |

## 4. Compile cost per pass (E)

To be measured per pass as each step lands, with a cleared and uniquified driver cache (memory:
nvidia-shader-cache-in-measurements).

| Pass | How often it compiles | Estimate |
|------|----------------------|----------|
| P1 material | per program and path | 10-20 s (no ray query, no light loop) |
| P2 GI lookup | once per path | 5-15 s |
| P3a / P3b visibility | once per path each | 15-30 s (ray query or the atlas filters) |
| P3 lights | once per path | 10-20 s (cluster walk, the standard model) |
| P4 trace and hits | once per path (hits per program today) | already separate |

For N programs that is about N x 15 + 60 s, against N x 80 s today. Only P1 grows with the
programs.

## 5. Migration order, one measurable step a night

1. **The reflection pass (S1).** Already being built; it is the first piece.
2. **The GI receiver lookup (P2; U-GIPASS tonight).** It writes prepared SH that shading
   evaluates about its own shading normal, so it does not wait for the material record. The
   RTX supports GI lookup over lights as the second split:
   - GI lookups cost 0.65 / 0.94 ms on Showcase (RT / non-RT) and 2.08 / 3.87 ms on Glint goal;
   - lights with shadows cost 0.40 / 0.96 ms on Showcase;
   - on the M5 the lookup is 2.09 / 2.43 ms, 84-86 % dependent loads and latency-bound at the
     whole kernel's occupancy. A small-register indirect pass hides that latency with many more
     warps.
   - Measure: the lookup pass's ms against the kernel's drop, both paths, Showcase, Glint goal
     and the quarry.
3. **The material pass and record (P1; the Mac's `t3/material-pass`).** The rest of today's
   kernel rebuilds `Surface` from the record instead of running `surface`. Measure: the record's
   write cost and P1's compile seconds.
4. **Visibility, then lights (P3a U-LVIS and P3b U-SUNRT tonight; P3 after the record).**
   - M5 1.88 / 1.84 ms, of which shadows are 0.95 / 1.27. Per-tile coherence shapes ray-query
     divergence (the lanterns finding), and the light bound applies per tile.
   - Measure: the pass against the remaining kernel, and the lanterns scene.
5. **Named engine inputs:** whatever is left in the per-program kernel moves to P1 or P2 (M5
   1.0 / 2.1 ms). Entry, decode and material remain, at most 0.8 ms on the M5.

## 6. What each step closes off for programmability

| Step | What it closes off (the own-`shade` fallback stays open) |
|------|----------------------------------------------------------|
| 1. Reflections | Traced reflections use a GGX lobe from the record's alpha and normal; a program's custom non-GGX lobe shape is not traced. |
| 2. GI lookup | Indirect diffuse arrives as a prepared L1 SH per pixel. A program can still evaluate it in any direction (back-lighting, wrap, thin transmission), but only at L1 accuracy, and no longer queries the cache itself. |
| 3. Material record | A program passes `shade` only the record's fields: no anisotropy, clearcoat, sheen, subsurface or custom per-pixel values. Each needs record growth or the fallback. Programs without a declared mapping keep the single kernel. |
| 4. Visibility, then lights | Local and sun visibility become per-pixel slots computed before shading (16 local slots), so a program cannot cast its own visibility queries. Lights then shade with the standard model. Wrap lighting, toon banding, retro-reflection and custom light response need the fallback until a material graph exists. |
| 5. Engine inputs | Decals and fog become fixed engine stages, not program-visible hooks. |

Mixed frames run both kinds: standard-model pixels through P1-P4, fallback pixels in their own
list.
