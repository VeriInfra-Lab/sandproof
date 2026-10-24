"""Capture deployment input components; no quota changes, Rust process or VM."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from tools import capture_acceptance as capture


def profile():
    tool=dict(path='/approved/chattr',sha256=[1]*32)
    return dict(schema_version=1,hosts=[dict(state_copy_bytes=32,memory_copy_bytes=1024**2,
        disk_copy_bytes=4096,api_timeout_ms=1000,worker_timeout_ms=2000,
        material_project=dict(project_id=63801,hard_bytes=128*1024**3,hard_inodes=4096,
            chattr=tool,lsattr=tool|dict(path='/approved/lsattr')))])


class CaptureAcceptanceTests(unittest.TestCase):
    def test_per_run_binding_keeps_approved_limits_and_uses_exact_private_parent(self):
        source=profile()
        saved=copy.deepcopy(source)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory)
            before=list(root.iterdir())
            first=capture.bind_settings(source,[root])
            second=capture.bind_settings(source,[root])
            self.assertEqual(list(root.iterdir()),before)
            self.assertEqual(first[0]['layout_parent'],str(root/'p'))
            self.assertNotEqual(first[0]['layout_binding'],second[0]['layout_binding'])
            self.assertEqual(len(first[0]['layout_binding']),32)
            for key,value in saved['hosts'][0].items():
                self.assertEqual(first[0][key],value)
        self.assertEqual(source,saved)

    def test_missing_or_replayed_layout_identity_and_invalid_bounds_are_rejected(self):
        for change in (
                {'layout_parent':'/old/layout'}, {'layout_binding':[1]*32},
                {'state_copy_bytes':0}, {'state_copy_bytes':10_000_001},
                {'memory_copy_bytes':2**63}, {'disk_copy_bytes':True},
                {'api_timeout_ms':999}, {'worker_timeout_ms':999},
                {'worker_timeout_ms':300_001}):
            value=profile();value['hosts'][0].update(change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                capture.validate_profile(value,1)
        with self.assertRaises(ValueError):
            capture.validate_profile(profile(),2)
        value=profile();value['hosts']*=2
        with self.assertRaisesRegex(ValueError,'share a material project'):
            capture.validate_profile(value,2)

    def test_complete_old_and_new_bundle_capacity_is_reserved_without_shrinking_a_vm(self):
        config=profile()['hosts'][0]
        with tempfile.TemporaryDirectory() as directory:
            disk=Path(directory)/'disk';disk.write_bytes(bytes(4096))
            native=dict(memory_mib=1,rootfs=str(disk))
            required=4*(32+1024**2+4096)
            config['material_project']['hard_bytes']=required
            self.assertEqual(capture.require_capacity(config,native,2),required)
            config['material_project']['hard_bytes']-=1
            with self.assertRaisesRegex(ValueError,'complete captures'):
                capture.require_capacity(config,native,2)
            config=profile()['hosts'][0];config['memory_copy_bytes']-=1
            with self.assertRaisesRegex(ValueError,'actual VM'):
                capture.require_capacity(config,native,2)
            config=profile()['hosts'][0];config['disk_copy_bytes']-=1
            with self.assertRaisesRegex(ValueError,'private disk'):
                capture.require_capacity(config,native,2)

    def test_observation_reads_the_original_config_and_rejects_a_foreign_pool(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);control=root/'control';control.mkdir(mode=0o700)
            path=control/'host.json'
            source=dict(native=dict(database=str(root/'db/controller.db'),node_state_dir=str(root/'state'),
                chroot_base=str(root/'p/v')),capture=dict(layout_parent=str(root/'p')))
            path.write_text(json.dumps(source));path.chmod(0o600)
            self.assertEqual(capture.original_chroot(root),root/'p/v')
            source['native']['chroot_base']=str(root/'other/v')
            path.write_text(json.dumps(source))
            with self.assertRaisesRegex(ValueError,'original native'):
                capture.original_chroot(root)
