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

## Archived picture result

RTX 4070 SUPER, Vulkan release build fbf3e3c5, procedural `game_scene` with Fox,
frames 0-29 on each path; material on, other optional passes off. The ordinary
picture lease exited 0 in 93.27 s, with no validation or panic text. Control off
was SHA-exact to the existing material-on reference on all 60 flight frames.
With control on, compared with material off, total pixel-frame counts over
2 / 8 display codes and the maximum were: flight RT 34 / 1 / 14;
flight non-RT 38 / 4 / 16; night RT 27 / 4 / 16; night non-RT 32 / 7 / 29.
The control does not eliminate every record difference. No Showcase share or
cost claim follows from these pictures. Material remains off; further mirror
work is held until material adds a measured gain on top of light visibility.
The raw counts and crop evidence remain in the unit's ignored measurement files.
