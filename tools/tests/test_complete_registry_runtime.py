"""Static fixture/protocol checks only; no VM, network or profile activation."""
import copy
from contextlib import closing
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools.tests.test_bounded_acceptance import service_fixture
from tools.template_build_service import configuration
from tools.template_build_store import BuildError

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
from _center_host_services import test_credentials
SPEC = importlib.util.spec_from_file_location("complete_registry_fixture", ROOT / "tests/system/_complete_registry_runtime.py")
FIXTURE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = FIXTURE
SPEC.loader.exec_module(FIXTURE)


class CompleteRegistryFixtureTests(unittest.TestCase):
    def test_explicit_two_pool_policy_reaches_real_static_service_preflight(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as temporary:
            parent = Path(temporary).resolve()
            source = parent / "source"
            source.mkdir(mode=0o700)
            service, policy, _profile, namespaces = service_fixture(source)
            prior = json.loads(service.read_text())
            limits = prior["limits"]
            limits.update(max_cpu=2, max_memory_mib=1024, max_disk_mib=2048, timeout_seconds=600,
                max_input_bytes=256 * 1024 * 1024, max_expanded_bytes=512 * 1024 * 1024,
                max_log_bytes=2 * 1024 * 1024)
            registry = {key: value for key, value in prior["registry"].items() if key != "root"}
            root = parent / "fresh"
            for role in ("runtime", "builder"):
                profile = policy[role]["bounded_host"]["physical_storage"]
                profile["coordination_file"] = str(root / "physical-profile")
                for index, pool in enumerate(profile["pools"]):
                    pool.update(database=str(root / str(index) / "db/controller.db"),
                        node_state_dir=str(root / str(index) / "state"), chroot_base=str(root / str(index) / "jails"))
                profile["public_build"].update(root=str(root / "builds"), registry_root=str(root / "registry"))
            policy_path = parent / "policy.json"
            policy_path.write_text(json.dumps(policy))
            policy_path.chmod(0o600)
            kernel = source / "kernel"
            tools = parent / "tools"
            tools.mkdir()
            digest = hashlib.sha256(kernel.read_bytes()).hexdigest()
            for name in prior["tools"]:
                (tools / name).write_bytes(kernel.read_bytes())
            args = SimpleNamespace(bounded_policy=policy_path, run_root=root,
                namespace=namespaces[0], build_namespace=namespaces[1], scratch_parent=parent,
                firecracker=kernel, jailer=kernel, kernel=kernel,
                rootfs=source / "builder.ext4", builder_rootfs=None, tools=tools, uid_base=1000,
                cgroup_parent=source / "0/cgroup", build_cgroup_parent=source / "1/cgroup",
                service_cgroup_record=source / 'cgroup-record', center_uid=os.geteuid())
            untouched = copy.deepcopy(policy)
            def assembled(_args, _root, settings, catalogs, key, sdk_address, _record, **kwargs):
                # A controlled assembly port checks the real new DTO contract;
                # this static test does not claim TLS/Redis or KVM startup.
                credentials = test_credentials(key, kwargs['extra_credentials'])
                self.assertEqual([entry['tenant'] for entry in credentials], [1, 2])
                for suffix in ('registry','registry/inbox','registry/artifacts'):
                    (root / suffix).mkdir(mode=0o700)
                for index, native in enumerate(settings):
                    self.assertFalse({'bind','credentials','internal_build_vms','build_retry_windows'} & native.keys())
                    path=root / str(index) / 'host.json'
                    path.write_text(json.dumps(dict(native=native,inbox={},internal={},
                                                    template_cache={},artifact_source={})))
                    path.chmod(0o600)
                private=kwargs['private_builds']
                self.assertEqual(private['policy'], {field:settings[1][field] for field in
                    ('public_ipv4_enabled','max_timeout_secs','max_lifetime_secs','startup_timeout_secs')})
                return SimpleNamespace(native_pools=settings,
                    processes=SimpleNamespace(bindings={f'host-{index}':dict(config=root / str(index) / 'host.json')
                                                          for index in range(2)}),
                    layout=dict(helper_backend_address='127.0.0.1:3102',private_address='127.0.0.1:3101',
                                build_key_file=Path(prior['center']['credential_file'])),
                    sdk_url='http://' + sdk_address,
                    center_config=dict(domain='center.invalid',templates={'templates':[entry for catalog in catalogs
                        for entry in catalog['templates']]},template_registry=dict(root=str(root/'registry'),**registry),
                        private_builds=private))
            with patch.object(FIXTURE, "require_bounded_host_tools"), \
                    patch.object(FIXTURE, "file_hash", return_value=digest), \
                    patch.object(FIXTURE.RuntimeServices, 'prepare', side_effect=assembled):
                deployment = FIXTURE.prepare_registry_runtime(args,
                    tool_hashes={name: digest for name in prior["tools"]}, registry_limits=registry,
                    build_limits=limits, ports=[3100, 3101, 3102])
            checked = configuration(deployment.build_config, require_physical_lease=False)
            self.assertEqual(checked["physical_storage_profile"], policy["runtime"]["bounded_host"]["physical_storage"])
            self.assertNotIn("internal_build_vms", checked["builder"])
            self.assertNotIn("build_retry_windows", checked["builder"])
            self.assertNotIn("max_vm_id", checked["runtime"])
            self.assertNotIn("max_vm_id", checked["builder"])
            self.assertEqual((root / "physical-profile").read_bytes(), b"")
            self.assertEqual(json.loads(policy_path.read_text()), untouched)
            self.assertNotEqual(deployment.key, deployment.foreign_key)
            self.assertEqual(deployment.services.sdk_url, "http://127.0.0.1:3100")
            self.assertEqual(checked['center']['private_address'], '127.0.0.1:3101')
            self.assertEqual(checked['center']['max_lifetime_secs'], checked['builder']['max_lifetime_secs'])
            self.assertNotIn('credentials', checked['runtime'])
            with patch.object(FIXTURE, "require_bounded_host_tools"):
                with self.assertRaises(FileExistsError):
                    FIXTURE.prepare_registry_runtime(args,
                        tool_hashes={name: digest for name in prior["tools"]}, registry_limits=registry,
                        build_limits=limits, ports=[3100, 3101, 3102])

    def test_absent_policy_fails_before_creating_a_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "uncreated"
            with self.assertRaises(ValueError):
                FIXTURE.prepare_registry_runtime(SimpleNamespace(bounded_policy=None),
                    tool_hashes={}, registry_limits={}, build_limits={}, ports=[1, 2, 3])
            self.assertFalse(root.exists())

    def test_permanent_frontier_does_not_depend_on_retained_sandbox_rows(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary).resolve() / "controller.db"
            with closing(sqlite3.connect(path)) as database, database:
                database.execute("CREATE TABLE controller_id_highwaters(kind TEXT,scope TEXT,high_water TEXT)")
                database.execute("INSERT INTO controller_id_highwaters VALUES('sandbox','','13')")
            self.assertEqual(FIXTURE.sandbox_highwater(path), 13)
            with closing(sqlite3.connect(path)) as database, database:
                database.execute("DELETE FROM controller_id_highwaters")
            with self.assertRaises(ValueError):
                FIXTURE.sandbox_highwater(path)

    def test_live_projection_cannot_be_reported_as_terminal_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary).resolve() / "controller.db"
            with closing(sqlite3.connect(path)) as database, database:
                database.execute("CREATE TABLE sandboxes(state_blob TEXT)")
                database.executemany("INSERT INTO sandboxes VALUES(?)", [(json.dumps({"phase": phase}),)
                    for phase in ("Destroyed", "Starting")])
            with self.assertRaises(AssertionError):
                FIXTURE.assert_terminal_rows(path)
            with closing(sqlite3.connect(path)) as database, database:
                database.execute("UPDATE sandboxes SET state_blob=?", (json.dumps({"phase": "Destroyed"}),))
            FIXTURE.assert_terminal_rows(path)


class PrivateBuilderProtocolTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.deployment = FIXTURE.RegistryDeployment(self.root, ["a" * 16, "b" * 16],
            [{}, {}], "runtime-key", "foreign-key", Mock(), self.root / "build.json")
        self.calls = []
        self.reclaimed = False
        self.unknown_post = False
        self.unknown_settlement = False
        self.absent = False
        self.backend = Mock(request=self.request, backend_key='private-key',
                            config={'center':{'private_address':'127.0.0.1:2','domain':'center.invalid'}})
        self.addCleanup(patch.stopall)
        patch.object(FIXTURE, "configuration", return_value={}).start()
        patch.object(FIXTURE, "Backend", return_value=self.backend).start()
        self.sandbox = Mock()
        self.connection = Mock()
        patch.dict(sys.modules, {
            "e2b": SimpleNamespace(Sandbox=self.sandbox),
            "e2b.connection_config": SimpleNamespace(ConnectionConfig=self.connection),
            "packaging.version": SimpleNamespace(Version=lambda value: value),
        }).start()

    def request(self, pool, method, path, body=None, **kwargs):
        self.assertEqual(pool, "builder")
        self.assertEqual(kwargs['owner'], 1)
        self.calls.append((method, path, body))
        if path == "/sandproof/internal-build-windows":
            return {"requestWindow": "original-window", "expiresAtMs": 1234}
        if path.endswith("/close"):
            return {"requestWindow": "original-window", "closed": True}
        if path.endswith("/settle"):
            intent = json.loads(next(self.root.glob("builder-intent-*.json")).read_text())
            self.assertEqual(intent["builder_settled"], body)
            if self.unknown_settlement:
                self.unknown_settlement = False
                raise BuildError(502, "SettlementUnknown")
            return {"requestWindow": "original-window", "settled": True}
        if method == "DELETE":
            self.reclaimed = True
            return None
        if method == "POST":
            intent = json.loads(next(self.root.glob("builder-intent-*.json")).read_text())
            self.assertEqual(intent["requestWindow"], body["requestWindow"])
            self.assertIn(intent["id"], path)
            self.assertNotIn("key", intent)
            if self.unknown_post:
                raise BuildError(502, "LaunchUnknown")
        if self.absent and method == "GET":
            raise BuildError(404, "NotFound")
        return {"sandboxID": "sb-builder-1", "cleanupComplete": self.reclaimed,
            "session": {"sandboxID": "sb-builder-1", "envdVersion": "0.9.0", "envdAccessToken": "private-session"}}

    def test_private_create_persists_original_window_then_fences_reclaims_and_settles(self):
        _sandbox, identity = self.deployment.open_builder(timeout=600, internet=False)
        self.deployment.close_builder(identity)
        self.assertEqual([method for method, _path, _body in self.calls],
                         ["POST", "POST", "POST", "GET", "DELETE", "GET", "POST"])
        self.assertEqual(self.calls[1][2], {"timeout": 600, "allowInternetAccess": False, "requestWindow": "original-window"})
        self.connection.assert_called_once()
        self.assertEqual(self.connection.call_args.kwargs['api_url'], 'http://127.0.0.1:2')
        self.assertEqual(self.connection.call_args.kwargs['extra_sandbox_headers'],
            {'X-Access-Token':'private-session','x-sandproof-build-key':'private-key','x-sandproof-build-owner':'1'})
        self.sandbox.create.assert_not_called()
        count = len(self.calls)
        self.deployment.close_builders()
        self.assertEqual(len(self.calls), count)

    def test_unknown_create_uses_saved_identity_for_cleanup_without_replacement_post(self):
        self.unknown_post = True
        with self.assertRaises(BuildError):
            self.deployment.open_builder()
        self.deployment.close_builders()
        vm_posts = [path for method, path, _body in self.calls
                    if method == "POST" and "/internal-build-vms/" in path and not path.endswith("/settle")]
        self.assertEqual(len(vm_posts), 1)
        intent = json.loads(next(self.root.glob("builder-intent-*.json")).read_text())
        self.assertTrue(intent["settled"])
        self.assertEqual(intent["builder_settled"]["sandboxID"], "sb-builder-1")

    def test_fenced_absence_settles_without_claiming_a_missing_vm_was_destroyed(self):
        self.unknown_post = True
        with self.assertRaises(BuildError):
            self.deployment.open_builder()
        self.absent = True
        self.deployment.close_builders()
        self.assertFalse(any(method == "DELETE" for method, _path, _body in self.calls))
        intent = json.loads(next(self.root.glob("builder-intent-*.json")).read_text())
        self.assertEqual(intent["builder_settled"], {"requestWindow": "original-window", "sandboxID": None})

    def test_lost_settlement_reply_reuses_durable_certificate_without_requery_or_refence(self):
        _sandbox, identity = self.deployment.open_builder()
        self.unknown_settlement = True
        with self.assertRaises(BuildError):
            self.deployment.close_builder(identity)
        count = len(self.calls)
        original_settlement = self.calls[-1]
        self.deployment.close_builders()
        self.assertEqual(self.calls[count:], [original_settlement])


if __name__ == "__main__":
    unittest.main()
