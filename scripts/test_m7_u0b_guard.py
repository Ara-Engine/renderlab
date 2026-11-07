"""CPU-only regression for the observed readiness/acquisition race. No GPU launch."""
import contextlib
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location("host",Path(__file__).with_name("m7-u0b.py"))
host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)


class LeaseTests(unittest.TestCase):
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
