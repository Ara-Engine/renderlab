# Shadow reuse and the next cost experiment

The accepted renderer remains unchanged at the production source underlying `45fca8d`. The quarry history experiment still fails its moving-cost screen. This unit measures whether executing the same visibility rays in compute changes that result enough to justify implementation.

## Reuse is a candidate, not accepted quality

The independent CPU preflight proposes a solid-support fallback when the strict history anchor fails only its normal or plane check. Four surrounding taps must remain nearby, all eligible taps must contain the same exactly lit or shadowed value, and light-facing or blocker-distance guards must pass. The existing four-ray contradiction check follows reuse. Fractional history does not cross facets through this fallback.

On the two retained quarry transitions, the emulation accepts 77.2% and 76.6% of eligible surface resets. About 99.84% and 99.86% of those accepted pixels equal the available fresh 256-ray estimate exactly. These are measured agreements with finite estimates, not full-image ground truth or a proof against thin-shadow leaks. The fallback has not run on the GPU or passed the contact and motion gates.

The report's probability `(1-e)^4` assumes independent Bernoulli trials. The renderer uses a deterministic low-discrepancy pattern. Unbiased marginal samples alone do not establish independence, so those expected-error numbers remain a heuristic model, not probability bounds. The preflight also compares errors over different selected pixel populations. It does not establish that the proposed fallback is more accurate than existing reuse.

The fitted execution model predicts that residual fresh-ray bursts still dominate cost. Its assumed 8-by-4 grouping is not an observed GPU execution map, and two timing points do not prove divergence or helper-lane cost. Compilation to `OpKill` alone does not measure every helper invocation. Existing zero hidden-query instrumentation remains the actual observation.

## One isolated compute probe

The data is a frozen set of covered receiver ray descriptors plus the unchanged acceleration structures, light disk and sampling sequence. Use exact float32 receiver origins and light-volume inputs from the same fragment execution. Do not silently substitute the quantized history normal or reconstructed world position; that would change the workload before comparing shader stages.

One isolated worker owns the private instrumentation and an existing warm target. Main owns this record and integration. No production reuse rule, public API or shader policy changes in this unit. Headless work must not change focus or capture another application. Check for a running game before taking the GPU slot.

1. Reuse the preserved quarry cost candidate and assets. Capture the exact cold-frame receiver inputs once outside timing if the history representation cannot reproduce them. Preserve the original fragment result and source identities.
2. Dispatch the same 256 directions per covered receiver in compute. Measure the existing closest-hit traversal and a terminate-on-first-accepted-hit variant. Both preserve ray origin, range, depth bias, material and visibility semantics. The latter omits nearest-distance output and cannot substitute for the complete history algorithm on its own.
3. Check visibility counts before the timing blocks. Each variant must agree with the same-run fragment result on at least 99.9% of covered receivers, and agree with each other to the same threshold. Report the entire difference distribution and worst mismatch. A miss invalidates the comparison; do not relax the threshold or repair it through changed bias.
4. Use two short alternating timing blocks after workload warmup. Record a same-session fragment control, compute timestamps, dispatch overhead, receiver preparation and extra memory separately. Keep atomics, readback and validation instrumentation outside reported timing regions. Run one separate validation observation. Retain contention and clock observations.
5. Stop for validation errors, device loss, or any cold or probe frame over two seconds. Permit one extra timing block only if the first two disagree on the decision. Do not run the contact suite, a quarry replay campaign or soft 4x in this unit.

The proposed 0.20 and 0.34 ns-per-ray lines are provisional forecasts derived from the old static base and proposed receiver counts. Report them alongside the measured milliseconds, but do not treat them as engine acceptance thresholds. Faster compute still needs receiver preparation, sparse dispatch, distance information, storage, shading and integration. A slow dense probe rejects this tested implementation as the next route; it does not prove every possible 256-ray algorithm impossible.

If the probe leaves plausible end-to-end room under the measured hard-4x anchor, root selects one bounded implementation unit. Otherwise stop this implementation route and use the evidence to choose a different fresh-visibility mechanism. The internal hard-4x anchor does not replace the matched Unreal quality and performance contract.

## Evidence and development cost

The preserved review, predeclared rules, CPU script and results are in `D:/Coding/renderlab-work/m6-microfacet-next/`. The CPU preflight reports 2.48 seconds excluding input hashing. Root checked the script's error formula and selection logic and retained the limitations above. No new GPU result is claimed here.

The instrument must report implementation time separately from compilation, verification and analysis. One focused build and one short headless session are the intended scope. Earlier accepted binaries and failed evidence remain intact.
