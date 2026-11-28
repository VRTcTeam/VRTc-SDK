# M34 — Fetcher TLS Handshake Integration Tests

**Status:** Implemented
**Engines:** `engine.vrtc.fetcher`, `engine.vrtc.network`
**Parent boundary:** M23 Network Infrastructure and M33 Fetcher TLS Minimum Enforcement

## Objective

M33 connected the Network Engine's minimum TLS policy to reqwest's client builder and verified the enum mapping and client construction. Those checks did not perform a TLS handshake. M34 adds focused local integration coverage proving that the configured minimum is enforced by the actual rustls-backed HTTP client.

## Implementation

- Extract the Fetcher's reqwest client configuration into one private builder helper used by production requests and the handshake tests.
- Start local OpenSSL test servers constrained to TLS 1.2 or TLS 1.3.
- Generate a short-lived test CA and a localhost server certificate at test runtime; the test client trusts only that test CA.
- Verify that a TLS 1.2 minimum accepts a TLS 1.2 server.
- Verify that a TLS 1.3 minimum rejects a TLS 1.2-only server.
- Verify that a TLS 1.3 minimum accepts a TLS 1.3 server.
- Keep all test traffic on loopback. No public endpoint, external provider, or network service is contacted.

## Security and compatibility

- The production Fetcher still resolves and validates public destinations before constructing a client, pins the selected address, revalidates redirects, and fails closed for unsupported proxy routes.
- The loopback address override and custom root certificate are confined to the test helper; they do not change the public Fetcher API or production trust configuration.
- The production builder retains request timeouts, disabled automatic redirects, disabled ambient proxy use, user-agent configuration, DNS pinning, and the route-specific minimum TLS version.
- Certificate and hostname validation remain enabled in the test client; the generated CA is trusted explicitly rather than disabling verification.
- No production dependency, public configuration field, or response contract is added.
- The focused handshake test requires the OpenSSL command-line tools. The Ubuntu GitHub Actions runner provides this prerequisite.

## Acceptance criteria

1. The shared client-builder helper is used by every production Fetcher hop.
2. A TLS 1.2-only server succeeds when the configured minimum is TLS 1.2.
3. A TLS 1.2-only server fails when the configured minimum is TLS 1.3.
4. A TLS 1.3 server succeeds when the configured minimum is TLS 1.3.
5. The focused test uses only loopback networking and a runtime-generated test CA.
6. Existing Fetcher URL validation, public-address DNS pinning, redirect policy, timeout, body-size, and proxy fail-closed behavior remain unchanged.
7. Rust formatting, targeted Fetcher tests, Clippy, and the repository CI checks pass before merge.

## Explicit non-goals

- Changing TLS policy defaults or adding TLS version configuration fields.
- Adding proxy transport or custom production trust roots.
- Bypassing public-address validation to make loopback fetching available.
- Testing browser navigation, subresource fetching, or external network availability.
- Treating successful TLS negotiation as proof that a remote service is otherwise trustworthy.
