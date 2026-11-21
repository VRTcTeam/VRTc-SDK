"""Small standard-library HTTP client for an operator-hosted VRTc API."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


class VrtcApiError(Exception):
    """Structured non-success response returned by the VRTc API."""

    def __init__(self, status: int, payload: dict[str, Any]) -> None:
        self.status = status
        self.payload = payload
        super().__init__(payload.get("message", "VRTc API request failed"))


class VrtcTransportError(Exception):
    """Network or transport failure while contacting the VRTc API."""


@dataclass
class SearchRequest:
    query: str
    limit: int = 0
    language: str | None = None
    domains: list[str] = field(default_factory=list)
    published_after: str | None = None
    published_before: str | None = None

    def to_payload(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "query": self.query,
            "limit": self.limit,
            "domains": self.domains,
        }
        if self.language is not None:
            payload["language"] = self.language
        if self.published_after is not None:
            payload["published_after"] = self.published_after
        if self.published_before is not None:
            payload["published_before"] = self.published_before
        return payload


@dataclass
class SearchResult:
    title: str
    url: str
    provider_id: str
    snippet: str | None = None
    score: float | None = None
    published_at: str | None = None
    language: str | None = None
    metadata: dict[str, str] = field(default_factory=dict)


@dataclass
class ProviderSearchError:
    provider_id: str
    code: str
    message: str
    retryable: bool


@dataclass
class SearchResponse:
    query: str
    results: list[SearchResult]
    providers_queried: list[str]
    provider_errors: list[ProviderSearchError]

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> SearchResponse:
        return cls(
            query=payload["query"],
            results=[SearchResult(**item) for item in payload.get("results", [])],
            providers_queried=payload.get("providers_queried", []),
            provider_errors=[
                ProviderSearchError(**item)
                for item in payload.get("provider_errors", [])
            ],
        )


@dataclass
class HealthResponse:
    status: str
    service: str


class VrtcClient:
    """Synchronous, dependency-free client for the VRTc HTTP API."""

    def __init__(self, base_url: str, timeout: float = 15.0) -> None:
        normalized = base_url.strip().rstrip("/")
        if not normalized:
            raise ValueError("base_url must not be empty")
        if timeout <= 0:
            raise ValueError("timeout must be positive")
        self.base_url = normalized
        self.timeout = timeout

    def search(self, request: SearchRequest | dict[str, Any]) -> SearchResponse:
        payload = request.to_payload() if isinstance(request, SearchRequest) else request
        response = self._request("/v1/search", payload)
        return SearchResponse.from_payload(response)

    def providers(self) -> list[str]:
        response = self._request("/v1/providers")
        return response.get("providers", [])

    def health(self) -> HealthResponse:
        response = self._request("/v1/health")
        return HealthResponse(**response)

    def _request(
        self, path: str, payload: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        data = json.dumps(payload).encode("utf-8") if payload is not None else None
        headers = {"Accept": "application/json"}
        if data is not None:
            headers["Content-Type"] = "application/json"

        request = Request(
            self.base_url + path,
            data=data,
            headers=headers,
            method="POST" if data is not None else "GET",
        )

        try:
            with urlopen(request, timeout=self.timeout) as response:
                body = response.read()
        except HTTPError as error:
            body = error.read()
            try:
                error_payload = json.loads(body.decode("utf-8")) if body else {}
            except (UnicodeDecodeError, json.JSONDecodeError):
                error_payload = {}
            raise VrtcApiError(error.code, error_payload) from error
        except URLError as error:
            raise VrtcTransportError(str(error.reason)) from error

        try:
            decoded = json.loads(body.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise VrtcTransportError("VRTc API returned invalid JSON") from error

        if not isinstance(decoded, dict):
            raise VrtcTransportError("VRTc API returned an unexpected JSON shape")
        return decoded
