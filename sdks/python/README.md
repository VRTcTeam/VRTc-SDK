# vrtc-sdk

Official Python client for an operator-hosted VRTc HTTP API. Python 3.10+; runtime uses only the standard library.

## Install

```sh
pip install vrtc-sdk
```

## Usage

```python
from vrtc_sdk import SearchRequest, VrtcClient

vrtc = VrtcClient("http://localhost:8080")
print(vrtc.health())
print(vrtc.providers())

response = vrtc.search(
    SearchRequest(
        query="WCAG 2.2 accessibility",
        limit=10,
        domains=["w3.org"],
    )
)
for result in response.results:
    print(result.title, result.url, result.provider_id)
```

HTTP errors raise VrtcApiError with the status and structured API payload. Network and invalid-JSON failures raise VrtcTransportError.

The client does not embed provider credentials or bypass host authentication. The API host is responsible for TLS, authentication, authorization, and rate limiting.
