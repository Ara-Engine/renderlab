# M5 normal-map preparation

This unit exposes reusable CPU preparation through `renderlab-cook::tangent` and `renderlab-cook::normal_map`. `normal_fixture::prepare` consumes both APIs to produce a reproducible owned glTF 2.0 binary fixture. The renderer, shaders, viewer, public asset records, and binary formats are unchanged. Normal-map rendering is still pending.

## Prepared tangent mesh

`tangent::prepare(vertices, indices, source)` returns `PreparedTangentMesh`. Its private fields own base `Vertex` records, remapped `u32` triangle indices, and exactly one `[f32; 4]` tangent per output vertex. Read accessors and `into_parts` expose that single vertex domain. `mode()` distinguishes `Authored` from `MikkTSpace`.

`TangentSource::Authored` preserves valid glTF XYZ and W exactly. It requires one tangent per base vertex, finite unit XYZ, orthogonality to the corresponding unit normal, and W exactly -1 or +1. Squared-length and orthogonality tolerances are `2e-4`. Every triangle must have one handedness sign. Invalid authored data returns an error instead of silently changing the author's basis. Missing authored data selects `GenerateMikkTSpace` explicitly.

Generation uses the pinned `mikktspace 0.3.0` crate. MikkTSpace returns a frame per triangle corner. The runtime-integration review subsequently caught a convention error in this original CPU checkpoint: generated W now negates Mikk's output for glTF, while authored W remains unchanged. The owned Blender exporter plane pins that conversion; see [the correction and evidence](m5-normal-maps.md). The remap key contains the original vertex index and every tangent component, including W. Different corner frames split the base vertex and its tangent together. Opposite signs never average. The remap preserves triangle order and every base attribute at each indexed corner. It drops unreferenced vertices and never welds distinct source indices. Signed zero is canonicalized in the remap key only.

UV and normal discontinuities must already have distinct base vertices. A glTF vertex index names a complete attribute tuple, so its accessor domain already represents those seams. An importer with independent per-corner attributes must expand that tuple before calling this API. MikkTSpace can additionally split a shared index at mirrored-UV handedness or tangent-direction discontinuities.

The input boundary rejects nonfinite attributes, nonunit normals, bad indices, empty meshes, incomplete triangles, and degenerate position triangles. The relative area must exceed `1e-6` times the longest squared edge length. Generation applies the same condition to UV triangles and rejects nonnormal f32 UV determinants or derivative squared lengths. These are declared unsupported numeric domains, not synthesized fallback bases. Small well-shaped triangles remain supported. Valid authored frames can support degenerate UVs because they supply the basis directly. Generation also checks the crate's signed corner-index range and validates every returned frame.

## Linear normal maps

`normal_map::prepare_rgba8` decodes RGB with `2 * byte / 255 - 1`. It ignores alpha and applies no sRGB conversion or Y inversion. `prepare_vectors` accepts finite decoded XYZ vectors. Both return a `PreparedNormalMap` with private, dimension-matched `NormalMip` levels. Base vectors normalize before filtering.

Each mip uses area box coverage of the previous decoded unit vectors, including fractional coverage at odd edges, then normalizes the average. Dimensions halve with floor division and a minimum of one until 1x1. Filtering uses f64 accumulation. A vector of length at most `1e-6` uses the declared `[0, 0, 1]` fallback. This applies to zero base vectors, cancelled averages, and scaled zero samples. Nonfinite input returns an error.

`NormalMip::rgba8` encodes signed XYZ into linear UNORM RGB and writes alpha 255. Quantization means a sampled flat normal is approximately `[0, 0, 1]`, so the runtime must normalize again. Mip construction retains floating-point vectors between levels and quantizes only on export. It does not filter encoded bytes.

`normal_map::apply_scale` provides the tested CPU reference for material scale. It multiplies decoded X and Y by any finite scale, including negative values, preserves Z, then normalizes. Scale is separate material state and is not baked into mips. The glTF convention and scale operation follow the [Khronos normal-texture specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#material-normaltexture) and [normal-texture schema](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/schema/material.normalTextureInfo.schema.json).

Renormalization discards the mean vector length and normal variance. Opposing slopes can produce the same coarse normal as a flat source. There is no roughness adjustment or specular antialiasing compensation. These CPU tests establish direction and unit-length policy, not gloss stability in rendered motion. Signed XYZ input is supported for defensive cancellation handling, even though glTF recommends positive Z normal textures.

## Owned fixture

The deterministic generator creates `normal-mapped.glb`, three prepared mip PNGs, and `preparation.json`. The source contains two triangle pairs with mirrored UVs. The first pair carries six analytical authored tangents. The second starts with four shared vertices and generates six remapped vertices. It has three placements, including scale `[-1.5, 0.75, 0.5]`, and a linear 5x3 normal swatch with positive-Y detail. The material uses UV0 and normal scale 0.5. All geometry and artwork are created for this project.

The rerunnable command uses the allocated cooker target and keeps artifacts ignored:

```powershell
cargo run -p renderlab-cook --example normal_fixture -j2 --offline --locked --target-dir D:/Coding/renderlab-work/m4-stream-followup/target -- D:/Coding/renderlab-work/m5-normal-preparation/fixture
```

The GLB is a source fixture, not an RLA1 payload. The existing importer continues to reject its tangent and normal-texture features. The focused tests decode the GLB with the existing glTF reader and feed its actual positions, normals, UVs, tangents, indices, and decoded PNG back through the production helpers. They compare the prepared results and source byte reproducibility.

## Adapter into the selected v2 schema

The next unit retains the base 48-byte vertex record and adds an optional 16-byte tangent stream. The exact integration sequence is:

1. Keep the existing bounded source, accessor, span, image, and hierarchy validation. In the extended import recipe, accept FLOAT VEC4 `TANGENT` only with the same count as `POSITION`. Material normal textures initially require `texCoord == 0`. Reject unsupported normal texture transforms and extensions explicitly before accessing UVs. Require UV0 and valid normals, or generate and expand flat normals before tangent preparation.
2. Read tangents with `Reader::read_tangents` only when normals are present. Pass usable present tangents as `TangentSource::Authored`; otherwise generate a frame when the material needs one. Without normals, glTF requires ignoring provided tangents. Replace the primitive's base vertices and indices with the returned vectors. Store its tangent vector as `Some(tangents)` in the optional v2 stream. Never attach the returned tangents to the old, unsplit vertex vector or indices.
3. Decode the normal image under the importer's image and aggregate allocation limits. Call `prepare_rgba8`. Charge float preparation storage as well as exported RGBA8 mip bytes. Store each `NormalMip::rgba8` result with its dimensions under the new linear normal-texture role. Preserve sampler state, UV0, and the finite normal scale in the v2 material. Scale must remain unbaked.
4. Version extended regular and virtual payloads, manifests, and cook identities. Include tangent mode, MikkTSpace version, frame tolerances, degeneracy policy, mip recipe, and fixture recipe when relevant. The old importer, settings, cache key algorithm, and RLA1 output remain byte-compatible in this unit. A future v2 adapter must keep frozen v1 keys and namespaces available.
5. Carry tangent seams through virtual simplification and page packing. Charge 16 additional bytes for each tangent-bearing vertex. Pages and runtime mesh bindings must index the remapped base and tangent streams together.
6. In the later runtime implementation, use `N = normalize(A^-T n)`, `T = normalize(A t - N dot(N, A t))`, and `B = W sign(det A) cross(N, T)`. Reorthogonalize after interpolation and define double-sided behavior. The CPU math test checks both W signs with a tilted normal under a mirrored nonuniform matrix against an analytical reference. It does not certify shader behavior.

## Dependency provenance

The only new direct dependency is cooker-local [gltf-rs/mikktspace](https://github.com/gltf-rs/mikktspace), pinned to 0.3.0 with default features disabled and only its `glam` feature enabled. The package is MIT or Apache-2.0. Its original Morten S. Mikkelsen implementation notice remains in the dependency's `src/generated.rs`; attribution and notice restrictions were inspected in the cached source. This project calls the dependency and copies none of that implementation. Source or binary redistribution must retain the applicable dependency notices.

This feature selects cached `glam 0.15.2`, MIT or Apache-2.0, instead of the default nalgebra dependency. It does not replace the renderer's existing `glam 0.30.10`. Cargo.lock records both versions and package checksums. No runtime crate dependency changes are introduced. The project license remains undecided.

## CPU verification

The 12 focused integration tests cover authored preservation and rejection, glTF decode semantics, stable shared indexing, mirror and UV/normal seams, exact per-corner correspondence, both handedness signs, nonuniform mirrored frame reference math, near-degenerate inputs, linear positive-Y decode, alpha independence, XY scale, mip direction and variance loss, cancellation fallback, odd edges, and fixture reproducibility.

Commands and final results are recorded in `.refs/m5-normal-preparation/handoff.md`. Tests use two Cargo jobs and `D:/Coding/renderlab-work/m4-stream-followup/target`. GPU, native viewer, Unreal, runtime normal mapping, motion/gloss quality, virtual tangent simplification, and the v2 format adapter are unrun boundaries of this unit.

Root reviewed the complete tangent/mip implementations, fixture and tests before integration. The integrated 12-test run passes, and the generated GLB is byte-identical to the retained artifact (`8adc2ce8e2b028dbf4c02355a50e8292fbd53b81160a14e1a1cb0b8e4f405658`). Source identities and commands are in `measurements/m5-normal-preparation/root-integration/`. The focused integration test command took 49.075 seconds including recompilation; fixture generation took 0.981 seconds. These are development command costs, not renderer timings. The renderer dependency graph still excludes the cooker, glTF, image codecs and MikkTSpace. Packaged licenses and the original Mikkelsen notice are preserved under `third-party/licenses/`.
