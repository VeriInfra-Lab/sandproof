"""Fail-closed integrity tests; the fake archive is never a production model."""

import io
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import unittest

from tools.check_task_contract import ROOT, check, digest


class TaskContractAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "source"
        self.cargo = Path(self.temp.name) / "cargo"
        self.environment = json.loads((ROOT / "verification/task-contract-environment.json").read_text())
        for name in self.environment["inputs"]:
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, destination)
        # A self-contained, cryptographically bound archive exercises the
        # integrity checker without depending on the developer's Cargo cache.
        tokio = self.environment["tokio"]
        self.registry = self.cargo / "registry/src/test-registry" / f"tokio-{tokio['version']}"
        self.registry.mkdir(parents=True)
        self.source = self.registry / "src/sync/semaphore.rs"
        self.source.parent.mkdir(parents=True)
        self.source.write_text("// synthetic source for admission checks only\n")
        self.archive = self.cargo / "registry/cache/test-registry" / f"tokio-{tokio['version']}.crate"
        self.archive.parent.mkdir(parents=True)
        with tarfile.open(self.archive, "w:gz") as archive:
            body = self.source.read_bytes()
            info = tarfile.TarInfo(f"tokio-{tokio['version']}/src/sync/semaphore.rs")
            info.size = len(body)
            archive.addfile(info, io.BytesIO(body))
        lock = self.root / "Cargo.lock"
        original = lock.read_text()
        self.assertEqual(original.count(tokio["checksum"]), 1)
        checksum = digest(self.archive)
        lock.write_text(original.replace(tokio["checksum"], checksum))
        tokio["checksum"] = checksum
        tokio["reviewed_sources"] = {"src/sync/semaphore.rs": digest(self.source)}
        (self.root / "verification/task-contract-environment.json").write_text(json.dumps(self.environment))
        self.seal()

    def seal(self):
        lines = [f"{digest(self.root / name)}  {name}\n" for name in self.environment["inputs"]]
        (self.root / "verification/task-contract-inputs.sha256").write_text("".join(lines))

    def test_frozen_inputs_and_bound_archive_are_admitted(self):
        self.assertEqual(len(check(self.root, self.cargo)["theorems"]), 29)

    def test_production_source_drift_is_rejected(self):
        path = self.root / "sandbox-host-service/src/lib.rs"
        path.write_text(path.read_text().replace("let _permit = permit;", "drop(permit);"))
        with self.assertRaisesRegex(ValueError, "Frozen contract input changed"):
            check(self.root, self.cargo)

    def test_omitted_frozen_input_is_rejected(self):
        path = self.root / "verification/task-contract-inputs.sha256"
        path.write_text("\n".join(path.read_text().splitlines()[:-1]) + "\n")
        with self.assertRaisesRegex(ValueError, "Missing or duplicate"):
            check(self.root, self.cargo)

    def test_new_theorem_requires_inventory_review_even_after_resealing(self):
        path = self.root / "SandboxModel/Boundaries/Task/Model.lean"
        path.write_text(path.read_text() + "\ntheorem not_registered : True := by trivial\n")
        self.seal()
        with self.assertRaisesRegex(ValueError, "theorem inventory"):
            check(self.root, self.cargo)

    def test_missing_audit_is_rejected_even_after_resealing(self):
        path = self.root / "SandboxModel/Boundaries/Task/Audit.lean"
        path.write_text("\n".join(path.read_text().splitlines()[:-1]) + "\n")
        self.seal()
        with self.assertRaisesRegex(ValueError, "theorem audit"):
            check(self.root, self.cargo)

    def test_cached_source_cannot_disagree_with_frozen_archive(self):
        self.source.write_text("// changed cache\n")
        with self.assertRaisesRegex(ValueError, "Cached Tokio source changed"):
            check(self.root, self.cargo)

    def test_archive_cannot_disagree_with_locked_checksum(self):
        self.archive.write_bytes(self.archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "Tokio archive differs"):
            check(self.root, self.cargo)
