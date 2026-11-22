# M32 — Browser Supervisor Shutdown Coordination

## Status

Implemented in the VRTc browser and runtime crates.

## Problem

M30 made individual render operations cancellation-safe by moving browser work into an owned Tokio task. M31 added bounded Chromium child-process reaping when that worker reaches its cleanup path. Neither milestone provided a host-controlled way to stop admitting new renders and wait for all already-admitted workers before the host destroys its Tokio runtime.

Without an explicit shutdown boundary, a host could tear down the runtime while a detached browser supervisor was still responsible for closing and reaping Chromium.

## Contract

- `BrowserRenderer::shutdown().await` permanently closes admission for that renderer instance.
- `VrtcRuntime::shutdown_browser().await` exposes the same operation through the runtime façade.
- Shutdown is idempotent.
- Calls already waiting for render capacity are not admitted as workers after shutdown begins; the supervisor rechecks the closed state at the spawn boundary.
- Workers admitted before shutdown remain owned by the supervisor and are awaited, even when their original oneshot receiver has been dropped.
- Worker completion is tracked with a watch channel and an RAII completion guard. No additional task-tracker dependency is required.
- The supervisor's admission gate and active-worker count are updated under one synchronous mutex. No mutex guard is held across an await.
- A worker panic still releases its active-worker accounting through guard drop; its caller observes a closed oneshot channel.

## Host lifecycle

Hosts must stop submitting new application-level work, await `shutdown_browser()`, and only then shut down the Tokio runtime. This API coordinates the browser renderer only; it does not claim to stop unrelated engines, provider tasks, queue workers, or external services.

## Failure and boundary behavior

- New render requests after shutdown begins return `BROWSER_RENDERER_SHUTTING_DOWN`.
- A poisoned supervisor state lock returns `BROWSER_SUPERVISOR_LOCK_POISONED`.
- An unexpectedly closed completion signal returns `BROWSER_SUPERVISOR_SIGNAL_CLOSED`.
- Existing render workers retain the M29 operation deadline and M28/M31 bounded browser cleanup behavior.
- Explicit shutdown waits for workers to finish their normal bounded cleanup. The method itself has no separate forced deadline because returning early would invalidate its wait-for-workers contract.
- No async API can guarantee graceful cleanup if the host abruptly aborts the Tokio runtime, terminates the process, or fails to await shutdown.

## Verification scope

The regression suite verifies that shutdown waits for a worker whose result receiver was dropped, closes future admission, is idempotent, and rejects subsequent browser renders without launching Chromium.
