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
| P1 | **Material, decode once** (per program, MP1 lists at 32 lanes) | covered pixels | hit record, vertex buffers, material parameters and textures, decals | material record with its receiver word (below); S1's glossy-list entry; emission |
| P2 | **GI receiver lookup** | indirect over covered pixels, small register footprint | geometry only: the hit record and camera ray (position, the vertex normal for the cell bin), cache tables | prepared SH, sky L2 + bounce L1, 48 B/px (binding 68); shading evaluates it at its shading normal |
| P3a | **Local-light visibility** | coherent screen tiles | position, light clusters, the local atlas or ray query | unorm8 visibility, 16 slots a pixel (binding 73) |
| P3b | **RT sun visibility**, split from the fused camera visibility | covered pixels | position, TLAS | sun visibility; no binding of its own (reuses `primary_trace_receivers` / `primary_vb_sun`), 80+ if one is ever needed |
| P3 | **Lights** (the standard model) | coherent screen tiles | record, P2's SH, P3a/P3b's visibility, light clusters | HDR (direct + indirect diffuse + untraced specular + emission, through fog); binding 62 (reflectance x fog transmittance, w) |
| P4 | **Reflections** (S1) | the glossy list, then hits, then the composite | list, record (lobe), TLAS or software BVH, binding 62 | HDR += w x reflectance x mean |
| - | Unchanged | | | TR1, post |

Units running tonight (2026-11-13), each behind a switch, default off:

| Unit | Pass | Switch | Output |
|------|------|--------|--------|
| the Mac (`t3/material-pass`) | P1 material record | `RENDERLAB_MATERIAL_PASS=1` | bindings 75, 78 |
| U-GIPASS | P2 GI receiver lookup | `RENDERLAB_GI_PASS` | binding 68, prepared SH (sky L2 + bounce L1), 48 B/px |
| U-LVIS | P3a local-light visibility | `RENDERLAB_LIGHT_VIS_PASS` | binding 73, unorm8 x 16 slots |
| U-SUNRT | P3b RT sun visibility (PR #94, default off) | `RENDERLAB_SUN_PASS` | no binding of its own (reuses `primary_trace_receivers` / `primary_vb_sun`); 80+ if one is ever needed |
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

As merged on main 95259a28 (`src/program.rs`, `MATERIAL_WRITE` and `rl_material_store`):

| Word | Field | Encoding |
|------|-------|----------|
| word 0 (binding 75, rgba32uint) .x | shading normal | octahedral |
| word 0 .y | base colour | unorm10 |
| word 0 .z | lobe alpha, metallic, MQ2 view share | pack4x8unorm (one byte spare) |
| word 0 .w | emission (times exposure) | rgb9e5 |
| word 1 (binding 78, rg32uint) .x | geometric normal | octahedral |
| word 1 .y | the lighting model | u32 |

The model lets later passes skip pixels whose program keeps its own `shade`.

**Decode once (coordinator, 2026-11-13).** The Mac measured about 0.75 ms of hit decode still in
shading after the material pass, so the material pass becomes the pass that decodes each pixel's
hit once for everything after it, rather than a separate G-buffer pass in front of it:

- **Word 1 (binding 78) becomes rgba32uint**, arriving with the Mac's PR #95 (the 32-byte record;
  main still has the 24-byte one above):
  - .x: the geometric normal (octahedral);
  - .y: the model, the MQ2 alpha and the roughness;
  - .z: the MQ2 view share (unorm16) and the MQ2 hint (f16; the material pass computes
    `tr1_view_hint`);
  - .w: the program slot (bits 0-28), thin (29), source_front (30) and lit_front (31).
- **Word 2, new: the receiver.** Group 0, binding 74 (reserved), rgba32uint, frame size:
  - .xyz: the trace point as f32 (`Receiver.world`);
  - .w: the epsilon's exponent (bits 0-7) and the interpolated vertex normal, octahedral 2 x 12
    bits (bits 8-31).
- **Size:** three rgba32uint words, 48 bytes a pixel: 99.5 MB at 1080p.
- **Group 0 bindings** (the Mac's count):
  - taken: 0-60, 62-65, 68 (GI prepared), 69 and 79 (reflections, #90), 70-72, 76, 77 (panes),
    73 (#91), 75 and 78 (the record);
  - free: 61, 66, 67 and 80 up; 74 is reserved for word 2.
- **Every later pass reads it.** Each builds its `Receiver` and a thin `SurfaceInput` from the
  record, without touching the hit image or the vertex buffers. Footprints come from the pixel's
  neighbouring camera rays against the receiver's plane.
- **Order:** visibility, then the material pass (decode once), then sun, GI, lights and shading.
- **Expected on the M5:** Showcase from +0.70 ms net to about even, Glint goal from +0.39 to
  about -0.35. The GI, light-visibility and sun passes each drop their own decode.
- **Until the word lands,** the pass units keep their own geometry reconstruction. Each then
  switches to the shared receiver read behind its own switch.
- **The Mac checks first** what still reads vertex attributes after `surface`. One thing stays:
  standard-program shading keeps its vertex attribute fetch for the per-light specular filter.
  `filtered_brdf` reads the normal's derivatives and the tangent, and for normal-mapped
  materials samples the normal map in a 16 x 16 quadrature per light. That is recorded as what
  the standard path keeps, not routed through the record.

### The reflection pass and the receiver word

The reflection pass's own kernels never decode a hit, today or after the split. Shading packs
what they need into the 48-byte glossy-list entry (the ray origin, the cone width and spread,
the lobe normal, the view and side directions, alpha, the strata count, the pixel and its first
slot).
- `rl_trace_pixels` reads the entry and one per-pixel texel, the receiver epsilon (binding 79).
- The composite reads the entry, the reflectance (binding 62) and its R slots.

So the receiver word saves them no decode. What it does allow:

- **The trace reads the receiver from the word.** Origin and epsilon come from the receiver word,
  the lobe normal from the record's shading normal, the side from its geometric normal and front
  bit, and the view direction from the pixel's camera ray. Binding 79 and its texture go away.
- **The entry shrinks from 48 to 16 bytes:** the pixel, the first slot, alpha with the count,
  and the cone width and spread.
- **What that saves is small.** At game_scene's 29,610 glossy pixels, 32 bytes less per entry
  is about 1 MB a frame of list writes in shading and reads in the trace and composite, plus one
  texel per entry: under 0.05 ms (E). The larger win stays in shading itself, where the lobe store
  writes a third of the bytes.
- **Condition:** only pixels with a record can use it. Programs without a declared mapping keep
  the single kernel and the full entry. The short entry is worth doing once every reporting
  program has a record; until then the full entry stays, and nothing is gained by carrying both.
- **Reflection hits** (`reflection_hits`) still decode their hit points from the vertex buffers:
  a hit is not a screen pixel, so no record covers it.

## 3. Where things go

| Item | Pass |
|------|------|
| Decals, normal mapping, `surface` | P1 |
| Emission | P1 (in the record); P3 adds it |
| MQ2 view share, lobe alpha, S1's list entry | P1 |
| GI cache lookup (diffuse) | P2 (prepared SH) |
| ES1 sky visibility | From P2's prepared sky L2. `es_prepare` (`shaders/environment_lobes.wgsl`) already takes it from the same blended lookup (`gi_rx_sky`) about the receiver's vertex normal, so it needs no record and no further cell reads. |
| Environment specular for the untraced band (the RLE2 lobe table along the reflection direction, times ES1's visibility) | The shading kernel, then P3 with the lights once the record lands: it needs the shading normal and roughness, which P2 does not have. |
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
2. **The material pass and the GI lookup, in parallel (both tonight).** They are independent:
   the GI lookup (P2, U-GIPASS) reads geometry only and writes prepared SH that shading evaluates
   at its own shading normal, so it never waits for the record. By RTX cost, the GI lookup is
   the one to land first; the RTX supports it over lights as the second split:
   - GI lookups cost 0.65 / 0.94 ms on Showcase (RT / non-RT) and 2.08 / 3.87 ms on Glint goal;
   - lights with shadows cost 0.40 / 0.96 ms on Showcase;
   - on the M5 the lookup is 2.09 / 2.43 ms, 84-86 % dependent loads and latency-bound at the
     whole kernel's occupancy. A small-register indirect pass hides that latency with many more
     warps.
   - Measure: the lookup pass's ms against the kernel's drop, both paths, Showcase, Glint goal
     and the quarry.
   - The material pass (P1, the Mac's `t3/material-pass`): the rest of today's kernel rebuilds
     `Surface` from the record instead of running `surface`. Measure: the record's write cost
     and P1's compile seconds.
3. **Lights and shadows (P3a U-LVIS and P3b U-SUNRT tonight, independent of the program; P3
   after the record).**
   - M5 1.88 / 1.84 ms, of which shadows are 0.95 / 1.27. Per-tile coherence shapes ray-query
     divergence (the lanterns finding), and the light bound applies per tile.
   - Measure: the pass against the remaining kernel, and the lanterns scene.
4. **The remaining engine inputs:** whatever is left in the per-program kernel moves to P1 or P2 (M5
   1.0 / 2.1 ms). Entry, decode and material remain, at most 0.8 ms on the M5.

## 6. What each step closes off for programmability

| Step | What it closes off (the own-`shade` fallback stays open) |
|------|----------------------------------------------------------|
| 1. Reflections | Traced reflections use a GGX lobe from the record's alpha and normal; a program's custom non-GGX lobe shape is not traced. |
| 2a. GI lookup | Indirect diffuse arrives as a prepared L1 SH per pixel. A program can still evaluate it in any direction (back-lighting, wrap, thin transmission), but only at L1 accuracy, and no longer queries the cache itself. |
| 2b. Material record | A program passes `shade` only the record's fields: no anisotropy, clearcoat, sheen, subsurface or custom per-pixel values. Each needs record growth or the fallback. Programs without a declared mapping keep the single kernel. |
| 3. Visibility, then lights | Local and sun visibility become per-pixel slots computed before shading (16 local slots), so a program cannot cast its own visibility queries. Lights then shade with the standard model. Wrap lighting, toon banding, retro-reflection and custom light response need the fallback until a material graph exists. |
| 4. Engine inputs | Decals and fog become fixed engine stages, not program-visible hooks. |

Mixed frames run both kinds: standard-model pixels through P1-P4, fallback pixels in their own
list.
