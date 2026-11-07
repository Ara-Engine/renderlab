"""CPU-only regression for the observed readiness/acquisition race. No GPU launch."""
import contextlib
import importlib.util
from pathlib import Path
import unittest
import tempfile
import json
import subprocess
import sys
from unittest.mock import patch

spec=importlib.util.spec_from_file_location("host",Path(__file__).with_name("m7-u0b.py"))
host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)


class LeaseTests(unittest.TestCase):
    def test_native_memory_inventory_and_counter_failure_are_safe(self):
        module=host.load("m7-u0-ue")
        with patch.object(host,"load",return_value=module),\
             patch.object(module.E2E,"powershell",side_effect=AssertionError("memory must not query WMI")),\
             patch.object(module.E2E,"available_gib",side_effect=OSError("injected PDH failure")):
            wrapped=host.ue(allow_game=True)
            memory=json.loads(wrapped.E2E.powershell("Get-CimInstance Win32_OperatingSystem"))
            self.assertGreater(memory["TotalVisibleMemorySize"],0)
            self.assertGreaterEqual(memory["FreePhysicalMemory"],0)
            self.assertEqual(wrapped.E2E.available_gib(),0)

    def test_conflict_monitor_timeout_rejects_owned_cpu_job(self):
        module=host.load("m7-u0-ue")
        scratch=host.OUT/"cpu-tests";scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);module.OUT=root
            with patch.object(module,"preflight"),\
                 patch.object(module.E2E,"available_gib",return_value=8),\
                 patch.object(module.E2E,"state",side_effect=subprocess.TimeoutExpired("injected inventory",30)):
                with self.assertRaisesRegex(RuntimeError,"conflict monitor unavailable"):
                    module.guarded([sys.executable,"-B","-c","import time; time.sleep(10)"],root,timeout=15)
            record=json.loads((root/"process.json").read_text())
            self.assertNotEqual(record["exit_code"],0)
            self.assertEqual(record["failures"][0]["reason"],"conflict monitor unavailable")

    def test_lost_atomic_acquisition_retries_but_owned_failure_propagates(self):
        module=host.load("m7-u0-ue");attempts=[];bodies=[]

        @contextlib.contextmanager
        def atomic(label,minutes):
            attempts.append(label)
            if len(attempts)==1:raise SystemExit(3)
            yield {"token":"cpu-fixture"}

        module.lease=atomic
        with patch.object(host,"load",return_value=module),\
             patch.object(host,"game_state",return_value="released"),\
             patch.object(host,"cpu_readiness",return_value={"available_gib":8}),\
             patch.object(Path,"exists",return_value=False),\
             patch.object(host.time,"sleep"):
            wrapped=host.ue(allow_game=True)
            with wrapped.lease("lost-race",1) as owner:
                bodies.append(owner["token"])
            self.assertEqual(attempts,["lost-race","lost-race"])
            self.assertEqual(bodies,["cpu-fixture"])
            # The same exit code from inside an acquired job is a real failure,
            # never permission to run its body again.
            with self.assertRaises(SystemExit) as error:
                with wrapped.lease("owned-failure",1):
                    raise SystemExit(3)
            self.assertEqual(error.exception.code,3)
            self.assertEqual(attempts,["lost-race","lost-race","owned-failure"])


if __name__=="__main__":unittest.main()
