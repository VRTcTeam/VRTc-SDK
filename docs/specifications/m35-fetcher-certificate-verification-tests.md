# M35 — Fetcher Certificate & Hostname Verification Tests

**Status:** Implemented; CI verification pending  
**Engines:** `engine.vrtc.fetcher`, `engine.vrtc.network`  
**Parent boundary:** M23 Network Infrastructure, M33 Fetcher TLS Minimum Enforcement, and M34 Fetcher TLS Handshake Integration Tests

## Objective

M34 proves that the Fetcher's configured TLS minimum is negotiated by a real local handshake. M35 closes the complementary verification gap: the integration suite must also prove that the transport rejects an untrusted certificate authority and a certificate whose subject alternative name does not match the requested hostname.

## Implementation

- Generalize the local handshake test helper to accept a requested hostname and an explicit test-CA trust choice.
- Verify that a server certificate signed by the generated test CA is rejected when that CA is not added to the test client's trust store.
- Verify that a client explicitly trusting the generated test CA still rejects the same certificate when the request hostname differs from its `DNS:localhost` subject alternative name.
- Keep the existing positive TLS 1.2 and TLS 1.3 handshake cases unchanged.
- Pin the alternate test hostname to the local server address through the same private reqwest builder used by production code.

## Security and compatibility

- The additional cases use only loopback networking and runtime-generated test certificates.
- The generated test CA is added only to the positive-control test client; no production trust store is changed.
- Certificate-chain and hostname validation remain enabled. No `danger_accept_invalid_certs` or equivalent bypass is introduced.
- Production DNS validation, public-address pinning, redirect revalidation, TLS minimum selection, and proxy fail-closed behavior are unchanged.
- No production dependency, public API, or response contract is added.
- The focused tests retain M34's OpenSSL CLI prerequisite.

## Acceptance criteria

1. A TLS 1.3 server with a certificate signed by the test CA succeeds when that CA is explicitly trusted and the hostname matches.
2. The same class of server certificate fails when the test CA is not trusted.
3. A certificate trusted through the test CA fails when the requested hostname does not match the certificate SAN.
4. The existing TLS 1.2/TLS 1.3 minimum-version handshake cases continue to pass.
5. Rust formatting, targeted Fetcher tests, Clippy, and repository CI pass before merge.

## Explicit non-goals

- Disabling or weakening production certificate or hostname validation.
- Adding custom production root certificates or user-configurable trust stores.
- Allowing loopback or private-address fetching through the production Fetcher.
- Adding proxy transport, new TLS policy fields, or new public SDK methods.
- Treating a successful handshake as proof that a remote endpoint is otherwise trustworthy.
