"""Static supervision fixtures only: no VM, mount or production activation."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools.bounded_acceptance import acceptance_pool_disk_budget, bounded_acceptance_layout, require_bounded_host_tools
from tools.template_build_service import configuration
from tools.template_build_store import BuildStore
from tools.tests.test_template_build_config import GIB, MIB, inputs, transfer_settings
from tools.tests.test_template_physical_storage import activate_for_test


def service_fixture(root):
    """Build an explicit synthetic full profile, not a deployment default."""
    runtime, builder, limits, base = inputs()
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
        for name in ("db", "state", "jails", "cgroup"):
            (parent / name).mkdir(mode=0o700)
        settings.update(namespace=namespaces[index], database=str(parent / "db/controller.db"),
                        node_state_dir=str(parent / "state"), chroot_base=str(parent / "jails"),
                        bind=f"127.0.0.1:{3100 + index}", uid_base=1000 + index * 100,
                        credentials=[{"tenant": 1, "secret": str(index)}],
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
    runtime["template_registry"].update(root=str(registry), max_templates_global=2,
        max_artifact_bytes_global=32 * GIB, max_artifact_bytes_per_tenant=20 * GIB)
    builder["internal_build_vms"] = True
    builder["build_retry_windows"] = {"window_duration_ms": 600000, "max_anchors": 4}
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
               "public_build": {"root": str(root / "builds"), "registry_root": str(registry),
                                "workspace_bytes": limits["workspace_bytes"],
                                "registry_bytes": 32 * GIB + 2 * (65536 + 87), "max_jobs": 4,
                                "max_templates": 2, "inode_budget": 128},
               "filesystem_overhead_bytes": 4 * MIB, "filesystem_overhead_inodes": 512}
    for settings in (runtime, builder):
        settings["bounded_host"]["physical_storage"] = copy.deepcopy(profile)
    config = {"root": str(root / "builds"), "bind": "127.0.0.1:3102",
              "runtime_config": str(root / "runtime.json"), "builder_config": str(root / "builder.json"),
              "tools": {name: {"path": str(kernel), "sha256": hashlib.sha256(kernel.read_bytes()).hexdigest()}
                        for name in ("buildctl", "buildkitd", "buildkit-runc", "umoci", "envd", "busybox", "regctl")},
              "limits": limits, "dns_servers": ["1.1.1.1"], "bounded_builds": bounded}
    for path, value in ((Path(config["runtime_config"]), runtime), (Path(config["builder_config"]), builder),
                        (root / "service.json", config)):
        path.write_text(json.dumps(value))
        path.chmod(0o600)
    policy = {"runtime": {key: copy.deepcopy(runtime[key]) for key in ("bounded_host", "maintenance")},
              "builder": {key: copy.deepcopy(builder[key]) for key in ("bounded_host", "maintenance", "build_retry_windows")},
              "public": {"bounded_builds": copy.deepcopy(bounded),
                         "workspace_bytes": limits["workspace_bytes"],
                         "workspace_bytes_per_tenant": limits["workspace_bytes_per_tenant"]}}
    return root / "service.json", policy, profile, namespaces


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
        builder["bounded_host"]["physical_storage"]["pools"][1]["rootfs_budget_bytes"] += 1
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


class FixtureDiskBudgetTests(unittest.TestCase):
    def test_legacy_floor_expands_for_two_real_32_gib_builder_disks(self):
        self.assertEqual(acceptance_pool_disk_budget(None, "builder", 32 * GIB), {
            "max_disk_bytes_global": 64 * GIB + 256 * MIB,
            "max_disk_bytes_per_tenant": 64 * GIB, "cleanup_reserve_bytes": 256 * MIB})
        self.assertEqual(acceptance_pool_disk_budget(None, "runtime", GIB), {
            "max_disk_bytes_global": 24 * GIB, "max_disk_bytes_per_tenant": 24 * GIB - 256 * MIB,
            "cleanup_reserve_bytes": 256 * MIB})

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
        for field in ("max_active", "legacy_bytes", "legacy_cleanup_bytes"):
            for invalid in (0, -1, True, None, "2", 1.5, 2**63):
                with self.subTest(field=field, value=invalid), self.assertRaisesRegex(ValueError, "size and count"):
                    acceptance_pool_disk_budget(None, "builder", GIB, **{field: invalid})
        with self.assertRaisesRegex(ValueError, "overflow"):
            acceptance_pool_disk_budget(None, "builder", 2**62)
        with self.assertRaisesRegex(ValueError, "role"):
            acceptance_pool_disk_budget(None, "other", GIB)


if __name__ == "__main__":
    unittest.main()
