import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("resize_followup", Path(__file__).with_name("resize-followup.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ResizeEvidenceTests(unittest.TestCase):
    def summarize(self, text):
        return probe.summarize(text.strip().splitlines())

    def test_legacy_frame_timing_is_not_a_second_configure(self):
        report = self.summarize("""
configure_ms=190.000 width=1920 height=1080
configure_ms=4933.549 width=984 height=661
frame=1738 revision=2 acquire_ms=0.1552 present_ms=0.2868 configure_ms=4933.5492
""")
        self.assertEqual(report["configure_count"], 2)
        self.assertEqual(report["post_initial_slow_configures"][0]["ms"], 4933.549)
        self.assertIsNone(report["configures"][1]["begin"])
        self.assertEqual(report["probe_status"], "absent")

    def test_nested_slow_acquire_timeout_survives_without_presented_frame(self):
        report = self.summarize("""
surface_probe version=1 pid=1 epoch_us=0
surface_probe begin=0 phase=redraw at_us=10
surface_probe begin=1 phase=acquire at_us=20
surface_probe end=1 phase=acquire at_us=500030 duration_us=500000 outcome=timeout
surface_probe end=0 phase=redraw at_us=500040 duration_us=500020 outcome=ok
""")
        self.assertEqual(report["probe_status"], "balanced")
        self.assertEqual(report["phases"]["acquire"]["outcomes"], {"timeout": 1})
        self.assertEqual(report["phases"]["acquire"]["max_ms"], 500)
        self.assertEqual([s["phase"] for s in report["slow_calls"]], ["redraw", "acquire"])

    def test_truncation_retains_innermost_active_call_without_claiming_deadlock(self):
        report = self.summarize("""
surface_probe version=1 pid=1 epoch_us=0
surface_probe begin=0 phase=redraw at_us=10
surface_probe begin=1 phase=configure at_us=20
""")
        self.assertEqual(report["probe_status"], "incomplete")
        self.assertEqual(report["unfinished_calls"][-1]["phase"], "configure")
        self.assertIn("truncated log", report["causality"])

    def test_wrong_phase_cannot_be_reported_as_a_balanced_probe(self):
        report = self.summarize("""
surface_probe version=1 pid=1 epoch_us=0
surface_probe begin=0 phase=acquire at_us=10
surface_probe end=0 phase=present at_us=30 duration_us=10 outcome=ok
""")
        self.assertEqual(report["probe_status"], "malformed")
        self.assertEqual(report["parse_error_count"], 1)
        self.assertEqual(report["phases"], {})


if __name__ == "__main__":
    unittest.main()
