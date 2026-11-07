# N8p: Surface is the virtual-shadow default

Decision: Islam, delegated, 2026-11-07; base main `cf001a5`, branch
`t3/n8p-surface-default`. `VirtualLookup::Surface` now defaults both the enum and
`VirtualShadowSettings`. The viewer inherits the renderer's settings; it has no separate
lookup setting or CLI default. The test manifest parser also defaults to Surface.
`VirtualLookup::Exact` remains the selectable reference. `SoftShadows::Exact` is a
different, traced sampling preference and is unchanged.

The decision adopts [N8o](non-rt-n8o.md)'s deterministic, caster-correct implementation.
It beats Unreal VSM on every recorded quality measure, fixes Exact's S1 pose 1 leak miss,
and cuts static/camera-moving work. It is below Exact on some measures. The planned
refinement is to reuse a cell only when that never scores worse than the pixel's own
walk, bringing the default toward Exact's quality. No reuse algorithm changes here.

## Recorded comparisons, not new measurements

All values below are from [N8o](non-rt-n8o.md), including its carried Exact rows from
[N8n](non-rt-n8n.md). RTX 4070 SUPER, NVIDIA 617.14, Vulkan, development/test profile;
quarry 1920x1080, LOD 0.35 px, lift 1. Quality uses the unchanged film-reference scorers;
quarry approaches each pose over 16 camera frames of 2 mm. S1-S3 validation is on;
quarry timings below have validation off. Unreal rows are the recorded matched subset,
not a new engine-wide comparison. Lower is better in these quality tables.

### Quarry, no RT

Each pair is pose 0 / pose 1. Deltas are Surface minus the named comparator.

| Measure | Surface | Exact | Unreal VSM | Delta vs Exact | Delta vs VSM |
|---|---:|---:|---:|---:|---:|
| Partial RMS | .365 / .358 | .353 / .348 | .483 / .483 | +.012 / +.010 | -.118 / -.125 |
| Solid flips | 2,685 / 2,724 | 2,434 / 2,474 | 12,390 / 12,390 | +251 / +250 | -9,705 / -9,666 |
| Umbra leaks | 3,452 / 3,525 | 3,289 / 3,366 | 14,935 / 14,935 | +163 / +159 | -11,483 / -11,410 |
| Lit darkened | 37 / 30 | 60 / 55 | 87 / 88 | -23 / -25 | -50 / -58 |
| Lost umbra edges | 2,651 / 2,699 | 2,376 / 2,424 | 11,884 / 11,884 | +275 / +275 | -9,233 / -9,185 |

Traced quarry Surface / Exact, poses 0 / 1: pRMS .308/.309 against .286/.290;
flips 714/756 against 340/356; leaks 1,447/1,478 against 1,202/1,213;
lit-darkened 1/1 against 0/0; lost edges 713/755 against 339/356. These are carried
traced-device results, not evidence of Exact parity on that path.

### S1, contact fold

Environments 0 and 1 agree unless noted; pairs are poses 0 / 1.

| Measure | Surface | Exact | Unreal VSM | Delta vs Exact | Delta vs VSM |
|---|---:|---:|---:|---:|---:|
| Partial RMS | .1085 / .1063 | .1085 / .1065 | .1642 / .1628 | 0 / -.0002 | -.0557 / -.0565 |
| Solid flips | 0 / 0 | 0 / 0 | 0 (env 1: 1) / 0 | 0 / 0 | 0 (env 1: -1) / 0 |
| Umbra leaks | 0 / 0 | 0 / 2 | 0 / 0 | 0 / -2 | 0 / 0 |
| Contacts lost | 0 / 0 | 0 / 0 | 5-6 / 2-3 | 0 / 0 | -(5-6) / -(2-3) |

### S2 and S3, every motion phase

Columns contain Surface / Exact / VSM. Exact's HDR RMS and contacts equal Surface's;
its leaks equal Surface's except S3 rotate 1 (1 against 2). Exact's solid flips are 0.
Leak/flip columns are per-phase maxima. These rows preserve every measure, including
the extra Surface leak against Exact and the moving phases that remain slower than VSM.

| Phase | Partial RMS (S / E / V) | HDR RMS (S=E / V) | Contacts lost (S=E / V) | Leaks / flips (S; V) | Shadow p50 ms (E / S / V) |
|---|---|---|---|---|---|
| S2 camera out | .0832 / .0833 / .1752 | .0107 / .0151 | 0 / 2,101 | 0/0; 62/167 | .50 / .36 / .35 |
| S2 camera hold | .0863 / .0865 / .1758 | .0115 / .0154 | 0 / 1,039 | 0/0; 57/152 | .19 / .0001 / .35 |
| S2 camera return | .0808 / .0809 / .1739 | .0103 / .0145 | 0 / 1,972 | 0/0; 62/155 | .49 / .35 / .35 |
| S2 shadow out | .0759 / .0759 / .1697 | .0099 / .0152 | 0 / 2,070 | 0/0; 65/120 | .37 / .38 / .36 |
| S2 shadow hold | .0766 / .0766 / .1675 | .0100 / .0150 | 0 / 1,059 | 0/0; 66/119 | .14 / .0001 / .35 |
| S2 shadow return | .0759 / .0759 / .1692 | .0099 / .0152 | 0 / 2,057 | 0/0; 67/120 | .37 / .39 / .35 |
| S3 static | .1186 / .1186 / .1878 | .0150 / .0194 | 0 / 2,929 | 1/0; 111/187 | .24 / .0001 / .37 |
| S3 rotate .25 | .1217 / .1217 / .1875 | .0151 / .0194 | 0 / 11,630 | 1/0; 111/190 | .24 / .27 / .59 |
| S3 hold slow | .1266 / .1266 / .1873 | .0152 / .0194 | 0 / 3,006 | 1/0; 109/191 | .25 / .0001 / .36 |
| S3 rotate 1 | .1215 / .1218 / .1863 | .0150 / .0200 | 0 / 12,289 | 2/0; 106/192 | .24 / .29 / .59 |
| S3 hold fast | .1210 / .1210 / .1875 | .0150 / .0205 | 0 / 3,378 | 0/0; 103/188 | .25 / .0001 / .36 |
| S3 jump recovery | .1159 / .1159 / .1811 | .0146 / .0219 | 0 / 3,153 | 0/0; 114/226 | .24 / .0002 / .37 |

### Flicker

Frame-to-frame residual-change RMS against the film reference, phase mean (maximum).
Surface is 4-350x below VSM in moving phases. Against Exact its phase mean is equal
or lower in 9 of 11 phases, but +0.3% in rotate .25 and +1.0% in rotate 1.

| Phase | Surface | Exact | Unreal VSM |
|---|---|---|---|
| S2 camera out | .01423 (.02288) | .01428 (.02288) | .06128 (.08098) |
| S2 camera hold | .00042 (.01333) | .00042 (.01340) | .06112 (.07462) |
| S2 camera return | .01354 (.02348) | .01358 (.02347) | .05769 (.07386) |
| S2 shadow out / return | .00488 (.00653) | .00488 (.00653) | .05270 / .05244 |
| S2 shadow hold | .00015 (.00468) | .00015 (.00468) | .05198 (.06533) |
| S3 static | 0 (0) | 0 (0) | .08033 (.09871) |
| S3 rotate .25 | .00325 (.00559) | .00324 (.00547) | .07995 (.10006) |
| S3 hold slow | .00109 (.00453) | .00119 (.00449) | .08014 (.09963) |
| S3 rotate 1 | .00633 (.03633) | .00627 (.03613) | .08155 (.10333) |
| S3 hold fast | .00203 (.00703) | .00214 (.00663) | .08239 (.10216) |
| S3 jump recovery | .00161 (.05327) | .00161 (.05327) | .08644 (.10644) |

### Timing and memory

No-RT quarry, clean-visit median ranges, shadow-pass ms. No new timing claim is made.

| Phase | Surface | Exact | Unreal VSM |
|---|---:|---:|---:|
| Static | .000 | .559-.614 | .50 |
| Sun turning, 16 frames | .570-.594 | .572-.618 | about .95 |
| Camera moving | 1.043-1.046 | 1.493-1.503 | about .5, estimate only |
| Caster moving | 1.560-1.649 | 1.676-1.707 | not recorded |

Camera work falls about .45 ms (30%) against Exact, but still costs about twice VSM's
estimated pass. Cold moving frames cost more. Long continuous turning costs Surface
more than Exact (N8o's wrapped traces); the short turning phase hides this miss.
No-RT full frames (static / camera / caster): Surface .71-.77 / 1.78-1.85 / 2.32-2.45 ms,
Exact 1.29-1.38 / 2.24-2.26 / 2.43-2.46; VSM records only 1.61 ms static.

| No-RT memory, MiB | Surface | Exact | Unreal VSM | Surface delta vs Exact / VSM |
|---|---:|---:|---:|---:|
| Allocator reserved | 3,503.7 | 3,439.7 | 3,912 | +64 / -408.3 |
| Allocated | 3,424.6 | 3,344.8 | not recorded | +79.8 / unknown |

The RT device reserves 4,137.6-4,201.6 MiB for both lookups, above Unreal VSM-RT-on's
3,946 (HRT 5,410); the non-RT memory win does not establish an RT-device win.
None of these modes meets the film-quality gate or the complete M6 acceptance target.

## Exact pins and verification

The manifest parser still uses the production default. The pre-N8p reference benches
explicitly choose Exact via `exact_virtual_settings`, unless a manifest names Surface:
N8 fixture/quarry, S1 contact fold, S2/S3 motion, and their diagnostic/variant screens.
The N1b visibility fixture/bench, M1 memory-mode round trips and N8j configuration-reset
identity test also select Exact explicitly. `n1b-batch.py`, `n1c-batch.py`, `n8-batch.py` and
`memory-m1.py` pin their historical reference configurations, including partial settings
and unnamed variants from lease scripts. Explicit Surface choices in N8n/N8o leases
remain Surface; their determinism and caster gates are unchanged. The audit covered 157
relevant historical lease/helpers under E:. Eight direct manifest generators also now
pin Exact: N8f `p4.sh`, N8g `p3.sh`, N8h `scripts/mem-runs.py`, N8i `p5.sh`, and
N8l/N8m/N8n/N8o `trace.sh`. Explicit Surface variants still win. Original launchers and
their patch are retained in `E:/renderlab-work/n8p/lease-pins/` and
`legacy-lease-pins.patch`; recorded manifests/results and the N8j frozen source snapshot
were not changed.

`scripts/n8p-default-check.py` runs one precompiled no-RT quarry process: two omitted-
lookup visits, explicit Surface and explicit Exact, both poses. It compares all three
phase hashes in full against N8o (24 comparisons), and retains the static/history caster
gates. It launches neither Cargo nor windows. `docs/milestones-5-6-decisions.tsv` is untouched.

Validated results, `E:/renderlab-work/n8p/`:

- `cargo test -p renderlab --lib -j 2`: exit 0, 114 passed, 112 ignored (117.1 s including
  the cold build; test execution .53 s).
- `cargo check -p renderlab-viewer -j 2`: exit 0 (25.2 s). No viewer was launched.
- Clippy, renderlab and viewer, all targets, `-D warnings` with N8o's existing
  `nonminimal_bool` and `items_after_test_module` allowances: exit 0 (6.9 s). Fmt passes.
- One GPU lease, `default-identity`, 2026-11-07 10:58:53-11:00:26 UTC, 92.9 s,
  RTX 4070 SUPER/NVIDIA 617.14/Vulkan, validation on. Started at 13,742 MiB available,
  minimum sampled 9,741.4 MiB;
  4 GiB start / 2 GiB abort guard, no game, no owned Cargo during the lease, no windows.
  Shared lock: `D:/Coding/renderlab-work/gpu-slot.lock`; reservation and release lines
  are in `E:/renderlab-work/gpu-schedule.md`.
- **All 24 full SHA-256 comparisons pass**: both default visits and explicit Surface
  equal N8o Surface in both poses after turning/camera/caster phases. Explicit Exact
  equals N8o/main's Exact hashes in all six comparisons. Surface's static gate is
  byte-identical to a fresh lookup and final-placement twin; history has 0 unexplained
  pixels and 0 repeat differences at both poses. `default-identity/identity-summary.json`
  contains the full hashes and caster rows. The binary SHA-256 is
  `a5733fb155714bb4b88ee145f2b9754d0ce7d67dc7255e7e88b37c7fa885089d`.
- CPU verification/build time: 150.4 s; GPU verification: 92.9 s. Reading, implementation,
  audit and documentation took the remainder of the roughly 16-minute unit. No new
  performance samples or quality scores were needed: retained N8o evidence is reused.
- The eight local lease launchers' embedded Python parses; changed batch generators
  pass a CPU manifest check for Exact pins and explicit Surface overrides. The owned
  Cargo target was deleted with scoped `cargo clean` (2,507 files, 1.2 GiB). No raw GPU
  dumps were generated, and no GPU lock, build or test process remains owned by N8p.
