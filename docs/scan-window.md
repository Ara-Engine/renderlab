# Check quarry window recovery

Reserve the GPU and native-window slot before running this check. Keep the foreground game closed. Use a fresh output directory on D: for every run. The helper launches and closes only its own viewer process. It does not build the viewer.

```powershell
cargo build --locked -p renderlab-viewer -j 2
$scan = 'D:/Coding/projects/desktop/renderlab/measurements/quarry-scan'
py -3 -B scripts/scan-window-check.py run --viewer target/debug/renderlab-viewer.exe --virtual-manifest "$scan/quarry-virtual.manifest" --cache "$scan/cache" --cameras "$scan/cameras.json" --limits "$scan/limits.json" --output D:/RenderlabEvidence/quarry-window-NEW --capture
```

The helper uses the first supplied camera pose, 4x MSAA, unlit base color, active Khronos validation, and an isolated shader copy. Its default 1000 ms page delay creates a loading interval. This deliberately delayed progressive cover does not certify settled detail or normal startup latency. Supply other scene paths through the same arguments. To check full prepared geometry, replace the virtual manifest and cache arguments with `--asset PATH`.

The check performs the following actions.

1. During virtual preparation, post a view-mode key, reject an invalid shader, resize, minimize, restore, and correct the shader. Require these actions to finish before the first scene submission.
2. Require a presented frame. Return to base color and resize through portrait and landscape extents. Require configuration and a subsequent presented frame after each resize.
3. Minimize again and reject a shader while minimized. Restore and require frames with the last valid pipeline. Correct the shader and require further frames.
4. Close the owned window. Require process exit 0 and clean active validation.

`result.json` reports automation status. `actions.json` stores action times and log offsets. `viewer.log` preserves preparation, reload, resize, configuration, and presentation events. Optional PNGs capture the visible client area of the owned viewer after briefly raising it above other windows and checking ownership at its corners and center. A parallel `-hwnd.png` preserves the unreliable HWND capture for diagnosis. Inspect them separately for actual content, complete silhouettes, aspect changes, and a valid image during shader rejection. A black window capture is inconclusive. An automation PASS alone does not certify image quality.

Input uses paired `WM_KEYDOWN` and `WM_KEYUP` messages to the owned HWND. Key release and shader restoration run during cleanup. A process handle limits fallback termination to the spawned viewer. This checks native event dispatch, not physical keyboard or input-to-photon latency. The held registered camera overrides orbit movement, so the arrow check certifies event delivery only. The helper requires Pillow with `ImageGrab.grab(window=...)` for captures. Capture may raise the owned window; it uses no global input hooks and saves only its client rectangle.

The helper refuses an existing output directory and stops if a protected game or another GPU quiet-window lock appears. Do not run it while a game is active. A minimized window must emit a zero-sized resize on this Windows backend. Windows winit does not emit `WindowEvent::Occluded`, so no result claims that event was tested.

## Inspect configuration evidence

```powershell
py -3 -B scripts/scan-window-check.py analyze D:/RenderlabEvidence/quarry-window-NEW/viewer.log
```

Each `configure_begin` records zero owned renderer flights and no retained acquired frame. `configure_deferred` marks a resize that waited for nonblocking renderer progress. No new prepared uploads or virtual rendering begin while a resize drains. Input handling and shader polling continue. A redraw that makes no presentation schedules its next requested retry at least 8 ms later, rather than immediately requesting another unsuccessful redraw.

`configure_ms` measures the synchronous driver call separately. The preceding `wait_ms` measures time since the latest resize request, including event scheduling and drain deferrals. Neither is scanout latency. Renderer completion does not establish that the presentation queue or driver is idle. Configuration can still block internally. These changes do not explain or prove removal of the historical 4933.549 ms configure stall.

Asset reading, decoding, device setup, individual upload calls, and individual selector calls remain synchronous. This patch does not fix the quarry's development-build preparation or draw CPU cost. Runtime evidence and exact binaries for the implementation pass are recorded in the worker handoff.

## Integrated root verification

The integrated viewer and draw-coalescing build passed the native check at `D:/RenderlabEvidence/quarry-window-root-2`. Its executable SHA-256 is `36f1af262014f05bf7ace076812cd80c426f599f722d4b6a4d80c5890536db89`. Root inspected the visible restored and portrait client images. In the same restored state, the HWND image was black while the client-area image showed the complete quarry. This establishes a capture-method failure for that observation, not a renderer recovery failure.

The run observed three nonzero-flight configure deferrals, eight configurations, two rejected shaders, two successful corrections, 2,115 presented frames, and clean active Khronos validation. After the initial 187.139 ms setup, configuration calls ranged from 11.508 to 18.657 ms. These are finite interaction observations with deliberate IO delay and logging. The historical 4.93-second driver outlier remains unexplained; no universal configuration-time bound is asserted. The earlier root attempt refused an unavailable foreground capture safely and remains preserved in `quarry-window-root-1`.
