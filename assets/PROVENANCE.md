# M3 source fixtures

`crates/cook/src/fixtures.rs` creates both source GLBs from geometry equations, numeric swatches and embedded PNG/JPEG encoded here. No third-party art, Ara assets or private content is used. Created here for Renderlab in November 2026. Copyright and project license remain undecided; this note grants no separate artwork license.

Run `cargo run -j 2 -p renderlab-cook -- fixtures measurements/m3-assets/sources`. Bulk GLBs are generated in the ignored measurements directory.

`instanced-boxes.glb` has one two-material box mesh reused four times under parent/child, mirrored, nonuniform and rotated transforms. Its interleaved attributes exercise normalized U16 UVs and U8 colors. One PNG is used in both sRGB base-color and linear metallic/roughness roles, with differing samplers.

`curved-thin.glb` has a curved toroidal body and 24 thin triangular fins, reused twice with a mirrored/nonuniform child. The body supplies smooth normals. Nonindexed fins omit normals and exercise flat generation. A 5x3 PNG and 7x5 JPEG exercise odd mip edges and sampler differences. These are CPU preparation fixtures; visual quality remains a renderer integration gate.

The generator is deterministic for the pinned Rust/image build. Cross-platform trigonometric/encoder bit identity has not been established. Source bytes, geometry and prepared SHA-256 values are recorded with each measurement run.
