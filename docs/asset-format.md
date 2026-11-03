# Prepared asset format, schema 1

Unit A supplies CPU preparation, not renderer integration or M3 acceptance. `renderlab-asset` has zero dependencies. Its owned public model is `Asset { meshes, placements, materials, textures }`. All math uses plain arrays. Mesh order, primitive order and selected-scene depth-first placement order are preserved. Placements refer to reusable meshes. The runtime can read a bounded prepared payload and call `decode(bytes, Limits)` without any cooker dependency.

`encode`, `decode`, `encode_mesh`, `Asset::validate` and `valid_transform` are the boundary APIs. Validation checks aggregate geometry counts, scalar domains, material/texture/mesh references, affine nonsingular placements and complete mip chains. Supplied finite nonzero normals are preserved. This establishes data validity, not correct rendered normals under every transform. GPU integration still must use and test its supported inverse-transpose/winding behavior.

## Binary payload

Every integer is little-endian u32 unless stated otherwise. Floats are IEEE-754 binary32 little-endian. There is no native struct padding, compression, string table or implicit alignment. Encoders preserve float bits, including signed zero. All nonfinite floats are rejected. Readers reject unknown versions, unknown enum tags, invalid booleans, truncation and trailing bytes. Length arithmetic is checked before slicing and allocating. A decoder charges element storage and container metadata against its decode budget before each allocation.

| Record | Ordered fields |
| --- | --- |
| Asset header | Four bytes `RLA\0`, schema `1`, mesh count, placement count, material count, texture count |
| Mesh | Primitive count, then primitive records |
| Primitive | Material index, vertex count, index count, vertices, u32 indices |
| Vertex | Position XYZ, normal XYZ, UV0 XY, linear color RGBA. 48 bytes |
| Placement | Mesh index, four columns of four floats. 68 bytes |
| Material | Linear base RGBA, metallic, roughness, double-sided boolean, optional base-color reference, optional MR reference |
| Optional reference | Present boolean. If present, texture index, wrap U, wrap V, mag filter, min filter, mipmap filter |
| Texture | Role, mip count, mip records |
| Mip | Width, height, RGBA8 byte count, exact bytes |

Boolean tags are 0/1. Texture role tags are 0 for base-color sRGB and 1 for linear metallic/roughness. Wrap tags are 0 clamp, 1 mirror, 2 repeat. Mag/min filter tags are 0 nearest and 1 linear. Mipmap tags are 0 disabled, 1 nearest, 2 linear. Separate `BaseColorTexture` and `MetallicRoughnessTexture` types carry the role-specific references. Validation rejects a reference to the wrong role. Material samplers are separate from image records, so one role-correct texture can serve several sampling choices.

Every primitive is nonempty and has a nonempty index count divisible by three. All indices address its owned vertices. Materials and vertex colors have finite [0,1] factors. UVs and positions must be finite. A placement's matrix has bottom row exactly [0,0,0,1] and a finite nonzero 3x3 determinant evaluated in f64. This is not a conditioning guarantee. Every mip has width/height at least one, exact `width * height * 4` bytes, and follows `(max(1,w/2), max(1,h/2))` through exactly one final 1x1 level.

`Counts.decoded_bytes` reports logical vertex/index/placement/material element storage and texture pixels. It excludes Vec/container headers, allocator overhead and spare capacity. It is not process RSS. Decoder admission additionally charges its container headers. Keep headroom if choosing limits close to a reported logical count.

Configurable development defaults (see [limits](limits.md)) are 256 MiB prepared input, 256 MiB decoded allocation budget, 4,096 meshes/materials, 16,384 primitives/placements, 256 role-correct textures, two million vertices, six million indices and maximum texture dimension 4,096. Limits are caller supplied and checked as totals across the asset. Decoder mip counts also have an absolute cap of 32. Allocation failure from the host allocator is not converted into a recoverable error.

## Cache identity and publication

The cooker uses SHA-256 from pinned Rust `sha2 0.10.9` with normal CPU dispatch and no optional assembly/C build. Key input is the algorithm byte string `renderlab-cook/static-glb/area-mips/1/gltf-1.4.1/image-0.25.10/sha2-0.10.9`, schema u32, one settings byte (0 flat missing normals, 1 reject), sixteen u64 limit fields in `cache::key` order, then the complete source GLB bytes. New source-policy fields add a `source-policy-2` suffix and six u64 fields before the source only when they differ from their historical defaults; unchanged defaults retain exact keys. Policy stays in the lookup key so source-only admission cannot be bypassed by a warm hit; payload identity is independent of policy. No URI resolution or external dependency is permitted. Bump the algorithm identity when output algorithms or their dependency behavior change. Exact dependency resolution remains in Cargo.lock.

Cache paths are `assets/<payload-sha256>`, `geometry/<mesh-sha256>` and `keys/<key-sha256>`. A geometry sidecar is four bytes `RLG\0`, schema u32 and one mesh record in the same encoding. Its identity includes material indices and full attributes/indices, and excludes placements, materials and textures. Transform-only edits demonstrably reuse the same sidecar bytes. Preparation still reruns on source edits. This is storage reuse, not incremental import or M4 streaming. The self-contained asset currently also embeds geometry; sidecar bytes are reported separately to expose this duplication.

The manifest is four bytes `RLM\0`, schema u32, 32 key bytes, 32 payload hash bytes, payload length u64, geometry count u32, then one `(hash[32], byte length u64)` record per mesh. Its exact length is `84 + 40 * mesh_count`, capped by `CookLimits.manifest_bytes` (1 MiB by default). Each entry and the aggregate geometry sidecar lengths must fit `Limits.max_file_bytes` before file reads. It names no arbitrary file paths. `load_manifest` checks exact entry length/hash, decodes the complete asset and verifies geometry identities against decoded meshes. Missing, partial, corrupt or mismatched entries cannot be hits. The runtime need only consume the self-contained asset; the cooker's convenience manifest loader intentionally verifies all preparation sidecars too.

Each new entry is written to a unique same-directory create-new temporary file, flushed with `sync_all`, then published by rename. An existing valid entry is reused. A corrupt entry is atomically replaced with its correct content-addressed bytes. Orphan temporary files from a killed process are ignored; they are never hits. The complete payload and all geometry are reread and validated before publishing either the key manifest or requested current manifest. Failure leaves the previous current manifest intact. Concurrent identical writers publish identical complete entries. Concurrent different keys targeting the same current path have last-successful-rename semantics; the manifest always identifies one complete asset.

No multi-file rollback or cache garbage collection is implemented. Completed orphan entries are harmless. Rename atomicity is tested on this Windows local filesystem and requires a filesystem with atomic same-directory replacement; network filesystem semantics and sudden-power-loss directory durability are unverified. `sync_all` on files does not establish parent-directory crash durability on every platform. The guarantee is complete observable publication and prior-manifest preservation on ordinary errors, not a backup or hostile-writer authentication boundary.

## Counters and CLI

`cook_file` returns the loaded owned asset, hashes, cache-hit state, miss diagnostic, counts and microsecond timings. Source read, key lookup/content verification, preparation, encoding/hashing, decoded load/geometry verification and publication are separate. Total includes validation/call overhead outside those buckets. Cold cooking rereads prepared output before publishing. Warm hits still hash the complete source, verify payload/sidecars, decode and validate; they do not import or decode source images. Publication includes filesystem flushes. Timings are not renderer startup, GPU upload or frame times.

```powershell
cargo run -j 2 -p renderlab-cook -- fixtures measurements/m3-assets/sources
cargo run -j 2 -p renderlab-cook -- cook measurements/m3-assets/sources/instanced-boxes.glb measurements/m3-assets/cache measurements/m3-assets/boxes.manifest
cargo run -j 2 -p renderlab-cook -- inspect measurements/m3-assets/cache measurements/m3-assets/boxes.manifest
```

Add `--reject-missing-normals` to `cook` to select strict normal presence. `scripts/m3-asset-check.ps1` reproduces the focused CPU checks and two-asset CLI evidence without GPU/window work. Generated assets, caches and measurements remain ignored.
