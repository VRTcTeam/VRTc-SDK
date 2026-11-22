# M31 — Chromium Child-Process Lifecycle & Reaping

## Objective

Close the process-ownership gap left by M28–M30. A browser render must not report cleanup as complete while a Chromium child process launched by VRTc is still unreaped.

## Verified dependency contract

The pinned direct dependency is `chromiumoxide = 0.9.1`. Its `Browser` API exposes:
- `close()` for graceful browser shutdown;
- `wait()` to collect the spawned child process exit status;
- `kill()` to forcibly terminate the spawned child and wait for its exit;
- no-op process ownership behavior for browser connections created with `Browser::connect`, which do not own a child process.

VRTc uses `Browser::launch`, so its renderer owns the process handle. The dependency's launch path also attempts to kill and wait for a child if browser initialization fails.

## Implementation

- Keep graceful `Browser::close()` as the first shutdown action.
- After the close attempt, always attempt to collect the launched Chromium child with bounded `Browser::wait()`.
- If the process does not exit before the cleanup interval, call `Browser::kill()` under a separate bounded interval. The dependency documents that this operation kills and waits for the child.
- If waiting fails, attempt forced termination before returning a structured process-lifecycle error.
- Continue stopping and joining the browser event-handler task even when close or process reaping fails.
- Preserve the original render error when rendering and cleanup both fail. For successful rendering, surface close, process, forced-termination, or handler cleanup failures rather than silently returning success.
- Treat a forced process termination as a cleanup failure signal even when the child was successfully reaped; this makes abnormal shutdown visible to callers.

## Safety and compatibility

- No new public configuration fields or browser engine contract changes.
- The existing cleanup timeout is applied independently to graceful close, process wait, forced kill/wait, and handler join. Total cleanup wall time is bounded by those sequential intervals, not by one single interval.
- This process ownership guarantee applies to Chromium instances launched by this renderer. It does not terminate externally managed browsers.
- Tokio runtime shutdown and abrupt host-process termination remain outside an in-process async cleanup guarantee. Chromium descendant-process behavior is subject to the operating system and Chromium's own process model.

## Acceptance criteria

1. Graceful close is attempted first.
2. A launched Chromium child is explicitly waited/reaped after close.
3. A child that does not exit in time receives bounded forced termination.
4. A wait error triggers a forced-termination attempt.
5. Event-handler shutdown is still attempted on every cleanup path.
6. Cleanup failures use stable structured error codes.
7. Render errors retain precedence over cleanup errors.
8. Documentation describes the dependency-backed lifecycle and its limits.
