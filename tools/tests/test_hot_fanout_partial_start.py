"""Check the separately built fault initializer without creating VM resources."""

import errno
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from tools import build_hot_fanout_guest as fixture
from tools.hot_fanout import linux


class PartialStartFixtureTests(unittest.TestCase):
    def test_ordinary_archive_uses_the_original_initializer(self):
        with tempfile.TemporaryDirectory() as directory:
            ordinary, fault = (Path(directory) / name for name in ("ordinary.pyz", "fault.pyz"))
            fixture.install_archive(ordinary, "warm", 4, 3)
            fixture.install_archive(fault, "warm", 4, 3, fault_case=fixture.PARTIAL_START)
            with zipfile.ZipFile(ordinary) as original, zipfile.ZipFile(fault) as trial:
                self.assertEqual(original.namelist(), trial.namelist())
                self.assertEqual(original.read("__main__.py").decode(),
                                 "CHECKPOINT = 'warm'\nBRANCHES = 4\nBATCHES = 3\n" + fixture.WORKLOAD)
                self.assertIn(b"def fail_third_once(", trial.read("__main__.py"))
                for name in original.namelist()[1:]:
                    self.assertEqual(original.read(name), trial.read(name))

    def test_only_the_first_third_spawn_is_stopped_after_two_live_children(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = root / "evidence.json"
            calls = []

            def spawn(scope, _child):
                calls.append(scope.name)
                (scope / "cgroup.procs").write_text("42\n")

            with patch.object(linux, "_spawn_member", spawn):
                exec(fixture.PARTIAL_START_WORKLOAD, {"PARTIAL_START_EVIDENCE": str(evidence)})

                def member(name):
                    scope = root / name
                    scope.mkdir()
                    linux._spawn_member(scope, None)

                for name in ("steady.0", "fault.0", "fault.1"):
                    member(name)
                self.assertFalse(evidence.exists())
                with self.assertRaises(OSError) as failure:
                    member("fault.2")
                self.assertEqual(failure.exception.errno, errno.EAGAIN)
                for name in ("refill.0", "refill.1", "refill.2"):
                    member(name)
            self.assertEqual(calls, ["steady.0", "fault.0", "fault.1", "refill.0", "refill.1", "refill.2"])
            self.assertEqual(json.loads(evidence.read_text()),
                             {"live_ordinals_before_failure": [0, 1], "failed_ordinal": 2})

    def test_missing_live_child_cannot_produce_successful_injection_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = root / "evidence.json"
            scopes = [root / f"fault.{index}" for index in range(3)]
            for scope in scopes:
                scope.mkdir()
                (scope / "cgroup.procs").touch()
            with patch.object(linux, "_spawn_member", lambda *_: None):
                exec(fixture.PARTIAL_START_WORKLOAD, {"PARTIAL_START_EVIDENCE": str(evidence)})
                for scope in scopes[:2]:
                    linux._spawn_member(scope, None)
                with self.assertRaises(OSError) as failure:
                    linux._spawn_member(scopes[2], None)
            self.assertEqual(failure.exception.errno, errno.EIO)
            self.assertFalse(evidence.exists())


if __name__ == "__main__":
    unittest.main()
