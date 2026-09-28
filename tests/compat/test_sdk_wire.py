"""Pinned E2B Python SDK against synthetic local HTTP responses, not a VM."""

import asyncio
from dataclasses import replace
import importlib.metadata
import json
import threading
import tomllib
import unittest
from datetime import datetime, timezone
from urllib.parse import parse_qs, quote, urlencode, urlsplit
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from e2b import AsyncSandbox, Sandbox, SandboxQuery, SandboxState
from e2b.exceptions import (
    AuthenticationException,
    RateLimitException,
    SandboxException,
    SandboxNotFoundException,
    ServiceBusyException,
)

from sdk_support import SANDBOX, isolated_sdk_environment
from sdk_network_samples import (
    DETAIL_FIELDS, DETAIL_NETWORK, PROFILES, validate_base_details, validate_network,
)


class Recorder(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def respond(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.server.records.append(
            {
                "method": self.command,
                "path": self.path,
                "body": json.loads(body) if body else None,
                "header_names": sorted(name.lower() for name in self.headers),
            }
        )
        if not self.server.responses:
            self.send_error(500, "unexpected request")
            return
        status, response, headers = self.server.responses.pop(0)
        encoded = json.dumps(response).encode() if response is not None else b""
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        for name, value in headers.items():
            self.send_header(name, value)
        self.end_headers()
        self.wfile.write(encoded)

    do_POST = respond
    do_DELETE = respond
    do_GET = respond
    do_PUT = respond


class WireContract(unittest.TestCase):
    def setUp(self):
        self.assertEqual(importlib.metadata.version("e2b"), "2.51.0")
        # Even a developer with real E2B credentials must only hit loopback.
        self.env_patch = isolated_sdk_environment()
        self.env_patch.start()
        self.addCleanup(self.env_patch.stop)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Recorder)
        self.server.records = []
        self.server.responses = []
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)
        url = f"http://127.0.0.1:{self.server.server_port}"
        self.options = {
            "api_url": url,
            "sandbox_url": url,
            "api_key": "synthetic-local-only",
            "domain": "example.invalid",
            "debug": False,
            "request_timeout": 5,
            "retries": 0,
        }

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)

    def tearDown(self):
        self.assertEqual(self.server.responses, [], "SDK did not consume expected responses")

    @property
    def records(self):
        return self.server.records

    @property
    def responses(self):
        return self.server.responses

    def test_sync_async_detail_network_lifecycle_and_legacy_fields(self):
        common = {**SANDBOX, **DETAIL_FIELDS, "startedAt": "2026-01-01T00:00:00Z",
                  "endAt": "2026-01-01T00:05:00Z", "state": "running",
                  "metadata": {"job": "雪"}}
        for cls in (Sandbox, AsyncSandbox):
            for internet in (None, False, True):
                sample = replace(DETAIL_NETWORK, internet=internet)
                raw = {**common, "allowInternetAccess": internet, "network": sample.wire_network}
                self.responses.append((200, raw, {}))
                info = cls.get_info(SANDBOX["sandboxID"], **self.options)
                if cls is AsyncSandbox:
                    info = asyncio.run(info)
                validate_network(info, raw, sample)
                validate_base_details(info, raw, common["metadata"])
            legacy = {key: value for key, value in common.items() if key not in ("domain", "envdAccessToken", "lifecycle")}
            self.responses.append((200, legacy, {}))
            info = cls.get_info(SANDBOX["sandboxID"], **self.options)
            if cls is AsyncSandbox:
                info = asyncio.run(info)
            self.assertIsNone(info.allow_internet_access)
            self.assertIsNone(info.network)
            self.assertIsNone(info.lifecycle)
            self.assertIsNone(info.sandbox_domain)

    def test_sync_async_list_filters_and_opaque_pagination_wire(self):
        metadata = {"reserved &+%=": "雪 =+&/%", "group": "red"}
        started = datetime(2026, 1, 2, 3, 4, 5, 123456, tzinfo=timezone.utc)
        for cls in (Sandbox, AsyncSandbox):
            self.responses.extend([(200, [], {"X-Next-Token": "opaque-next"}), (200, [], {})])
            paginator = cls.list(
                query=SandboxQuery(metadata=metadata, state=[SandboxState.RUNNING, SandboxState.PAUSED],
                                   started_after=started, template="base"),
                limit=2, order="asc", **self.options,
            )
            if cls is Sandbox:
                self.assertEqual(paginator.next_items(), [])
                self.assertTrue(paginator.has_next)
                self.assertEqual(paginator.next_items(), [])
            else:
                async def pages():
                    self.assertEqual(await paginator.next_items(), [])
                    self.assertTrue(paginator.has_next)
                    self.assertEqual(await paginator.next_items(), [])
                asyncio.run(pages())
            self.assertFalse(paginator.has_next)
            self.assertIsNone(paginator.next_token)
            first, following = self.records[-2:]
            self.assertEqual(first["method"], "GET")
            self.assertEqual(urlsplit(first["path"]).path, "/v2/sandboxes")
            params = parse_qs(urlsplit(first["path"]).query)
            self.assertEqual(params["metadata"], [urlencode({quote(k): quote(v) for k, v in metadata.items()})])
            self.assertEqual(params["state"], ["running,paused"])
            self.assertEqual(params["order"], ["asc"])
            self.assertEqual(params["template"], ["base"])
            self.assertEqual(params["limit"], ["2"])
            self.assertEqual(datetime.fromisoformat(params["startedAfter"][0]), started)
            self.assertEqual(parse_qs(urlsplit(following["path"]).query), {**params, "nextToken": ["opaque-next"]})

    def test_create_omits_server_defaults(self):
        self.responses.append((201, SANDBOX, {}))
        sandbox = Sandbox.create(**self.options)
        self.assertEqual(sandbox.sandbox_id, "probe-sandbox")
        self.assertEqual(len(self.records), 1)
        self.assertEqual(self.records[0]["method"], "POST")
        self.assertEqual(self.records[0]["path"], "/v2/sandboxes")
        for key in ("timeout", "allow_internet_access", "secure", "autoPause"):
            self.assertNotIn(key, self.records[0]["body"])

    def test_create_explicit_values_and_deprecated_secure(self):
        self.responses.append((201, SANDBOX, {}))
        Sandbox.create(
            template="probe-template", timeout=86400,
            allow_internet_access=False, secure=False, **self.options
        )
        self.assertEqual(self.records[0]["body"]["timeout"], 86400)
        self.assertIs(self.records[0]["body"]["allow_internet_access"], False)
        self.assertNotIn("secure", self.records[0]["body"])

    def test_create_static_ipv4_network_wire(self):
        for cls in (Sandbox, AsyncSandbox):
            for sample in PROFILES:
                with self.subTest(sdk=cls.__name__, profile=sample.name):
                    self.responses.append((201, SANDBOX, {}))
                    sandbox = cls.create(**sample.options, **self.options)
                    if cls is AsyncSandbox:
                        sandbox = asyncio.run(sandbox)
                    body = self.records[-1]["body"]
                    if 'allow_internet_access' in sample.options:
                        self.assertIs(body['allow_internet_access'], sample.internet)
                    else:
                        self.assertNotIn('allow_internet_access', body)
                    if sample.wire_network is None:
                        self.assertNotIn('network', body)
                    else:
                        self.assertEqual(body['network'], sample.wire_network)
                    raw = {**SANDBOX, **DETAIL_FIELDS,
                           'startedAt': '2026-01-01T00:00:00Z',
                           'endAt': '2026-01-01T00:05:00Z', 'state': 'running',
                           'metadata': {}, 'allowInternetAccess': sample.internet}
                    if sample.wire_network is not None:
                        raw['network'] = sample.wire_network
                    self.responses.append((200, raw, {}))
                    info = sandbox.get_info()
                    if cls is AsyncSandbox:
                        info = asyncio.run(info)
                    validate_network(info, raw, sample)

    def test_connect_uses_v2_and_preserves_explicit_zero(self):
        self.responses.extend([(200, SANDBOX, {}), (200, SANDBOX, {})])
        Sandbox.connect("probe-sandbox", **self.options)
        Sandbox.connect("probe-sandbox", timeout=0, **self.options)
        self.assertEqual(len(self.records), 2)
        self.assertTrue(all(r["method"] == "POST" for r in self.records))
        self.assertEqual(self.records[0]["path"], "/v2/sandboxes/probe-sandbox/connect")
        self.assertNotIn("timeout", self.records[0]["body"])
        self.assertEqual(self.records[1]["body"]["timeout"], 0)

    def test_kill_success_and_not_found(self):
        self.responses.extend(
            [(204, None, {}), (404, {"code": 404, "message": "missing"}, {})]
        )
        self.assertIs(Sandbox.kill("probe-sandbox", **self.options), True)
        self.assertIs(Sandbox.kill("probe-sandbox", **self.options), False)
        self.assertEqual([r["method"] for r in self.records], ["DELETE", "DELETE"])
        self.assertTrue(all(r["path"] == "/sandboxes/probe-sandbox" for r in self.records))

    def test_set_timeout_forwards_zero_and_negative(self):
        for value in (-1, 0, 60):
            self.responses.append((204, None, {}))
            Sandbox.set_timeout("probe-sandbox", value, **self.options)
            self.assertEqual(self.records[-1]["body"], {"timeout": value})
            self.assertEqual(self.records[-1]["method"], "POST")
            self.assertEqual(self.records[-1]["path"], "/sandboxes/probe-sandbox/timeout")

    def test_errors_are_not_one_generic_rejection(self):
        for status, error_type in (
            (400, SandboxException), (401, AuthenticationException),
            (404, SandboxNotFoundException), (409, SandboxException),
            (429, RateLimitException), (503, ServiceBusyException),
        ):
            with self.subTest(status=status):
                self.responses.append((status, {"code": status, "message": "synthetic"}, {}))
                with self.assertRaises(error_type):
                    Sandbox.connect("probe-sandbox", **self.options)

    def test_429_create_retry_has_no_automatic_idempotency_key(self):
        self.responses.extend(
            [(429, {"code": 429, "message": "busy"}, {"Retry-After": "0"}),
             (201, SANDBOX, {})]
        )
        Sandbox.create(**{**self.options, "retries": 3})
        self.assertEqual(len(self.records), 2)
        self.assertEqual(self.records[0]["body"], self.records[1]["body"])
        self.assertFalse(
            any("idempotency" in name for r in self.records for name in r["header_names"])
        )

    def test_500_does_not_trigger_control_plane_retry(self):
        self.responses.append((500, {"code": 500, "message": "synthetic unknown"}, {}))
        with self.assertRaises(SandboxException):
            Sandbox.create(**{**self.options, "retries": 3})
        self.assertEqual(len(self.records), 1)

    def test_error_body_requires_code_not_only_message(self):
        self.responses.append((404, {"message": "synthetic malformed error"}, {}))
        with self.assertRaises(KeyError):
            Sandbox.kill("probe-sandbox", **self.options)

    def test_async_harbor_style_create_and_connect(self):
        async def run():
            await AsyncSandbox.create(
                timeout=86400, allow_internet_access=True,
                metadata={"session_id": "synthetic-session"}, **self.options
            )
            await AsyncSandbox.connect("probe-sandbox", **self.options)

        self.responses.extend([(201, SANDBOX, {}), (200, SANDBOX, {})])
        asyncio.run(run())
        self.assertEqual(
            [r["path"] for r in self.records],
            ["/v2/sandboxes", "/v2/sandboxes/probe-sandbox/connect"],
        )
        self.assertEqual(self.records[0]["body"]["timeout"], 86400)
        self.assertNotIn("timeout", self.records[1]["body"])

    def test_debug_mode_is_not_a_lifecycle_test(self):
        sandbox = Sandbox.create(**{**self.options, "debug": True})
        self.assertEqual(sandbox.sandbox_id, "debug_sandbox_id")
        self.assertTrue(sandbox.kill())
        self.assertEqual(self.records, [])


class ContractManifest(unittest.TestCase):
    def test_manifest_matches_executable_cases(self):
        from pathlib import Path

        directory = Path(__file__).parent
        manifest = json.loads((directory / "contract.json").read_text())
        project = tomllib.loads((directory / "pyproject.toml").read_text())
        self.assertEqual(manifest["package"], "e2b")
        self.assertEqual(manifest["version"], importlib.metadata.version("e2b"))
        self.assertIn("e2b==" + manifest["version"], project["project"]["dependencies"])
        self.assertEqual(
            manifest["git_revision"], project["tool"]["uv"]["sources"]["e2b"]["rev"]
        )
        self.assertEqual(
            sorted(manifest["tested_cases"]),
            unittest.defaultTestLoader.getTestCaseNames(WireContract),
        )
