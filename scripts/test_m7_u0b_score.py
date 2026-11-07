"""CPU invariants for the retained physical-floor interface. Scratch stays on E:."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

spec=importlib.util.spec_from_file_location("score",Path(__file__).with_name("m7-u0b-score.py"))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)


class PhysicalFloorTests(unittest.TestCase):
    def test_strict_raw_data_can_defer_archival_until_lease_release(self):
        scratch=Path("E:/renderlab-work/m7-u0b/cpu-tests");scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);name="fixture";path=root/(name+".json")
            meta=dict(width=2,height=2,samples=2,accepted_for_scoring=True,strict_replay=dict(bit_identical=True))
            score.save(path,meta)
            for suffix,value in (("mean",10),("se",1),("half0",11),("half1",9)):
                np.full((2,2,3),value,"<f4").tofile(root/(name+"-"+suffix+".f32"))
            args=argparse.Namespace(root=root,name=name)
            with patch.dict(score.os.environ,{"RENDERLAB_M7_DEFER_REFERENCE_COMPACT":"1"}):
                score.compact(args)
            self.assertFalse((root/(name+".npz")).exists())
            self.assertTrue((root/(name+"-half0.f32")).exists())
            pending=root/(name+".compact-pending.json")
            self.assertTrue(pending.exists())
            score.save(path,dict(meta,samples=4))
            with self.assertRaisesRegex(ValueError,"metadata changed"):
                score.compact(args)
            score.save(path,meta)
            score.compact(args)
            self.assertFalse(pending.exists())
            with np.load(root/(name+".npz")) as arrays:
                np.testing.assert_array_equal(arrays["L0"],np.full((2,2,3),11,"f4"))
            self.assertFalse((root/(name+"-half0.f32")).exists())

    def test_independent_batches_keep_actual_halves_and_reject_duplicate_seeds(self):
        scratch=Path("E:/renderlab-work/m7-u0b/cpu-tests");scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);inputs=[]
            for i,value in enumerate((10.,30.)):
                path=root/f"batch{i}.npz";inputs.append(path)
                score.archive(path,dict(L0=np.full((2,2,3),value+1,"f4"),L1=np.full((2,2,3),value-1,"f4")))
                score.save(path.with_suffix(".json"),dict(bench="fixture",pose="p",scene_sha256="same-source",
                    smith="correlated",max_depth=1024,camera_background=[0,0,0],width=2,height=2,
                    seed=i,samples=4,archive_sha256=score.sha(path)))
            args=argparse.Namespace(root=root/"combined",name="result",inputs=inputs)
            score.combine(args)
            meta=json.loads((args.root/"result.json").read_text())
            self.assertEqual(meta["samples"],8)
            with np.load(args.root/"result.npz") as arrays:
                np.testing.assert_array_equal(arrays["L0"],np.full((2,2,3),21,"f4"))
                np.testing.assert_array_equal(arrays["L1"],np.full((2,2,3),19,"f4"))
            args.inputs=[inputs[0],inputs[0]]
            with self.assertRaisesRegex(ValueError,"duplicate seed"):
                score.combine(args)

    def test_identical_physical_images_and_correlated_reference_reuse(self):
        scratch=Path("E:/renderlab-work/m7-u0b/cpu-tests");scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);reference=root/"reference";reference.mkdir()
            walk=root/"walk";walk.mkdir();capture=walk/"fixture";capture.mkdir()
            poses=[dict(eye=[x,0.,0.],target=[x,0.,-1.],fov_y=.85) for x in (0.,1.)]
            means=[np.full((2,2,3),v,"f4") for v in (100.,200.)]
            for i,(pose,mean) in enumerate(zip(poses,means)):
                path=reference/f"r{i}.npz"
                # Each reference has nonzero independent-half uncertainty. Reusing
                # the very same reference on a held camera must cancel it in a step.
                score.archive(path,dict(L0=mean+1,L1=mean-1,hit=np.ones((2,2,3),"u4"),
                    **{"pop_all-covered":np.ones((2,2),bool)}))
                frame=dict(open={k:pose[k] for k in ("eye","target")},
                           close={k:pose[k] for k in ("eye","target")},fov_y=.85,sky_only=False)
                score.save(path.with_suffix(".json"),dict(frame=frame,samples=16,
                    camera_background=[0,0,0],archive_sha256=score.sha(path)))
            frames=[poses[0],poses[0],poses[1]];native=[]
            for i,index in enumerate((0,0,1)):
                path=capture/f"f{i}.npz"
                score.archive(path,dict(hdr=means[index]*2.**-14,pre_exposure=np.float32(2.**-14)))
                native.append(dict(archive=str(path),archive_sha256=score.sha(path),pre_exposure=2.**-14))
            score.save(capture/"recipe.json",dict(config="fixture",frames=frames))
            score.save(capture/"native.json",dict(frames=native))
            score.save(capture/"camera-readback.json",dict(all_ok=True))
            score.save(capture/"readback.json",dict(all_ok=True,effective_mode={}))
            score.save(capture/"process.json",dict(exit_code=0,failures=[]))
            args=argparse.Namespace(root=walk,reference=reference,stops=-14)
            score.floor(args)
            result=json.loads((capture/"physical-floor.json").read_text())
            self.assertTrue(result["complete"])
            self.assertEqual(len(result["frames"]),3)
            self.assertEqual(len(result["motion"]),4)
            for frame in result["frames"]:
                for measure in frame["measures"].values():
                    self.assertEqual(measure["all-covered"]["rms"],0)
                    self.assertGreater(measure["all-covered"]["reference_noise_rms"],0)
            for step in result["motion"]:
                self.assertEqual(step["measures"]["all-covered"]["rms"],0)
                if step["static_camera"]:
                    self.assertEqual(step["measures"]["all-covered"]["reference_noise_rms"],0)
            score.save(capture/"process.json",dict(exit_code=0,failures=[dict(reason="late monitor failure")]))
            with self.assertRaisesRegex(ValueError,"capture coverage/readback failed"):
                score.floor(args)
            score.save(capture/"process.json",dict(exit_code=0,failures=[]))
            # An image-domain gain is not a substitute for the measured exposure.
            native[0]["pre_exposure"]=2.**-12
            score.save(capture/"native.json",dict(frames=native))
            with self.assertRaisesRegex(ValueError,"pre-exposure drift"):
                score.floor(args)
            native[0]["pre_exposure"]=2.**-14
            score.save(capture/"native.json",dict(frames=native))
            meta=reference/"r0.json";value=json.loads(meta.read_text())
            value["camera_background"]=[1,1,1];score.save(meta,value)
            with self.assertRaisesRegex(ValueError,"unverified reference"):
                score.floor(args)


if __name__=="__main__":unittest.main()
