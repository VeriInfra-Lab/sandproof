"""Unprivileged candidate-contract regressions; no KVM execution claims."""

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_e2e_artifacts as artifacts


class CandidateArtifactsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.candidate = "a" * 40
        self.versions = dict(internal_protocol=1, center_store=1, host_store=5)
        self.paths = {}
        for role, name in artifacts.BINARIES.items():
            path = self.root / name
            path.write_bytes(("fixture-not-a-runnable-service-" + role).encode())
            path.chmod(0o700)
            self.paths[role] = str(path)
        # Production uses the original root-owned/ancestor protection checker.
        # This substitute only permits unprivileged filesystem contract tests;
        # it never causes the fixtures above to be executed or attest a VM.
        self.protected = patch.object(artifacts, "trusted_path", side_effect=lambda path: path)
        self.protected.start()
        self.addCleanup(self.protected.stop)

    def record(self):
        return artifacts.record(self.candidate, self.paths, self.versions)

    def validate(self, value):
        return artifacts.validate(value, candidate=self.candidate,
                                  paths=self.paths, versions=self.versions)

    def test_manifest_binds_exact_two_products_versions_and_private_paths(self):
        manifest = self.record()
        self.assertIs(self.validate(manifest), manifest)
        self.assertEqual(manifest["architecture"], artifacts.ARCHITECTURE)
        self.assertEqual(manifest["candidate"], self.candidate)
        self.assertEqual(manifest["versions"], self.versions)
        self.assertEqual(set(manifest["artifacts"]), {"center", "host"})
        for role, path in self.paths.items():
            self.assertEqual(manifest["artifacts"][role]["path"], path)
            self.assertEqual(len(manifest["artifacts"][role]["sha256"]), 64)

    def test_old_single_server_and_additional_roles_cannot_be_new_candidates(self):
        old = dict(schema_version=1, candidate=self.candidate,
                   server_sha256="b" * 64)
        candidates = [old]
        for change in ({"server": {"path": "/old/server", "sha256": "b" * 64}},
                       {}, {"center": self.record()["artifacts"]["center"]},
                       dict(self.record()["artifacts"], server={})):
            candidates.append(dict(self.record(), artifacts=change))
        for candidate in candidates:
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                self.validate(candidate)

    def test_approved_candidate_schema_and_contract_changes_reject(self):
        original = self.record()
        for field, value in (("candidate", "b" * 40), ("candidate", "test"),
                             ("architecture", "single-host"), ("schema_version", True),
                             ("schema_version", 2), ("versions", dict(self.versions, host_store=6)),
                             ("versions", dict(self.versions, host_store=True)),
                             ("versions", dict(self.versions, extra=1))):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.validate(dict(original, **{field: value}))
        with self.assertRaises(ValueError):
            self.validate(dict(original, private_key="must-not-be-exported"))

    def test_implicit_or_unbounded_versions_are_rejected_before_artifact_access(self):
        for invalid in (None, {}, dict(self.versions, internal_protocol=0),
                        dict(self.versions, center_store=-1), dict(self.versions, host_store=2**32),
                        dict(self.versions, host_store=1.0), dict(self.versions, host_store="5")):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError), \
                    patch.object(artifacts, "_fingerprint") as fingerprint:
                artifacts.record(self.candidate, self.paths, invalid)
            fingerprint.assert_not_called()

    def test_binary_role_alias_and_modified_artifact_binding_reject(self):
        for paths in (dict(self.paths, center=self.paths["host"]),
                      dict(self.paths, center="/somewhere/sandproof-server"),
                      dict(self.paths, center="relative/sandproof-center"),
                      dict(self.paths, center=str(self.root / "nested/../sandproof-center")),
                      dict(self.paths, extra="/unexpected/artifact")):
            with self.subTest(paths=paths), self.assertRaises(ValueError):
                artifacts.record(self.candidate, paths, self.versions)
        for field, value in (("path", "/different/sandproof-center"),
                             ("sha256", "A" * 64), ("sha256", "b" * 63),
                             ("sha256", None)):
            changed = self.record()
            changed["artifacts"]["center"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.validate(changed)

    def test_regular_non_executable_file_and_symlink_are_not_sealed(self):
        path = Path(self.paths["center"])
        path.chmod(0o400)
        with self.assertRaises(ValueError):
            self.record()
        path.unlink()
        path.symlink_to(self.paths["host"])
        with self.assertRaises(OSError):
            self.record()

    def test_file_mutation_during_fingerprinting_rejects_mixed_identity(self):
        original = artifacts.hashlib.file_digest

        def change_after_read(source, algorithm):
            result = original(source, algorithm)
            Path(self.paths["center"]).write_bytes(b"changed-after-read")
            return result

        with patch.object(artifacts.hashlib, "file_digest", side_effect=change_after_read), \
                self.assertRaisesRegex(ValueError, "changed while being fingerprinted"):
            self.record()

    def test_private_no_clobber_publication_and_reopen_rechecks_actual_binaries(self):
        path = self.root / "candidate-artifacts.json"
        manifest = artifacts.publish(path, candidate=self.candidate,
                                     paths=self.paths, versions=self.versions)
        self.assertEqual(artifacts.load(path, candidate=self.candidate,
                                       paths=self.paths, versions=self.versions), manifest)
        with self.assertRaises(FileExistsError):
            artifacts.publish(path, candidate=self.candidate, paths=self.paths, versions=self.versions)
        Path(self.paths["host"]).write_bytes(b"different-artifact")
        with self.assertRaisesRegex(ValueError, "prepared center/host artifacts changed"):
            artifacts.load(path, candidate=self.candidate, paths=self.paths, versions=self.versions)

    def test_non_private_manifest_duplicate_json_and_report_links_reject(self):
        path = self.root / "manifest.json"
        path.write_text(json.dumps(self.record()))
        path.chmod(0o644)
        with self.assertRaises(ValueError):
            artifacts.load(path, candidate=self.candidate, paths=self.paths, versions=self.versions)
        path.chmod(0o600)
        path.write_text('{"schema_version":1,"schema_version":1}')
        with self.assertRaisesRegex(ValueError, "duplicate JSON key"):
            artifacts.load(path, candidate=self.candidate, paths=self.paths, versions=self.versions)
        path.unlink()
        path.symlink_to(self.paths["center"])
        with self.assertRaises(ValueError):
            artifacts.load(path, candidate=self.candidate, paths=self.paths, versions=self.versions)

    def test_public_identity_is_allowlisted_and_old_or_mixed_report_cannot_pass(self):
        manifest = self.record()
        identity = artifacts.public_identity(manifest)
        self.assertEqual(set(identity), {"architecture", "candidate", "versions",
                                       "center_sha256", "host_sha256"})
        self.assertNotIn(str(self.root), json.dumps(identity))
        self.assertEqual(artifacts.require_report_identity(identity, manifest), identity)
        for change in (dict(server_sha256="b" * 64), {},
                       dict(identity, host_sha256="f" * 64), dict(identity, candidate="b" * 40),
                       dict(identity, versions=dict(self.versions, internal_protocol=True)),
                       dict(identity, config_path="/private/credentials")):
            with self.subTest(change=change), self.assertRaises(ValueError):
                artifacts.require_report_identity(change, manifest)

    def test_malformed_nested_manifest_export_rejects_without_leaking_inputs(self):
        for value in (None, {}, dict(self.record(), artifacts=[]),
                      dict(self.record(), artifacts={"center": None, "host": {}})):
            with self.subTest(value=value), self.assertRaises(ValueError):
                artifacts.public_identity(value)

    def test_boundary_prerequisite_cannot_turn_an_old_report_into_new_execution(self):
        manifest = self.record()
        report = dict(services=artifacts.public_identity(manifest),
            service_boundary=dict(public_api="center", center_unprivileged=True,
                redis_authority="validated", host_transport="mutual-tls", host_public_management=False))
        safe = artifacts.require_service_boundary(report, manifest)
        self.assertEqual(safe, report)
        self.assertNotIn("passed", safe)  # No scoring/cleanup/execution attestation.
        for changed in (dict(server_sha256="a" * 64),
                        dict(report, server_sha256="a" * 64),
                        dict(report, service_boundary=None),
                        dict(report, service_boundary=dict(report["service_boundary"], center_unprivileged=False)),
                        dict(report, service_boundary=dict(report["service_boundary"], center_unprivileged=1)),
                        dict(report, service_boundary=dict(report["service_boundary"], host_public_management=True)),
                        dict(report, service_boundary=dict(report["service_boundary"], host_public_management=0)),
                        dict(report, service_boundary=dict(report["service_boundary"], host_transport="plaintext"))):
            with self.subTest(report=changed), self.assertRaises(ValueError):
                artifacts.require_service_boundary(changed, manifest)


if __name__ == "__main__":
    unittest.main()
