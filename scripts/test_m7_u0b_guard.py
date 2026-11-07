"""CPU-only regression for the observed readiness/acquisition race. No GPU launch."""
import contextlib
import importlib.util
from pathlib import Path
import unittest
import tempfile
import json
import subprocess
import sys
import os
from types import SimpleNamespace
from unittest.mock import patch

spec=importlib.util.spec_from_file_location("host",Path(__file__).with_name("m7-u0b.py"))
host=importlib.util.module_from_spec(spec);spec.loader.exec_module(host)


class LeaseTests(unittest.TestCase):
    def test_native_process_inventory_keeps_identity_and_rejects_query_errors(self):
        rows=host.native_process_inventory()
        current=next(p for p in rows if p["ProcessId"]==os.getpid())
        self.assertEqual(current["ParentProcessId"],os.getppid())
        self.assertEqual(Path(current["ExecutablePath"]).resolve(),Path(sys.executable).resolve())
        module=host.load("m7-u0-ue")
        with patch.object(host,"load",return_value=module),\
             patch.object(host,"native_process_inventory",side_effect=OSError("injected native snapshot failure")):
            wrapped=host.ue(allow_game=True)
            snapshot=wrapped.MONITOR_STATE()
            self.assertEqual(snapshot["available_gib"],0)
            self.assertEqual(snapshot["foreign_renderers"][0]["Name"],"u0b-monitor-error")
            self.assertIn("native snapshot failure",snapshot["monitor_error"])
        processes=[dict(Name=name,ProcessId=pid,ParentProcessId=0,ExecutablePath=path)
            for name,pid,path in (("cs2.exe",2000000001,"E:/Steam/steamapps/common/cs2/cs2.exe"),
                ("reference-tests.exe",2000000002,"E:/renderlab-work/foreign/reference-tests.exe"),
                ("cargo.exe",2000000003,"C:/tools/cargo.exe"))]
        with patch.object(host,"native_process_inventory",return_value=rows+processes),\
             patch.object(host.subprocess,"run",return_value=subprocess.CompletedProcess([],0,"GPU inventory","")):
            actual=wrapped.E2E.state()
        self.assertIn(2000000001,[p["ProcessId"] for p in actual["games"]])
        self.assertIn(2000000002,[p["ProcessId"] for p in actual["foreign_renderers"]])
        self.assertIn(2000000003,[p["ProcessId"] for p in actual["builds"]])

    def test_compiler_valve_is_timing_only_while_ram_floor_stays_mandatory(self):
        scratch=host.OUT/"cpu-tests";scratch.mkdir(exist_ok=True)
        class Waiting(Exception):pass
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);(root/"resource-pause.json").write_text('{}')
            memory=SimpleNamespace(available_gib=lambda:8.)
            module=SimpleNamespace(START_GIB=4.,FLAGS=0,E2E=memory,
                save=lambda p,v:p.write_text(json.dumps(v)))
            with patch.object(host,"OUT",root),\
                 patch.object(host,"native_process_inventory",return_value=[dict(Name="cargo.exe")]),\
                 patch.object(host.time,"sleep",side_effect=Waiting):
                record=host.cpu_readiness(module,timing=False)
                self.assertFalse(record["waiting"])
                self.assertFalse(record["cargo_free_required"])
                self.assertEqual(record["foreign_compilers"],["cargo.exe"])
                with self.assertRaises(Waiting):host.cpu_readiness(module,timing=True)
                memory.available_gib=lambda:3.99
                with self.assertRaises(Waiting):host.cpu_readiness(module,timing=False)
                memory.available_gib=lambda:8.
                with patch.object(host,"native_process_inventory",side_effect=OSError("injected inventory failure")):
                    with self.assertRaises(Waiting):host.cpu_readiness(module,timing=False)
        module=host.load("m7-u0-ue")
        with patch.object(host,"load",return_value=module),\
             patch.object(module.E2E,"state",side_effect=lambda:dict(games=[],foreign_renderers=[],builds=[dict(Name="cargo.exe")])):
            wrapped=host.ue(allow_game=True)
            self.assertNotIn("monitor_error",wrapped.MONITOR_STATE())
            wrapped.WAIT_FOR_FOREIGN_CPU_CLEAR=True
            self.assertIn("quiet timing",wrapped.MONITOR_STATE()["monitor_error"])

    def test_short_static_lease_requeues_every_unfinished_configuration(self):
        scratch=host.OUT/"cpu-tests";scratch.mkdir(exist_ok=True);calls=[]
        class CaptureComplete(Exception):pass
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary)
            def capture(args):
                calls.append(args.configs)
                config=args.configs[0]
                case=root/"study/floor-static"/config;case.mkdir(parents=True)
                for name,record in (("native",{}),("camera-readback",dict(all_ok=True)),
                        ("readback",dict(all_ok=True,config=config)),("process",dict(exit_code=0))):
                    (case/(name+".json")).write_text(json.dumps(record))
            def score(args):
                self.assertEqual({p.name for p in args.cases},{"a","b","c"})
                raise CaptureComplete()
            with patch.object(host,"OUT",root),\
                 patch.object(host,"ue",return_value=SimpleNamespace(CONFIGS=dict.fromkeys(("a","b","c")))),\
                 patch.object(host,"study",side_effect=capture),\
                 patch.object(host,"load",return_value=SimpleNamespace(static_floor=score)),\
                 patch.object(host.time,"sleep"):
                with self.assertRaises(CaptureComplete):host.floors(SimpleNamespace())
        self.assertEqual(calls,[["a","b","c"],["b","c"],["c"]])

    def test_live_poll_omits_optional_gpu_telemetry_but_keeps_process_inventory(self):
        module=host.load("m7-u0-ue");inventory=[]
        def snapshot():
            gpu=module.E2E.subprocess.run(["nvidia-smi","--query-gpu=name"],timeout=10)
            inventory.append(module.E2E.powershell("Get-CimInstance Win32_Process"))
            return dict(games=[],foreign_renderers=[],gpu=gpu.stdout)
        with patch.object(host,"load",return_value=module),\
             patch.object(module.E2E,"state",side_effect=snapshot),\
             patch.object(module.E2E,"powershell",return_value="live process inventory"),\
             patch.object(host.subprocess,"run",return_value=subprocess.CompletedProcess([],0,"actual GPU inventory","")) as command:
            wrapped=host.ue(allow_game=True)
            self.assertEqual(wrapped.E2E.state()["gpu"],"actual GPU inventory")
            self.assertEqual(wrapped.MONITOR_STATE()["gpu"],"not sampled by live conflict guard")
            self.assertEqual(command.call_count,1)
            self.assertEqual(inventory,["live process inventory"]*2)
        with patch.object(module.E2E,"powershell",side_effect=OSError("injected process inventory failure")):
            failed=wrapped.MONITOR_STATE()
        self.assertEqual(failed["available_gib"],0)
        self.assertEqual(failed["foreign_renderers"][0]["Name"],"u0b-monitor-error")

    def test_late_monitor_error_rejects_even_a_successful_child(self):
        module=host.load("m7-u0-ue")
        scratch=host.OUT/"cpu-tests";scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as temporary:
            root=Path(temporary);module.OUT=root;signal=root/"release-child";children=[]
            original=module.subprocess.Popen
            def spawn(*a,**kw):
                child=original(*a,**kw);children.append(child);return child
            def snapshot():
                signal.touch();children[0].wait(timeout=5)
                return dict(games=[],foreign_renderers=[],monitor_error="injected inventory error after child exit")
            with patch.object(module,"preflight"),\
                 patch.object(module.E2E,"available_gib",return_value=8),\
                 patch.object(module.E2E,"state",side_effect=snapshot),\
                 patch.object(module.subprocess,"Popen",side_effect=spawn):
                script="import pathlib,sys,time\np=pathlib.Path(sys.argv[1])\nwhile not p.exists(): time.sleep(.01)"
                with self.assertRaisesRegex(RuntimeError,"conflict monitor unavailable"):
                    module.guarded([sys.executable,"-B","-c",script,str(signal)],root,timeout=8)
            record=json.loads((root/"process.json").read_text())
            self.assertEqual(record["exit_code"],0)
            self.assertEqual(record["failures"][0]["reason"],"conflict monitor unavailable")

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
            # A saturated host can deliver the inventory result after a short
            # child has naturally exited. Either way, the job must be rejected.
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
