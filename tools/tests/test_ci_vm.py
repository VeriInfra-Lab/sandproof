import base64
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from tools import ci_vm


class CiVmTests(unittest.TestCase):
    def test_same_slot_lock_refuses_duplicates_but_same_role_second_slot_can_run(self):
        with tempfile.TemporaryDirectory() as temp, patch.object(ci_vm, 'ROOT', Path(temp)):
            with ci_vm.lock_slot('rust', 'rust'):
                self.assertTrue((Path(temp) / 'role-rust.lock').exists())
                with self.assertRaises(BlockingIOError):
                    ci_vm.lock_slot('rust', 'rust')
                with ci_vm.lock_slot('rust', 'rust-2'):
                    with self.assertRaises(BlockingIOError):
                        ci_vm.lock_slot('rust', 'rust-2')
            with ci_vm.lock_slot('rust', 'rust'):
                pass  # The original slot is reusable only after its lock closes.

    def test_role_slots_preserve_original_lock_and_reject_cross_role_or_unbounded_names(self):
        for role in ci_vm.ROLES:
            self.assertEqual(ci_vm.valid_slot(role, role), role)
            if role != 'bootstrap':
                self.assertEqual(ci_vm.valid_slot(role, role + '-2'), role + '-2')
            for slot in ('../outside', role + '-3', 'bootstrap-2', 'unknown'):
                with self.assertRaises(ValueError):
                    ci_vm.valid_slot(role, slot)
        with self.assertRaises(ValueError):
            ci_vm.valid_slot('rust', 'sdk-2')

    def test_roles_fit_current_host_with_reserve(self):
        self.assertEqual(len(ci_vm.ROLES) - 1, 5)
        self.assertLess(sum(v[0] for k, v in ci_vm.ROLES.items() if k != 'bootstrap'), 12 * 1024)

    def test_restricted_network_and_no_host_sharing(self):
        args = ci_vm.qemu_command(Path('/var/lib/sandproof-ci/runs/owned'), 'rust')
        joined = ' '.join(args)
        self.assertIn('restrict=on', joined)
        self.assertIn('guestfwd=tcp:10.0.2.100:3128-cmd:/usr/bin/socat - TCP:127.0.0.1:3129', joined)
        for forbidden in ('hostfwd', 'virtfs', 'fsdev', 'vhost', '/dev/kvm,'):
            self.assertNotIn(forbidden, joined)
        self.assertIn('readonly=on', joined)
        self.assertIn('host,-vmx,-svm', joined)

    def test_each_role_has_own_fixed_budget(self):
        for role, (memory, cpus, disk) in ci_vm.ROLES.items():
            args = ci_vm.qemu_command(Path('/private/run'), role)
            self.assertEqual(args[args.index('-m') + 1], str(memory))
            self.assertEqual(args[args.index('-smp') + 1], str(cpus))
            self.assertGreaterEqual(disk, 12)

    def test_job_config_has_no_admin_groups_or_secrets(self):
        jit = base64.b64encode(b'opaque-test-jit-configuration').decode()
        config = ci_vm.cloud_config('sdk', jit)
        user = config['users'][0]
        self.assertEqual(user['groups'], 'users')
        self.assertFalse(user['sudo'])
        files = {entry['path']: entry for entry in config['write_files']}
        self.assertEqual(files['/home/runner/.jitconfig']['permissions'], '0600')
        unit = files['/etc/systemd/system/sandproof-ci-job.service']['content']
        self.assertIn('User=runner', unit)
        self.assertIn('NoNewPrivileges=true', unit)
        self.assertIn('poweroff', unit)
        self.assertNotIn(jit, unit)
        for forbidden in ('GH_TOKEN=', 'GITHUB_APP_PRIVATE_KEY=', 'BEGIN PRIVATE KEY'):
            self.assertNotIn(forbidden, str(config))

    def test_invalid_jit_rejected(self):
        for value in ('', 'bad secret', 'x' * 131073, 'invalid-length-jit='):
            with self.assertRaises((ValueError, base64.binascii.Error)):
                ci_vm.cloud_config('title', value)

    def test_bootstrap_requires_pins(self):
        with self.assertRaises(ValueError):
            ci_vm.cloud_config('bootstrap')
        config = ci_vm.cloud_config('bootstrap', pins={'test': True})
        self.assertEqual(config['runcmd'], [['bash', '/root/ci-bootstrap.sh']])

    def test_overlay_cannot_truncate_base_partitions(self):
        base_size = ci_vm.ROLES['bootstrap'][2] * 1024 ** 3
        for role in ci_vm.ROLES:
            ci_vm.validate_disk_capacity(role, base_size)
            with self.assertRaises(ValueError):
                ci_vm.validate_disk_capacity(role, ci_vm.ROLES[role][2] * 1024 ** 3 + 1)
            with self.assertRaises(ValueError):
                ci_vm.validate_disk_capacity(role, 0)


if __name__ == '__main__':
    unittest.main()
