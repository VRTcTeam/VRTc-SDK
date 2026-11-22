# M27 — Browser Render Capacity & Backpressure

## Status

Implementation milestone. Browser rendering remains offline-snapshot-only and policy-gated. This milestone bounds simultaneous Chromium render jobs and the time a caller may wait for capacity; it does not add live navigation or a second browser backend.

## Seven-rule application

| Rule | M27 application |
|---|---|
| Implementasi | Add a semaphore-backed render capacity budget and bounded admission wait. |
| Adaptasi | Preserve the existing renderer, policy evaluation, CSP, JavaScript disablement, and per-operation timeout behavior. |
| Adopsi | Use Tokio's maintained semaphore and timeout primitives already present in the workspace. |
| Recreate | Expose VRTc-owned capacity settings and stable structured errors instead of leaking Tokio synchronization details. |
| Gunakan | Apply one shared capacity gate to every request handled by a BrowserRenderer instance. |
| Reverse engineering sistemnya | Model admission control as a bounded queue followed by a finite set of render slots; no proprietary browser internals are copied. |
| Tolak ukur sistematikanya | Validate positive configuration, queue-timeout behavior, bounded concurrency, and unchanged policy denial. |

## Configuration

BrowserRenderConfig adds:

- max_concurrent_renders: maximum simultaneous render operations for one renderer instance; default 2.
- max_queue_wait_ms: maximum time a caller waits for a render slot; default 5000.

Both values must be positive. max_concurrent_renders must also stay within Tokio Semaphore's supported permit range; oversized values are rejected rather than allowed to panic. Existing operation_timeout_ms and max_html_bytes validation remains in force.

Input-size and URL-policy checks happen before admission, so rejected requests do not consume a render slot. A slot is held from admission through browser launch, page rendering, and cleanup. If no slot becomes available before the queue-wait limit, the renderer returns BROWSER_RENDER_CAPACITY_TIMEOUT with ResourceExhausted. A closed semaphore returns BROWSER_RENDER_CAPACITY_CLOSED.

The capacity is per BrowserRenderer instance and process. It is not a distributed/global quota. Operators running multiple runtime instances must budget each instance accordingly.

## Security and compatibility constraints

- Never increase concurrency implicitly based on request volume.
- Keep capacity limits finite and configurable.
- Do not weaken policy evaluation, offline CSP, JavaScript disabling, input/output limits, or operation timeouts.
- Do not claim the queue is durable or distributed.
- The queue-wait budget is distinct from the existing per-operation timeout; it does not claim a whole-request wall-clock deadline.
- Preserve the existing offline snapshot-only contract.

## Acceptance criteria

1. Zero concurrency, zero queue-wait, and concurrency values above the semaphore permit limit are rejected as invalid configuration.
2. Concurrent render operations cannot exceed the configured semaphore permits.
3. Waiting for a permit is bounded by max_queue_wait_ms.
4. Queue timeout returns the stable BROWSER_RENDER_CAPACITY_TIMEOUT error.
5. A rejected URL is denied before browser capacity is acquired.
6. Existing renderer policy and snapshot behavior remain unchanged.
7. Workspace formatting, Clippy, and tests pass.

## Explicit limitations

- Capacity is local to one renderer instance.
- The renderer still launches and closes its Chromium process per accepted request.
- This milestone does not implement process pooling, browser reuse, distributed admission control, or live browsing.
