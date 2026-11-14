# Near-mirror reader control

`RENDERLAB_MATERIAL_READER_CONTROL=1` is a default-off wgpu diagnostic used with
`RENDERLAB_MATERIAL_PASS=1`. Mapped pixels at roughness at most 3277/65535
(the upward-rounded record code containing 0.05) evaluate their original `surface`
from the full decoded `SurfaceInput`. The same surface feeds shading and the
reflection lobe; `material_out` supplies its normal to the active filter cache,
and the control recomputes its original TR1 hint. Other pixels use the record.
Unmapped programs retain their existing shading. No shared record or WGSL is edited.

The full hit/attribute decode remains available in the current material reader;
this control preserves and uses it rather than inventing reduced inputs. Its
purpose is to test the mirror-edge differences measured by #98 (maximum display
deltas 39/34/95/91 on flight/night, RT/non-RT). It is not a default flip or a new
roughness floor. GPU picture and cost results must establish whether it is useful.

`RENDERLAB_MATERIAL_READER_STATS=32` counts once at renderer frame 32 and
prints an asynchronous `material_reader_stats` line: low-roughness mapped
pixels, all mapped pixels, hits and total frame pixels, plus both pixel shares.
It uses the hit image to reject stale miss records, then the record's model and
roughness codes. Run more than 32 frames and allow normal device polling to
complete the readback. This diagnostic allocates a separate compute pass and
must be disabled for performance measurements. It lets an integration capture
measure Showcase's share without changing its application code or saving bulk
receiver images.

The focused naga/layout test covers the ordinary writer, ordinary reader and
controlled reader on both paths, tiles/lists and resident/streamed stages, for
standard and game mappings. Timing must compare whole interleaved pairs and
report the complete frame, including the material writer and local-light pass.
