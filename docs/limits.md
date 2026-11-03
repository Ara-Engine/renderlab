# Development limits and resource budgets

The defaults are development/test settings, preserving the registered M3/M4 campaign. They are not production recommendations or format ceilings. Callers can raise or remove data policy without editing source. Runtime memory and work budgets remain finite and configurable. Raising limits does not establish huge-asset or billion-triangle scalability; the GLB importer still reads and parses the whole source and prepares owned geometry/images.

## Library APIs

`renderlab_asset::Limits`, `renderlab_asset::virtual_mesh::VirtualLimits`, and `renderlab_cook::CookLimits` retain their existing public fields and defaults. Each now has `unlimited()`. This uses `usize::MAX` (and `u32::MAX` for dimensions) as comparison ceilings, never allocation requests. Individual fields can be tightened or raised with struct updates. `CookLimits::unlimited()` and `VirtualLimits::unlimited()` also uncap their nested prepared limits.

```rust
let policy = renderlab_cook::CookLimits {
    source_bytes: 256 << 20,
    prepared: renderlab_asset::Limits {
        max_dimension: 8192,
        ..renderlab_asset::Limits::default()
    },
    ..renderlab_cook::CookLimits::default()
};
// prepare(source, settings, policy) and cook_file(..., policy) use the same policy.
```

Virtual callers supply policy in `StreamOptions.limits`. `open` applies it to manifest/package admission, bootstrap decoding, structural validation and prepared-index creation. `new` applies it to caller-owned packages. Scene scratch admission and every selector call use the configured live `metadata_bytes`; no downstream default is restored.

```rust
use renderlab::virtual_geometry::{StreamOptions, VirtualAsset};
let options = StreamOptions {
    limits: renderlab::asset::virtual_mesh::VirtualLimits::unlimited(),
    geometry_bytes: 32 << 20,
    metadata_bytes: 32 << 20,
    texture_bytes: 128 << 20,
    bootstrap_bytes: 160 << 20,
    read_slots: 8,
    uploads_per_tick: 3,
    ..StreamOptions::default()
};
// VirtualAsset::open(cache, manifest, options)?;
```

An admitted scene still needs its pinned cover, one staging page, output minimum and reserved scratch. Geometry charges include uploading, resident and retiring pages plus actual staging until completion. Read slots include queued/active reads and completed CPU payloads awaiting upload. One worker is retained; channels allocate for at most `min(read_slots, package.pages.len())`, and their element storage is charged to metadata. Large settings never preallocate from a configuration maximum. Uploads are counted per `advance`/preparation tick, not per presented image. Calling two advances in an application frame spends two tick quotas.

`Renderer::new[_with_options]` takes the existing `renderlab::Limits`: `max_instances`, `max_mesh_bytes`, and per-texture `max_texture_bytes`. Configure these too when the uploaded resource needs it. Renderer limits are printed separately by tools. Limits that require unsupported current-device buffer/binding sizes are rejected before allocation; texture dimensions and mip/byte validity are checked before upload. Tools retain their current device request. Raising a policy cannot request new GPU capabilities implicitly. The three-flight submission design can admit fewer uploads than a tick quota; it reserves a draw slot and bounds outstanding GPU work.

## One optional JSON file for the tools

`--limits FILE` works with `renderlab-cook cook`, `inspect`, `m4-cook`, `m4-inspect`, `m4-prove`, the viewer, `virtual-stream`, and `asset-cost`. Paths and existing command flags are unchanged. The file is read first; existing explicit flags (`--geometry-bytes`, `--candidates`, `--output-clusters`, `--error-pixels`, `--io-delay-ms`) override it regardless of argument order. Duplicate `--limits`, unknown keys/sections, negative/fractional sizes, overflowing sizes, and zero read/upload/memory limits fail with the field name. Candidate zero remains the existing deliberate no-refinement diagnostic; zero output is rejected or fails minimum-cover admission. Pixel error can be zero; IO delay can be zero.

Only the sections relevant to each tool are consumed, allowing one file to be reused across cooking and viewing. Every omitted field retains its declared default. `cook`, `prepared`, `virtual` and `viewer` accept `"unlimited"` either for the entire section or an individual field. A section affects only itself: `"cook":"unlimited"` in JSON does not change `prepared`; use both sections. Runtime and renderer fields require finite numeric budgets. There are no units suffixes; sizes are integer bytes/counts. JSON `null` is invalid.

For example, save this as `measurements/my-limits.json`:

```json
{
  "cook": { "source_bytes": "unlimited", "samplers": 8192 },
  "prepared": "unlimited",
  "virtual": "unlimited",
  "runtime": {
    "geometry_bytes": 33554432,
    "metadata_bytes": 33554432,
    "texture_bytes": 134217728,
    "bootstrap_bytes": 167772160,
    "read_slots": 8,
    "uploads_per_tick": 3
  },
  "renderer": { "max_texture_bytes": 134217728 },
  "viewer": { "max_placement_slots": "unlimited" }
}
```

```powershell
cargo run --locked -j 2 -p renderlab-cook -- cook measurements/source.glb measurements/cache measurements/current.manifest --limits measurements/my-limits.json
cargo run --locked -j 2 -p renderlab-cook -- inspect measurements/cache measurements/current.manifest --limits measurements/my-limits.json
cargo run --locked -j 2 -p renderlab-cook -- m4-cook measurements/original.rla measurements/cache measurements/virtual.manifest --limits measurements/my-limits.json
cargo run --locked -j 2 -p renderlab-viewer -- --virtual-manifest measurements/virtual.manifest --cache measurements/cache --limits measurements/my-limits.json
cargo run --locked -j 2 --example virtual-stream -- --manifest measurements/virtual.manifest --cache measurements/cache --cameras measurements/m4-cook/registration-initial/cameras.json --limits measurements/my-limits.json --output measurements/my-replay
```

The viewer command opens a window; respect the foreground-game guard. The replay is offscreen. Cooker reports include effective `limits` in JSON and print `effective_limits` on stderr. Viewer logs the effective data/runtime/renderer/viewer settings. Replay saves `limits.json`, including the input file, explicit overrides, defaults and `"unlimited"` markers, beside its existing command, startup, frame and resource records. `asset-cost` uses the same prepared/viewer/renderer policy.

## Accepted fields and defaults

| Section | Fields and defaults |
| --- | --- |
| `cook` | `source_bytes` 67108864; `json_bytes` 8388608; `nodes` 16384 (also edge work); `hierarchy_depth` 128; `images` 256; `image_decoded_bytes` 134217728; `accessors`/`buffer_views` 65536; `scenes` 256; `samplers` 4096; `json_values` 500000; `manifest_bytes` 1048576 |
| `prepared` | `max_file_bytes`/`max_decoded_bytes` 268435456; `max_meshes`/`max_materials` 4096; `max_primitives`/`max_placements` 16384; `max_textures` 256; `max_vertices` 2000000; `max_indices` 6000000; `max_dimension` 4096 |
| `virtual` | `max_groups` 65536; `max_clusters` 131072; `max_pages` 8192; `max_metadata_bytes` 8388608; `max_geometry_bytes` 536870912. Uses the file's `prepared` section for nested policy. |
| `runtime` | `geometry_bytes`/`metadata_bytes` 8388608; `texture_bytes` 67108864; `bootstrap_bytes` 75497472; `read_slots` 4; `uploads_per_tick` 2; `max_candidates` 1024; `max_clusters` 65536; `error_pixels` 1; `io_delay_ms` 0 |
| `renderer` | `max_instances` 1024; `max_mesh_bytes`/`max_texture_bytes` 67108864. The viewer's existing automatic instance capacity considers requested fixture/prepared instances before applying an explicit file override. |
| `viewer` | `max_placement_slots` 1000000 for prepared primitive-placement expansion |

Data admissibility and live memory are separate settings: raising `runtime.metadata_bytes` still respects an explicitly reported `virtual.max_metadata_bytes`; use `virtual:"unlimited"` or raise that policy too. Texture resource decode still respects `prepared.max_decoded_bytes`/`max_dimension` and renderer upload limits. This explicit layering permits a permissive importer with a small running pool, or a strict data contract with a larger running pool.

Bootstrap bounds the encoded package by `bootstrap_bytes` and prepared file policy. Encoded resources are separately bounded by `texture_bytes`. Encoded geometry metadata and decoded metadata/validation allocations share `metadata_bytes`; resource material/texture/mip containers are charged from actual counts, replacing the fixed 1 MiB reservation. Decoded resources have a finite combined texture-plus-remaining-metadata allowance, then retained texel capacity is checked against `texture_bytes`. Bootstrap temporarily holds encoded and decoded resources; it is separate from the read-slot payload bound and does not represent process RSS or physical driver memory.

## Constraints that cannot be disabled

RLA/RLV/GLB versions and u32 count/ID/span widths, the RLV original-cluster sentinel, meshopt callback i32 IDs, checked offsets/lengths/products, exact input extents/digests, finite scalar domains, acyclic ordered dependencies, index/material references, legal complete cuts, full valid mip chains and 64 KiB page layout remain enforced. Thirty-two mip levels follow from u32 image dimensions. GLB remains the static embedded subset. serde_json retains its recursion safety check; the image codecs retain their own representability/validity checks. The virtual cooker's existing magnitude domain (1e6) bounds its upstream numerical calculations. Actual GPU dimensions/buffer/binding capabilities and renderer upload admission always apply. Host allocation failure is not universally recoverable; uncap only for inputs and memory capacity your application is prepared to handle.

Prepared payload/page bytes and format versions do not change with policy. Existing default cache keys remain identical. Historical cache lookup keys already include policy; that is retained so a warm source hit cannot skip stricter source-only admission (including hierarchy/image/JSON work). New source-policy fields add a key suffix only when different from their old defaults. Different policy lookup keys can point to identical content-addressed payloads. Every cache load decodes/revalidates the complete prepared payload/package with the effective limits; a permissive cache is never authority to bypass a restrictive caller. There is no second prepared format for limits.
