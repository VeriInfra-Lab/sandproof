"""Keep the checked-in M0 lock fingerprints tied to actual repository inputs."""

import hashlib
import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
BASELINE = ROOT / "references/v1-baseline.json"


class V1BaselineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.baseline = json.loads(BASELINE.read_text())

    def test_locked_inputs_match_baseline(self):
        code = self.baseline["code_baseline"]
        for path, expected in (
            (ROOT / "Cargo.lock", code["cargo_lock_sha256"]),
            (ROOT / "tests/compat/uv.lock", code["sdk_lock_sha256"]),
        ):
            with self.subTest(path=path.name):
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)
        toolchain = (ROOT / "rust-toolchain.toml").read_text()
        self.assertIn(f'channel = "{code["rust_toolchain"]}"', toolchain)

    def test_historical_evidence_is_tied_to_exact_commits_and_test_files(self):
        records = self.baseline["historical_evidence"]
        self.assertEqual(len({record["id"] for record in records}), len(records))
        for record in records:
            with self.subTest(id=record["id"]):
                self.assertRegex(record["commit"], r"^[0-9a-f]{40}$")
                self.assertTrue((ROOT / record["test"]).is_file())
                self.assertRegex(record["pr"], r"^https://github.com/ningZhang-cs/sandproof/pull/\d+$")
                self.assertTrue(record["scope"])

    def test_current_host_observation_is_not_misreported_as_new_vm_run(self):
        host = self.baseline["observed_linux_host"]
        self.assertIn("未运行", host["limit"])
        for artifact in host["artifacts"].values():
            self.assertTrue(artifact["path"].startswith("/"))
            self.assertTrue(re.fullmatch(r"[0-9a-f]{64}", artifact["sha256"]))


if __name__ == "__main__":
    unittest.main()
