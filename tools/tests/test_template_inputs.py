import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import signal
import tarfile
import tempfile
import time
import unittest
from unittest.mock import MagicMock, patch

from tools.template_inputs import InvalidTemplate, extract_context, inspect_oci, startup_config
from tools.template_worker import Commands, LogBudget, guest_path, install_platform, terminate, validate_request


def archive_bytes(entries):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w") as archive:
        for name, data, kind, link in entries:
            entry = tarfile.TarInfo(name)
            entry.type, entry.linkname = kind, link
            entry.size = len(data) if kind == tarfile.REGTYPE else 0
            entry.mode = 0o6755
            archive.addfile(entry, io.BytesIO(data) if entry.size else None)
    return output.getvalue()


class TemplateInputTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def context(self, entries, **limits):
        source = self.root / "context.tar"
        source.write_bytes(archive_bytes(entries))
        extract_context(source, self.root / "context", max_bytes=limits.get("max_bytes", 1000),
                        max_entries=limits.get("max_entries", 20))

    def test_context_copy_internal_link_and_modes(self):
        self.context([("Dockerfile", b"FROM scratch\nCOPY payload /payload", tarfile.REGTYPE, ""),
                      ("payload", b"hello", tarfile.REGTYPE, ""), ("link", b"", tarfile.SYMTYPE, "payload")])
        self.assertEqual((self.root / "context/link").read_bytes(), b"hello")
        self.assertEqual((self.root / "context/payload").stat().st_mode & 0o7777, 0o755)

    def test_context_boundaries(self):
        cases = [
            [("../escape", b"x", tarfile.REGTYPE, "")],
            [("/escape", b"x", tarfile.REGTYPE, "")],
            [("link", b"", tarfile.SYMTYPE, "../escape")],
            [("link", b"", tarfile.SYMTYPE, "/escape")],
            [("link", b"", tarfile.SYMTYPE, "target"), ("link/file", b"x", tarfile.REGTYPE, "")],
            [("link", b"", tarfile.LNKTYPE, "Dockerfile")],
            [("device", b"", tarfile.CHRTYPE, "")],
            [("fifo", b"", tarfile.FIFOTYPE, "")],
            [("same", b"x", tarfile.REGTYPE, ""), ("same", b"y", tarfile.REGTYPE, "")],
            [("Dockerfile", b"", tarfile.SYMTYPE, "target")],
        ]
        for case in cases:
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                source = Path(temporary) / "input.tar"
                source.write_bytes(archive_bytes(case))
                with self.assertRaises((InvalidTemplate, FileExistsError)):
                    extract_context(source, Path(temporary) / "out", max_bytes=1000, max_entries=20)
                self.assertFalse((Path(temporary) / "escape").exists())

    def test_context_limits_and_no_reuse(self):
        entries = [("Dockerfile", b"FROM scratch", tarfile.REGTYPE, "")]
        with self.assertRaisesRegex(InvalidTemplate, "byte limit"):
            self.context(entries, max_bytes=1)
        with self.assertRaises(FileExistsError):
            self.context(entries)

    def make_oci(self, layer_entries=None):
        layout = self.root / "oci"
        (layout / "blobs/sha256").mkdir(parents=True)
        def add(data, media):
            digest = hashlib.sha256(data).hexdigest()
            (layout / "blobs/sha256" / digest).write_bytes(data)
            return {"digest": "sha256:" + digest, "size": len(data), "mediaType": media}
        layer = archive_bytes(layer_entries or [("etc/value", b"hello", tarfile.REGTYPE, ""),
                                               ("etc/.wh.old", b"", tarfile.REGTYPE, ""),
                                               ("bin", b"", tarfile.SYMTYPE, "/usr/bin")])
        layer_descriptor = add(gzip.compress(layer), "application/vnd.oci.image.layer.v1.tar+gzip")
        config = {"os": "linux", "architecture": "amd64", "rootfs": {"type": "layers", "diff_ids": [
            "sha256:" + hashlib.sha256(layer).hexdigest()]}, "config": {"Env": ["VALUE=hello"], "User": "1000"}}
        config_descriptor = add(json.dumps(config).encode(), "application/vnd.oci.image.config.v1+json")
        manifest = {"schemaVersion": 2, "config": config_descriptor, "layers": [layer_descriptor]}
        descriptor = add(json.dumps(manifest).encode(), "application/vnd.oci.image.manifest.v1+json")
        (layout / "index.json").write_text(json.dumps({"schemaVersion": 2, "manifests": [descriptor]}))
        (layout / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}')
        return layout, layer_descriptor

    def test_oci_hashes_gzip_whiteout_absolute_guest_link(self):
        layout, _ = self.make_oci()
        result = inspect_oci(layout, max_bytes=100000, max_entries=20)
        self.assertEqual(result["container_config"]["User"], "1000")
        self.assertTrue(result["image_digest"].startswith("sha256:"))

    def test_oci_bad_hash_bomb_path_device_and_multiindex(self):
        layout, layer = self.make_oci()
        with self.assertRaisesRegex(InvalidTemplate, "expanded byte limit"):
            inspect_oci(layout, max_bytes=1024, max_entries=20)
        path = layout / "blobs/sha256" / layer["digest"][7:]
        path.write_bytes(b"x" * layer["size"])
        with self.assertRaisesRegex(InvalidTemplate, "fingerprint"):
            inspect_oci(layout, max_bytes=100000, max_entries=20)
        for name, kind in (("../outside", tarfile.REGTYPE), ("dev/x", tarfile.CHRTYPE)):
            with tempfile.TemporaryDirectory() as temporary:
                old = self.root
                self.root = Path(temporary)
                try:
                    layout, _ = self.make_oci([(name, b"", kind, "")])
                    with self.assertRaises(InvalidTemplate):
                        inspect_oci(layout, max_bytes=100000, max_entries=20)
                    index = json.loads((layout / "index.json").read_text())
                    index["manifests"] *= 2
                    (layout / "index.json").write_text(json.dumps(index))
                    with self.assertRaisesRegex(InvalidTemplate, "one OCI image"):
                        inspect_oci(layout, max_bytes=100000, max_entries=20)
                finally:
                    self.root = old

    def test_startup_preserved_not_executed_and_rejects_unsupported(self):
        config = {"Env": ["KEY=$(touch /tmp/nope)"], "WorkingDir": "/app", "User": "1000:1000",
                  "Entrypoint": ["/entry", "a b"], "Cmd": ["$not_shell"]}
        self.assertEqual(startup_config(config), config)
        for changes in ({"Volumes": {"/data": {}}}, {"WorkingDir": "/../bad"}, {"Env": ["BAD"]},
                        {"Cmd": "not argv"}, {"User": "x\x00y"}):
            with self.subTest(changes=changes), self.assertRaises(InvalidTemplate):
                startup_config({**config, **changes})

    def test_platform_resolver_never_follows_guest_absolute_link_on_host(self):
        root = self.root / "root"
        (root / "usr/sbin").mkdir(parents=True)
        (root / "sbin").symlink_to("/usr/sbin")
        self.assertEqual(guest_path(root, "/sbin/init"), root / "usr/sbin/init")
        (root / "bad").symlink_to("../outside")
        with self.assertRaises(InvalidTemplate):
            guest_path(root, "/bad/x")
        (root / "loop").symlink_to("loop")
        with self.assertRaisesRegex(InvalidTemplate, "loop"):
            guest_path(root, "/loop")

    def test_platform_reserves_trusted_init(self):
        root = self.root / "root"
        for path in ("etc", "usr/sbin", "opt"):
            (root / path).mkdir(parents=True)
        (root / "sbin").symlink_to("usr/sbin")
        (root / "etc/os-release").write_text('ID=debian\n')
        (root / "etc/passwd").write_text('root:x:0:0:root:/root:/bin/sh\n')
        binary = self.root / "binary"
        binary.write_bytes(b"trusted")
        install_platform(root, binary, binary)
        self.assertEqual((root / "usr/sbin/init").readlink(), Path("/opt/sandproof/busybox"))
        self.assertIn("/opt/sandproof/agent", (root / "etc/inittab").read_text())
        with self.assertRaisesRegex(InvalidTemplate, "reserved"):
            install_platform(root, binary, binary)

    def test_worker_limits_are_explicit_no_boolean_or_unknown_field(self):
        request = {"kind": "dockerfile", "disk_mib": 1024, "min_free_mib": 64, "max_input_bytes": 100,
                   "max_expanded_bytes": 10000 * 1024, "max_entries": 100, "timeout_seconds": 30, "max_log_bytes": 1024}
        validate_request(request)
        for changes in ({"timeout_seconds": True}, {"disk_mib": 0}, {"min_free_mib": 1024}, {"extra": 1}):
            with self.subTest(changes=changes), self.assertRaises(InvalidTemplate):
                validate_request({**request, **changes})

    def test_command_nonzero_deadline_and_bounded_output(self):
        log = io.BytesIO()
        runner = Commands(time.monotonic() + 10, log, 3)
        # These commands exit by themselves. Avoid testing Linux process-group
        # signal permissions through the macOS execution sandbox.
        with patch("tools.template_worker.terminate", side_effect=lambda child: child.wait(timeout=2)):
            with self.assertRaisesRegex(InvalidTemplate, "log limit"):
                runner.run(["/bin/sh", "-c", "printf abcdef"])
        self.assertEqual(log.getvalue(), b"abc")
        with patch("tools.template_worker.terminate", side_effect=lambda child: child.wait(timeout=2)):
            with self.assertRaises(RuntimeError):
                Commands(time.monotonic() + 10, io.BytesIO(), 10).run(["/bin/sh", "-c", "exit 7"])
        with self.assertRaises(TimeoutError):
            Commands(time.monotonic() - 1, io.BytesIO(), 10).run(["/bin/sh", "-c", "exit 0"])

    def test_process_group_cleanup_and_shared_log_budget(self):
        child = MagicMock(pid=12345)
        child.poll.return_value = None
        child.wait.side_effect = [subprocess.TimeoutExpired("tool", 3), 0]
        with patch("tools.template_worker.os.killpg") as kill:
            terminate(child)
            self.assertEqual(kill.call_args_list[0].args, (12345, signal.SIGTERM))
            self.assertEqual(kill.call_args_list[1].args, (12345, signal.SIGKILL))
        budget = LogBudget(10)
        self.assertEqual(budget.take(6), 6)
        self.assertEqual(budget.take(6), 4)
        self.assertEqual(budget.take(1), 0)


if __name__ == "__main__":
    unittest.main()
