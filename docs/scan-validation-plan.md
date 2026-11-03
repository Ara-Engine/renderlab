# Real-scan validation

## Authorization and outcome

Islam requested a scene from his local scans, fixes for the disclosed caveats, and proper measured results on 2026-11-03. He explicitly left the PC available. This work continues beyond the completed M4. Keep the independent library and fast development loop intact.

The deliverable is a reproducible local scene, inspected full-detail and streamed images, a responsive viewer, and measurements on that scene with exact content, build, quality, hardware and contention labels. Synthetic-only success does not satisfy this task. An unresolved condition remains open, even if other checks pass.

## Intake

The four Downloads ZIPs contain FBX and JPG asset collections, with no assembled scene file or license document in their entry lists. The initial choice is `south_african_slate_quarry_raw.zip`, 66 FBX entries and 707 JPG entries, 8,421,215,674 uncompressed bytes. Inspect actual geometry and texture assignments before selecting the scene composition. Preserve full-resolution source geometry for the reference. Record every selected asset and omitted feature; do not silently replace a scan scene with one low-detail prop.

Keep archives, extracted assets, derived assets, images and measurements local in ignored directories on D:. Do not publish them. The user's authorization covers local testing; acquisition-specific rights are not established by ZIP names. Fab's current standard license allows compatible tools, while legacy Megascans terms depend on the acquisition plan. No redistribution permission is inferred. Sources checked: https://www.fab.com/eula and https://www.unrealengine.com/eula/content.

Blender 5.1 and Unreal 5.7 are installed. Intake observed about 52 GiB free on D:, 10.4 GiB free RAM and RTX 4070 SUPER / NVIDIA 617.14 at 2% utilization, 2,193 MiB VRAM used. These are intake observations, not a quiet measurement window. Conversion must be staged within available memory and disk.

## Units and acceptance

1. Convert actual scans and render a meaningful quarry composition. Inspect imported transforms, normals, UVs and material assignments. Record source and prepared counts, sizes, identities, conversion costs and all material limitations. Inspect the first image before expanding the campaign.
2. Establish fixed camera paths with wide views, grazing edges, closeups, motion and abrupt cuts. Capture full-detail references. Measure streamed silhouettes and material agreement against the same content and inspect temporal stability. Preserve failing baselines before fixes. Establish edge-quality acceptance against a higher-sample reference before selecting a fix.
3. Diagnose the resize stall and exercise resize, minimize, restore and reload on the real scene. Instrument acquisition, configuration, CPU preparation, GPU execution and presentation separately. A finite rerun without a stall alone does not prove the old cause fixed.
4. Measure optimized and development builds separately with a quiet, recorded baseline and continuing load samples. Remove subprocess guard overhead from timed frame work. Report distributions, outliers, memory, uploads, pipeline churn, startup and editing latency. Measure full-detail and streamed paths at matched visual settings. Keep any Unreal comparison explicitly scoped to matched features and content.
5. Review consequential changes and raw evidence, push validated checkpoints with Snowy7 identity, and show the actual scene and results. Do not claim whole-engine or Nanite superiority without the corresponding comparison.

## Workflow and ownership

Use pstack's figure-it-out workflow and a committed decision trail in `scan-validation-decisions.tsv`. Root owns acceptance, user communication, documentation and pushes. Use Sol for bounded implementation and Opus sparingly for consequential judgment or hillclimb. Inspect delegated artifacts directly.

The immediate blocker is conversion and a first image of real content. Intake/conversion can be separated from renderer changes, but they share memory, disk, builds and GPU time. Start with one worker owning conversion tooling and local scene artifacts; serialize builds and GPU work. Only split later work when its files and runtime resources are independent. Root reserves AGENTS.md, brief.md and this plan.
