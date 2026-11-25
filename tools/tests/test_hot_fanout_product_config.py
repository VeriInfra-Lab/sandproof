"""Config setup for the one-host, one-template public hot fan-out probe."""

from copy import deepcopy
import hashlib
from pathlib import Path
import runpy
import unittest


MODULE = runpy.run_path(str(Path(__file__).resolve().parents[2] / "tests/system/_hot_fanout_config.py"))
descriptor_digest = MODULE["descriptor_digest"]
protected_configs = MODULE["protected_configs"]


class HotFanoutProductConfigTests(unittest.TestCase):
    def setUp(self):
        approved = dict(template_id="training", revision="r1", owner=1, envd_version="0.9.0",
                        kernel={"path": "/original/vmlinux", "sha256": "a" * 64, "size_bytes": 2},
                        rootfs={"path": "/original/rootfs.ext4", "sha256": "b" * 64, "size_bytes": 3},
                        resources={"memory_mib": 2048, "cpu_count": 4, "host_memory_bytes": 3221225472,
                                   "disk_bytes": 3})
        self.receipt = {
            "binding": approved, "native_hot_fanout": {
                "template": {"descriptor_digest": list(descriptor_digest(approved)),
                             "checkpoint_id": "warm0", "max_branches": 4},
                "max_batches": 8, "max_branches_per_vm": 4}}
        self.staged = deepcopy(approved)
        self.staged["kernel"]["path"] = "/center/asset-0"
        self.staged["rootfs"]["path"] = "/center/asset-1"
        self.center = {"templates": {"templates": [self.staged]},
                       "template_registrations": [{"template_id": "training", "revision": "training-r1"}]}
        self.host = {"native": {}, "template_cache": {"root": "/native/cache"}}

    def test_hash_uses_rust_binding_field_order(self):
        # The wire representation matches the Rust struct declaration, including
        # the field order of nested ResourceShape.
        raw = (
            '{"template_id":"training","revision":"r1","owner":1,'
            f'"kernel":{{"path":"/center/asset-0","sha256":"{"a"*64}","size_bytes":2}},'
            f'"rootfs":{{"path":"/center/asset-1","sha256":"{"b"*64}","size_bytes":3}},'
            '"resources":{"cpu_count":4,"memory_mib":2048,"disk_bytes":3,'
            '"host_memory_bytes":3221225472},"envd_version":"0.9.0"}')
        self.assertEqual(descriptor_digest(self.staged), hashlib.sha256(raw.encode()).digest())

    def test_set_pin_to_host_local_cache_projection(self):
        approved = deepcopy(self.receipt)
        protected_configs(self.center, [self.host], self.receipt)
        self.assertEqual(approved, self.receipt)
        self.assertEqual(self.center["branching"], {
            "limits": {"max_batches": 8, "max_branches_per_vm": 4, "first_port": 31000},
            "template_revisions": ["training-r1"]})
        key = hashlib.sha256(b"training-r1\0" + descriptor_digest(self.staged)).hexdigest()
        local = deepcopy(self.staged)
        local["kernel"]["path"] = f"/native/cache/artifacts/{key}/kernel"
        local["rootfs"]["path"] = f"/native/cache/artifacts/{key}/rootfs.ext4"
        expected = deepcopy(approved["native_hot_fanout"])
        expected["template"]["descriptor_digest"] = list(descriptor_digest(local))
        self.assertEqual(self.host["native"]["hot_fanout"], expected)

    def test_refuse_changed_artifact_before_creating_protected_config(self):
        self.staged["rootfs"]["size_bytes"] += 1
        with self.assertRaises(ValueError):
            protected_configs(self.center, [self.host], self.receipt)
        self.assertNotIn("branching", self.center)
        self.assertNotIn("hot_fanout", self.host["native"])
