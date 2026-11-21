# M26 — Browser Engine Abstraction

## Status

Implementation milestone. The first concrete backend is the existing Chromium renderer powered by `chromiumoxide`. WebKit, Gecko, Playwright, remote browser services, and live navigation are **not** claimed as implemented by this milestone.

## Objective

Separate the stable VRTc browser-rendering contract from the selected browser implementation. Callers select an engine by stable ID and consume the same request/result types. Backend metadata and capabilities are explicit; unsupported features must not be inferred from a browser family name.

## Seven-rule evaluation

| Rule | M26 application |
|---|---|
| Implementasi | Define the `BrowserEngine` trait, engine metadata/capabilities, and in-process registry. |
| Adaptasi | Keep the existing isolated Chromium snapshot renderer as the first adapter without changing its policy boundary. |
| Adopsi | Reuse the maintained `chromiumoxide` CDP client already present in the workspace. |
| Recreate | Own VRTc's stable engine identity, dispatch, capability model, and error contract rather than exposing CDP types. |
| Gunakan | Preserve the current `BrowserRenderer` API and route optional callers through `BrowserEngineRegistry`. |
| Reverse engineering sistemnya | Model the browser lifecycle as an adapter boundary: identity, capability declaration, request dispatch, result, and failure. No private/proprietary engine internals are copied. |
| Tolak ukur sistematikanya | Evaluate contract stability, policy preservation, capability honesty, dispatch behavior, bounded rendering, and backend replaceability. |

## Contract

- `BrowserEngine` is a `Send + Sync` trait exposing `info()` and asynchronous `render()`.
- `BrowserEngineInfo` carries a stable ID, display name, package version, family, and explicit capabilities.
- `BrowserEngineRegistry` stores trusted Rust implementations in-process, rejects empty identity fields and duplicate IDs, exposes metadata, and dispatches rendering by ID.
- Registry locks are released before asynchronous rendering begins.
- Unknown engine IDs return a structured `BROWSER_ENGINE_NOT_FOUND` error.
- The registry is not a dynamic library loader, plugin sandbox, remote service client, or permission bypass.

## Initial backend

The existing `BrowserRenderer` implements `BrowserEngine` as `browser.chromiumoxide`. It declares:
- fetched-snapshot rendering: supported;
- JavaScript control: supported (JavaScript is disabled for this offline renderer);
- remote subresource blocking: supported by the injected restrictive CSP;
- live navigation: unsupported.

The renderer continues to accept already-fetched HTML, evaluate `PolicyEngine`, create an `about:blank` page, disable scripts, apply offline CSP, enforce input/output size limits, and enforce operation timeouts. The source URL remains provenance/policy input, not a navigation target.

## Security and compatibility constraints

- Do not weaken the existing policy gate, CSP, script disabling, timeout, or byte limits when adding an engine.
- Browser engine family is descriptive, not proof of capabilities.
- New backends must declare capabilities based on verified implementation behavior.
- Future remote engines require explicit network policy, authentication handling, secret redaction, bounded requests, and failure isolation.
- Preserve the existing `BrowserRenderer::new` and `BrowserRenderer::render` entry points for compatibility.
- No WebKit/Gecko dependency is introduced until platform support, license, maintenance, isolation, and benchmark evidence are reviewed.

## Acceptance criteria

1. Existing Chromium rendering remains behind the same policy and offline-snapshot restrictions.
2. The backend implements the common trait without leaking CDP types into callers.
3. Registry registration rejects duplicate IDs and invalid identity.
4. Registry metadata reports only capabilities implemented by the backend.
5. Dispatch resolves the selected backend and returns structured errors for unknown IDs.
6. Registry does not hold its lock across an async render call.
7. Focused tests cover metadata, duplicate registration, unknown selection, and policy preservation.
8. Workspace CI (format, Clippy, and tests) passes before merge.

## Follow-on work

- Add a second backend only after a separate compatibility and security review.
- Evaluate WebKit and Gecko as independent adapters; do not pretend they are drop-in compatible.
- Add engine-specific resource isolation and lifecycle controls before supporting live browsing.
- Add benchmark corpus for snapshot fidelity, memory, startup latency, and policy enforcement.
