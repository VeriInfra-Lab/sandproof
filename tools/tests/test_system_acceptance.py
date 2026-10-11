"""C02 supervisor components only: private temp files, SQLite, fake HTTP/processes.

These tests never run the KVM preflight, root tools, service binary or VM. They
are not evidence of C02 KVM acceptance.
"""

from contextlib import ExitStack, redirect_stdout
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools import system_acceptance as reports
from tools import ci_e2e_artifacts as artifacts

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tests/system"))
import sdk_c02_acceptance as runner
import server_vm_id_capacity as native_resources
import _resource_evidence as resource_callback

NS = "1234567890abcdef"
CANARY = "secret-canary-key-env-absolute-path"


class NativeResourceAuditTests(unittest.TestCase):
    def test_resource_range_uses_explicit_native_not_old_flat_or_global_id_suffix(self):
        native = {'namespace': NS, 'uid_base': 3200000,
                  'bounded_host': {'maintenance': {'max_host_slots': 2}}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'jails/.claims').mkdir(parents=True)
            # A stale/public flat file cannot change the explicit checked span.
            (root / 'config.json').write_text(json.dumps({'secret': CANARY}))
            identity = dict(resource_id=f'vm-{NS}-17', pid=345, start_time_ticks=100)
            with patch.object(native_resources, 'process_start_ticks', return_value=101), \
                    patch.object(native_resources, 'processes_in_uid_range', return_value=[]) as uids, \
                    patch.object(Path, 'rglob', return_value=[]):
                native_resources.check_no_resources(root, NS, Path('/approved/firecracker'),
                    3200000, [identity], native=native)
            uids.assert_called_once_with(3200000, 3200001)
            with self.assertRaises(TypeError):
                native_resources.check_no_resources(root, NS, Path('/approved/firecracker'), 3200000)

    def test_foreign_pool_or_invalid_slot_span_is_rejected_before_resource_scan(self):
        native = {'namespace': NS, 'uid_base': 3200000,
                  'bounded_host': {'maintenance': {'max_host_slots': 2}}}
        variants = [dict(native, namespace='f' * 16), dict(native, uid_base=3200001),
                    dict(native, bounded_host={'maintenance': {'max_host_slots': True}}),
                    dict(native, bounded_host={'maintenance': {'max_host_slots': 2**32}})]
        with patch.object(Path, 'iterdir') as scanned, \
                patch.object(native_resources, 'processes_in_uid_range') as uids:
            for settings in variants:
                with self.subTest(settings=settings), self.assertRaises(ValueError):
                    native_resources.check_no_resources(Path('/private/run'), NS,
                        Path('/approved/firecracker'), 3200000, native=settings)
            scanned.assert_not_called()
            uids.assert_not_called()


def complete_report():
    report = reports.new_report("sp12-abcdefgh")
    report.update(status="pass", kvm="executed", commit="a" * 40,
                  cleanup="confirmed", known_sandboxes=4, known_claims=4, staged_copies="removed")
    report["staged_files"] = {name: "removed" for name in reports.STAGED_FILES}
    report["hashes"] = {name: "b" * 64 for name in reports.HASH_NAMES}
    report["preflight"] = {name: "pass" for name in reports.PREFLIGHT_NAMES}
    report["phases"] = {name: "pass" for name in reports.PHASES}
    report["probes"] = {name: {"status": "pass"} for name in reports.PROBES}
    report['services']=dict(architecture=artifacts.ARCHITECTURE,candidate='a'*40,
        versions=dict(internal_protocol=1,center_store=5,host_store=15),center_sha256='b'*64,host_sha256='b'*64)
    report['service_boundary']=dict(public_api='center',center_unprivileged=True,
        redis_authority='validated',host_transport='mutual-tls',host_public_management=False)
    manifest=dict(schema_version=1,architecture=artifacts.ARCHITECTURE,candidate='a'*40,
        versions=report['services']['versions'],artifacts={role:dict(path='/approved/'+binary,sha256='b'*64)
            for role,binary in artifacts.BINARIES.items()})
    def role(uid,inode,checksum):
        return dict(uid=uid,gid=uid,config_sha256=checksum*64,
            directory=dict(device=1,inode=inode,uid=uid,gid=uid,mode=0o700))
    report['private_candidate_manifest']=manifest
    report['execution_manifest']=artifacts.execution_manifest(manifest,
        run_id=hashlib.sha256((report['run_id']+report['commit']).encode()).hexdigest(),
        candidate_manifest_sha256=report['hashes']['candidate_manifest'],
        roles=dict(center=role(1001,1,'a'),redis=role(1002,2,'b'),host=[role(0,3,'c')]),
        redis=dict(artifacts.REDIS_PIN,binary_sha256='d'*64,binary_bytes=1000),
        tls_sha256='e'*64,budget_sha256='f'*64)
    report['hashes']['execution_manifest']=hashlib.sha256((json.dumps(report['execution_manifest'],
        sort_keys=True,indent=2)+'\n').encode()).hexdigest()
    report['service_processes']={name:dict(pid=index+10,start_time_ticks=100,
        uid=uid,gid=uid,executable=dict(device=1,inode=index+20))
        for index,(name,uid) in enumerate((('center',1001),('redis',1002),('host-0',0)))}
    return report


def state(phase="Destroyed"):
    return {"owner": 1, "phase": phase, "terminal": phase == "Destroyed",
            "current_vm": None if phase == "Destroyed" else 0,
            "net_slot": None if phase == "Destroyed" else 0}


def rows(count=4, phase="Destroyed"):
    return {f"sb-{NS}-{number}": state(phase) for number in range(count)}


def original_center(count=4):
    return dict(sandboxes=[dict(private=False,create=dict(identity=dict(sandbox_id='sandbox-'+str(number+1),
        tenant_id='1',node=dict(deployment_id='domain',node_id='host',node_generation=1),request_window=1)),
        operations=[dict(intent=dict(kind='create'))]) for number in range(count)])


def fake_services(count=4):
    return SimpleNamespace(planned_audit=Mock(side_effect=lambda role:original_center(count) if role=='center' else {}),
        wait_ready=Mock(),processes=SimpleNamespace(children={'host-0':SimpleNamespace(poll=lambda:None)}))


def original_report(count=4):
    report=reports.new_report('sp12-abcdefgh')
    identities=[row['create']['identity'] for row in original_center(count)['sandboxes']]
    report['global_identities']={value['sandbox_id']:value for value in identities}
    report['global_native']={value['sandbox_id']:native for value,native in zip(identities,rows(count))}
    return report


def database(path, values):
    path.parent.mkdir(exist_ok=True)
    with sqlite3.connect(path) as connection:
        connection.execute("CREATE TABLE sandboxes(sandbox_id TEXT, owner_id TEXT, state_blob BLOB)")
        connection.execute("CREATE TABLE controller_checkpoint(id INTEGER,input_cut INTEGER,payload BLOB)")
        connection.execute("CREATE TABLE controller_inputs(sequence INTEGER,input_json BLOB)")
        connection.execute("INSERT INTO controller_checkpoint VALUES(1,0,?)", (
            json.dumps({"namespace": NS, "core": {"maintenance": {"retained": []}}}).encode(),))
        connection.executemany("INSERT INTO sandboxes VALUES(?,?,?)", values)


def smoke():
    return {"pinned_sdk": "2.51.0", "management": "create/connect/get/list/set_timeout/kill",
            "data": "command/timeout/kill/reconnect/large-output/files/directories",
            "authorization": "foreign-tenant/wrong-token/revoked-token/NUL-or-invalid-UTF8-path/duplicate-token/oversized-upload",
            "expiry": "automatic kill observed",
            "scope": "isolated real VM; template API, egress, Harbor and proofs not covered"}


def data_probe():
    case = lambda user: {"user": user, "sha256": {key: "a" * 64 for key in
                         ("text.txt", "nested/binary.bin", "nested/deep/empty")},
                         "reconnected_pid": 1, "killed_pid": 2}
    return {"sdk_version": "2.51.0",
            "sync": {"sandbox_id": "sandbox-3", "cases": [case("root"), case("ubuntu")]},
            "async": {"sandbox_id": "sandbox-4", "cases": [case("root"), case("ubuntu")]},
            "scope": "real controller/gateway/VM data calls; resource residuals and timeout/fault matrices require separate evidence"}


class ReportTests(unittest.TestCase):
    def test_old_or_mixed_execution_or_missing_actual_role_receipt_cannot_pass(self):
        for field,value in (('execution_manifest',None),('service_processes',{}),
                ('private_candidate_manifest',{})):
            report=complete_report()
            report[field]=value
            self.assertEqual(reports.safe_export(report)['status'],'fail')
        for owner in (0,True,1002):
            report=complete_report()
            report['service_processes']['center']['uid']=owner
            self.assertEqual(reports.safe_export(report)['status'],'fail')
        report=complete_report()
        report['execution_manifest']['candidate_manifest_sha256']='f'*64
        self.assertEqual(reports.safe_export(report)['status'],'fail')
    def test_pass_requires_every_phase_probe_hash_preflight_and_exact_full_count(self):
        self.assertEqual(reports.safe_export(complete_report())["status"], "pass")
        for field in ("phases", "probes", "hashes", "preflight"):
            for key in complete_report()[field]:
                report = complete_report()
                del report[field][key]
                self.assertNotEqual(reports.safe_export(report)["status"], "pass", (field, key))
        for field, value in (("known_sandboxes", 3), ("known_claims", 3), ("commit", None),
                             ("cleanup", "unknown"), ("kvm", "not_run"), ("status", "fail")):
            report = complete_report()
            report[field] = value
            self.assertNotEqual(reports.safe_export(report)["status"], "pass", field)

    def test_original_failure_survives_cleanup_success(self):
        report = complete_report()
        reports.record_failure(report, reports.PROBES[0], reports.Failure("probe_failed"))
        report["phases"][reports.PROBES[0]] = "pass"
        self.assertEqual(reports.safe_export(report)["status"], "fail")
        self.assertEqual(report["failures"][0]["code"], "probe_failed")

    def test_export_is_allowlist_with_secret_canaries_in_every_extra_field(self):
        report = complete_report()
        report.update(config=CANARY, database=CANARY, environment=CANARY, logs=CANARY,
                      private_identity={"namespace": CANARY}, path="/private/" + CANARY)
        report["hashes"]["extra"] = CANARY
        report["preflight"]["extra"] = CANARY
        report["probes"][reports.PROBES[0]]["raw"] = CANARY
        report["probes"][reports.PROBES[0]]["outcomes"] = [CANARY]
        reports.record_failure(report, "prepare", OSError(CANARY))
        report["failures"].append({"stage": CANARY, "code": CANARY, "message": CANARY})
        exported = json.dumps(reports.safe_export(report))
        self.assertNotIn(CANARY, exported)
        self.assertNotIn("/private/", exported)
        self.assertNotIn("private_identity", exported)

    def test_malformed_nested_values_never_escape_or_pass(self):
        for field in ("hashes", "preflight", "phases", "probes", "failures", "cleanup_errors"):
            for value in (None, [], [CANARY], CANARY, {CANARY: CANARY}):
                report = complete_report()
                report[field] = value
                result = reports.safe_export(report)
                self.assertNotIn(CANARY, json.dumps(result))
                if not (field in ("failures", "cleanup_errors") and value == []):
                    self.assertNotEqual(result["status"], "pass")
        for value in (None, [], CANARY):
            self.assertNotEqual(reports.safe_export(value)["status"], "pass")

    def test_intent_only_is_interrupted_and_unknown_not_replayed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            report = reports.read_report(root)
            self.assertEqual((report["status"], report["cleanup"]), ("interrupted", "unknown"))
            self.assertEqual(report["failures"][0]["code"], "unresolved_intent")

    def test_interruption_reader_retains_ready_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            reports.write_once(root / "ready.json", complete_report())
            report = reports.read_report(root)
            self.assertEqual(report["commit"], "a" * 40)
            self.assertEqual((report["status"], report["cleanup"]), ("interrupted", "unknown"))

    def test_durable_write_no_clobber_or_symlink_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = root / "target"
            target.write_text(CANARY)
            output = root / "terminal.json"
            output.symlink_to(target)
            with self.assertRaises(FileExistsError):
                reports.write_once(output, complete_report())
            self.assertEqual(target.read_text(), CANARY)
            self.assertTrue(output.is_symlink())
            output.unlink()
            reports.write_once(output, {"first": True})
            with self.assertRaises(FileExistsError):
                reports.write_once(output, complete_report())
            self.assertEqual(json.loads(output.read_text()), {"first": True})

    def test_post_link_directory_fsync_failure_invalidates_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            with patch.object(reports.os, "fsync", side_effect=[None, OSError(CANARY)]):
                with self.assertRaises(OSError):
                    reports.write_once(root / "terminal.json", complete_report())
            self.assertFalse((root / "terminal.json").exists())
            self.assertNotEqual(reports.read_report(root)["status"], "pass")

    def test_post_link_temporary_unlink_failure_invalidates_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            unlink = os.unlink
            calls = []
            def fail_first(path):
                calls.append(path)
                if len(calls) == 1:
                    raise OSError(CANARY)
                unlink(path)
            with patch.object(reports.os, "unlink", side_effect=fail_first):
                with self.assertRaises(OSError):
                    reports.write_once(root / "terminal.json", complete_report())
            self.assertFalse((root / "terminal.json").exists())

    def test_delivery_failure_downgrades_durable_pass(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            (root / "run.lock").touch(mode=0o600)
            reports.write_once(root / "terminal.json", complete_report())
            reports.write_once(root / "delivery-failed.json", {})
            self.assertEqual(reports.read_report(root)["status"], "fail")

    def test_probe_output_is_required_and_strict(self):
        self.assertEqual(reports.validate_probe(reports.PROBES[0], smoke(), NS), {"status": "pass"})
        self.assertEqual(reports.validate_probe(reports.PROBES[1], data_probe(), NS), {"status": "pass"})
        bad = [None, {}, {**smoke(), "pinned_sdk": "wrong"}, {**smoke(), "extra": CANARY}]
        for value in bad:
            with self.assertRaises(reports.Failure):
                reports.validate_probe(reports.PROBES[0], value, NS)
        for edit in (lambda v: v["sync"].update(sandbox_id="sb-foreign-1"),
                     lambda v: v["sync"]["cases"][0].update(killed_pid=True),
                     lambda v: v["async"].update(cases=[]),
                     lambda v: v["sync"]["cases"][0].update(user=CANARY)):
            value = data_probe()
            edit(value)
            with self.assertRaises(reports.Failure):
                reports.validate_probe(reports.PROBES[1], value, NS)


class IdentityTests(unittest.TestCase):
    def test_exact_retirement_survives_sql_pruning_but_absence_is_not_confirmation(self):
        ident = f'sb-{NS}-1'
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'controller.db'
            database(path, [])
            self.assertEqual(runner.durable_rows(path, NS), {})
            with self.assertRaises(reports.Failure):
                runner.wait_for_destroyed(path, NS, {ident}, timeout=0)
            retained = [[{'sandbox': 1}, {'owner': 1, 'ledger': {'sandbox': 1, 'result': 'Destroyed'}}]]
            with sqlite3.connect(path) as connection:
                connection.execute('UPDATE controller_checkpoint SET payload=?', (
                    json.dumps({'namespace': NS, 'core': {'maintenance': {'retained': retained}}}).encode(),))
            observed = runner.durable_rows(path, NS)
            self.assertEqual(set(observed), {ident})
            self.assertEqual(observed[ident]['phase'], 'Retired')
            self.assertTrue(runner.destroyed(observed))
            self.assertEqual(runner.wait_for_destroyed(path, NS, {ident}), observed)
            retained[0][1]['owner'] = 2
            with sqlite3.connect(path) as connection:
                connection.execute('UPDATE controller_checkpoint SET payload=?', (
                    json.dumps({'namespace': NS, 'core': {'maintenance': {'retained': retained}}}).encode(),))
            self.assertEqual(runner.durable_rows(path, NS), {})

    def test_real_sqlite_exact_identity_and_partial_destroyed_count(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "controller.db"
            database(path, [(ident, "tenant-1", json.dumps(value)) for ident, value in rows(2).items()])
            observed = runner.durable_rows(path, NS)
            self.assertEqual(len(observed), 2)
            self.assertTrue(runner.destroyed(observed))
            self.assertEqual(runner.wait_for_destroyed(path, NS, observed), observed)

    def test_missing_database_never_created(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "missing.db"
            with self.assertRaises(reports.Failure):
                runner.durable_rows(path, NS)
            self.assertFalse(path.exists())

    def test_foreign_malformed_and_inconsistent_rows_fail_closed(self):
        for ident, owner, value in ((f"sb-{NS}-01", "tenant-1", state()),
                                    ("sb-foreign-1", "tenant-1", state()),
                                    (f"sb-{NS}-1", "tenant-2", state()),
                                    (f"sb-{NS}-1", "tenant-1", {**state(), "owner": True}),
                                    (f"sb-{NS}-1", "tenant-1", {**state(), "phase": "invented"}),
                                    (f"sb-{NS}-1", "tenant-1", {**state(), "current_vm": True})):
            with tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / "controller.db"
                database(path, [(ident, owner, json.dumps(value))])
                with self.assertRaises(reports.Failure):
                    runner.durable_rows(path, NS)

    def test_404_or_204_is_not_terminal_proof(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "controller.db"
            database(path, [(ident, "tenant-1", json.dumps(value)) for ident, value in rows(1, "Revoking").items()])
            with self.assertRaises(reports.Failure) as failure:
                runner.wait_for_destroyed(path, NS, rows(1), timeout=0)
            self.assertEqual(failure.exception.code, "terminal_state_invalid")

    def test_cleanup_attempts_every_identity_after_first_delete_fails(self):
        report = original_report()
        process = fake_services()
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        with patch.object(runner, "durable_rows", return_value=rows()), \
                patch.object(runner, "kill_one", side_effect=[OSError(CANARY), None, None, None]) as kill, \
                patch.object(runner,'terminal_result',side_effect=[None]*4+[{'status':'cleaned'}]*4), \
                patch.object(runner, "wait_for_destroyed", return_value=rows()), \
                patch.object(runner, "assert_no_exact_resources", return_value=4):
            with self.assertRaises(reports.Failure):
                runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY, process, report)
        self.assertEqual([call.args[-1] for call in kill.call_args_list], sorted(report['global_identities']))
        self.assertEqual(report["cleanup"], "unknown")
        self.assertEqual(report["cleanup_errors"][0]["code"], "io_error")

    def test_partial_failure_audits_variable_claim_count_but_keeps_unknown(self):
        report = original_report(1)
        reports.record_failure(report, reports.PROBES[0], reports.Failure("probe_failed"))
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        with patch.object(runner, "durable_rows", return_value=rows(1)), \
                patch.object(runner, "kill_one") as kill, \
                patch.object(runner,'terminal_result',side_effect=[None,{'status':'cleaned'}]), \
                patch.object(runner, "wait_for_destroyed", return_value=rows(1)), \
                patch.object(runner, "assert_no_exact_resources", return_value=1) as audit:
            with self.assertRaises(reports.Failure):
                runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY,
                               fake_services(1), report)
        self.assertEqual(kill.call_count, 1)
        self.assertNotIn("expected", audit.call_args.kwargs)
        self.assertEqual(report["known_claims"], 1)
        self.assertEqual(report["cleanup"], "unknown")

    def test_cleanup_reads_late_committed_id_without_replaying_probe(self):
        report = original_report(2)
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        services=fake_services(2)
        services.planned_audit.side_effect=[original_center(1),{},original_center(2),{},original_center(2),{}]
        with patch.object(runner, "durable_rows", return_value=rows(2)), \
                patch.object(runner,'terminal_result',side_effect=[None,None,{'status':'cleaned'},None,{'status':'cleaned'},{'status':'cleaned'}]), \
                patch.object(runner, "kill_one") as kill, \
                patch.object(runner, "wait_for_destroyed", return_value=rows(2)), \
                patch.object(runner, "assert_no_exact_resources", return_value=2):
            runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY,
                           services, report)
        self.assertEqual([c.args[-1] for c in kill.call_args_list], ['sandbox-1','sandbox-2'])


class ProbeProcessTests(unittest.TestCase):
    def test_private_resource_callback_uses_exact_global_id_and_never_guesses_native(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve()
            requests=root/'resource-requests'
            requests.mkdir(mode=0o700)
            sandbox='sandbox-29'
            name=hashlib.sha256(sandbox.encode()).hexdigest()
            reports.write_once(requests/(name+'.ack.json'),dict(sandbox=sandbox,recorded=True))
            with patch.dict(os.environ,{'SANDPROOF_TEST_EVIDENCE_ROOT':str(root)}):
                resource_callback.record_resource_identity(sandbox)
            self.assertEqual(reports.json_file(requests/(name+'.request.json')),dict(sandbox=sandbox))
            self.assertFalse((root/'resource-evidence').exists())

    def test_private_resource_callback_foreign_nonboolean_or_linked_ack_is_unknown(self):
        for attack in ('foreign','integer','symlink','hardlink'):
            with self.subTest(attack=attack),tempfile.TemporaryDirectory() as temporary:
                root=Path(temporary).resolve()
                requests=root/'resource-requests'
                requests.mkdir(mode=0o700)
                sandbox='sandbox-29'
                ack=requests/(hashlib.sha256(sandbox.encode()).hexdigest()+'.ack.json')
                value=dict(sandbox='sandbox-30' if attack=='foreign' else sandbox,
                    recorded=1 if attack=='integer' else True)
                reports.write_once(ack,value)
                if attack=='symlink':
                    ack.rename(root/'target')
                    ack.symlink_to(root/'target')
                if attack=='hardlink':
                    os.link(ack,root/'target')
                with patch.dict(os.environ,{'SANDPROOF_TEST_EVIDENCE_ROOT':str(root)}), \
                        self.assertRaises((OSError,RuntimeError)):
                    resource_callback.record_resource_identity(sandbox)

    def test_native_or_unbounded_callback_id_rejected_before_request_write(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve()
            (root/'resource-requests').mkdir(mode=0o700)
            with patch.dict(os.environ,{'SANDPROOF_TEST_EVIDENCE_ROOT':str(root)}):
                for sandbox in ('sb-'+NS+'-1','sandbox-0','sandbox-01','sandbox-'+str(2**64),True):
                    with self.subTest(sandbox=sandbox),self.assertRaises(RuntimeError):
                        resource_callback.record_resource_identity(sandbox)
            self.assertEqual(list((root/'resource-requests').iterdir()),[])
    def run_fake_probe(self, body, *, timeout=5):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            scripts = root / "tests/system"
            scripts.mkdir(parents=True)
            (scripts / reports.PROBES[0]).write_text(body)
            with patch.object(runner, "CHECKOUT", root):
                return runner.run_probe(root, reports.PROBES[0], "http://127.0.0.1:1",
                                        runner.probe_environment("owner", "foreign"), NS, timeout=timeout)

    def test_fixed_child_environment_drops_optimization_proxies_and_secrets(self):
        with patch.dict(os.environ, {"PYTHONOPTIMIZE": "1", "PYTHONPATH": CANARY,
                                     "HTTPS_PROXY": CANARY, "PRIVATE_TOKEN": CANARY}):
            environment = runner.probe_environment("owner", "foreign")
            self.assertEqual(set(environment), {"PATH", "LANG", "SANDPROOF_TEST_API_KEY", "SANDPROOF_TEST_FOREIGN_API_KEY"})
            body = "import os,json\nassert __debug__\nassert 'HTTPS_PROXY' not in os.environ\nassert 'PRIVATE_TOKEN' not in os.environ\nprint(" + repr(json.dumps(smoke())) + ")\n"
            self.assertEqual(self.run_fake_probe(body), {"status": "pass"})

    def test_nonzero_missing_and_malformed_output(self):
        for body, code in (("raise SystemExit(3)", "probe_failed"), ("pass", "probe_output_invalid"),
                           ("print('{}')", "probe_output_invalid"), ("print('not-json')", "probe_output_invalid")):
            with self.assertRaises(reports.Failure) as failure:
                self.run_fake_probe(body)
            self.assertEqual(failure.exception.code, code)

    def test_timeout_reaps_exact_probe_child(self):
        real_popen = subprocess.Popen
        created = []
        def popen(*args, **kwargs):
            process = real_popen(*args, **kwargs)
            created.append(process)
            return process
        with patch.object(runner.subprocess, "Popen", side_effect=popen):
            with self.assertRaises(reports.Failure) as failure:
                self.run_fake_probe("import time; time.sleep(60)", timeout=0.1)
        self.assertEqual(failure.exception.code, "timeout")
        self.assertIsNotNone(created[0].poll())

    def test_log_fsync_secondary_does_not_replace_nonzero(self):
        with patch.object(runner.os, "fsync", side_effect=OSError(CANARY)):
            with self.assertRaises(reports.Failure) as failure:
                self.run_fake_probe("raise SystemExit(7)")
        self.assertEqual(failure.exception.code, "probe_failed")
        self.assertEqual(len(failure.exception.secondary), 2)

    def test_optimized_runner_preflight_is_rejected_before_host_tools(self):
        code = "import sys;sys.path.insert(0,'tests/system');import sdk_c02_acceptance as r;from types import SimpleNamespace\ntry:r.preflight(SimpleNamespace(), {})\nexcept r.Failure as e:print(e.code)\n"
        result = subprocess.run([sys.executable, "-O", "-c", code], cwd=ROOT,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "optimized_python")


class ResourceAuditTests(unittest.TestCase):
    def test_saved_original_identity_audits_after_claim_retirement_without_fabricating_count(self):
        ident = f'sb-{NS}-1'
        record = {'resource_id': f'vm-{NS}-0', 'pid': 123, 'start_time_ticks': 456}
        value = {'sandbox': ident, 'context': {'position': 0, 'binding': {'owner': 1, 'sandbox': 1, 'vm': 0}},
                 'record': record, 'network': {'resource_id': record['resource_id'], 'namespace_inode': 789}}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / 'resource-evidence'
            evidence.mkdir()
            (evidence / (ident + '.json')).write_text(json.dumps(value))
            real_entries, real_iterdir = runner.entries, Path.iterdir
            def controlled_entries(path):
                return [] if path == Path('/run/netns') else real_entries(path)
            def controlled_iterdir(path):
                return iter([]) if path == Path('/proc') else real_iterdir(path)
            with patch.object(runner, 'entries', side_effect=controlled_entries), \
                    patch.object(Path, 'iterdir', controlled_iterdir), \
                    patch.object(runner, 'process_start_ticks', return_value=None), \
                    patch.object(runner, 'processes_in_uid_range', return_value=[]), \
                    patch.object(runner.os, 'walk', return_value=iter([])), \
                    patch.object(Path, 'is_dir', return_value=True):
                self.assertEqual(runner.assert_no_exact_resources(root, NS, 1000, 'firecracker', expected=1), 1)
                with self.assertRaises(reports.Failure):
                    runner.assert_no_exact_resources(root, NS, 1000, 'firecracker', expected=4)
                with patch.object(runner, 'process_start_ticks', return_value=456):
                    with self.assertRaises(reports.Failure) as failure:
                        runner.assert_no_exact_resources(root, NS, 1000, 'firecracker', expected=1)
                    self.assertEqual(failure.exception.code, 'resource_residual')

    def audit(self, root, record, ticks, *, expected=None):
        claims = root / "jails/.claims"
        claim = claims / f"vm-{NS}-0"
        claim.mkdir(parents=True, exist_ok=True)
        if record is not None:
            (claim / "record.json").write_text(json.dumps(record))
        real_entries = runner.entries
        def controlled_entries(path):
            return [] if path == Path("/run/netns") else real_entries(path)
        with patch.object(runner, "entries", side_effect=controlled_entries), \
                patch.object(runner, "process_start_ticks", return_value=ticks), \
                patch.object(runner, "processes_in_uid_range", return_value=[]), \
                patch.object(runner.os, "walk", return_value=iter([])), \
                patch.object(Path, "is_dir", return_value=True):
            return runner.assert_no_exact_resources(root, NS, 1000, "firecracker", expected=expected)

    def test_pid_reuse_differs_from_live_exact_identity(self):
        record = {"resource_id": f"vm-{NS}-0", "pid": 123, "start_time_ticks": 456}
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(self.audit(Path(temporary), record, 789), 1)
            with self.assertRaises(reports.Failure) as failure:
                self.audit(Path(temporary), record, 456)
            self.assertEqual(failure.exception.code, "resource_residual")

    def test_missing_or_malformed_claim_never_proves_clean(self):
        record = {"resource_id": f"vm-{NS}-0", "pid": 123, "start_time_ticks": 456}
        for value in (None, {**record, "pid": True}, {**record, "pid": 0},
                      {**record, "start_time_ticks": 0}, {**record, "resource_id": "vm-other-0"}):
            with tempfile.TemporaryDirectory() as temporary:
                with self.assertRaises(reports.Failure):
                    self.audit(Path(temporary), value, None)

    def test_partial_audit_does_not_demand_four_tombstones(self):
        record = {"resource_id": f"vm-{NS}-0", "pid": 123, "start_time_ticks": 456}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(self.audit(root, record, None), 1)
            with self.assertRaises(reports.Failure):
                self.audit(root, record, None, expected=4)


class SupervisorTests(unittest.TestCase):
    def setUp(self):
        self.signals = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        self.addCleanup(lambda: [signal.signal(sig, handler) for sig, handler in self.signals.items()])

    def fixture(self, root):
        report = complete_report()
        report.update(reports.new_report("sp12-abcdefgh"))
        report["private_identity"] = {"namespace": NS}
        args = SimpleNamespace(scratch_parent=root, uid_base=1000, firecracker=Path("firecracker"))
        process = SimpleNamespace(poll=Mock(return_value=None))
        events = []
        def preflight(args, report):
            events.append("preflight")
            report["commit"] = "a" * 40
            report["hashes"] = complete_report()["hashes"]
            report["preflight"] = complete_report()["preflight"]
            report['services']=complete_report()['services']
            report['global_identities']=original_report()['global_identities']
            report['global_native']=original_report()['global_native']
        def cleanup(*args):
            events.append("cleanup")
            out = args[-1]
            out.update(cleanup="confirmed", known_sandboxes=4, known_claims=4)
            out["phases"]["cleanup"] = "pass"
            return set(rows())
        def stop(_process):
            events.append("stop")
            process.poll.return_value = 0
        services=fake_services()
        services.processes.children={'host-0':process}
        services.start=Mock()
        services.stop_all=Mock(side_effect=lambda:stop(None))
        services.cleanup_staging=Mock()
        services.manifest=complete_report()['private_candidate_manifest']
        services.execution=Mock(return_value=(complete_report()['execution_manifest'],complete_report()['service_processes']))
        process.services=services
        stack = ExitStack()
        patches = {"preflight": preflight, "prepare": Mock(return_value=services),
                   "run_probe": Mock(return_value={"status": "pass"}),
                   "durable_rows": Mock(return_value=rows()), "wait_for_destroyed": Mock(return_value=rows()),
                   "assert_no_exact_resources": Mock(return_value=4), "unchanged": Mock(),
                   "cleanup": cleanup, "remove_staged_copies": lambda root, report: report.update(staged_copies="removed", staged_files={name: "removed" for name in reports.STAGED_FILES}),
                   "free_loopback_port": Mock(return_value=12345)}
        for name, replacement in patches.items():
            stack.enter_context(patch.object(runner, name, side_effect=replacement))
        return args, report, process, events, stack

    def test_all_phases_and_stop_precede_terminal_publication(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args, report, _process, events, stack = self.fixture(root)
            with stack:
                runner.run_profile(args, root, report)
            self.assertEqual(events, ["preflight", "cleanup", "stop"])
            self.assertEqual(report["status"], "pass")
            self.assertNotEqual(reports.safe_export(report)["status"], "pass")
            report["phases"]["report"] = "pass"
            self.assertEqual(reports.safe_export(report)["status"], "pass")

    def test_each_phase_failure_is_sticky_and_server_always_stops(self):
        for target in ("preflight", "prepare", "start", "wait_ready", "run_probe",
                       "wait_for_destroyed", "assert_no_exact_resources", "unchanged"):
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                args, report, process, events, stack = self.fixture(root)
                owner=process.services if target in ('start','wait_ready') else runner
                with stack, patch.object(owner, target, side_effect=OSError(CANARY)):
                    runner.run_profile(args, root, report)
                self.assertEqual(report["status"], "fail")
                if target not in ('preflight','prepare'):
                    self.assertIn("stop", events)
                if target in ("preflight", "prepare"):
                    self.assertEqual(report["cleanup"], "not_created")
                    self.assertNotIn("cleanup", events)
                else:
                    self.assertIn("cleanup", events)
                self.assertNotIn(CANARY, json.dumps(reports.safe_export(report)))

    def test_original_failure_and_cleanup_and_stop_errors_are_separate(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, report, process, _events, stack = self.fixture(Path(temporary))
            with stack, patch.object(runner, "run_probe", side_effect=reports.Failure("probe_failed")), \
                    patch.object(runner, "cleanup", side_effect=reports.Failure("database_unknown")), \
                    patch.object(process.services, "stop_all", side_effect=OSError(CANARY)):
                runner.run_profile(args, Path(temporary), report)
            self.assertEqual(report["failures"], [{"stage": reports.PROBES[0], "code": "probe_failed"}])
            self.assertEqual([e["code"] for e in report["cleanup_errors"]], ["database_unknown", "io_error"])
            self.assertEqual(report["cleanup"], "unknown")

    def test_late_identity_after_server_stop_invalidates_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            args, report, _process, events, stack = self.fixture(Path(temporary))
            with stack, patch.object(runner, "durable_rows", side_effect=[rows(), rows(3)]):
                runner.run_profile(args, Path(temporary), report)
            self.assertEqual(events[-1], "stop")
            self.assertEqual((report["status"], report["cleanup"]), ("fail", "unknown"))

    def test_terminal_write_failure_returns_nonzero_and_retains_intent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            args = SimpleNamespace(scratch_parent=root, uid_base=1000)
            real_write = reports.write_once
            def write(path, report):
                if path.name == "terminal.json":
                    raise OSError(CANARY)
                real_write(path, report)
            def profile(_args, _root, report):
                report.update({**complete_report(), "run_id": report["run_id"]})
            output = io.StringIO()
            with patch.object(runner.host, "trusted_path", side_effect=lambda p: p), \
                    patch.object(runner, "run_profile", side_effect=profile), \
                    patch.object(runner, "write_once", side_effect=write), redirect_stdout(output):
                code = runner.exercise(args)
            report = json.loads(output.getvalue())
            self.assertEqual(code, 1)
            self.assertEqual(report["status"], "fail")
            self.assertEqual(report["failures"][-1]["code"], "report_write_failed")
            private = next(root.iterdir())
            self.assertTrue((private / "intent.json").exists())
            self.assertNotEqual(reports.read_report(private)["status"], "pass")
            self.assertNotIn(CANARY, output.getvalue())

    def test_live_writer_blocks_terminal_acceptance(self):
        import fcntl
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            reports.write_once(root / "terminal.json", complete_report())
            with (root / "run.lock").open("xb") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                self.assertNotEqual(reports.read_report(root)["status"], "pass")
            self.assertEqual(reports.read_report(root)["status"], "pass")

    def test_real_sigint_and_sigterm_produce_structured_terminal_failures(self):
        # Signal only this disposable supervisor; all host/service operations are fakes.
        for sig in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signal=sig), tempfile.TemporaryDirectory() as temporary:
                code = f'''
import sys,os,signal
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0,"tests/system")
import sdk_c02_acceptance as r
root=Path({temporary!r})
args=SimpleNamespace(scratch_parent=root,uid_base=1000,firecracker=Path("firecracker"))
def interrupt(*args,**kwargs): os.kill(os.getpid(),{int(sig)})
def preflight(_args,report): report['commit']='a'*40
from unittest.mock import Mock
services=SimpleNamespace(start=Mock(),wait_ready=Mock(),manifest={{}},execution=Mock(return_value=({{'services':{{}}}},{{}})),stop_all=Mock(),processes=SimpleNamespace(children={{'host-0':SimpleNamespace(poll=lambda:None)}}))
with patch.object(r.host,"trusted_path",side_effect=lambda p:p), patch.object(r,"preflight",side_effect=preflight), patch.object(r,"free_loopback_port",return_value=3100), patch.object(r,"prepare",return_value=services), patch.object(r,"run_probe",side_effect=interrupt), patch.object(r,"cleanup",side_effect=r.Failure("database_unknown")):
 raise SystemExit(r.exercise(args))
'''
                result = subprocess.run([sys.executable, "-c", code], cwd=ROOT, capture_output=True,
                                        text=True, timeout=10, check=False)
                self.assertEqual(result.returncode, 1, result.stderr)
                report = json.loads(result.stdout)
                self.assertEqual(report["status"], "interrupted")
                self.assertEqual(report["failures"][0]["code"], "interrupted")
                self.assertEqual(report["cleanup_errors"][0]["code"], "database_unknown")
                private = next(Path(temporary).iterdir())
                self.assertTrue((private / "terminal.json").exists())


class FinalBoundaryTests(unittest.TestCase):
    def test_schema_profile_and_run_id_cannot_be_normalized_into_pass(self):
        for field, value in (("schema_version", True), ("schema_version", 1),
                             ("profile", "other"), ("run_id", "/private/" + CANARY)):
            report = complete_report()
            report[field] = value
            self.assertNotEqual(reports.safe_export(report)["status"], "pass")

    def test_fifo_lock_does_not_block_export(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            os.mkfifo(root / "run.lock")
            with self.assertRaises(reports.Failure):
                reports.read_report(root)

    def test_double_publication_fault_keeps_original_and_marks_uncertainty(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            primary = OSError("original-ENOSPC")
            unlink = os.unlink
            def fail_invalidation(path):
                if Path(path).name == "terminal.json":
                    raise OSError("secondary-invalidation-failed")
                unlink(path)
            with patch.object(reports.os, "fsync", side_effect=[None, primary]), \
                    patch.object(reports.os, "unlink", side_effect=fail_invalidation):
                with self.assertRaises(OSError) as caught:
                    reports.write_once(root / "terminal.json", complete_report())
            self.assertIs(caught.exception, primary)
            self.assertTrue(primary.publication_unknown)
            self.assertEqual(len(primary.secondary), 1)
            # The leftover file is explicitly not proof of accepted publication.
            self.assertTrue((root / "terminal.json").exists())
            reports.write_once(root / "publication-unknown.json", {})
            reports.write_once(root / "intent.json", reports.new_report("sp12-abcdefgh"))
            (root / "run.lock").touch(mode=0o600)
            self.assertNotEqual(reports.read_report(root)["status"], "pass")

    def test_full_four_rows_without_claims_cannot_confirm_cleanup(self):
        report = original_report()
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        with patch.object(runner, "durable_rows", return_value=rows()), \
                patch.object(runner, "kill_one"), \
                patch.object(runner,'terminal_result',return_value={'status':'cleaned'}), \
                patch.object(runner, "wait_for_destroyed", return_value=rows()), \
                patch.object(runner, "assert_no_exact_resources", return_value=0):
            with self.assertRaises(reports.Failure):
                runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY,
                               fake_services(), report)
        self.assertEqual(report["cleanup"], "unknown")

    def test_unreaped_probe_cannot_race_cleanup_discovery_or_delete(self):
        report = reports.new_report("sp12-abcdefgh")
        report["probe_reaped"] = False
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        with patch.object(runner, "durable_rows") as discover, patch.object(runner, "kill_one") as kill:
            with self.assertRaises(reports.Failure):
                runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY,
                               SimpleNamespace(poll=lambda: None), report)
        discover.assert_not_called()
        kill.assert_not_called()
        self.assertEqual(report["cleanup"], "unknown")

    def test_previously_known_ids_do_not_replay_destroy_when_original_authority_unavailable(self):
        report = original_report(2)
        args = SimpleNamespace(uid_base=1000, firecracker=Path("firecracker"))
        services=fake_services(2)
        services.planned_audit.side_effect=reports.Failure('database_unknown')
        with patch.object(runner, "durable_rows") as native, \
                patch.object(runner, "kill_one") as kill:
            with self.assertRaises(reports.Failure):
                runner.cleanup(Path("/unused"), NS, args, None, "http://127.0.0.1:1", CANARY,
                               services, report)
        kill.assert_not_called()
        native.assert_not_called()
        self.assertEqual(len(report['global_identities']),2)
        self.assertEqual(report["cleanup"], "unknown")

    def test_existing_supervisor_hash_helper_stays_compatible(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "input"
            path.write_bytes(b"component")
            self.assertEqual(runner.sha256_file(path), reports.digest(path))

    def test_cleanup_uses_existing_exact_http_delete_semantics(self):
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        import threading
        import urllib.request
        requests = []
        class Handler(BaseHTTPRequestHandler):
            def do_DELETE(self):
                requests.append((self.path, self.headers.get("X-API-Key")))
                self.send_response(204 if len(requests) == 1 else 404)
                self.end_headers()
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            base = f"http://127.0.0.1:{server.server_port}"
            for ident in rows(2):
                runner.kill_one(opener, base, CANARY, ident)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.assertEqual(requests, [("/sandboxes/" + ident, CANARY) for ident in rows(2)])


class StagedCopyTests(unittest.TestCase):
    def staged(self, root):
        report = complete_report()
        report["private_staged"] = {}
        for name, filename in (("center", "sandproof-center"),("host", "sandproof-host"), ("kernel", "vmlinux"),
                               ("rootfs", "rootfs.ext4")):
            path = root / filename
            path.write_bytes(("staged-" + name).encode())
            info = path.stat()
            report["private_staged"][name] = {"device": info.st_dev, "inode": info.st_ino,
                "bytes": info.st_size,'owner':info.st_uid,'path':str(path.resolve()),
                'parent_device':path.parent.stat().st_dev,'parent_inode':path.parent.stat().st_ino}
            report["hashes"][name] = reports.digest(path)
            report["staged_files"][name] = "retained"
        return report

    def test_success_removes_only_exact_large_copies(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.staged(root)
            for name in ("intent.json", "terminal.json", "controller.db", "server.log", "unrelated"):
                (root / name).write_text(CANARY)
            runner.remove_staged_copies(root, report)
            self.assertEqual(report["staged_copies"], "removed")
            self.assertEqual({p.name for p in root.iterdir()},
                             {"intent.json", "terminal.json", "controller.db", "server.log", "unrelated"})
            self.assertTrue(all(p.read_text() == CANARY for p in root.iterdir()))

    def test_symlink_hardlink_replacement_and_changed_bytes_are_not_deleted(self):
        for attack in ("symlink", "hardlink", "replacement", "bytes"):
            with self.subTest(attack=attack), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                report = self.staged(root)
                victim = root / "sandproof-host"
                other = root / "unrelated"
                other.write_text(CANARY)
                if attack == "symlink":
                    victim.unlink()
                    victim.symlink_to(other)
                elif attack == "hardlink":
                    os.link(victim, root / "extra-link")
                elif attack == "replacement":
                    victim.rename(root / "original")
                    victim.write_bytes(b"staged-host")
                else:
                    victim.write_bytes(b"altered-bytes")
                with self.assertRaises((OSError, reports.Failure)):
                    runner.remove_staged_copies(root, report)
                self.assertTrue(victim.exists())
                self.assertEqual(other.read_text(), CANARY)
                self.assertEqual(report["staged_copies"], "unknown")

    def test_last_file_mismatch_preserves_all_four_copies(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.staged(root)
            (root / "rootfs.ext4").write_bytes(b"changed")
            with self.assertRaises(reports.Failure):
                runner.remove_staged_copies(root, report)
            self.assertTrue(all((root / name).exists() for name in
                                ("sandproof-center","sandproof-host", "vmlinux", "rootfs.ext4")))

    def test_partial_unlink_records_exact_removed_remaining_and_unknown_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.staged(root)
            unlink = os.unlink
            def fail_second(name, **kwargs):
                if name == "vmlinux":
                    raise OSError(CANARY)
                return unlink(name, **kwargs)
            with patch.object(runner.os, "unlink", side_effect=fail_second):
                with self.assertRaises(OSError):
                    runner.remove_staged_copies(root, report)
            self.assertFalse((root / "sandproof-center").exists())
            self.assertFalse((root / "sandproof-host").exists())
            self.assertTrue((root / "rootfs.ext4").exists())
            self.assertEqual(report["staged_files"],
                             {"center": "removed","host":"removed", "kernel": "unknown", "rootfs": "retained"})
            exported = reports.safe_export(report)
            self.assertEqual(exported["staged_files"], report["staged_files"])
            self.assertNotEqual(exported["status"], "pass")

    def test_unlink_failure_prevents_terminal_success(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.staged(root)
            with patch.object(runner.os, "unlink", side_effect=OSError(CANARY)):
                with self.assertRaises(OSError):
                    runner.remove_staged_copies(root, report)
            self.assertEqual(report["staged_copies"], "unknown")
            self.assertNotEqual(reports.safe_export(report)["status"], "pass")

    def test_post_unlink_directory_fsync_failure_keeps_truthful_removed_states(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            report = self.staged(root)
            with patch.object(runner.os, "fsync", side_effect=OSError(CANARY)):
                with self.assertRaises(OSError) as failure:
                    runner.remove_staged_copies(root, report)
            reports.record_failure(report, "artifact_cleanup", failure.exception)
            self.assertEqual(report["staged_copies"], "unknown")
            self.assertEqual(set(report["staged_files"].values()), {"removed"})
            self.assertFalse(any((root / name).exists() for name in
                                 ("sandproof-center","sandproof-host", "vmlinux", "rootfs.ext4")))
            self.assertEqual(report["phases"]["artifact_cleanup"], "fail")
            self.assertNotEqual(reports.safe_export(report)["status"], "pass")

    def test_supervisor_removes_copies_only_after_stop_and_never_after_failure(self):
        helper = SupervisorTests()
        original = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        try:
            for fail in (False, True):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    args, report, _process, events, stack = helper.fixture(root)
                    with stack, patch.object(runner, "remove_staged_copies",
                                             side_effect=lambda *args: events.append("copies")) as remove:
                        if fail:
                            with patch.object(runner, "run_probe", side_effect=reports.Failure("probe_failed")):
                                runner.run_profile(args, root, report)
                            remove.assert_not_called()
                        else:
                            runner.run_profile(args, root, report)
                            self.assertEqual(events[-2:], ["stop", "copies"])
        finally:
            for sig, handler in original.items():
                signal.signal(sig, handler)

    def test_supervisor_copy_cleanup_error_is_a_terminal_failure(self):
        helper = SupervisorTests()
        original = {sig: signal.getsignal(sig) for sig in (signal.SIGINT, signal.SIGTERM)}
        try:
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                args, report, _process, _events, stack = helper.fixture(root)
                with stack, patch.object(runner, "remove_staged_copies", side_effect=OSError(CANARY)):
                    runner.run_profile(args, root, report)
                self.assertEqual(report["status"], "fail")
                self.assertEqual(report["failures"][-1], {"stage": "artifact_cleanup", "code": "io_error"})
        finally:
            for sig, handler in original.items():
                signal.signal(sig, handler)


if __name__ == "__main__":
    unittest.main()
