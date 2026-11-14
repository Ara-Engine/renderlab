# Held thin-line coverage experiments

Default off: `RENDERLAB_TR1_COVERAGE_UNORM=1` changes only coverage precision;
`RENDERLAB_TR1_COVERAGE_CYCLE=1` also settles eligible held geometry over the
256-point jitter cycle. The cycle switch includes the precision variant.

## Reproduction and input split

Ara P198, Glint start at tick 4, 1920x1080 native, 400 settling frames then
eight captured jittered frames. Metric: each pixel's maximum absolute Rec.709
luma difference over the seven adjacent frame pairs, in 8-bit output codes.

| Input | Non-RT >=4 / >=16 | RT >=4 / >=16 |
|---|---:|---:|
| Baseline | 969 / 88 | 509 / 12 |
| TR1 off | 0 / 0 | 0 / 0 |
| Jitter off, TR1 on | 0 / 0 | 0 / 0 |
| Near-point sun and idle GI cache | 963 / 88 | 510 / 13 |

The last row is a sampling proxy, not a cached soft-shadow image: the sun has
a positive 0.000001-degree disk, required by the non-RT policy, and GI uses
`rotate=0`. Its measured tail contains no GI gather/update work. Native scale
and no sharpening were already the baseline. Lighting variance does not explain
the large edge changes. These fresh numbers differ from the archived 1177/110
and 582/20 because the fresh video follows an additional warmup; the archived
quantized maps reproduce their recorded counts exactly.

## Precision and estimator

The still coverage window is already 256. Its f16 coverage value can absorb
positive 1/256 updates near one. A CPU replay with one miss every 256 samples
has true coverage 0.996094; its late-cycle f16 mean is about 0.937431, compared
with 0.996086 using UNORM16. Precision alone barely reduces the within-cycle
variation. It is a bias fix hypothesis, not proof that Glint's crawl is fixed.

The UNORM variant retains the existing exponential estimator. The cycle variant
accumulates a two-surface count until 256 observations, then holds coverage while
surface colour histories continue updating. On acquiring a held pair it retains
a young cumulative history; an untagged full-age history contributes at most four
prior samples, so its previous age cannot cause immediate settlement. This is a bounded estimator
with a small prior, not an exact analytic area integral.

Cycle eligibility requires native resolution, a CPU-still view, no restart,
no skin/moved receiver or paired placement, no relevant
light restart, and both static/event windows at least 256. Motion, cuts and
events immediately return to the original coverage update. The global motion
box still clamps colour; an unrelated mover does not invalidate this pair's
unchanged geometric coverage. Stable pair identity
is checked before continuing counts. Third-surface retention preserves its
existing two-layer limitation and bit-copies the coverage state.

## Storage and programming contract

No allocation, target-format, binding or memory increase. The second-layer
target remains **rgba32uint**. Only its existing packed word changes:

- UNORM: low 16 bits coverage, high 16 bits f16 effective weight.
- Counted: two f16 counts (first-surface observations and total), tagged by
  otherwise unused bit 25 of the layer identity word. Integers through 256 are
  exactly representable; a fractional spatial prior is quantized once per step.

Coverage decoding is through `tr1_second` and the picking pass's `id_settle`;
both use the shared decoder under either switch. The third-surface carry copies
the word and tag without interpreting them. The image is never sampled as a
float target. Colour fields, material/shading APIs and programmable shading are
unchanged. Switch off composes the original resolve source unchanged.

## Validation status

Prototype only. Linux lib: 301 passed, zero failed, 181 ignored; both resolve
variants and picking decoder pass Naga. Guarded release builds pass. An initial
small GPU visit starts/draws all three variants on both paths (25.18 seconds):

| Path | Off / UNORM / cycle peak held HDR luma change | Mean error vs 256 raw samples |
|---|---|---|
| Non-RT | 0.0030335 / 0.0029297 / 0 | 0.000141818 / 0.000067360 / 0.000005562 |
| RT | 0.0030335 / 0.0029297 / 0 | 0.000160756 / 0.000061929 / 0.000003740 |

These are a constant-colour synthetic fixture, not Glint. Its cut frame is
identical across variants. Inspection exposed a short-prior restart after an
unrelated mover; preserving young cumulative priors and unchanged-pair counts
addresses it. The focused regression reads coverage words directly: main's
global motion box can change the unchanged line's colour too (0.2036 HDR delta
in the control), so unchanged RGB is not a valid assertion for coverage alone.
That correction's final GPU regression and actual Glint still/motion/cut crops
remain pending. Fewer changing pixels alone do not certify absence of ghosting.
