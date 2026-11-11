Ara backend on Metal (M5 Pro), head 977c4e8, 960x640. Summaries only (frame.json: per-pose
grids, cache table summaries, each frame's telemetry). 16 moving frames unless noted.
full:      --interactive --reconstruction --reflections --filter --atmosphere --frames 16
half:      --scale performance --interactive --reflections --filter --atmosphere --frames 16
twothirds: --scale quality --interactive --reflections --filter --atmosphere --frames 16
local:     --local-shadows --interactive --reconstruction --reflections --filter --atmosphere --frames 16
normal:    --normal-map --interactive --reconstruction --reflections --filter --atmosphere --frames 16
dof:       --traced --depth-of-field --atmosphere (one frame)
full is identical in every pixel and table digest to the 520b2bb summary (scratch/ara-recon-metal-520b2bb).
Without raw tables --compare against these ends NO VERDICT; the per-pose grid lines are the use.
