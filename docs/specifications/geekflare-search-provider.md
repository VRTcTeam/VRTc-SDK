# Geekflare Search Provider Adapter

**Provider ID:** `provider.geekflare.search`  
**Capability:** `search.query`  
**Connection:** Remote API  
**Endpoint:** `POST https://api.geekflare.com/search`

## Scope

This adapter integrates Geekflare's Search API with VRTc's provider-neutral `SearchProvider` contract. It starts with standard structured search results and supports web, news, and image source selection. Geekflare remains an optional provider, not a dependency of the VRTc core.

## Authentication

The host application supplies the API key to `GeekflareSearchProvider::new` or `with_options`. The adapter sends it in the `x-api-key` header. Credentials are never included in provider descriptors, search payloads, result metadata, or formatted errors. Provider Debug output redacts the key.

Do not commit API keys. The host should read `GEEKFLARE_API_KEY` from its own secret/configuration mechanism and pass the value into the constructor. No `.env` file is required or read by this adapter.

## Request mapping

The adapter sends JSON to `/search` with:

- `query`: canonical VRTc query;
- `limit`: VRTc result limit, defaulting to 10 when unset;
- `source`: configured web, news, or images source;
- optional `location`, `city`, `time`, and `category`;
- `includeDomains`: VRTc request domain allowlist;
- optional `excludeDomains`;
- `format: json`;
- optional page extraction controlled by `scrape` and `scrapeLimit`.

The adapter uses a bounded HTTP client timeout. VRTc's Search Engine continues to enforce its own per-provider timeout and concurrency limits.

## Result normalization

The standard Geekflare response's `data` array is mapped to VRTc SearchResult:

- `title` and `url` are required;
- `snippet` becomes the VRTc snippet;
- `date` is retained as raw metadata; it is not promoted to VRTc's RFC 3339 publication timestamp because Geekflare may return date-only values;
- `position` is retained as metadata and converted into a monotonic rank score (`1 / position`);
- scraped `content`, when returned, is retained in metadata.

The Search Engine assigns authoritative provider provenance from the registered adapter ID and applies its normal filters, deduplication, ranking, and result limit.

Grounded-answer and full Google SERP response shapes are not enabled in this first adapter version because they are not the standard result-array contract. They can be added as distinct capabilities later.

## Error handling

- 401/403: credential or permission failure;
- 402: account credits exhausted;
- 429: provider rate limit;
- timeout: retryable timeout;
- 5xx: retryable upstream error;
- malformed or unexpected response: safe contract error.

Response bodies and credential values are not copied into error messages.

## Registration

```rust
use std::sync::Arc;
use vrtc_core::RegistrationSource;
use vrtc_provider_geekflare::GeekflareSearchProvider;
use vrtc_runtime::{RuntimeConfig, VrtcRuntime};

let runtime = VrtcRuntime::new(RuntimeConfig::default())?;
let api_key = std::env::var("GEEKFLARE_API_KEY")?;
let provider = GeekflareSearchProvider::new(api_key)?;
let descriptor = provider.descriptor(RegistrationSource::Dynamic);

runtime.register_search_provider(
    descriptor,
    Arc::new(provider),
    RegistrationSource::Dynamic,
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Credits and operations

Geekflare documents a shared credit pool across its APIs. Standard Search API requests cost 2 credits; optional scraped pages and grounded answers incur additional costs. The adapter does not retry rate limits or exhausted credits automatically. Operators should monitor provider usage and configure budget controls.
