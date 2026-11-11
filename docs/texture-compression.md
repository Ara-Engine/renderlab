# Block-compressed textures (U-BC)

Unit U-BC, 2026-11-11. Row 5 of [ara-render-replacement.md](ara-render-replacement.md): Ara's cooked textures are BC7 and BC5, and reached Renderlab decoded to RGBA8. A prepared texture can now carry the blocks themselves.

## Status: parked, never compiled (2026-11-11, 18:56 UTC)

The coordinator stopped this unit before its first build: the PC's build slot went to the Ara integration thread, root's flipbook capture, U-IDS and then a picture defect. Branch `t3/bc` holds everything below as written. **No line of it has been compiled, no test has run, and nothing has been on the GPU.** The rest of this page describes the design as written, not behaviour that was observed.

- **Built (written, formatted with rustfmt, read through once):** the texture format and its validation in `crates/asset`; `.rla` schema 7; `src/texture_bc.rs` (how a device holds a texture, the level writer, the decode, test fixtures); the upload and streamed-residency paths using it; the engine's optional BC device feature; the two-channel normal rebuild in the shaders; the dependency and its notices.
- **Tested:** nothing. Written but never run: CPU unit tests in `texture_bc.rs`, `material.rs`, `texture_residency.rs` and `crates/asset/tests/format.rs`, and five ignored GPU tests in `src/texture_bc_tests.rs` (decode against the GPU, the fixture on three renderer paths, a streamed camera cut on two, the engine on both paths, memory of three 4096 maps).
- **Not done:** every acceptance measurement in the brief (fixture within one code, memory before and after, decoder against the GPU, lib suite, default path byte-identical on the game scene); a picture check; `Cargo.lock` does not yet list `bcdec_rs` (the first build adds it, so `--locked` builds of this branch fail until then); the Ara bridge still decodes to RGBA8.
- **Known risks to look at first:** compile errors (about 30 texture literals were edited by script, and the tests were written without a compiler); the BC5 fixture comparison may exceed one code in sharp highlights, because BC5 keeps more than eight bits where its RGBA8 decode does not; the engine comparison with every default on may not be repeatable to one code; one added branch in the normal-map helpers has not been timed on the default path.

**Exact next step.** Merge main into `t3/bc`. Then, with the build slot, from the repository root (the runner is tools/u-bc/host.py; logs and results go to `D:/Coding/renderlab-work/unit-bc`):

1. `py -3 -B tools/u-bc/host.py build check1 -- cargo test -j 2 --no-run -p renderlab --lib -p renderlab-asset -p renderlab-cook --tests` (not `--locked`: this adds `bcdec_rs` to Cargo.lock; commit the lock). Fix what the compiler reports.
2. Leases of at most 15 minutes, in this order: `py -3 -B tools/u-bc/host.py lease bc-a 15 -- py -3 -B tools/u-bc/run_steps.py a` (decode, memory, fixture), then the same with `bc-b` and `run_steps.py b` (residency cut, engine), then `bc-lib` and `run_steps.py lib` (the lib suite and the asset and cook tests). Results land in the work folder's `out/`; look at `out/fixture-*-bc.png` and `out/engine-*-bc.png`.
3. `py -3 -B tools/u-bc/host.py build game -- cargo build --release -j 2 --example game_scene`, copy the exe to the work folder's `bin/game_scene.exe`, then lease `bc-game` with `run_steps.py game`: 40 frames a path against main's saved frames in `D:/Coding/renderlab-work/root-check/fix-game` (valid only while main's default frames are unchanged; if the `Disk` sun lookup has become the default, save new main frames first).
4. Fill in the results here, add the decision-table row, then change the Ara bridge's `texture::prepare` to pass each KTX2 level's blocks (`Bc7`/`Bc7Srgb` as `TextureFormat::Bc7` under the matching role, `Bc5` as `TextureFormat::Bc5` under `NormalLinear`) instead of decoding and re-filtering; it keeps `decode_texture` for mean colours, alpha masks and the splat atlas.

tools/u-bc/host.py is derived from tools/p-flicker/host.py (owner `u-bc`, admission before locks, 10 GiB start bar, yields to pending Ara and U-IDS requests) and still carries a gate on root's `root-flipbook4` release line, which is met. Neither script has run a build or a lease yet.

## What a caller sends

`renderlab_asset::Texture` (the engine's `TextureData`) has a `format` beside its `role`:

| `TextureFormat` | Level bytes | Roles |
|---|---|---|
| `Uncompressed` | the role's texels: RGBA8, or RGBA16Float pairs for slope moments | all |
| `Bc7` | 16-byte blocks of 4x4 texels | base colour (sRGB), metallic-roughness, three-channel normals, normals with variance |
| `Bc5` | 16-byte blocks of 4x4 texels, two channels | `NormalLinear` only: tangent-space X in R and Y in G |

- The role still says what the channels mean and whether colour is sRGB, so there is no separate sRGB format.
- A level of a compressed texture is `ceil(w/4) * ceil(h/4)` blocks in row order; a level under 4 texels is one block. Level sizes halve as before (rounded down, at least 1).
- A compressed chain may stop at any level. An uncompressed chain still ends at 1x1.
- BC4 is not a format: no role is a single channel (metallic-roughness reads G and B). The decoder dependency handles BC4 blocks, so adding it with a single-channel role is a table entry.

`Engine::create_texture` and `Renderer::try_upload_texture` take these as they took RGBA8. Nothing else in the API changed; `Mip::rgba8` keeps its name and now holds the level in the texture's format.

## What the device holds

- **With `TEXTURE_COMPRESSION_BC`** (`Engine` requests it wherever the adapter has it; the RTX and RADV do) and a level 0 of whole blocks: the blocks, as `Bc7RgbaUnormSrgb`, `Bc7RgbaUnorm` or `Bc5RgUnorm`. wgpu refuses a compressed texture whose level 0 is not a multiple of 4 texels, so such a texture takes the other route.
- **Otherwise**: each level is decoded to RGBA8 as it is written (`texture_bc.rs`, the `bcdec_rs` crate), so callers never branch on the device. The retained source of a streamed texture stays compressed; a level is decoded again when it becomes resident.
- `Limits::max_texture_bytes` bounds what the device holds, so a texture that fits as blocks can be refused on a device that has to decode it.
- `renderlab::decode_texture` is the same decode for callers that need texels (a mean colour, an alpha mask).

## Two-channel normals

A BC5 normal map stores X and Y. Z is rebuilt as `sqrt(1 - X^2 - Y^2)` in the shader from the filtered sample (`rl_normal_texel`, shaders/program_api.wgsl), on both kinds of device: a decoded BC5 map is still shaded from its X and Y. The material's `normal_mapped` word is 2 for such a map (1 for three channels), and every normal-map helper (`sample_normal_map`, `sample_normal_map_grad`, `material_normal`, the specular filter) returns the three-channel form. A program author changes nothing.

Two consequences, both recorded rather than hidden:
- Rebuilding Z after filtering keeps the normal unit length. An RGBA8 map filters a rounded Z and its filtered normal is shorter where neighbouring texels disagree. On smooth maps the two agree to within the precision of RGBA8; on sharp ones the BC5 form is the more exact.
- `material_normal_variance` reads that shortening as normal spread for maps without stored variance. A BC5 map has no shortening to read, so it reports no spread. The standard program's default path does not use it (`SPECULAR_AA` is off).

## Streamed residency

Compressed chains stream as blocks. The rules of [memory-m3.md](memory-m3.md) hold unchanged (the probe, the one-level margin for anisotropic samplers), with two additions:
- A tail of sampled blocks is itself a compressed texture, so its level 0 stays at least 4 texels along each axis (`max_top`).
- Copies between tails cover whole blocks: a level under 4 texels copies one block.

## Prepared assets and the cook tool

The `.rla` encoding gains schema 7, written only when a texture is compressed ([asset-format.md](asset-format.md)). Assets without compressed textures keep their schema and bytes, so every existing cooked file and cache key is unchanged. The cook tool carries compressed textures through `Asset`, `encode` and `decode` (virtual packages encode their resources the same way); it does not compress. The reference path tracer reads texels itself and refuses compressed textures by name; decode first.

## Dependency

`bcdec_rs 0.2.1` (MIT, safe Rust, `no_std`, no dependencies) is the one decoder: the upload path on devices without BC sampling, and the tests' ground truth. It joins the renderer's normal dependency graph; notices are in THIRD_PARTY_NOTICES.md. The test fixtures are generated by test-only encoders in `texture_bc.rs` (BC7 mode 6 and range-fit BC5) and seeded blocks in every BC7 mode, so no encoder is a dependency.

## For the ara-gpu backend

New texture formats: BC7 RGBA unorm, BC7 RGBA unorm sRGB, BC5 RG unorm. Device feature: BC texture compression (`textureCompressionBC` on Vulkan; `supportsBCTextureCompression` on Metal, absent on some Apple GPUs, where the decode route applies). Material uniform: `normal_mapped` is now 0, 1 or 2. Shader: `rl_normal_texel` in program_api.wgsl.
