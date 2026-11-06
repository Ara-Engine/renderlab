"""Focused registration checks: native widths, closed tracks, geometry and scoring."""
import importlib.util
from pathlib import Path
import unittest

import numpy as np


def load(name):
    spec=importlib.util.spec_from_file_location(name.replace("-","_"),Path(__file__).with_name(name+".py"))
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


class MotionTests(unittest.TestCase):
    def test_sun_projection_scalar_and_rgb(self):
        projection=load("bench-motion-score").old_sun_projection
        for channels in (1,3):
            shape=(2,2) if channels==1 else (2,2,channels)
            before=np.arange(np.prod(shape),dtype=float).reshape(shape)/10
            current=before+.2;use=np.array([[True,False],[True,True]])
            self.assertAlmostEqual(projection(before,current,before,use),1)
            self.assertAlmostEqual(projection(current,current,before,use),0)
            self.assertAlmostEqual(projection(.25*before+.75*current,current,before,use),.25)
            self.assertIsNone(projection(current,current,current,use))

    def test_s2_projection_and_tracks(self):
        scene=load("bench-motion-scene").s2();f=scene["width_registration"]["focal_pixels"]
        wires=[m for m in scene["meshes"] if m["name"].startswith("wire-")]
        self.assertEqual(len(wires),4)
        for mesh in wires:self.assertAlmostEqual(mesh["width_m"]*f/mesh["centre_view_depth_m"],mesh["projected_width_px"],places=12)
        for track in scene["tracks"]:
            frames=track["frames"];self.assertEqual(len(frames),160)
            for key in ("eye","target","plate_x","blocker_x"):self.assertEqual(frames[0][key],frames[-1][key])
            if "pixels_per_frame" in track:
                displacement=frames[1]["eye"][0]-frames[0]["eye"][0]
                self.assertAlmostEqual(displacement/scene["width_registration"]["metres_per_pixel"],track["pixels_per_frame"],places=12)

    def test_s3_topology_and_sun(self):
        scene=load("bench-motion-scene").s3(16);slab=scene["meshes"][0]
        self.assertEqual(len(slab["indices"]),16*16*6)
        self.assertEqual(len(scene["meshes"][1]["indices"])//3,64*12)
        frames=scene["tracks"][0]["frames"];self.assertEqual(len(frames),385)
        self.assertAlmostEqual(frames[32]["azimuth_degrees"]-frames[31]["azimuth_degrees"],.25/60)
        self.assertAlmostEqual(frames[192]["azimuth_degrees"]-frames[191]["azimuth_degrees"],1/60)
        self.assertAlmostEqual(frames[352]["azimuth_degrees"]-frames[351]["azimuth_degrees"],5)
        self.assertEqual(frames[-1]["phase_age"],32)
        for m in scene["meshes"]:
            p=np.array(m["positions"]);idx=np.array(m["indices"]).reshape(-1,3)
            area=np.cross(p[idx[:,1]]-p[idx[:,0]],p[idx[:,2]]-p[idx[:,0]])
            self.assertTrue((np.linalg.norm(area,axis=-1)>0).all())
        np.testing.assert_allclose(np.linalg.norm([f["to_light"] for f in frames],axis=-1),1,atol=1e-14)

    def test_missing_domains(self):
        # Keep the retained S1 missing-domain and packed-HDR boundary tests.
        suite=unittest.defaultTestLoader.loadTestsFromModule(load("bench-contact-fold-test"))
        result=unittest.TestResult();suite.run(result)
        self.assertEqual(result.errors,[]);self.assertEqual(result.failures,[])


if __name__=="__main__":unittest.main()
