# Third-party notices

PP1's optional filmic tone curve uses Krzysztof Narkowicz's five-constant ACES-like fit from [ACES Filmic Tone Mapping Curve (2016)](https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/), offered by its author under CC0 or MIT; CC0 is selected. It is a curve approximation, not the ACES colour-management implementation. The bloom, depth-of-field and motion filters are original implementations from the cited papers/talks in `docs/m7-pp1.md`; no third-party filter source was copied.

Renderlab's project license remains undecided. These notices cover dependencies and the attributed tone curve above. The self-authored fixture generator introduces no third-party artwork.

SK1 additionally uses the externally downloaded **Fox** test model and animations.
Their CC0/CC BY 4.0 attribution, pinned source and modification notice are in
[third-party/Fox.md](third-party/Fox.md). The GLB is not bundled in this repository.

M3's cooker pins `gltf 1.4.1`, `image 0.25.10`, `sha2 0.10.9`, and `serde_json 1.0.149`. Their actual packaged MIT and Apache license files were inspected and copied into `third-party/licenses/<crate>-<version>/` before adoption. Each permits either MIT or Apache-2.0. `gltf` has default features disabled and enables only accessor utilities. `image` has default features disabled and enables only PNG/JPEG. `sha2` uses Rust implementations with its normal CPU dispatch, with no optional assembly or C build. It supplies SHA-256 content identity and integrity checks; Renderlab does not implement cryptography.

Additional selected dependency notices are preserved alongside these after lockfile resolution. Cargo.lock records exact versions and archive checksums. Runtime packages do not depend on the cooker or its codecs.

All 32 registry packages newly added to Cargo.lock have their packaged notices copied in full, including alternative license files. `third-party/license-audit.json` records their actual manifest declarations, notice filenames and SHA-256 checksums. Existing runtime dependencies are outside this addition's notice inventory. The new packages are:

- MIT or Apache-2.0 choices (MIT selected here): block-buffer 0.10.4, cpufeatures 0.2.17, crc32fast 1.5.4, crypto-common 0.1.7, digest 0.10.7, fdeflate 0.3.7, flate2 1.1.10, gltf/gltf-derive/gltf-json 1.4.1, image 0.25.10, itoa 1.0.18, lazy_static 1.5.1, png 0.18.1, serde_json 1.0.149, sha2 0.10.9, typenum 1.20.1 and version_check 0.9.5.
- MIT selected from additional alternatives: adler2 2.0.1 (also 0BSD/Apache-2.0), byteorder 1.5.0 and byteorder-lite 0.1.0 (also Unlicense), miniz_oxide 0.8.9 and 0.9.1 (also Zlib/Apache-2.0), zune-core 0.5.3 and zune-jpeg 0.5.15 (also Apache-2.0/Zlib).
- MIT: generic-array 0.14.7, inflections 1.1.1, simd-adler32 0.3.10 and zmij 1.0.23.
- BSD-3-Clause selected (also Apache-2.0): moxcms 0.8.1 and pxfm 0.1.30.
- Zlib: zlib-rs 0.6.8. This is resolved in Cargo.lock but is not enabled in the current Windows cooker tree.

These packages are registry dependencies, not vendored library source. Copyright and permission text remains verbatim in the copied files. No Renderlab or fixture license is assigned by these selections.

M4 Unit A adds cooker-only `meshopt = 0.7.0` (archive checksum in Cargo.lock), which packages meshoptimizer 1.3 and its clusterLOD example. The wrapper declares MIT OR Apache-2.0, but its crate archive omits the root license files. MIT is selected using the exact packaged source commit's [LICENSE-MIT](https://raw.githubusercontent.com/gwihlidal/meshopt-rs/67c589e166a41be6f7e1e41360ecd7cb02f0b9c9/LICENSE-MIT), preserved verbatim in `third-party/licenses/meshopt-0.7.0/`. Both packaged upstream MIT notice blocks are also preserved there. No Unreal/Ara source or third-party artwork was copied.

The only newly resolved packages for that M4 addition are meshopt 0.7.0 and float-cmp 0.10.0 (MIT). Its packaged notice is preserved. The cooker now activates the existing lockfile's C++ build tools cc 1.11.0, find-msvc-tools 0.1.15 and shlex 2.0.1 (MIT selected from MIT OR Apache-2.0); their packaged notices are preserved too. `third-party/m4-license-audit.json` records exact archive and notice checksums and distinguishes new resolution from newly used build dependencies. Optional bindgen is disabled. Runtime/viewer dependency checks include build edges and exclude these cooker additions.

M5 normal preparation adds cooker-only `mikktspace 0.3.0` with its `glam` feature and `glam 0.15.2`. Both declare MIT or Apache-2.0; MIT is selected here and both packaged licenses are preserved. The dependency's original Morten S. Mikkelsen 2011 notice is also preserved verbatim from the comment header of `src/generated.rs`. No tangent implementation source was copied. `third-party/m5-normal-license-audit.json` identifies the five notice files and their SHA-256 hashes; Cargo.lock retains the package checksums. These dependencies do not enter the renderer's normal dependency graph.
