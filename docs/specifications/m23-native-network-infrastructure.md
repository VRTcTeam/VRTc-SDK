# M23 — Native DNS, TLS Policy & Network Route Planning

**Status:** Initial native network infrastructure engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.network`  
**Capability:** `network.resolve-route`

## Scope

M23 introduces a VRTc-owned network infrastructure boundary for DNS resolution, TLS policy validation, and explicit outbound route planning.

- DNS lookup has a configurable timeout, deterministic address ordering, duplicate removal, and a public-address policy. When public-only mode is enabled, the entire DNS answer set is rejected if any returned address is non-public.
- TLS policy requires certificate and hostname validation. The initial minimum protocol policy is TLS 1.2 or TLS 1.3.
- Proxy endpoints are validated as explicit HTTP(S) endpoints. Host rules select a configured enabled proxy; unmatched destinations use direct routing.
- Proxy selection returns a route plan. It does not secretly change the HTTP Fetcher transport or tunnel traffic.

## Security boundaries

- DNS public-address filtering is a defense-in-depth check, not a complete defense against DNS rebinding. Callers must pin the validated address for the actual connection and repeat validation for every redirect.
- The Fetcher currently performs its own DNS resolution and pins one validated address per request hop. M23 does not silently replace that behavior.
- TLS certificate validation cannot be disabled through this engine. Custom trust roots and certificate pinning are future work.
- Proxy credentials are not accepted in route endpoint configuration. Secret handling and authenticated proxy support require a dedicated secret-provider contract.
- Route planning does not establish a proxy connection. A transport integration must explicitly consume the selected route and preserve DNS/IP policy, TLS validation, redirect checks, and timeout limits.
- The engine does not provide anonymity, bypass access controls, or evade service restrictions.

## Runtime integration

The runtime statically registers `engine.vrtc.network` and exposes `network_engine()`. The engine provides `resolve()` and `plan_route()` for explicit caller use.

## Deferred work

- Integrate a policy-enforcing resolver and selected proxy route into the Fetcher transport.
- Add DNS-over-HTTPS / DNS-over-TLS as opt-in resolver backends with independent trust and privacy decisions.
- Add certificate-chain diagnostics and configurable private trust stores.
- Add connection pooling and per-route health/circuit-breaker telemetry.
- Add authenticated proxy support through a secret-provider interface.
