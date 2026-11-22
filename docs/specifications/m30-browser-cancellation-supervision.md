# M30 — Cancellation-Safe Browser Render Supervision

## Objective

Ensure that cancelling the caller's render future does not immediately drop the in-flight browser lifecycle and bypass its bounded shutdown path.

## Implementation

- `BrowserRenderer::render` clones only the immutable renderer configuration and shared policy/capacity handles needed by an owned worker.
- The worker runs in a Tokio task and owns the complete `render_inner` future, including the browser handle and event-handler task.
- The caller receives the result through a Tokio oneshot receiver. Dropping the caller future drops only that receiver; it does not cancel the worker task.
- The worker retains M29's single active-operation deadline and M28's bounded browser-close and handler-stop cleanup.
- If the worker terminates without sending a result, the caller receives the stable `BROWSER_RENDER_SUPERVISOR_FAILED` error.
- The render-capacity permit remains owned by the worker until its operation and cleanup finish, preventing cancellation from prematurely releasing admission capacity while browser work is still active.

## Compatibility and safety

- No public configuration fields or browser engine contract changes are introduced.
- URL policy evaluation and HTML input validation remain before worker admission.
- The worker still renders only caller-supplied snapshots in an isolated `about:blank` page with JavaScript disabled and restrictive offline CSP.
- Caller cancellation is now isolated from the browser operation's task lifetime, but runtime shutdown can still terminate Tokio tasks.
- This is an in-process task-lifetime guarantee, not a persistent process supervisor or an operating-system child-process reaper.

## Acceptance criteria

1. Dropping the caller's oneshot receiver does not cancel the owned operation task.
2. The worker retains the render permit until its bounded operation and cleanup finish.
3. The existing M28 cleanup path remains responsible for browser close and event-handler termination.
4. A worker that exits without returning a value maps to a stable structured error.
5. Rust formatting, Clippy, workspace tests, SDK checks, repository/specification checks, CodeQL, and Dependency Review pass before merge.

## Explicit limitations and next boundary

This milestone prevents ordinary caller-future cancellation from bypassing the browser worker's cleanup path. It does not guarantee cleanup after Tokio runtime shutdown, process termination, or worker panic. It also does not prove that Chromium's operating-system child process is always reaped when browser launch or shutdown itself fails. OS-level process ownership, termination escalation, and reaping require a separate process-lifecycle implementation and platform-specific validation.
