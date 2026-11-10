# Shader compile structure for P-PIPE

Status: CPU census tool implemented; measured tables and boundary recommendations are being assembled. This is a draft measurement PR, not a rendering change.

The reproducible CPU-only test calls production WGSL composers, selects production entry points and specialization constants, then validates and emits SPIR-V using Naga 30.0.1 with runtime checks both enabled and disabled. It creates no wgpu instance, adapter or device. Generated WGSL, SPIR-V and JSON stay in an ignored output directory, not this commit.

Base under measurement: main `ff19aef3cc5cc01d2048120bae7b44e32e35a5d6`, after `27d4f0b`. Small production refactors extract identical string composition into functions shared with the test; other accessors are test-only. Measurements describe a declared Vulkan reference profile, not captured NVIDIA driver input or GPU timings.

```bash
CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR="$PWD/.refs/shader-compile-structure/target" \
SHADER_STRUCTURE_OUT="$PWD/.refs/shader-compile-structure/out" \
SHADER_STRUCTURE_SPIRV_OPT=/path/to/spirv-opt \
cargo test --lib cpu_shader_compile_structure --locked -j 2 -- \
  --ignored --nocapture --test-threads=1
```

`SHADER_STRUCTURE_SPIRV_OPT` is optional. Strict optimizer validation errors are retained; a separately labelled skip-validation diagnostic measures size only. Remove the disposable target and raw dumps after summarizing them. The focused `cpu_shader_structure_analyzer` test verifies nested-loop and call-site counting without a device.

## Initial measured tables

Optimizer sizes and full inventories are still in progress; `—` below means not yet run in this raw census.

All values below are CPU census measurements in bytes; C/U = runtime checks on/off. `†` means strict SPIRV-Tools validation failed: size is the separate `--skip-validation` diagnostic, not a validated shipping candidate. Rows are entry-point binaries, not whole WGSL files.

| Path / module / entry | WGSL lines | SPV C / U | `-O` C / U | Instructions C / U | Functions C / U | Loops C / U | Depth C / U |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| `nonrt/program-list-soft-streamed/primary_shade_list` | 3345 | 232912 / 220404 | — / — | 13967 / 13226 | 192 / 189 | 44 / 44 | 4 / 4 |
| `nonrt/program-list-hard-streamed/primary_shade_list` | 3339 | 232548 / 220040 | — / — | 13945 / 13204 | 192 / 189 | 44 / 44 | 4 / 4 |
| `nonrt/program-soft-streamed/primary_shade_tiles` | 3293 | 231848 / 219336 | — / — | 13900 / 13159 | 191 / 188 | 44 / 44 | 4 / 4 |
| `nonrt/program-hard-streamed/primary_shade_tiles` | 3287 | 231484 / 218972 | — / — | 13878 / 13137 | 191 / 188 | 44 / 44 | 4 / 4 |
| `nonrt/program-list-soft-resident/primary_shade_list` | 3278 | 228836 / 216328 | — / — | 13713 / 12972 | 187 / 184 | 44 / 44 | 4 / 4 |
| `nonrt/program-list-hard-resident/primary_shade_list` | 3272 | 228472 / 215964 | — / — | 13691 / 12950 | 187 / 184 | 44 / 44 | 4 / 4 |
| `nonrt/program-soft-resident/primary_shade_tiles` | 3226 | 227756 / 215244 | — / — | 13645 / 12904 | 186 / 183 | 44 / 44 | 4 / 4 |
| `nonrt/program-hard-resident/primary_shade_tiles` | 3220 | 227392 / 214880 | — / — | 13623 / 12882 | 186 / 183 | 44 / 44 | 4 / 4 |
| `nonrt/program-film/primary_shade_tiles` | 3358 | 222536 / 210016 | — / — | 13350 / 12609 | 181 / 178 | 44 / 44 | 4 / 4 |
| `rt/program-list-hard-streamed/primary_shade_list` | 3353 | 216236 / 202652 | — / — | 13026 / 12232 | 187 / 184 | 43 / 43 | 3 / 3 |
| `rt/program-hard-streamed/primary_shade_tiles` | 3301 | 215172 / 201584 | — / — | 12959 / 12165 | 186 / 183 | 43 / 43 | 3 / 3 |
| `rt/program-list-hard-resident/primary_shade_list` | 3286 | 212160 / 198576 | — / — | 12772 / 11978 | 182 / 179 | 43 / 43 | 3 / 3 |

All values are CPU census measurements. Both sides use **checked** emission to isolate the user source. `=` means identical SHA-256, including when WGSL changes; Δ0 means different bytes at unchanged length. `removed` means production does not build that family for toon.

| Path / module / entry | Standard bytes | Toon bytes (Δ) | Glass-tint bytes (Δ) |
|---|---:|---:|---:|
| `nonrt/program-list-soft-streamed/primary_shade_list` | 232912 | 121128 (-111784) | = |
| `nonrt/program-list-hard-streamed/primary_shade_list` | 232548 | 120780 (-111768) | = |
| `nonrt/program-soft-streamed/primary_shade_tiles` | 231848 | 120208 (-111640) | = |
| `nonrt/program-hard-streamed/primary_shade_tiles` | 231484 | 119860 (-111624) | = |
| `nonrt/program-list-soft-resident/primary_shade_list` | 228836 | 118372 (-110464) | = |
| `nonrt/program-list-hard-resident/primary_shade_list` | 228472 | 118024 (-110448) | = |
| `nonrt/program-soft-resident/primary_shade_tiles` | 227756 | 117420 (-110336) | = |
| `nonrt/program-hard-resident/primary_shade_tiles` | 227392 | 117072 (-110320) | = |
| `nonrt/program-film/primary_shade_tiles` | 222536 | 108540 (-113996) | = |
| `rt/program-list-hard-streamed/primary_shade_list` | 216236 | 111060 (-105176) | = |
| `rt/program-hard-streamed/primary_shade_tiles` | 215172 | 110140 (-105032) | = |
| `rt/program-list-hard-resident/primary_shade_list` | 212160 | 108304 (-103856) | = |
| `rt/program-list-soft-streamed/primary_shade_list` | 211176 | 106892 (-104284) | = |
| `rt/program-hard-resident/primary_shade_tiles` | 211080 | 107368 (-103712) | = |
| `rt/program-soft-streamed/primary_shade_tiles` | 210112 | 105972 (-104140) | = |
| `rt/program-list-soft-resident/primary_shade_list` | 207100 | 104136 (-102964) | = |
| `rt/program-soft-resident/primary_shade_tiles` | 206020 | 103200 (-102820) | = |
| `rt/program-film/primary_shade_tiles` | 200748 | 94220 (-106528) | = |
| `shared/program-raster-map/fs_lit` | 177864 | 88288 (-89576) | = |
| `rt/program-raster-fresh/fs_lit` | 172560 | 82984 (-89576) | = |
| `rt/program-raster-hardware-4x/fs_lit` | 156884 | 67252 (-89632) | = |
| `rt/program-raster-hardware-1x/fs_lit` | 156864 | 67232 (-89632) | = |
| `rt/transparency-reflection/tp_reflect` | 143168 | 90576 (-52592) | = |
| `nonrt/transparency-glass/fs_transparent` | 139904 | 83156 (-56748) | 139980 (+76) |
| `rt/transparency-glass/fs_transparent` | 122048 | 65192 (-56856) | 122124 (+76) |
| `rt/gi-cell-mixed/gi_shade_cells` | 62156 | removed | = |
| `rt/gi-cell/gi_shade_cells` | 61796 | removed | = |
| `nonrt/gi-cell-mixed/gi_shade_cells` | 61600 | removed | = |
| `nonrt/gi-cell/gi_shade_cells` | 61240 | removed | = |
| `rt/transparency-glass/fs_tp_lobe` | 16368 | 14948 (-1420) | = |
| `nonrt/transparency-glass/fs_tp_lobe` | 15832 | 14412 (-1420) | = |
| `rt/program-raster-hardware-4x/vs_tangent` | 7384 | 7464 (+80) | = |
| `rt/program-raster-fresh/vs_tangent` | 7384 | 7480 (+96) | = |
| `rt/transparency-glass/vs_tangent` | 7380 | 7460 (+80) | = |
| `rt/program-raster-hardware-1x/vs_tangent` | 7364 | 7444 (+80) | = |
| `nonrt/transparency-glass/vs_tangent` | 6844 | 6916 (+72) | = |
| `shared/program-raster-map/vs_tangent` | 6828 | 6900 (+72) | = |
| `rt/program-raster-hardware-4x/vs` | 5796 | 5876 (+80) | = |
| `rt/program-raster-fresh/vs` | 5796 | 5892 (+96) | = |
| `rt/transparency-glass/vs` | 5792 | 5872 (+80) | = |
| `rt/program-raster-hardware-1x/vs` | 5776 | 5856 (+80) | = |
| `nonrt/transparency-glass/vs` | 5256 | 5328 (+72) | = |
| `shared/program-raster-map/vs` | 5240 | 5312 (+72) | = |

All other 154 baseline entry-point binaries are byte-identical for both edits; their exact identities are the census rows absent from this change table. No new entry family is introduced by either edit.

Measured checked, standard → toon: {'changed': 39, 'unchanged': 154, 'removed': 4}.

Measured checked, standard → glass_tint: {'unchanged': 195, 'changed': 2}.

Measured unchecked, standard → toon: {'changed': 39, 'unchanged': 154, 'removed': 4}.

Measured unchecked, standard → glass_tint: {'unchanged': 195, 'changed': 2}.

All counts below are also measurements. Default-policy byte totals sum only matched, changed binaries across enumerated conditional families; they are not a startup workload.

| User edit, actual default policy | Changed | Identical | Removed | Bytes before → after, matched rows |
|---|---:|---:|---:|---:|
| standard → toon | 39 | 154 | 4 | 4831456 → 2659036 |
| standard → glass_tint | 24 | 173 | 0 | 4588196 → 4879864 |
