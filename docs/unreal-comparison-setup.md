# Unreal M5 retained-fixture comparison

The **directional-black 1x subset has now run on both engines** with installed **UE 5.7.4 CL 51494982**. Full images and decoded linear HDR were inspected. Display agreement is close, but the low-roughness specular disagreement remains unresolved. This does not establish full-fixture parity or performance. No renderer/cooker/viewer/fixture/Cargo source changes or private scan imports were needed.

Input: actual exported [M5-A scene](../measurements/m5-a/fixture-retained-1x/scene.json), file SHA256 **db9020f23a880791501daf5850d09381706c33dd1a73a3bfaa83be7245c3ee8c**. Read-only recipe source support/lighting_fixture.rs is also pinned, SHA256 **8865ad1c2fdeea8885809b4467196904de2baaded6030200bf1ec1548ff0384d**. Changed bytes fail before generation. The generated scene.json is copied byte-for-byte, not invented or canonically rehashed as a new source identity.

## Reproduction commands

CPU only:

    py -3 -B scripts/ue-m5-prepare.py --variant directional-black
    py -3 -B scripts/ue-m5-prepare.py --variant full --root D:/Coding/renderlab-work/ue-m5/full
    py -3 -B scripts/ue-m5-test.py

The default private root D:/Coding/renderlab-work/ue-m5 contains scene.json, manifest.json, nine mesh-buffer JSON files, expected.json, renderlab-lighting.json, M5Calibration.uproject, private DDC config and launch scripts. Both variants use the same generator/helper; there is no parallel bootstrap/import pipeline.

**Only after root assigns the UE/GPU/heavy-build slot**, run one preparation and one 1x capture:

    & D:/Coding/renderlab-work/ue-m5/prepare-assets.ps1
    & D:/Coding/renderlab-work/ue-m5/capture.ps1

Preparation runs UnrealEditor-Cmd.exe with -run=pythonscript -NullRHI and saves /Game/M5/Fixture, nine static meshes and nine materials. NullRHI is asset preparation, not image verification. Capture uses -AllowCommandletRendering -vulkan and public scene-capture/float-target/ImageWrite EXR APIs, then exits after one capture. Both paths now execute successfully. Two actual API errors were fixed. The commandlet's late-loaded StaticMeshEditorSubsystem requires its stateless class-default-object methods; SceneCapture show flags require the reflected show_flag_settings property, rather than a nonexistent Python setter method. Exact failed and successful logs are retained locally under .refs/m5-m6/ue-runtime.

The existing M5-A release example accepts the paired lighting override without changes or rebuilding:

    $env:VK_IMPLICIT_LAYER_PATH = 'D:/Coding/projects/desktop/renderlab/measurements/empty-vulkan-implicit'
    & ./target/release/examples/m5-lighting.exe --samples 1 --frames 1 --validate --lighting D:/Coding/renderlab-work/ue-m5/renderlab-lighting.json --output D:/Coding/renderlab-work/ue-m5/renderlab-directional-black

This command passed with active Vulkan validation, zero errors and no missing layer. The binary SHA256 is **4d6d90854834051dec566140bd144b340671aab39582d5c162c318102bfa14e7**, verified against .refs/m5-a/identities.json. The example renders the override, but its fixture scene.json export still reports original fixture lighting. Preserve/hash the override alongside the output; use manifest.json + renderlab-lighting.json as the variant identity. Actual packed-lighting SHA256 is **09f49dd89e695d589d72a05e741fdf89666464dc7f207f301b1f11285b1f3813**. Prior retained full-fixture images are not the directional-black baseline.

## Geometry and camera

All nine source objects are retained: six 64x32 UV spheres, **2,145 vertices / 4,096 triangles each**, plus floor, mirrored nonuniform panel and reversed-index double-sided panel, **4 vertices / 2 triangles each**. Total **12,882 vertices / 24,582 triangles**. Radius .5 m, duplicate seam/pole IDs, degenerate pole triangles, authored normals, zero UVs, white vertex colors and original index sequence follow the actual source recipe. No UE built-in spheres.

Small owned JSON buffers go directly through public [Geometry Script append-buffer APIs](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/GeometryScript_MeshEdits?application_version=5.7) and [static-mesh asset APIs](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/GeometryScript_AssetUtils?application_version=5.7). No general importer or large external tools.

- UE centimetres = 100*(-source Z, source X, source Y). Exact recorded column-major affine matrices are baked; normals use normalized inverse transpose. No Euler/scale decomposition.
- RH CCW to UE LH clockwise requires no additional index swap for the axis reflection. Negative source determinants reverse each triangle, preserving Renderlab's source-facing policy. Reversed source-panel indices remain reversed and material Two Sided is retained.
- Nanite/collision, normal/tangent recomputation, degenerate removal and generated lightmap UVs are disabled. Full precision UVs and high precision tangent basis are requested. Preparation checks every dynamic vertex/triangle/corner normal, then static source triangle counts and oriented position/normal multisets. Failure stops preparation.
- The manifest records unchanged source object/material/matrix/recipe values, source-local and converted-world buffer hashes, file hashes and bounds. Independent CPU projected object centers are in expected.json.
- Camera is retained at 960x640, pose 0, exact eye/target/up/.78-radian vertical FOV, near=.05 m, far=50 m and motion delta=.04 m per pose. UE horizontal FOV derives from aspect; a custom 5 cm near plane is requested. Infinite UE far differs from source far=50 m; the fixture lies inside both ranges. Actual camera readback and geometry coverage checks below support this fixed pose; motion remains unrun.

**Precision limitation:** scene.json exports recipes/counts, not raw Rust vertex buffers. Python rounds f32 arithmetic and trig results, but Python libm and Rust platform sinf/cosf have no established byte-equality guarantee. CPU baking and GPU f32 transforms can also differ slightly. Topology is exact and normals follow the source recipe; Rust buffer bit equality is **unverified**. The precise missing input for that stronger claim is the retained raw little-endian position/normal/index stream or independently comparable per-buffer hashes. No M5-A source change is required to construct or run this independent reference.

Actual static render-data readback contains **24,070 triangles**, versus 24,582 source-model triangles. Four spheres retain 4,032 each and two retain 3,968 each. UE drops 512 pole triangles despite remove_degenerates=false. All missing oriented triangles have zero area at the recorded 1e-4 cm position rounding; there are no extra triangles. This check does not certify raw buffer identity. Maximum read-back normal-vector error is **2.37e-5**. High-precision tangent basis, full-precision UVs, authored normals and disabled recomputation were read back. The source-front, mirrored and back-panel silhouettes agree in the inspected full images. UE inverse-opacity geometry coverage differs from the independent source raster by three pixels at 960x640.

## Materials, lights and explicit variants

Material graph constants preserve linear base color, metallic, perceptual roughness and sidedness. Legacy Default Lit is selected; Substrate is requested off and queried. **Specular=.5**, corresponding to **F0=.04**, fixes the old zero-specular mismatch; Epic documents [.5 as approximately 4% reflectivity](https://dev.epicgames.com/documentation/en-us/unreal-engine/material-inputs-in-unreal-engine). This alone does not prove BRDF equality. Source uses plain nonmetallic Lambert, correlated Smith GGX/Schlick and alpha=max(roughness^2,1e-4). UE diffuse/visibility/roughness floor and energy conservation/preservation require actual queries and HDR calibration. Runtime records read back material model/properties/graph constants and issue energy-CVar discovery queries. Missing names remain unresolved, not assumed zero.

Both manifests retain the exact three source lights. Full prepares all three. Directional is lux; local lights are candela, inverse square enabled, zero source radius/soft radius/length, position/range times 100, spot half-angle radians converted to degrees. Travel maps to actor forward. Peak intensity times normalized linear RGB reconstructs the source vector. UE stores LightColor in 8 bits; preparation reads back actual linear color/intensity/reconstructed RGB and light transforms/settings. Report/calibrate quantization error rather than silently adjusting one engine.

| Field | Observed status and remaining limits |
|---|---|
| Directional lux/RGB/travel | Actual reconstructed RGB [1000,947.306514,846.873224] lux; green -0.2835%, blue -0.3679%; travel readback agrees |
| Point/spot inverse square | Source 1 cm distance regularization; UE equivalence unverified |
| Range window | Source saturate(1-(d/r)^4)^2; UE formula unverified, same radius is insufficient |
| Spot window | Source squared inner/outer cosine interpolation; UE formula unverified |
| Environment [4,5,8] cd/m2 | PENDING constant cubemap/DFG calibration; no fake fill, full capture refuses to run |
| Default-lit energy/diffuse/Smith/roughness | Shading energy conservation/preservation both 1, material energy conservation 0; legacy approximate Smith differs from source exact correlated Smith; sharp specular mismatch unresolved |
| 4x AA | Unsupported here; forward-MSAA target/resolve proof absent |

**directional-black** preserves all geometry/materials/camera and the original colored directional light, removes both local lights and sets environment black **on both engines**, through the emitted Renderlab preset. It is a named calibration subset, not the full fixture. No skylight/reflection capture, atmosphere, fog, GI or ambient fill is invented. Full inputs remain prepared for follow-through, but capture is explicitly blocked while its environment is pending.

## HDR, settings and measurements

First capture: [SceneCapture2D](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/SceneCaptureComponent2D?application_version=5.7), SCS_SCENE_COLOR_HDR, RTF_RGBA16F and synchronous [ImageWrite EXR export](https://dev.epicgames.com/documentation/en-us/unreal-engine/python-api/class/ImageWriteBlueprintLibrary?application_version=5.7), Artifacts/scene-linear.exr. Alpha is inverse opacity. Manual exposure, physical camera off, bias zero and no pre-exposure are requested. Metadata reads exposure/camera/show-flag properties, records CVar numeric queries plus console-query logs and hashes engine/input/output identities. A numeric getter's zero cannot establish CVar existence; inspect console value/help/priority and missing-name errors. Energy discovery names are explicitly tentative.

GI/Lumen, reflection environment/SSR, AO/DFAO, contact/direct/virtual shadows, bloom/vignette/grain/motion blur, fog/atmosphere, eye adaptation, tone mapping and AA are requested disabled. Config/show-flag text alone is not effective-renderer evidence.

Decode actual EXR RGB; verify physical intensity/exposure scale on flat floor interiors using independent direct-BRDF evaluation, channel/origin conventions, black background, finite values and clipping. File magic/existence proves only export. Reconstruct Renderlab unexposed RGB as stored exposed RGB / metadata pre-exposure (.00390625 at -8 stops); exclude clipping/saturation. Preserve raw linear outputs and apply the same display_rgb() to both decoded unexposed arrays: -8 stops, per-channel Reinhard, sRGB. UE filmic images are not the baseline. Inspect full silhouettes, highlights and mirrored/back panels, then a matched motion sequence.

Private filesystem DDC reuses installed read-only compressed cache without shared/Zen/cloud service. Check disk before runtime: the 2 GiB generation floor is not a shader-cache reservation. Initial shader-loading process spans are recorded below; isolated DDC size and shader-only time were not measured. No C++ game module is introduced.

Whole-process timing includes startup/loading/shader work/script/readback/export/shutdown. Capture JSON separately measures loading/shader completion and CPU wall around capture plus synchronous readback/EXR encoding. These are **not GPU frame time or steady-state throughput**. Cold preparation, warm startup-to-image, runtime CPU critical path and full GPU span stay separate. No 2x claim. No 4x claim until actual forward MSAA and linear HDR resolve are demonstrated; CVar values are insufficient.

## Bounded runtime result

Seven focused CPU tests pass after both retained API fixes. They cover pinned source/hash rejection, sphere topology/normals/hash, affine/normal/axis/facing conversion, material/light preservation, camera/display math, deterministic variants and process-free source generation. Actual preparation, capture and static render-data readback supplement those tests. The successful capture log has zero API/compile errors, no failed-shader fallback, and zero shaders left at exit. Every required CVar has a console value/priority line; show-flag property readback is retained. Root reviewed the generator, editor helper, tests and independent comparison math; inspected the complete comparison and highlight crops; and verified all 62 saved source, binary and evidence hashes before the checkpoint. The hash audit is `.refs/m5-m6/ue-runtime/root-audit.json`.

UE EXR RGB is finite, nonnegative and unclipped. Its maximum RGB is [5148,2220,624.5] cd/m2; Renderlab reconstructs [86976,37792,10568] from stored HDR / .00390625, also without clipping. The same -8-stop Reinhard+sRGB curve was applied offline, with no brightness fit or image alignment. Full-frame display RMSE is **0.4957/255**, MAE **0.09715/255**. Linear RGB RMSE is **80.844 cd/m2**, relative L1 **1.0434%**. Three metal-highlight pixels account for **97.865% of squared HDR disagreement**. The roughness-.1 metal interior has **84.75% relative L1** disagreement; this cannot be hidden by the low whole-frame display error.

Four fixed 9x9 floor interiors agree with an independent f64 Lambert/GGX/Smith/Schlick evaluation to **0.0385-0.0604%** relative L1 for Renderlab and **0.3809-0.4461%** for UE. This supports unit exposure scale, without an arbitrary gain. Camera readback confirms 63.314503-degree horizontal FOV, 5 cm near plane, expected right/up/forward vectors and zero roll. Nonzero lit coverage differs by nine pixels; UE alpha/source geometry coverage differs by three. The lit-support count alone does not certify black silhouettes, so the complete images were also inspected.

The primary radiance disagreement is localized to UE's low-roughness direct specular response. Source BRDF/precision settings differ, but the exact cause of the much weaker peak is **not proven**. An offline normalized normal-quantization probe did not reproduce it. Follow up with actual UE roughness/normal-buffer values at pixels (363,268), (364,268), (363,267) and (340,306), before changing material, light, exposure or renderer behavior. Do not tune a gain to these pixels.

Local raw EXRs, Renderlab HDR, complete images, a comparison contact sheet, numerical records, source/binary hashes, commands and logs are retained in .refs/m5-m6/ue-runtime and D:/Coding/renderlab-work/ue-m5. Successful NullRHI preparation took 10.423 s. Initial Vulkan startup/shader compilation then an API failure took 742.750 s. The successful retry took 209.252 s, including 194.353 s at the loading/shader barrier and .422 s around capture/readback/EXR export. These are process/CPU wall spans, not GPU frame time. No throughput conclusion follows. Full environment/local attenuation/cones, 4x, motion, exact Rust-buffer equality and the combined quality/performance gate remain open.
