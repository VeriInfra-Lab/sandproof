"""Use the real pinned SDK constructor, synthetic guest transport only."""
import contextlib
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.template_build_service import Backend


class BuildSdkSession(unittest.TestCase):
    def test_existing_private_vm_session_uses_real_sdk_without_second_create(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / "source.tar"
            source.write_bytes(b"opaque input")
            candidate = b"\0" * (1024 * 1024)
            status = {"state": "candidate", "rootfs_sha256": hashlib.sha256(candidate).hexdigest()}
            profile = {"runtime": {"credentials": [{"tenant": 1, "secret": "runtime-key"}]},
                "builder": {"bind": "127.0.0.1:1", "domain": "example.invalid", "credentials": [{"secret": "builder-key"}]},
                "tools": {}, "dns_servers": ["1.1.1.1"]}
            backend = Backend(profile)
            row = {"id": "a" * 32, "input_hash": hashlib.sha256(source.read_bytes()).hexdigest(), "deadline": 1000,
                "request": json.dumps({"kind": "dockerfile", "resources": {"diskMiB": 1, "minFreeMiB": 16}})}
            store = SimpleNamespace(directory=lambda _: directory, clock=lambda: 0,
                limits={"max_input_bytes": 65536, "max_expanded_bytes": 65536, "max_entries": 100, "max_log_bytes": 65536})
            session = {"sandboxID": "sb-test-1", "envdVersion": "0.9.0", "envdAccessToken": "private-token"}
            def stream(path, **_opts):
                data = json.dumps(status).encode() if path.endswith("status.json") else candidate if path.endswith("candidate.ext4") else b"bounded log\n"
                return contextlib.nullcontext(iter([data]))
            # Keep Sandbox itself real: incorrect kwargs/version/configuration
            # must fail here, not hide behind a constructor test double.
            with patch.object(backend, "vm", return_value={"session": session}) as identity, \
                    patch("e2b.sandbox_sync.main.Filesystem") as files, \
                    patch("e2b.sandbox_sync.main.Commands") as commands:
                files.return_value.read.side_effect = stream
                commands.return_value.run.return_value.stdout = "12345678-1234-1234-1234-123456789abc\n"
                self.assertEqual(backend.build(store, row), status)
                identity.assert_called_once_with(row, create=True)
                config = files.call_args.args[2]
                self.assertEqual(config.sandbox_headers["X-Access-Token"], "private-token")
                self.assertEqual((directory / "candidate.ext4").stat().st_size, len(candidate))
