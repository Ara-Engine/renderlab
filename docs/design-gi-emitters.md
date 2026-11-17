# Emitters as lights in the lighting cache (gicut, 2026-11-17)

## The defect

Outpost's hangar, with fog off and only the tone curve, against its 16384-sample path-traced
reference (main-thread/ref/outpost/sheets/outpost-hangar.png): the walls and floor are heavily
blotchy on both paths (33 codes mean, p99 115), where the reference is smooth. The hangar is lit by
glowing ceiling strips and lamps, and the strips' light reaches surfaces only through the cache.

Light from a glowing surface reaches a cache cell only by chance today. A cell's bounce value is the
L1 SH of what its fixed directions hit (`gi_gather`, 64 directions, each standing for 2π/64 sr): a
direction that meets a strip adds the strip's radiance over 0.1 sr, one that misses adds none. A
ceiling strip a few centimetres wide and a few metres away subtends far less than one direction's
share, so whether a cell's directions land on it, and how many do, changes from cell to cell. The
result is a fixed, cell-sized mottle wherever emitters dominate the light: the hangar, and Glint's
glowing chevrons and stripes on nearby surfaces. It is the same failure `gi_near.wgsl` (U-SPECK)
fixed for the surroundings of lamps, and the general one: a small bright source must be sampled
where it is, not found by uniform directions.

## The rule (next-event estimation for emitters)

Emitters are sampled as lights, by their solid angle, and the uniform directions leave them out, so
every emitter's light is counted once:

1. **The emitter list.** Every triangle of a draw whose program declares `emission`, at its world
   position, with its radiance from that program's own `emission` at the triangle (evaluated on the
   GPU by the program's cell module, as cell shading evaluates it; Ara's glow included), its area and
   its power. Rebuilt when draws, transforms or parameters change. Bounded (`GI_EMITTERS_MAX`); past
   the bound the faintest go, counted in the GI log.
2. **Direct light from emitters, per cell, in the gather.** For each emitter above the cell's horizon
   and within its reach, the exact L1 SH of a uniform polygon over the cell's upper hemisphere: L0
   from the triangle's solid angle (Van Oosterom-Strackee), L1 from its vector form factor (Lambert's
   formula, Σ θᵢ γ̂ᵢ / 2), after clipping the triangle to the hemisphere. That is exact where the cell
   sees the whole emitter, which removes the blotches outright. Occlusion multiplies it by a
   visibility fraction from shadow rays to fixed points on the triangle (a stratified set per
   triangle, the same every frame for a cell: W1, no sample counts). Emitters too faint to matter for
   the cell (bound below a fraction of its total) share one joint fraction instead of their own rays.
3. **Uniform directions leave emitters out.** A fixed direction (or a `gi_near` aimed ray) whose hit
   is on an emitting draw adds the hit's reflected light only: its outgoing radiance minus its
   emission. Emitting draws are flagged in the draw table the cache already reads.
4. **Further bounces follow.** The emitter light is in the cell's bounce SH, so cell shading's
   outgoing radiance carries it, and other cells' gathers bounce it on, as for every other light.

Receivers and `shade`'s screen probes read the same cell SH. The probes' own rays follow the same
rule with the same list: emitter radiance by explicit sampling, hits on emitting draws adding their
reflected light only. One list and one rule for the cache and the gather, not two.

Glint's chevrons and stripes, and the lamps' glowing parts, fall under the same rule. That is also
the cure for the bright specks next to small bright emitters on surfaces that see them.

## Cost model, to measure

- Build: one thread per emitting triangle when the list changes (static scenes: once).
- Gather: per gathered cell, the emitters in reach (culled by a per-emitter range from its power,
  as local lights have one) × a closed form (three edges), plus one shadow ray per significant
  emitter. Hangar: a few hundred strip triangles; Glint: the course's stripes, most out of reach of
  any cell. A world grid over the emitters takes the per-cell scan if it shows in the timings.
- Memory: the list (56 B a triangle at the bound) and a bit per draw.

## Judging

The hangar against its reference, both paths, held and walking; Glint's chevrons (glint-bars,
glint-goal); Showcase's figure (its chest glow); with the cost per pass against main. The second
bounce shortfall on Glint's goal view (docs/design-gi-refill-priority.md's sibling item) is
measured separately.
