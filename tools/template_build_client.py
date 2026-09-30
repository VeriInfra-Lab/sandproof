#!/usr/bin/env python3
"""Ordinary user: submit a context archive or public image; no host credentials."""
import argparse
import http.client
import json
import os
from pathlib import Path
import time
import urllib.parse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", required=True)
    inputs = parser.add_mutually_exclusive_group(required=True)
    inputs.add_argument("--context", type=Path, help="tar or tar.gz with Dockerfile at its root")
    inputs.add_argument("--image", help="public Linux amd64 Debian/Ubuntu OCI reference")
    parser.add_argument("--cpu", type=int, default=1)
    parser.add_argument("--memory-mib", type=int, default=256)
    parser.add_argument("--disk-mib", type=int, default=1024)
    parser.add_argument("--min-free-mib", type=int, default=64)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--idempotency-key")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    key = os.environ.get("SANDBOX_API_KEY")
    if not key:
        raise SystemExit("SANDBOX_API_KEY required; do not put credentials in arguments")
    url = urllib.parse.urlsplit(args.url)
    if url.scheme not in ("http", "https") or url.username or url.password or url.path not in ("", "/") or url.query or url.fragment:
        raise SystemExit("service origin required")
    def request(method, path, body=None, size=None):
        connection_class = http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        connection = connection_class(url.hostname, url.port, timeout=30)
        headers = {"X-API-Key": key}
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if size is not None:
            headers["Content-Length"] = str(size)
        if args.idempotency_key:
            headers["Idempotency-Key"] = args.idempotency_key
        try:
            connection.request(method, path, body, headers)
            response = connection.getresponse()
            data = response.read(65537)
            if len(data) > 65536:
                raise RuntimeError("oversized build response")
            value = json.loads(data)
            if response.status >= 400:
                raise RuntimeError(value.get("error", "BuildRequestFailed"))
            return value
        finally:
            connection.close()
    body = {"kind": "oci" if args.image else "dockerfile", "timeoutSeconds": args.timeout,
            "allowInternetAccess": not args.offline,
            "resources": {"cpuCount": args.cpu, "memoryMiB": args.memory_mib, "diskMiB": args.disk_mib, "minFreeMiB": args.min_free_mib}}
    if args.image:
        body["image"] = args.image
    result = request("POST", "/sandproof/template-builds", body)
    path = "/sandproof/template-builds/" + result["buildID"]
    print(json.dumps(result), flush=True)
    if args.context and result["state"] == "AwaitingInput":
        with args.context.open("rb") as incoming:
            request("PUT", path + "/context", incoming, args.context.stat().st_size)
    deadline = time.monotonic() + args.timeout + 120
    while time.monotonic() < deadline:
        result = request("GET", path)
        if result["state"] in ("Ready", "Failed", "CleanupUnknown"):
            print(json.dumps(result), flush=True)
            if result["state"] != "Ready":
                raise SystemExit(1)
            return
        time.sleep(1)
    raise SystemExit("query deadline reached; original build ID remains valid, do not submit a replacement")


if __name__ == "__main__":
    main()
