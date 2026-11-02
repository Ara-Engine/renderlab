# M3 source fixtures

`crates/cook/src/fixtures.rs` creates both source GLBs from geometry equations, numeric swatches and embedded PNG/JPEG encoded here. No third-party art, Ara assets or private content is used. Created here for Renderlab in November 2026. Copyright and project license remain undecided; this note grants no separate artwork license.

Run `cargo run -j 2 -p renderlab-cook -- fixtures measurements/m3-assets/sources`. Bulk GLBs are generated in the ignored measurements directory.

`instanced-boxes.glb` has one two-material box mesh reused four times under parent/child, mirrored, nonuniform and rotated transforms. Its interleaved attributes exercise normalized U16 UVs and U8 colors. One PNG is used in both sRGB base-color and linear metallic/roughness roles, with differing samplers.

`curved-thin.glb` has a curved toroidal body and 24 thin triangular fins, reused twice with a mirrored/nonuniform child. The body supplies smooth normals. Nonindexed fins omit normals and exercise flat generation. A 5x3 PNG and 7x5 JPEG exercise odd mip edges and sampler differences. These are CPU preparation fixtures; visual quality remains a renderer integration gate.

The generator is deterministic for the pinned Rust/image build. Cross-platform trigonometric/encoder bit identity has not been established. Source bytes, geometry and prepared SHA-256 values are recorded with each measurement run.

M4 Unit A adds `crates/cook/src/virtual_fixtures.rs`, also self-authored from equations. `m4-register DIRECTORY` saves two dense M3-format full-detail references: a six-face cube projected onto a sphere (288 subdivisions per face, canonical shared position bits and independent face UV seams), and a two-material wavy sheet (704 subdivisions, shared material boundary) with 48 separate narrow fins. The sheet/fins have intentional open boundaries; only the sphere is the closed-topology proof fixture. Neither asset uses downloaded art or textures. Each has more than 32 MiB of full-detail geometry, remaining below unchanged prepared limits. Direct procedural prepared generation leaves the GLB contract unchanged.

Registration includes generator settings/source hash, full-detail prepared/source hash, material/texture identity, executable/lock/build identity and all 631 explicit 1920x1080 camera poses (overview, approach, sweep, abrupt cut, return, 300 settled frames). The initial ignored registration is `measurements/m4-cook/registration-initial/registration.json`; registration refuses to overwrite an existing record. Later regeneration preserves the initial identity and records the newer generator/executable hash separately. Trigonometric bit portability remains unverified. No project or separate fixture license has been selected.
