import base64
from pathlib import Path
import unittest
from tools import ci_vm


class CiVmTests(unittest.TestCase):
    def test_roles_fit_current_host_with_reserve(self):
        self.assertEqual(len(ci_vm.ROLES) - 1, 5)
        self.assertLess(sum(v[0] for k, v in ci_vm.ROLES.items() if k != 'bootstrap'), 12 * 1024)

    def test_restricted_network_and_no_host_sharing(self):
        args = ci_vm.qemu_command(Path('/var/lib/sandproof-ci/runs/owned'), 'rust')
        joined = ' '.join(args)
        self.assertIn('restrict=on', joined)
        self.assertIn('guestfwd=tcp:10.0.2.100:3128-cmd:/usr/bin/socat - TCP:127.0.0.1:3129', joined)
        for forbidden in ('hostfwd', 'virtfs', 'fsdev', 'vhost', '/home/ning', '/dev/kvm,'):
            self.assertNotIn(forbidden, joined)
        self.assertIn('readonly=on', joined)

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
        self.assertEqual(user['groups'], 'runner')
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


if __name__ == '__main__':
    unittest.main()
