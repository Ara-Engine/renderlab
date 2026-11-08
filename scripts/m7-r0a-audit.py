"""Audit retained R0a identities and coverage; never launches a GPU process."""
import importlib.util
import json
from pathlib import Path


def main():
    spec = importlib.util.spec_from_file_location("r0a", Path(__file__).with_name("m7-r0a.py"))
    r = importlib.util.module_from_spec(spec); spec.loader.exec_module(r)
    r.cpu_note("audit final reference/capture/score identities and source hashes"); r.no_quiet()
    read = lambda path: json.loads(path.read_text())
    out = r.OUT
    identity = out/"cook/c/identity.json"; package = read(identity)
    for name,digest in package["content"].items():
        assert r.sha(out/"ue-package-c"/name)==digest,name
    static = read(out/"scores/c-static.json"); motion = read(out/"scores/c-motion.json")
    assert static["complete"] and len(static["rows"])==130
    assert motion["complete"] and len(motion["rows"])==10
    manifest = read(out/"reference/final/manifest.json")
    assert manifest["complete"] and len(manifest["cases"])==13
    equivalences = {fixture:read(out/"motion"/(fixture+"-static-equivalence.json")) for fixture in ("RF3","RF5")}
    for row in static["rows"]:
        case = row["fixture"]+"-"+row["pose"]
        if row.get("status")=="unsupported":
            assert row["fixture"]=="RF10" and row["config"].startswith("renderlab-")
            continue
        assert row["reference_archive_sha256"]==manifest["cases"][case]["archive_sha256"]
        if row["scene_sha256"]!=r.sha(out/"scenes"/row["fixture"]/"scene.json"):
            proof = equivalences[row["fixture"]]
            assert proof["static_inputs_equal"] and row["scene_sha256"] in proof["equivalent_source_sha256"]
    modes = {}; capture_proofs = []; motion_proofs = []
    for path in sorted((out/"capture/c").rglob("complete.json")):
        root = path.parent; complete = read(path); settings = read(root/"readback.json")
        process = read(root/"process.json")
        assert complete["package_identity_sha256"]==r.sha(identity),str(root)
        assert settings["all_ok"] and settings["effective_mode_ok"],str(root)
        assert process["exit_code"]==0 and not process["failures"],str(root)
        assert Path(process["command"][0]).resolve()==(out/"ue-package-c/Windows/Engine/Binaries/Win64/UnrealGame.exe").resolve()
        assert read(root/"scene-readback.json")["all_ok"]
        modes[root.name] = settings["effective_mode"]
        capture_proofs.append(dict(case=str(root.relative_to(out)),settings_sha256=r.sha(root/"readback.json"),
            process_sha256=r.sha(root/"process.json"),decoder_sha256=r.sha(root/"native.json")))
    assert len(capture_proofs)==104
    for path in sorted((out/"motion/capture/c").rglob("complete.json")):
        root = path.parent; complete = read(path); process = read(root/"process.json")
        assert all(complete[key] for key in ("camera_ok","settings_ok","motion_settings_qualified"))
        fixture = root.parent.name
        assert read(root/"recipe.json")["scene_sha256"]==r.sha(out/"scenes"/fixture/"scene.json")
        assert process["exit_code"]==0 and not process["failures"]
        assert Path(process["command"][0]).resolve()==(out/"ue-package-c/Windows/Engine/Binaries/Win64/UnrealGame.exe").resolve()
        transforms = read(root/"motion-readback.json")
        assert transforms["all_ok"] and len(transforms["frames"])==96
        assert sum(len(read(root/f"native-{frame:03d}.json")["frames"]) for frame in (31,48,64,79,95))==21
        motion_proofs.append(dict(case=str(root.relative_to(out)),process_sha256=r.sha(root/"process.json"),
            settings_sha256=r.sha(root/"readback.json"),transforms_sha256=r.sha(root/"motion-readback.json")))
    assert len(motion_proofs)==6
    qualifier = r.load("m7-u0b-score").qualified_replay
    references = []
    for case,value in manifest["cases"].items():
        path = out/"reference/final"/value["archive"]
        assert r.sha(path)==value["archive_sha256"]
        assert qualifier(read(path.with_suffix(".json")))
        references.append(dict(case=case,samples=value["samples"],sha256=value["archive_sha256"]))
    states = read(out/"motion/manifest.json")
    for fixture,registration in states.items():
        for state in registration["states"]:
            spp = 4096 if state["shutter"]=="zero" else 256
            name = state["name"]+"-centre"
            root = out/"reference/m-ordered"/state["name"]/str(spp)
            assert qualifier(read(root/(name+".json"))),state["name"]
            if spp==4096:
                assert (root/(name+".npz")).exists()
        for frame in (48,64):
            for nodes in (8,16):
                path = out/"motion/reference"/f"{fixture}-f{frame:03d}-sh{nodes}.npz"
                members = read(path.with_suffix(".json"))["members"]
                assert path.exists() and len(members)==nodes
                for member in members:
                    archive = Path(member["archive"])
                    assert read(archive.with_suffix(".archive.json"))["sha256"]==member["sha256"]
    checks = read(out/"checks.json")
    assert checks["cargo_exit"]==checks["fmt_exit"]==0
    assert all(r.sha(r.REPO/name)==digest for name,digest in checks["rust_sources"].items())
    compatibility = read(out/"compatibility/a/comparison.json")
    assert len(compatibility["files"])==60 and all(x["identical"] for x in compatibility["files"])
    controls = []
    for name in ("oracle/XRF7/16384/XRF7-between-panels-crosscheck.json",
                 "oracle-ordered/XRF10GW/4096/XRF10GW-pitch30-crosscheck.json",
                 "oracle-mirror-hi/XRF10MW/16384/XRF10MW-pitch30-crosscheck.json",
                 "oracle-guided-hi/XRF10MW/16384/XRF10MW-pitch30-crosscheck.json"):
        path = out/"reference"/name; value = read(path); assert value["pass"]
        controls.append(dict(file=str(path.relative_to(out)),sha256=r.sha(path),result=value))
    result = dict(base="79719917c0fba4d159ec81a3d466b33f6937abed",package_identity_sha256=r.sha(identity),
        hardware=dict(gpu="NVIDIA GeForce RTX 4070 SUPER",driver="617.14",cpu="AMD Ryzen 7 7800X3D",ram_gib=31.116),
        static_rows=130,static_supported_images=128,unsupported_rows=2,
        motion_workloads=10,motion_scored_witnesses=130,native_motion_readbacks=126,
        settings=modes,capture_proofs=capture_proofs,motion_proofs=motion_proofs,references=references,
        independent_controls=controls,checks=dict(cargo_exit=0,fmt_exit=0,library_passed=134,library_ignored=130,
            seconds=checks["seconds"],binary_sha256=checks["binary_sha256"],reference_binary_sha256=r.sha(r.REFERENCE_BINARY),
            default_reference_identical_arrays=60),
        static_track_equivalences=equivalences,
        artifacts={name:r.sha(out/name) for name in ("scores/c-static.json","scores/c-motion.json","scores/shutter-summary.json","scores/native-telemetry.json","reference/final/manifest.json")},
        limits=["RF10 HDR floor remains noise-limited at 16384 spp; close rankings are not certified",
                "current Renderlab RF10 produces no image on either path",
                "current motion rows are independent warmed state snapshots, not continuous history",
                "RF8 is procedural rock, not a private-quarry acceptance run",
                "memory telemetry covers components and transient pools, not total VRAM",
                "functional timings under compiler load; no performance claim"])
    target = r.REPO/"docs/m7-r0a-evidence.json"; r.save(target,result)
    print(json.dumps(dict(output=str(target),static_rows=130,motion_workloads=10,identities_ok=True)))


if __name__=="__main__":
    main()
