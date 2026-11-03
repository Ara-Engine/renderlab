"""Small CPU behavior checks for scan quality tools. All temporary bytes stay on D:."""
import importlib.util
import io
import json
from contextlib import contextmanager
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

import numpy as np
from PIL import Image


HERE = Path(__file__).parent


def module(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"scan-quality-{name}.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


variants = module("variants")
compare = module("compare")


@contextmanager
def temporary():
    scratch = Path("D:/Coding/renderlab-quality-tests").resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch) as temp:
        if Path(temp).resolve().parent != scratch:
            raise ValueError("temporary cleanup target escaped the test directory")
        yield temp


class QualityTests(unittest.TestCase):
    def test_linear_mips_retain_energy_and_nested_resolution_control(self):
        pixels = np.full((4, 4, 4), 255, dtype=np.uint8)
        pixels[::2, :, :3] = 0
        half = variants.half_srgb(pixels)
        self.assertTrue(np.all(half[:, :, :3] == 188))
        self.assertTrue(np.all(half[:, :, 3] == 255))
        with temporary() as temp:
            source = Path(temp) / "source.png"
            Image.fromarray(pixels).save(source)
            two, four = io.BytesIO(), io.BytesIO()
            variants.write_texture(two, source, 2)
            variants.write_texture(four, source, 4)
            self.assertEqual(two.getvalue()[8:], four.getvalue()[8 + 12 + 64:])

    def test_resource_splice_preserves_geometry_and_page_identity(self):
        with temporary() as temp:
            root = Path(temp)
            cache = root / "canonical/virtual-v1"
            (cache / "packages").mkdir(parents=True)
            (cache / "pages").mkdir()
            header = b"RLA\0\1\0\0\0" + struct.pack("<IIII", 1, 1, 1, 1)
            mesh = struct.pack("<IIII", 1, 0, 3, 3) + bytes(3 * 48) + struct.pack("<III", 0, 1, 2)
            placement = struct.pack("<I16f", 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1)
            material = struct.pack("<6f9I", 1, 1, 1, 1, 0, 0.85, 1, 1, 0, 2, 2, 1, 1, 2, 0)
            texture = struct.pack("<IIIII", 0, 1, 1, 1, 4) + bytes([100, 120, 140, 255])
            asset = root / "canonical.rla"
            asset.write_bytes(header + mesh + placement + material + texture)
            records = variants.layout(asset)
            resource = b"RLA\0\1\0\0\0" + struct.pack("<IIII", 0, 0, 1, 1) + material + texture
            page_hash = bytes.fromhex("ab" * 32)
            tail = b"opaque geometry metadata" + struct.pack("<III", 3, 3, 128) + page_hash
            package = cache / "packages/original"
            package.write_bytes(b"RLV\0\1\0\0\0" + struct.pack("<IIII", 1, 1, 1, len(resource)) + resource + tail)
            page = cache / "pages" / page_hash.hex()
            page.write_bytes(bytes(65536))
            manifest = root / "original.manifest"
            raw = b"RVM\0\1\0\0\0" + bytes(64) + struct.pack("<Q", package.stat().st_size)
            manifest.write_bytes(raw)
            out = root / "variant"
            out.mkdir()
            plan = {"output": str(out), "canonical_cache": str(root / "canonical"),
                    "full_bytes": asset.stat().st_size, "resource_bytes": len(resource),
                    "package_bytes": package.stat().st_size}
            virtual = raw, package, package.read_bytes()[:24], len(resource), 1
            variants.create_variant(plan, asset, records, {}, virtual)
            self.assertEqual((out / "full.rla").read_bytes(), asset.read_bytes())
            rewritten = out / "cache/virtual-v1/packages" / plan["proof"]["package_sha256"]
            self.assertEqual(rewritten.read_bytes(), package.read_bytes())
            linked = out / "cache/virtual-v1/pages" / page_hash.hex()
            self.assertTrue(page.samefile(linked))
            asset.write_bytes(asset.read_bytes()[:-1])
            with self.assertRaisesRegex(ValueError, "span exceeds"):
                variants.layout(asset)

    def test_edge_gate_catches_a_hole_separately_from_smooth_error(self):
        reference = np.ones((32, 32, 3), dtype=np.float32) * 0.3
        coverage = np.zeros((32, 32), dtype=np.float32)
        coverage[4:28, 4:28] = 1
        masks = compare.masks(reference, coverage)
        self.assertTrue(masks["edge"].any())
        self.assertTrue(masks["smooth_interior"][16, 16])
        residual = np.zeros_like(reference)
        residual[15:17, 15:17] = 0.3
        values = compare.statistics(residual, masks)
        self.assertEqual(values["edge"]["mae"], 0)
        self.assertGreater(values["smooth_interior"]["mae"], 0)

    def test_compare_cli_passes_exact_images_and_rejects_a_missing_region(self):
        with temporary() as temp:
            root = Path(temp)
            (root / "asset.rla").write_bytes(b"fixture identity")
            poses = [{"frame": i, "phase": "synthetic", "width": 32, "height": 32} for i in range(12)]
            (root / "cameras.json").write_text(json.dumps({"frames": poses}))
            reference_poses = [dict(p, width=64, height=64) for p in poses]
            (root / "reference-cameras.json").write_text(json.dumps({"frames": reference_poses}))
            plan = {"canonical_asset": str(root / "asset.rla"), "output": str(root),
                    "canonical_payload_sha256": variants.digest(root / "asset.rla"), "motion_pairs": [[2, 3]],
                    "acceptance": {"background_rgb8": [36, 44, 56], "edge_mae_max": .04,
                                   "edge_p95_max": .15, "smooth_mae_max": .02,
                                   "coverage_mismatch_fraction_max": .0002, "temporal_edge_mae_max": .04,
                                   "temporal_texture_mae_max": .025, "edges_improvement_min": .2}}
            (root / "plan.json").write_text(json.dumps(plan))
            for label in ("baseline", "candidate", "reference"):
                folder = root / label
                folder.mkdir()
                high = label == "reference"
                camera = root / ("reference-cameras.json" if high else "cameras.json")
                args = ["test.exe", "--mode", "full", "--asset", str(root / "asset.rla"),
                        "--cameras", str(camera), "--frames", "12", "--samples", "4"]
                (folder / "run.txt").write_text("test\nargs=" + json.dumps(args) + "\n")
                image = np.full((32, 32, 3), [36, 44, 56], dtype=np.uint8)
                image[4:28, 4:28] = np.tile([[100, 100, 100], [190, 190, 190]], (24, 12, 1))
                image[4:28, 4:16] = [150, 150, 150]
                if high:
                    image = image.repeat(2, axis=0).repeat(2, axis=1)
                for i in range(12):
                    Image.fromarray(image).save(folder / f"frame-{i:04}.png")
            command = [sys.executable, str(HERE / "scan-quality-compare.py"), str(root / "plan.json"),
                       str(root / "baseline"), str(root / "candidate"), str(root / "reference")]
            result = subprocess.run(command + [str(root / "pass")], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            for label in ("baseline", "candidate", "reference"):
                record = root / label / "run.txt"
                args = json.loads(record.read_text().split("args=", 1)[1])
                args[args.index("--frames") + 1] = "2"
                args.extend(["--start", "1"])
                record.write_text("test\nargs=" + json.dumps(args) + "\n")
            result = subprocess.run(command + [str(root / "spatial"), "--frames", "1,2"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            spatial = json.loads((root / "spatial/results.json").read_text())
            self.assertEqual(spatial["compared_frames"], [1, 2])
            self.assertEqual(spatial["temporal_status"], "UNRUN")
            with Image.open(root / "candidate/frame-0002.png") as image:
                damaged = np.array(image)
            damaged[12:20, 12:20] = [36, 44, 56]
            Image.fromarray(damaged).save(root / "candidate/frame-0002.png")
            result = subprocess.run(command + [str(root / "fail"), "--frames", "1,2"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            report = json.loads((root / "fail/results.json").read_text())
            self.assertTrue(any("coverage" in issue for issue in report["issues"]))
            self.assertEqual(report["visual_status"], "UNRUN")


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main()
