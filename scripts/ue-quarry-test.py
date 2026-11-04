"""Focused CPU tests for quarry transport and visibility coverage semantics."""
import importlib.util
import csv
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prepare, scoring, profiler = load("ue-quarry-prepare"), load("ue-quarry-score"), load("ue-quarry-profile")


class TransportTests(unittest.TestCase):
    def test_affine_basis_normal_and_mirrored_facing(self):
        vertices = np.array([[1, 2, 3, 0, 0, 1, .25, .5, 1, .5, .25, 1],
                             [2, 2, 3, 0, 0, 1, 1, .5, 1, .5, .25, 1],
                             [1, 3, 3, 0, 0, 1, .25, 1, 1, .5, .25, 1]], dtype="<f4")
        indices = np.array([0, 1, 2], dtype="<u4")
        for sx in (2., -2.):
            # A nonuniform affine with shear also exercises the inverse transpose.
            matrix = np.array([[sx, .4, .2, 10], [0, 3, .1, 20], [0, 0, 4, 30], [0, 0, 0, 1.]])
            baked, actual_indices = prepare.bake(vertices, indices, matrix.T.flatten())
            imported = baked[:, [0, 2, 1]] * 100
            world = vertices[:, :3] @ matrix[:3, :3].T + matrix[:3, 3]
            expected = world[:, [2, 0, 1]] * [-100, 100, 100]
            np.testing.assert_allclose(imported, expected, rtol=1e-6)
            imported_normals = baked[:, [3, 5, 4]]
            self.assertAlmostEqual(float(np.linalg.norm(imported_normals[0])), 1, places=6)
            for edge in (imported[1] - imported[0], imported[2] - imported[0]):
                self.assertAlmostEqual(float(edge @ imported_normals[0]), 0, delta=1e-3)
            a, b, c = imported[actual_indices]
            self.assertLess(float(np.cross(b - a, c - a) @ imported_normals[0]), 0)
            np.testing.assert_array_equal(baked[:, 6:], vertices[:, 6:])
            np.testing.assert_array_equal(indices, [0, 1, 2])

    def test_sampler_mapping_retains_wrap_and_mip_intent(self):
        self.assertEqual(prepare.gltf_sampler([0, 2, 0, 1, 1, 2]),
                         dict(wrapS=10497, wrapT=33071, magFilter=9729, minFilter=9987))
        self.assertEqual(prepare.gltf_sampler([0, 1, 1, 0, 0, 0])["minFilter"], 9728)
        with self.assertRaises(KeyError):
            prepare.gltf_sampler([0, 3, 0, 1, 1, 2])

    def test_truncated_rla_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.rla"
            path.write_bytes(b"RLA\0\1\0\0\0" + struct.pack("<5I", 1, 1, 1, 0, 1) + struct.pack("<3I", 0, 3, 3))
            with self.assertRaisesRegex(ValueError, "span outside"):
                prepare.read_layout(path)


class VisibilityTests(unittest.TestCase):
    def test_profiler_growing_header_keeps_frame_ordinals_and_event_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.csv"
            with path.open("w", newline="") as stream:
                writer = csv.writer(stream)
                writer.writerow(["EVENTS", "GPUTime"])
                for frame in range(165):
                    writer.writerow(["event" if frame == 100 else "", frame] + ([2] if frame >= 4 else []))
                writer.writerow(["EVENTS", "GPUTime", "LateStat"])
                writer.writerow(["[HasHeaderRowAtEnd]", 1])
            result = profiler.summarize(path)
            self.assertEqual(result["captured_rows"], 165)
            self.assertEqual(result["statistics"]["GPUTime"]["median"], 127.5)
            self.assertEqual(result["statistics"]["LateStat"]["median"], 2)

    def test_ratio_has_no_gain_fit_clipping_or_zero_denominator_fill(self):
        cover = np.ones((2, 3), dtype=bool)
        unshadowed = np.full((2, 3, 3), 100.)
        fraction = np.array([[0, .25, 1], [.75, 1.2, .5]])
        shadowed = unshadowed * fraction[..., None]
        unshadowed[1, 2] = 0
        ratio, valid, report = scoring.visibility(shadowed, unshadowed, cover)
        np.testing.assert_allclose(ratio[valid], fraction[valid])
        self.assertTrue(np.isnan(ratio[1, 2]))
        self.assertEqual(report["undefined"], 1)
        self.assertEqual(report["outside_unit_interval"], 1)
        self.assertAlmostEqual(report["maximum_channel_ratio_disagreement"], 0)

    def test_nonfinite_negative_and_uncovered_are_separate(self):
        unshadowed = np.ones((1, 4, 3))
        shadowed = unshadowed.copy()
        shadowed[0, 0, 0] = np.nan
        shadowed[0, 1, 0] = -1
        cover = np.array([[True, True, True, False]])
        _, valid, report = scoring.visibility(shadowed, unshadowed, cover)
        np.testing.assert_array_equal(valid, [[False, False, True, False]])
        self.assertEqual((report["nonfinite"], report["negative_rgb"], report["covered"]), (1, 1, 3))

    def test_reference_gate_identity_and_profile_coverage(self):
        gate = scoring.load_gate(scoring.GATE)
        shape = (1080, 1920)
        cover = np.zeros(shape, dtype=bool)
        cover[10:12, 10:13] = True
        ref = np.ones(shape)
        ref[10, 11] = .5
        ref[11, 11] = .95
        prod = ref.copy()
        arrays = dict(cover=cover, reference=ref, sigma=np.full(shape, .01), depth=np.ones(shape),
                      production=prod, q_prod=np.array(.02), unshadowed=np.ones((*shape, 3)))
        arrays["shadowed"] = np.repeat(ref[..., None], 3, -1)
        # Undefined on one true partial; another faint partial is incorrectly lit.
        arrays["unshadowed"][10, 11] = 0
        arrays["shadowed"][11, 11] = 1
        metadata = dict(reference=dict(seeds=list(range(1, 65)), rays_per_seed=256, production_seed=0,
                                       spread_seeds=list(range(65, 81))), resolution=[1920, 1080],
                        asset_sha256=prepare.ASSET_SHA, reference_sha256="a" * 64,
                        ue_capture_identity_sha256="b" * 64, paired_settings_sha256="c" * 64,
                        environment="black-both", rgb_scale="unexposed-linear", pose=0, state="cut", mode="vsm-smrt")
        result, *_ = scoring.score(arrays, metadata, gate, 1e-6)
        self.assertEqual(result["status"], "PARTIAL_PROFILE")
        self.assertEqual(result["G3"]["partial"]["reference_n"], 2)
        self.assertEqual(result["G3"]["partial"]["evaluated_n"], 1)
        self.assertEqual(result["G1"]["missing"], 1)
        self.assertEqual(result["G6"]["count"], 1)
        self.assertAlmostEqual(result["G6"]["entries"][0]["reference"], .95)
        metadata["reference"]["seeds"] = [0]
        with self.assertRaisesRegex(ValueError, "seeds"):
            scoring.validate_metadata(metadata)


if __name__ == "__main__":
    unittest.main()
