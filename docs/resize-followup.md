# Resize stall follow-up

The historical cause remains **INCONCLUSIVE**. The earliest measured slow stage is the synchronous `Surface::configure` call. No new performance fix is justified by the preserved evidence. This unit adds an opt-in host-call probe and an offline analyzer. It changes no draining, rendering, quality, residency, or oracle policy.

## What the preserved timeline establishes

The original `measurements/m4-window/progressive` run stalled before the first image after two rejected rendering attempts. The verified M4 correction prepares the minimum cover before acquisition and retains an acquired image across backpressure. That is a separate defect from the later resize outlier. Vulkan's unpresented-image discard is a no-op in the installed wgpu-hal 30.0.1 source.

The subsequent `progressive2` run used the self-authored material/thin fixture, held overview camera, 1x samples, FIFO, active validation, and a 3000 ms artificial page delay. It did not use the quarry. The preserved executable hash is `4d0425c151d6f236a67beaeb74b5a761afd3bb687186f074467a10ba3107cb44`. Its log identifies RTX 4070 SUPER, Vulkan, NVIDIA 617.14, and a development build. The scene label `cubes_triangle` is stale metadata; the recorded manifest argument identifies the actual fixture.

The helper applied 24 resizes, a portrait resize, three minimize/restore cycles, then an opaque cover/uncover and a final resize to a 984x661 client area. The last part is recoverable from `progressive2/actions.jsonl`, its byte offsets, the original helper, and `viewer.log`.

- At 12:51:45.800988 UTC the owned cover began. It ended at 12:51:46.405038 UTC on 2026-11-03.
- Log line 65112 reports frame 1737 with acquire 0.1466 ms and present 0.2170 ms. Line 65113 reports its GPU sample. The preceding render attempt was accepted.
- Line 65114 records the 984x661 resize. Line 65115 records the held camera pose. There is no configure-entry timestamp or flight count in this binary's log.
- The helper's restored capture action at 12:51:47.071796 UTC and close action at 12:51:47.603806 UTC both record 4,914,872 log bytes, ending at line 65115. The helper therefore reported restoration before the configure-return log existed. Its action label does not establish a newly presented frame.
- Line 65116 records `configure_ms=4933.549`. Line 65120 reports frame 1738 with acquire 0.1552 ms, present 0.2868 ms, and the same configure duration at greater precision. Active validation finishes with zero errors.

The standalone configure timer excludes acquisition, rendering, presentation, and the later log write. The accepted preceding frame and source ownership imply no retained acquired image at that resize. They do not establish zero pending presentation work. Neither log byte offsets nor the helper's wall clock reveal the internal wait or an exact configure start time.

Final M4 `motion2` and `progressive3` logs contain 58 post-initial configurations. Their documented maximum is 136.560 ms. Commit `0fa9c12` later added nonblocking application draining, withheld new uploads while draining, and paced failed redraw retries. The integrated quarry run at `D:/RenderlabEvidence/quarry-window-root-2` has three logged nonzero-flight deferrals and seven post-initial configurations, with a maximum of 18.657 ms. The final full-detail quarry run has five deferrals and six post-initial configurations, with a maximum of 20.521 ms. These workloads, builds, and interaction sequences differ. They provide recovery evidence, not a before/after estimate of a causal fix.

The black HWND capture in `quarry-window-root-2` is an independently verified capture artifact. Its visible-client capture showed the quarry. It supplies no evidence of a renderer black frame or the configure stall's cause. See [native recovery evidence](scan-window.md) and [M4 verification](m4-final-verification.md).

## Where the blocking work can occur

The installed sources inspected for this diagnosis are wgpu, wgpu-core, and wgpu-hal 30.0.1, plus winit 0.30.13. Exact paths, hashes, and numbered excerpts are preserved in the worker's ignored `.refs/resize-followup/source-evidence.json`.

1. `viewer/src/main.rs` polls renderer completion in `wake`. A pending resize drops the retained frame and defers if renderer flights remain. `wait_ms` spans the latest resize request through configuration readiness, including scheduling and deferrals. It is not a GPU wait duration. New resize requests replace this origin.
2. `wgpu-core/src/device/resource.rs:5257` implements `configure_surface`. It queries surface capabilities before waiting. At line 5343 it calls `maintain(PollType::wait_indefinitely())`. `maintain` waits for the last successful submission through the HAL fence at lines 868-880. It then processes completion and resource maintenance.
3. After rejecting a still-acquired output, core calls HAL configuration at line 5381. `wgpu-hal/src/vulkan/instance.rs:1004` takes the swapchain write lock, releases the old swapchain resources, and creates the new swapchain.
4. `wgpu-hal/src/vulkan/swapchain/native.rs:376` releases old resources. Lines 379-390 call `vkDeviceWaitIdle` for presentation work as well. This is distinct from renderer flight completion. The next swapchain creation reaches `vkCreateSwapchainKHR` at lines 236-239, obtains its images, creates synchronization objects, and eventually destroys the old swapchain.
5. Acquisition has its own waits. The native backend waits for the prior submission, calls `vkAcquireNextImageKHR`, and on Windows waits on an acquisition fence. Presentation calls `vkQueuePresentKHR`. These calls can be slow independently of configuration.
6. winit's Windows `WM_SIZE` handler forwards a `Resized` event at `event_loop.rs:1397`. Windows does not emit `WindowEvent::Occluded` in this installed backend. Synchronous Win32 calls, event dispatch, OS scheduling, and composition can delay observable response outside renderer work.

Source inspection proves these possible call paths. It does not locate the 4.933-second interval within them. The timer also includes any descheduling during the call. No preserved native stack, Vulkan call trace, or scheduling trace separates capability queries, submission-fence waits, maintenance, locks, presentation idle, swapchain recreation, or driver/desktop contention.

## Falsifiable hypotheses and the probe

The next observation should separate these mechanisms before another implementation change.

| Hypothesis | Required observation |
| --- | --- |
| Application drain or wake work delays resize | Long request-to-configure interval or `wake`/`poll` span, while the configure span is short. Deferrals and flight counts identify application queue pressure. |
| Surface configuration blocks after renderer completion | Long `configure` span with the existing zero-flight/no-retained-frame entry. A native stack or API trace is still needed to distinguish the waits inside it. |
| Acquisition or presentation causes the apparent resize pause | Long `acquire`, `pre_present_notify`, or `present` span. Acquire outcomes must remain visible even without a presented frame. |
| Window calls or dispatch delay the resize | Long helper `resize_call_begin` to `resize_call_end`, or a gap before the viewer's timestamped resize event. A scheduler trace is needed to attribute that gap to the desktop or driver. |

`--trace-surface` emits versioned begin/end records with IDs, monotonic microseconds, outcomes, and one approximate wall-clock origin for correlation. It brackets wake, redraw, renderer poll, retained-frame release, configure, acquire or frame reuse, pre-present notification, and present. Begin records precede potentially blocking calls. A missing end means no return was observed; process termination or a truncated log remains possible. Configure's `returned` outcome is not a substitute for clean GPU validation.

The probe is disabled by default. It retains no GPU resources and introduces no waits. Diagnostic logging perturbs timing, so probe runs do not certify an uninstrumented performance gate. Outer wake/redraw spans contain child calls and logging; do not sum their durations. Startup asset loading/device creation occurs before the probe's origin. Existing startup markers still cover those stages.

The scan-window helper forwards `--trace-surface`, supports `--samples 1` for the historical fixture, and brackets its synchronous resize call. Its default remains 4x. `--allow-game-active` records the operator's explicit override of the game-name guard, while retaining the quiet-window lock check. That override does not imply a quiet machine or authorize another worker's slot.

Offline analysis requires only Python.

```powershell
py -3 -B scripts/resize-followup.py D:/RenderlabEvidence/resize-followup-probe/viewer.log
py -3 -B -m unittest discover -s scripts -p test_resize_followup.py -v
```

The analyzer counts standalone configuration records once, preserves entry evidence and unfinished calls, reports per-phase counts/outcomes/maxima, and rejects malformed records. Its default 250 ms slow-call threshold is a diagnostic trigger, not a product latency guarantee. Legacy logs without the probe remain explicitly uninstrumented. No analyzer result upgrades causality from inconclusive.

## Verified native integration

On 2026-11-03 the allocated verification slot completed one control/probe pair on the same development executable, SHA-256 `b5cff6cfe6ecb85cffecb3ba5f131e3d17aa9b184ec2a96f02c035cfe6413df0`. The source tree was `5c039cbc12706e71b149da55ba556b9c936d48d6`; this publication adds documentation only. Build, affected viewer formatting/Clippy, four Python tests and the viewer CPU test passed. The build/check commands took approximately 127 seconds; the two native helpers totaled 20.547 seconds, excluding setup, inspection and reporting.

Both sessions used the unchanged self-authored thin fixture, 1x samples, active validation, held overview, FIFO and 3000 ms page delay. The helper exercised resize, minimize and invalid/corrected shaders during preparation and after presentation began. Both helpers/viewers exited normally within their external 240-second deadlines, with positive Khronos activation and zero validation errors. VALORANT was active under Islam's explicit testing override; these are contended functional observations, not a performance comparison.

| Observation | Control | Probe |
| --- | ---: | ---: |
| Presented frames | 444 | 378 |
| Initial configure, ms | 385.905 | 227.405 |
| Post-initial configures | 8 | 8 |
| Post-initial configure maximum, ms | 21.274 | 22.449 |
| Successful-frame acquire maximum, ms | 8.8602 | 5.1973 |
| Successful-frame present maximum, ms | 1.3163 | 0.9265 |
| Rejected / corrected shader reloads | 2 / 2 | 2 / 2 |

All configure entries reported zero renderer flights and no retained frame. The probe recorded 5,926 balanced begin/end pairs, including one outdated acquisition during minimize/restore followed by successful configuration and presentation. The disabled control emitted no probe records. The two probe-side drain deferrals and successful recovery exercise the existing lifecycle without changing it.

Four PID-verified, window-only Windows Graphics Capture images were inspected, including root inspection of the probe's recovered image. They show the expected orange thin fixture and cyan/orange surface. The four PNGs are byte-identical (`450cd33c4028f32104984d8b51af961a07bf39c01b3c290246a562852f209cea`); independent log offsets establish newly presented frames after each recovery. This verifies those observed states, not every intermediate size or fully refined content. No desktop/game image was captured. The capture wrapper adds about 0.55–0.69 seconds to one helper resize-call record; that interval is capture overhead and must not be attributed to Win32 or renderer configuration.

Local raw sessions: `D:/RenderlabEvidence/resize-followup-verify-control-20261103-2139` and `D:/RenderlabEvidence/resize-followup-verify-probe-20261103-2141`. The verification checkout's `.refs/resize-verification/handoff.md` and `verification.json` preserve exact commands, tracked-source hashes, resource snapshots, capture provenance and phase counts. Build output remains isolated at `D:/Coding/renderlab-work/resize-stall/target`; all owned processes exited. No source correction was needed after the original diagnostic patch.

## Stop rule and any future recurrence

The pair is complete; no post-initial configure reached the 250 ms diagnostic stop threshold. Do not repeat large loops to hunt the rare historical outlier. Future targeted runs still need external deadlines: the viewer's `--seconds 180` cannot interrupt a blocked driver or synchronous Win32 call. Preserve logs and terminate only experiment-owned processes if a deadline is reached.

If a slow configure recurs with drained flights, the next separately authorized experiment is one native stack/scheduling or Vulkan API trace on that concrete trigger. A timed `vkDeviceWaitIdle`, fence wait, capability query, or swapchain operation is the evidence needed to move beyond the current aggregate call. Do not bypass required synchronization, move configuration to a worker thread, reduce quality, or change present mode from code inspection alone.

The successful pair establishes diagnostic integration and observed image recovery. It supplies no quiet performance gate, Unreal comparison, universal latency bound or attribution of the historical stall. The cause remains inconclusive; no synchronization workaround is promoted.
