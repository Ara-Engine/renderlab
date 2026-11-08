"""CPU-only R0a floor tables and inspectable full-frame/difference sheets."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import struct

import numpy as np
from PIL import Image, ImageDraw

CONFIG_NAMES = {"sw-mesh-epic":"SW E", "hw-sc-epic":"HW-SC E", "hw-hit-epic":"HW-Hit E",
    "sw-mesh-cine":"SW C", "hw-sc-cine":"HW-SC C", "hw-hit-cine":"HW-Hit C",
    "ssr-off-informative":"SSR off", "ssr-on-informative":"SSR on",
    "renderlab-nonrt":"Renderlab nonRT", "renderlab-rt":"Renderlab RT"}


def controller():
    spec = importlib.util.spec_from_file_location("r0a", Path(__file__).with_name("m7-r0a.py"))
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def qa(args, r):
    r.cpu_note("create full-frame reflection reference/native/difference sheets"); r.no_quiet()
    codes = r.load("m7-u0b-score").codes
    out = r.OUT/"qa"/args.label
    previous = out/"inventory.json"
    inventory = [entry for entry in json.loads(previous.read_text())["images"]
        if entry["case"].split("-")[0] not in args.fixtures] if previous.exists() else []
    for fixture in args.fixtures:
        scene = json.loads((r.OUT/"scenes"/fixture/"scene.json").read_text())
        for pose in scene["poses"]:
            case = fixture+"-"+pose["name"]
            path = r.static_reference_path(args, fixture, case)
            if not path.exists():
                continue
            with np.load(path) as data:
                truth = (data["L0"]+data["L1"])/2
            images = [("reference", truth)]
            for config in args.configs:
                path = r.OUT/"capture"/args.label/fixture/pose["name"]/config/"native.npz"
                if path.exists():
                    with np.load(path) as data:
                        images.append((config, data["hdr"][..., :3]/data["pre_exposure"]))
            for backend in ("nonrt", "rt"):
                path = r.OUT/"current"/(args.current_label or args.label)/backend/(case+".npz")
                if (path.parent/(case+".unsupported.json")).exists():
                    images.append(("renderlab-"+backend+" — unsupported mixed programs", None))
                    continue
                if path.exists():
                    with np.load(path) as data:
                        images.append(("renderlab-"+backend, data["L"]))
            for exposure in (-14, -10):
                tiles = []; differences = []
                for name, value in images:
                    if value is None:
                        tile = Image.new("RGB", (480, 294), "#161a1f")
                        ImageDraw.Draw(tile).text((8, 6), name, fill="white")
                        ImageDraw.Draw(tile).text((8, 130), "No frame produced; explicit baseline miss", fill="white")
                        tiles.append(tile)
                        differences.append(tile.copy())
                        continue
                    display = np.clip(codes(value, exposure), 0, 255).astype("u1")
                    image = Image.fromarray(display)
                    full = out/case/(name+f"-ev{exposure}.png")
                    full.parent.mkdir(parents=True, exist_ok=True); image.save(full)
                    tile = Image.new("RGB", (480, 294), "#161a1f")
                    tile.paste(image.resize((480, 270)), (0, 24))
                    ImageDraw.Draw(tile).text((8, 6), name, fill="white")
                    tiles.append(tile)
                    inventory.append(dict(file=str(full), sha256=r.sha(full), case=case, exposure=exposure))
                    error = np.clip(4*np.abs(codes(value, exposure)-codes(truth, exposure)),0,255).astype("u1")
                    diff = Image.fromarray(error)
                    diff_full = full.with_name(full.stem+"-diff4.png"); diff.save(diff_full)
                    diff_tile = Image.new("RGB", (480,294), "#161a1f")
                    diff_tile.paste(diff.resize((480,270)),(0,24))
                    ImageDraw.Draw(diff_tile).text((8,6),"4x error: "+name,fill="white")
                    differences.append(diff_tile)
                    inventory.append(dict(file=str(diff_full),sha256=r.sha(diff_full),case=case,exposure=exposure))
                columns = min(4, len(tiles)); rows = (len(tiles)+columns-1)//columns
                sheet = Image.new("RGB", (columns*480, rows*294), "#161a1f")
                for i, tile in enumerate(tiles):
                    sheet.paste(tile, (480*(i%columns), 294*(i//columns)))
                sheet.save(out/(case+f"-ev{exposure}-overview.png"))
                for i,tile in enumerate(differences):
                    sheet.paste(tile,(480*(i%columns),294*(i//columns)))
                sheet.save(out/(case+f"-ev{exposure}-differences.png"))
    r.save(out/"inventory.json", dict(images=inventory, inspection="pending human/agent visual inspection; this file alone is not acceptance"))


def number(value, scale=1):
    return "—" if value is None else f"{value*scale:.3f}"


def quarry_histogram(args, r):
    """Read factors/counts only; do not decode or redistribute private geometry."""
    r.cpu_note("stream plain-quarry material/count histogram; no geometry export"); r.no_quiet()
    source = Path(json.loads(Path("E:/renderlab-work/m7-u0b/quarry.json").read_text())["asset"])
    with source.open("rb") as stream:
        def u32():
            return struct.unpack("<I", stream.read(4))[0]
        if stream.read(4)!=b"RLA\0":
            raise ValueError("asset magic")
        version = u32()
        if version not in range(1, 7):
            raise ValueError("unsupported asset version")
        meshes, placements, materials, textures = [u32() for _ in range(4)]
        primitive_counts = []
        for _ in range(meshes):
            counts = {}
            for _ in range(u32()):
                material, vertices, indices = [u32() for _ in range(3)]
                if indices%3:
                    raise ValueError("non-triangle primitive")
                tangents = 0
                if version>=2:
                    basis, tangents = u32(), u32()
                    if basis not in (0, 1, 2):
                        raise ValueError("tangent basis")
                stream.seek(48*vertices+16*tangents+4*indices, 1)
                counts[material] = counts.get(material, 0)+indices//3
            primitive_counts.append(counts)
        placed = {}
        for _ in range(placements):
            mesh = u32(); stream.seek(64, 1)
            for material, count in primitive_counts[mesh].items():
                placed[material] = placed.get(material, 0)+count
        def reference():
            present = u32()
            if present not in (0, 1):
                raise ValueError("texture reference flag")
            if present:
                index = u32(); stream.seek(20, 1)
                return index
            return None
        values = []
        for material in range(materials):
            base = struct.unpack("<4f", stream.read(16))
            metallic, roughness = struct.unpack("<2f", stream.read(8))
            sided = u32(); colour = reference(); mr = reference()
            if version>=2:
                normal = reference(); stream.seek(4, 1)
            else:
                normal = None
            values.append(dict(material=material, roughness=roughness, metallic=metallic,
                placed_triangles=placed.get(material, 0), base_colour_texture=colour,
                metallic_roughness_texture=mr, normal_texture=normal))
        if any(x["metallic_roughness_texture"] is not None for x in values):
            raise ValueError("plain-quarry histogram needs effective texture sampling, cannot report factors as final roughness")
    total = sum(placed.values())
    bins = {"B0 [0,.05)":0, "B1 [.05,.4)":0, "B2 [.4,1]":0}
    for material in values:
        rough = material["roughness"]
        key = "B0 [0,.05)" if rough<.05 else "B1 [.05,.4)" if rough<.4 else "B2 [.4,1]"
        bins[key] += material["placed_triangles"]
    r.save(r.OUT/"scores/plain-quarry-roughness.json", dict(asset_sha256=r.sha(source), version=version,
        materials=values, mesh_count=meshes, placement_count=placements, texture_count=textures,
        placed_triangles=total, histogram={key:dict(triangles=n, fraction=n/total) for key,n in bins.items()},
        weighting="placed full-detail triangles, not screen-visible pixels or surface area; no MR texture in this plain asset"))


def table(headers, rows):
    return "\n".join(["| "+" | ".join(headers)+" |", "|"+"|".join(["---"]*len(headers))+"|"]+
                     ["| "+" | ".join(map(str, row))+" |" for row in rows])+"\n"


def metric_row(prefix, m):
    if not m.get("pixels"):
        return [*prefix, "no frame" if m.get("status")=="unsupported" else "0 pixels", *(["—"]*7)]
    return [*prefix, number(m["rgb_relative_rms"], 100), number(m["luminance_relative_l1"], 100),
        number(m["signed_energy_bias"], 100), number(m["display_rms_-14"]), number(m["p99_absolute_display_error_-14"]),
        number(m["display_rms_-10"]), number(m["p99_absolute_display_error_-10"]), number(m["split_noise_relative_rms"], 100)]


def tables(args, r):
    r.cpu_note("write complete per-fixture, per-population static and motion floor tables"); r.no_quiet()
    static = json.loads((r.OUT/"scores"/(args.label+"-static.json")).read_text())
    motion = json.loads((r.OUT/"scores"/(args.label+"-motion.json")).read_text())
    if not static["complete"] or not motion["complete"]:
        raise ValueError("floor coverage incomplete")
    expected_static = {(fixture, pose["name"], config)
        for fixture in args.fixtures
        for pose in json.loads((r.OUT/"scenes"/fixture/"scene.json").read_text())["poses"]
        for config in [*args.configs, "renderlab-nonrt", "renderlab-rt"]}
    if {(x["fixture"], x["pose"], x["config"]) for x in static["rows"]}!=expected_static:
        raise ValueError("static table does not contain the requested complete matrix")
    expected_motion = {(fixture, config) for fixture in ("RF3", "RF5")
        for config in ["sw-mesh-epic", "hw-sc-epic", "hw-hit-epic", "renderlab-nonrt", "renderlab-rt"]}
    if {(x["fixture"], x["config"]) for x in motion["rows"]}!=expected_motion or any(len(x["frames"])!=13 for x in motion["rows"]):
        raise ValueError("motion table does not contain every mode and witness")
    pages = ["# M7-R0a numerical floors\n\nCompanion to [m7-r0a.md](m7-r0a.md). No gain fit or pixel rejection. "
             "E/C mean Epic/Cinematic; SW means software mesh-SDF; SC means hardware surface-cache lighting. "
             "SSR rows are informative only. Renderlab has environment-only reflections.\n\n"
             "Errors R and L are RGB relative RMS and luminance relative L1 (%); ΔE is signed energy bias (%). "
             "D/P are display RMS/p99 maximum-channel absolute error in 8-bit F0 codes at the stated exposure. "
             "N is the reference split-half RMS (%), not a candidate error allowance. "
             "Every JSON population also records split display noise at both exposures.\n"]
    headers = ["Fixture / pose", "Mode", "R %", "L %", "ΔE %", "D −14", "P −14", "D −10", "P −10", "N %"]
    pages.append("## Complete reflector populations\n")
    pages.append(table(headers, [metric_row([x["fixture"]+" / "+x["pose"], CONFIG_NAMES[x["config"]]], x["measures"].get("reflector", {"pixels":0,"status":x.get("status")})) for x in static["rows"]]))
    pages.append("## Roughness, off-screen and feature populations\n\n"
        "Exact roughness rows are retained so future tier cutoffs do not require a rerender. "
        "The convenience bands are B0 [0,.05), B1 [.05,.4), B2 [.4,1]. "
        "Reflected-feature populations are ideal centre-reflection guides; the final image is scored unchanged.\n")
    rows = []
    for item in static["rows"]:
        for population, value in item["measures"].items():
            if population=="reflector" or population in ("image", "ground", "direct-objects", "occluder", "toon"):
                continue
            rows.append(metric_row([item["fixture"]+" / "+item["pose"]+" / "+population, CONFIG_NAMES[item["config"]]], value))
    pages.append(table(headers, rows))
    pages.append("## Reflected feature position, energy and blur\n\n"
        "E/Eref is the full RGB energy inside the reference footprint. C/Cref is target-colour contrast energy "
        "over the entire reflector. Position is the colour centroid's Euclidean error in pixels. "
        "Blur is candidate minus reference 10–90% edge width; missing edges remain misses. "
        "A dash means the definition is inapplicable or the candidate feature is missing, never zero.\n")
    rows = []
    for item in static["rows"]:
        for feature, value in item["features"].items():
            edge = value.get("edge_spread", {})
            rows.append([item["fixture"]+" / "+item["pose"]+" / "+feature, CONFIG_NAMES[item["config"]],
                value["guide_pixels"], number(value["footprint_energy_ratio"]), number(value.get("chromatic_energy_ratio")),
                number(value.get("centroid_error_pixels")), number(edge.get("median_extra_pixels")),
                number(edge.get("p95_extra_pixels")), str(edge.get("candidate_missing_edges", "—"))+" / "+str(edge.get("eligible_reference_edges", "—"))])
    pages.append(table(["Fixture / feature", "Mode", "Pixels", "E/Eref", "C/Cref", "Position px", "Median blur px", "p95 blur px", "Missing / edges"], rows))
    pages.append("## Motion and settling\n\n"
        "The motion starts at frame 32 and ends at frame 79; frame 80 is the first settled frame. "
        "Ages 1/2/4/8 are frames 32/33/35/39 and 80/81/83/87. Each line scores the complete reflector. "
        "All exact-roughness, off-screen and feature populations remain in the consolidated motion JSON. "
        "Renderlab rows are independent warmed state snapshots; only Unreal rows measure continuous history.\n")
    rows = []
    for item in motion["rows"]:
        by_frame = {x["frame"]:x for x in item["frames"]}
        for phase, ages in item["ages"].items():
            for age, frame in ages.items():
                rows.append(metric_row([item["fixture"]+" / "+phase+" / age "+age+" (f"+str(frame)+")", CONFIG_NAMES[item["config"]]], by_frame[frame]["measures"]["reflector"]))
    pages.append(table(headers, rows))
    pages.append("## Adjacent-frame residuals and visible trails\n\n"
        "Residuals are frame-to-frame changes of candidate-minus-reference display error. "
        "Trails require excess target-colour contrast above two display codes in an old-only reflection footprint. "
        "Their extent is Chebyshev distance from the current footprint; >32 is a lower bound. "
        "These values do not replace the complete-image or missing-feature errors.\n")
    rows = []
    for item in motion["rows"]:
        for pair in item["adjacent_motion_residuals"]:
            residual = pair["display_residual"]
            trails = pair["visible_trails"]
            trail = lambda stop: max((x["extent_pixels"] for k, x in trails.items() if k.endswith("@"+str(stop))), default=None)
            extent = lambda value: "—" if value is None else (">32" if value>32 else str(value))
            rows.append([item["fixture"]+" / "+str(pair["pair"]), CONFIG_NAMES[item["config"]],
                number(residual["-14"]["rgb_rms"]), number(residual["-14"]["p99"]), extent(trail(-14)),
                number(residual["-10"]["rgb_rms"]), number(residual["-10"]["p99"]), extent(trail(-10))])
    pages.append(table(["Fixture / frames", "Mode", "Residual −14", "p99 −14", "Trail px −14", "Residual −10", "p99 −10", "Trail px −10"], rows))
    shutters_data = json.loads((r.OUT/"scores/shutter-summary.json").read_text())
    if not shutters_data["complete"]:
        raise ValueError("finite-shutter summary incomplete")
    pages.append("## Motion-integrated reference convergence\n\n"
        "Centred 180-degree shutter, 8/16 midpoint nodes, 256 samples per node. "
        "The 8-minus-16 difference includes Monte Carlo noise. N16 is actual-half display noise, "
        "preserving shared-seed covariance between time nodes. These do not score zero-shutter native images.\n")
    pages.append(table(["Fixture / frame", "8 vs 16 D −14", "N16 −14", "8 vs 16 D −10", "N16 −10", "Centre vs 16 D −14"],
        [[x["fixture"]+" / "+str(x["frame"]), number(x["quadrature_plus_mc_difference"]["display_rms_-14"]),
          number(x["quadrature_plus_mc_difference"]["reference_split_display_noise_rms_-14"]),
          number(x["quadrature_plus_mc_difference"]["display_rms_-10"]),
          number(x["quadrature_plus_mc_difference"]["reference_split_display_noise_rms_-10"]),
          number(x["centre_vs_integrated"]["display_rms_-14"])] for x in shutters_data["rows"]]))
    telemetry_data = json.loads((r.OUT/"scores/native-telemetry.json").read_text())
    if not telemetry_data["complete"]:
        raise ValueError("native telemetry incomplete")
    pages.append("## Functional telemetry, not a performance claim\n\n"
        "CSV median/p95 over 64 frames with async at its default; separate single instrumented "
        "ProfileGPU frame with async off. Foreign compilers were active. ProfileGPU parents overlap; "
        "do not sum them. Memory is the CSV transient resource allocation and ray-tracing geometry, "
        "not total process/VRAM usage. All Lumen component allocations remain in native-telemetry.json.\n")
    rows = []
    for item in telemetry_data["rows"]:
        csv = item["csv"]; profile = item["profile_gpu_async_off"]
        metric = lambda key, stat="median": number((csv.get(key) or {}).get(stat))
        rows.append([item["fixture"]+" / "+item["pose"],CONFIG_NAMES[item["config"]],
            metric("GPUTime"),metric("GPUTime","p95"),metric("GPU/LumenReflections"),
            number(profile["reflections"]["ms"]),number(profile["gi"]["ms"]),number(profile["tsr"]["ms"]),
            metric("TransientMemoryUsedMB"),number(item["ray_tracing_geometry_mb"])])
    pages.append(table(["Fixture / pose","Mode","GPU p50 ms","GPU p95 ms","CSV reflections ms","Profile reflections ms","Profile GI ms","Profile TSR ms","Transient MB","RT geometry MB"],rows))
    target = r.REPO/"docs/m7-r0a-floors.md"
    target.write_text("\n".join(pages), encoding="utf-8")
    r.save(r.OUT/"scores/table-provenance.json", dict(static_sha256=r.sha(r.OUT/"scores"/(args.label+"-static.json")),
        motion_sha256=r.sha(r.OUT/"scores"/(args.label+"-motion.json")), output_sha256=r.sha(target)))


def shutters(args, r):
    r.cpu_note("summarize and display 8/16-node motion-integrated references"); r.no_quiet()
    codes = r.load("m7-u0b-score").codes
    rows = []
    for fixture in ("RF3", "RF5"):
        for frame in (48, 64):
            paths = {nodes:r.OUT/"motion/reference"/f"{fixture}-f{frame:03d}-sh{nodes}.npz" for nodes in (8, 16)}
            values = {}
            for nodes, path in paths.items():
                with np.load(path) as data:
                    values[nodes] = (data["L0"], data["L1"])
            case = f"{fixture}-f{frame:03d}-zero"
            centre_path = r.OUT/"reference"/args.reference_label/case/str(args.spp*2)/(case+"-centre.npz")
            with np.load(centre_path) as data:
                centre = (data["L0"]+data["L1"])/2
                mask = data["pop_reflector"]
            image8 = sum(values[8])/2; image16 = sum(values[16])/2
            row = dict(fixture=fixture, frame=frame,
                quadrature_plus_mc_difference=r.measures(image8, *values[16], mask),
                centre_vs_integrated=r.measures(centre, *values[16], mask),
                archives={str(nodes):dict(file=str(path), sha256=r.sha(path)) for nodes,path in paths.items()},
                definition="8/16 midpoint nodes over centred 180-degree shutter; actual halves preserve covariance between shared-seed time nodes; differences include Monte Carlo noise")
            for exposure in (-14, -10):
                displays = [codes(centre, exposure), codes(image8, exposure), codes(image16, exposure),
                    np.minimum(255, 4*np.abs(codes(image8, exposure)-codes(image16, exposure)))]
                sheet = Image.new("RGB", (1920, 294), "#161a1f")
                for index, (name, display) in enumerate(zip(("zero shutter", "8 time nodes", "16 time nodes", "4x |8 - 16|"), displays)):
                    tile = Image.fromarray(np.clip(display, 0, 255).astype("u1")).resize((480, 270))
                    sheet.paste(tile, (480*index, 24)); ImageDraw.Draw(sheet).text((480*index+8, 6), name, fill="white")
                output = r.OUT/"qa"/args.label/f"{fixture}-f{frame:03d}-shutters-ev{exposure}.png"
                output.parent.mkdir(parents=True, exist_ok=True); sheet.save(output)
            rows.append(row)
    r.save(r.OUT/"scores/shutter-summary.json", dict(rows=rows, complete=len(rows)==4,
        candidate_comparison="Native and current renderer captures are zero-shutter; finite-shutter references do not set a mismatched native floor"))


def motion_qa(args, r):
    r.cpu_note("create motion witness and difference contact sheets from saved arrays"); r.no_quiet()
    codes = r.load("m7-u0b-score").codes
    inventory = []
    for fixture in ("RF3", "RF5"):
        for config in ("sw-mesh-epic", "hw-sc-epic", "hw-hit-epic", "renderlab-nonrt", "renderlab-rt"):
            current = config.startswith("renderlab-")
            root = r.OUT/"current"/((args.current_label or args.label)+"-motion")/config.removeprefix("renderlab-") if current else r.OUT/"motion/capture"/args.label/fixture/config
            native = {}
            if not current:
                for first in (31, 48, 64, 79, 95):
                    frames = json.loads((root/f"native-{first:03d}.json").read_text())["frames"]
                    native.update({first+i:Path(value["archive"]) for i,value in enumerate(frames)})
            flipbooks = {ev:[] for ev in (-14,-10)}
            for phase, witnesses in (("start", (31,32,33,35,39)), ("moving", (48,64,79)), ("stop", (80,81,83,87,95))):
                sheets = {ev:Image.new("RGB", (1440,294*len(witnesses)), "#161a1f") for ev in (-14,-10)}
                for row,frame in enumerate(witnesses):
                    case = f"{fixture}-f{frame:03d}-zero"
                    with np.load(r.OUT/"reference"/args.reference_label/case/str(args.spp*2)/(case+"-centre.npz")) as data:
                        truth = (data["L0"]+data["L1"])/2
                    path = root/(case+"-centre.npz") if current else native[frame]
                    with np.load(path) as data:
                        image = data["L"] if current else data["hdr"][...,:3]/data["pre_exposure"]
                    for ev,sheet in sheets.items():
                        a,b = codes(truth,ev),codes(image,ev)
                        for col,(name,value) in enumerate((("reference",a),(config,b),("4x absolute error",np.minimum(255,4*np.abs(a-b))))):
                            sheet.paste(Image.fromarray(np.clip(value,0,255).astype("u1")).resize((480,270)), (480*col,294*row+24))
                            ImageDraw.Draw(sheet).text((480*col+8,294*row+6),f"f{frame:03d} {name}",fill="white")
                        flipbooks[ev].append((frame,sheet.crop((0,294*row,1440,294*(row+1))).resize((960,196))))
                for ev,sheet in sheets.items():
                    output = r.OUT/"qa"/args.label/f"{fixture}-{config}-{phase}-ev{ev}.png"
                    output.parent.mkdir(parents=True,exist_ok=True); sheet.save(output)
                    inventory.append(dict(file=str(output),sha256=r.sha(output)))
            for ev,frames in flipbooks.items():
                durations = [round(1000*(frames[i+1][0]-frame)/60) if i+1<len(frames) else 500
                    for i,(frame,_) in enumerate(frames)]
                output = r.OUT/"qa"/args.label/f"{fixture}-{config}-witnesses-ev{ev}.webp"
                frames[0][1].save(output,save_all=True,append_images=[image for _,image in frames[1:]],
                    duration=durations,loop=0,lossless=True)
                inventory.append(dict(file=str(output),sha256=r.sha(output),frames=[frame for frame,_ in frames],durations_ms=durations))
    r.save(r.OUT/"qa"/args.label/"motion-inventory.json",dict(images=inventory,
        scope="13 registered witnesses per fixture; continuous Unreal runs and independent warmed Renderlab snapshots"))


def telemetry(args, r):
    r.cpu_note("summarize native CSV, instrumented ProfileGPU and component memory"); r.no_quiet()
    rows = []
    for fixture in args.fixtures:
        scene = json.loads((r.OUT/"scenes"/fixture/"scene.json").read_text())
        for pose in scene["poses"]:
            for config in args.configs:
                root = r.OUT/"capture"/args.label/fixture/pose["name"]/config
                summary = json.loads((root/"summary.json").read_text())
                log = (root/"ue.log").read_text(encoding="utf-8-sig",errors="replace")
                memory = {}
                for key in ("CardBuffer", "MeshCardsBuffer", "PageTable", "CardPages", "SceneInstanceIndexToMeshCardsIndexBuffer"):
                    values = re.findall(r"\s"+key+r": ([0-9.]+)Mb",log)
                    memory[key] = float(values[-1]) if values else None
                rt = re.findall(r"D3D12RayTracing: Total size: ([0-9.]+) MB",log)
                rows.append(dict(fixture=fixture,pose=pose["name"],config=config,
                    csv={key:summary["statistics"].get(key) for key in ("GPUTime","GPU/LumenReflections","GPU/LumenSceneUpdate","GPU/LumenSceneLighting","GPU/LumenScreenProbeGather","GPU/TemporalSuperResolution","TransientMemoryUsedMB","TransientMemoryAliasedMB")},
                    profile_gpu_async_off=summary["pass_costs_async_off"],
                    component_memory_mb=memory, ray_tracing_geometry_mb=float(rt[-1]) if rt else None,
                    summary_sha256=r.sha(root/"summary.json"),native_log_sha256=r.sha(root/"ue.log"),
                    timing_eligible_for_claim=False))
    r.save(r.OUT/"scores/native-telemetry.json",dict(rows=rows,complete=len(rows)==104,
        memory_scope="Lumen component buffer allocations, ray-tracing geometry and CSV transient resources; overlapping pools are not summed; these are not total process/VRAM allocations",
        timing_scope="functional runs under foreign compiler load; 64 CSV frames, one instrumented async-off ProfileGPU frame; no comparative performance claim"))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("command", choices=("qa", "tables", "quarry-histogram", "shutters", "motion-qa", "telemetry"))
    p.add_argument("--label", default="b")
    p.add_argument("--reference-label", default="d")
    p.add_argument("--current-label")
    p.add_argument("--spp", type=int, default=2048)
    p.add_argument("--fixtures", nargs="+", default=[f"RF{i}" for i in range(1, 11)])
    p.add_argument("--configs", nargs="+", default=["sw-mesh-epic", "hw-sc-epic", "hw-hit-epic", "sw-mesh-cine", "hw-sc-cine", "hw-hit-cine", "ssr-off-informative", "ssr-on-informative"])
    args = p.parse_args(); {"qa":qa, "tables":tables, "quarry-histogram":quarry_histogram,
        "shutters":shutters, "motion-qa":motion_qa, "telemetry":telemetry}[args.command](args, controller())


if __name__=="__main__":
    main()
