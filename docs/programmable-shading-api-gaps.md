# Programmable shading: caller files, defaults, arrays and normal mapping

2026-11-05, based on `44014cf`. This closes the authoring gaps listed in
[phase 2](programmable-shading-phase-2.md#api-gaps-encountered). Visibility admission,
mixed traced dispatch and post programs retain their existing contracts.

## Register and watch a user program

`renderlab::program_watch::ShaderWatch` is now public and caller-owned. The viewer
uses this same implementation, registering its `--program` file against its
program handle. `--program shaders/programs/toon.wgsl --view lit` works without an
application-side toon parameter list. The viewer still uses its standard handle
for that override; this adds no material-selection UI.

```rust
let source = std::fs::read_to_string(&path)?;
let program = renderer.try_create_program(&renderlab::ProgramDescriptor {
    label: "my-style", source: &source,
})?;
let mut watch = renderlab::program_watch::ShaderWatch::programs_only();
watch.watch_program(&program, &path, &source);
// Optional engine prelude, always polled after programs:
watch.watch_prelude(prelude_path, &accepted_prelude);

// In the application's update loop (the viewer polls every 50 ms):
for reload in watch.poll(&mut renderer) {
    match reload.result {
        Ok(report) => println!("{}: {:?}", reload.path.display(), report.compile),
        Err(error) => eprintln!("{}: {error}", reload.path.display()),
    }
}
```

Any number of owned handles can be registered. Registering the same handle again
replaces its watch. Registration performs no I/O or compilation; the supplied
source must be the source last accepted by that handle's renderer. Unchanged text
does nothing. Each changed file has its own reload transaction, and a rejected
file is retried after another file successfully reloads (including a prelude that
adds a required helper). Missing/unreadable files are skipped until readable,
matching the existing viewer behavior. The watcher retains its program handles;
dropping it releases those references. It starts no threads or services.

## Declare defaults and fixed arrays in WGSL

```wgsl
struct Parameters {
    thresholds: array<vec4<f32>, 2>,
    tint: vec3<f32>, gain: f32,
};
const DEFAULT_PARAMETERS: Parameters = Parameters(
    array<vec4<f32>, 2>(
        vec4<f32>(0.1, 0.2, 0.35, 0.5),
        vec4<f32>(0.65, 0.8, 0.9, 1.0)),
    vec3<f32>(0.2, 0.4, 0.8), 1.0);
// Read an arbitrary threshold as params.thresholds[index / 4u][index % 4u].
```

Defaults are an optional ordinary WGSL `const` named `DEFAULT_PARAMETERS`, of
type `Parameters`. naga evaluates constant expressions; Renderlab packs the
evaluated IR using naga's member offsets and array strides. An absent constant
keeps the previous zero defaults. Toon now declares its original seven values in
its program file. Programs using only zero defaults need no source change.

```rust
let Some(material) = renderer.try_create_material(&renderlab::MaterialDescriptor {
    program: Some(&program), // parameters: &[] uses the program's defaults
    ..Default::default()
})? else { return Ok(()); }; // retry after upload admission becomes available
// Only these fields override the defaults. This is still a buffer write.
renderer.try_update_parameters(&material, &[
    ("gain", renderlab::Value::F32(0.75)),
    ("thresholds", renderlab::Value::Array(vec![
        renderlab::Value::Vec4([0.05, 0.15, 0.3, 0.45]),
        renderlab::Value::Vec4([0.6, 0.75, 0.9, 1.0]),
    ])),
])?;
```

Arrays have an exact reflected length and element type. Wrong length, mixed
element types, unknown names and wrong member types reject the entire material
creation/update before writes. Support is one-dimensional fixed arrays of the
existing scalar/vector types, within the existing 256-byte block cap and WGSL
uniform layout. In particular, `array<vec3<f32>, N>` and
`array<vec4<f32>, N>` have 16-byte strides. Plain `array<f32, N>` and
`array<vec2<f32>, N>` fail uniform validation; use packed vec4s for scalar
thresholds. No device feature, bind layout, dispatch or hit record changed.
`Value` is now `Clone`, rather than `Copy`, because it owns array elements.

Live materials retain explicit overrides separately from defaults. On a successful
reload, compatible overrides survive by name; omitted, new or retyped members
read the new defaults. Incompatible overrides are discarded and cannot reappear
on a subsequent revert. Editing just a default repacks materials even if the
schema is unchanged. Prelude reloads also repack when a default uses a changed
engine constant. `ReloadReport::zeroed_parameters` lists new/retyped fields with
zero defaults; fields with nonzero defaults are excluded. Rejected reloads install
neither pipelines nor parameter blocks. Diagnostics name program source lines,
including invalid uniform-array layout.

## Share normal-map reconstruction

```wgsl
fn surface(input: SurfaceInput) -> Surface {
    let n = material_normal(input);
    // Use n with the program's own Surface and lighting model.
    return Surface(n);
}
```

`material_normal(input)` is in the shared program prelude. It samples the normal
map and applies the standard program's exact tangent repair, handedness, finite
scale arithmetic, fallback normals and double-sided source-back reversal. Call
it uniformly on raster paths, as with the mesh-UV sampling helpers. Programs can
still choose geometric or interpolated normals, or their own reconstruction.
Standard calls this helper; toon retains its intentionally unperturbed normals.

## Verification

Evidence, images, manifests, build logs and GPU lease records are under
`E:/renderlab-work/program-api-gaps/`. `summary.json` and `raster-suites.json`
record results; every run includes its binary hash, manifest, preflight, test log
and acquired/released GPU lease times. Final gate binary SHA-256:
`1d0ecd41ce2d6bc57636b35ed2b0f1a0d072ae8109a1a2a7599a067ebb859a52`.

Windows/Vulkan, RTX 4070 SUPER 12 GB, NVIDIA 617.14, Ryzen 7 7800X3D,
wgpu/naga 30.0.1, release tests, two Cargo jobs. Quarry and material fixture,
camera/lighting/environment, resolutions and quality match the
[phase-2 gate](programmable-shading-phase-2.md#gate-evidence). No rendering
algorithm, sampling budget or quality setting changed.

| Gate | Result |
|---|---|
| Primary acceptance | 285/285 complete files byte-identical to immutable current-main hashes |
| Interactive acceptance | 36/36 byte-identical; 32 static repeats, 384 comparisons pass |
| Standard/toon pairs | 72/72 HDR/count files per scene match phase 2; 36 style visibility pairs identical |
| Toon static/return | 1,152 static HDR/count comparisons pass; return frames exact on every path/seed |
| Raster lit suites | All 18 pass; all 385 captured readbacks byte-identical to current-main hashes |
| API boundary | Both paths: default-only materials, array padding/updates, wrong length/type rejection, atomic invalid updates, explicit override preservation, retyping, live file edits, line-mapped rejection, exact revert and prelude-dependent retry/default updates pass |
| CPU | 88 library tests; library/viewer all-target Clippy with `-D warnings`, formatting and diff checks pass |
| GPU validation | Core + Synchronization active, zero errors in all 29 completed processes |

The primary, Interactive and raster hash oracles are the saved
`E:/renderlab-work/env-dedup/runs/candidate-{primary,static,suite}-hashes.json`
maps. Toon hashes come from phase 2's `fixture-1/run.json` and `quarry-2/run.json`.
Interactive's compacted count inputs were regenerated from the paired quarry
gate, whose complete count/HDR hashes independently match the immutable oracles;
the separate 36-file comparison remains its regression check.

Verified visually: `fixture-{overview,crops,motion}.png` and
`quarry-{overview,crops,motion}.png`, including all six path/seed combinations at
0 → 1 → return. Bands, blue rim, silhouettes and floor/rock contacts retain the
phase-2 appearance. Existing 1x aliasing and Interactive grain remain. This is
byte-preservation evidence, not a new final-quality or Unreal acceptance claim.
The viewer was checked by compilation and its shared watcher was exercised
offscreen; no viewer window was launched.

### Registered-file edit latency

`gpu_registered_program_files_defaults_arrays` registers a caller-created program
with the public watcher. It writes a process-unique **live shading multiplier**,
waits 25 ms within the viewer's 50 ms polling interval, polls, reloads, then renders
and completes an offscreen image. One sample per path, 64×64 indexed quad with a
flat parameter-driven shader, 1x sampling, validation on, pipeline cache enabled.
It does not use the quarry's environment or lighting program. Each edit creates
exactly its two active pipelines, changes the image and preserves exact revert
bytes. No presentation or scanout is measured.

| Path | Save → detect | Compile | Save → completed image |
|---|---:|---:|---:|
| Raster | 25.66 ms | 23.99 ms | 51.14 ms |
| Traced | 25.55 ms | 50.76 ms | 78.28 ms |

These are single-sample development-loop observations on this small shader, not
evidence that the quarry's 100 ms target is met. Two earlier API runs changed
only the defaults constant, which could reuse identical GPU code; their latency
numbers are excluded. `api-live-1/registered-reload.json` holds the retained
measurements. No competing game, Unreal Editor or renderlab GPU job appeared in
the admitted preflights or monitors. The other worker's CPU build contention was
not separately sampled, so these are not controlled performance benchmarks.

### Time and cleanup

Implementation (reading, API design, edits and documentation) was interleaved
with verification; active implementation time was not independently stopwatched.
Verification: six successful release compile/check cycles total 331.61 s,
including a 132 s cold test build. Viewer check and successful Clippy runs total
45.99 s; the final CPU suite takes 0.58 s. GPU children total 430.29 s. Image
analysis/inspection is separate from those execution times.

All 29 GPU leases were released: **14:37:54.898300–14:53:58.436160 UTC**,
486.26 s held in total, at most 79.48 s per lease. Each lease used atomic mkdir
and `owner.json`, admitted at least 6 GiB available RAM (minimum 7,560 MiB), and
checked for games, Unreal Editor and foreign renderlab GPU jobs before launch
and during execution. Two occupied-slot deferrals launched no GPU work. Every
test ran offscreen/background. See `gpu-windows.json` for individual times.

Bulk raw captures are summarized by hash before removal; inspected PNGs and
JSON/log summaries remain on E:. The worker's own E: Cargo target is removed
after verification; `cleanup.json` records the result. No generated evidence or
private asset is committed.
