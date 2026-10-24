"""Static supervision fixtures only: no VM, mount or production activation."""
import copy
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch

from tools.bounded_acceptance import (
    acceptance_disk_budgets, acceptance_pool_disk_budget, bounded_acceptance_layout, require_bounded_host_tools,
    wait_private_build_window, provision_acceptance_coordination, validate_acceptance_socket_paths,
    prepared_ci_physical_profile, prepared_ci_runtime_configuration,
)
from tools.template_physical_storage import envelope
from tools.template_build_config import apply_acceptance_build_policy
from tools.template_build_service import configuration
from tools.template_build_store import BuildStore
from tools.tests.test_template_build_config import GIB, MIB, inputs, transfer_settings
from tools.tests.test_template_physical_storage import activate_for_test


def service_fixture(root):
    """Build an explicit synthetic full profile, not a deployment default."""
    runtime, builder, limits, base, registry_config = inputs()
    limits.update(max_jobs=4, max_jobs_per_tenant=4)
    kernel = root / "kernel"
    kernel.write_bytes(b"synthetic kernel")
    disk = root / "builder.ext4"
    with disk.open("wb") as output:
        output.truncate(32 * GIB)  # sparse trusted fixture, never mounted
    namespaces = ["0123456789abcdef", "123456789abcdef0"]
    pools = []
    for index, settings in enumerate((runtime, builder)):
        parent = root / str(index)
        parent.mkdir(mode=0o700)
        for name in ("db", "state", "p", "cgroup"):
            (parent / name).mkdir(mode=0o700)
        settings.update(namespace=namespaces[index], database=str(parent / "db/controller.db"),
                        node_state_dir=str(parent / "state"), chroot_base=str(parent / "p/v"),
                        uid_base=1000 + index * 100,
                        kernel=str(kernel), firecracker=str(kernel),
                        host_resources={"cgroup_parent": str(parent / "cgroup")})
        settings["data_transfer"] = transfer_settings() | settings.get("data_transfer", {})
        settings["bounded_host"] = {"uid_base": settings["uid_base"], "uplink_base": index * 2,
            "console_policy": "discard", "maintenance": {"max_retained_results": 10, "max_host_slots": 2,
                "windows": {"max_open_windows": 2, "max_duration_ms": 600000}}}
        settings["maintenance"] = {"capacity": {"max_online_storage_bytes": MIB}}
        pools.append({"role": "builder" if index else "runtime", "namespace": namespaces[index],
                      **{field: settings[field] for field in ("database", "node_state_dir", "chroot_base")},
                      "rootfs_budget_bytes": settings["max_disk_bytes_global"] - settings["cleanup_reserve_bytes"],
                      "max_host_slots": 2, "kernel_copy_bytes": 20 * GIB, "runtime_copy_bytes": MIB,
                      "cleanup_reserve_bytes": settings["cleanup_reserve_bytes"], "host_metadata_bytes": MIB,
                      "host_metadata_inodes": 20, "jail_inodes_per_slot": 12})
    registry = root / "registry"
    for directory in (registry, registry / "inbox", registry / "artifacts", root / "builds"):
        directory.mkdir(mode=0o700)
    runtime["templates"]["templates"] = []
    registry_config.update(root=str(registry), max_templates_global=2,max_templates_per_tenant=2,
        max_artifact_bytes_global=32 * GIB, max_artifact_bytes_per_tenant=20 * GIB)
    private_windows = {"window_duration_ms": 600000, "max_anchors": 4}
    base.update(template_id="actual-base", owner=None)
    base["rootfs"]["path"] = str(disk)
    base["kernel"] = {"size_bytes": kernel.stat().st_size}
    builder["templates"].update(templates=[base], aliases={"base": "actual-base"})
    bounded = {"namespace": 17, "windows": {"max_open_windows": 4, "max_duration_ms": 600000},
               "retention_ms": 1000, "max_request_bytes": 2048, "max_result_bytes": 8192,
               "max_record_bytes": 65536, "database_bytes": 8 * MIB, "wal_bytes": 17 * MIB,
               "recovery_reserve_bytes": 8 * MIB,
               "helper": {"path": str(kernel), "sha256": hashlib.sha256(kernel.read_bytes()).hexdigest(),
                          "owner_uid": os.geteuid(), "max_input_bytes": 65536,
                          "max_output_bytes": 65536, "timeout_seconds": 5}}
    profile = {"version": 1, "coordination_file": str(root / "physical-profile"), "pools": pools,
               "public_build": {"workspace_owner_uid": os.geteuid(), "registry_owner_uid": os.geteuid(),
                                "root": str(root / "builds"), "registry_root": str(registry),
                                "workspace_bytes": limits["workspace_bytes"],
                                "registry_bytes": 32 * GIB + 2 * (65536 + 87), "max_jobs": 4,
                                "max_templates": 2, "inode_budget": 128},
               "host_template_caches": [],
               "service_data": [],
               "filesystem_overhead_bytes": 4 * MIB, "filesystem_overhead_inodes": 512}
    for settings in (runtime, builder):
        settings["bounded_host"]["physical_storage"] = copy.deepcopy(profile)
    config = {"root": str(root / "builds"), "bind": "127.0.0.1:3102",
              "runtime_config": str(root / "runtime.json"), "builder_config": str(root / "builder.json"),
              "tools": {name: {"path": str(kernel), "sha256": hashlib.sha256(kernel.read_bytes()).hexdigest()}
                        for name in ("buildctl", "buildkitd", "buildkit-runc", "umoci", "envd", "busybox", "regctl")},
              "limits": limits, "dns_servers": ["1.1.1.1"], "bounded_builds": bounded,
              "registry":registry_config,"registry_owner_uid":os.geteuid(),"builder_template":copy.deepcopy(base),
              "center":{"private_address":"127.0.0.1:3103","domain":"example.invalid",
                        "credential_file":str(root/'private-build.key'),"private_windows":private_windows,
                        "max_timeout_secs":runtime['max_timeout_secs'],"max_lifetime_secs":runtime['max_lifetime_secs']}}
    (root/'private-build.key').write_text('private-test-helper-key-0000000001')
    (root/'private-build.key').chmod(0o600)
    def host_file(native):
        # Explicit typed shape for static Python preflight only. No TLS, cache
        # or native activation is claimed by these component placeholders.
        return dict(native=native,inbox={},internal={},template_cache={},artifact_source={})
    for path, value in ((Path(config["runtime_config"]), host_file(runtime)), (Path(config["builder_config"]), host_file(builder)),
                        (root / "service.json", config)):
        path.write_text(json.dumps(value))
        path.chmod(0o600)
    policy = {"runtime": {key: copy.deepcopy(runtime[key]) for key in ("bounded_host", "maintenance")},
              "builder": {key: copy.deepcopy(builder[key]) for key in ("bounded_host", "maintenance")}
                         | {'build_retry_windows':copy.deepcopy(private_windows)},
              "public": {"bounded_builds": copy.deepcopy(bounded),
                         "workspace_bytes": limits["workspace_bytes"],
                         "workspace_bytes_per_tenant": limits["workspace_bytes_per_tenant"]}}
    return root / "service.json", policy, profile, namespaces


class AcceptanceDiskBudgetTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.service, self.policy, profile, _ = service_fixture(Path(temporary.name).resolve())
        self.deployment = {pool['role']: {
            'max_disk_bytes_global': pool['rootfs_budget_bytes'] + pool['cleanup_reserve_bytes'],
            'max_disk_bytes_per_tenant': pool['rootfs_budget_bytes'],
            'cleanup_reserve_bytes': pool['cleanup_reserve_bytes'],
            'max_active_global': 2, 'max_timeout_secs': 600,
        } for pool in profile['pools']}
        self.deployment['host'] = {'minimum_free_disk_bytes': 200 * GIB,
                                  'runtime_disk_bytes': profile['pools'][0]['rootfs_budget_bytes']}

    def grow_profile(self):
        for role in ('runtime', 'builder'):
            pools = self.policy[role]['bounded_host']['physical_storage']['pools']
            pools[0]['cleanup_reserve_bytes'] += GIB
            pools[0]['rootfs_budget_bytes'] += 4 * GIB
            pools[1]['cleanup_reserve_bytes'] += 2 * GIB

    def test_physical_reserves_reach_both_pools_and_host_without_mutating_inputs(self):
        self.grow_profile()
        original = copy.deepcopy(self.deployment)
        original_policy = copy.deepcopy(self.policy)
        updated = acceptance_disk_budgets(self.deployment, self.policy)
        self.assertEqual(updated['runtime']['cleanup_reserve_bytes'], original['runtime']['cleanup_reserve_bytes'] + GIB)
        self.assertEqual(updated['runtime']['max_disk_bytes_global'], original['runtime']['max_disk_bytes_global'] + 5 * GIB)
        self.assertEqual(updated['builder']['cleanup_reserve_bytes'], original['builder']['cleanup_reserve_bytes'] + 2 * GIB)
        self.assertEqual(updated['host']['minimum_free_disk_bytes'], 207 * GIB)
        self.assertEqual(updated['host']['runtime_disk_bytes'], original['host']['runtime_disk_bytes'] + 4 * GIB)
        for role in ('runtime', 'builder'):
            for field in ('max_active_global', 'max_timeout_secs'):
                self.assertEqual(updated[role][field], original[role][field])
        self.assertEqual(self.deployment, original)
        self.assertEqual(self.policy, original_policy)
        with self.assertRaises(ValueError):
            acceptance_disk_budgets(original, None)

    def test_larger_total_cannot_hide_a_smaller_tenant_or_cleanup_budget(self):
        for field, other in (('rootfs_budget_bytes', 'cleanup_reserve_bytes'),
                             ('cleanup_reserve_bytes', 'rootfs_budget_bytes')):
            policy = copy.deepcopy(self.policy)
            for role in ('runtime', 'builder'):
                pool = policy[role]['bounded_host']['physical_storage']['pools'][0]
                pool[field] -= 1
                pool[other] += 10 * GIB
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'undercut'):
                acceptance_disk_budgets(self.deployment, policy)
        self.grow_profile()
        self.deployment['host']['minimum_free_disk_bytes'] = 2**63 - 1
        with self.assertRaisesRegex(ValueError, 'overflow'):
            acceptance_disk_budgets(self.deployment, self.policy)

    def test_larger_profile_passes_real_service_preflight_only_with_matching_quotas(self):
        self.grow_profile()
        service = json.loads(self.service.read_text())
        host_files={role:json.loads(Path(service[role+'_config']).read_text()) for role in ('runtime','builder')}
        runtime, builder = [host_files[role]['native'] for role in ('runtime','builder')]
        # Generic acceptance assigns the usable pool quota to its one tenant.
        for settings in (runtime, builder):
            settings['max_disk_bytes_per_tenant'] = (
                settings['max_disk_bytes_global'] - settings['cleanup_reserve_bytes'])
        with self.assertRaisesRegex(ValueError, 'differs from a configured pool budget'):
            apply_acceptance_build_policy(copy.deepcopy(runtime), copy.deepcopy(builder),
                                          copy.deepcopy(service), self.policy)
        deployment = acceptance_disk_budgets(
            {'runtime': runtime, 'builder': builder, 'host': self.deployment['host']}, self.policy)
        self.assertEqual(apply_acceptance_build_policy(
            deployment['runtime'], deployment['builder'], service, self.policy), 'windowed')
        for role in ('runtime', 'builder'):
            host_files[role]['native']=deployment[role]
            Path(service[role + '_config']).write_text(json.dumps(host_files[role]))
        self.service.write_text(json.dumps(service))
        configuration(self.service, require_physical_lease=False)


class PreparedCIDiskBudgetTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.service, self.policy, self.original, _ = service_fixture(self.root)
        cfg = json.loads(self.service.read_text())
        self.runtime = json.loads(Path(cfg['runtime_config']).read_text())['native']
        self.runtime['templates']['templates'] = [{
            'template_id': 'ci-fixed', 'kernel': {'path': str(self.root / 'kernel'), 'size_bytes': 64 * MIB},
            'rootfs': {'path': str(self.root / 'ci.ext4'), 'size_bytes': 10 * GIB},
            'resources': {'cpu_count': 1, 'memory_mib': 2048, 'disk_bytes': 10 * GIB}}]

    def profile(self, **changes):
        return prepared_ci_physical_profile(self.policy, **({'kernel_bytes': 64 * MIB,
            'runtime_bytes': MIB, 'disk_budget_bytes': 200 * GIB} | changes))

    def test_runtime_only_keeps_full_cleanup_and_twice_envelope_under_200_gib(self):
        original = copy.deepcopy(self.policy)
        profile = self.profile()
        self.assertEqual(self.policy, original)
        self.assertIsNone(profile['public_build'])
        self.assertEqual([p['role'] for p in profile['pools']], ['runtime'])
        for field in ('rootfs_budget_bytes', 'cleanup_reserve_bytes', 'host_metadata_bytes',
                      'host_metadata_inodes', 'max_host_slots', 'jail_inodes_per_slot'):
            self.assertEqual(profile['pools'][0][field], self.original['pools'][0][field])
        peak, _ = envelope(profile)
        self.assertLessEqual(2 * peak, 200 * GIB)
        self.assertEqual(self.profile(disk_budget_bytes=2 * peak), profile)
        with self.assertRaisesRegex(ValueError, 'exceeds disk ceiling'):
            self.profile(disk_budget_bytes=2 * peak - 1)

    def test_invalid_ceiling_artifacts_and_oversized_runtime_are_rejected(self):
        for value in (None, True, 0, 201 * GIB):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.profile(disk_budget_bytes=value)
        for field, value in (('kernel_bytes', True), ('kernel_bytes', 21 * GIB),
                             ('runtime_bytes', 0), ('runtime_bytes', 2 * MIB)):
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                self.profile(**{field: value})
        for role in ('runtime', 'builder'):
            self.policy[role]['bounded_host']['physical_storage']['pools'][0]['rootfs_budget_bytes'] = 101 * GIB
        with self.assertRaisesRegex(ValueError, 'exceeds disk ceiling'):
            self.profile()

    def test_static_configuration_has_no_registry_and_preserves_original_task_resources(self):
        original = copy.deepcopy(self.runtime)
        profile = self.profile()
        result = prepared_ci_runtime_configuration(self.runtime, self.policy, profile)
        self.assertEqual(self.runtime, original)
        self.assertNotIn('template_registry', result)
        self.assertNotIn('internal_build_vms', result)
        self.assertNotIn('build_retry_windows', result)
        self.assertEqual(result['templates']['templates'], original['templates']['templates'])
        self.assertEqual(result['data_transfer'], original['data_transfer'])
        self.assertEqual(result['bounded_host']['physical_storage'], profile)
        self.assertEqual((result['vcpu_count'], result['memory_mib']), (1, 2048))
        self.assertEqual(result['rootfs'], str(self.root / 'ci.ext4'))

    def test_foreign_layout_fallback_or_oversized_template_is_not_registered(self):
        profile = self.profile()
        for change in ('fallback', 'empty', 'layout', 'disk', 'kernel'):
            settings = copy.deepcopy(self.runtime)
            if change == 'fallback':
                settings['templates']['templates'][0]['template_id'] = 'runtime-base'
            elif change == 'empty':
                settings['templates']['templates'] = []
            elif change == 'layout':
                settings['namespace'] = 'f' * 16
            else:
                field = 'rootfs_budget_bytes' if change == 'disk' else 'kernel_copy_bytes'
                artifact = 'rootfs' if change == 'disk' else 'kernel'
                settings['templates']['templates'][0][artifact]['size_bytes'] = profile['pools'][0][field] + 1
            with self.subTest(change=change), self.assertRaises(ValueError):
                prepared_ci_runtime_configuration(settings, self.policy, profile)


class PrivateBuildWindowObservationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.database = Path(temporary.name).resolve() / "builds.db"
        self.job = "a" * 32
        with closing(sqlite3.connect(self.database)) as connection:
            connection.execute("CREATE TABLE jobs(id TEXT PRIMARY KEY,state TEXT,private_window TEXT)")
            connection.execute("INSERT INTO jobs VALUES(?,?,?)", (self.job, "Building", None))
            connection.execute("INSERT INTO jobs VALUES(?,?,?)", ("b" * 32, "Building", '"bw-other"'))
            connection.commit()
        self.now = 0.0

    def test_null_then_committed_original_window_closes_reader_before_wait(self):
        connect = sqlite3.connect
        readers = []

        def observe(*args, **kwargs):
            self.assertEqual(args, (self.database.as_uri() + "?mode=ro",))
            self.assertEqual(kwargs, {"uri": True, "timeout": 0})
            connection = connect(*args, **kwargs)
            readers.append(connection)
            return connection

        def persist_original(delay):
            with self.assertRaises(sqlite3.ProgrammingError):
                readers[-1].execute("SELECT 1")
            with closing(connect(self.database)) as writer:
                writer.execute("UPDATE jobs SET private_window=? WHERE id=?", ('"bw-private-1-0"', self.job))
                writer.commit()
            self.now += delay

        with patch("tools.bounded_acceptance.sqlite3.connect", side_effect=observe):
            self.assertEqual(wait_private_build_window(self.database, self.job,
                clock=lambda: self.now, sleep=persist_original), "bw-private-1-0")
        self.assertEqual(len(readers), 2)
        for reader in readers:
            with self.assertRaises(sqlite3.ProgrammingError):
                reader.execute("SELECT 1")

    def test_permanent_null_terminal_missing_or_invalid_original_window_refuses(self):
        def advance(delay):
            self.now += delay

        with self.assertRaisesRegex(TimeoutError, "observation deadline"):
            wait_private_build_window(self.database, self.job, timeout=0.25,
                clock=lambda: self.now, sleep=advance)
        self.assertEqual(self.now, 0.25)
        for state, window in (("Ready", None), ("Failed", None), ("Building", "not-json"),
                              ("Building", '""'), ("Building", "null"), ("Building", '{}')):
            with self.subTest(state=state, window=window):
                with closing(sqlite3.connect(self.database)) as connection:
                    connection.execute("UPDATE jobs SET state=?,private_window=? WHERE id=?", (state, window, self.job))
                    connection.commit()
                with self.assertRaises(ValueError):
                    wait_private_build_window(self.database, self.job, clock=lambda: self.now,
                        sleep=lambda _delay: self.fail("invalid or terminal record must not wait"))
        with self.assertRaisesRegex(ValueError, "record is missing"):
            wait_private_build_window(self.database, "c" * 32, clock=lambda: self.now,
                sleep=lambda _delay: self.fail("missing original record must not wait"))


class BoundedToolsTests(unittest.TestCase):
    def test_missing_or_nonexecutable_dependency_refuses_before_launch(self):
        with patch('tools.check_kvm_host.trusted_path', side_effect=FileNotFoundError):
            with self.assertRaisesRegex(ValueError, 'conntrack'):
                require_bounded_host_tools()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'conntrack'
            path.write_bytes(b'test dependency')
            with patch('tools.check_kvm_host.trusted_path', return_value=path), \
                    patch('tools.bounded_acceptance.os.access', return_value=False):
                with self.assertRaisesRegex(ValueError, 'executable'):
                    require_bounded_host_tools()
            with patch('tools.check_kvm_host.trusted_path', return_value=path), \
                    patch('tools.bounded_acceptance.os.access', return_value=True):
                self.assertEqual(require_bounded_host_tools(), str(path))


class StaticConfigurationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.path, self.policy, self.profile, self.namespaces = service_fixture(self.root)

    def test_static_preflight_does_not_join_or_create_unactivated_coordinator(self):
        originals = {path: path.read_bytes() for path in (self.path, self.root / "runtime.json", self.root / "builder.json")}
        with patch("tools.template_build_service.PhysicalProfileLease") as lease, \
                patch("tools.template_physical_storage.os.fsync", side_effect=AssertionError("static fsync")):
            config = configuration(self.path, require_physical_lease=False)
        lease.assert_not_called()
        self.assertEqual(config["physical_storage_profile"], self.profile)
        self.assertEqual(config["limits"]["builder_disk_bytes"], 32 * GIB)
        self.assertFalse(Path(self.profile["coordination_file"]).exists())
        self.assertEqual(list((self.root / "builds").iterdir()), [])
        self.assertEqual(originals, {path: path.read_bytes() for path in originals})
        with self.assertRaises(FileNotFoundError):
            configuration(self.path)

    def test_default_validation_joins_only_exact_activated_profile(self):
        coordinator = Path(self.profile["coordination_file"])
        coordinator.touch(mode=0o600)
        with self.assertRaises(json.JSONDecodeError):
            configuration(self.path)
        self.assertEqual(coordinator.read_bytes(), b"")
        activate_for_test(self.profile)  # synthetic marker, never production activation
        static = configuration(self.path, require_physical_lease=False)
        active = configuration(self.path)
        self.assertEqual(static, active)
        marker = json.loads(coordinator.read_text())
        marker["profile"]["filesystem_overhead_bytes"] += 1
        coordinator.write_text(json.dumps(marker))
        with self.assertRaisesRegex(ValueError, "unactivated or changed"):
            configuration(self.path)

    def test_static_preflight_still_refuses_mismatched_profile(self):
        builder_path = self.root / "builder.json"
        builder = json.loads(builder_path.read_text())
        builder["native"]["bounded_host"]["physical_storage"]["pools"][1]["rootfs_budget_bytes"] += 1
        builder_path.write_text(json.dumps(builder))
        with self.assertRaisesRegex(ValueError, "identical explicit"):
            configuration(self.path, require_physical_lease=False)
        self.assertFalse(Path(self.profile["coordination_file"]).exists())
        self.assertEqual(list((self.root / "builds").iterdir()), [])

    def test_static_result_cannot_start_store_without_original_physical_lease(self):
        config = configuration(self.path, require_physical_lease=False)
        with self.assertRaises(FileNotFoundError):
            BuildStore(config["root"], config["limits"], binding=config["binding"],
                       bounded=config["bounded_builds"], physical_profile=config["physical_storage_profile"])
        self.assertEqual(list((self.root / "builds").iterdir()), [])

    def test_production_config_cannot_disable_lease_validation(self):
        config = json.loads(self.path.read_text())
        config["require_physical_lease"] = False
        self.path.write_text(json.dumps(config))
        with self.assertRaisesRegex(ValueError, "invalid build service config"):
            configuration(self.path)

    def test_lease_flag_is_explicitly_boolean_not_truthy(self):
        for invalid in (None, 0, 1, "false", [], {}):
            with self.subTest(value=invalid), self.assertRaisesRegex(ValueError, "lease validation flag"):
                configuration(self.root / "absent.json", require_physical_lease=invalid)


class BoundedLayoutTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        _, self.policy, self.profile, self.namespaces = service_fixture(self.root)

    def layout(self, policy=None, **overrides):
        values = {"run_root": self.root, "namespace": self.namespaces[0], "build_namespace": self.namespaces[1]}
        values.update(overrides)
        return bounded_acceptance_layout(self.policy if policy is None else policy, **values)

    def test_fixed_profile_passes_without_identity_rewrite_or_activation(self):
        original = copy.deepcopy(self.policy)
        with patch("tools.template_physical_storage.PhysicalProfileLease", side_effect=AssertionError("lease")):
            self.assertEqual(self.layout(), (self.root, self.namespaces))
        self.assertEqual(self.policy, original)
        self.assertFalse(Path(self.profile["coordination_file"]).exists())

    def test_both_namespaces_and_root_must_be_explicit(self):
        for field in ("run_root", "namespace", "build_namespace"):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "explicit run root"):
                self.layout(**{field: None})
        for invalid in ("", "R" * 16, "a" * 15, "a" * 17, 17, True):
            with self.subTest(namespace=invalid), self.assertRaisesRegex(ValueError, "namespaces"):
                self.layout(namespace=invalid)
        with self.assertRaisesRegex(ValueError, "namespaces"):
            self.layout(build_namespace=self.namespaces[0])
        with self.assertRaisesRegex(ValueError, "namespaces"):
            self.layout(build_namespace=self.namespaces[0][:6] + "f" * 10)

    def test_alias_and_noncanonical_root_are_refused(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        for invalid in ("relative", "/", str(self.root) + "/", str(self.root) + "/./new",
                        str(self.root) + "/../new", alias / "not-created"):
            with self.subTest(root=invalid), self.assertRaisesRegex(ValueError, "canonical"):
                self.layout(run_root=invalid)
        self.assertFalse((self.root / "not-created").exists())

    def test_profile_mismatch_is_refused_not_rebound(self):
        for location, field, replacement in (("pool", "namespace", "a" * 16),
                                             ("pool", "database", str(self.root / "other/controller.db")),
                                             ("pool", "node_state_dir", str(self.root / "other/state")),
                                             ("pool", "chroot_base", str(self.root / "other/jails")),
                                             ("public", "root", str(self.root / "other/builds")),
                                             ("public", "registry_root", str(self.root / "other/registry"))):
            candidate = copy.deepcopy(self.policy)
            for role in ("runtime", "builder"):
                profile = candidate[role]["bounded_host"]["physical_storage"]
                target = profile["pools"][0] if location == "pool" else profile["public_build"]
                target[field] = replacement
            original = copy.deepcopy(candidate)
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "explicit acceptance layout"):
                self.layout(candidate)
            self.assertEqual(candidate, original)

    def test_partial_or_divergent_profiles_cannot_be_called_full_acceptance(self):
        for role, field in (("runtime", "capacity"), ("builder", "capacity"),
                            ("runtime", "physical_storage"), ("builder", "physical_storage")):
            candidate = copy.deepcopy(self.policy)
            candidate[role]["maintenance" if field == "capacity" else "bounded_host"].pop(field)
            with self.subTest(role=role, field=field), self.assertRaisesRegex(ValueError, "complete bounded"):
                self.layout(candidate)
        candidate = copy.deepcopy(self.policy)
        candidate["builder"]["bounded_host"]["physical_storage"]["filesystem_overhead_bytes"] += 1
        with self.assertRaisesRegex(ValueError, "identical immutable"):
            self.layout(candidate)


class CoordinationProvisioningTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        parent = Path(temporary.name).resolve()
        fixture = parent / "fixture"
        fixture.mkdir(mode=0o700)
        _, policy, _, self.namespaces = service_fixture(fixture)
        self.root = parent / "fresh"
        self.root.mkdir(mode=0o700)
        self.policy = json.loads(json.dumps(policy).replace(str(fixture), str(self.root)))
        self.path = self.root / "physical-profile"

    def provision(self):
        provision_acceptance_coordination(self.policy, run_root=self.root,
            namespace=self.namespaces[0], build_namespace=self.namespaces[1])

    def test_empty_file_is_private_durable_and_never_activated_or_replaced(self):
        original = copy.deepcopy(self.policy)
        fsync = os.fsync
        with patch("tools.bounded_acceptance.os.fsync", wraps=fsync) as sync:
            self.provision()
        self.assertEqual(sync.call_count, 2)
        self.assertEqual(self.path.read_bytes(), b"")
        info = self.path.stat()
        self.assertEqual(info.st_mode & 0o777, 0o600)
        self.assertEqual(info.st_uid, os.geteuid())
        self.assertEqual(info.st_nlink, 1)
        self.assertEqual(self.policy, original)
        self.path.write_bytes(b"existing activation identity")
        with self.assertRaisesRegex(ValueError, "empty private"):
            self.provision()
        self.assertEqual(self.path.stat().st_ino, info.st_ino)
        self.assertEqual(self.path.read_bytes(), b"existing activation identity")

    def test_missing_identity_in_existing_deployment_is_not_recreated(self):
        (self.root / "0").mkdir(mode=0o700)
        with self.assertRaisesRegex(ValueError, "empty private"):
            self.provision()
        self.assertFalse(self.path.exists())

    def test_symlink_and_outside_file_are_not_followed(self):
        outside = self.root.parent / "canary"
        outside.write_bytes(b"canary")
        self.path.symlink_to(outside)
        with self.assertRaises(ValueError):
            self.provision()
        self.path.unlink()
        for role in ("runtime", "builder"):
            self.policy[role]["bounded_host"]["physical_storage"]["coordination_file"] = str(outside)
        with self.assertRaisesRegex(ValueError, "directly inside"):
            self.provision()
        self.assertEqual(outside.read_bytes(), b"canary")
        self.assertFalse(self.path.exists())

    def test_shared_root_and_failed_sync_do_not_allow_continuation(self):
        self.root.chmod(0o755)
        with self.assertRaisesRegex(ValueError, "empty private"):
            self.provision()
        self.assertFalse(self.path.exists())
        self.root.chmod(0o700)
        with patch("tools.bounded_acceptance.os.fsync", side_effect=OSError("sync failed")):
            with self.assertRaisesRegex(OSError, "sync failed"):
                self.provision()
        self.assertEqual(self.path.read_bytes(), b"")
        with self.assertRaisesRegex(ValueError, "empty private"):
            self.provision()


class SocketLayoutTests(unittest.TestCase):
    def test_uid_slot_bound_cannot_hide_lifetime_identifier_overflow(self):
        root = Path("/var/lib/sp-01a0e1bd/bc9161f-00")
        namespaces = ["0123456789abcdef", "123456789abcdef0"]
        with self.assertRaisesRegex(ValueError, "Firecracker socket"):
            validate_acceptance_socket_paths(root, namespaces, "/bin/firecracker")

    def test_encoded_limit_counts_multibyte_paths_on_all_platforms(self):
        namespace = "0123456789abcdef"
        tail = Path("0/p/v/firecracker") / f"vm-{namespace}-{2**64 - 1}" / "root/api.socket"
        prefix = Path("/tmp/é")
        root = Path(str(prefix) + "a" * (107 - len(os.fsencode(prefix / tail))))
        self.assertEqual(len(os.fsencode(root / tail)), 107)
        validate_acceptance_socket_paths(root, [namespace], "/bin/firecracker")
        with self.assertRaisesRegex(ValueError, "Firecracker socket"):
            validate_acceptance_socket_paths(Path(str(root) + "b"), [namespace], "/bin/firecracker",
)

    @unittest.skipUnless(sys.platform == "linux", "real 107/108-byte socket boundary requires Linux")
    def test_encoded_limit_matches_real_unix_socket_boundary(self):
        with tempfile.TemporaryDirectory(dir="/tmp") as directory:
            namespace = "0123456789abcdef"
            tail = Path("0/p/v/firecracker") / f"vm-{namespace}-{2**64 - 1}" / "root/api.socket"
            # Include multibyte characters: the kernel limit counts bytes.
            prefix = Path(directory) / "é"
            padding = 107 - len(os.fsencode(prefix / tail))
            self.assertGreaterEqual(padding, 0)
            root = Path(str(prefix) + "a" * padding)
            validate_acceptance_socket_paths(root, [namespace], "/bin/firecracker")
            path = root / tail
            path.parent.mkdir(parents=True)
            with socket.socket(socket.AF_UNIX) as listener:
                listener.bind(str(path))
            too_long = Path(str(root) + "b")
            with self.assertRaisesRegex(ValueError, "Firecracker socket"):
                validate_acceptance_socket_paths(too_long, [namespace], "/bin/firecracker")
            path = too_long / tail
            path.parent.mkdir(parents=True)
            with socket.socket(socket.AF_UNIX) as listener, self.assertRaises(OSError):
                listener.bind(str(path))


class FixtureDiskBudgetTests(unittest.TestCase):
    def test_missing_profile_cannot_infer_fixture_disk_or_cleanup_defaults(self):
        for role in ("runtime", "builder"):
            with self.subTest(role=role), self.assertRaises(ValueError):
                acceptance_pool_disk_budget(None, role, 32 * GIB)

    def test_full_profile_values_are_exact_not_inferred_from_base_or_active_limit(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            _, policy, profile, _ = service_fixture(root)
            original = copy.deepcopy(policy)
            for role in ("runtime", "builder"):
                pool = next(item for item in profile["pools"] if item["role"] == role)
                self.assertEqual(acceptance_pool_disk_budget(policy, role, 32 * GIB, max_active=99), {
                    "max_disk_bytes_global": pool["rootfs_budget_bytes"] + pool["cleanup_reserve_bytes"],
                    "max_disk_bytes_per_tenant": pool["rootfs_budget_bytes"],
                    "cleanup_reserve_bytes": pool["cleanup_reserve_bytes"]})
            self.assertEqual(policy, original)
            self.assertFalse(Path(profile["coordination_file"]).exists())

    def test_nonrepresentable_or_untyped_sizes_refuse(self):
        for invalid in (0, -1, True, False, None, "32", 1.5, 2**63):
            with self.subTest(base=invalid), self.assertRaisesRegex(ValueError, "size and count"):
                acceptance_pool_disk_budget(None, "builder", invalid)
        for field in ("max_active",):
            for invalid in (0, -1, True, None, "2", 1.5, 2**63):
                with self.subTest(field=field, value=invalid), self.assertRaisesRegex(ValueError, "size and count"):
                    acceptance_pool_disk_budget(None, "builder", GIB, **{field: invalid})
        with self.assertRaisesRegex(ValueError, "role"):
            acceptance_pool_disk_budget(None, "other", GIB)


if __name__ == "__main__":
    unittest.main()
