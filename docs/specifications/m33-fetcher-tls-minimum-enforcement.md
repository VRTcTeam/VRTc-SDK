# M33 — Fetcher TLS Minimum Enforcement

**Status:** Implemented  
**Engines:** `engine.vrtc.fetcher`, `engine.vrtc.network`  
**Parent boundary:** M23 Network Infrastructure and M24 Fetcher & Network Integration

## Objective

Apply the Network Engine's configured minimum TLS version to the actual HTTP transport. Before M33, the Network Engine validated TLS policy and included the minimum version in each route plan, but the Fetcher's reqwest connector did not consume that value.

## Implementation

- Map `TlsMinimumVersion::Tls12` to reqwest's `tls::Version::TLS_1_2`.
- Map `TlsMinimumVersion::Tls13` to reqwest's `tls::Version::TLS_1_3`.
- Apply the mapped value through `ClientBuilder::min_tls_version` when constructing each per-hop HTTP client.
- Rebuild the client after every redirect and apply the newly planned route's TLS policy for that hop.
- Keep reqwest's rustls backend and normal certificate-chain and hostname verification enabled.
- Add focused tests for both enum mappings and successful construction of clients with each supported minimum.

## Seven-rule application

| Rule | M33 application |
|---|---|
| Implementasi | Connect the existing VRTc TLS policy to the actual Fetcher transport configuration. |
| Adaptasi | Use reqwest's supported TLS configuration API behind the existing Fetcher boundary. |
| Adopsi | Reuse the workspace's existing reqwest/rustls dependency; add no TLS or proxy dependency. |
| Recreate | Preserve VRTc-owned policy semantics and structured errors without exposing transport internals. |
| Gunakan | Apply the shared Network Engine route decision for each request hop. |
| Reverse engineering sistemnya | Rely on public reqwest API documentation and the declared workspace feature configuration. |
| Tolak ukur sistematikanya | Verify both policy mappings and that both TLS configurations produce valid reqwest clients; retain workspace checks as integration validation. |

## Security and compatibility

- Certificate and hostname verification remain mandatory; M33 does not add an invalid-certificate option.
- The Fetcher continues to reject proxy routes with `PROXY_TRANSPORT_NOT_AVAILABLE`; M33 does not enable proxy transport.
- Public-address DNS validation, address pinning, redirect revalidation, HTTPS downgrade blocking, request timeouts, and response-size limits remain unchanged.
- HTTP URLs do not perform a TLS handshake; the configured minimum applies when the client establishes HTTPS connections.
- No public FetchConfig fields, runtime configuration fields, or public response contracts change.
- No new dependencies are introduced.

## Acceptance criteria

1. TLS 1.2 policy maps to reqwest TLS 1.2.
2. TLS 1.3 policy maps to reqwest TLS 1.3.
3. Reqwest client construction accepts both supported minimum versions with the workspace rustls feature.
4. Every redirect hop applies the TLS minimum from that hop's Network Engine route.
5. Certificate and hostname validation remain enabled.
6. Existing DNS pinning, proxy fail-closed behavior, redirect checks, and body/time limits remain unchanged.
7. Rust formatting, Clippy, workspace tests, SDK checks, repository/specification checks, CodeQL, and Dependency Review complete successfully before merge.

## Explicit non-goals

- Implementing proxy transport or authenticated proxies.
- Changing DNS resolution or address-pinning behavior.
- Enabling browser live navigation, JavaScript execution, or subresource fetching.
- Adding custom trust roots, certificate pinning, or TLS diagnostic export.
- Changing the public Fetcher API or introducing new configuration knobs.
