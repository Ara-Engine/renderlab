# M6 first shadow adapter

The first hard-shadow adapter compiles and passes its focused GPU, ownership, reload and retained material/lighting gates on the RTX 4070 SUPER. Complete fixture and quarry images were inspected on 2026-11-03/04. The default 1024 maps fail quarry image quality through false stippled shadows. A 4096-map configuration improves the inspected images and four probed source-visibility samples, but does not establish global correction. This is partial M6, with no Unreal parity or performance claim.

The adapter follows [the selected design](m5-lighting-plan.md) and [the CPU planner contract](m6-shadow-plan.md). It renders nearest-depth hard shadows into a single-sample Depth32Float array. Directional requests supply explicit cascade ends. Point requests create six faces. Spot requests create one perspective view. Every admitted lit frame redraws all requested views. Diagnostic and disabled frames perform no shadow fitting, raster passes or matrix uploads. Initialization uses a small inert texture/buffer binding to keep one lighting ABI. Once enabled, the last full generation remains allocated and charged after disabling, until replacement or renderer drop; three 4096 layers retain about 201 MB.

## Caller contract

`Shadows::requests` corresponds by position to the complete current `Lighting::lights` vector. A nonempty vector must have exactly the same length and matching directional, point or spot variants. `ShadowRequest::None` explicitly disables an individual light; an empty request vector disables the whole set. `set_lighting` preserves and validates the current shadow requests. `set_lighting_and_shadows` replaces both atomically when changing the set's shape. Failed validation preserves the prior settings. There are no light IDs or public render graph.

```rust
renderer.set_lighting_and_shadows(&lighting, &Shadows {
    requests: vec![ShadowRequest::Directional {
        split_ends: vec![4.0, 12.0, 40.0],
    }],
    resolution: 1024,
    ..Default::default()
})?;
```

Coverage distances are metres. Directional distance is positive camera-forward depth. Local near/far coverage is radial and independent of photometric attenuation. Outside the requested distance the light is unshadowed. Spot outer half angles of pi/2 remain supported by M5 lighting, but a shadow request rejects them rather than narrowing the cone. Submitted caster bounds intersecting a local light's near sphere reject the frame before queue writes. This conservative test can reject a light in empty space inside a loose object bound. It never treats the 1 cm attenuation regularization as evidence that near casters are absent.

The shared resolution must be at least two and fit device dimensions. The view count must fit array-layer, storage-binding and uniform-buffer limits. `shadow_limits()` reports the effective device bounds. `max_bytes` defaults to 256 MiB per map/matrix resource generation and accepts `None` to remove that policy. It includes depth texels, aligned raster uniforms and sampling records. Retired in-flight generations are charged separately in `Stats::ownership.shadow_bytes`; the policy is not a total driver-memory limit. No request is silently truncated to fit.

`depth_bias` is normalized map depth in [0,1). `normal_bias` is a nonnegative geometric-normal receiver offset in metres, oriented toward the light. Defaults are 0.00002 and 0.001 m. The nearest lookup compares the receiver's projected geometric plane at the actual sampled texel centre, with a small slope allowance for one 1/16-texel subpixel step. This fixed the observed planar self-shadowing, including an approximately 86-degree grazing receiver. It is not a proof for arbitrary raster geometry. The four-bit minimum raster precision is documented in [Vulkan's required limits](https://docs.vulkan.org/refpages/latest/refpages/source/Required_Limits.html); receiver-plane comparison is described in [Microsoft's shadow-map guidance](https://learn.microsoft.com/en-us/windows/desktop/DxTechArts/cascaded-shadow-maps). All maps use one resolution. Per-light resolution packing is deferred.

Perspective depth bias depends strongly on the chosen near plane. The evidence example now exposes `--shadow-near`, defaulting to 0.05 m. Its earlier 0.0001 m near plane gave an excessive effective world-space offset at metre distances. Caller-requested near coverage remains explicit and validated; there is no hidden widening or reduction of the coverage domain.

Viewer lighting JSON may include the following additional field. Omission disables shadows. The whole lighting/shadow document validates and stages together; malformed documents retain the last valid settings. The shared parser and file polling live in `support/lighting_controls.rs`. `support/lighting_settings.rs` preserves the frozen M5 tool's lighting-only entry point and rejects nonempty shadow requests. There is one JSON implementation, with no new warning suppression.

```json
"shadows": {
  "requests": [{"type": "directional", "split_ends": [4.0, 12.0, 40.0]}],
  "resolution": 1024,
  "depth_bias": 0.00002,
  "normal_bias": 0.001,
  "max_bytes": 268435456
}
```

Local entries use `{"type":"point","near":0.001,"far":30.0}` or `spot`. Disabled entries use `{"type":"none"}`. The lighting-only tooling parser explicitly rejects nonempty shadow requests; it does not silently ignore coverage.

## Ownership and execution

`Renderer::ShadowState` owns requests, array layers, sampling records, disjoint aligned view-uniform ranges, the prior admitted light bases and the published generation. The raster and sampling buffers receive identical authoritative world-to-clip matrix bytes. Resource replacements retain the old generation until its last possible queue use completes. Both regular and virtual rendering enter the existing `submit_prepared` owner. There is no per-frame completion wait in the renderer.

Regular caster bounds come from every submitted mesh and transform slot before camera culling. F64 affine corner evaluation includes an outward guard for actual f32 multiplication. Shadow draws address original transform slots through the vertex instance index, independently of camera-visible ID buffers. Source positions and mirrored/double-sided winding are shared with the camera path.

Virtual shadows use the complete current published cut for every placement. Conservative hierarchy bounds are used for planning, and every range is drawn from the same cut as the camera. Existing post-submit page stamping covers both consumers. No independent shadow cut exists. **Light-view refinement demand is not implemented.** Distant/offscreen shadow geometry may remain too coarse, so this adapter cannot establish M6 completion or arbitrary-world shadow quality.

Directional and spot light bases transport the last admitted orientation under motion. This removes the least-aligned helper-axis switch for small light changes. A discontinuous antipodal light edit uses the prior right axis. CPU continuity checks and GPU images cross the helper-axis tie. The focused GPU regression also moves the receiver through a cascade boundary with a 10 cm-wide caster and a grazing receiver, at 1x and 4x. Hard edge changes across cascades remain visible; these short cases do not certify general motion stability.

The single WGSL reload builds diagnostic, lit, presentation and shadow pipelines as one bundle. Shadow syntax, interface or entry-point failures reject the whole replacement. The shared world-position helper keeps camera and depth geometry aligned. Changes that displace geometry still require the existing conservative-bound contract; shader validation cannot prove geometric bounds.

## Observations and verification

`CpuFrame` separates shadow bounds/fitting/assignment, resource allocation/matrix upload, and pass encoding. These intervals are excluded from the existing upload/encode fields. GPU query readback separates first-to-last shadow generation, mesh shading and HDR presentation. `render_span` directly measures the first rendered pass beginning through the final pass end. It is not the sum of pass timers. Uploads, presentation and acquisition remain outside that GPU span. Hard lookup cost is inseparable from the mesh pass. No cache cost exists.

`examples/m6-shadows.rs` prepares the owned box/slab fixture or reuses the cooked twelve-scan quarry. It captures HDR, PNG and settings/camera/asset identities at 1x or 4x, with separate camera/sun/spot/point/blocker motion controls. Its image/paired-sample path drains for capture and sample identification. Those paced pass samples do not establish pipelined frame throughput. `--cost` compares disabled/enabled/enabled/disabled shadows under the same lit shader and now captures both disabled and enabled images for inspection.

`scripts/m6-shadow-check.ps1 -Stage Source` runs only scoped formatting, a cached-dependency CPU harness and Naga validation. The harness includes the actual planner/policy sources, the unchanged Camera implementation and verbatim affine-bound/near-domain functions. It substitutes the small Error/Aabb shells those functions need. It is not a renderer typecheck or device/pipeline-layout test.

Retained device evidence is in `.refs/m6-gpu/verified-handoff.md`. `-Stage Fixture` passes 18 CPU checks and four focused GPU tests. These cover offscreen directional/point/spot blockers, a point-face seam, original/mirrored/reversed/double-sided winding, moving blockers, whole-image unblocked planar equality, camera cascade and light-basis crossings, thin/grazing geometry, transactional reload/settings, exact regular/virtual images, busy admission, near-domain rejection and generation retirement. `-Stage Retained` passes affected all-target Clippy, 49 library CPU tests and nine relevant historical GPU tests, including the displacement/culling oracle. Vulkan validation was positively active with zero reported errors. Ordinary warnings about unused vertex attributes in the position-only shadow shader are retained in the logs.

Inspected complete images and contact sheets are in ignored `measurements/m6-gpu/final-fixture-*/`, `final-fixture-motion-sheet.png`, `planar-motion-sheet.png`, `camera-tie-sheet.png`, `quarry-4096-*/` and `quarry-motion-sheet.png`. The fixture has three frames for each camera/sun/spot/point/blocker motion and a static control, at both sample counts. A separate three-pose camera X/Z component-tie crossing also stays coherent at 1x/4x, within the valid caller-supplied-up look-at domain. Every captured HDR image reports zero nonfinite and zero resolved saturated texels. The 4x resolve does not detect all possible per-sample saturation.

The unchanged quarry asset hash is `1ce53c381b467345addc1dce5a25fad45435bd90cb12ad2a0d14bb7a593257f3`. It contains twelve meshes/placements, 13,034,903 source triangles and twelve 4096-square base textures with their mip chains. The full-resident path uses 1920x1080 output, 16x anisotropy and the same prepared-asset limits and lighting preset as the M5 evidence. No recooking or scan normal/material format change occurred.

The 1024-map quarry images have false shadow stippling over the floor and rocks. An independent f64 ray/triangle probe of four selected floor pixels finds three unobstructed rays and one real blocker 0.0373 m away. At 4096 and 1x, the three unobstructed pixels exactly match the disabled image, while the real blocker remains shadowed. Sparse dark speckle remains visible on the pale sand and plateau in the 4096 pose-0 image; whether those pixels represent real micro-occlusion or false shadowing is unverified. This small selected probe diagnoses the 1024 failure and supports the observed resolution improvement. It does not certify the rest of the scan, contact bias, or global temporal stability. Both failed and improved complete images are retained.

Short feature-cost observations use the Ryzen 7 7800X3D, RTX 4070 SUPER, NVIDIA 617.14 and Vulkan validation, in an optimized build. Each pose runs disabled/enabled/enabled/disabled. The table summarizes poses 1 and 2, four samples per state; pose 0 and all raw rows remain recorded. The camera pass draws all 13,034,903 triangles in twelve draws. Three 4096 maps assign 27,069,338 caster triangles in 25 shadow draws and own 201,327,696 bytes per generation. These are small, variable, paced observations. The exact hardware limiter was not profiled, and the first 4096 startup overlapped the tail of a two-job Clippy run. They do not establish stable throughput or a resolution ranking.

| Motion / AA | GPU span off/on median, ms | Shadow generation median [range], ms | CPU render/admission off/on median, ms |
| --- | ---: | ---: | ---: |
| Camera, 1x | 6.914 / 17.020 | 10.069 [9.558, 11.981] | .472 / 1.353 |
| Camera, 4x | 8.745 / 16.656 | 7.916 [7.669, 8.471] | .482 / 1.366 |
| Sun, 4x | 5.326 / 11.430 | 6.184 [5.892, 6.288] | .522 / 1.323 |

Shadow planning/upload/encoding CPU medians are respectively .018/.166/.020 ms, .019/.162/.025 ms and .020/.163/.024 ms for those rows. Mesh lookup cost remains inside mesh shading. No filtering or cache maintenance exists to price independently. The render span excludes uploads and native acquisition/presentation. Readback drains belong to the evidence tool. Exact ranges, source/binary IDs and commands are in the ignored handoff and JSON records.

A bounded background native viewer run accepted lighting/shadow edits, rejected malformed and mismatched settings, rejected a missing shadow shader entry point while continuing frames, installed a corrected bundle, and observed 1/5/arrow controls. It exited normally with active clean validation. PID-bound window capture shows the initial owned viewer, but repeated window captures remained byte-identical even after tint/camera changes, including a delayed retry. Those captures are stale for later states and cannot certify visible native reload or motion. The live logs and headless image tests are separate evidence. No foreground activation, physical input, desktop/game capture or foreign-app shutdown occurred.

## Acceptance gaps

Independent Opus review accepted this as a bounded partial checkpoint, with no blocking source defect. Root verified 24 source/binary identities and inspected full fixture and quarry images. The final source differs from the device-tested shader and resource owner only in comments clarifying receiver-plane rounding, a reserved texel field and disabled-state memory; no executable expression changed. The offscreen-caster GPU checks exercise the regular path. The exact-cut virtual test does not include an offscreen placement, so that case remains unverified.

There is no PCF, filtered point seam reconstruction, overlapping cascade transition band or finite-source softness. The inspected thin/grazing and cascade images retain stair steps and changes in edge sampling. The default 1024 quarry quality miss remains explicit. The 4096 captures are an improvement, not complete source-visibility or motion acceptance. Light-view refinement, distant/thin virtual-caster LOD quality, broad contact/distance references and visible native reload/motion verification remain open. No Unreal parity or speed claim is made.
