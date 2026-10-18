"""Reject B3 source, use-site and theorem inventory drift."""

import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from tools.check_revocation_contract import ROOT, check, digest


class RevocationContractAdmissionTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name) / "source"
        self.cargo = Path(temp.name) / "cargo"
        self.environment = json.loads((ROOT / "verification/revocation-contract-environment.json").read_text())
        for name in self.environment["inputs"]:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        version = self.environment["tokio"]["version"]
        registry = self.cargo / f"registry/src/test-registry/tokio-{version}"
        for name in self.environment["tokio"]["reviewed_sources"]:
            path = registry / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// synthetic source, admission test only\n")
            self.environment["tokio"]["reviewed_sources"][name] = digest(path)
        # The existing B1 suite tests the full archive/cache check separately.
        mock = patch("tools.check_revocation_contract.check_b1", return_value={"tokio": {"version": version}})
        mock.start()
        self.addCleanup(mock.stop)
        self.seal()

    def seal(self):
        (self.root / "verification/revocation-contract-environment.json").write_text(json.dumps(self.environment))
        lines = [f"{digest(self.root / name)}  {name}\n" for name in self.environment["inputs"]]
        (self.root / "verification/revocation-contract-inputs.sha256").write_text("".join(lines))

    def test_bound_contract_is_admitted(self):
        self.assertEqual(len(check(self.root, self.cargo)["theorems"]), 21)

    def test_actual_adapter_drift_is_rejected(self):
        path = self.root / "sandbox-data-transport/src/revocation.rs"
        path.write_text(path.read_text().replace("biased;", "// changed order"))
        with self.assertRaisesRegex(ValueError, "Frozen B3 input changed"):
            check(self.root, self.cargo)

    def test_new_consumer_requires_signal_and_work_review(self):
        path = self.root / "sandbox-host-service/src/new_consumer.rs"
        path.write_text("fn new_consumer() { until_revoked(&mut unrelated, work); }\n")
        with self.assertRaisesRegex(ValueError, "call inventory changed"):
            check(self.root, self.cargo)

    def test_missing_input_is_rejected(self):
        path = self.root / "verification/revocation-contract-inputs.sha256"
        path.write_text("\n".join(path.read_text().splitlines()[:-1]) + "\n")
        with self.assertRaisesRegex(ValueError, "Missing or duplicate"):
            check(self.root, self.cargo)

    def test_missing_audit_rejected_after_resealing(self):
        path = self.root / "SandboxModel/Boundaries/Revocation/Audit.lean"
        path.write_text("\n".join(path.read_text().splitlines()[:-1]) + "\n")
        self.seal()
        with self.assertRaisesRegex(ValueError, "theorem audit"):
            check(self.root, self.cargo)

    def test_relabel_as_extracted_rust_is_rejected(self):
        self.environment["proof_kind"] = "extracted-async-rust"
        self.seal()
        with self.assertRaisesRegex(ValueError, "not an extracted async"):
            check(self.root, self.cargo)
