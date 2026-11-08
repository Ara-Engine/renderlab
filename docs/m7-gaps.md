# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Reflections

From M7-R1 ([m7-r1.md](m7-r1.md)). Scores are whole-reflector display RMS at -14 stops (8-bit F0 codes) against R0a's references, static, 1x, GI cache off; Unreal floors are R0a's.

1. **Non-RT has no traced reflections.** Mirror and glossy surfaces reflect only the environment, through ES1's `indirect_radiance` (occluded, with bounce, once the world radiance cache is on). RF1-RF9 are unchanged and miss UE SW Epic on every fixture: RF1 46.1 vs 37.2, RF2 21.0 vs 15.9, RF3 45.8 vs 32.2, RF4 45.7 vs 40.6, RF6 52.3 vs 12.3, RF7 48.0 vs 35.6, RF8 9.9 vs 5.2, RF9 46.2 vs 37.0. Local objects never appear in non-RT reflections. The fix is R1n's LOD-BVH software rays (design 5.2, R0c's BVH), which did not fit the time-box. The patent-fallback trigger (`NonRtReflections::CoherentHybrid`) has not been evaluated, because no design-around tier has been measured yet.
2. **Raster primaries get no traced reflections.** The RT device's code default is still raster primaries (IQ1, RT1d), and only traced-primary frames trace reflections. Raster frames keep the environment estimate, as non-RT does.
3. **Mixed-program traced frames are rejected**, so a reflection hit is always the frame's one program. RF10 (a toon sphere in RF1) still produces no frame on either path. The toon program is untested as a reflection-hit program in production; only R0b's probe has tested it.
4. **One bounce.** Hits take their own untraced specular. RF7 (facing glossy panels) misses at 23.3 vs UE HW-Hit Epic 20.2; its recursive panel-to-panel reflections are missing.
5. **Rough band (roughness ≥ 0.5) is untraced.** RF8's wet rocks miss at 7.9 vs 3.9 (traced 0.3/0.4 bands 6.8/7.5; untraced 0.5 band 9.3). RF2's 0.5/0.7/1 spheres miss at 15.9/13.1/10.4 vs 15.3/11.9/5.5, with no local colour bleed, which needs the cache's bounce in `indirect_radiance` (G2b).
6. **RF2's mirror spheres** miss at roughness 0 and 0.05: 14.1/15.0 vs 11.6/14.4. The reference split noise is 4.8 codes, so this ranking isn't certified.
7. **Glossy grain without TR1.** Four static rays leave visible sparkle on roughness 0.3-0.4 metals that reflect lit and shadowed surfaces (RF2). TR1's rotated strata integrate it (m7-r1.md §5); with TR1 off, the sparkle remains.
8. **Hit lighting is simpler than camera-hit lighting.** At hits:
   - the sun is one hard shadow ray, with no disk penumbra;
   - the indirect inputs read the open environment, never the world radiance cache's occlusion or bounce. A cache lookup per hit cost +0.62 ms of quarry shading with the cache on (m7-r1.md §4);
   - footprints are an isotropic ray cone only;
   - there are no decals until DC1's interface merges.
9. **Motion is unmeasured.** RF3/RF5 motion, trails and settle against R0a's continuous Unreal runs: no new campaign tonight.
10. **Primary-miss background** (environment where the references are black) is R0a's registered renderer item, not a reflection error, but it dominates the full-image difference panels.
11. **Cost.**
   - On reflective content the cost is unmeasured; no comparison with Lumen's 0.07/0.22 ms reflection pass has been made.
   - The quarry traces no reflection rays (every material is roughness 0.85). It pays nothing measurable with the cache off (the default) and **+0.29 ms of shading with the cache on**: the inlined hit shading's static cost (m7-r1.md §4). If G2b makes the cache the default, that becomes a default cost. The candidate fixes are a separate hit-shading pass, or a second program composition with open-only indirect inputs.
12. **RT quarry with the cache on:** one `d1` capture differed from main's program by 309 small pixel clusters with no ray traced. They are presumed to be the cache's run-to-run variation, but no repeat confirmed it (m7-r1.md §4).
13. **Programmability constraint:** in reflecting modules, `surface()` reads the open environment from the indirect inputs; `shade()` reads the prepared values. Reflection hits always read the open environment.
