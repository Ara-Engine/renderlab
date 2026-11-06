"""Focused CPU checks for S1 transport, registration and missing-domain scoring."""
import importlib.util
from pathlib import Path
import unittest

import numpy as np


def load(name):
    spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m


class BenchTests(unittest.TestCase):
    def test_scene(self):
        s=load("bench-contact-fold-scene").scene()
        self.assertEqual(s["triangles"],38)
        self.assertEqual(len(s["track"]),224)
        self.assertEqual(s["track"][0]["eye"],s["track"][-1]["eye"])
        np.testing.assert_allclose(np.linalg.norm(s["to_light"]),1,atol=1e-14)
        strips=[m for m in s["meshes"] if m["name"].startswith("strip")]
        self.assertEqual(len(strips),9)
        self.assertEqual({round(m["positions"][0][1]*10000) for m in strips},{2,10,50})

    def test_triangles(self):
        s=load("bench-contact-fold-scene").scene()
        for m in s["meshes"]:
            p=np.array(m["positions"],dtype=float);n=np.array(m["normals"])[0]
            geometric=np.cross(p[1]-p[0],p[2]-p[0]);geometric/=np.linalg.norm(geometric)
            np.testing.assert_allclose(geometric,n,atol=1e-14)

    def test_scores_preserve_missing(self):
        module=load("bench-contact-fold-score")
        ref=np.array([[0.,1.,.25],[0.,1.,.25],[0.,1.,.25]])
        cover=np.ones(ref.shape,bool);d=dict(ref=ref,cover=cover,depth=np.ones_like(ref),n=np.full(ref.shape,256))
        pops=module.metrics.populations(d)
        candidate=np.array([[1.,0.,1.],[0.,1.,.25],[0.,1.,.25]])
        valid=cover.copy();valid[0,2]=False
        scored=module.scored(candidate,d,pops,valid)
        self.assertEqual(scored["solid_flips"],2)
        self.assertEqual(scored["leaks"],1)
        self.assertEqual(scored["missing_covered"],1)
        self.assertEqual(scored["partial"]["reference_n"],3)
        self.assertEqual(scored["partial"]["missing_n"],1)

    def test_hdr_packed_format(self):
        module=load("bench-contact-fold-score")
        # Unsigned mini-floats: 1.0 = exponent 15, mantissa zero on all channels.
        packed=np.array([0,(15<<6)|((15<<6)<<11)|((15<<5)<<22)],dtype=np.uint32)
        np.testing.assert_array_equal(module.unpack_r11g11b10(packed),[[0,0,0],[1,1,1]])


if __name__ == "__main__":unittest.main()
