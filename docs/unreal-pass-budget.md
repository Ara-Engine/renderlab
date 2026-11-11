# Unreal's frame, pass by pass, next to Renderlab's (matched quarry)

Root, 2026-11-11. Measurement only. It answers one question: where Unreal's 4 ms goes, so each non-RT unit has a target pass and a number ([milestone-8.md](milestone-8.md#non-rt-goal-2026-11-11)).

## Result

1. **Sun shadows are the whole non-RT gap on the quarry.** Renderlab's non-RT sun pass is 4.2 ms still, 6.2 ms moving and 39.5 ms on a cut. Unreal's virtual shadow maps cost about 0.6 ms in every state, page rendering and projection together.
2. **Everything else is already level with Unreal.** Without the sun pass Renderlab's non-RT frame is 4.1 to 4.4 ms; Unreal's whole software-Lumen frame is 4.5 ms in this profile.
3. **A third of Unreal's frame is temporal super resolution** (1.4 to 1.5 ms). Renderlab's reconstruction and post together are 1.2 ms.
4. **Unreal's cut costs nothing:** 4.57 ms on the cut frame against 4.49 ms still.
5. **Matching Unreal's shadow cost gives parity, not a win.** Sun shadows at 0.6 ms put Renderlab non-RT at about 4.7 to 5.0 ms against 4.5 ms. Going well past Unreal needs GI (1.6 to 2.0 ms against Unreal's 1.2 ms) and the fixed passes cut as well.

## Unreal, software Lumen at Epic

UE 5.7.4 packaged build, D3D12 SM6, offscreen, 1920x1080, VSM with SMRT 8x8, TSR at 100 % with 200 % history. One `ProfileGPU` frame per state, with RDG events at level 3 and async compute off so passes attribute. Inclusive milliseconds.

| Pass | Still | Walking | Cut frame | Frame after |
|---|---|---|---|---|
| **Frame total** | 4.49 | 4.55 | 4.57 | 4.54 |
| Temporal super resolution | 1.40 | 1.51 | 1.50 | 1.49 |
| Lumen screen probe gather | 0.64 | 0.64 | 0.63 | 0.62 |
| Nanite visibility buffer | 0.40 | 0.38 | 0.42 | 0.41 |
| Shadow depths (virtual shadow map pages) | 0.28 | 0.28 | 0.28 | 0.28 |
| Direct lighting (includes shadow projection) | 0.23 | 0.23 | 0.22 | 0.22 |
| Lumen scene update (card captures) | 0.20 | 0.19 | 0.20 | 0.19 |
| Base pass | 0.18 | 0.19 | 0.17 | 0.16 |
| Translucency volume lighting | 0.15 | 0.14 | 0.14 | 0.14 |
| Lumen scene lighting (radiosity) | 0.14 | 0.14 | 0.14 | 0.14 |
| Lumen reflections | 0.07 | 0.07 | 0.06 | 0.07 |
| Virtual shadow map page marking | 0.07 | 0.07 | 0.07 | 0.07 |
| Nanite base pass, stochastic lighting, VRS, virtual texture (each) | 0.04-0.06 | | | |

- The still column is the mean of three launches; its total ranged 4.47 to 4.52 ms.
- Hardware Lumen at Epic: 5.45 ms still (5.39 to 5.55), 4.76 walking, 4.65 on the cut, 4.71 after. Its screen probe gather is 0.57 to 0.83 ms and direct lighting 0.56 to 0.68 ms.
- **The profiled frame is slower than the yardstick's timing.** The yardstick's software-Lumen frame is 4.02 ms still with async compute on ([p-yard.md](p-yard.md)); this profile's is 4.49 ms. Use the table for shares, and the yardstick for the ratio.

Grouped:

| Group | Unreal passes | Unreal, ms |
|---|---|---|
| Primary visibility and materials | Nanite visibility buffer, base passes | about 0.65 |
| Sun shadows | page marking, shadow depths, the shadow share of direct lighting | about 0.6 at most |
| GI and reflections | screen probe gather, scene update, scene lighting, reflections, translucency volume | about 1.2 |
| Reconstruction and post | TSR and the rest of post-processing | 1.46-1.57 |

## Renderlab on the same frames

Main 575596b code, RTX 4070 SUPER, 1080p, GPU pass log, two runs (P-RELIT's yardstick run, [p-relit.md](p-relit.md)).

| Pass | Non-RT still | Non-RT moving | Non-RT cut | RT still | RT moving | RT cut |
|---|---|---|---|---|---|---|
| **Frame** | 8.4-8.5 | 10.5-10.7 | 43.2-45.1 | 3.15-3.23 | 4.02-4.03 | 13.2-14.6 |
| Visibility (RT: primary and sun rays) | 0.63 | 0.65 | 0.63 | 0.55 | 1.45 | 11.36 |
| Sun shadow | 4.24 | 6.22 | 39.47 | in visibility | | |
| GI (all `gi_` passes) | 1.57 | 1.77 | 2.02 | 0.72 | 0.73 | 0.78 |
| Shading | 0.67 | 0.67 | 0.65 | 0.65 | 0.65 | 0.64 |
| TR1 | 0.53 | 0.53 | 0.47 | 0.53 | 0.52 | 0.47 |
| Post | 0.64 | 0.68 | 0.93 | 0.64 | 0.64 | 0.66 |

## Pass against pass (non-RT, still)

| Group | Unreal | Renderlab | Reading |
|---|---|---|---|
| Primary visibility and materials | 0.65 | 0.63 + 0.67 shading | Level; Renderlab's shading also does the lighting that Unreal's 0.23 ms direct lighting does |
| Sun shadows | about 0.6 | 4.24 (6.22 moving, 39.5 cut) | The gap |
| GI and reflections | about 1.2 | 1.57 (2.02 on a cut) | 0.4 ms behind; the quarry traces no reflection rays on either side worth counting |
| Reconstruction and post | 1.46-1.57 | 1.17 | 0.3 to 0.4 ms ahead |

## How it was measured

- `py -3 -B tools/p-yard/ue.py profile quarry ue-sw-e ue-hw-e --frame N` inside a lease, for N = 190 (walking), 320 (the cut) and 321. Each launch also carries the driver's own frame-64 profile of the still pose. `tools/p-yard/ue_table.py` builds the tables from the logs.
- Six launches, about 13 s each, no window. Outputs: `D:/Coding/renderlab-work/p-yard2/ue/quarry/profile-*/`.
- One instrumented frame per state and launch. Inclusive parents overlap, so rows are not summed across levels.
- Not measured here: Unreal's picture on these frames (the yardstick has it), and any scene other than the quarry. Showcase has no Unreal build.
