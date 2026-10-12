import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

from tools.template_physical_storage import envelope, validate_configuration, PhysicalProfileLease


def configuration(root):
    pools, settings = [], []
    for index, role in enumerate(("runtime", "builder")):
        parent = root / role
        parent.mkdir(mode=0o700)
        for name in ("db", "node", "jails"):
            (parent / name).mkdir(mode=0o700)
        namespace = str(index) + "123456789abcdef"
        item = {"namespace": namespace, "database": str(parent / "db/controller.db"),
                "node_state_dir": str(parent / "node"), "chroot_base": str(parent / "jails"),
                "kernel": str(root / "kernel"), "firecracker": str(root / "firecracker"),
                "max_disk_bytes_global": 1200, "cleanup_reserve_bytes": 200,
                "bounded_host": {"maintenance": {"max_host_slots": 2}},
                "maintenance": {"capacity": {"max_online_storage_bytes": 100}},
                "templates": {"templates": [{"kernel": {"size_bytes": 100}}]}}
        settings.append(item)
        pools.append({"role": role, "namespace": namespace,
                      **{field: item[field] for field in ("database", "node_state_dir", "chroot_base")},
                      "rootfs_budget_bytes": 1000, "max_host_slots": 2, "kernel_copy_bytes": 200,
                      "runtime_copy_bytes": 100, "cleanup_reserve_bytes": 200, "host_metadata_bytes": 100,
                      "host_metadata_inodes": 20, "jail_inodes_per_slot": 12})
    for name in ("public", "registry"):
        (root / name).mkdir(mode=0o700)
    for name in ("inbox", "artifacts"):
        (root / "registry" / name).mkdir(mode=0o700)
    public = {"root": str(root / "public"), "bounded_builds": {"database_bytes": 500, "wal_bytes": 500, "recovery_reserve_bytes": 500},
              "limits": {"workspace_bytes": 2000, "max_jobs": 2},
              "registry_owner_uid": os.geteuid(),
              "registry": {"root": str(root / "registry"), "max_templates_global": 1,
                           "max_artifact_bytes_global": 1000, "max_artifact_bytes_per_tenant": 200}}
    path = root / "physical-profile"
    path.touch(mode=0o600)
    profile = {"version": 1, "coordination_file": str(path), "pools": pools,
               "public_build": {"workspace_owner_uid": os.geteuid(), "registry_owner_uid": os.geteuid(),
                                "root": public["root"], "registry_root": str(root / "registry"),
                                "workspace_bytes": 2000, "registry_bytes": 1000 + 65536 + 87, "max_jobs": 2, "max_templates": 1, "inode_budget": 100},
               "host_template_caches": [],
               "service_data": [],
               "filesystem_overhead_bytes": 2 * 1024 * 1024 + 65536, "filesystem_overhead_inodes": 512}
    for item in settings:
        item["bounded_host"]["physical_storage"] = copy.deepcopy(profile)
    return settings[0], settings[1], public, profile


def activate_for_test(profile):
    path = Path(profile["coordination_file"])
    info = path.stat()
    path.write_text(json.dumps({"version": 1, "device": info.st_dev, "inode": info.st_ino, "profile": profile}))


class PhysicalProfileTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name).resolve(strict=True)
        self.runtime, self.builder, self.public, self.profile = configuration(self.root)

    def tearDown(self):
        self.directory.cleanup()

    def validate(self):
        return validate_configuration(self.runtime, self.builder, self.public, file_size=lambda _: 100)

    def test_absent_null_or_partial_profiles_always_refuse(self):
        for missing in ('absent', 'null', 'one_pool', 'capacity'):
            runtime, builder = copy.deepcopy((self.runtime, self.builder))
            if missing in ('absent', 'null'):
                for pool in (runtime, builder):
                    pool['bounded_host'].pop('physical_storage')
                    pool['maintenance'].pop('capacity')
                    if missing == 'null':
                        pool['bounded_host']['physical_storage'] = None
                        pool['maintenance']['capacity'] = None
            elif missing == 'one_pool':
                builder['bounded_host'].pop('physical_storage')
            else:
                builder['maintenance'].pop('capacity')
            with self.subTest(missing=missing), self.assertRaisesRegex(ValueError, 'identical explicit'):
                validate_configuration(runtime, builder, self.public, file_size=lambda _: 100)
        self.assertEqual((self.root / 'physical-profile').read_bytes(), b'')

    def test_shared_peak_includes_old_sparse_copies_registry_and_workspace(self):
        _, peak, inodes = self.validate()
        self.assertEqual(peak, 2 * 1024 * 1024 + 65536 + 2 * (1000 + 2 * (200 + 100) + 200) + 2000 + 1000 + 65536 + 87)
        self.assertEqual(inodes, 512)
        activate_for_test(self.profile)
        lease = PhysicalProfileLease(self.profile)
        # Rootfs allocation is not inferred from current sparse block count.
        # Activation reserves 2*peak, so full self-warmup leaves >=peak free.
        for allocated in (0, 1, peak // 2, peak):
            self.assertTrue(lease.admit(2 * peak - allocated))
        self.assertFalse(lease.admit(peak - 1))
        lease.close()

    def test_cache_copies_unknown_downloads_and_metadata_are_charged_without_sparse_credit(self):
        baseline, _ = envelope(self.profile)
        cache = dict(namespace=self.runtime['namespace'], root=str(self.root / 'cache'), max_entries=2, max_downloads=1,
            max_entry_bytes=1000, artifact_bytes=2000, temporary_bytes=1000,
            metadata_bytes=4096 + 2 * 2 * (64 * 1024 + 4096), inode_budget=19)
        self.profile['host_template_caches'] = [cache]
        peak, _ = envelope(self.profile)
        self.assertEqual(peak - baseline, sum(cache[name] for name in
            ('artifact_bytes', 'temporary_bytes', 'metadata_bytes')))
        for name, value in (('temporary_bytes', 999), ('metadata_bytes', cache['metadata_bytes'] - 1),
                            ('inode_budget', 18), ('max_downloads', 3),
                            ('max_entry_bytes', 2001), ('artifact_bytes', True)):
            changed = copy.deepcopy(self.profile)
            changed['host_template_caches'][0][name] = value
            with self.subTest(field=name), self.assertRaises(ValueError):
                envelope(changed)
        changed = copy.deepcopy(self.profile)
        changed['host_template_caches'][0]['root'] = self.profile['public_build']['registry_root']
        with self.assertRaisesRegex(ValueError, 'overlap'):
            envelope(changed)

    def test_cache_inode_and_byte_overflow_cannot_hide_in_original_overhead(self):
        self.profile['host_template_caches'] = [dict(namespace=self.runtime['namespace'], root=str(self.root / 'cache'),
            max_entries=2, max_downloads=1, max_entry_bytes=1, artifact_bytes=1,
            temporary_bytes=1, metadata_bytes=4096 + 2 * 2 * (64 * 1024 + 4096), inode_budget=512)]
        with self.assertRaisesRegex(ValueError, 'inode provision'):
            envelope(self.profile)
        self.profile['filesystem_overhead_inodes'] = 1024
        self.profile['host_template_caches'][0]['artifact_bytes'] = 2**63 - 1
        with self.assertRaisesRegex(ValueError, 'overflow'):
            envelope(self.profile)

    def test_each_pool_has_a_distinct_finite_cache_and_all_copies_are_charged(self):
        baseline, _ = envelope(self.profile)
        caches = [dict(namespace=settings['namespace'], root=str(self.root / ('cache-' + role)),
            max_entries=1, max_downloads=1, max_entry_bytes=1000, artifact_bytes=1000,
            temporary_bytes=1000, metadata_bytes=4096 + 2 * (64 * 1024 + 4096), inode_budget=14)
            for role, settings in (('runtime', self.runtime), ('builder', self.builder))]
        self.profile['host_template_caches'] = caches
        peak, _ = envelope(self.profile)
        self.assertEqual(peak - baseline, sum(c[name] for c in caches for name in
            ('artifact_bytes', 'temporary_bytes', 'metadata_bytes')))
        for invalid in (None, caches * 2, [caches[0], caches[0]],
                        [dict(caches[0], namespace='f' * 16)],
                        [caches[0], dict(caches[1], root=caches[0]['root'])]):
            candidate = dict(self.profile, host_template_caches=invalid)
            with self.subTest(caches=invalid), self.assertRaises(ValueError):
                envelope(candidate)
        old = dict(self.profile)
        old['host_template_cache'] = old.pop('host_template_caches')
        with self.assertRaisesRegex(ValueError, 'fixed physical profile'):
            envelope(old)

    def test_original_private_cache_lock_cannot_be_missing_exposed_or_aliased(self):
        cache_root = self.root / 'cache'
        cache_root.mkdir(mode=0o700)
        self.profile['host_template_caches'] = [dict(namespace=self.runtime['namespace'],
            root=str(cache_root), max_entries=1, max_downloads=1, max_entry_bytes=1000,
            artifact_bytes=1000, temporary_bytes=1000,
            metadata_bytes=4096 + 2 * (64 * 1024 + 4096), inode_budget=14)]
        activate_for_test(self.profile)
        lock = cache_root / '.cache.lock'
        with self.assertRaises(FileNotFoundError):
            PhysicalProfileLease(self.profile)
        lock.touch(mode=0o600)
        lease = PhysicalProfileLease(self.profile)
        lock.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'cache lock'):
            lease.validate()
        lock.chmod(0o600)
        lock.write_bytes(b'not an empty lock')
        with self.assertRaisesRegex(ValueError, 'cache lock'):
            lease.validate()
        lock.write_bytes(b'')
        os.link(lock, cache_root / 'extra-link')
        with self.assertRaisesRegex(ValueError, 'cache lock'):
            lease.validate()
        lease.close()

    def test_public_owner_is_explicit_and_not_a_general_private_path_exception(self):
        for name in ('workspace_owner_uid','registry_owner_uid'):
            for owner in (None, True, -1, 2**32):
                changed = copy.deepcopy(self.profile)
                changed['public_build'][name] = owner
                with self.subTest(field=name,owner=owner), self.assertRaisesRegex(ValueError, 'owner'):
                    envelope(changed)
        # A foreign owner is allowed only for the exact public roots. It does
        # not permit the coordination file or native pool to change owner.
        self.profile['public_build']['registry_owner_uid'] = os.geteuid() + 1
        activate_for_test(self.profile)
        with self.assertRaisesRegex(ValueError, 'private'):
            PhysicalProfileLease(self.profile)

    def test_service_data_has_explicit_roots_owners_and_separate_peak_not_hidden_overhead(self):
        baseline, _ = envelope(self.profile)
        for name in ('center-material','redis-data'):
            (self.root/name).mkdir(mode=0o700)
        services = [dict(root=str(self.root/name),owner_uid=os.geteuid(),
            peak_bytes=1024*1024,inode_budget=8) for name in ('center-material','redis-data')]
        self.profile['service_data'] = services
        self.assertEqual(envelope(self.profile)[0]-baseline,2*1024*1024)
        self.assertEqual(self.profile['filesystem_overhead_bytes'],2*1024*1024+65536)
        activate_for_test(self.profile)
        lease = PhysicalProfileLease(self.profile)
        lease.close()
        for invalid in (None,services*5,[services[0],services[0]],
                        [dict(services[0],owner_uid=True)], [dict(services[0],peak_bytes=0)],
                        [dict(services[0],root=self.profile['public_build']['registry_root'])],
                        [dict(services[0],root=str(self.root/'runtime'))],
                        [dict(services[0],root=str(self.root/'center-material/sub')) , services[0]],
                        [dict(services[0],peak_bytes=2**63-1)]):
            with self.subTest(services=invalid),self.assertRaises(ValueError):
                envelope(dict(self.profile,service_data=invalid))
        services[0]['owner_uid'] += 1
        activate_for_test(self.profile)
        with self.assertRaisesRegex(ValueError,'private'):
            PhysicalProfileLease(self.profile)

    def test_peer_mismatch_rollback_roles_and_copy_bounds_refuse(self):
        self.builder["max_disk_bytes_global"] += 1
        with self.assertRaisesRegex(ValueError, "pool budget"):
            self.validate()
        self.builder["max_disk_bytes_global"] -= 1
        self.builder["bounded_host"].pop("physical_storage")
        with self.assertRaisesRegex(ValueError, "identical explicit"):
            self.validate()
        self.builder["bounded_host"]["physical_storage"] = copy.deepcopy(self.profile)
        self.profile["pools"][1]["role"] = "runtime"
        with self.assertRaisesRegex(ValueError, "roles"):
            envelope(self.profile)
        self.profile["pools"][1]["role"] = "builder"
        for settings in (self.runtime, self.builder):
            settings["bounded_host"]["physical_storage"]["pools"][0]["kernel_copy_bytes"] = 199
        with self.assertRaisesRegex(ValueError, "copy maximum"):
            self.validate()

    def test_unactivated_replaced_or_altered_marker_refuses(self):
        with self.assertRaises((ValueError, json.JSONDecodeError)):
            PhysicalProfileLease(self.profile)
        activate_for_test(self.profile)
        lease = PhysicalProfileLease(self.profile)
        path = Path(self.profile["coordination_file"])
        contents = path.read_bytes()
        path.rename(self.root / "previous")
        path.write_bytes(contents)
        path.chmod(0o600)
        with self.assertRaisesRegex(ValueError, "identity changed"):
            lease.admit(envelope(self.profile)[0])
        lease.close()
        with self.assertRaisesRegex(ValueError, "unactivated or changed"):
            PhysicalProfileLease(self.profile)

    def test_known_inode_universe_and_rounding_cannot_be_omitted(self):
        self.profile["filesystem_overhead_inodes"] = 1
        with self.assertRaisesRegex(ValueError, "inode provision"):
            envelope(self.profile)
        self.profile["filesystem_overhead_inodes"] = 512
        self.profile["filesystem_overhead_bytes"] = 1
        activate_for_test(self.profile)
        with self.assertRaisesRegex(ValueError, "rounding"):
            PhysicalProfileLease(self.profile)

    def test_public_journal_cannot_initialize_outside_workspace(self):
        self.public["bounded_builds"]["database_bytes"] = self.public["limits"]["workspace_bytes"]
        with self.assertRaisesRegex(ValueError, "complete public journal"):
            self.validate()
        self.assertEqual(list((self.root / "public").iterdir()), [])

    def test_noncanonical_path_spellings_are_rejected(self):
        for spelling in ("/root/./physical", "/root//physical", "/root/physical/"):
            candidate = copy.deepcopy(self.profile)
            candidate["coordination_file"] = spelling
            with self.assertRaisesRegex(ValueError, "canonical"):
                envelope(candidate)

    def test_direct_store_rejects_journal_and_budget_mismatch_before_mutation(self):
        from tools.template_build_store import BuildStore, MIB
        from tools.tests.test_template_build_service import limits
        bounded = {"namespace": 17, "windows": {"max_open_windows": 4, "max_duration_ms": 600000},
                   "retention_ms": 1000, "max_request_bytes": 2048, "max_result_bytes": 8192,
                   "max_record_bytes": 65536, "database_bytes": 8 * MIB, "wal_bytes": 17 * MIB,
                   "recovery_reserve_bytes": 8 * MIB,
                   "helper": {"path": "/explicit-test-helper", "sha256": "a" * 64, "owner_uid": os.geteuid(),
                              "max_input_bytes": 65536, "max_output_bytes": 65536, "timeout_seconds": 5}}
        values = limits()
        values.update(workspace_bytes=2000, max_jobs=2)
        with self.assertRaisesRegex(ValueError, "complete public journal"):
            BuildStore(self.root / "public", values, bounded=bounded, physical_profile=self.profile)
        self.assertEqual(list((self.root / "public").iterdir()), [])
        values["workspace_bytes"] = 40 * MIB
        with self.assertRaisesRegex(ValueError, "exact bounded build root and budgets"):
            BuildStore(self.root / "public", values, bounded=bounded, physical_profile=self.profile)
        self.assertEqual(list((self.root / "public").iterdir()), [])

    def test_uncertain_matching_marker_durability_fails_before_build_writes(self):
        from unittest.mock import patch
        activate_for_test(self.profile)
        with patch("tools.template_physical_storage.os.fsync", side_effect=OSError("injected sync failure")):
            with self.assertRaisesRegex(OSError, "sync failure"):
                PhysicalProfileLease(self.profile)
        self.assertEqual(list((self.root / "public").iterdir()), [])
        lease = PhysicalProfileLease(self.profile)
        lease.close()

    def test_null_pool_options_refuse_before_physical_writes(self):
        for settings in (self.runtime, self.builder):
            settings["bounded_host"] = None
            settings["maintenance"] = None
        with self.assertRaisesRegex(ValueError, "identical explicit"):
            self.validate()
        self.assertEqual(Path(self.profile["coordination_file"]).read_bytes(), b"")
        self.runtime["maintenance"] = ["invalid"]
        with self.assertRaisesRegex(ValueError, "optional physical"):
            self.validate()

    def test_missing_capacity_and_profiles_refuse_without_activation(self):
        for settings in (self.runtime, self.builder):
            settings.pop("maintenance")
            settings["bounded_host"].pop("physical_storage")
        with self.assertRaisesRegex(ValueError, "identical explicit"):
            self.validate()
        self.assertEqual(Path(self.profile["coordination_file"]).read_bytes(), b"")

    def test_overflow_and_public_untracked_history_refuse(self):
        self.profile["pools"][0]["max_host_slots"] = 2**63 - 1
        with self.assertRaisesRegex(ValueError, "overflow"):
            envelope(self.profile)
        self.public["limits"]["max_jobs"] = 10000
        with self.assertRaisesRegex(ValueError, "workspace/registry"):
            self.validate()


if __name__ == "__main__":
    unittest.main()
