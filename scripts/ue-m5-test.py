"""Focused stdlib tests; no Unreal import, process, native window or GPU."""

import ast
import configparser
import copy
import importlib.util
import json
import math
from pathlib import Path
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ue_m5_prepare", HERE / "ue-m5-prepare.py")
prepare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(prepare)


class PreparationTests(unittest.TestCase):
    def test_units_axes_and_projection(self):
        self.assertEqual(prepare.to_ue_cm([1, 2, 3]), [-300, 100, 200])
        scene = prepare.default_scene()
        probes = prepare.projected_pixels(scene)
        self.assertAlmostEqual(probes[0]["expected_pixel_xy"][0], 320)
        self.assertAlmostEqual(probes[0]["expected_pixel_xy"][1], 180)
        focal = 180 / math.tan(math.radians(25))
        self.assertAlmostEqual(probes[1]["expected_pixel_xy"][0], 320 + focal/5)
        self.assertAlmostEqual(probes[2]["expected_pixel_xy"][1], 180 + focal*.6/4.2)

    def test_reject_unimplemented_fixture_inputs(self):
        for key, value in (("samples", 4), ("environment_rgb_cd_m2", [1, 1, 1]), ("specular", .5),
                           ("unknown_setting", 1), ("diffuse_linear_rgb", [float("nan"), .18, .18])):
            with self.subTest(key=key), self.assertRaises(ValueError):
                scene = prepare.default_scene()
                scene[key] = value
                prepare.validate_scene(scene)
        scene = prepare.default_scene()
        scene["primitives"][0]["kind"] = "private_import"
        with self.assertRaises(ValueError):
            prepare.validate_scene(scene)

    def test_shared_offline_display(self):
        self.assertEqual(prepare.display_rgb([0, -1, 0], -6), [0, 0, 0])
        for value in prepare.display_rgb([64, 64, 64], -6):
            self.assertAlmostEqual(value, .7353569830524495)
        with self.assertRaises(ValueError):
            prepare.display_rgb([float("inf"), 1, 1], -6)

    def test_generate_and_rerun_are_deterministic(self):
        prepare.DEFAULT_ROOT.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="cpu-test-", dir=prepare.DEFAULT_ROOT) as temporary:
            root = Path(temporary)
            self.assertTrue(root.resolve().is_relative_to(prepare.DEFAULT_ROOT.resolve()))
            prepare.generate(root, prepare.DEFAULT_ENGINE, prepare.default_scene())
            before = {p.relative_to(root): p.read_bytes() for p in root.rglob("*") if p.is_file()}
            prepare.generate(root, prepare.DEFAULT_ENGINE, copy.deepcopy(prepare.default_scene()))
            after = {p.relative_to(root): p.read_bytes() for p in root.rglob("*") if p.is_file()}
            self.assertEqual(before, after)
            expected = json.loads((root / "expected.json").read_text())
            self.assertAlmostEqual(expected["lambert_patch_cd_m2"][0], 180/math.pi)
            project = json.loads((root / "M5Calibration.uproject").read_text())
            self.assertEqual([p["Name"] for p in project["Plugins"]], ["PythonScriptPlugin", "EditorScriptingUtilities"])
            config = configparser.ConfigParser(interpolation=None)
            config.read(root / "Config/DefaultEngine.ini", encoding="utf-8")
            self.assertEqual(config["SystemSettings"]["r.AntiAliasingMethod"], "0")
            self.assertEqual(config["SystemSettings"]["r.UsePreExposure"], "0")
            self.assertEqual(config["/Script/Engine.RendererSettings"]["r.ForwardShading"], "False")
            self.assertNotIn("Zen", config["RenderlabLocalDDC"]["Root"])
            for script in (root / "Content/Python").glob("*.py"):
                ast.parse(script.read_text(), filename=str(script))
            self.assertIn("-NullRHI", (root / "prepare-assets.ps1").read_text())
            capture = (root / "capture.ps1").read_text()
            self.assertIn("-AllowCommandletRendering", capture)
            self.assertNotIn("-NullRHI", capture)

    def test_generator_has_no_process_api(self):
        tree = ast.parse((HERE / "ue-m5-prepare.py").read_text())
        imports = {node.names[0].name for node in ast.walk(tree) if isinstance(node, ast.Import)}
        self.assertFalse(imports & {"unreal", "subprocess", "os"})
        ast.parse((HERE / "ue-m5-editor.py").read_text())


if __name__ == "__main__":
    unittest.main()
