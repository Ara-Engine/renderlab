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
| P1 | **Material** (per program, MP1 lists at 32 lanes) | covered pixels | hit record, mesh attributes, material parameters and textures, decals | material record; S1's glossy-list entry; emission |
| P2 | **GI receiver lookup** | indirect over covered pixels, small register footprint | record (shading normal, flags), position (hit record + camera ray), cache tables | prepared irradiance and untraced specular (rgba16f x 2) |
| P3 | **Lights and shadows** | coherent screen tiles | record, position, light clusters, shadow maps / VSM / ray query, P2's outputs | HDR (direct + indirect diffuse + untraced specular + emission, through fog); binding 62 (reflectance x fog transmittance, w) |
| P4 | **Reflections** (S1) | the glossy list, then hits, then the composite | list, record (lobe), TLAS or software BVH, binding 62 | HDR += w x reflectance x mean |
| - | Unchanged | | | TR1, post |

A program with its own `shade` keeps today's single kernel as the fallback: it runs in its own
MP1 list, writes HDR directly, and sets a record flag so P2 and P3 skip its pixels.

## 2. The material record (the Mac's layout, shared by port and wgpu)

24 bytes a pixel: one rgba32uint (16 B) plus one rg32uint (8 B). That is 50 MB at 1080p and
199 MB at 4K.

| Field | Encoding | Bytes |
|-------|----------|------:|
| shading normal | octahedral, 2 x u16 | 4 |
| geometric normal | octahedral, 2 x u16 | 4 |
| base colour + coverage | rgb8 + u8 | 4 |
| metallic, roughness | 2 x u8 | 2 |
| emission | rgb9e5 | 4 |
| flags (double-sided/front, thin, transmission/IOR class, normal-mapped, own-`shade`) + program slot | u16 + u16 | 4 |
| MQ2 view share, lobe alpha | 2 x u8 | 2 |

Position is not stored. It comes from the hit record's distance along the camera ray (traced
primaries), or from depth (raster or the visibility buffer).

## 3. Where things go

| Item | Pass |
|------|------|
| Decals, normal mapping, `surface` | P1 |
| Emission | P1 (in the record); P3 adds it |
| MQ2 view share, lobe alpha, S1's list entry | P1 |
| GI cache lookup (diffuse) | P2 |
| ES1 blocked sky specular, environment specular for the untraced band | P2 (the same cell reads as the lookup, which is why they move together) |
| Lights: cluster walk, standard model (Lambert + GGX from the record) | P3 |
| Local shadows (atlas, ray query), sun (VSM, ray query) | P3 |
| Fog and aerial perspective | P3 (writes transmittance into binding 62 for P4) |
| Traced reflections | P4 |

## 4. Compile cost per pass (E)

To be measured per pass as each step lands, with a cleared and uniquified driver cache (memory:
nvidia-shader-cache-in-measurements).

| Pass | How often it compiles | Estimate |
|------|----------------------|----------|
| P1 material | per program and path | 10-20 s (no ray query, no light loop) |
| P2 GI lookup | once per path | 5-15 s |
| P3 lights and shadows | once per path | 30-50 s (ray query, cluster walk, shadow filters) |
| P4 trace and hits | once per path (hits per program today) | already separate |

For N programs that is about N x 15 + 60 s, against N x 80 s today. Only P1 grows with the
programs.

## 5. Migration order, one measurable step a night

1. **The reflection pass (S1).** Already being built; it is the first piece.
2. **The material pass and record.** P1 writes the record; the rest of today's kernel reads it
   instead of running `surface`. It is a prerequisite of both candidates for the next step,
   since the GI lookup and the lights both need the shading normal from the record.
   - Measure: record write cost, and P1's compile seconds.
3. **The GI receiver lookup (P2).** The RTX supports GI lookup over lights as the second split:
   - GI lookups cost 0.65 / 0.94 ms on Showcase (RT / non-RT) and 2.08 / 3.87 ms on Glint goal;
   - lights with shadows cost 0.40 / 0.96 ms on Showcase;
   - on the M5 the lookup is 2.09 / 2.43 ms, 84-86 % dependent loads and latency-bound at the
     whole kernel's occupancy. A small-register indirect pass hides that latency with many more
     warps.
   - Measure: the lookup pass's ms against the kernel's drop, both paths, Showcase, Glint goal
     and the quarry.
4. **Lights and shadows (P3)** as a program-independent tile pass.
   - M5 1.88 / 1.84 ms, of which shadows are 0.95 / 1.27. Per-tile coherence shapes ray-query
     divergence (the lanterns finding), and the light bound applies per tile.
   - Measure: the pass against the remaining kernel, and the lanterns scene.
5. **Named engine inputs:** whatever is left in the per-program kernel moves to P1 or P2 (M5
   1.0 / 2.1 ms). Entry, decode and material remain, at most 0.8 ms on the M5.

## 6. What each step closes off for programmability

| Step | What it closes off (the own-`shade` fallback stays open) |
|------|----------------------------------------------------------|
| 1. Reflections | Traced reflections use a GGX lobe from the record's alpha and normal; a program's custom non-GGX lobe shape is not traced. |
| 2. Material record | A program passes `shade` only the record's fields: no anisotropy, clearcoat, sheen, subsurface or custom per-pixel values. Each needs record growth or the fallback. |
| 3. GI lookup | Indirect diffuse is prepared irradiance about the shading normal. A program can no longer query `indirect_radiance` in an arbitrary direction (back-lighting, wrap, thin transmission) except through the fallback. |
| 4. Lights | Lights shade with the standard model. Wrap lighting, toon banding, retro-reflection and custom light response need the fallback until a material graph exists. |
| 5. Engine inputs | Decals and fog become fixed engine stages, not program-visible hooks. |

Mixed frames run both kinds: standard-model pixels through P1-P4, fallback pixels in their own
list.
