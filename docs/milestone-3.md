# Milestone 3: prepared assets and materials

In progress. Unit A (prepared format and isolated cooker) is verified; unit B (GPU materials, viewer integration and basic AA) remains unimplemented. This is not M3 acceptance. Follow [the execution contract](milestone-3-plan.md).

## Unit A checkpoint, 2026-11-03

`renderlab-asset` is a dependency-free owned data model and bounded schema-1 binary codec. `renderlab-cook` supplies a strict static embedded GLB subset, PNG/JPEG decoding, role-correct mips, content-addressed preparation and atomic complete-manifest publication. See [the format](asset-format.md), [supported subset](asset-subset.md), [fixture provenance](../assets/PROVENANCE.md) and [dependency notices](../THIRD_PARTY_NOTICES.md). Runtime/viewer source and their direct dependency graphs are unchanged in this unit.

Root inspected the format reader/validator, GLB framing/accessor and hierarchy handling, cache publication/integrity path, mip math and fixture verification. Root reproduced `scripts/m3-asset-check.ps1` and the eleven additional CLI boundary observations at `measurements/m3-assets/20261103-005837`. Formatting, affected-crate all-target checks, Clippy with warnings denied, eleven Rust tests, two-asset CLI cooking/inspection and independent Python payload/hash inspection passed. Existing renderer/viewer all-target checking passed too. The helper records before/after source hashes and fails if they differ; locked dependency resolution is used for checks. Actual CLI binary SHA-256: `1cf9a741f709054276ca1ca9c17116bf28e6691a72a6ac7497618d13865f4553`.

The extra CLI observations cover cold/warm cache, truncated cached payload rejection and repair, transform/source and settings invalidation, unsupported alpha, truncated source, external missing resource, invalid geometry index and preservation/loading of the previous manifest after failures. Rust tests additionally check simultaneous identical publication, injected partial-write failure, bounded malformed prepared data, geometry changes and independent linear/sRGB/odd-edge mip expectations. Failures return diagnostics rather than loading partial artifacts.

| Generated source | Meshes / primitives / placements | Vertices / indices | Source / prepared bytes | One cold / warm internal total |
| --- | --- | --- | --- | --- |
| Instanced boxes | 1 / 2 / 4 | 24 / 36 | 3,824 / 2,044 | 9.130 / 3.475 ms |
| Curved body and thin fins | 1 / 2 / 2 | 1,333 / 6,984 | 73,808 / 92,720 | 31.636 / 13.662 ms |

These are single functional samples from an unoptimized debug CLI on the shared Windows Ryzen 7 7800X3D host, Rust 1.96.0, two build jobs. First preparation cache was empty; OS/compiler caches and desktop contention were uncontrolled. Timings exclude process launch and fixture generation, include validation and publication flushes, and are not optimized throughput comparisons. Stage breakdowns, counts and host metadata are saved with the run. The earlier worker run remains at `20261103-005326` and has different timings.

Source SHA-256 values are `01e25581431fa40bb2d23092e03ffef96d70bdff5873b776edd5d9102c02ca1f` and `fe68c1f7e03e96e1d87efe91509fcf3bc0d828026dbb96432ee0d15c5e0027e7`. Prepared values are `d62f14452111fd34280b1cd56841bf08d79d6d432ee38270c6d44773e25c219c` and `8fabb94f03e50e703e4d4d6eb1371be44c6f9e790c06bae1a65c5ccdae10cd4e`. Repeated local fixture generation reproduces bytes. A parent translation edit changes the key and prepared payload while retaining boxes geometry hash `ea8f9a7afd41103899346f349d17f89cf43b69ac1766b59b95af48c50c4f3c2a` and reusing its 1,332-byte sidecar.

The standalone payload currently embeds geometry as well as publishing reusable geometry sidecars: disk duplication is measured and intentional at this stage. Import/mip generation still reruns after source edits; sidecar reuse only avoids rewriting equal geometry. Warm hits still read, verify, decode and validate. No streaming or incremental-import claim follows from those hashes.

Thirty-two new registry packages are locked for the isolated cooker graph. Actual packaged notices are preserved, including alternatives, with a hash audit; this does not license Renderlab itself. Runtime/viewer normal dependency-tree checks exclude glTF, image codecs and SHA. The prepared crate has no dependencies. First-ever cold dependency build cost was not measured; practical runtime/cooker edit latency is a unit B gate.

Atomic rename and ordinary failure preservation were tested on the local Windows filesystem. Network-filesystem atomicity, power-loss directory durability, exact process memory caps, cache authentication against hostile writers, cache GC, cross-platform generated-byte identity and rendering are unverified. Public input limits bound admitted bytes/counts, not parser/codec scratch or total RSS. GPU materials/normals/winding, AA, actual visual quality, interaction, resource replacement and final development-loop evidence remain outstanding.
