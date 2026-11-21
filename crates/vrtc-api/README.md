# VRTc API

This crate exposes an Axum router for the VRTc Search Engine.

The host application supplies a configured SearchEngine and mounts the returned router. The crate does not open a port or configure authentication, TLS, rate limits, or provider credentials.

Available endpoints:

- POST /v1/search
- GET /v1/providers
- GET /v1/health

See the M4 HTTP API contract in ../../docs/specifications/m4-http-api.md.
