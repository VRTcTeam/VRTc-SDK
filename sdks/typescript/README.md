# @vrtc/sdk

Official TypeScript client for an operator-hosted VRTc HTTP API. Requires Node.js 18 or newer.

## Install

```sh
npm install ./sdks/typescript
```

## Usage

```ts
import { VrtcClient } from "@vrtc/sdk";

const vrtc = new VrtcClient({ baseUrl: "http://localhost:8080" });

const health = await vrtc.health();
const providers = await vrtc.providers();
const results = await vrtc.search({
  query: "WCAG 2.2 accessibility",
  limit: 10,
  domains: ["w3.org"]
});
```

Pass an AbortSignal to cancel an individual HTTP request. API errors are surfaced as VrtcApiError with the HTTP status and structured response payload.

The client does not embed provider credentials or bypass host authentication. The API host is responsible for TLS, authentication, authorization, and rate limiting.
