"""Bounded owned-window lighting/reload check. No foreground change or screen capture."""
from pathlib import Path
import ctypes as c
from ctypes import wintypes as w
import importlib.util
import json
import os
import subprocess
import sys
import time

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def main():
    out = ROOT / 'measurements/m5-a/native'
    out.mkdir(parents=True, exist_ok=True)
    shader = out / 'scene.wgsl'
    source = (ROOT / 'shaders/scene.wgsl').read_bytes()
    shader.write_bytes(source)
    settings = out / 'lighting.json'
    initial = (ROOT / 'fixtures/quarry-lighting.json').read_text()
    settings.write_text(initial)
    spec = importlib.util.spec_from_file_location('window_check', ROOT / 'scripts/scan-window-check.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    env = os.environ.copy()
    env['VK_IMPLICIT_LAYER_PATH'] = str(ROOT / 'measurements/empty-vulkan-implicit')
    cmd = [str(ROOT / 'target/debug/renderlab-viewer.exe'), '--background', '--validate', '--log-frames', '--seconds', '25', '--view', 'lit', '--samples', '4', '--lighting', str(settings), '--shader', str(shader), '--capture-hdr', str(out / 'viewer')]
    logpath = out / 'viewer.log'
    process = None
    events = []
    started = time.monotonic()

    def log():
        return logpath.read_text(errors='replace')

    def wait(pattern, count=0):
        until = time.monotonic() + 12
        while time.monotonic() < until:
            if log().count(pattern) > count:
                events.append({'pattern': pattern, 'seconds': time.monotonic()-started})
                return
            if process.poll() is not None:
                raise RuntimeError(f'viewer exited {process.returncode}: {log()[-2000:]}')
            time.sleep(.025)
        raise TimeoutError(pattern)

    try:
        with logpath.open('wb') as stream:
            process = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=stream, stderr=stream, creationflags=subprocess.CREATE_NO_WINDOW)
            window = module.Window(process)
            wait('startup_to_present_call_proxy_ms=')
            wait('hdr_capture prefix=')
            settings.write_text('{invalid')
            wait('lighting_rejected')
            changed = json.loads(initial)
            changed['exposure_stops'] = -6
            before = log().count('lighting_updated')
            settings.write_text(json.dumps(changed))
            wait('lighting_updated', before)
            shader.write_bytes(b'invalid WGSL')
            wait('reload_rejected')
            before = log().count('frame=')
            wait('frame=', before)
            shader.write_bytes(source.replace(b'vec3<f32>(1.0, 1.0, 1.0)', b'vec3<f32>(0.6, 0.8, 1.0)'))
            wait('reload_compile_ms=')
            wait('revision=2')
            for key, mode in [(0x31, 'BaseColor'), (0x35, 'Lit')]:
                window.key(key)
                window.key(key, released=True)
                until = time.monotonic()+5
                while time.monotonic() < until:
                    title = c.create_unicode_buffer(1024)
                    window.u.GetWindowTextW(window.handle(), title, len(title))
                    if f'| {mode} ' in title.value:
                        events.append({'key': chr(key), 'mode': mode, 'seconds': time.monotonic()-started})
                        break
                    time.sleep(.025)
                else:
                    raise TimeoutError(f'owned key {key} mode {mode}')
            window.u.PostMessageW(window.handle(), 0x10, 0, 0)
            process.wait(timeout=8)
        if process.returncode != 0 or 'gpu_validation_summary errors=0 missing_layer=0 active_layer=1' not in log():
            raise RuntimeError(log()[-3000:])
        (out / 'result.json').write_text(json.dumps({'command':cmd,'events':events,'exit':process.returncode,'seconds':time.monotonic()-started,'capture':'owned renderer HDR; no desktop capture','foreground_changes':False},indent=2))
        print(out / 'result.json')
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            process.wait(timeout=8)
        shader.write_bytes(source)
        settings.write_text(initial)


if __name__ == '__main__':
    main()
