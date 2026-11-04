"""Bound one owned, inactive quarry run and snapshot its first slow configure.

No build, input injection, desktop capture, API interception or global settings.
CDB snapshots suspend only the owned viewer. Snapshot-affected durations are not
performance samples. The installed API dump cannot pre-log blocking Vulkan APIs.
"""
import argparse
from collections import deque
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import threading
import time


class Tail:
    def __init__(self, capacity):
        self.capacity, self.size, self.dropped = capacity, 0, 0
        self.lines = deque()

    def append(self, line):
        line = line[-self.capacity:]
        self.lines.append(line)
        self.size += len(line)
        while self.size > self.capacity:
            self.size -= len(self.lines.popleft())
            self.dropped += 1


class Observation:
    """Receipt clocks trigger capture; viewer monotonic clocks measure spans."""
    def __init__(self, slow_ms):
        self.slow_ms = slow_ms
        self.tail, self.important = Tail(1024 * 1024), Tail(256 * 1024)
        self.session, self.active, self.phases = {}, {}, {}
        self.configures, self.errors = [], []
        self.context, self.last_frame = None, None
        self.frames = self.rejected = self.first_scene = 0
        self.ready = self.validation_clean = False
        self.trigger = None
        self.lines = 0

    def feed(self, raw, received_ns):
        self.lines += 1
        self.tail.append(raw)
        line = raw.decode("utf-8", errors="replace").strip()
        if line.startswith(("surface_probe version=", "configure_", "resize_requested=",
                            "virtual_first_", "gpu_validation_", "viewer_", "adapter=",
                            "timing_status=", "effective_")):
            self.important.append(raw)
        if line.startswith("timing_status="):
            self.ready = True
        if match := re.match(r"frame=(\d+) revision=(\d+) ", line):
            self.last_frame = dict(frame=int(match[1]), revision=int(match[2]))
            self.frames += 1
        self.rejected += int(line.startswith("caller_ms=") and "accepted=false" in line)
        self.first_scene += int(line.startswith("virtual_first_scene_submitted_us="))
        if line.startswith("configure_begin "):
            self.context = dict(text=line, last_present=self.last_frame, received_ns=received_ns,
                                fields=dict(re.findall(r"(\w+)=(\S+)", line)))
        if "gpu_validation_summary errors=0 missing_layer=0 active_layer=1" in line:
            self.validation_clean = True
        if (line.startswith(("viewer_stopped", "gpu_diagnostic ERROR")) or "gpu_validation_error" in line
                or "panicked at" in line):
            self.important.append(raw)
            if len(self.errors) < 16:
                self.errors.append(line[:4096])
        if not line.startswith("surface_probe "):
            return
        fields = dict(re.findall(r"(\w+)=(\S+)", line))
        try:
            if "version" in fields:
                if self.session or fields["version"] != "1":
                    raise ValueError("expected one version=1 session")
                self.session = fields
            elif "begin" in fields:
                identifier = int(fields["begin"])
                if identifier in self.active:
                    raise ValueError("duplicate active span")
                span = dict(id=identifier, phase=fields["phase"], at_us=int(fields["at_us"]),
                            received_ns=received_ns, line=self.lines)
                if span["phase"] == "configure":
                    span.update(index=len(self.configures), context=self.context)
                    self.important.append(raw)
                self.active[identifier] = span
            elif "end" in fields:
                identifier = int(fields["end"])
                span = self.active[identifier]
                at_us, duration_us = int(fields["at_us"]), int(fields["duration_us"])
                if fields["phase"] != span["phase"] or not 0 <= duration_us <= at_us - span["at_us"]:
                    raise ValueError("span clock or identity mismatch")
                del self.active[identifier]
                phase = self.phases.setdefault(span["phase"], dict(count=0, max_ms=0, outcomes={}))
                phase["count"] += 1
                phase["max_ms"] = max(phase["max_ms"], duration_us / 1000)
                outcome = fields["outcome"]
                if outcome in ("error", "lost", "validation") and len(self.errors) < 16:
                    self.errors.append(line)
                phase["outcomes"][outcome] = phase["outcomes"].get(outcome, 0) + 1
                if span["phase"] == "configure":
                    self.important.append(raw)
                    complete = dict(**span, end_us=at_us, duration_ms=duration_us / 1000,
                                    outcome=outcome, frames_at_return=self.frames)
                    self.configures.append(complete)
                    threshold_ms = self.slow_ms if span["index"] > 0 else 1000
                    if duration_us >= threshold_ms * 1000 and self.trigger is None:
                        self.trigger = dict(**complete, kind="returned_slow", triggered_ns=received_ns)
        except (KeyError, ValueError) as error:
            if len(self.errors) < 16:
                self.errors.append(f"probe line {self.lines}: {error}")

    def check_slow(self, now_ns):
        if self.trigger is None:
            for span in self.active.values():
                threshold_ms = self.slow_ms if span.get("index", 0) > 0 else 1000
                if (span["phase"] == "configure"
                        and now_ns - span["received_ns"] >= threshold_ms * 1e6):
                    self.trigger = dict(**span, kind="still_open", triggered_ns=now_ns,
                                        receipt_elapsed_ms=(now_ns - span["received_ns"]) / 1e6)
                    break
        return self.trigger

    def report(self):
        return dict(session=self.session, configures=self.configures, trigger=self.trigger,
                    unfinished_calls=list(self.active.values()), phases=self.phases,
                    errors=self.errors, presented_frames=self.frames, last_present=self.last_frame,
                    backpressure_rejections=self.rejected, first_scene_submissions=self.first_scene,
                    validation_clean=self.validation_clean, lines_seen=self.lines,
                    retained_bytes=self.tail.size + self.important.size,
                    dropped_tail_lines=self.tail.dropped)


class OwnedWindow:
    def __init__(self, process):
        self.process = process
        self.u = c.WinDLL("user32", use_last_error=True)
        self.u.SetProcessDPIAware()
        signatures = {
            "GetWindowThreadProcessId": ([w.HWND, c.POINTER(w.DWORD)], w.DWORD),
            "GetWindowTextW": ([w.HWND, w.LPWSTR, c.c_int], c.c_int),
            "GetForegroundWindow": ([], w.HWND),
            "SetWindowPos": ([w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT], w.BOOL),
            "ShowWindowAsync": ([w.HWND, c.c_int], w.BOOL),
            "IsIconic": ([w.HWND], w.BOOL),
            "GetClientRect": ([w.HWND, c.POINTER(w.RECT)], w.BOOL),
            "PostMessageW": ([w.HWND, w.UINT, w.WPARAM, w.LPARAM], w.BOOL),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.u, name)
            function.argtypes, function.restype = args, result
        self.callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
        self.u.EnumWindows.argtypes = [self.callback, w.LPARAM]
        self.u.EnumWindows.restype = w.BOOL
        self.hwnd = None

    def find(self):
        found = []

        def visit(hwnd, unused):
            pid, title = w.DWORD(), c.create_unicode_buffer(256)
            self.u.GetWindowThreadProcessId(hwnd, c.byref(pid))
            if pid.value == self.process.pid:
                self.u.GetWindowTextW(hwnd, title, len(title))
                if title.value.startswith("Renderlab |"):
                    found.append(hwnd)
            return True

        self.u.EnumWindows(self.callback(visit), 0)
        if len(found) > 1:
            raise RuntimeError("multiple owned viewer windows")
        self.hwnd = found[0] if found else None
        return self.hwnd

    def check(self):
        pid = w.DWORD()
        tid = self.u.GetWindowThreadProcessId(self.hwnd, c.byref(pid))
        if self.process.poll() is not None or pid.value != self.process.pid:
            raise RuntimeError("owned HWND identity changed")
        if self.u.GetForegroundWindow() == self.hwnd:
            raise RuntimeError("owned viewer became foreground")
        return dict(pid=pid.value, hwnd=int(self.hwnd), window_tid=tid,
                    foreground_hwnd=self.u.GetForegroundWindow())

    def resize(self, width, height):
        self.check()
        # Preserve position and z-order; never activate. The supervisor stays
        # independent if this cross-thread Win32 call waits for the viewer.
        if not self.u.SetWindowPos(self.hwnd, None, 0, 0, width, height, 0x16):
            raise c.WinError(c.get_last_error())
        rect = w.RECT()
        if not self.u.GetClientRect(self.hwnd, c.byref(rect)):
            raise c.WinError(c.get_last_error())
        return dict(width=str(rect.right - rect.left), height=str(rect.bottom - rect.top))

    def minimize(self, minimized):
        self.check()
        if not self.u.ShowWindowAsync(self.hwnd, 7 if minimized else 4):
            raise c.WinError(c.get_last_error())


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(args):
    if os.name != "nt":
        raise RuntimeError("native run requires Windows")
    root = Path(__file__).resolve().parent.parent
    output = args.output.resolve()
    if output.drive.upper() != "D:":
        raise RuntimeError("evidence must stay on D:")
    if Path("D:/AraBuild/render-v3/quiet-window.lock").exists():
        raise RuntimeError("another GPU quiet window is active")
    for path in (args.viewer, args.cdb, args.virtual_manifest, args.cameras, args.limits):
        if not path.is_file():
            raise FileNotFoundError(path)
    if not args.cache.is_dir():
        raise FileNotFoundError(args.cache)
    if args.owned_capture and not args.owned_capture.is_file():
        raise FileNotFoundError(args.owned_capture)
    output.mkdir(parents=True, exist_ok=False)
    cameras = json.loads(args.cameras.read_text())
    cameras["frames"] = cameras["frames"][:1]
    held = output / "held-camera.json"
    held.write_text(json.dumps(cameras))
    command = [str(args.viewer.resolve()), "--background", "--trace-surface", "--log-frames",
               "--validate", "--samples", "4", "--anisotropy", "16", "--seconds", "110",
               "--virtual-manifest", str(args.virtual_manifest.resolve()), "--cache", str(args.cache.resolve()),
               "--cameras", str(held), "--limits", str(args.limits.resolve()), "--progressive",
               "--io-delay-ms", "1500", "--shader", str(root / "shaders/scene.wgsl")]
    implicit = output / "empty-vulkan-implicit"
    implicit.mkdir()
    env = os.environ.copy()
    env.update(VK_IMPLICIT_LAYER_PATH=str(implicit), TEMP=str(output), TMP=str(output),
               _NT_SYMBOL_PATH="", _NT_ALT_SYMBOL_PATH="")
    env.pop("VK_INSTANCE_LAYERS", None)
    state, lock, stop = Observation(args.slow_ms), threading.Lock(), threading.Event()
    started = time.monotonic()
    deadline = started + args.deadline
    events, snapshots, action_errors = [], [], []
    process, window, reader, actor = None, None, None, None
    flags = subprocess.CREATE_NO_WINDOW | subprocess.BELOW_NORMAL_PRIORITY_CLASS

    def record(action, **fields):
        with lock:
            events.append(dict(action=action, monotonic_ns=time.monotonic_ns(), epoch_ns=time.time_ns(),
                               last_present=state.last_frame, configures=len(state.configures), **fields))

    def read():
        try:
            with process.stdout:
                while raw := process.stdout.readline(65536):
                    with lock:
                        state.feed(raw, time.monotonic_ns())
                        if raw.startswith(b"surface_probe version=") and int(state.session["pid"]) != process.pid:
                            state.errors.append("probe PID differs from owned process")
        except Exception as error:
            with lock:
                state.errors.append(f"log reader failed: {error!r}")

    def wait_for(predicate, timeout=12):
        end = min(deadline, time.monotonic() + timeout)
        while time.monotonic() < end and not stop.is_set():
            with lock:
                if predicate():
                    return
            if process.poll() is not None:
                raise RuntimeError("owned viewer exited during actions")
            stop.wait(0.025)
        raise TimeoutError("action wait cancelled or expired")

    def resize(width, height, presented):
        with lock:
            before = len(state.configures)
        record("resize_call_begin", width=width, height=height, **window.check())
        begin = time.monotonic_ns()
        client = window.resize(width, height)
        record("resize_call_end", elapsed_ms=(time.monotonic_ns() - begin) / 1e6, client=client)
        if presented:
            def recovered():
                return any(
                    span["context"] and all(span["context"]["fields"].get(k) == v for k, v in client.items())
                    and state.frames > span["frames_at_return"] for span in state.configures[before:])
            wait_for(recovered)
        record("resize_recovered" if presented else "loading_resize_sent")

    def minimized(value):
        record("minimize_call" if value else "restore_call", **window.check())
        window.minimize(value)
        wait_for(lambda: bool(window.u.IsIconic(window.hwnd)) == value)

    def snapshot(label):
        identity = window.check()
        commands = ".echo RESIZE_STACK_BEGIN; .time; ~* kv 40; lm; .echo RESIZE_STACK_END; q"
        debugger = [str(args.cdb.resolve()), "-pv", "-sins", "-noshell", "-nosqm",
                    "-y", str(args.viewer.resolve().parent) + ";" + str(args.sdk.resolve() / "Bin"),
                    "-p", str(process.pid), "-c", commands]
        record("snapshot_begin", label=label, **identity)
        info = dict(label=label, command=debugger, identity=identity, start_ns=time.monotonic_ns())
        with lock:
            info["configure_ids_open_before_attach"] = [s["id"] for s in state.active.values()
                                                       if s["phase"] == "configure"]
        with (output / f"{label}.stack.txt").open("wb") as stream:
            with subprocess.Popen(debugger, env=env, stdout=stream, stderr=stream,
                                  creationflags=flags) as tool:
                try:
                    tool.wait(timeout=max(0.1, min(8, deadline - time.monotonic())))
                except subprocess.TimeoutExpired:
                    tool.kill()
                    tool.wait(timeout=2)
                    info["timeout"] = True
                info["exit"] = tool.returncode
        info["end_ns"] = time.monotonic_ns()
        stack = (output / f"{label}.stack.txt").read_text(errors="replace")
        info["identity_and_stack_verified"] = bool(
            re.search(rf"\bId:\s*{process.pid:x}\.[0-9a-f]+\b", stack, re.I)
            and "Child-SP" in stack and "RESIZE_STACK_END" in stack and info["exit"] == 0)
        info["viewer_symbols_visible"] = bool(re.search(r"renderlab_viewer![^\s+]+", stack))
        info["window_thread_visible"] = bool(re.search(
            rf"\bId:\s*{process.pid:x}\.{identity['window_tid']:x}\b", stack, re.I))
        snapshots.append(info)
        record("snapshot_end", label=label, exit=info["exit"])

    def actions():
        try:
            wait_for(lambda: state.ready, timeout=65)
            wait_for(lambda: window.find() is not None)
            record("window_ready", **window.check())
            resize(940, 640, False)
            minimized(True)
            stop.wait(0.15)
            minimized(False)
            with lock:
                if state.frames or state.first_scene:
                    raise RuntimeError("loading actions missed startup backpressure interval")
            record("loading_actions_finished_before_first_scene")
            wait_for(lambda: state.frames > 0, timeout=30)
            record("startup_presented", **window.check())
            # A finite sequence samples loading, presented work and two minimize
            # cycles. It does not hunt a rare outlier with thousands of resizes.
            for width, height in ((1000, 700), (620, 900), (1100, 680), (760, 510)):
                resize(width, height, True)
            for width, height in ((984, 700), (1000, 700)):
                minimized(True)
                stop.wait(0.15)
                with lock:
                    frames = state.frames
                minimized(False)
                wait_for(lambda: state.frames > frames)
                resize(width, height, True)
            for width, height in ((900, 600), (1000, 700), (1000, 720)):
                resize(width, height, True)
            record("sequence_completed", transitions=16, **window.check())
        except Exception as error:
            if stop.is_set():
                record("actions_cancelled", detail=repr(error))
            else:
                action_errors.append(repr(error))

    reason = "error"
    try:
        process = subprocess.Popen(command, cwd=root, env=env, stdout=subprocess.PIPE,
                                   stderr=subprocess.STDOUT, creationflags=flags)
        (output / "owned-process.json").write_text(json.dumps(dict(pid=process.pid, command=command)))
        window = OwnedWindow(process)
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        actor = threading.Thread(target=actions, daemon=True)
        actor.start()
        while time.monotonic() < deadline:
            with lock:
                trigger = state.check_slow(time.monotonic_ns())
                failed = bool(state.errors)
            if trigger:
                reason = "first_slow_configure"
                stop.set()
                record("slow_trigger", trigger=trigger)
                if window.hwnd:
                    snapshot("slow-configure")
                break
            if failed or action_errors:
                reason = "error"
                break
            if process.poll() is not None:
                reason = "viewer_exited_early"
                break
            if not actor.is_alive():
                reason = "sequence_complete_no_recurrence"
                snapshot("stack-capability")
                if args.owned_capture:
                    with lock:
                        frames = state.frames
                    wait_for(lambda: state.frames > frames)
                    record("recovery_capture_begin", **window.check())
                    capture = subprocess.run(
                        [sys.executable, "-B", str(args.owned_capture.resolve()), str(window.hwnd),
                         str(process.pid), str(output / "recovered.png")],
                        capture_output=True, text=True, env=env, creationflags=flags,
                        timeout=max(0.1, min(8, deadline - time.monotonic())))
                    record("recovery_capture_end", exit=capture.returncode,
                           stdout=capture.stdout[-4096:], stderr=capture.stderr[-4096:],
                           helper_sha256=sha256(args.owned_capture))
                    if capture.returncode != 0 or not (output / "recovered.png").is_file():
                        raise RuntimeError("owned-window recovery capture failed")
                break
            time.sleep(0.02)
        else:
            reason = "external_deadline"
    except Exception as error:
        reason = "supervisor_error"
        record("supervisor_error", detail=repr(error))
    finally:
        stop.set()
        if process is not None and process.poll() is None:
            if window and window.hwnd:
                owner = w.DWORD()
                window.u.GetWindowThreadProcessId(window.hwnd, c.byref(owner))
                if owner.value == process.pid:
                    window.u.PostMessageW(window.hwnd, 0x10, 0, 0)
            try:
                process.wait(timeout=max(0.1, min(3, deadline - time.monotonic())))
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=3)
        for thread in (actor, reader):
            if thread:
                thread.join(timeout=1)
        with lock:
            (output / "viewer-tail.log").write_bytes(b"".join(state.tail.lines))
            (output / "viewer-context.log").write_bytes(b"".join(state.important.lines))
            report = state.report()
        report.update(stop_reason=reason, exit=process.returncode if process else None,
                      command=command, snapshots=snapshots, actions=events, action_errors=action_errors,
                      elapsed_seconds=time.monotonic() - started,
                      application_request_wait_ms=[float(span["context"]["fields"]["wait_ms"])
                                                   for span in report["configures"] if span["context"]],
                      driver_api_wait_ms=None,
                      binary_sha256=sha256(args.viewer),
                      pdb_sha256=sha256(args.viewer.with_suffix(".pdb")) if args.viewer.with_suffix(".pdb").exists() else None,
                      source_hashes={str(path.relative_to(root)): sha256(path) for path in
                                     (root / "Cargo.lock", root / "viewer/src/main.rs",
                                      root / "viewer/src/surface_probe.rs", Path(__file__))},
                      source_head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root,
                                                          text=True, creationflags=flags).strip(),
                      input_hashes={str(path): sha256(path) for path in
                                    (args.virtual_manifest, args.cameras, args.limits, held)},
                      causality="Unresolved. A stack is one sampled location, not an exact API duration or scheduler trace.",
                      capture_method="CDB -pv, local symbols only, owned PID; no image or memory dump",
                      image_capture="PID-verified WGC HWND helper, inspect separately" if args.owned_capture else "not captured",
                      limits=dict(deadline_seconds=args.deadline, slow_ms=args.slow_ms,
                                  initial_slow_ms=1000, tail_bytes=1048576,
                                  context_bytes=262144, transitions=16))
        report["native_observation_valid"] = bool(
            reason == "sequence_complete_no_recurrence" and report["exit"] == 0
            and report["validation_clean"] and not report["errors"] and not report["action_errors"]
            and snapshots and all(s["identity_and_stack_verified"] and s["window_thread_visible"]
                                  and s["viewer_symbols_visible"] for s in snapshots))
        (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(dict(stop_reason=reason, output=str(output), exit=report["exit"],
                          presented_frames=report["presented_frames"], configures=len(report["configures"]))))
    return 0 if report["native_observation_valid"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--viewer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--virtual-manifest", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--cameras", type=Path, required=True)
    parser.add_argument("--limits", type=Path, required=True)
    parser.add_argument("--sdk", type=Path, default=Path("C:/VulkanSDK/1.4.363.0"))
    parser.add_argument("--cdb", type=Path, default=Path("C:/Program Files (x86)/Windows Kits/10/Debuggers/x64/cdb.exe"))
    parser.add_argument("--owned-capture", type=Path,
                        help="existing PID-verified, HWND-only WGC helper; no foreground capture")
    parser.add_argument("--slow-ms", type=float, default=250)
    parser.add_argument("--deadline", type=float, default=120)
    args = parser.parse_args()
    if not 0 < args.slow_ms < float("inf") or not 30 <= args.deadline <= 120:
        parser.error("slow-ms must be finite and positive; deadline must be 30..120 seconds")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
