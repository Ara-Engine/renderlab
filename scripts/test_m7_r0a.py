"""CPU checks at the fixture and scheduling contracts; no GPU calls."""
import importlib.util
from pathlib import Path
import unittest

import numpy as np


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name+".py"))
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


class Fixtures(unittest.TestCase):
    def test_geometry_and_populations_are_well_formed(self):
        scenes = load("m7-r0a-scenes")
        for name, make in scenes.BENCHES.items():
            with self.subTest(name=name):
                s = make(); names = {m["name"] for m in s["meshes"]}
                self.assertEqual(len(names), len(s["meshes"]))
                for m in s["meshes"]:
                    p = np.asarray(m["positions"]); n = np.asarray(m["normals"])
                    idx = np.asarray(m["indices"]).reshape(-1, 3)
                    self.assertTrue(np.isfinite(p).all() and np.isfinite(n).all())
                    self.assertEqual(p.shape, n.shape)
                    self.assertTrue((idx>=0).all() and (idx<len(p)).all())
                    v = p[idx]; area = np.cross(v[:, 1]-v[:, 0], v[:, 2]-v[:, 0])
                    self.assertTrue((np.linalg.norm(area, axis=1)>1e-12).all(), m["name"])
                    self.assertTrue((np.abs(np.linalg.norm(n, axis=1)-1)<1e-5).all())
                for pop, members in s["populations"].items():
                    self.assertTrue(set(members)<=names, (name, pop, members))

    def test_motion_uses_one_common_shutter_time(self):
        scenes = load("m7-r0a-scenes")
        s = scenes.rf5()
        a = scenes.state_at(s, "object-sun", 48)
        b = scenes.state_at(s, "object-sun", 49)
        mid = scenes.state_at(s, "object-sun", 48.5)
        self.assertTrue(np.allclose(mid["movers"]["offscreen-red"],
                                   (np.array(a["movers"]["offscreen-red"])+b["movers"]["offscreen-red"])/2))
        self.assertAlmostEqual(np.linalg.norm(mid["to_light"]), 1)
        frozen = scenes.snapshot(s, mid)
        original = next(m for m in s["meshes"] if m["name"]=="offscreen-red")
        moved = next(m for m in frozen["meshes"] if m["name"]=="offscreen-red")
        self.assertTrue(np.allclose(np.array(moved["positions"])-original["positions"], mid["movers"]["offscreen-red"]))
        self.assertEqual(frozen["sun"]["to_light"], mid["to_light"])
        track = s["tracks"][0]["frames"]
        self.assertNotEqual(track[31]["movers"], track[32]["movers"])
        self.assertEqual(track[79]["movers"], track[80]["movers"])
        red = next(m for m in s["meshes"] if m["name"]=="offscreen-red")
        blue = next(m for m in s["meshes"] if m["name"]=="offscreen-blue")
        red_max = np.asarray(red["positions"])[:,0].max()
        blue_min = np.asarray(blue["positions"])[:,0].min()
        self.assertTrue(all(red_max+f["movers"]["offscreen-red"][0]<blue_min for f in track),
                        "moving slabs must not overlap on their shared front plane")

    def test_quiet_window_needs_its_own_release(self):
        r = load("m7-r0a")
        text = "[m7-u0b QUIET window] label=quiet-camera token=a\n"
        self.assertTrue(r.active_quiet(text))
        self.assertTrue(r.active_quiet(text+"m7-tr1 study-quiet-camera window released now\n"))
        self.assertFalse(r.active_quiet(text+"m7-u0b study-quiet-camera window released now\n"))
        self.assertTrue(r.active_quiet("", dict(owner="foreign", label="quiet-cost")))
        tr1 = "[m7-tr1 QUIET active] owner=m7-tr1 label=tq-1 start=now\n"
        self.assertTrue(r.active_quiet(tr1))
        self.assertFalse(r.active_quiet(tr1+"m7-tr1 tr1-tq-1 window released now\n"))

    def test_scores_keep_missing_energy_and_reference_noise_visible(self):
        r = load("m7-r0a")
        a = np.full((3, 4, 3), 11., dtype="f4")
        b = np.full_like(a, 9.)
        mask = np.ones((3, 4), bool)
        exact = r.measures((a+b)/2, a, b, mask)
        self.assertEqual(exact["rgb_relative_rms"], 0)
        self.assertAlmostEqual(exact["split_noise_relative_rms"], .1)
        missing = r.measures(np.zeros_like(a), a, b, mask)
        self.assertEqual(missing["signed_energy_bias"], -1)
        self.assertEqual(missing["rgb_relative_rms"], 1)

    def test_trail_extent_is_bounded_and_never_calls_a_missing_feature_zero(self):
        r = load("m7-r0a")
        feature = np.zeros((5, 5), bool); feature[2, 3] = True
        visible = np.zeros_like(feature); visible[2, 0] = True
        self.assertEqual(r.trail_extent(visible, feature)["extent_pixels"], 3)
        capped = r.trail_extent(visible, np.zeros_like(feature), cap=2)
        self.assertTrue(capped["lower_bound"])
        self.assertEqual(capped["extent_pixels"], 3)


if __name__ == "__main__":
    unittest.main()
