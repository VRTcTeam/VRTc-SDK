# M8 — Native HTML Parsing & Content Extraction

**Status:** Initial native implementation integrated into the Rust runtime  
**Engine ID:** `engine.vrtc.extract`  
**Capability:** `content.extract`

## Purpose

Turn fetched HTML into structured, readable content with metadata, headings, links, and source provenance. VRTc owns the extraction heuristics and output contract.

## Research basis

- Scrapy/StormCrawler separate fetching, parsing, and indexing.
- Trafilatura demonstrates the value of main-content extraction, metadata, and multiple output forms.
- Mozilla Readability-derived systems demonstrate candidate scoring using text density, link density, tag structure, and class/id hints.
- Rust's `scraper` crate provides an HTML5 parser and CSS selector interface. It is adopted as a parsing primitive; the VRTc extraction policy and result schema are implemented in this crate.

The selected parser is not the extraction engine by itself.

## Responsibilities

- Parse HTML into a browser-compatible document tree.
- Score likely content containers using text length, paragraph/headings density, link density, and positive/negative structural hints.
- Extract title, description, author, publication metadata, language, canonical URL, headings, links, and selected metadata.
- Produce normalized plain text and Markdown-style block output.
- Resolve relative links against the source URL and retain only HTTP(S) links.
- Enforce input byte, minimum text, link-count, and metadata-count limits.
- Report the extraction method and a heuristic quality estimate.

## Explicit non-responsibilities

- HTTP retrieval and redirect handling belong to the HTTP Fetcher.
- JavaScript execution belongs to the Browser & Rendering Engine.
- PDF and office document parsing belong to a separate document parser (Tika-style capability).
- This initial engine does not claim full Trafilatura/Readability parity and does not emit sanitized HTML.

## Security and quality notes

- HTML is untrusted input. This engine emits text/Markdown, not raw HTML intended for direct browser rendering.
- Metadata and links are source data and must remain associated with the source URL.
- The confidence field is a heuristic quality estimate, not a calibrated probability or a factuality score.
- The initial candidate scorer is intentionally transparent and deterministic; evaluation datasets and human correction are required before ranking it as production-grade.
- Extraction must not be used to circumvent login, paywalls, CAPTCHAs, robots rules, or access controls.

## Runtime integration

The engine is statically registered by `VrtcRuntime` and exposed through `extraction_engine()`. It declares `content.extract`. The public HTTP route and SDK client methods remain future API milestones.
