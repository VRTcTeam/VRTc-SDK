import json
import unittest
from io import BytesIO
from unittest.mock import patch
from urllib.error import HTTPError

from vrtc_sdk import SearchRequest, VrtcApiError, VrtcClient


class FakeResponse:
    def __init__(self, payload):
        self.body = json.dumps(payload).encode("utf-8")

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        return False

    def read(self):
        return self.body


class VrtcClientTests(unittest.TestCase):
    @patch("vrtc_sdk.client.urlopen")
    def test_search_sends_request_and_parses_response(self, urlopen):
        urlopen.return_value = FakeResponse({
            "query": "VRTc",
            "results": [{
                "title": "Docs",
                "url": "https://example.org/docs",
                "provider_id": "provider.test",
                "metadata": {},
            }],
            "providers_queried": ["provider.test"],
            "provider_errors": [],
        })

        response = VrtcClient("http://localhost:8080").search(
            SearchRequest(query="VRTc", domains=["example.org"])
        )

        self.assertEqual(response.query, "VRTc")
        self.assertEqual(response.results[0].provider_id, "provider.test")
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, "http://localhost:8080/v1/search")
        self.assertEqual(json.loads(request.data)["domains"], ["example.org"])

    @patch("vrtc_sdk.client.urlopen")
    def test_http_error_preserves_structured_payload(self, urlopen):
        body = json.dumps({"code": "NO_SEARCH_PROVIDERS", "message": "No providers"}).encode()
        urlopen.side_effect = HTTPError(
            "http://localhost:8080/v1/search",
            503,
            "Unavailable",
            {},
            BytesIO(body),
        )

        with self.assertRaises(VrtcApiError) as raised:
            VrtcClient("http://localhost:8080").search(SearchRequest(query="test"))

        self.assertEqual(raised.exception.status, 503)
        self.assertEqual(raised.exception.payload["code"], "NO_SEARCH_PROVIDERS")

    @patch("vrtc_sdk.client.urlopen")
    def test_health_endpoint(self, urlopen):
        urlopen.return_value = FakeResponse({"status": "ok", "service": "vrtc-search"})
        health = VrtcClient("http://localhost:8080").health()
        self.assertEqual(health.status, "ok")


if __name__ == "__main__":
    unittest.main()
