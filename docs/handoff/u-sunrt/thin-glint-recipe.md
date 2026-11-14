# Thin-line Glint gate: existing P200 candidate

Renderlab head **75c8370261b966ec0d502f800f7d9131e7e0d93c**, branch `t3/thin-line-crawl`, includes main `5ec6af43` (and `50fc872a`). Validated PC fixture, current-main 30-frame identity both paths, all-pass starts. Linux full lib: off / UNORM / CYCLE / both each302/0, each313/0 with backend-ara. Mac flag-on picking remains pending.

**Reuse** `D:/Coding/renderlab-work/ara-pin/bin-thin-161bc52c`; its wgpu coverage implementation is unchanged. Later changes merge main and align Apple picking. Binary ara.exe SHA256: `ad5a663334e7c813ea4ea95bf8e81c317a9aaa973bd81180d9f0ef5493aaeac6`. No new candidate build is requested.

Project: `D:/Coding/renderlab-work/ara-pr169/tree/samples/glint` (P200's build tree). Physical scene `content/levels/glint.scene` SHA256 `1cc06fd0aa857c303189c7c0982b8b160deff01a9c9ddb2ad5e6712732abb2f6`, byte-equal to that tree's `origin/main`; replay `replay/skyward.json` SHA256 `9ad874ff29df17609606f4bdfa5cbf14e0587e3d90ebb3b4b7c543f772afad44`. Keep these hashes with every table. Harness records scene/replay/binary hashes in each metric.json.

## Held start (ordinary GPU lease)

```powershell
$env:THIN_BIN='D:/Coding/renderlab-work/ara-pin/bin-thin-161bc52c'
$env:THIN_PROJECT='D:/Coding/renderlab-work/ara-pr169/tree/samples/glint'
$env:THIN_COMPANION='D:/Coding/renderlab-work/ara-pr169/tree/dotnet/artifacts/bin'
$env:THIN_OUT='D:/Coding/renderlab-work/u-sunrt/thin-crawl/p200-final'
py -3 -B D:/Coding/renderlab-work/u-sunrt/thin_capture.py baseline unorm cycle
```

Fresh paused player per row/path, seed 7, tick 4, 1920x1080 native, settle 400 then capture 8 frames. Metric: each pixel's maximum absolute Rec.709 luma change over 7 adjacent pairs; counts >=4 and >=16 output codes. Earlier P198 counts: NR 969/88, RT 509/12; re-measure candidate-off as control. Use a fresh THIN_OUT for each run and retain every image.

| Row | RENDERLAB_TR1_COVERAGE_UNORM | RENDERLAB_TR1_COVERAGE_CYCLE |
|---|---|---|
| baseline | unset | unset |
| unorm | 1 | unset |
| cycle | unset | 1 |

Harness clears both switches and unrelated pass/probe switches between rows, removes Vulkan layer overrides, sets ARA_BIN, ARA_DOTNET_ARTIFACTS, ARA_ALLOW_STALE_COMPANIONS=1, and RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK=I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK. Non-RT: ARA_RENDERER=renderlab-nonrt; RT: unset. Pass/CPU/RPC logs and all 8 raw PNGs retained.

## Moving / cut / reference crop

For each row above and each path, set ARA_BIN=$env:THIN_BIN, ARA_DOTNET_ARTIFACTS=$env:THIN_COMPANION, the same ACK, and ARA_VIDEO_KEEP to comma-separated 0..33. From `D:/Coding/renderlab-work/ara-pr169/tree`:

```powershell
& "$env:THIN_BIN/ara-samples.exe" glint-replay --captures '<unique-row-path-output>' --hold 34 --travel 0.2 --size 1920x1080
```

Canonical skyward replay; inspect start rails/rims in motion and each shot's cut/reveal. Compare each frame and frame-to-frame change against off: pixels >8 codes and maximum code difference. Inspect moving edges for trails. Compare held rail/rim crops before/after with the accepted ray-traced still; do not count smoothness gained by wrong mean coverage. Preserve all images and logs.

## Storage / limits

**Zero additional GPU bytes, targets or bindings.** History remains rgba32uint, sampled as u32. UNORM changes the existing coverage half to 16-bit fixed point; colour/weight halves remain fp16. Cycle uses the same word with a count-format tag in spare bit 25. TR1 and picking decode through shared helpers on both engines; third-surface retention copies bits. Cycle retains a startup prior, so it is a bounded statistic rather than 256 entirely fresh samples; no claim of an exact analytic integral.
