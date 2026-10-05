"""Keep the checked-in M0 lock fingerprints tied to actual repository inputs."""

import hashlib
import json
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

    def test_external_source_inputs_are_fixed_revisions(self):
        sources = self.baseline["source_revisions"]
        for name in ("e2b_sdk", "e2b_runtime_behavior_reference", "harbor_adapter_reference"):
            with self.subTest(name=name):
                self.assertRegex(sources[name], r"^[0-9a-f]{40}$")
        self.assertRegex(sources["firecracker_release"], r"^[0-9]+\.[0-9]+\.[0-9]+$")

    def test_public_manifest_is_not_a_machine_inventory_or_test_verdict(self):
        self.assertEqual(self.baseline["schema_version"], 2)
        self.assertEqual(set(self.baseline), {
            "schema_version", "purpose", "code_baseline", "source_revisions", "evidence_policy",
        })
        self.assertIn("不是部署机器台账", self.baseline["purpose"])
        self.assertIn("不得把这些输入锁当作通过结果", self.baseline["evidence_policy"])


if __name__ == "__main__":
    unittest.main()
