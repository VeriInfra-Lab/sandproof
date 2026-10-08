import base64
import json
import os
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace
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
            if role not in ci_vm.PREPARATION_ROLES:
                self.assertEqual(ci_vm.valid_slot(role, role + '-2'), role + '-2')
            for slot in ('../outside', role + '-3', 'bootstrap-2', 'unknown'):
                with self.assertRaises(ValueError):
                    ci_vm.valid_slot(role, slot)
        with self.assertRaises(ValueError):
            ci_vm.valid_slot('rust', 'sdk-2')

    def test_roles_fit_current_host_with_reserve(self):
        self.assertEqual(len(ci_vm.ROLES) - len(ci_vm.PREPARATION_ROLES), 5)
        self.assertLess(sum(v[0] for k, v in ci_vm.ROLES.items() if k not in ci_vm.PREPARATION_ROLES), 12 * 1024)

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

    def test_legacy_and_content_addressed_golden_identity(self):
        pins = {'golden_sha256': 'a' * 64, 'cloud_sha256': 'b' * 64}
        self.assertEqual(ci_vm.source_image('sdk', pins),
                         (ci_vm.ROOT / 'images/golden.qcow2', 'a' * 64))
        pins['golden_image'] = 'golden-' + 'a' * 64 + '.qcow2'
        self.assertEqual(ci_vm.source_image('preheat', pins)[0].name, pins['golden_image'])
        for invalid in ('../golden.qcow2', '/tmp/golden.qcow2', 'golden-' + 'b' * 64 + '.qcow2', None):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                ci_vm.source_image('sdk', dict(pins, golden_image=invalid))
        with self.assertRaises(ValueError):
            ci_vm.source_image('sdk', {'golden_sha256': None})

    def test_preheat_has_no_runner_or_credentials_and_root_owns_final_receipt(self):
        bundle = {'input_sha256': 'a' * 64, 'manifest': {'test': True}}
        nonce = 'c' * 64
        config = ci_vm.cloud_config('preheat', pins={}, dependency_bundle=bundle,
                                    completion_nonce=nonce)
        files = {entry['path']: entry for entry in config['write_files']}
        self.assertEqual(files['/root/ci-preheat.sh']['permissions'], '0700')
        script = files['/root/ci-preheat.sh']['content']
        self.assertIn('runuser -u runner -- python3', script)
        self.assertIn('stop-preparation-processes.py', script)
        self.assertIn('cmp /root/tools-before.json /root/tools-after.json', script)
        self.assertIn('SANDPROOF_CI_DEPENDENCIES_SEALED ' + 'a' * 64 + ' ' + nonce, script)
        self.assertFalse(any('.jit' in path or 'sandproof-ci-job.service' in path for path in files))
        inventory = files['/root/tool-inventory.py']['content']
        self.assertIn('/home/runner', inventory)
        self.assertIn('/home/runner/.cargo/registry', inventory)
        self.assertNotIn('config.toml', inventory)  # config files are included, never excluded.
        with self.assertRaises(ValueError):
            ci_vm.cloud_config('preheat', 'opaque', {}, bundle, nonce)
        with self.assertRaises(ValueError):
            ci_vm.cloud_config('sdk', base64.b64encode(b'opaque-test-jit-configuration').decode(),
                               {}, bundle, nonce)

    def test_preheated_publish_is_standalone_no_replace_and_does_not_activate(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'images').mkdir()
            directory = root / 'run'
            directory.mkdir()
            (root / 'pins.json').write_text('{"old":true}\n')
            (directory / 'work.qcow2').write_bytes(b'work')
            def convert(command, **kwargs):
                if command[1] == 'convert':
                    Path(command[-1]).write_bytes(b'new-standalone')
                    return __import__('subprocess').CompletedProcess(command, 0)
                return __import__('subprocess').CompletedProcess(command, 0,
                    stdout=json.dumps({'format': 'qcow2'}))
            with patch.object(ci_vm, 'ROOT', root), patch.object(ci_vm.subprocess, 'run', side_effect=convert):
                path = ci_vm.publish_preheated(directory, {'golden_sha256': 'b' * 64}, 'c' * 64)
                candidate = json.loads(path.read_text())
                image = root / 'images' / candidate['golden_image']
                self.assertEqual(candidate['golden_sha256'], ci_vm.digest(image))
                self.assertEqual(image.stat().st_mode & 0o777, 0o444)
                self.assertEqual((root / 'pins.json').read_text(), '{"old":true}\n')
                self.assertFalse((directory / 'prepared.qcow2').exists())
                with self.assertRaises(FileExistsError):
                    ci_vm.publish_preheated(directory, {}, 'c' * 64)
                self.assertEqual(image.read_bytes(), b'new-standalone')

    def test_preheated_stage_does_not_remove_unfamiliar_existing_file(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            stage = directory / 'prepared.qcow2'
            stage.write_bytes(b'not-owned')
            with self.assertRaises(FileExistsError):
                ci_vm.publish_preheated(directory, {}, 'a' * 64)
            self.assertEqual(stage.read_bytes(), b'not-owned')

    def test_preheated_backed_image_or_convert_failure_cannot_publish_pins(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            def convert(command, **kwargs):
                return __import__('subprocess').CompletedProcess(command, 0,
                    stdout=json.dumps({'format': 'qcow2', 'backing-filename': 'old.qcow2'}))
            with patch.object(ci_vm.subprocess, 'run', side_effect=convert):
                with self.assertRaises(ValueError):
                    ci_vm.publish_preheated(directory, {}, 'a' * 64)
            self.assertFalse((directory / 'prepared-pins.json').exists())
            self.assertFalse((directory / 'prepared.qcow2').exists())

    def preparation_fixture(self, root):
        state = root / 'supervisor'
        state.mkdir(mode=0o700)
        (state / 'state.json').write_text('{}')
        (root / 'runs').mkdir()
        original_stat, original_lstat = Path.stat, Path.lstat
        original_read_text = Path.read_text
        def owned_stat(path, *args, **kwargs):
            result = original_stat(path, *args, **kwargs)
            return SimpleNamespace(st_uid=0, st_mode=result.st_mode) if path.parent == state or path == state else result
        def owned_lstat(path, *args, **kwargs):
            result = original_lstat(path, *args, **kwargs)
            return SimpleNamespace(st_uid=0, st_mode=result.st_mode) if path.parent == state else result
        def read_text(path, *args, **kwargs):
            if path == Path('/proc/meminfo'):
                return 'MemAvailable: 16777216 kB\n'
            return original_read_text(path, *args, **kwargs)
        patches = [patch.object(ci_vm, 'ROOT', root), patch.object(ci_vm.os, 'geteuid', return_value=0),
                   patch.object(Path, 'stat', owned_stat), patch.object(Path, 'lstat', owned_lstat),
                   patch.object(Path, 'read_text', read_text),
                   patch.object(ci_vm.subprocess, 'run', return_value=__import__('subprocess').CompletedProcess([], 0, stdout='')),
                   patch.object(ci_vm.shutil, 'disk_usage', return_value=SimpleNamespace(free=100 * 1024 ** 3))]
        for item in patches:
            item.start()
            self.addCleanup(item.stop)
        return state

    def test_preparation_claims_and_releases_only_its_idle_maintenance(self):
        with tempfile.TemporaryDirectory() as temp:
            state = self.preparation_fixture(Path(temp))
            with ci_vm.preparation_guard('a' * 32):
                self.assertEqual(json.loads((state / 'paused').read_text()), {'owner': 'ci-dependencies-' + 'a' * 32})
                self.assertTrue((state / 'exclusive-host').exists())
            self.assertFalse((state / 'paused').exists())
            self.assertFalse((state / 'exclusive-host').exists())

    def test_preparation_refuses_existing_maintenance_leases_and_workers(self):
        with tempfile.TemporaryDirectory() as temp:
            state = self.preparation_fixture(Path(temp))
            marker = state / 'paused'
            marker.write_text('another-owner')
            with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                self.fail('must refuse other owner')
            self.assertEqual(marker.read_text(), 'another-owner')
            marker.unlink()
            (state / 'state.json').write_text('{"sdk":{"phase":"running"}}')
            with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                self.fail('must not stop in-flight lease')
            self.assertFalse(marker.exists())
            (state / 'state.json').write_text('{}')
            with patch.object(ci_vm.subprocess, 'run', return_value=__import__('subprocess').CompletedProcess([], 0, stdout='active worker')):
                with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                    self.fail('must not start alongside unknown worker')

    def test_preparation_retains_maintenance_if_vm_cleanup_is_unknown(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            state = self.preparation_fixture(root)
            with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                directory = root / 'runs' / ('a' * 32)
                directory.mkdir()
                (directory / 'work.qcow2').write_bytes(b'possibly-active')
            self.assertTrue((state / 'paused').exists())
            self.assertTrue((state / 'exclusive-host').exists())

    def test_preparation_refuses_insecure_state_symlink_and_insufficient_space(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            state = self.preparation_fixture(root)
            state.chmod(0o755)
            with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                self.fail('world-readable maintenance state refused')
            state.chmod(0o700)
            (state / 'admission.lock').symlink_to(root / 'outside')
            with self.assertRaises(OSError), ci_vm.preparation_guard('a' * 32):
                self.fail('lock symlink refused')
            self.assertFalse((root / 'outside').exists())
            (state / 'admission.lock').unlink()
            with patch.object(ci_vm.shutil, 'disk_usage', return_value=SimpleNamespace(free=71 * 1024 ** 3)):
                with self.assertRaises(ValueError), ci_vm.preparation_guard('a' * 32):
                    self.fail('conversion disk reserve required')
            self.assertFalse((state / 'paused').exists())


if __name__ == '__main__':
    unittest.main()
