"""Focused CPU contract tests: temporal replay, root orders and scoring identity."""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image
import images
import lease
import prepare
from capture import WorkingSet


class Contracts(unittest.TestCase):
    def test_replay_has_unblended_cut_and_contiguous_motion(self):
        frames=prepare.frames([2,2,5],[0,1,0],.85,False)
        cuts=[i for i,f in enumerate(frames) if f["cut"]]
        self.assertEqual(cuts,[72])
        self.assertTrue(all(f["phase"]=="move" for f in frames[40:72]))
        self.assertNotEqual(frames[71]["eye"],frames[72]["eye"])
        self.assertTrue(frames[72]["capture"])

    def test_quoted_order_is_not_a_directive_and_release_must_be_later(self):
        root=Path("D:/Coding/renderlab-work/ara1/tests");root.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as d:
            path=Path(d)/"schedule.md"
            path.write_text("root gate window released\n[root order] new hold\nworker quotes [root order] old\n")
            state=lease.order_state(path)
            self.assertEqual(state["line"],"[root order] new hold")
            with patch.object(lease,"order_state",return_value=state):
                with self.assertRaises(RuntimeError):lease.check_order(state["sha256"],["gate"])
                with self.assertRaises(RuntimeError):lease.check_order("old")
            state["following"]=["m7-worker says gate window released 2026-11-09T00:00:00Z"]
            self.assertFalse(lease.released_after(state,"gate"))
            state["following"].append("root gate-1 window released 2026-11-09T00:00:01Z (no job remains)")
            self.assertTrue(lease.released_after(state,"gate"))

    def test_scorer_refuses_camera_identity_or_size_drift(self):
        root=Path("D:/Coding/renderlab-work/ara1/tests");root.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as d:
            folder=Path(d);scene=folder/"scene.json"
            scene.write_text(json.dumps(dict(size=[4,4],frames=[dict(phase="still",cut=False)])))
            image=folder/"reference.png";Image.new("RGB",(4,4)).save(image)
            identity=dict(scene_sha256=images.digest(scene),frame_index=0,image_sha256=images.digest(image),
                          display_transform="AcesFit+sRGB",accepted_for_scoring=True)
            Path(str(image)+".json").write_text(json.dumps(identity))
            manifest=folder/"comparison.json"
            manifest.write_text(json.dumps(dict(scene=str(scene),scene_sha256=images.digest(scene),display_transform="AcesFit+sRGB",
                                                 frames=[dict(index=0,reference=str(image))])))
            images.checked_manifest(manifest)
            document=json.loads(manifest.read_text());document["frames"][0]["rt"]=str(image)
            manifest.write_text(json.dumps(document))
            score=folder/"score.json";images.score(manifest,score)
            result=json.loads(score.read_text())
            self.assertEqual(result["rows"][0]["linear_display_rgb"]["whole_image"]["rms"],0.0)
            identity["frame_index"]=1;Path(str(image)+".json").write_text(json.dumps(identity))
            with self.assertRaises(ValueError):images.checked_manifest(manifest)
            Image.new("RGB",(5,4)).save(image)
            with self.assertRaises(ValueError):images.checked_manifest(manifest)

    def test_geometry_is_indexed_and_ground_uses_original_uv_scale(self):
        for vertices,indices in [prepare.cube(),prepare.sphere(),prepare.plane()]:
            self.assertEqual(len(indices)%3,0)
            self.assertLess(int(indices.max()),len(vertices))
        self.assertEqual(prepare.plane()[0][-1,6:].tolist(),[60.0,60.0])

    def test_four_column_compositor_keeps_exact_panel_pixels(self):
        root=Path("D:/Coding/renderlab-work/ara1/tests");root.mkdir(parents=True,exist_ok=True)
        with tempfile.TemporaryDirectory(dir=root) as d:
            folder=Path(d);row=dict(index=0);colors=[(255,0,0),(0,255,0),(0,0,255),(120,100,80)]
            for (key,_),color in zip(images.PANELS,colors):
                path=folder/(key+".png");Image.new("RGB",(16,8),color).save(path);row[key]=path
            result=images.sheet(row,dict(size=[16,8],frames=[dict(phase="synthetic CPU fixture")]))
            self.assertEqual(result.size,(640,54))
            for i,color in enumerate(colors):self.assertEqual(result.getpixel((i*160+80,50)),color)
            self.assertEqual(images.sheet(row,dict(size=[16,8],frames=[dict(phase="synthetic")]),[0,0,.5,1]).size,(640,54))

    def test_working_set_counter_on_cpu_process(self):
        memory=WorkingSet(os.getpid())
        try:
            sample=memory.sample()
            self.assertGreater(sample["working_set_bytes"],0)
            self.assertGreaterEqual(sample["peak_working_set_bytes"],sample["working_set_bytes"])
        finally:memory.close()


if __name__=="__main__":unittest.main()
