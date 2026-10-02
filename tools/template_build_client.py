#!/usr/bin/env python3
"""Ordinary user: submit a context archive or public image; no host credentials."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import re
import secrets
import time
import urllib.parse


# A log page is bounded in source bytes. JSON can expand each byte to a
# six-byte escape (including replacement characters for invalid UTF-8).
# Keep ordinary status/error responses at their existing 64 KiB limit.
RESPONSE_BYTES = 65536
LOG_RESPONSE_BYTES = 6 * 65536 + 1024
LOG_PATH = re.compile(r"/sandproof/template-builds/[0-9a-f]{32}/logs")


class BuildClient:
    """Bounded authenticated calls; no retry of submissions or context uploads."""
    def __init__(self, url, key):
        self.url = urllib.parse.urlsplit(url)
        if (self.url.scheme not in ("http", "https") or not self.url.hostname
                or self.url.username or self.url.password or self.url.path not in ("", "/")
                or self.url.query or self.url.fragment):
            raise ValueError("service origin required")
        self.key = key

    def request(self, method, path, body=None, *, size=None, token=None):
        connection_class = http.client.HTTPSConnection if self.url.scheme == "https" else http.client.HTTPConnection
        connection = connection_class(self.url.hostname, self.url.port, timeout=30)
        headers = {"X-API-Key": self.key}
        if isinstance(body, dict):
            body = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if size is not None:
            headers["Content-Length"] = str(size)
        if token:
            headers["Idempotency-Key"] = token
        response_limit = RESPONSE_BYTES
        if method == "GET" and LOG_PATH.fullmatch(urllib.parse.urlsplit(path).path):
            response_limit = LOG_RESPONSE_BYTES
        try:
            connection.request(method, path, body, headers)
            response = connection.getresponse()
            data = response.read(response_limit + 1)
            if len(data) > response_limit:
                raise RuntimeError("oversized build response")
            value = json.loads(data)
            if response.status >= 400:
                raise RuntimeError(value.get("error", "BuildRequestFailed"))
            return value
        finally:
            connection.close()


def saved_intent(path, client, request, *, build_mode, token=None, context=None):
    """Persist the full retry domain before any uncertain build submission."""
    path = Path(path)
    if build_mode not in ("windowed", "legacy-unbounded"):
        raise ValueError("explicit supported build mode required")
    digest = None
    if context is not None:
        hasher = hashlib.sha256()
        with Path(context).open("rb") as incoming:
            while block := incoming.read(1024 * 1024):
                hasher.update(block)
        digest = hasher.hexdigest()
    expected = {"origin": urllib.parse.urlunsplit(client.url), "request": request, "context_sha256": digest,
                "build_mode": build_mode, "credential_sha256": hashlib.sha256(client.key.encode()).hexdigest()}
    if path.exists():
        if path.is_symlink() or path.stat().st_size > 65536:
            raise ValueError("invalid saved build intent")
        intent = json.loads(path.read_text())
        if any(intent.get(key) != value for key, value in expected.items()) or (token and intent.get("idempotency_key") != token):
            raise ValueError("saved build intent conflicts; never replace an uncertain request")
        if (not intent.get("idempotency_key") or (build_mode == "windowed"
                and not isinstance(intent.get("requestWindow"), dict))
                or (build_mode == "legacy-unbounded" and "requestWindow" in intent)):
            raise ValueError("saved intent lacks original request window/key")
        return intent
    intent = expected | {"idempotency_key": token or secrets.token_hex(16)}
    if build_mode == "windowed":
        opened = client.request("POST", "/sandproof/template-build-windows", {"durationMs": request["timeoutSeconds"] * 1000})
        intent["requestWindow"] = opened["requestWindow"]
    # Exclusive stable path: no replacing a prior invocation's retry identity.
    with path.open("x") as output:
        os.chmod(path, 0o600)
        json.dump(intent, output, sort_keys=True)
        output.flush()
        os.fsync(output.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return intent


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
    parser.add_argument("--intent", type=Path, required=True, help="durable retry identity; reuse this file after a lost response")
    parser.add_argument("--build-mode", choices=("windowed", "legacy-unbounded"), required=True,
                        help="explicit deployment mode; never downgrade an uncertain windowed request")
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    key = os.environ.get("SANDBOX_API_KEY")
    if not key:
        raise SystemExit("SANDBOX_API_KEY required; do not put credentials in arguments")
    client = BuildClient(args.url, key)
    body = {"kind": "oci" if args.image else "dockerfile", "timeoutSeconds": args.timeout,
            "allowInternetAccess": not args.offline,
            "resources": {"cpuCount": args.cpu, "memoryMiB": args.memory_mib, "diskMiB": args.disk_mib, "minFreeMiB": args.min_free_mib}}
    if args.image:
        body["image"] = args.image
    intent = saved_intent(args.intent, client, body, build_mode=args.build_mode,
                          token=args.idempotency_key, context=args.context)
    submission = body | ({"requestWindow": intent["requestWindow"]} if args.build_mode == "windowed" else {})
    result = client.request("POST", "/sandproof/template-builds", submission,
                            token=intent["idempotency_key"])
    path = "/sandproof/template-builds/" + result["buildID"]
    print(json.dumps(result), flush=True)
    if args.context and result["state"] == "AwaitingInput":
        with args.context.open("rb") as incoming:
            client.request("PUT", path + "/context", incoming, size=args.context.stat().st_size)
    deadline = time.monotonic() + args.timeout + 120
    while time.monotonic() < deadline:
        result = client.request("GET", path)
        if result["state"] in ("Ready", "Failed", "CleanupUnknown"):
            print(json.dumps(result), flush=True)
            if result["state"] != "Ready":
                raise SystemExit(1)
            return
        time.sleep(1)
    raise SystemExit("query deadline reached; original build ID remains valid, do not submit a replacement")


if __name__ == "__main__":
    main()
