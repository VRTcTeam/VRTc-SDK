# M29 — Browser Render Operation Deadline

## Objective

Prevent queueing and sequential browser operations from each receiving a fresh full timeout. A render request must consume one shared active-operation budget.

## Implementation

- Start a monotonic deadline before render-capacity admission.
- Bound queue admission by the smaller of `max_queue_wait_ms` and the remaining operation budget.
- Use the same absolute deadline for Chromium launch, page creation, JavaScript disablement, snapshot rendering, DOM extraction, and title retrieval.
- Retain stage-specific structured errors so callers can distinguish queue, launch, page, policy, render, and content failures.
- Preserve M28 cleanup behavior: browser close and handler termination are attempted with bounded cleanup grace even after the active-operation deadline expires.

## Compatibility and safety

- No new public configuration fields are introduced.
- `operation_timeout_ms` now describes the shared active-operation budget; it is no longer renewed independently for each stage.
- `max_queue_wait_ms` remains an additional upper bound on admission, never an extension of the operation deadline.
- Cleanup grace is deliberately outside the active-operation deadline. This allows shutdown to be attempted after the deadline, so total wall-clock time can exceed `operation_timeout_ms` by the bounded M28 cleanup phases.
- Snapshot-only rendering, policy evaluation, JavaScript disablement, restrictive offline CSP, and M27 capacity limits remain unchanged.

## Acceptance criteria

1. Queue admission cannot consume more than the operation budget or queue-wait limit.
2. Browser launch and every page/render operation use the same absolute deadline.
3. The deadline is not reset between sequential browser operations.
4. Cleanup remains attempted and bounded after operation timeout.
5. Existing stage-specific error codes and render-error precedence remain intact.
6. Rust workspace CI, CodeQL, and Dependency Review pass before merge.

## Limitations

This milestone bounds the active render lifecycle with one shared deadline, then allows the separately bounded M28 cleanup grace. It does not yet guarantee asynchronous cleanup when the caller drops the render future during an in-flight browser operation; cancellation supervision and operating-system process reaping remain separate work.
