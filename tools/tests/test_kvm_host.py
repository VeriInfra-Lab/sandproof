from contextlib import ExitStack
import copy
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools import check_kvm_host as host


def configuration():
    return {
        "firecracker_version": "1.16.1",
        "artifacts": {name: {"path": f"/opt/sandproof/{name}", "sha256": "a" * 64}
                      for name in host.ARTIFACTS},
        "chroot_base": "/var/lib/sandproof/jailer",
        "min_free_disk_bytes": 1024,
        "min_available_memory_bytes": 2048,
    }


class ConfigurationTests(unittest.TestCase):
    def load(self, config):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "host.json"
            path.write_text(json.dumps(config))
            return host.load_config(path)

    def test_valid_config_has_exact_content_digest(self):
        config = configuration()
        loaded, digest = self.load(config)
        self.assertEqual(loaded, config)
        self.assertEqual(digest, hashlib.sha256(json.dumps(config).encode()).hexdigest())

    def test_rejects_missing_unknown_and_malformed_fields(self):
        invalid = [None, [], {**configuration(), "allow_skip": True}]
        for field in configuration():
            candidate = configuration()
            del candidate[field]
            invalid.append(candidate)
        for value in (0, -1, True, "1024"):
            invalid.append({**configuration(), "min_free_disk_bytes": value})
        for value in ("relative/path", "/opt/../tmp/jailer", "/opt/bad\x00path"):
            invalid.append({**configuration(), "chroot_base": value})
        invalid.append({**configuration(), "firecracker_version": "latest"})
        for digest in ("REPLACE_ME", "a" * 63, "A" * 64, None):
            candidate = configuration()
            candidate["artifacts"]["rootfs"]["sha256"] = digest
            invalid.append(candidate)
        for candidate in invalid:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                self.load(candidate)

    def test_duplicate_keys_are_not_silently_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "host.json"
            path.write_text('{"artifacts": {}, "artifacts": {}}')
            with self.assertRaisesRegex(ValueError, "duplicate"):
                host.load_config(path)

    def test_example_requires_operator_to_pin_real_artifacts(self):
        with self.assertRaisesRegex(ValueError, "sha256"):
            host.load_config(host.ROOT / "tests/system/host.example.json")


class ProbeTests(unittest.TestCase):
    def test_trusted_path_rejects_replaceable_ancestors(self):
        trusted = SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_uid=0)
        unsafe = [
            SimpleNamespace(st_mode=stat.S_IFLNK | 0o777, st_uid=0),
            SimpleNamespace(st_mode=stat.S_IFDIR | 0o775, st_uid=0),
            SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_uid=1000),
        ]
        for metadata in unsafe:
            with self.subTest(metadata=metadata), patch.object(Path, "lstat", side_effect=[trusted, metadata]):
                with self.assertRaises(host.CheckFailed) as failure:
                    host.trusted_path(Path("/opt/unsafe/firecracker"))
                self.assertEqual(failure.exception.evidence["component"], "/opt/unsafe")
        with patch.object(Path, "lstat", return_value=trusted):
            self.assertEqual(host.trusted_path(Path("/opt/bin")), Path("/opt/bin"))

    def test_real_file_hash_and_drift(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "kernel"
            path.write_bytes(b"approved kernel")
            spec = {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            with patch.object(host, "trusted_path", side_effect=lambda value: value):
                self.assertEqual(host.artifact_fingerprint(spec)["sha256"], spec["sha256"])
                path.write_bytes(b"different kernel")
                result = host.check("kernel", lambda: host.artifact_fingerprint(spec))
            self.assertEqual(result["status"], "fail")
            self.assertNotEqual(result["sha256"], result["expected_sha256"])

    def test_non_regular_artifact_is_rejected_before_hashing(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(
            host, "trusted_path", side_effect=lambda value: value
        ):
            result = host.check("rootfs", lambda: host.artifact_fingerprint(
                {"path": temp, "sha256": "a" * 64}))
        self.assertEqual(result["status"], "fail")

    @unittest.skipUnless(sys.platform == "linux", "Linux-only ioctl module")
    def test_kvm_only_queries_api_and_closes_descriptor_on_failure(self):
        import fcntl
        for version in (12, 11):
            with self.subTest(version=version), ExitStack() as stack:
                stack.enter_context(patch.object(Path, "stat", return_value=SimpleNamespace(st_mode=stat.S_IFCHR)))
                stack.enter_context(patch.object(os, "open", return_value=321))
                close = stack.enter_context(patch.object(os, "close"))
                ioctl = stack.enter_context(patch.object(fcntl, "ioctl", return_value=version))
                result = host.check("kvm", lambda: host.device(Path("/dev/kvm"), kvm=True))
                self.assertEqual(result["status"], "pass" if version == 12 else "fail")
                ioctl.assert_called_once_with(321, 0xAE00, 0)
                close.assert_called_once_with(321)

    def test_device_open_denial_is_unavailable(self):
        with patch.object(host.platform, "system", return_value="Linux"), patch.object(
            Path, "stat", return_value=SimpleNamespace(st_mode=stat.S_IFCHR)
        ), patch.object(os, "open", side_effect=PermissionError("denied")):
            result = host.check("kvm", lambda: host.device(Path("/dev/kvm"), kvm=True))
        self.assertEqual(result["status"], "unavailable")

    def test_version_requires_correct_product_and_exact_release(self):
        for output in ("Firecracker v1.16.1", "Jailer v1.16.1", "Firecracker v1.16.10", "Firecracker v1.16.1-dev"):
            with self.subTest(output=output), patch.object(host, "command", return_value=output):
                result = host.check("version", lambda: host.binary_version("/approved/bin", "firecracker", "1.16.1"))
                self.assertEqual(result["status"], "pass" if output == "Firecracker v1.16.1" else "fail")

    def test_timeout_and_nonzero_are_distinct_and_stderr_is_not_reported(self):
        failures = [
            (subprocess.TimeoutExpired(["tool"], 10), "timeout"),
            (subprocess.CalledProcessError(2, ["tool"], stderr="secret"), "fail"),
        ]
        for error, status in failures:
            with self.subTest(status=status), patch.object(host.subprocess, "run", side_effect=error):
                result = host.check("version", lambda: host.binary_version("/approved/bin", "jailer", "1.16.1"))
                self.assertEqual(result["status"], status)
                self.assertNotIn("secret", json.dumps(result))

    def test_dirty_source_is_not_presented_as_commit_evidence(self):
        with patch.object(host, "command", side_effect=["a" * 40, " M tool.py"]):
            result = host.check("source", host.source_revision)
        self.assertEqual(result["status"], "fail")
        self.assertTrue(result["dirty"])
        self.assertEqual(result["revision"], "a" * 40)

    def test_memory_and_disk_boundaries(self):
        config = configuration()
        with patch.object(Path, "read_text", return_value="MemAvailable: 2 kB\n"):
            self.assertEqual(host.memory_headroom(config)["available_bytes"], 2048)
            config["min_available_memory_bytes"] += 1
            self.assertEqual(host.check("memory", lambda: host.memory_headroom(config))["status"], "fail")
        with patch.object(host, "trusted_path", side_effect=lambda value: value), patch.object(
            Path, "is_dir", return_value=True
        ), patch.object(host.shutil, "disk_usage", return_value=SimpleNamespace(free=1024)):
            self.assertEqual(host.disk_headroom(config)["available_bytes"], 1024)
            config["min_free_disk_bytes"] += 1
            self.assertEqual(host.check("disk", lambda: host.disk_headroom(config))["status"], "fail")


class ReportTests(unittest.TestCase):
    def probes(self, stack):
        # These controlled probes test aggregation, not a claim of real KVM
        # support on a developer laptop or ordinary CI runner.
        for name in ("source_revision", "linux_host", "root_identity", "device",
                     "cgroup_v2", "host_tools", "disk_headroom", "memory_headroom",
                     "artifact_fingerprint", "binary_version"):
            stack.enter_context(patch.object(host, name, return_value={}))

    def test_success_requires_every_probe_to_pass(self):
        with ExitStack() as stack:
            self.probes(stack)
            report = host.inspect_host(configuration(), "a" * 64)
        self.assertTrue(report["prerequisites_passed"])
        self.assertEqual(report["scope"], "host_prerequisites_only")
        self.assertEqual(len(report["checks"]), 15)

    def test_bad_fingerprint_blocks_execution_but_keeps_other_results(self):
        with ExitStack() as stack:
            self.probes(stack)
            fingerprint = stack.enter_context(patch.object(host, "artifact_fingerprint", side_effect=host.CheckFailed("drift")))
            version = stack.enter_context(patch.object(host, "binary_version"))
            report = host.inspect_host(configuration(), "a" * 64)
        self.assertFalse(report["prerequisites_passed"])
        self.assertEqual(fingerprint.call_count, 4)
        version.assert_not_called()
        statuses = {item["name"]: item["status"] for item in report["checks"]}
        self.assertEqual(statuses["firecracker_version"], "not_run")
        self.assertEqual(statuses["jailer_version"], "not_run")
        self.assertEqual(statuses["memory"], "pass")

    def test_cli_bad_config_returns_json_and_exit_two(self):
        result = subprocess.run(
            [sys.executable, str(host.ROOT / "tools/check_kvm_host.py"), "--config",
             str(host.ROOT / "tests/system/host.example.json")],
            capture_output=True, text=True, timeout=10,
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        report = json.loads(result.stdout)
        self.assertFalse(report["prerequisites_passed"])
        self.assertIn("configuration_error", report)

    def test_cli_missing_host_artifacts_returns_complete_failure_report(self):
        with tempfile.TemporaryDirectory() as temp:
            config = copy.deepcopy(configuration())
            for name, artifact in config["artifacts"].items():
                artifact["path"] = f"{temp}/{name}"
            path = Path(temp) / "host.json"
            path.write_text(json.dumps(config))
            result = subprocess.run(
                [sys.executable, str(host.ROOT / "tools/check_kvm_host.py"), "--config", str(path)],
                capture_output=True, text=True, timeout=30,
            )
        self.assertEqual(result.returncode, 1, result.stderr)
        report = json.loads(result.stdout)
        self.assertFalse(report["prerequisites_passed"])
        self.assertEqual(len(report["checks"]), 15)
        self.assertTrue(all(item["status"] != "pass" for item in report["checks"]
                            if item["name"] in host.ARTIFACTS))


if __name__ == "__main__":
    unittest.main()
