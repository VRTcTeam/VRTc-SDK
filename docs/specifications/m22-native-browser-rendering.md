# M22 — Native Browser & Rendering Engine

**Status:** Initial isolated offline HTML snapshot renderer integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.browser`  
**Capability:** `browser.render`

## Research and adoption basis

- Chromium Headless provides browser-grade HTML/CSS layout and DOM rendering. The browser executable remains a separately installed runtime dependency.
- `chromiumoxide` provides an async Rust client for the Chrome DevTools Protocol (CDP), licensed MIT OR Apache-2.0. It is used as a local process/control library, not exposed as a public remote-debugging service.
- The renderer combines browser layout with the VRTc Policy Engine and offline document loading. It does not use Chromium as a replacement for the Fetcher or Crawler.

## Initial native capability

The renderer accepts an HTML snapshot already retrieved by the HTTP Fetcher, plus its source URL and crawl-policy context. It then:
1. evaluates the source URL and crawl context through the native Policy Engine;
2. launches a local headless Chromium process;
3. opens an `about:blank` page rather than navigating to the source URL;
4. disables JavaScript execution;
5. injects a restrictive CSP that blocks network connections, frames, objects, forms, scripts, and non-data external resources;
6. loads the supplied HTML snapshot and returns the rendered DOM, title, byte count, and elapsed time.

## Security boundary

- The browser renderer does not perform network fetches.
- The renderer does not execute page JavaScript.
- The renderer does not follow redirects or navigate to arbitrary URLs.
- The source URL is policy-checked as provenance/context only.
- Input and output HTML size and browser operations are bounded by configuration.
- A local Chromium/Chrome binary must already be installed or explicitly configured. The crate does not download browser binaries automatically.
- The CDP connection is internal to the renderer and must not be exposed to untrusted clients.
- The browser renderer consumes fetched HTML; it does not bypass robots, authentication, access controls, or site terms.

## Not yet implemented

- Dynamic JavaScript execution.
- Browser-side network request interception through the native Fetcher.
- Secure redirect and subresource revalidation for network-enabled browser sessions.
- Persistent browser pools, contexts, cookies, storage, screenshots, PDF output, and device emulation.
- Browser crash recovery and multi-process worker scheduling.
- Automatic browser rendering from crawler discoveries.

Dynamic rendering must remain disabled until every top-level navigation, redirect, iframe, worker, and subresource request is routed through a policy-enforcing network boundary. A browser process must not be treated as a replacement for the Fetcher or Policy Engine.

## Runtime integration

The runtime statically registers `engine.vrtc.browser` / `browser.render` and exposes `browser_renderer()`.

## Licensing

VRTc depends on the upstream `chromiumoxide` crate under its MIT OR Apache-2.0 license. Chromium itself remains a separately installed executable governed by its own distribution terms.
