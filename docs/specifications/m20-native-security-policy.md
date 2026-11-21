# M20 — Native Security & Retrieval Policy Engine

**Status:** Initial URL and crawl policy engine integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.policy`  
**Capability:** `policy.evaluate`

## Purpose

M20 centralizes reusable policy decisions for search, retrieval, and crawling. It is a VRTc-owned policy engine, not a network fetcher and not an access-control bypass.

## Native policy checks

- HTTP and HTTPS scheme allow/deny configuration.
- URL length bounds and rejection of embedded credentials.
- Exact and subdomain-aware host allow/deny rules.
- Blocking of localhost, local names, and literal non-public IP addresses by default.
- Same-origin crawl scope by default, with explicit allowed-origin configuration.
- Maximum crawl depth.
- Explicit robots approval before crawl URLs are allowed.
- URL fragment removal in normalized allow decisions.

## Separation of responsibilities

- **Policy Engine:** evaluates configured scheme, host, origin, depth, and robots-approval constraints.
- **HTTP Fetcher:** remains responsible for DNS resolution, public-address validation and pinning, redirect revalidation, HTTPS downgrade prevention, and response bounds.
- **Crawler:** remains responsible for robots.txt retrieval/parsing, caching, crawl frontier, politeness, and same-origin crawl orchestration.

A hostname that resolves to a private address cannot be safely validated by string matching alone. The Fetcher's DNS-time checks remain mandatory; M20 does not replace them.

## Robots semantics

When crawl policy requires robots approval:
- `robots_allowed = None` returns a denial with `requires_robots_evaluation = true`;
- `robots_allowed = Some(false)` returns a denial;
- `robots_allowed = Some(true)` allows the request if all other policy checks pass.

Only the Crawler should provide the robots result after applying the relevant user-agent rules. The policy engine does not fetch robots.txt itself.

## Runtime integration

The runtime statically registers `engine.vrtc.policy` / `policy.evaluate` and exposes `policy_engine()`.

## Boundaries

- No DNS resolver or network requests.
- No authentication/session bypass.
- No CAPTCHA, paywall, login, or access-control bypass.
- No policy override for robots.txt or site terms.
- No distributed policy synchronization or signed policy bundle yet.
