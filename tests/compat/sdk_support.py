"""Synthetic, credential-free fixtures shared by pinned E2B SDK tests."""

import os
from unittest.mock import patch


SANDBOX = {
    "templateID": "probe-template",
    "sandboxID": "probe-sandbox",
    "clientID": "probe-client",
    "envdVersion": "0.5.14",
    "envdAccessToken": "synthetic-token",
    "domain": "example.invalid",
}


def isolated_sdk_environment():
    """Remove ambient E2B credentials and proxies for one test."""
    safe_env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith("E2B_")
        and key.lower() not in {"http_proxy", "https_proxy", "all_proxy", "no_proxy"}
    }
    return patch.dict(os.environ, safe_env, clear=True)
