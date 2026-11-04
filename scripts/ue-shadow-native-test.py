"""Focused CPU tests for RDG resource versions, packed formats and shader layouts."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

import numpy as np

spec = importlib.util.spec_from_file_location("native_dump", Path(__file__).with_name("ue-shadow-dump.py"))
dump = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dump)


class DumpTests(unittest.TestCase):
    def test_adjacent_json_objects_and_malformed_tail(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "objects.json"
            path.write_text(' {"one": 1}\n{"two": 2}  ')
            self.assertEqual(dump.objects(path), [dict(one=1), dict(two=2)])
            path.write_text('{"one": 1}broken')
            with self.assertRaises(json.JSONDecodeError):
                dump.objects(path)

    def test_consumer_reads_preceding_writer_even_when_pointer_order_disagrees(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Resources").mkdir()
            instance = dump.Dump.__new__(dump.Dump)
            instance.root = root
            instance.resources = {"tex": dict(Format="PF_FloatRGBA", ExtentX=1, ExtentY=1)}
            instance.passes = [dict(Id=0, Pointer="ffff", OutputResources=["tex.mip0"]),
                               dict(Id=1, Pointer="aaaa", OutputResources=["tex.mip0"])]
            for writer, value in (("ffff", 1), ("aaaa", 2)):
                np.full(4, value, dtype="<f2").tofile(root / "Resources" / f"tex.mip0.v{writer}.bin")
            value, identity = instance.resource("tex.mip0", dict(Id=1))
            self.assertEqual(identity["version"], "ffff")
            np.testing.assert_array_equal(value, [[[1, 1, 1, 1]]])
            value, identity = instance.resource("tex.mip0", dict(Id=2))
            self.assertEqual(identity["version"], "aaaa")
            np.testing.assert_array_equal(value, [[[2, 2, 2, 2]]])

    def test_native_bgra_and_ten_bit_normal_are_not_interchangeable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Resources").mkdir()
            instance = dump.Dump.__new__(dump.Dump)
            instance.root, instance.passes = root, []
            instance.resources = {"mask": dict(Format="PF_B8G8R8A8", ExtentX=1, ExtentY=1),
                                  "normal": dict(Format="PF_A2B10G10R10", ExtentX=1, ExtentY=1)}
            (root / "Resources/mask.mip0.v00000000000000000000.bin").write_bytes(bytes([255, 0, 128, 255]))
            np.array([1023 | (512 << 10) | (0 << 20) | (3 << 30)], dtype="<u4").tofile(root / "Resources/normal.mip0.v00000000000000000000.bin")
            mask, _ = instance.resource("mask.mip0", dict(Id=0))
            self.assertAlmostEqual(float(mask[0, 0, 0] ** 2 * mask[0, 0, 2] ** 2), (128 / 255) ** 2)
            normal, _ = instance.resource("normal.mip0", dict(Id=0))
            np.testing.assert_allclose(normal[0, 0] * 2 - 1, [1, 1 / 1023, -1])

    def test_inline_nested_parameters_and_shader_array_padding(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Structures").mkdir()
            instance = dump.Dump.__new__(dump.Dump)
            instance.root = root
            def member(name, kind, offset, columns=1, elements=0, metadata="unused"):
                return dict(Name=name, BaseType=kind, Offset=offset, NumRows=1,
                            NumColumns=columns, NumElements=elements, StructMetadata=metadata)
            instance.metadata = {
                "outer": dict(Members=[member("Light", "UBMT_NESTED_STRUCT", 0, metadata="light"),
                                       member("Values", "UBMT_FLOAT32", 16, columns=2, elements=2)]),
                "light": dict(Members=[member("SourceRadius", "UBMT_FLOAT32", 0)])}
            values = np.zeros(12, dtype="<f4")
            values[0], values[4:6], values[8:10] = .04, [1, 2], [3, 4]
            values.tofile(root / "Structures/params.bin")
            parsed = instance.scalar_parameters(dict(Parameters="params", ParametersMetadata="outer"))
            self.assertAlmostEqual(parsed["Light.SourceRadius"], .04)
            self.assertEqual(parsed["Values"], [1, 2, 3, 4])


if __name__ == "__main__":
    unittest.main()
