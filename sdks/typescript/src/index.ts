export interface SearchRequest {
  query: string;
  limit?: number;
  language?: string;
  domains?: string[];
  published_after?: string;
  published_before?: string;
}

export interface SearchResult {
  title: string;
  url: string;
  snippet?: string;
  score?: number;
  provider_id: string;
  published_at?: string;
  language?: string;
  metadata: Record<string, string>;
}

export interface ProviderSearchError {
  provider_id: string;
  code: string;
  message: string;
  retryable: boolean;
}

export interface SearchResponse {
  query: string;
  results: SearchResult[];
  providers_queried: string[];
  provider_errors: ProviderSearchError[];
}

export interface HealthResponse {
  status: string;
  service: string;
}

export interface VrtcClientOptions {
  baseUrl: string;
  fetchImpl?: typeof fetch;
}

export interface VrtcErrorPayload {
  code?: string;
  message?: string;
  category?: string;
  retryable?: boolean;
  component_id?: string;
  details?: Record<string, string>;
}

export class VrtcApiError extends Error {
  readonly status: number;
  readonly payload: VrtcErrorPayload;

  constructor(status: number, payload: VrtcErrorPayload) {
    super(payload.message ?? "VRTc API request failed");
    this.name = "VrtcApiError";
    this.status = status;
    this.payload = payload;
  }
}

/** Lightweight client for an operator-hosted VRTc HTTP API. */
export class VrtcClient {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: VrtcClientOptions) {
    const normalized = options.baseUrl.trim().replace(/\/+$/, "");
    if (!normalized) {
      throw new TypeError("baseUrl must not be empty");
    }
    this.baseUrl = normalized;
    this.fetchImpl = options.fetchImpl ?? fetch;
  }

  async search(request: SearchRequest, signal?: AbortSignal): Promise<SearchResponse> {
    return this.request<SearchResponse>("/v1/search", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(request),
      signal
    });
  }

  async providers(signal?: AbortSignal): Promise<string[]> {
    const response = await this.request<{ providers: string[] }>("/v1/providers", {
      method: "GET",
      signal
    });
    return response.providers;
  }

  async health(signal?: AbortSignal): Promise<HealthResponse> {
    return this.request<HealthResponse>("/v1/health", {
      method: "GET",
      signal
    });
  }

  private async request<T>(path: string, init: RequestInit): Promise<T> {
    const response = await this.fetchImpl(this.baseUrl + path, {
      ...init,
      headers: {
        Accept: "application/json",
        ...init.headers
      }
    });

    const body = await response.text();
    const payload = body ? JSON.parse(body) as unknown : undefined;

    if (!response.ok) {
      throw new VrtcApiError(
        response.status,
        (payload && typeof payload === "object" ? payload : {}) as VrtcErrorPayload
      );
    }

    return payload as T;
  }
}
