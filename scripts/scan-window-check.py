"""Run one owned native quarry viewer check, or summarize saved configure logs."""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time


def summarize(text):
    durations = [float(v) for v in re.findall(r"^configure_ms=([\d.]+)", text, re.M)]
    begins = re.findall(r"^configure_begin .*", text, re.M)
    return {
        "configure_count": len(durations),
        "configure_ms": durations,
        "maximum_configure_ms": max(durations, default=None),
        "instrumented_configure_count": len(begins),
        "all_instrumented_configures_drained": all(
            "flights=0 pending_frame=false" in line for line in begins) if begins else None,
        "configure_deferrals": len(re.findall(r"^configure_deferred ", text, re.M)),
        "reload_rejections": text.count("reload_rejected"),
        "reload_installs": text.count("reload_compile_ms="),
        "presented_frames": len(re.findall(r"^frame=\d+ revision=", text, re.M)),
        "validation_clean": bool(re.search(
            r"gpu_validation_summary errors=0 missing_layer=0 active_layer=1\b", text)),
        "viewer_stopped": "viewer_stopped" in text,
        "interpretation": "Finite native-window observation. No driver-stall root-cause or latency-bound claim.",
    }


class Window:
    def __init__(self, process):
        self.process = process
        self.u = c.WinDLL("user32", use_last_error=True)
        self.u.SetProcessDPIAware()
        signatures = {
            "GetWindowThreadProcessId": ([w.HWND, c.POINTER(w.DWORD)], w.DWORD),
            "GetWindowTextW": ([w.HWND, w.LPWSTR, c.c_int], c.c_int),
            "IsWindowVisible": ([w.HWND], w.BOOL),
            "IsIconic": ([w.HWND], w.BOOL),
            "ShowWindow": ([w.HWND, c.c_int], w.BOOL),
            "PostMessageW": ([w.HWND, w.UINT, w.WPARAM, w.LPARAM], w.BOOL),
            "MoveWindow": ([w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.BOOL], w.BOOL),
            "MapVirtualKeyW": ([w.UINT, w.UINT], w.UINT),
            "GetForegroundWindow": ([], w.HWND),
            "SetForegroundWindow": ([w.HWND], w.BOOL),
            "GetClientRect": ([w.HWND, c.POINTER(w.RECT)], w.BOOL),
            "ClientToScreen": ([w.HWND, c.POINTER(w.POINT)], w.BOOL),
            "AttachThreadInput": ([w.DWORD, w.DWORD, w.BOOL], w.BOOL),
            "SetWindowPos": ([w.HWND, w.HWND, c.c_int, c.c_int, c.c_int, c.c_int, w.UINT], w.BOOL),
            "WindowFromPoint": ([w.POINT], w.HWND),
            "GetAncestor": ([w.HWND, w.UINT], w.HWND),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.u, name)
            function.argtypes, function.restype = args, result
        self.callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
        self.u.EnumWindows.argtypes = [self.callback, w.LPARAM]
        self.u.EnumWindows.restype = w.BOOL
        self.held = set()

    def handle(self):
        if self.process.poll() is not None:
            raise RuntimeError(f"owned viewer exited {self.process.returncode}")
        found = []

        def visit(hwnd, unused):
            pid = w.DWORD()
            self.u.GetWindowThreadProcessId(hwnd, c.byref(pid))
            title = c.create_unicode_buffer(512)
            self.u.GetWindowTextW(hwnd, title, len(title))
            if pid.value == self.process.pid and title.value.startswith("Renderlab |"):
                found.append(hwnd)
            return True

        self.u.EnumWindows(self.callback(visit), 0)
        if len(found) != 1:
            raise RuntimeError(f"expected one owned viewer window, got {len(found)}")
        return found[0]

    def key(self, key, released=False):
        scan = self.u.MapVirtualKeyW(key, 0)
        flags = 1 | (scan << 16) | ((1 << 24) if 37 <= key <= 40 else 0)
        if released:
            flags |= (1 << 30) | (1 << 31)
        if not self.u.PostMessageW(self.handle(), 0x101 if released else 0x100, key, flags):
            raise c.WinError(c.get_last_error())
        if released:
            self.held.discard(key)
        else:
            self.held.add(key)

    def release(self):
        for key in tuple(self.held):
            self.key(key, released=True)

    def resize(self, width, height):
        if not self.u.MoveWindow(self.handle(), 80, 70, width, height, True):
            raise c.WinError(c.get_last_error())

    def capture(self, path):
        from PIL import ImageGrab
        hwnd = self.handle()
        current = c.WinDLL("kernel32").GetCurrentThreadId()
        front = self.u.GetWindowThreadProcessId(self.u.GetForegroundWindow(), None)
        attached = self.u.AttachThreadInput(current, front, True)
        try:
            self.u.SetForegroundWindow(hwnd)
        finally:
            if attached:
                self.u.AttachThreadInput(current, front, False)
        time.sleep(0.4)
        if self.u.IsIconic(hwnd):
            raise RuntimeError("refuse capture of minimized viewer")
        rect, origin = w.RECT(), w.POINT()
        if not self.u.GetClientRect(hwnd, c.byref(rect)) or not self.u.ClientToScreen(hwnd, c.byref(origin)):
            raise c.WinError(c.get_last_error())
        if not self.u.SetWindowPos(hwnd, w.HWND(-1), 0, 0, 0, 0, 0x13):
            raise c.WinError(c.get_last_error())
        try:
            time.sleep(0.3)
            # Windows rounds the client corners; hit-test within the drawable rectangle.
            inset = min(12, rect.right // 4, rect.bottom // 4)
            for x, y in ((inset, inset), (rect.right - inset, inset), (inset, rect.bottom - inset), (rect.right - inset, rect.bottom - inset), (rect.right // 2, rect.bottom // 2)):
                hit = self.u.WindowFromPoint(w.POINT(origin.x + x, origin.y + y))
                if self.u.GetAncestor(hit, 2) != hwnd:
                    raise RuntimeError("refuse capture with another window covering the viewer")
            ImageGrab.grab(bbox=(origin.x, origin.y, origin.x + rect.right, origin.y + rect.bottom)).save(path)
            ImageGrab.grab(window=hwnd).save(path.with_stem(path.stem + "-hwnd"))
        finally:
            self.u.SetWindowPos(hwnd, w.HWND(-2), 0, 0, 0, 0, 0x13)


def run(args):
    if os.name != "nt":
        raise RuntimeError("native run requires Windows")
    output = args.output.resolve()
    if output.drive.upper() != "D:":
        raise RuntimeError("private evidence must stay on D:")
    root = Path(__file__).resolve().parent.parent
    output.mkdir(parents=True, exist_ok=False)
    shader = output / "scene.wgsl"
    original = (root / "shaders/scene.wgsl").read_bytes()
    shader.write_bytes(original)
    cameras = json.loads(args.cameras.read_text())
    cameras["frames"] = cameras["frames"][:1]
    held_camera = output / "held-camera.json"
    held_camera.write_text(json.dumps(cameras))
    command = [str(args.viewer.resolve()), "--validate", "--samples", str(args.samples), "--log-frames",
               "--seconds", "180", "--shader", str(shader), "--cameras", str(held_camera),
               "--limits", str(args.limits.resolve()), "--anisotropy", str(args.anisotropy)]
    if args.interactive_camera:
        command += ["--interactive-camera"]
    if args.trace_surface:
        command += ["--trace-surface"]
    if args.asset:
        command += ["--asset", str(args.asset.resolve())]
    else:
        command += ["--virtual-manifest", str(args.virtual_manifest.resolve()),
                    "--cache", str(args.cache.resolve()), "--progressive",
                    "--io-delay-ms", str(args.io_delay_ms)]
    env = os.environ.copy()
    implicit = output / "empty-vulkan-implicit"
    implicit.mkdir()
    env.update(VK_IMPLICIT_LAYER_PATH=str(implicit), TEMP=str(output), TMP=str(output))
    log_path = output / "viewer.log"
    events, process, window = [], None, None
    error, forced = None, False
    next_guard = 0.0

    def guard():
        nonlocal next_guard
        if time.monotonic() < next_guard:
            return
        next_guard = time.monotonic() + 1
        if not args.allow_game_active:
            tasks = subprocess.check_output(["tasklist.exe", "/FO", "CSV"],
                                            creationflags=subprocess.CREATE_NO_WINDOW).lower()
            if any(name in tasks for name in (b"valorant-win64-shipping", b"fortniteclient", b"eldenring")):
                raise RuntimeError("protected game active")
        if Path("D:/AraBuild/render-v3/quiet-window.lock").exists():
            raise RuntimeError("another GPU quiet window is active")

    def log():
        return log_path.read_text(errors="replace") if log_path.exists() else ""

    def event(name, **values):
        events.append(dict(action=name, time_ns=time.time_ns(),
                           log_bytes=log_path.stat().st_size, **values))
        (output / "actions.json").write_text(json.dumps(events, indent=2))

    def wait(needle, before=0, timeout=12):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            guard()
            if log().count(needle) > before:
                return
            if process.poll() is not None or "viewer_stopped" in log():
                raise RuntimeError("viewer stopped while waiting for " + needle)
            time.sleep(0.04)
        raise TimeoutError("waiting for " + needle)

    def tap(key, expected):
        before = log().count("input=" + expected)
        guard()
        try:
            window.key(key)
            wait("input=" + expected, before)
        finally:
            window.key(key, released=True)
        event("input_observed", key=expected)

    def reload(bad):
        needle = "reload_rejected" if bad else "reload_compile_ms="
        before = log().count(needle)
        guard()
        shader.write_bytes(original + b"\ninvalid shader syntax\n" if bad else original)
        wait(needle, before)
        event("shader_rejected" if bad else "shader_restored")

    def resized(width, height, expect_frame):
        before = log().count("resize_requested=")
        configured = log().count("\nconfigure_ms=")
        guard()
        event("resize_call_begin", width=width, height=height)
        started = time.monotonic_ns()
        window.resize(width, height)
        event("resize_call_end", elapsed_ms=(time.monotonic_ns() - started) / 1e6)
        wait("resize_requested=", before)
        if expect_frame:
            wait("\nconfigure_ms=", configured)
            wait("\nframe=", log().count("\nframe="))
        event("resize_observed", width=width, height=height, frame_expected=expect_frame)

    try:
        guard()
        with log_path.open("wb") as stream:
            process = subprocess.Popen(command, cwd=root, env=env, stdout=stream, stderr=stream,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
        window = Window(process)
        wait("timing_status=", timeout=90)
        window.handle()
        event("window_ready", pid=process.pid)
        if not args.asset:
            if "virtual_first_scene_submitted_us=" in log():
                raise RuntimeError("loading check missed preparation; increase --io-delay-ms")
            tap(0x32, "Digit2")
            reload(True)
            resized(780, 520, False)
            guard()
            window.u.ShowWindow(window.handle(), 6)
            time.sleep(0.2)
            if not window.u.IsIconic(window.handle()):
                raise RuntimeError("minimize did not take effect")
            event("loading_minimized")
            window.u.ShowWindow(window.handle(), 9)
            reload(False)
            if "virtual_first_scene_submitted_us=" in log():
                raise RuntimeError("preparation ended before loading actions finished")
            event("loading_actions_finished_before_first_scene")
        wait("\nframe=", timeout=100)
        tap(0x31, "Digit1")
        resized(1000, 700, True)
        if args.capture:
            window.capture(output / "baseline.png")
        for width, height in ((620, 900), (1100, 680), (760, 510), (1000, 700)):
            resized(width, height, True)
            if args.capture:
                window.capture(output / f"resize-{width}x{height}.png")
        guard()
        before = log().count("resize_requested=0x0")
        window.u.ShowWindow(window.handle(), 6)
        wait("resize_requested=0x0", before)
        event("minimized")
        reload(True)
        frames = log().count("\nframe=")
        window.u.ShowWindow(window.handle(), 9)
        wait("\nframe=", frames)
        event("restored_with_rejected_shader")
        if args.capture:
            window.capture(output / "rejected.png")
        frames = log().count("\nframe=")
        reload(False)
        wait("\nframe=", frames)
        tap(0x27, "ArrowRight")
        if args.capture:
            window.capture(output / "restored.png")
    except Exception as exception:
        error = repr(exception)
    finally:
        if process is not None and process.poll() is None:
            try:
                window.release()
                window.u.PostMessageW(window.handle(), 0x10, 0, 0)
                process.wait(timeout=8)
            except Exception as cleanup_error:
                forced = True
                error = error or repr(cleanup_error)
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=8)
        try:
            shader.write_bytes(original)
        except OSError as restore_error:
            error = error or repr(restore_error)
    report = summarize(log())
    report.update(command=command, pid=process.pid if process else None,
                  exit=process.returncode if process else None, error=error,
                  forced_termination=forced,
                  game_guard_overridden=args.allow_game_active,
                  binary_sha256=hashlib.sha256(args.viewer.read_bytes()).hexdigest(),
                  head=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
                  source_sha256={str(p.relative_to(root)): hashlib.sha256(p.read_bytes()).hexdigest()
                                 for directory in ("src", "viewer/src", "support", "crates")
                                 for p in (root / directory).rglob("*.rs")},
                  input_method="PID-scoped WM_KEYDOWN/WM_KEYUP, not physical input latency",
                  visual_review="required separately" if args.capture else "not captured")
    passed = not error and report["exit"] == 0 and report["validation_clean"] and not report["viewer_stopped"]
    report["status"] = "PASS" if passed else "ISSUES"
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "source_sha256"}, indent=2))
    return 0 if passed else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    analyze = commands.add_parser("analyze")
    analyze.add_argument("logs", nargs="+", type=Path)
    native = commands.add_parser("run")
    native.add_argument("--viewer", type=Path, required=True)
    native.add_argument("--output", type=Path, required=True)
    native.add_argument("--cameras", type=Path, required=True)
    native.add_argument("--limits", type=Path, required=True)
    scene = native.add_mutually_exclusive_group(required=True)
    scene.add_argument("--asset", type=Path)
    scene.add_argument("--virtual-manifest", type=Path)
    native.add_argument("--cache", type=Path)
    native.add_argument("--io-delay-ms", type=int, default=1000)
    native.add_argument("--capture", action="store_true")
    native.add_argument("--anisotropy", type=int, choices=range(1, 17), default=1)
    native.add_argument("--samples", type=int, choices=(1, 4), default=4)
    native.add_argument("--interactive-camera", action="store_true")
    native.add_argument("--trace-surface", action="store_true")
    native.add_argument("--allow-game-active", action="store_true",
                        help="explicit operator authorization; observations remain contended")
    args = parser.parse_args()
    if args.command == "analyze":
        print(json.dumps({str(path): summarize(path.read_text(errors="replace")) for path in args.logs}, indent=2))
        return 0
    if args.virtual_manifest and not args.cache:
        parser.error("--virtual-manifest requires --cache")
    if args.io_delay_ms < 0:
        parser.error("--io-delay-ms must be nonnegative")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
