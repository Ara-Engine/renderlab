# Quarry scan intake

This checkpoint uses the user's local South African slate quarry scan collection. It is an authored composition, because the archive supplies assets without an assembled scene or layout. The reproducible local record is `measurements/quarry-scan/scan-manifest.json`. Private source geometry, derived assets, captures, and measurements remain ignored.

## Content and conversion

All 66 FBX files were imported sequentially in fresh Blender 5.1 processes. The machine-readable `catalog.json` records source entry identities, counts, bounds, material slots, UV sets, supplied normals, transforms, and the archive texture inventory. The `uddmcgbia` and `wk1ncbxja` families include opacity maps. Their 21 mesh variations are excluded from this opaque baseline.

The composition uses twelve distinct scans with 13,034,903 source triangles. Four ground patches, six rocks or ledges, and two larger quarry faces form a small quarry bank. `ScenePlacement` records every translation and Y rotation. Uniform scales are 1. Geometry is neither decimated nor normalized to fit a budget. Source FBX object scale and aspect ratio remain intact.

The Blender FBX importer supplies the initial coordinate, unit, and floating-point representation. The converter preserves every triangle corner position, supplied corner normal, and UV exactly relative to that imported representation, after the glTF axis conversion and UV V flip. Original archive entries are preserved byte-for-byte and checked against SHA-256 hashes. This does not claim bit-exact native FBX double precision through Blender's float mesh representation.

Blender 5.1's stock glTF exporter rounds normals to four decimals. `scan-blender.py` replaces that static normal extraction step inside its own background process. The installed exporter is untouched. Full triangle-corner comparisons verify positions, normals, and UVs after every selected export. The converter rejects multiple material slots, extra UV sets, and vertex colors instead of silently dropping them. The selected scans each have one material slot and one UV set.

The imported object transforms include a 0.01 scale. The composition retains that transform and treats glTF world units as metres. The physical unit interpretation remains unverified without acquisition metadata or a known dimension. It is not a rescaled miniature benchmark.

## Material fidelity

The initial baseline embeds one 2048-square JPEG base-color image per scan. The source base colors are 8192-square and remain available locally. Twelve complete RGBA8 mip chains occupy 268,435,440 bytes. The initial texture choice bounds conversion, upload, and replay memory while leaving the source available for later texture-quality checks.

The renderer displays unlit base color. The conversion adds no lighting bake. Materials are opaque and double-sided, with metallic 0 and constant roughness 0.85. Source normal, roughness, AO, bump, cavity, gloss, and specular maps remain unused. Billboard, opacity, and translucency maps are catalogued but excluded. This is not full source-material fidelity, and the images do not establish lighting or shadow quality.

The source FBX material slots contain no linked texture images. The converter assigns each matching source base color explicitly. Individual unlit previews and the final composition captures are inspected for material assignment and surface continuity. Raw scan patch perimeters and overlaps remain visible. These are authored composition boundaries, not an original continuous quarry landscape.

## Local reproduction

Run commands from the repository root. Use fresh evidence and replay labels when rerunning, because the helpers preserve prior process records and outputs.

```powershell
$scanRoot = 'measurements/quarry-scan'
$scanIds = @('xb5ebhf','xb5gfgm','xbnhajp','xb5lch2','xb5gbjo','xb5sabi','xbkgcc1','xbkgcfh','xbkgdbu','xbkndhe','xcghcfe','xckjagi')
py -3 scripts/scan-intake.py C:/Users/snowy/Downloads/south_african_slate_quarry_raw.zip $scanRoot --all-geometry --extract $scanIds
py -3 scripts/scan-batch.py $scanRoot
py -3 scripts/scan-batch.py $scanRoot --ids $scanIds --export --preview
py -3 scripts/scan-catalog.py $scanRoot
py -3 scripts/scan-scene.py $scanRoot
py -3 scripts/scan-compose.py $scanRoot "$scanRoot/placements.json" "$scanRoot/quarry.glb"
py -3 scripts/scan-execute.py --timeout 240 .refs/quarry-scan/cook-new -- target/debug/renderlab-cook.exe cook "$scanRoot/quarry.glb" "$scanRoot/cache" "$scanRoot/quarry.manifest" --limits "$scanRoot/limits.json"
$scanPayload = (Get-Content .refs/quarry-scan/cook-new.stdout.log -Raw | ConvertFrom-Json).payload_sha256
py -3 scripts/scan-execute.py --gpu --timeout 180 .refs/quarry-scan/full-new -- target/debug/examples/virtual-stream.exe --mode full --asset "$scanRoot/cache/assets/$scanPayload" --cameras "$scanRoot/cameras.json" --frames 16 --capture 0,4,8,12 --samples 4 --validate --limits "$scanRoot/limits.json" --output "$scanRoot/full-new"
py -3 scripts/scan-results.py $scanRoot .refs/quarry-scan captures/quarry-scan --runs full-new
```

`scan-execute.py` stores the exact command, executable hash, renderer source hashes, exit status, observed host memory, and optional driver observations. Its process-local Vulkan configuration excludes unrelated implicit overlays while retaining explicit Khronos validation. GPU work stops if the protected game starts. The monitor and image readback add overhead. These are development baseline observations, not a controlled performance campaign.

`limits.json` raises the existing configurable policy for this actual scene. It allows a 1 GiB source, a 2 GiB prepared payload, 16 million vertices, 90 million indices, and finite runtime budgets. The initial streaming geometry, metadata, and texture budgets are each 512 MiB. GPU mesh upload admission is 128 MiB per resource. Effective settings are written by the existing tools. None of these settings proves general scalability.

## First rendered baseline

The final full-detail GLB has SHA-256 `2901710be6e4aae4efc2e1d3ce0d0333f5807eb46048b1efa55e51c194284047`. Its prepared payload has SHA-256 `86bdd8afdf69bace8a08d3974e8e9da9af20e6b261880b9728730d9f3387b0b3`. The cooker retains 39,104,709 indices and 6,633,832 prepared vertices. UV and normal seams account for exported vertex duplication. The twelve original source meshes remain the full-detail reference.

Both full-detail sample counts completed sixteen 1920x1080 camera poses, with wide, close, grazing, and reverse captures. Both logged active Khronos validation with zero errors. The first full-resolution images were inspected before the composition expanded. A grazing view then exposed a placement gap under foreground rocks. Three translations were corrected and both final baselines repeated. Earlier images remain local as evidence.

The same prepared asset also completed virtual cooking. The result contains 13,126 groups, 214,062 clusters, 16,339 pages, and 1,070,792,704 disk page bytes. The virtual package SHA-256 is `3deb5931e22207c8a26cd3ba8122ec425d8d53ef0ee2ae243f34898e0302749b`. The source RLA hash matches the full-detail reference.

The first streamed replay rendered all sixteen poses and passed sixteen legal-cover checks. Its 180-second process deadline expired during the unconditional final per-group diagnostic dump, after the images and renderer summary had been written. The failure is retained in `.refs/quarry-scan/stream-1.process.json`. This is a replay-tool scaling limitation. It does not establish a clean shutdown validation pass for that process. Full run outcomes, image inspection, and next actions are in the local handoff.

The streamed 4x process completed with a longer finite deadline and active validation with zero errors. It also passed sixteen legal-cover checks. Its final camera pose retained residency debt and a projected denied error above the requested target. Those moving captures do not certify settled quality. The native full-detail viewer separately presented sixteen registered poses and exited with clean active validation. Input, resize, and minimize recovery remain untested in this checkpoint.

## Rights and scope

The archive contains no license or acquisition record. The user's local-test authorization does not establish acquisition-specific rights or permission to redistribute. The applicable reference pages are the [Fab EULA](https://www.fab.com/eula) and the [Epic Content License Agreement](https://www.unrealengine.com/eula/content). No legal conclusion is inferred from private use. Source assets and derived meshes and images stay local.

The checkpoint does not close the larger validation task. Higher-sample AA acceptance, resize diagnosis, continuous motion quality, controlled optimized benchmarks, complete materials, and any Unreal comparison remain separate work. Exact run results and minimal next actions are recorded in `.refs/quarry-scan/handoff.md`.
