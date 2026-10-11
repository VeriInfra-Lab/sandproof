"""Unprivileged candidate-contract regressions; no KVM execution claims."""

import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import ci_e2e_artifacts as artifacts
from tools.tests.storage_observation_fixture import observation


class CandidateArtifactsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.root.chmod(0o700)
        self.candidate = "a" * 40
        self.versions = dict(internal_protocol=2, center_store=6, host_store=16)
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

    def execution_inputs(self):
        roles = {}
        for index, (role, uid) in enumerate((("center", 1001), ("host", 0), ("redis", 1002)), 1):
            roles[role] = dict(uid=uid, gid=uid, config_sha256=str(index) * 64,
                               directory=dict(device=1, inode=index, uid=uid, gid=uid, mode=0o700))
        roles['host'] = [roles['host']]
        return dict(run_id="b" * 64, candidate_manifest_sha256="c" * 64, roles=roles,
                    redis=dict(artifacts.REDIS_PIN, binary_sha256="d" * 64, binary_bytes=1024),
                    tls_sha256="e" * 64, budget_sha256="f" * 64)

    def execution(self):
        return artifacts.execution_manifest(self.record(), **self.execution_inputs())

    def process_evidence(self):
        return {name:dict(pid=index+10,start_time_ticks=100,uid=uid,gid=uid,
            executable=dict(device=1,inode=index+20)) for index,(name,uid) in
            enumerate((('center',1001),('redis',1002),('host-0',0)))}

    def test_actual_process_receipt_requires_complete_bounded_exact_role_set(self):
        execution=self.execution()
        facts=self.process_evidence()
        self.assertEqual(artifacts.require_process_evidence(facts,execution),facts)
        for field,value in (('pid',True),('pid',0),('start_time_ticks',0),('uid',0),('gid',1002)):
            changed=copy.deepcopy(facts)
            changed['center'][field]=value
            with self.subTest(field=field,value=value),self.assertRaises(ValueError):
                artifacts.require_process_evidence(changed,execution)
        for changed in ({},dict(facts,server=facts['host-0']),dict(facts,**{'host-1':facts['host-0']})):
            with self.assertRaises(ValueError):
                artifacts.require_process_evidence(changed,execution)
        changed=copy.deepcopy(facts)
        changed['redis']['pid']=changed['center']['pid']
        with self.assertRaises(ValueError):
            artifacts.require_process_evidence(changed,execution)

    def test_all_six_dk_cases_are_mandatory_same_candidate_and_same_group_execution(self):
        evidence=dict(schema_version=1,candidate=self.candidate,execution_manifest_sha256='a'*64,
            service_storage=observation(),
            cases=[dict(id='DK'+str(number).zfill(2),status='pass',exact_cleanup='passed',
                execution_manifest_sha256='a'*64,evidence_sha256=str(number)*64) for number in range(1,7)])
        self.assertEqual(artifacts.require_dk(evidence,candidate=self.candidate),evidence)
        for missing in range(6):
            changed=copy.deepcopy(evidence)
            del changed['cases'][missing]
            with self.assertRaises(ValueError):
                artifacts.require_dk(changed,candidate=self.candidate)
        for field,value in (('id','DK05'),('status','not_run'),('status','failed'),
                ('exact_cleanup','unknown'),('execution_manifest_sha256','b'*64),('evidence_sha256','x')):
            changed=copy.deepcopy(evidence)
            changed['cases'][-1][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):
                artifacts.require_dk(changed,candidate=self.candidate)
        with self.assertRaises(ValueError):
            artifacts.require_dk(dict(evidence,candidate='f'*40),candidate=self.candidate)

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

    def test_dk_retains_validated_storage_and_rejects_missing_or_unknown_final_inventory(self):
        evidence=dict(schema_version=1,candidate=self.candidate,execution_manifest_sha256='a'*64,
            service_storage=observation(),
            cases=[dict(id='DK0'+str(number),status='pass',exact_cleanup='passed',
                execution_manifest_sha256='a'*64,evidence_sha256=str(number)*64) for number in range(1,7)])
        persisted=json.loads(json.dumps(evidence))
        self.assertEqual(artifacts.require_dk(persisted,candidate=self.candidate)['service_storage'],observation())
        for value in (None,dict(observation(),sampler_exited=False),dict(observation(),roots=[])):
            with self.assertRaises(ValueError):
                artifacts.require_dk(dict(persisted,service_storage=value),candidate=self.candidate)
        del persisted['service_storage']
        with self.assertRaises(ValueError):
            artifacts.require_dk(persisted,candidate=self.candidate)

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
                             ("schema_version", 2), ("versions", dict(self.versions, host_store=15)),
                             ("versions", dict(self.versions, host_store=True)),
                             ("versions", dict(self.versions, extra=1))):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.validate(dict(original, **{field: value}))
        with self.assertRaises(ValueError):
            self.validate(dict(original, private_key="must-not-be-exported"))

    def test_implicit_or_unbounded_versions_are_rejected_before_artifact_access(self):
        for invalid in (None, {}, dict(self.versions, internal_protocol=0),
                        dict(self.versions, center_store=-1), dict(self.versions, host_store=2**32),
                        dict(self.versions, host_store=1.0), dict(self.versions, host_store="16")):
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

    def test_execution_manifest_binds_roles_material_and_run_without_io_or_verdict(self):
        manifest = self.record()
        inputs = self.execution_inputs()
        with patch.object(artifacts, "_fingerprint") as fingerprint, \
                patch.object(artifacts, "private_path") as private_path:
            execution = artifacts.execution_manifest(manifest, **inputs)
            observed = artifacts.require_execution_manifest(execution, manifest, expected=execution)
        fingerprint.assert_not_called()
        private_path.assert_not_called()
        self.assertEqual(observed, execution)
        self.assertEqual(execution["services"], artifacts.public_identity(manifest))
        self.assertEqual(execution["roles"], inputs["roles"])
        self.assertEqual(execution["redis"], inputs["redis"])
        self.assertNotIn(str(self.root), json.dumps(execution))
        for field in ("passed", "Cleaned", "private_key", "config_path", "server_sha256"):
            self.assertNotIn(field, execution)
        # The returned facts are a copy, not references to mutable caller input.
        inputs["roles"]["host"][0]["directory"]["inode"] = 10
        inputs["redis"]["binary_sha256"] = "0" * 64
        manifest["versions"]["host_store"] = 100
        self.assertEqual(execution["roles"]["host"][0]["directory"]["inode"], 2)
        self.assertEqual(execution["redis"]["binary_sha256"], "d" * 64)
        self.assertEqual(execution["services"]["versions"], self.versions)

    def test_execution_refuses_missing_legacy_or_additional_roles_and_secrets(self):
        inputs = self.execution_inputs()
        invalid_roles = [None, {}, {"host": inputs["roles"]["host"]},
                         dict(inputs["roles"], server=inputs["roles"]["host"])]
        for field in ("config", "private_key", "path", "passed"):
            changed = copy.deepcopy(inputs["roles"])
            changed["center"][field] = "must-not-be-recorded"
            invalid_roles.append(changed)
        changed = copy.deepcopy(inputs["roles"])
        changed["center"]["directory"]["path"] = "/protected/center"
        invalid_roles.append(changed)
        for roles in invalid_roles:
            with self.subTest(roles=roles), self.assertRaises(ValueError):
                artifacts.execution_manifest(self.record(), **dict(inputs, roles=roles))
        valid = self.execution()
        for evidence in (dict(server_sha256="b" * 64), dict(valid, server_sha256="b" * 64),
                         dict(valid, passed=True), dict(valid, Cleaned=True),
                         dict(valid, schema_version=True), dict(valid, schema_version=2)):
            with self.subTest(evidence=evidence), self.assertRaises(ValueError):
                artifacts.require_execution_manifest(evidence, self.record(), expected=valid)

    def test_role_process_identities_are_explicit_disjoint_and_strictly_numeric(self):
        for role, pair in (("center", (0, 0)), ("redis", (0, 0)), ("host", (1000, 1000)),
                           ("host", (False, 0)), ("host", (0, False)),
                           ("center", (None, None)), ("center", (1001, None)),
                           ("center", (True, 1001)), ("center", (999, 1001)),
                           ("redis", (1001, 1002)), ("redis", (2**32 - 1, 1002))):
            inputs = self.execution_inputs()
            facts = inputs['roles'][role][0] if role == 'host' else inputs['roles'][role]
            facts.update(zip(("uid", "gid"), pair))
            facts["directory"].update(zip(("uid", "gid"), pair))
            with self.subTest(role=role, pair=pair), self.assertRaises(ValueError):
                artifacts.execution_manifest(self.record(), **inputs)

    def test_role_directories_require_original_inode_owner_and_private_mode(self):
        for field, value in (("device", True), ("device", -1), ("device", 2**64),
                             ("inode", 0), ("inode", 2**64), ("inode", "1"),
                             ("uid", 0), ("uid", 1001.0), ("gid", 0),
                             ("mode", 0o755), ("mode", 0o1700), ("mode", 448.0)):
            inputs = self.execution_inputs()
            inputs["roles"]["center"]["directory"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                artifacts.execution_manifest(self.record(), **inputs)
        inputs = self.execution_inputs()
        inputs["roles"]["redis"]["directory"].update(device=1, inode=1)
        with self.assertRaisesRegex(ValueError, "independent protected directory"):
            artifacts.execution_manifest(self.record(), **inputs)

    def test_execution_uses_original_fixed_redis_receipt_and_bounded_material(self):
        for field, value in (("version", "wrong"), ("archive_sha256", "0" * 64),
                             ("binary_sha256", "A" * 64), ("binary_sha256", None),
                             ("binary_bytes", 0), ("binary_bytes", True),
                             ("binary_bytes", artifacts.REDIS_BINARY_LIMIT + 1),
                             ("path", "/private/redis")):
            inputs = self.execution_inputs()
            inputs["redis"][field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                artifacts.execution_manifest(self.record(), **inputs)
        inputs = self.execution_inputs()
        inputs["redis"].pop("binary_bytes")
        with self.assertRaises(ValueError):
            artifacts.execution_manifest(self.record(), **inputs)

    def test_all_run_material_bindings_require_sha256_and_exact_expected_facts(self):
        fields = ("run_id", "candidate_manifest_sha256", "tls_sha256", "budget_sha256")
        for field in fields:
            for bad in (None, "c" * 63, "C" * 64, True):
                inputs = self.execution_inputs()
                inputs[field] = bad
                with self.subTest(field=field, value=bad), self.assertRaises(ValueError):
                    artifacts.execution_manifest(self.record(), **inputs)
        inputs = self.execution_inputs()
        inputs["roles"]["host"][0]["config_sha256"] = "0" * 63
        with self.assertRaises(ValueError):
            artifacts.execution_manifest(self.record(), **inputs)
        expected = self.execution()
        changed = []
        for field in fields:
            changed.append(dict(expected, **{field: "0" * 64}))
        for role in ("center", "host", "redis"):
            value = copy.deepcopy(expected)
            facts = value['roles'][role][0] if role == 'host' else value['roles'][role]
            facts["config_sha256"] = "0" * 64
            changed.append(value)
        value = copy.deepcopy(expected)
        value["roles"]["center"]["directory"]["inode"] = 100
        changed.append(value)
        value = copy.deepcopy(expected)
        value["redis"]["binary_sha256"] = "0" * 64
        changed.append(value)
        for evidence in changed:
            with self.subTest(evidence=evidence), self.assertRaisesRegex(ValueError, "approved run staging"):
                artifacts.require_execution_manifest(evidence, self.record(), expected=expected)

    def test_all_host_pool_and_private_helper_configurations_are_explicit_bounded_and_distinct(self):
        inputs = self.execution_inputs()
        host = inputs['roles']['host'][0]
        second = dict(host, config_sha256='6'*64, directory=dict(host['directory'], inode=6))
        helper = dict(host, config_sha256='7'*64, directory=dict(host['directory'], inode=7))
        inputs['roles']['host'] = [host, second, helper]
        self.assertEqual(len(artifacts.execution_manifest(self.record(), **inputs)['roles']['host']), 3)
        for invalid in ([], host, [host]*2, [host, second, helper, helper],
                        [host, dict(second, config_sha256=host['config_sha256'])]):
            with self.subTest(host=invalid), self.assertRaises(ValueError):
                artifacts.execution_manifest(self.record(), **dict(inputs, roles=dict(inputs['roles'], host=invalid)))

    def test_execution_cannot_attest_mixed_candidates_versions_or_invalid_expectations(self):
        expected = self.execution()
        for identity in (dict(expected["services"], candidate="0" * 40),
                         dict(expected["services"], host_sha256="0" * 64),
                         dict(expected["services"], versions=dict(self.versions, host_store=15))):
            evidence = dict(expected, services=identity)
            with self.subTest(identity=identity), self.assertRaises(ValueError):
                artifacts.require_execution_manifest(evidence, self.record(), expected=expected)
        for invalid in (dict(expected, schema_version=True), dict(expected, private_key="secret"),
                        dict(expected, services=dict(expected["services"], candidate="0" * 40))):
            with self.subTest(expected=invalid), self.assertRaises(ValueError):
                artifacts.require_execution_manifest(expected, self.record(), expected=invalid)


if __name__ == "__main__":
    unittest.main()
