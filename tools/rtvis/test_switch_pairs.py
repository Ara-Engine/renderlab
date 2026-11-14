import unittest

from switch_pairs import environment, sets


class SwitchEnvironments(unittest.TestCase):
    def test_default_really_omits_the_override(self):
        for inherited in ["0", "1", ""]:
            env = environment({"RENDERLAB_LIGHT_VIS_PASS": inherited}, "default", {})
            self.assertNotIn("RENDERLAB_LIGHT_VIS_PASS", env)
        self.assertIn(("default", {}), list(sets()))

    def test_all_off_and_explicit_on_are_separate_cases(self):
        off = environment({}, "none", {})
        on = environment({}, "lvis", {"RENDERLAB_LIGHT_VIS_PASS": "1"})
        self.assertEqual(off["RENDERLAB_LIGHT_VIS_PASS"], "0")
        self.assertEqual(on["RENDERLAB_LIGHT_VIS_PASS"], "1")


if __name__ == "__main__":
    unittest.main()
