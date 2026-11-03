"""Focused fixture-conversion checks. No UE, Cargo, GPU, windows or native renderer."""
import ast
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


class FixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scene = prepare.load_fixture()

    def test_actual_export_identity_and_hash_failure(self):
        self.assertEqual(prepare.sha(prepare.FIXTURE.read_bytes()), prepare.FIXTURE_SHA)
        prepare.DEFAULT_ROOT.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="cpu-test-", dir=prepare.DEFAULT_ROOT) as temporary:
            path = Path(temporary) / "changed-scene.json"
            changed = copy.deepcopy(self.scene)
            changed["lighting"]["lights"][0]["lux"][0] += 1
            prepare.write_json(path, changed)
            with self.assertRaisesRegex(ValueError, "fixture bytes"):
                prepare.load_fixture(path)
            # Whitespace changes also reject: identity is exported bytes, not invented canonical JSON.
            path.write_bytes(prepare.FIXTURE.read_bytes()+b"\n")
            with self.assertRaises(ValueError):
                prepare.load_fixture(path)

    def test_sphere_exact_topology_and_authored_normals(self):
        sphere = prepare.geometry(self.scene["objects"][0]["geometry"])
        self.assertEqual(len(sphere["positions"]), 2145)
        self.assertEqual(len(sphere["indices"]), 12288)
        self.assertEqual(sphere["indices"][:6], [0, 1, 65, 1, 66, 65])
        self.assertEqual(sphere["indices"][-6:], [2078, 2079, 2143, 2079, 2144, 2143])
        for y in range(32):
            for x in range(64):
                offset = 6*(y*64+x)
                a = y*65+x
                self.assertEqual(sphere["indices"][offset:offset+6], [a, a+1, a+65, a+1, a+66, a+65])
        for i, (p, n) in enumerate(zip(sphere["positions"], sphere["normals"])):
            y, x = divmod(i, 65)
            analytic = [math.sin(math.pi*y/32)*math.cos(math.tau*x/64),
                        math.cos(math.pi*y/32), math.sin(math.pi*y/32)*math.sin(math.tau*x/64)]
            for value, expected in zip(n, analytic):
                self.assertAlmostEqual(value, expected, delta=7e-7)
            self.assertAlmostEqual(prepare.dot(n, n), 1, delta=3e-7)
            self.assertEqual(p, [prepare.f32(v*.5) for v in n])
        # Preserve all duplicated pole/seam IDs and degenerate triangles; no welding/cleanup.
        self.assertEqual(sphere["positions"][0], sphere["positions"][64])
        self.assertEqual(len(set(sphere["indices"])), 2145)
        self.assertEqual(prepare.sha(prepare.mesh_bytes(sphere)), SPHERE_BUFFER_SHA)

    def test_column_major_transform_normals_and_source_winding(self):
        self.assertEqual(prepare.to_ue([1, 2, 3]), [-300, 100, 200])
        for obj in self.scene["objects"]:
            mesh, record = prepare.convert_object(obj)
            columns = obj["transform_column_major"]
            source = prepare.geometry(obj["geometry"])
            self.assertEqual(len(mesh["positions"]), obj["vertices"])
            self.assertEqual(len(mesh["indices"]), 3*obj["triangles"])
            for i, p in enumerate(source["positions"]):
                world = [columns[row]*p[0]+columns[4+row]*p[1]+columns[8+row]*p[2]+columns[12+row] for row in range(3)]
                self.assertEqual(mesh["positions"][i], [prepare.f32(v) for v in prepare.to_ue(world)])
                self.assertAlmostEqual(prepare.dot(mesh["normals"][i], mesh["normals"][i]), 1, delta=2e-7)
            self.assertEqual(record["mirrored_index_swap"], obj["name"] == "mirrored_nonuniform")
            if obj["geometry"]["shape"] == "xy_quad":
                a, b, c = [mesh["positions"][v] for v in mesh["indices"][:3]]
                geometric = prepare.cross([v-u for u, v in zip(a, b)], [v-u for u, v in zip(a, c)])
                normal = mesh["normals"][0]
                # UE clockwise source-front has cross-dot-normal < 0, even on mirrored geometry.
                self.assertEqual(prepare.dot(geometric, normal) < 0, not obj["geometry"]["reverse_indices"])
                # Inverse transpose remains perpendicular under rotated nonuniform/mirrored scale.
                self.assertAlmostEqual(prepare.dot(normal, [v-u for u, v in zip(a, b)]), 0, delta=2e-5)
                self.assertAlmostEqual(prepare.dot(normal, [v-u for u, v in zip(a, c)]), 0, delta=2e-5)
        mirrored = next(o for o in self.scene["objects"] if o["name"] == "mirrored_nonuniform")
        mesh, _ = prepare.convert_object(mirrored)
        self.assertEqual(mesh["indices"], [0, 2, 1, 0, 3, 2])
        with self.assertRaises(ValueError):
            prepare.matrix_parts([0]*16)

    def test_light_photometry_positions_range_and_cones(self):
        records = [prepare.light_record(l) for l in self.scene["lighting"]["lights"]]
        self.assertEqual([r["intensity_units"] for r in records], ["lux", "candela", "candela"])
        for record, original in zip(records, self.scene["lighting"]["lights"]):
            self.assertEqual(record["source"], original)
            rgb = original["lux"] if original["type"] == "directional" else original["candela"]
            for v, expected in zip(record["linear_color"], rgb):
                self.assertAlmostEqual(v*record["intensity"], expected)
        self.assertEqual(records[1]["position_cm"], [-100, -200, 200])
        self.assertEqual(records[1]["attenuation_radius_cm"], 600)
        self.assertEqual(records[2]["attenuation_radius_cm"], 700)
        self.assertAlmostEqual(math.radians(records[2]["inner_cone_half_angle_degrees"]), .25)
        self.assertAlmostEqual(math.radians(records[2]["outer_cone_half_angle_degrees"]), .5)
        self.assertAlmostEqual(prepare.dot(records[0]["travel_ue"], records[0]["travel_ue"]), 1)

    def test_camera_and_shared_display(self):
        probes = prepare.projected_pixels(self.scene)
        self.assertEqual(len(probes), 9)
        # Three sphere centers on the first row preserve vertical projection and 1.5m spacing.
        self.assertAlmostEqual(probes[1]["pixel_xy"][0], 480)
        self.assertAlmostEqual(probes[0]["pixel_xy"][1], probes[2]["pixel_xy"][1])
        self.assertAlmostEqual(probes[1]["pixel_xy"][0]-probes[0]["pixel_xy"][0],
                               probes[2]["pixel_xy"][0]-probes[1]["pixel_xy"][0])
        self.assertEqual(prepare.display_rgb([0, -1, 0], -8), [0, 0, 0])
        self.assertAlmostEqual(prepare.display_rgb([256], -8)[0], .7353569830524495)
        with self.assertRaises(ValueError):
            prepare.display_rgb([math.inf], -8)

    def test_generated_manifests_preserve_values_and_variants(self):
        with tempfile.TemporaryDirectory(prefix="cpu-test-", dir=prepare.DEFAULT_ROOT) as temporary:
            root = Path(temporary)
            for variant in prepare.VARIANTS:
                target = root / variant
                prepare.generate(target, prepare.DEFAULT_ENGINE, variant=variant)
                manifest = json.loads((target / "manifest.json").read_text())
                expected = json.loads((target / "expected.json").read_text())
                self.assertEqual((target / "scene.json").read_bytes(), prepare.FIXTURE.read_bytes())
                self.assertEqual(expected["total_vertices"], 12882)
                self.assertEqual(expected["total_triangles"], 24582)
                self.assertEqual(manifest["camera"], self.scene["camera"])
                self.assertEqual(manifest["source_lighting"], self.scene["lighting"])
                self.assertEqual(manifest["runtime_status"], "UNRUN")
                for obj, original in zip(manifest["objects"], self.scene["objects"]):
                    self.assertEqual(obj["source"], original)
                    self.assertEqual(obj["material"]["base_color_linear"], original["base_color_linear"])
                    self.assertEqual(obj["material"]["roughness"], original["perceptual_roughness"])
                    self.assertEqual(obj["material"]["metallic"], original["metallic"])
                    self.assertEqual(obj["material"]["two_sided"], original["double_sided"])
                    self.assertEqual(obj["material"]["specular"]*.08, .04)
                    data = (target / obj["mesh"]).read_bytes()
                    self.assertEqual(prepare.sha(data), obj["mesh_file_sha256"])
                    self.assertEqual(prepare.sha(prepare.mesh_bytes(json.loads(data))), obj["ue_world_buffer_sha256"])
                active = json.loads((target / "renderlab-lighting.json").read_text())
                self.assertEqual(active, manifest["active_lighting"])
                if variant == "directional-black":
                    self.assertEqual(active["lights"], self.scene["lighting"]["lights"][:1])
                    self.assertEqual(active["environment"], [0, 0, 0])
                else:
                    self.assertEqual(active, self.scene["lighting"])
                    self.assertIn("PENDING", manifest["environment_status"])
                before = {p.relative_to(target): p.read_bytes() for p in target.rglob("*") if p.is_file()}
                prepare.generate(target, prepare.DEFAULT_ENGINE, variant=variant)
                after = {p.relative_to(target): p.read_bytes() for p in target.rglob("*") if p.is_file()}
                self.assertEqual(before, after)
                self.assertIn("-NullRHI", (target / "prepare-assets.ps1").read_text())
                self.assertIn("-AllowCommandletRendering", (target / "capture.ps1").read_text())
                self.assertNotIn("-NullRHI", (target / "capture.ps1").read_text())
                for script in (target / "Content/Python").glob("*.py"):
                    ast.parse(script.read_text(encoding="utf-8-sig"))

    def test_generator_does_not_launch_processes(self):
        tree = ast.parse((HERE / "ue-m5-prepare.py").read_text(encoding="utf-8-sig"))
        imports = {n.names[0].name for n in ast.walk(tree) if isinstance(n, ast.Import)}
        self.assertFalse(imports & {"unreal", "subprocess", "os", "ctypes"})
        ast.parse((HERE / "ue-m5-editor.py").read_text(encoding="utf-8-sig"))


# Regression identity of the owned generator's recipe, NOT evidence of Rust buffer equality.
SPHERE_BUFFER_SHA = "e9091cd750c13a8307a4b6f4da252f0d1a73353aaea6a3dec92534b806965adf"
if __name__ == "__main__":
    unittest.main(testRunner=unittest.TextTestRunner(stream=sys.stdout))
