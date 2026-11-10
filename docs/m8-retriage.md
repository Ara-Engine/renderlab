# M8 P-RETRIAGE: rejected and optional speed-ups

2026-11-10. Documentation-only audit of main `0512465`, delivered on
`t3/p-retriage`. No build, Cargo command, renderer execution, GPU work or new
image inspection was performed. Timings and image judgements below are the
original units' recorded evidence, not fresh measurements.

## The decision rule

[Islam's clarification](milestone-8.md#quality-bar) replaces identity, every-crop
and fixed speed-floor acceptance rules: judge the visible image against Unreal,
still, moving and on cuts, on both paths; keep faster changes that are not visibly
worse. A failed hash or tiny RMS/flicker delta alone does not establish a visible
regression. Conversely, recorded fringe loss, displaced shadows, collapsed
penumbrae and self-shadowing are substantive defects, even when most crops win.

**Finding:** there is no documented, large, immediately mergeable ARA1 frame-time
win discarded solely for a sub-visible score. The largest dormant measured win
is P-RTS, but its final design loses real S2 shadow fringes. Some earlier listing
variants deserve a visual recheck. The cleanest smaller opportunity is the
non-RT-only batched GI gather; its combined RT/non-RT implementation was dropped
because RT did not win. P-FLAT2 is a particularly clear obsolete numeric gate,
but has **no measured speed-up** and must not be advertised as one.

Verdicts mean:

- **REVIVE:** no recorded visible defect; a current visual check and paired timing
  can qualify it. This is a recommendation to resume, not merge approval.
- **RECHECK:** the record cannot decide visibility or current value. The named
  image/sequence check is the deciding visual check, followed by timing.
- **STAYS REJECTED:** a substantive image defect or measured slowdown remains;
  changing an acceptance rule does not repair it.

Historical baselines matter. An optimization faster than November 7 Exact is not
automatically faster than today's Surface, and faster than optional 256-sample
specular filtering is not faster than today's unfiltered default. Pass-time
savings are labelled separately from drained frame service. Do not add numbers
from different cohorts or assume alternatives' savings stack.

## Ranked work to resume

Order is by measured potential on **ARA1 game and quarry**, with uncertain or
unmeasured gains explicitly below the measured leads. This is a work queue, not a
claim that its entries already pass the visual bar.

| Rank | Candidate / verdict | Game opportunity | Quarry opportunity | One deciding visual check |
|---|---|---|---|---|
| 1 | Earlier P-RTS listing variants, **RECHECK** | RT cut sun/primary pass 17.45 ms to 1.40–2.40 ms for in-disk variants; 1.41 ms for the 1.5x-cone variant. About 15–16 ms of pass time, not independently measured service for every variant. | Final listing demonstrates 8.39 ms cut service saving, but that final design fails S2; the earlier variants have no separate quarry service table. | Replay the **S2 0.04-rad large-light moving fringe**, including its cold frame, for each earlier variant. ARA1's tiny sun cannot decide this. |
| 2 | Batched GI gather, **REVIVE for a non-RT-only experiment** | Non-RT moving service 11.40 to 11.18 ms: 0.22 ms in the quiet screen. RT game 5.14 to 5.12 ms is inconclusive. | Non-RT moving service 11.85 to 11.34 ms: 0.51 ms; matched fill-window gather 5.085 to 4.657 ms. RT moving 3.89 to 3.97 ms, so retain the old RT kernel. | Inspect quarry cut-to-settle for missing/delayed bounce after selecting the batched kernel only on non-RT. Existing combined-candidate images were identical. |
| 3 | M6-close L5 lit-cell reuse, **RECHECK** | No game measurement; do not transfer the quarry gain. | Historical non-RT camera shadow pass 1.049–1.065 to 0.995/1.002 ms, approximately 0.05–0.07 ms (4–6%). | Moving quarry **contact/penumbra boundary** sequence with L5 alone, without L6's leaking cap; check light leaks where a reused lit cell crosses a shadow edge. |
| 4 | N8m 48-byte walk records, **REVIVE as an explicit memory trade** | No game measurement. | Historical non-RT camera shadow pass 1.527–1.536 to 1.483–1.499 ms, roughly 0.04 ms (2.5%), for +149 MiB. Current Surface gain unknown. | Current quarry moving contact/silhouette frame pair after porting the record layout; historical outputs were identical. |
| 5 | P-LTS3 C froxel-plane culling, **RECHECK; no gain established** | Dense 64-light game is the intended beneficiary; no valid timing gain. Ordinary game gain unknown. | No complete quarry pair or valid gain. | The **128-to-129-light boundary transition** with a grazing froxel-edge light, on both paths; inspect for a missing contributing light. Other unfinished scenes still need their normal acceptance checks. |
| 6 | P-FLAT2 swing restart, **RECHECK for quality, unranked by speed** | No ARA1 timing. Courtyard RT crate grain 0.079 to 0.057, non-RT 0.075 to 0.054. | No performance measurement. | Play the matched courtyard fire/Fox sequence at normal exposure on both paths, watching the brazier silhouette and arch for visibly damped fire flicker or trails. |

Rank 1 is high reward but high uncertainty: final wide-probe listing is **not**
revived. Ranks 3–4 are small, historical gains. Rank 4 was rejected for memory
rather than the three obsolete gates, so is included as an adjacent trade, not a
misclassified score rejection. Ranks 5–6 have no established speed gain and are
not members of the strict “measurably faster” set. There is no honest numerical
ARA1 frame-time ranking for them.

## Candidate ledger

### A. P-RTS: separate the numeric rejections from actual fringe loss

Source: [P-RTS sections 3–5](p-rts.md). Release Vulkan, RTX 4070 SUPER, 1080p
timing; 960x540 ARA1 quality witnesses. GPU exclusive, with compiler overlap in
some service runs. “Primary” includes camera rays and sun visibility. Storage:
merged optional machinery from `t3/p-rts` (memory records `db326de`), controlled
by **`PENUMBRA_LISTING = false`**; shaders `primary_penumbra_list.wgsl` and
`primary_penumbra.wgsl`. Archived variants/summaries live under
`D:/Coding/renderlab-work/p-rts/analysis` and `runs`; that evidence root exists.

| Variant | Recorded speed | Recorded image evidence / old rejection | Verdict |
|---|---|---|---|
| 8 in-disk probes, 5x5 | RT game cut/moving primary 1.40/1.06 ms versus 17.45/2.33 | At most 3 game pixels by 3 codes, 7 quarry pixels by 1 code; 16 worse quarry crop rows, relative change at most 2.3e-5. No inspected visible regression recorded for this variant; S2 not cleared. | **RECHECK**, S2 fringe sequence. These are numeric/image differences, not proof of perceptibility. |
| 8 in-disk, 7x7 / 9x9 | Game cut/moving primary 1.74/1.29 and 2.13/1.57 ms | Game at most 2 pixels different, quarry 4/3; 12/11 worse crop rows. No separate visible-defect judgement or large-light clearance. | **RECHECK**, same S2 check for each window. |
| 16 in-disk, 5x5 / 7x7 | Game cut/moving primary 2.4/1.6 and 2.3/1.8 ms | Game at most 1/0 pixels different; quarry 4/3; 8/8 worse crop rows. Numeric rejection, large-light evidence absent. | **RECHECK**, same S2 check. |
| Listed pixels trace twice the budget | Game cut/moving primary 1.75/1.10 ms | Up to 480 game pixels, 6 codes; 408 quarry pixels; 27 worse crop rows. Reference noise changes sign. The doc rejects crop losses, not an inspected visible defect; exact probe configuration needs recovering from its archived run. | **RECHECK**, S2 moving fringe and its noise in the matched sequence; do not infer that twice the samples fixes unlisted fringes. |
| 8 probes, 1.5x cone, 5x5 | Game cut/moving primary 1.41/1.07 ms | Game identical, quarry at most 4 pixels different, zero worse quarry crops. No S2 clearance. Superseded by wider probes, not proved defective in its own visual record. | **RECHECK**, S2 sequence. Strong ARA1 evidence, unresolved wide-light risk. |
| 8 probes, 2x cone, 3x3 | Game cut/moving primary 1.14/0.92 ms | ARA1 game identical, quarry one pixel by one code, zero worse crops. **S2 loses real fringe:** 280 cold-frame pixels become fully lit, up to 0.066 visibility loss; moving error up to 0.16. | **STAYS REJECTED**: actual fringe loss, irrespective of ARA1 scores. |
| Final: 8 in-disk plus 8 wide probes | Game service still/move/cut 4.08/6.05/21.61 to 4.16/5.15/5.64 ms; quarry 3.27/4.22/15.89 to 3.23/3.73/7.50; PBR cut 12.83 to 4.15. Game cut saves 15.97 ms, quarry 8.39. | All eight non-quarry scenes identical; quarry one pixel by one code, slightly better score. **S2 45/48 captures differ:** moving pixels lose up to 0.125 visibility, cold up to 0.047. The reason to stay off is the real missed fringe, not the harmless quarry pixel. | **STAYS REJECTED** as implemented. A conservative certificate would be a new implementation. |

No new patent combination is claimed in the unit: classical adaptive sampling,
no reservoir, history reuse, or spatially filtered output. Its claim notes
distinguish US 11,688,042 / 12,423,782 because the neighbourhood chooses further
rays rather than filters visibility. This audit reports that mapping, not new
legal clearance.

### B. P-NRT3 batched GI gather: recover the path that actually benefits

Sources: [P-NRT3](p-nrt3.md), superseded by the measured
[P-NRT3b](p-nrt3b.md#rejected-gather). Multiple lanes screen consecutive queued
cells in parallel, batch deferred-cell appends, and then gather live cells with
the original rays and reduction. The combined two-change candidate passed
164 ARA1 PNG, 186 Q-walk hash, 26 RF float-buffer and 14 MP1 comparisons, all
identical on both paths; 200 lib tests passed. No visible defect is recorded.

The quiet gather-only screen gives the rank-2 figures. Non-RT total GI work in
quarry windows fell 6.787 to 6.294 ms and 1.720 to 1.648 ms. RT gather at the same
windows rose 0.037888 to 0.039488 and 0.036064 to 0.038048 ms; RT moving service
was slightly worse. The early RT still phase was variable, not a proved large
regression. The unit removed gather because it could not establish a win on both
paths within the time box, rather than because of an image failure. **REVIVE the
non-RT kernel selection; RECHECK the both-path version rather than assuming an RT
win.** The split-only scheduling change is already on main (see below).

Location: original gather in `t3/p-nrt3` at `9e4d474`; P-NRT3b fixed its scalar
name to `gi_drain_base`, then restored `shaders/gi_cache.wgsl` from main. Evidence
`D:/Coding/renderlab-work/p-nrt3b`, including rejected combined `final.exe`,
screening `cand1.exe`, `trace-summary.json` and `final-summary.json`. Do not
cherry-pick the entire old branch over the already merged split. No sampling,
history or patent fallback change is introduced; existing GI P0 mapping applies.

### C. Historical non-RT shadow candidates

Sources: [N8k](non-rt-n8k.md), [M6-close section 3](m6-acceptance.md),
[N8c caster LOD](non-rt-n8c.md#caster-lod-built-measured-rejected-as-a-default-m),
[N8n](non-rt-n8n.md), [N8m](non-rt-n8m.md).
These are mostly pre-M7 measurements, included because the requested memories
explicitly point here. No ARA1 game timing exists for them.

| Change / location | Measured gain and why left out | Quality evidence, current verdict |
|---|---|---|
| Map-only, `VirtualLookup::Map`, `t3/n8k-map-lookup`; merged opt-in `shaders/virtual_shadow_lookup.wgsl` | Historical non-RT quarry moving shadow 1.95 to 0.94–0.96 ms, about 1 ms saved versus Exact. Small scenes slower: S3 0.235 to 0.47, S2 0.35–0.48 to 0.95–1.01. Caster motion no faster. | Sub-texel **false self-shadow**: quarry lit-darkened 263/242 versus VSM 87/88, S1 flips 48/38 versus 0–1. These are identified wrong shadows, not merely a crop RMS. **STAYS REJECTED** as default. Best-tie settings also free real penumbrae; no visibility-safe default is recorded. |
| Hybrid six-sample map/exact lookup, `VirtualLookup::Hybrid`, same branch/module | Quarry moving 1.95 to 1.51–1.53 ms (0.42–0.44 saved); failed the <=1 ms floor despite beating VSM on every recorded bench measure. Static 0.56 to 0.78–0.83; S3 0.235 to 0.56, S2 0.35–0.48 to 0.89–1.06: slower. | No recorded visible defect for the retained six-sample form, but no unchanged-image proof against Exact. The old speed-floor rejection no longer suffices. **STAYS REJECTED as today's general default**, because Surface subsequently measures ~1.05 ms moving and zero idle shadow cost. A fresh adaptive-selection experiment is **RECHECK**, not an established gain over main. |
| Hybrid cap 4, same settings/evidence | 1.47–1.50 ms versus six-sample Hybrid 1.51–1.53; about 0.02–0.05 saved | **Collapses S2/S3 penumbrae to umbra. STAYS REJECTED.** |
| M6-close L5 lit-cell reuse, `t3/m6-close`; `E:/renderlab-work/m6-close/variants` and `l*.sh`; `RENDERLAB_VSC_SHADER` override | Show lit cells with consensus threshold 224 / 256, camera shadow 0.995 / 1.002 ms versus 1.049–1.065. Not shipped in a hillclimb still missing the VSM/2 floor; halves walks but only ~4% time. | “Showing lit never darkens a receiver” is a one-sided observation; it does not certify no light leaks. No L5-specific inspected defect is recorded. **RECHECK**, boundary motion sequence in rank 3. Do not confuse L5 with L6. |
| M6-close L6, L5 plus `VSM_CAP_RULE=2`, cap 16 | Camera 0.762–0.785 ms, about 27% faster; cap alone 0.961–0.984 | N8e/N8f measured **3,400–3,700 additional lit umbra receivers**, not a harmless identity delta. **STAYS REJECTED.** `surface-camera-walk-floor` records the same reason. |
| Caster simplification, `VirtualShadowSettings::caster_error_texels` (default -1), N8c | Non-RT quarry turning shadow 4.54–4.89 to 3.82–4.18 ms at 0.25 texel, 3.44–3.76 at 0.5 texel; up to ~1.1 ms saved. Static gain small. | Coarse casters above source receivers cause **false shadows**, lit-darkened 60 to 152 / 2,437. Moving origins out to compensate loses umbra. **STAYS REJECTED** as a default; recorded geometry mismatch, not just metric noise. |
| Every surface cell shown, N8n P1; historical `VSC_REUSE`/canonical variants under `E:/renderlab-work/n8n` (later screen switches removed) | Non-RT quarry camera 1.31–1.33 ms, faster than preceding Exact ~1.5 ms; slower than final Surface 1.03–1.04 | Moves contact/penumbra borders to neighbouring cells and darkens lit receivers (697 versus VSM 87); canonical jitter adds S1 leaks/S2 flicker. **STAYS REJECTED**, with real boundary displacement and no current speed advantage. |
| N8e rolling screen-space reprojection, `E:/renderlab-work/n8e/variants/s9-reprojection*` | Intended motion reuse; bounded form moving 1.8–4.1 ms, no better than dense; static 0.99–1.00 versus 0.84–0.91 | **Shifted/stuck shadow edges**; after a 2 px camera step 6,159 wrong pixels, 1,792 false shadows; bounded drift still 508/117. **STAYS REJECTED**, defect and no gain. |
| N8m 48-byte records, `t3/n8m-occupancy`, `E:/renderlab-work/n8m/variants/rec48.diff` (file exists) | Camera shadow 1.527–1.536 to 1.483–1.499 ms, ~2.5%; not kept because +149 MiB. Historical total 3,589 MiB still below comparator 3,912. | **Byte-identical** on both poses and caster identity. No visible defect. **REVIVE** as a measured memory/speed choice, with a current Surface timing. Not rejected for quality or a speed floor. |

Patent status of these legacy shadows needs care. The historical unit notes do
not provide a complete claim clearance for each mode. The subsequently recorded
[VIA mapping](patent-via-8471853.md) marks the shipping triangle march **AT RISK**
for US 8,471,853 and 8,605,088. That applies to candidates retaining the march,
including Hybrid, L5/L6 and changed walk-record layouts. Map-only cannot be
declared patent-clear merely because it drops the exact walk; N8k calls its
sampling SMRT-like without a complete independent mapping. A retained speed
change does not resolve these existing risks. Do not promote a new patented
fallback from this audit. The [S4 gather design](design-nonrt-sunvis-s4.md) offers
a mapped design-around, not an already measured replacement.

### D. P-FLAT-TR1 and P-FLAT2: obsolete quality gate, no speed evidence

[P-FLAT-TR1](p-flat-tr1.md) B/B' ship opt-in via
`Reconstruction::light_budget`, default 0. They improve crate crops but lag the
fire numerically (B' 0.002 RT wall slope 0.972 versus 0.985). The unit explains
the temporal lag, but does not quantify perceptibility. No paired frame timing
was completed; the 64-ray restart cost of ~1.5 ms is an **estimate**, not a saving.
**RECHECK** the fire sequence for this quality option, with P-FLAT2 preferred
because it bounds intensity swing over the whole history. Neither belongs in
the measured-speed set.

P-FLAT2's note is **only on `t3/p-flat2:docs/p-flat2.md`**, not main. Read it
together with the newer verification record in
[`6b41b45:docs/p-verify.md`](https://github.com/Ara-Engine/renderlab/blob/6b41b452d29e68ed54b1601bc6cc8a56f913c13d/docs/p-verify.md),
also reached through the decisions ledger. That later record supersedes the
branch note saying the silhouette correction was never compiled:

- Corrected `68a902b` was built, six courtyard flights and Fox fixture ran.
  Full-frame frame-24 comparisons on both paths were visually inspected,
  including foliage, transparency, reflections and shadow edges. No perceptible
  flicker regression was established.
- Budget 0.004 wall slope RT 0.984687 to 0.984580 (difference -0.000107),
  non-RT 0.982017 to 0.981980 (-0.000037). Other budgets also miss the strict
  wall floor. Those numeric misses stopped the plan; ground sometimes improves.
- Crate grain improves as in rank 6. At budget 0.001 the non-RT arch crop is
  0.017 versus 0.016, an unresolved score, not a reported inspected defect.
  Candidate RF was interrupted; Q-walk, MQ1/MV1/SK1/object-motion/lib matrix
  remained incomplete. Process wall durations are **not frame performance**.

**RECHECK** the fire/Fox motion sequence (rank 6); the old slope floor is not a
valid standalone rejection. No speed number exists, and relit RT frames keep
the same 64 rays, so do not promise a frame-time benefit. Branch source and
`D:/Coding/renderlab-work/p-flat2`, plus P-VERIFY's `flat2/analysis` and inspected
`flat2/inspection.png`, preserve the work. Patent notes use engine light state
and receiver position, not pixel moments or colour differences; distinguish
US 12,437,371, pending 2026/0162231, 10,116,916 and 10,362,289, with P0-cleared
plain min/max clamping. This is the unit's mapping, not a new assessment.

### E. U1b: faster alternatives to an optional expensive mode

[U1b](m7-u1b.md) is an important denominator trap. `SPECULAR_AA=false` remains
the standard default. The passing spatial distribution was left optional for
cost and quarry memory. Turning it on **adds** work compared with that default;
its passing quality and missed 2x floor do not establish a discarded speed-up.

There are, however, measured faster replacements for the optional full-moment
256-sample filter, with numeric losses to that achieved result. These are
**RECHECK** for users selecting filtered shading, not a claim of faster ARA1
default frames. Location: merged experimental `filtered_brdf` selectors and
`t3/m7-u1b`, with variant-index evidence under
`E:/renderlab-work/m7-u1b/cost-iteration-1/`, documented in U1b's continuation.

RTX 4070 SUPER, dev test build, 1080p **14-triangle specular fixture**, no
environment or shadow pass; camera-shading control SW/RT 1.779/4.041 ms:

| Selector index | SW / RT camera shading ms | Saving versus optional 256 control | Quality evidence / verdict |
|---|---:|---:|---|
| 12, v3 RGBA8 spatial 256 | 1.773 / 4.041 | 0.006 / 0 | Zero failures in original/refined gates; no established useful speed gain. Retained compatibility route, not a lost optimization. |
| 13, adaptive factor 4 | 0.943 / 1.829 | 0.836 / 2.212 | 72/72 failures; recorded mapped r=.30 miss. **RECHECK** mapped r=.30 moving highlight for visible tail/energy change. |
| 14, adaptive factor 8 | 0.905 / 1.984 | 0.874 / 2.057 | Passes original/refined gates, but camera RMS .158/.159 to .325/.328 versus achieved 256. **RECHECK** moving r=.08/.15 mapped highlights. |
| 15 / 20, variance hybrid with GL9 / GL25 | 0.810/1.471 and 0.845/1.643 | 0.969/2.570 and 0.934/2.398 | Zero original/refined failures; not lossless against achieved filter. **RECHECK** same highlight sequence. |
| 16, fixed spatial GL64 | 0.315 / 0.705 | 1.464 / 3.336 | Refined failures 42/34. **RECHECK** mapped highlight motion; failures alone do not decide visibility. |
| 17, spatial 144 | 0.635 / 1.294 | 1.144 / 2.747 | Refined failures 19/19. **RECHECK** same check. |
| 18, spatial 64 | 0.315 / 0.696 | 1.464 / 3.345 | Refined failures 23/28. **RECHECK** same check. |
| 19, spatial 256 v3 + GL25 | 1.067 / 2.240 | 0.712 / 1.801 | Zero original/refined failures; changed achieved output. **RECHECK** geometric r=.08 highlight. |
| 21, separable hybrid + GL25 | 0.643 / 1.099 | 1.136 / 2.942 | Zero original/refined failures; whole-frame static RMS .152/.153 to .172/.171; mapped r=.15 RMS .451/.455 to .589/.589. **RECHECK** r=.15 mapped highlight motion. |
| 22, legacy RGBA8 full 256 | 1.632 / 3.437 | 0.147 / 0.604 | Almost unchanged aggregate errors, tiny RT bias increase .041739 to .041825; no recorded visible defect. **REVIVE** as the low-memory optional filtered route after visual check/timing. |
| 23, legacy 256 + GL25 | 1.008 / 2.165 | 0.771 / 1.876 | Zero original/refined failures; small numeric changes versus achieved 256. **RECHECK** geometric sharp-highlight motion. |

The indexed modes cannot be ranked in ARA1 milliseconds without measuring how
often these materials occur. Within this fixture, 21 saves most among the
documented zero-original/refined-failure alternatives; 22 is the most conservative
near-equality opportunity. Earlier covariance/Gaussian-mixture/angular-tail
screens also failed numeric gates, but their per-variant speed-ups are not
established in that initial table; the broad-GGX tail energy defect is explicitly
diagnosed. Do not invent speed numbers for them. U1b's approach is spatial only,
with no temporal moments, variance clipping, SVGF or patented temporal fallback;
that is the recorded patent design-around basis. Quarry memory remains a real
resource question even after removing a fixed speed floor.

## Checked exclusions and already recovered results

These prevent old rejection headlines or attractive diagnostic numbers becoming
false revival claims. They also account for the specifically requested units.

| Unit / location | Evidence and disposition |
|---|---|
| P-S1 and original S3, `t3/p-s1`, `D:/Coding/renderlab-work/p-s1/s1-umbra.patch` | [P-S1](p-s1.md): game cut shadow 26.70–26.75 to 28.35 ms, quarry still 1.26–1.32 to 2.04. Only 2 of 26 whole-image changed quarry pixels move away from reference: obsolete quality rule, **but slower anyway, STAYS REJECTED**. 15% umbra-work floor fails at 2.8%; that proxy is not a measured speed-up. S3 replayable game work is only 1.2%. S1 claim mapping says no US 10,055,883 / HFTS / 2011/0141112 combination; unchanged march now has separate VIA risk. |
| P-S3Q, `t3/p-s3q`, `D:/Coding/renderlab-work/p-s3q/s3q-replay.patch` | [P-S3Q](p-s3q.md): 40/40 frames identical; quarry cut shadow 9.22 to 11.00 ms, game 26.73 to 28.66. **STAYS REJECTED**, slower. No visible defect needed to reject a slowdown; VIA march risk also remains. |
| P-NSQ2 C1 / C3, `t3/p-nsq2`, external `variants` | [P-NSQ2](p-nsq2.md): C1 exact clear proof game cut 31.72–31.75 to 31.53–31.77 (no established gain), quarry 32.79 to 32.24 single screen; C3 ring replay game 44.51–44.65, quarry 33.84, slower. Both identical. Rejected for no benefit/slowdown, not identity. **STAYS REJECTED** as measured; C1's isolated quarry delta can justify a new short timing if prioritized, not a claimed cleared win. |
| P-NSQ2 `peel_own_near`, existing `D:/Coding/renderlab-work/p-nsq2/variants/peel-own-near.patch` | Quarry contact .01505 to .01493, game .02394 to .02395 / .02398 to .02400: numeric rejection. But quarry moving shadow 3.81 to 7.21 ms, cut 34.2 to 58.5; game cut +3–4 ms. **STAYS REJECTED as a speed-up**, not faster. Could remain a separate quality investigation. |
| P-NSQ near-only cap / original cap16 | [P-NSQ](p-nsq.md): local near cap improves quarry .01505 to .01491 but costs real wide-sun penumbra (.02398 to .02407–.02414). No isolated faster timing for this near-only form. Full cap and fixed-eight sampling are demonstrably harder/grainier than RT and collapse wide penumbrae. **STAYS REJECTED** as a faster-default rollback. |
| P-NSQ2 diagnostic component removals | Disabling coarser levels saves game cut ~1.91 ms, peel ~2.13, refinements ~0.99; march-off saves ~24 ms game/~28 quarry. The note explicitly says **attribution controls, not candidates**, without quality acceptance. No previously rejected speed-up to merge. VIA mapping still applies to retained march portions. |
| P-NSQ3 | [Initial branch note](p-nsq3.md) is stale: decisions ledger records subsequent quiet default-8 verification and merge, game cut shadow 31.410 to 26.720 ms, quarry 30.891 to 9.277, identical 46/46. Four lanes 27.68, 32 lanes 31.35 are worse than retained 8. **Already recovered**, no unmerged win to resurrect. |
| P-R2, `t3/p-r2`, `tools/p-r2` | [R2](p-r2.md): ray-count imbalance game 1.0797, PBR 1.1433, RF max 1.08365, below proxy floor 1.4. **No compaction implemented or timed.** The 0.2–0.4 ms saving is a design forecast. New bar permits a redesigned cheap experiment, but no measured faster patch exists; not a STAYS REJECTED-on-quality judgement. Patent status of a future implementation not established by this counter probe. |
| P-GIE / G-E, `t3/p-gie`, `tools/p-gie` | [G-E](p-gie.md): max cell keys game 58 RT/83 non-RT, quarry 143 both, exceeds bounded-design gate 40. **No cooperative implementation or speed measurement.** A fallback/spill design is new work, not a wrongly rejected measured gain. No new patent method implemented. |
| P-LTS3 A, B, BRDF hoist and C; `D:/Coding/renderlab-work/p-lts3` | [P-LTS3](p-lts3.md): A/BRDF image pairs identical; no consistent gain. B's unstable 15.81/30.79 ms cannot certify a win or systematic regression. C patch exists and has 24 LC PNGs, 960p/odd-edge and three 1080p pairs identical, inspected prefix preserved; no quiet gain, fourth pair/MP1/quarry/courtyard/128–129 unfinished. C is **RECHECK** (rank 5), the others have no demonstrated faster result. Unit records no new patented technique. |
| P-LTS2 D, `D:/Coding/renderlab-work/p-lts2/d-ready.patch` as documented | [P-LTS2](p-lts2.md): byte-identical, but contended four-case service 6.530/13.433/4.594/8.568 to 6.711/13.642/4.883/8.724 ms: no benefit. **STAYS REJECTED** on available timing, not because it missed 3.5 ms. No sampling/patent method change. |
| P-LTS radius-zero guard | [P-LTS](p-lts.md): six RT soft-spot pixels differ, max 3 codes; reverted. No separately measured speed gain. **RECHECK** soft-spot penumbra in motion only if an isolated timing first shows it useful; not a ranked speed win. |
| P-NRT / P-NRT2 | [P-NRT](p-nrt.md) exact cuts already merged; [P-NRT2](p-nrt2.md) environment/fog fix merged. GI vec4 loading conflicted with packed BVH and had no measured gain; not an obsolete gate casualty. |
| P-NRT3b two submits | Branch note says DON'T MERGE, **superseded by main's decision**: source equivalent to `1ff89af` merged. Game recovered RT 4.90 to 4.63 / non-RT 10.16 to 9.90; quarry recovered 4.57 to 4.18 / 13.54 to 12.85 ms. Early RT still pooled median +0.6%, mean improved, overlap inconclusive. Already kept under sensible interpretation; do not revive twice. |
| P-MEM packed BVH | Initial +5.5/+6.1% concern superseded by [P-BVHT](p-bvht.md) quiet pair: moving +0.44%, cut -0.91%, recovered -1.04%, all service means improve and 532 HDR/PNG comparisons each identical. ~95.14 MiB saving stays merged. Not an off-default casualty. |
| M7 U1 / U1b, TR1/GI/reflection opt-ins | [U1](m7-u1.md) costs +0.002 ms and has r=.08 bias/flicker defect; not faster. U1b default issue and faster optional replacements are separated above. Historical TR1/G1–G2/R1 opt-ins mostly **add** work and later became defaults in the overnight push; old missed all-population/speed gates are not evidence of a discarded frame-time saving. |
| M7 CC1 shared surface/shade call site | [CC1](m7-cc1.md): native compile 36.854/34.182 to 23.582/15.883 s, identical game pixels, but frame service RT 5.832 to 10.709 ms / non-RT 9.192 to 10.705. **STAYS REJECTED for frame throughput**. This is a development-latency trade, not an identity/speed-floor rejection. Retained single-call-site GI refinement is already merged. |
| P-SHC / P-SHC2 structural split | Compile-only diagnostics reduce native compile to 11.04/16.93 s, still over 1–2 s; no produced image or producer-pass/frame-cost A/B for that split. Not a measured faster rendering candidate; a smaller latency improvement could now be worth a complete implementation. |
| P-S4 / S4B | [S4](design-nonrt-sunvis-s4.md): 26.7 to 4–8 ms is a **forecast**, not a completed measured patch. The old “reject below 10 ms saving” floor is revoked by milestone-8 and the visual-gate memory. The gather form has a design-around claim map; scatter form was dropped for US 10,055,883 combination risk. Not in the historical measured ranking. |

## Evidence coverage and limits

Read the M7/M8 decision rows, current [gap list](m7-gaps.md), milestone-8 bar,
brief/handoff and industry-cost methodology; the linked P-RTS, P-S1, P-FLAT/TR1,
branch-only P-FLAT2 plus later P-VERIFY, P-LTS/LTS2/LTS3, P-NSQ/2/3,
P-NRT/2/3/3b, P-GI, R2/G-E, U1/U1b, CC1 and legacy shadow notes discussed
above. Newer ledger decisions take precedence over stale branch headlines.

Read the requested Claude memories `penumbra-listing-limits`,
`map-only-shadow-lookup-limits`, `surface-camera-walk-floor`,
`game-cut-cost-not-umbra`, plus `surface-cache-lessons`,
`walk-record-compression-limits`, `silhouette-history-lessons`,
`mip-tail-sampling-identity`, `vsm-lookup-latency-not-divergence` and
`visual-gate-incremental-wins`; searched the memory directory for other
rejection/default/floor references. Memories are leads; unit records decide.
For example, texture feedback residency's moving blur is a visible quality
reduction, not a hash-only objection; compression/paged walk records were slower.

Local existence was checked for the P-RTS/P-FLAT2/P-NRT3b evidence roots,
P-LTS3 C patch, P-NSQ2 own-peel patch, N8m rec48 diff and M6-close evidence root.
Other individual archived artifacts are locations reported by their unit notes;
availability of every old binary/variant was not certified. No raw dump was
created or deleted. Only this document is changed by P-RETRIAGE.

The next owner should reuse existing assets and witnesses, inspect the named
deciding motion sequence, and take one short paired current-main timing per
retained candidate. A tiny score difference is evidence to explain, not an
automatic veto. None of this audit establishes new superiority over Unreal or
new current-main frame times.
