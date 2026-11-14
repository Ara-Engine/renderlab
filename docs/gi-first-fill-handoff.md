# First-fill handoff (close-out WIP)

Base: main 190fa08f. This branch is HALF DONE and MUST NOT MERGE or run as-is.

The previous prototype is t3/gi-first-fill at d5f9c252. The shipped ordered 12k
scheduler is on main through #121. This new branch contains only the start of
its reproducible replacement, stopped by the fleet close-out instruction.

Edits saved here:
- Read raw bounce, rather than filtered HOT bounce, when classifying first fill.
- Fix the selected budget to 12,288; retire timing-fed mode selection.
- Start replacing claim atomic tickets with a stable chosen-key threshold.
- Add four pipeline handles for high/low key histograms and selections.
- Calculate scratch capacity for a slot-sized first-fill list and refresh lists.

EXACT NEXT STEP: implement the missing gi_priority_claim_hist_high,
gi_priority_claim_select_high, gi_priority_claim_hist_low and
gi_priority_claim_select_low entry points, then wire them before claim rounds.
Update compact_available's caller in src/gi_cache.rs to supply slots. Replace
atomic gather ranks with stable slot-prefix ranks in gi_refill_compact.wgsl;
allow all visible raw-source first fills, with a fixed refresh cap. The current
Rust caller and WGSL are deliberately not build-ready. No build or test was run.
The default enable change is provisional and must only land after the run gate.

Implementation direction: chosen key-a is unique after gi_miss_chosen, so two
16-bit radix histograms can choose an exact deterministic claim threshold without
using raw append order. Keep the visible reservation and spend remaining claims
on background. Gather ranking can use per-slot classification, per-block integer
prefix counts, and a stable prefix across blocks. Reuse the existing ray arena
before sky setup; it is no longer needed by gathers at claim admission time.
No timing-fed count, ticket arrival or arbitrary tie may choose the admitted set.

Current acceptance: PC integration smoke on the three game scenes, both paths;
no crashes, no slower view, inspect cut +1/+20. No new comparison campaign,
identity lease, Linux suite or design-note gate. Off-switch for the first day.

Preserved local evidence under D:/Coding/renderlab-work/u-gipass:
- outpost-12k-tail/report.md: original moving/held tail and corrected pose join.
- outpost-12k-tail/aa-control/report.md: accepted A/A, C/C, C/A control.
- spin-analysis/report.md: regional gather-rotation bias, no running-mean code.
- combo-timing-report.md: historical prewarm-on 12k timing.

The fire/deck 12k residual is real above repeat spread; the ground residual is
inside repeat spread. A/A was byte-identical at six held offsets, while C/C was
not. The replacement must not become less reproducible than A. These are saved
findings, not a request to repeat the dropped provenance/order-probe campaigns.
