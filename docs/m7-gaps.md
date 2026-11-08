# M7 gap list to Unreal and the reference

One section per feature. Each lists the measured or inspected gaps that remain after its overnight first pass (2026-11-08 push).

## Post

PP1 provides bloom, camera motion blur, camera-requested near/far gather DOF, tone/white balance/contrast/saturation/3D LUT and optional lens effects on both paths. [Implementation and evidence](m7-pp1.md).

- Motion vectors currently reconstruct camera motion over static geometry from TR1's hit/projection inputs. Prior object/skinned/particle transforms and transparent-layer velocities still need a shared attachment. Hidden/disoccluded colour cannot be reconstructed from one surface.
- DOF needs better near-field coverage, hidden-background reconstruction, shaped apertures and temporal stability for large bokeh. MSAA raster depth/velocity uses sample zero.
- The filmic option is an ACES-like fit, not full ACES; RGB white balance is approximate. Display-linear RGBA8 LUTs have no HDR/log shaper or file importer.
- Bloom uses a fixed normalized six-level optical approximation. Grain is simple noise; lens dirt/diffraction/anamorphic calibration are absent. Post shader reload is not wired into the editable prelude transaction.
- No new Unreal/reference campaign tonight; full post quality parity remains unproven. Final effect timings and image findings are in the PP1 report.
