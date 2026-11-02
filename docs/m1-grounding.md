# Existing renderer grounding

Pstack how report. Baseline implementation at 5b7b2c4; source paths refer to M0 before mesh implementation.

## Overview

At `5b7b2c4050ebc5f388c8cdd66e9f5bde567d320b`, Renderlab is the completed M0 development fixture. It renders analytic spheres and a floor through a fullscreen triangle. Revised M1 implementation has not started. Its purpose is to establish real meshes, depth, resource lifetimes and trustworthy measurements while preserving practical iteration speed. [milestone-1-plan.md:3](/D:/Coding/projects/desktop/renderlab/docs/milestone-1-plan.md:3), [project-handoff.md:25](/D:/Coding/projects/desktop/renderlab/docs/project-handoff.md:25)

The recorded rationale for wgpu is reduced unsafe lifetime and synchronization infrastructure compared with native Vulkan. Separating winit avoids imposing windowing on library users. Explicit binding layouts support transactional reload. The analytic fixture avoided asset preparation while establishing the viewer loop. These are documented decisions, not inferred motives. [milestone-0.md:7](/D:/Coding/projects/desktop/renderlab/docs/milestone-0.md:7)

This explanation distinguishes current behavior, upstream API constraints and unresolved M1 work. No files were changed, builds run or GPU work performed.

## Key Concepts

- `Renderer` owns the device, queue, pipeline layout, bind group, 48-byte uniform buffer, pipeline and color format. Resources persist across ordinary frames. It owns submission and exposes `device()` and `present()`. [src/lib.rs:26](/D:/Coding/projects/desktop/renderlab/src/lib.rs:26)
- `View` owns the `Arc<Window>`, surface, surface configuration, renderer, scene parameters, shader source and interaction/timing state. `App` owns initialization and the shader path. [viewer/src/main.rs:15](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:15)
- `Scene` is six scalar parameters for the analytic fixture. It contains no meshes, matrices, resource handles or object collection. [src/lib.rs:4](/D:/Coding/projects/desktop/renderlab/src/lib.rs:4)
- The caller supplies the color `TextureView` and dimensions to `render()`. The library currently owns no render target or depth attachment. This boundary already supports both surface rendering and offscreen captures. [src/lib.rs:133](/D:/Coding/projects/desktop/renderlab/src/lib.rs:133), [src/lib.rs:175](/D:/Coding/projects/desktop/renderlab/src/lib.rs:175)

## How It Works

**Initialization.** On `resumed`, the viewer creates one 960×640 window, a Vulkan instance, a surface backed by a cloned window `Arc`, and a compatible high-performance adapter. It requests a device with default features, selects an sRGB surface format and FIFO presentation, configures the surface, then creates the renderer. Initial shader or device failures terminate through `expect`; there is no initial fallback pipeline. [viewer/src/main.rs:34](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:34)

**Rendering and submission.** Each `render()` packs dimensions and scene parameters into twelve floats, allocates a byte `Vec`, and calls `queue.write_buffer`. It creates an encoder, clears the supplied color attachment, binds the persistent pipeline and uniform, draws three vertices, and submits the finished commands. The submission index is discarded. [src/lib.rs:133](/D:/Coding/projects/desktop/renderlab/src/lib.rs:133)

The vertex shader synthesizes a fullscreen triangle from `vertex_index`. The fragment shader computes camera rays, analytic intersections and shading. The pipeline has neither vertex-buffer layouts nor depth state. M1 mesh and depth support therefore requires an actual rendering change, not merely different fixture data. [shaders/scene.wgsl:1](/D:/Coding/projects/desktop/renderlab/shaders/scene.wgsl:1), [src/lib.rs:99](/D:/Coding/projects/desktop/renderlab/src/lib.rs:99)

wgpu copies `write_buffer` input into staging immediately, but executes the transfer ahead of commands in the next submission. Thus the temporary CPU bytes can disappear after the write, and the submitted draw sees the queued uniform update. Native staging is itself a temporary allocation retained until submission completion. [wgpu queue.rs:143](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/queue.rs:143)

**Surface lifecycle.** Resize events update dimensions and configure only nonzero sizes. Redraw likewise skips zero dimensions. Successful and suboptimal acquisitions render and present; outdated acquisition reconfigures; timeout and occlusion skip that frame; lost or validation results print a restart diagnostic and exit. Suboptimal acquisition does not currently trigger reconfiguration. [viewer/src/main.rs:89](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:89), [viewer/src/main.rs:160](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:160)

The acquired frame stays local to the redraw handler. After submission, the viewer calls `pre_present_notify()` and `Renderer::present`, which calls wgpu 30’s `Queue::present`. Ordinary frames have no explicit completion wait. Redraws are requested through `about_to_wait`, with a nominal 16 ms wake interval. [viewer/src/main.rs:168](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:168), [viewer/src/main.rs:204](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:204)

**Shader reload.** During drawable redraws, the viewer checks file contents at most every 50 ms. Read failures leave the previous source intact. Changed text becomes the remembered source before compilation, so unchanged invalid text is not retried continuously. A subsequent correction triggers another attempt. [viewer/src/main.rs:127](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:127)

`compile()` creates a shader and candidate pipeline inside a validation error scope, using the existing explicit layout and color format. It synchronously waits for the scope result. `reload()` assigns the candidate only after success, preserving the old pipeline and bindings on failure. Compilation runs on the event thread. The guarantee preserves rendering capability, not a frozen image; parameters and animation can still change. [src/lib.rs:85](/D:/Coding/projects/desktop/renderlab/src/lib.rs:85)

**Measurements.** Startup and successful-reload frames are exceptions to ordinary asynchronous rendering. Before presenting, the viewer explicitly waits for GPU completion. It then logs startup from `main()` and reload latency from shader-file modification time. Pipeline compilation is logged separately. These are completion-plus-presentation-call proxies, not scanout measurements or GPU frame times. [viewer/src/main.rs:138](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:138), [viewer/src/main.rs:168](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:168)

The measurement script records builds, optionally uses a fresh target, runs three no-op builds, edits a library default, rebuilds and launches the viewer, then performs three shader-edit launches. It restores exact source bytes in `finally`. External timings include process creation and log-polling overhead. Historical results are development builds with uncontrolled contention; “cold” means a fresh target with downloads and other caches warm. [measure-dev-loop.ps1:37](/D:/Coding/projects/desktop/renderlab/scripts/measure-dev-loop.ps1:37), [milestone-0.md:27](/D:/Coding/projects/desktop/renderlab/docs/milestone-0.md:27)

## Where Things Live

These are the smallest existing integration points, not a selected architecture.

| Location | Existing responsibility and M1 seam |
|---|---|
| [src/lib.rs:36](/D:/Coding/projects/desktop/renderlab/src/lib.rs:36) | Resource creation and pipeline validation. Natural inspection points for persistent meshes, replacement and creation counters. |
| [src/lib.rs:133](/D:/Coding/projects/desktop/renderlab/src/lib.rs:133) | Upload, pass encoding and submission. Timing instrumentation must identify these separate operations. |
| [viewer/src/main.rs:44](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:44) | Adapter/device negotiation, required for timestamp capabilities. |
| [viewer/src/main.rs:89](/D:/Coding/projects/desktop/renderlab/viewer/src/main.rs:89) | Resize and acquisition boundaries, where target dimensions and presentation measurements meet window behavior. |
| [src/lib.rs:175](/D:/Coding/projects/desktop/renderlab/src/lib.rs:175) | Offscreen image capture and existing reload assertions. |
| [Cargo.toml:11](/D:/Coding/projects/desktop/renderlab/Cargo.toml:11) | Deliberate Vulkan/WGSL dependency selection and development profile. |
| [measure-dev-loop.ps1:42](/D:/Coding/projects/desktop/renderlab/scripts/measure-dev-loop.ps1:42) | Source-edit anchors that will need reconciliation with a mesh fixture. |

## Gotchas

**Asynchronous GPU timestamp readback is absent.** The viewer’s default device request does not enable timestamps. wgpu requires explicitly requested features even when the adapter supports them. Pass-boundary timestamps require `TIMESTAMP_QUERY`; arbitrary encoder or in-pass writes have additional feature requirements. Actual host support and fallback behavior remain unverified. [wgpu device.rs:99](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/device.rs:99), [wgpu-types features.rs:1633](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-types-30.0.1/src/features.rs:1633)

Resolved timestamps occupy eight bytes each and require aligned resolve destinations. Without `MAPPABLE_PRIMARY_BUFFERS`, `MAP_READ` may combine only with `COPY_DST`, constraining direct mapping of query-resolve buffers. Native timestamp differences require multiplication by `Queue::get_timestamp_period()`. [wgpu command_encoder.rs:220](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/command_encoder.rs:220), [wgpu-types buffer.rs:64](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-types-30.0.1/src/buffer.rs:64)

Mapping must follow submission. `map_buffer_on_submit` supplies that ordering. Callbacks need submission or polling to progress and should remain short because they execute inside those calls. Mapped buffers cannot be reused by GPU commands, and mapped views must be dropped before unmapping. [wgpu command_buffer_actions.rs:62](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/command_buffer_actions.rs:62), [wgpu buffer.rs:925](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/buffer.rs:925)

The unresolved design questions are bounded readback capacity, behavior when capacity is exhausted, sample-to-frame association, errors and callback progress while minimized. A pass timestamp interval also excludes CPU preparation, surface acquisition and presentation, and does not automatically include preceding queued uploads. These costs must retain separate labels.

**Safe replacement and bounded retention are different claims.** wgpu-core retains resource trackers with submitted encoders and releases their references after completion. Pipeline references are included. Dropping a Renderlab handle therefore does not immediately free the underlying resource. Explicit destruction differs from dropping; destroying resources referenced by unsubmitted bind groups can invalidate submission. [wgpu-core life.rs:328](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-core-30.0.1/src/device/life.rs:328), [wgpu-core track/mod.rs:639](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-core-30.0.1/src/track/mod.rs:639)

That upstream lifetime mechanism does not prove Renderlab’s future replacement policy stays bounded. Replacement throughput, outstanding submissions, retained application handles and physical driver allocation remain separate concerns. Current image tests drain the GPU after every capture, so they cannot establish replacement safety under sustained in-flight work. [src/lib.rs:216](/D:/Coding/projects/desktop/renderlab/src/lib.rs:216)

**Stable persistent objects do not mean zero allocations.** Current frames allocate uniform bytes, and wgpu allocates write staging. M1 requires static geometry to remain uploaded and movement uploads to be bounded and identified. Accounting must distinguish current owned resources, cumulative creation/churn, retained resources and driver memory. No such counters exist yet. [src/lib.rs:148](/D:/Coding/projects/desktop/renderlab/src/lib.rs:148), [project-handoff.md:47](/D:/Coding/projects/desktop/renderlab/docs/project-handoff.md:47)

**Resize can serialize internally.** wgpu documents that reconfiguration waits for GPU idleness, rejects zero dimensions and cannot occur while an old `SurfaceTexture` remains alive. Consequently, resize-only replacement tests can conceal lifetime mistakes by draining prior work. Depth dimensions and sample compatibility must also follow the actual target. [wgpu surface.rs:103](/C:/Users/snowy/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-30.0.1/src/api/surface.rs:103)

**Real-window evidence remains limited.** M0 records inspection of a failed-reload window and one synthetic camera key event. Resize/minimize and continuous-motion stability were not certified. The code has explicit surface-error exits but no explicit device-recovery implementation; the historical summary should not substitute for testing those paths. [milestone-0.md:19](/D:/Coding/projects/desktop/renderlab/docs/milestone-0.md:19)

**M1’s acceptance contract is fixed; its architecture is open.** Preserve library/window separation, transactional rejection of malformed or incompatible shaders, correction recovery and practical development latency. Add real triangles/cubes, camera/depth correctness, transform-only movement, bounded replacement, clean GPU validation and separated costs without per-frame completion waits. The plan calls for order-independent overlapping-geometry checks, full-image reload equality and actual viewer interaction. Mesh ownership, render graphs, MSAA and timing implementation remain undecided. Importers, GI, advanced lighting, Nanite and Ara integration remain outside M1. [milestone-1-plan.md:18](/D:/Coding/projects/desktop/renderlab/docs/milestone-1-plan.md:18), [project-handoff.md:35](/D:/Coding/projects/desktop/renderlab/docs/project-handoff.md:35), [brief.md:23](/D:/Coding/projects/desktop/renderlab/docs/brief.md:23)
