"""CPU checks for measured-row alignment and the packaged motion schedule."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import json
import os
import time

spec = importlib.util.spec_from_file_location("quarry_e2e", Path(__file__).with_name("quarry-e2e.py"))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class EvidenceTests(unittest.TestCase):
    def test_authorized_pressure_uses_available_counter_and_preserves_default_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            record = dict(games=[], foreign_renderers=[], free_gib=5.9, available_gib=6.1)
            with patch.object(bench, "state", return_value=record):
                with self.assertRaises(SystemExit):
                    bench.preflight(Path(directory) / "default.json")
                accepted = bench.preflight(Path(directory) / "pressure.json", memory_pressure=True)
                self.assertEqual(accepted["launch_memory_metric"], "available_gib")
                self.assertEqual(accepted["launch_floor_gib"], 6)

    def test_pressure_abort_terminates_only_the_owned_gpu_child(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            processes = [dict(ProcessId=12345, ParentProcessId=os.getpid(), Name="e2e-tests.exe"),
                         dict(ProcessId=54321, ParentProcessId=42, Name="e2e-tests.exe")]
            with patch.object(bench, "available_gib", return_value=2.9), \
                    patch.object(bench, "powershell", return_value=json.dumps(processes)), \
                    patch.object(bench.subprocess, "run") as command:
                with self.assertRaisesRegex(RuntimeError, "contention monitor failed"):
                    with bench.monitor(out, memory_abort_gib=3):
                        time.sleep(.4)
                command.assert_called_once_with(["taskkill", "/PID", "12345", "/T", "/F"],
                                                capture_output=True, creationflags=bench.FLAGS)
            record = json.loads((out / "contention.json").read_text())
            self.assertEqual(record["memory_samples"][0]["available_gib"], 2.9)
            self.assertEqual(len(record["failures"]), 1)

    def test_growing_csv_header_keeps_numeric_columns_and_fixed_motion_block(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.csv"
            lines = ["EVENTS,View/PosX,View/PosY,View/PosZ,FrameTime"]
            lines += [f",-620,{520+i%2},340,2.5" for i in range(160)]
            lines += ["EVENTS,View/PosX,View/PosY,View/PosZ,FrameTime,GPUTime", "[Metadata],ignored"]
            path.write_text("\n".join(lines), encoding="utf-8")
            rows = bench.csv_rows(path)
            self.assertEqual(len(rows), 160)
            self.assertEqual(rows[159]["FrameTime"], 2.5)
            self.assertEqual(rows[159]["GPUTime"], 0)
            check = bench.verify_motion(path, "camera")
            self.assertEqual(check["camera_y"], [520., 521.] * 32)
            path.write_text(path.read_text().replace(",521,", ",520,"))
            with self.assertRaisesRegex(ValueError, "did not alternate"):
                bench.verify_motion(path, "camera")

    def test_struct_arguments_stay_in_exec_files_and_motion_never_cuts(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            packaged = out / "package/Engine/Binaries/Win64/UnrealGame.exe"
            with patch.object(bench, "PACKAGE", packaged):
                events = bench.ue_events(out, "camera", False)
            moves = [(frame, command) for frame, command in events if command.startswith("exec ")]
            self.assertEqual([frame for frame, _ in moves], list(range(160)))
            self.assertTrue(all("," not in command for _, command in moves))
            command = (out / "camera-1.txt").read_text()
            self.assertIn("Y=521", command)
            self.assertIn("false () false", command)
            self.assertNotIn("cut", command.lower())
            with patch.object(bench, "PACKAGE", packaged):
                bench.ue_events(out, "sun", False)
            self.assertIn("K2_SetActorRotation", (out / "sun-0.txt").read_text())
            self.assertIn("Yaw=-52.130102354", (out / "sun-1.txt").read_text())

    def test_native_gpu_tree_accepts_compact_and_wide_engine_formats(self):
        profiler = bench.load("ue-quarry-profile")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ue.log"
            path.write_text("┃ 0.100 ms ┃ 1.200 ms ┃ compact ┃\n"
                            "┃ 0 ┃ 1 ┃ 2 ┃ 3 ┃ 10% ┃ 0.200 ms ┃ 4 ┃ 5 ┃ 6 ┃ 100% ┃ 2.300 ms ┃ wide ┃\n", encoding="utf-8")
            tree = profiler.log_evidence(path)["gpu_tree"]
            self.assertEqual([r["inclusive_ms"] for r in tree], [1.2, 2.3])
            self.assertEqual(tree[1]["exclusive_ms"], .2)
            self.assertEqual(tree[1]["name"].strip(), "wide")

    def test_statistics_retain_tail_and_interpolate_quantiles(self):
        row = bench.stats([4., 1., 2., 3.])
        self.assertEqual(row["n"], 4)
        self.assertEqual(row["median"], 2.5)
        self.assertEqual(row["maximum"], 4)
        self.assertAlmostEqual(row["p95"], 3.85)


if __name__ == "__main__":
    unittest.main()
