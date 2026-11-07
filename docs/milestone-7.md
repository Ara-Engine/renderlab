# M7: global illumination, reflections and temporal reconstruction

Authorized by Islam on 2026-11-07, at main `cf001a5`. M7 runs in the main checkout. The M5-M6 shadow hillclimb continues alongside it in an isolated worktree (see [milestones-5-6.md](milestones-5-6.md)).

## Scope

- Indirect diffuse light (bounce GI) and sky/environment visibility.
- Glossy and specular reflections.
- Temporal reconstruction of the final image under motion.

Two M5 items fold in here, because GI and reflections replace the mechanisms they were going to use: environment visibility and glossy-motion quality. Transparency, volumes and atmosphere are out of scope unless a later update adds them.

## Acceptance

- **Quality floor:** Unreal 5.7 quality on every measure, static and in motion. That includes disocclusion, camera cuts, and moving lights and casters. Film quality is the target. References are path-traced and pixel-integrated, and include motion-integrated sequences.
- **Unreal comparators:** Lumen software and hardware GI, Lumen reflections and TSR, on matched scenes, cameras, lights and materials. Unreal 5.7.4 CL51494982 runs offscreen with no visible window.
- **Performance floor:** at least 2x Unreal at matched quality, measured end to end on both paths. Every speed claim gets a final quiet-window run.
- **Both paths are first-class.** Every gate is measured with hardware RT and without. Non-RT GI should rival other engines' RT GI.
- **No screen-space core lighting:** no SSGI or SSR as the core answer. Caching, temporal reuse, denoising and approximations are allowed when the final image passes.
- **Shading stays user-programmable.** GI and reflection inputs reach user programs as clean values. Record any constraint a design imposes.
- **Memory:** total VRAM stays below Unreal's on the matched scene.
- **Licences:** copy no Epic, Unity or NVIDIA RTX SDK code. ReSTIR- and SVGF-family methods need a patent review first.

## Execution

1. Design first, on CPU only. Write competing sketches and a measured plan against Unreal 5.7. Ground the design in docs/research/gi.md, reflections.md and reuse-temporal-denoise.md once they land, plus the existing non-RT machinery: the surface-keyed cache, cluster LOD and the virtual clipmap.
2. Implement in the order the design records, as separately measurable units, each priced per stage.
3. Check once per retained change, in batched runs on the shared GPU lease.
4. Push validated checkpoints. Record decisions and misses in the decisions trail.

## Ownership

- **Main checkout (M7):** lighting, environment, shading, and the new GI, reflection and temporal files.
- **Hillclimb worktree:** shadow files only. It doesn't edit M7's files.
