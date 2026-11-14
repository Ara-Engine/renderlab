"""Small layout/counter check, not real-ray evidence or shader bit certification."""
import json
from pathlib import Path
import tempfile
import unittest
import numpy as np
from replay_geometry import Geometry, LEAF
from glossy_to_rays import convert, directions


class ReplayCheck(unittest.TestCase):
    def test_glossy_layout_epsilon_and_pair_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            stem = Path(directory)/'fixture'
            geometry = stem.with_suffix('.json')
            geometry.write_text(json.dumps(dict(frame=30, view=4, generation=7)))
            meta = dict(version=1, path='nonrt', frame=30, view=4, generation=7,
                        primary_frame_sided_y=0, room=8, size=[32,32],
                        appended=2, entries=1, capacity=1, entry_bytes=48, header_bytes=16)
            glossy = Path(str(stem)+'.glossy.json')
            glossy.write_text(json.dumps(meta))
            words = np.zeros(16, dtype='<u4')
            words[0] = 2
            words[4:7] = np.array([1,2,3], dtype='<f4').view('<u4')
            words[11] = 4<<16
            words.tofile(Path(str(stem)+'.glossy.bin'))
            np.array([.0125], dtype='<f4').tofile(Path(str(stem)+'.glossy-epsilon.bin'))
            output = Path(directory)/'rays.jsonl'
            stats = convert(geometry, glossy, output)
            self.assertEqual((stats['slots'], stats['rays'], stats['overflow_entries_not_listed']), (4,4,1))
            rays = [json.loads(line) for line in output.read_text().splitlines()]
            self.assertEqual(rays[0]['origin'], [1,2,3])
            self.assertEqual(rays[0]['tmin'], float(np.float32(.0125)))
            self.assertTrue(all(ray['direction'][2] > .99 for ray in rays))
            entry = words[4:].copy()
            entry[6] = 0x7fff7fff  # oct -z: every mirror stratum falls below its side.
            self.assertTrue(all(not ok for _,_,ok in directions(entry, 0)))
            meta['frame'] = 31
            glossy.write_text(json.dumps(meta))
            with self.assertRaisesRegex(ValueError, 'frame mismatch'):
                convert(geometry, glossy, output)

    def test_flat_and_top_level_hits_and_misses(self):
        for top in (False, True):
            with self.subTest(top=top), tempfile.TemporaryDirectory() as directory:
                path = Path(directory)/'sample.json'
                words = np.zeros(84, dtype='<u4')
                floats = words.view('<f4')
                words[0] = 2
                if top:
                    words[1:3] = [56, 0]
                    floats[56:68] = [0, 0, 0, 1, 1, 0]*2
                    # Visit higher instance first: the lower-index equal-t hit wins.
                    words[68:70] = [LEAF | 1, LEAF]
                for i in range(2):
                    at = 8+i*24
                    floats[at:at+12] = np.eye(4, dtype='f4')[:3].reshape(-1)
                    floats[at+12:at+18] = [0, 0, 0, 1, 1, 0]
                    words[at+18:at+22] = [72, 72, LEAF | 1, 7+i]
                floats[72:81] = [0, 0, 0, 1, 0, 0, 0, 1, 0]
                words[81] = 42
                words.tofile(path.with_suffix('.words.bin'))
                path.write_text(json.dumps(dict(
                    version=1, format='reflection-bvh2-u32-le',
                    path='nonrt-software-reflections', hardware_acceleration_structures=False,
                    header_words=8, instance_words=24, payload_words=len(words),
                    frame=30, view=0, generation=1, written_instances=2, reserved_instances=2,
                    tlas=dict(base_word=56 if top else 0, root=0, refits=0),
                    source_draws=[dict(placements=[dict(included=True)]*2)],
                    meshes=[dict(mesh=0, root=LEAF | 1, node_base_word=72,
                                 triangle_base_word=72, nodes=0, triangles=1, depth=0)],
                )))
                geometry = Geometry(path)
                self.assertEqual(geometry.inventory()['mesh_instances'], {0:2})
                hit = geometry.trace([.25, .25, 1], [0, 0, -1], .001, 1e6)
                self.assertEqual(hit['record'][:2], [8, 42])
                self.assertEqual(hit['t'], 1.)
                self.assertEqual(hit['triangle_tests'], 2)
                self.assertEqual(hit['blas_visits'], 2)
                miss = geometry.trace([2, 2, 1], [0, 0, -1], .001, 1e6)
                self.assertEqual(miss['record'][0], 0)
                self.assertEqual(miss['triangle_tests'], 0)
                # Release memmap before TemporaryDirectory removes the test payload on Windows.
                del geometry.f
                geometry.w._mmap.close()


if __name__ == '__main__':
    unittest.main()
