"""Private image directories and actual shell control flow; no VM or mounts.

The service/BusyBox programs are controlled fixtures. They verify installation
and one-shot invocation, not real daemon validation, PID persistence or KVM.
"""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from tools import guest_artifacts as artifacts, template_guest as guest
from tools.system_acceptance import write_once
from tools.template_stage import stage, stage_downloaded, file_hash
from tools.template_worker import Commands, install_platform, check_tools
from tools.tests.test_guest_artifacts import ROOT, manifest


class GuestFixture(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name).resolve()
        self.release = self.work / 'release'
        self.release.mkdir(mode=0o700)
        self.manifest = manifest(self.release)
        self.service_log = self.work / 'service.jsonl'
        service = self.release / artifacts.BINARIES['service']
        service.chmod(0o700)
        service.write_text(f'#!{sys.executable}\n' + f'LOG = {str(self.service_log)!r}\n' + '''
import json, os, sys
from pathlib import Path
mode = sys.argv[1]
assert sys.argv[2] == '--config' and len(sys.argv) == 4
config_path = Path(sys.argv[3])
config = json.loads(config_path.read_text())
with open(LOG, 'a') as out: out.write(json.dumps(sys.argv[1:]) + '\\n')
state = config_path.parent / 'state' / 'pid.bin'
if mode == 'check':
    sys.exit(0 if config['runtime']['worker_threads'] > 0 else 41)
if mode == 'initialize':
    if os.environ.get('GUEST_INIT_FAILURE') == 'before': sys.exit(42)
    with state.open('xb') as out: out.write(b'original VM identity')
    if os.environ.get('GUEST_INIT_FAILURE') == 'after': sys.exit(43)
elif mode == 'serve':
    if not state.exists(): sys.exit(44)
    print('rust_started')
else: raise AssertionError(mode)
''')
        service.chmod(0o500)
        self.manifest['artifacts']['service'] = artifacts.fingerprint(service, artifacts.BINARY_LIMIT)
        write_once(self.release / 'manifest.json', self.manifest)
        self.config = json.loads((ROOT / 'sandbox-guest-service/src/daemon/tests/configuration.json').read_bytes())
        self.config['listen'] = '0.0.0.0:49983'
        self.config['paths'].update(worker=guest.PLATFORM + '/' + artifacts.BINARIES['worker'],
                                    pid_directory=guest.PLATFORM + '/state',
                                    gate_directory='/run/sandproof-guest/gates')
        self.config['worker'] = artifacts.worker_configuration(self.manifest)
        self.config_path = self.work / 'configuration.json'
        self.profile_path = self.work / 'profile.json'
        self.save_profile()

    def save_profile(self):
        for path in (self.config_path, self.profile_path):
            path.unlink(missing_ok=True)
        write_once(self.config_path, self.config)
        self.profile = dict(release=str(self.release), manifest_sha256=file_hash(self.release / 'manifest.json'),
                            configuration=str(self.config_path), configuration_sha256=file_hash(self.config_path))
        write_once(self.profile_path, self.profile)

    def service_calls(self):
        return [json.loads(v)[0] for v in self.service_log.read_text().splitlines()] if self.service_log.exists() else []


class GuestInstallTests(GuestFixture):
    def setUp(self):
        super().setUp()
        self.root = self.work / 'root'
        for name in ('etc', 'sbin', 'run'):
            (self.root / name).mkdir(parents=True)
        (self.root / 'etc/os-release').write_text('ID=debian\n')
        (self.root / 'etc/passwd').write_text('root:x:0:0:root:/root:/bin/sh\n')
        self.platform = self.root / 'opt/sandproof'
        self.busybox = self.work / 'busybox'
        self.busybox.write_text(f'#!{sys.executable}\n' + '''
import json, os, sys
from pathlib import Path
root = Path(os.environ['BOOT_TEST_ROOT'])
args = sys.argv[1:]
logical = [a.replace(str(root), '') for a in args]
with (root.parent / 'applets.jsonl').open('a') as out: out.write(json.dumps(logical) + '\\n')
if args[0] == os.environ.get('BOOT_TEST_FAILURE'): sys.exit(73)
if args[0] == 'grep': sys.exit(0)
if args[0] == 'rm': (root / 'run/sandproof-boot-ready').unlink(missing_ok=True)
elif args[0] == 'mount' and logical[-1] == '/run':
    old = root / ('old-run-' + str(len(list(root.iterdir()))))
    (root / 'run').rename(old)
    (root / 'run').mkdir()
    (root.parent / 'run-is-tmpfs').touch()
elif args[0] == 'mkdir' and '-m' in args:
    assert args[1:3] == ['-m', '700']
    target = Path(args[-1])
    assert target.is_relative_to(root / 'run')
    target.mkdir(mode=0o700)
elif args[0] == 'mv':
    assert all(Path(a).is_relative_to(root / 'opt/sandproof') for a in args[1:])
    os.rename(args[1], args[2])
elif args[0] == 'stat': print('1021994' if (root.parent / 'run-is-tmpfs').exists() else 'ef53')
elif args[0] == 'touch': (root / 'run/sandproof-boot-ready').touch()
elif args[0] not in ('mount', 'chmod', 'mkdir', 'ln', 'ip', 'sync'): raise AssertionError(args)
''')
        self.busybox.chmod(0o500)

    def install(self):
        self.prepared = guest.load_profile(self.profile_path)
        self.log = io.BytesIO()
        install_platform(self.root, self.busybox, self.busybox, rust_guest=self.prepared,
                         commands=Commands(time.monotonic() + 10, self.log, 65536))

    def relocate_scripts(self):
        # Relocate only private fixture paths; all mount/network operations go
        # to the recording applet above, never the host's mount/ip utilities.
        for name in ('boot', 'agent'):
            path = self.platform / name
            path.write_text(path.read_text().replace('/opt/sandproof', str(self.platform))
                            .replace('/run', str(self.root / 'run')))

    def run_script(self, name, *, failure='', initialize=''):
        return subprocess.run(['/bin/sh', str(self.platform / name)], capture_output=True, text=True, timeout=10,
                              env={**os.environ, 'BOOT_TEST_ROOT': str(self.root),
                                   'BOOT_TEST_FAILURE': failure, 'GUEST_INIT_FAILURE': initialize})

    def test_copies_pinned_files_without_source_archive_and_runs_bounded_check(self):
        self.install()
        self.assertEqual(self.service_calls(), ['check'])
        for role, name in artifacts.BINARIES.items():
            self.assertEqual(artifacts.fingerprint(self.platform / name, artifacts.BINARY_LIMIT, executable=True),
                             self.prepared[1]['artifacts'][role])
        self.assertEqual(file_hash(self.platform / guest.CONFIGURATION), self.profile['configuration_sha256'])
        self.assertEqual(file_hash(self.platform / 'guest-manifest.json'), self.profile['manifest_sha256'])
        self.assertEqual((self.platform / 'state').stat().st_mode & 0o777, 0o700)
        self.assertEqual((self.platform / 'new-vm').stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.platform / guest.CONFIGURATION).stat().st_mode & 0o777, 0o400)
        self.assertFalse((self.platform / 'envd').exists())
        self.assertFalse((self.platform / 'source.tar').exists())
        self.assertFalse((self.platform / 'state/pid.bin').exists())
        self.assertIn('serve --config', (self.platform / 'agent').read_text())
        with self.assertRaisesRegex(ValueError, 'reserved'):
            install_platform(self.root, self.busybox, self.busybox, rust_guest=self.prepared)

    def test_initialize_once_across_respawn_reboot_and_missing_state(self):
        self.install()
        self.relocate_scripts()
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        boot = self.run_script('boot')
        self.assertEqual(boot.returncode, 0, boot.stderr)
        for _ in range(2):
            self.assertIn('rust_started', self.run_script('agent').stdout)
        self.assertFalse((self.platform / 'new-vm').exists())
        self.assertTrue((self.platform / 'initialization-attempted').is_file())
        self.assertEqual(self.run_script('boot').returncode, 0)
        self.assertIn('rust_started', self.run_script('agent').stdout)
        (self.platform / 'state/pid.bin').unlink()
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.run_script('boot').returncode, 0)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.service_calls().count('initialize'), 1)
        self.assertFalse((self.platform / 'state/pid.bin').exists())

    def test_consumed_permission_survives_unknown_initialize_and_never_replays(self):
        self.install()
        self.relocate_scripts()
        boot = self.run_script('boot', initialize='after')
        self.assertEqual(boot.returncode, 43, boot.stderr)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertFalse((self.platform / 'new-vm').exists())
        self.assertEqual(self.run_script('boot').returncode, 0)
        self.assertIn('rust_started', self.run_script('agent').stdout)
        self.assertEqual(self.service_calls().count('initialize'), 1)

    def test_failure_before_pid_creation_cannot_reinitialize_on_reboot(self):
        self.install()
        self.relocate_scripts()
        self.assertEqual(self.run_script('boot', initialize='before').returncode, 42)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.run_script('boot').returncode, 0)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.service_calls().count('initialize'), 1)

    def test_marker_rename_or_sync_failure_cannot_start_service(self):
        self.install()
        self.relocate_scripts()
        self.assertEqual(self.run_script('boot', failure='mv').returncode, 73)
        self.assertTrue((self.platform / 'new-vm').exists())
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.run_script('boot', failure='sync').returncode, 73)
        self.assertFalse((self.platform / 'new-vm').exists())
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(self.service_calls(), ['check'])
        self.assertEqual(self.run_script('boot').returncode, 0)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertNotIn('initialize', self.service_calls())

    def test_installed_state_is_not_created_when_actual_check_fails(self):
        self.config['runtime']['worker_threads'] = 0
        self.save_profile()
        with self.assertRaisesRegex(RuntimeError, 'tool failed'): self.install()
        self.assertEqual(self.service_calls(), ['check'])
        self.assertFalse((self.platform / 'state').exists())
        self.assertFalse((self.platform / 'new-vm').exists())
        self.assertFalse((self.platform / 'boot').exists())

    def test_invalid_or_conflicting_new_vm_marker_never_initializes(self):
        self.install()
        self.relocate_scripts()
        marker = self.platform / 'new-vm'
        marker.unlink()
        target = self.work / 'untouched'
        target.write_bytes(b'original')
        marker.symlink_to(target)
        self.assertNotEqual(self.run_script('boot').returncode, 0)
        self.assertNotEqual(self.run_script('agent').returncode, 0)
        self.assertEqual(target.read_bytes(), b'original')
        marker.unlink()
        marker.write_bytes(b'new')
        attempted = self.platform / 'initialization-attempted'
        attempted.write_bytes(b'previous attempt')
        self.assertNotEqual(self.run_script('boot').returncode, 0)
        self.assertEqual(attempted.read_bytes(), b'previous attempt')
        self.assertEqual(self.service_calls(), ['check'])

    def test_expired_original_command_deadline_never_executes_check_or_provisions(self):
        prepared = guest.load_profile(self.profile_path)
        with self.assertRaises(TimeoutError):
            install_platform(self.root, self.busybox, self.busybox, rust_guest=prepared,
                             commands=Commands(time.monotonic() - 1, io.BytesIO(), 65536))
        self.assertEqual(self.service_calls(), [])
        self.assertFalse((self.platform / 'new-vm').exists())

    def test_stale_payload_or_writable_ancestor_refuses_before_check(self):
        prepared = guest.load_profile(self.profile_path)
        path = self.release / artifacts.BINARIES['worker']
        path.chmod(0o700)
        path.write_bytes(b'x' * path.stat().st_size)
        with self.assertRaisesRegex(ValueError, 'fingerprint'):
            install_platform(self.root, self.busybox, self.busybox, rust_guest=prepared,
                             commands=Commands(time.monotonic() + 10, io.BytesIO(), 65536))
        self.assertEqual(self.service_calls(), [])
        shutil.rmtree(self.platform)  # exact failed private fixture only
        (self.root / 'opt').chmod(0o777)
        with self.assertRaisesRegex(ValueError, 'ancestor'):
            install_platform(self.root, self.busybox, self.busybox, rust_guest=prepared,
                             commands=Commands(time.monotonic() + 10, io.BytesIO(), 65536))
        self.assertEqual(self.service_calls(), [])


class GuestProfileTests(GuestFixture):
    def test_layout_unknown_fields_wrong_pin_and_flat_module_imports(self):
        for field, value in [('listen', '127.0.0.1:49983'), ('worker', '/usr/bin/other'),
                             ('pid_directory', '/run/state'), ('gate_directory', '/tmp/gates'),
                             ('owner_uid', False)]:
            original = copy.deepcopy(self.config)
            if field == 'listen': self.config[field] = value
            else: self.config['paths'][field] = value
            self.save_profile()
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, 'layout'):
                guest.load_profile(self.profile_path)
            self.config = original
        self.save_profile()
        self.profile['extra'] = True
        self.profile_path.write_text(json.dumps(self.profile))
        with self.assertRaisesRegex(ValueError, 'exactly'): guest.load_profile(self.profile_path)
        self.save_profile()
        self.profile['configuration_sha256'] = '0'*64
        self.profile_path.write_text(json.dumps(self.profile))
        with self.assertRaisesRegex(ValueError, 'configuration differs'): guest.load_profile(self.profile_path)
        # The exact flat helper layout used by the original disposable worker.
        modules = self.work / 'modules'
        modules.mkdir()
        for name in ('template_guest', 'guest_artifacts', 'build_base_guest', 'build_ci_guest',
                     'system_acceptance', 'check_kvm_host'):
            shutil.copyfile(ROOT / 'tools' / (name + '.py'), modules / (name + '.py'))
        result = subprocess.run([sys.executable, '-I', '-c',
                                 'import sys; sys.path.insert(0, sys.argv[1]); import template_guest', str(modules)],
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rust_profile_requires_extra_actual_busybox_applets(self):
        profile = {name: {'path': str(self.release / artifacts.BINARIES['worker']),
                          'sha256': self.manifest['artifacts']['worker']['sha256']}
                   for name in ('buildctl', 'buildkitd', 'buildkit-runc', 'umoci', 'envd', 'busybox')}
        applets = b'sh\ninit\nmount\ngrep\nmkdir\nchmod\nln\nip\ntouch\nreboot\numount\nrm\nstat\n'
        with patch('tools.template_worker.subprocess.check_output', side_effect=[applets, b'ef53\n']):
            check_tools(profile)
        del profile['envd']
        with patch('tools.template_worker.subprocess.check_output', return_value=applets):
            with self.assertRaisesRegex(ValueError, 'bootstrap applets'): check_tools(profile, rust_guest=True)
        with patch('tools.template_worker.subprocess.check_output', side_effect=[applets + b'mv\nsync\n', b'ef53\n']):
            check_tools(profile, rust_guest=True)


class GuestStagingTests(GuestFixture):
    """Original opaque staging; no ext4/extent assertions or Linux simulation."""
    def setUp(self):
        super().setUp()
        self.registry = self.work / 'registry'
        for path in (self.registry, self.registry / 'inbox', self.registry / 'artifacts'):
            path.mkdir(mode=0o700)
        self.kernel, self.disk = self.work / 'kernel', self.work / 'disk'
        self.kernel.write_bytes(b'kernel')
        self.disk.write_bytes(b'opaque candidate bytes')
        self.identity = guest.load_profile(self.profile_path)[1]
        self.status = dict(state='candidate', ready=False, startup_applied=False,
                           disk_bytes=self.disk.stat().st_size, min_free_bytes=1,
                           image_digest='sha256:' + 'a'*64, rootfs_sha256=file_hash(self.disk),
                           runtime_defaults=dict(env_vars={}, user='root', workdir='/', argv=[]), guest=self.identity)

    def stage(self, profile=True):
        return stage(self.registry, self.kernel, self.disk, self.status, 1, 1, 256, 384,
                     candidate_id='a'*32, rust_guest_profile=self.profile_path if profile else None)

    def test_protected_identity_is_required_persisted_and_checked_on_resume(self):
        with self.assertRaisesRegex(ValueError, 'protected'): self.stage(False)
        self.assertEqual(list((self.registry / 'inbox').iterdir()), [])
        candidate = self.stage()
        path = self.registry / 'inbox' / candidate / 'manifest.json'
        saved = json.loads(path.read_bytes())
        self.assertEqual(saved['binding']['guest'], self.identity)
        self.assertEqual(self.stage(), candidate)
        self.status['guest'] = dict(self.identity, source_commit='b'*40)
        with self.assertRaisesRegex(ValueError, 'protected'): self.stage()
        self.status['guest'] = self.identity
        saved['binding']['guest']['configuration']['sha256'] = 'e'*64
        path.chmod(0o600)
        path.write_text(json.dumps(saved))
        with self.assertRaisesRegex(ValueError, 'identity conflict'): self.stage()

    def test_download_handoff_resume_preserves_identity_after_source_name_disappears(self):
        directory = self.work / ('b'*32)
        directory.mkdir(mode=0o700)
        disk = directory / 'candidate.ext4'
        self.disk.rename(disk)
        disk.chmod(0o600)
        for _ in range(2):
            candidate = stage_downloaded(self.registry, self.kernel, disk, self.status, 1, 1, 256, 384,
                                         candidate_id='b'*32, rust_guest_profile=self.profile_path)
            self.assertFalse(disk.exists())
            saved = json.loads((self.registry / 'inbox' / candidate / 'manifest.json').read_bytes())
            self.assertEqual(saved['binding']['guest'], self.identity)
        del self.status['guest']
        with self.assertRaisesRegex(ValueError, 'protected'):
            stage_downloaded(self.registry, self.kernel, disk, self.status, 1, 1, 256, 384,
                             candidate_id='b'*32, rust_guest_profile=self.profile_path)
