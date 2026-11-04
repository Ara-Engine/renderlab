import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location("resize_causal", Path(__file__).with_name("resize-causal.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class CausalObservationTests(unittest.TestCase):
    def setUp(self):
        self.observation = probe.Observation(250)

    def feed(self, line, ns=0):
        self.observation.feed((line + "\n").encode(), ns)

    def initial(self):
        self.feed("surface_probe version=1 pid=123 epoch_us=1000")
        self.feed("surface_probe begin=0 phase=configure at_us=0")
        self.feed("surface_probe end=0 phase=configure at_us=600000 duration_us=600000 outcome=returned")

    def test_short_initial_setup_is_excluded_but_first_open_resize_triggers_once(self):
        self.feed("surface_probe begin=0 phase=configure at_us=0")
        self.assertIsNone(self.observation.check_slow(900_000_000))
        self.feed("surface_probe end=0 phase=configure at_us=900000 duration_us=900000 outcome=returned")
        self.feed("surface_probe begin=1 phase=configure at_us=900010", 900_000_000)
        self.assertIsNone(self.observation.check_slow(1_149_000_000))
        trigger = self.observation.check_slow(1_151_000_000)
        self.assertEqual(trigger["kind"], "still_open")
        self.assertEqual(trigger["id"], 1)
        self.assertIs(self.observation.check_slow(9_000_000_000), trigger)

    def test_initial_configure_over_one_second_is_still_captured(self):
        self.feed("surface_probe begin=0 phase=configure at_us=0")
        self.assertEqual(self.observation.check_slow(1_001_000_000)["id"], 0)

    def test_completed_slow_configure_stops_even_when_poll_missed_it(self):
        self.initial()
        self.feed("surface_probe begin=1 phase=configure at_us=600010")
        self.feed("surface_probe end=1 phase=configure at_us=900020 duration_us=300000 outcome=returned")
        self.assertEqual(self.observation.trigger["kind"], "returned_slow")
        self.assertEqual(self.observation.trigger["duration_ms"], 300)

    def test_owned_context_and_unfinished_span_survive_eviction(self):
        self.initial()
        self.feed("frame=42 revision=3 prep_ms=1")
        self.feed("configure_begin width=984 height=661 flights=0 pending_frame=false wait_ms=5.5 deferrals=2")
        self.feed("surface_probe begin=1 phase=configure at_us=600010")
        for _ in range(400):
            self.feed("discardable" * 1024)
        report = self.observation.report()
        context = report["unfinished_calls"][0]["context"]
        self.assertEqual(context["last_present"], dict(frame=42, revision=3))
        self.assertIn("flights=0 pending_frame=false", context["text"])
        self.assertLessEqual(report["retained_bytes"], 1280 * 1024)
        self.assertGreater(report["dropped_tail_lines"], 0)

    def test_wrong_phase_preserves_unfinished_call_and_marks_error(self):
        self.initial()
        self.feed("surface_probe begin=1 phase=configure at_us=600010")
        self.feed("surface_probe end=1 phase=present at_us=900020 duration_us=300000 outcome=ok")
        report = self.observation.report()
        self.assertEqual(report["unfinished_calls"][0]["phase"], "configure")
        self.assertTrue(report["errors"])
        self.assertEqual(len(report["configures"]), 1)

    def test_acquire_timeout_and_backpressure_do_not_disappear_without_frame(self):
        self.feed("surface_probe begin=0 phase=acquire at_us=0")
        self.feed("surface_probe end=0 phase=acquire at_us=400000 duration_us=400000 outcome=timeout")
        self.feed("caller_ms=0.01 render_whole_ms=1.5 accepted=false")
        self.feed("reload_rejected compile_ms=1 diagnostic=Validation Error")
        report = self.observation.report()
        self.assertEqual(report["phases"]["acquire"]["max_ms"], 400)
        self.assertEqual(report["phases"]["acquire"]["outcomes"], {"timeout": 1})
        self.assertEqual(report["backpressure_rejections"], 1)
        self.assertIsNone(report["trigger"])
        self.assertFalse(report["errors"])
        self.feed("gpu_diagnostic ERROR wgpu_hal::vulkan::instance real failure")
        self.assertTrue(self.observation.errors)


if __name__ == "__main__":
    unittest.main()
