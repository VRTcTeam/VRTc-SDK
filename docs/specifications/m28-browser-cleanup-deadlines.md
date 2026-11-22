# M28 — Browser Cleanup Deadlines

## Objective

Ensure that browser renderer cleanup cannot wait indefinitely after a render attempt. M27 bounds admission to renderer capacity; M28 closes the lifecycle gap after browser work has started.

## Implementation

- Apply the existing `operation_timeout_ms` budget to `Browser::close()`.
- Abort the Chromium event-handler task regardless of whether browser close succeeds or times out.
- Await the aborted handler task with a bounded timeout.
- Return stable `BROWSER_CLEANUP_TIMEOUT`, `BROWSER_CLEANUP_FAILED`, and `BROWSER_HANDLER_STOP_TIMEOUT` errors for cleanup failures.
- Preserve the original render error when both rendering and cleanup fail; cleanup must not mask the primary failure.
- Treat cleanup failure after an otherwise successful render as a failed render operation rather than reporting success while browser shutdown is uncertain.

## Compatibility and safety

- No new public configuration field is introduced; the existing operation timeout is reused.
- Snapshot-only rendering, URL policy checks, JavaScript disablement, restrictive CSP, and M27 capacity admission remain unchanged.
- No live navigation, remote subresource loading, browser pooling, or distributed capacity is added.
- Cleanup is bounded per phase; total shutdown can consume up to two operation-timeout intervals (browser close plus handler join).

## Acceptance criteria

1. Browser close is guarded by a timeout.
2. Event-handler task is aborted on every cleanup path.
3. Handler join is bounded.
4. Render errors take precedence over cleanup errors.
5. Successful rendering is not reported when cleanup itself fails.
6. Focused test verifies a stalled handler is aborted and joined.
7. CI, CodeQL, and Dependency Review pass before merge.

## Limitations

This milestone bounds cleanup after the browser launch has returned. It does not yet impose one end-to-end deadline across queue admission, browser launch, rendering, and cleanup. Cancellation of the outer caller future and operating-system-level orphan-process reaping remain separate lifecycle concerns.
