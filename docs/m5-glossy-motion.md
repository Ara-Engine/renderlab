# M5 glossy motion: folded into M7, not implemented

Unit M5-GM, 2026-11-07. Islam stopped it before any implementation, when he authorized [M7](milestone-7.md). Glossy-motion quality now belongs to M7's reflections and temporal reconstruction. This note hands over what the unit's grounding found. Labels: **M** is measured in the cited evidence; **I** is an inference here.

## What was produced

The unit stopped during the reading phase.

- **Not built:** the glossy bench, the motion-integrated reference and the scorer.
- **Not run:** Unreal TSR and no diagnosis measurement of our shimmer.
- **No GPU work:** no lease, schedule reservation, Cargo target or build.

So none of these exist to commit. Nothing below is a new measurement.

## Current state relevant to the shimmer

| Fact | Where | Consequence for glossy motion |
|---|---|---|
| The standard program's material normal renormalizes the sampled, mip-filtered normal | `material_normal` in `shaders/program_api.wgsl` | Normal-map detail below a pixel reaches the BRDF as a single sharp normal (I) |
| Normal mips box-filter unit vectors, then renormalize. Mean length and variance are discarded, with no roughness compensation | `crates/cook/src/normal_map.rs`; [preparation](m5-normal-preparation.md) | A coarse mip loses the variance that Toksvig or LEAN would need. Recovering it requires a format and recipe change at preparation (I) |
| The BRDF uses `alpha = max(rough², 1e-4)` with no normal-variance or geometric-curvature filtering | `brdf` in `shaders/scene.wgsl` | No specular AA on either path. Scan micro-normals at roughness 0.1 alias as highlights (I) |
| The traced path samples with ray-differential footprints. The raster path uses quad derivatives | `SurfaceInput.uv_dx/uv_dy`; `shaders/program_traced.wgsl` | Mip selection exists on both paths. Only the variance is missing (I) |
| Traced Temporal film resets every frame in motion, giving Interactive 1x | `SoftShadows::Temporal` in `src/shadows.rs` | Moving glossy frames get no temporal integration on the RT path (M, by construction) |
| T2 world-anchored records antialias moving silhouettes, but cost 3.5–5.8 ms and are off by default | [T2a](m6-motion-aa-gpu.md) | View-dependent output lags record age, and there is no view-angle cap. This is an open constraint for specular reuse (M) |
| The raster (non-RT) path has 1x/4x MSAA and no temporal history | [M5-A](m5-lighting-plan.md) | MSAA does not filter shading, so highlights inside a pixel still alias (I) |

**The only prior shimmer observation (M, [normal maps](m5-normal-maps.md)).** On the roughness-0.08 fixture at 4x, adjacent mapped frames change by a mean absolute HDR of 0.0400 and 0.0361, with local maxima of 128 and 94.5. Unmapped frames change by 0.00220 and 0.00254. These are raw frame-to-frame changes that include true view response. They do not isolate aliasing.

**Unreal glossy parity is unresolved (M, [setup](unreal-comparison-setup.md)).** On the directional-black fixture, the roughness-0.1 metal interior disagrees by 84.75% relative L1. UE's peaks are 11.5–16.8x weaker than the physical reference predicts, and the cause is unproven. Any matched glossy or TSR comparison must localize or bound this before it can score highlight stability against Unreal.

**S2's coverage ([bench-motion](bench-motion.md)).** S2 has a faceted roughness-0.1 patch, but no textures, normal maps or environment. Its reference and metrics target shadow visibility on crops. UE already runs TSR Epic with 200% history there, so the S2 Unreal project and scripts are the cheapest base for a glossy variant (I).

## Plan the unit would have followed (unvalidated proposal for M7)

- **Bench:** extend S2's shared scene/track JSON with a glossy variant. It would add a normal-mapped low-roughness scan crop, small bright highlights from a near-grazing sun, camera tracks at 0.25–1 px/frame, and a moving-light track. The same manifest drives both engines.
- **Reference:** per output frame, many stratified jittered sub-frames over the pixel footprint, with a shutter parameter. Zero shutter matches S2 and UE with motion blur off. Shutter-integrated sequences match M7's "motion-integrated" references. Decide which one each gate uses before capture.
- **Metrics:** per-frame HDR and display error. For flicker, temporal-difference error against the reference: RMS of `(c_t − c_{t−1}) − (r_t − r_{t−1})`. For highlight stability, the reference-masked highlight energy and peak-position drift over time. Score each on both paths and against UE TSR.
- **Candidate order:**
  1. Variance-filtered roughness: Toksvig for mips, Kaplanyan/Tokuyoshi for geometric curvature. It would be a standard-program helper that custom programs opt into, and adds nothing to receiver data or dispatch.
  2. LEAN/LEADR-style variance stored at preparation.
  3. Only then, temporal reuse.

  The first two are inside-the-pixel filters, independent of M7's temporal design (I).

## Handover

- **Branch:** `t3/m5-glossy-motion` on main `6b6c84a`, containing only this note.
- **Left alone:** `docs/milestones-5-6-decisions.tsv` and all source, shader and test files.
- **Time:** reading only. No implementation and no verification time.
