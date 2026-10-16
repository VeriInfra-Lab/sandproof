"""Private installed/proc fixtures and exact SDK orchestration; no VM/network."""
import copy
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools import guest_probe as probe, guest_artifacts as artifacts
from tools.system_acceptance import write_once
from tools.tests.test_guest_artifacts import manifest


class GuestObservationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.platform = self.root / 'platform'
        self.platform.mkdir()
        self.proc = self.root / 'proc'
        (self.proc / 'net').mkdir(parents=True)
        self.expected = {}
        for role, name in probe.NAMES.items():
            path = self.platform / name
            path.write_bytes(('private fixture ' + role).encode())
            path.chmod(0o500 if role in ('service', 'worker') else 0o400)
            self.expected[role] = artifacts.fingerprint(path, probe.LIMITS[role])
        self.owner = os.geteuid()
        self.listener()
        self.process(42)

    def listener(self, *, inode=77, address='00000000', state='0A'):
        (self.proc / 'net/tcp').write_text(
            'sl local_address rem_address st tx_queue rx tr retr uid timeout inode\n'
            f'0: {address}:{49983:04X} 00000000:0000 {state} 0:0 0:0 0 0 0 {inode}\n')

    def process(self, pid):
        directory = self.proc / str(pid)
        (directory / 'fd').mkdir(parents=True)
        (directory / 'exe').symlink_to(self.platform / probe.NAMES['service'])
        argv = (str(self.platform / probe.NAMES['service']) + '\0serve\0--config\0'
                + str(self.platform / probe.NAMES['configuration']) + '\0')
        (directory / 'cmdline').write_bytes(argv.encode())
        self.ticks(pid, 123)
        (directory / 'fd/6').symlink_to('socket:[77]')
        return directory

    def ticks(self, pid, value):
        (self.proc / str(pid) / 'stat').write_text(
            f'{pid} (name with ) spaces) S ' + '0 ' * 18 + str(value) + ' 0\n')

    def observe(self):
        observed = probe.installed(self.platform, self.expected, self.owner)
        return probe.observe_daemon(self.proc, self.platform, self.expected,
                                    observed['service'], self.owner)

    def test_actual_files_match_original_descriptors_and_one_listening_executable(self):
        self.assertEqual(self.observe(), dict(pid=42, start_ticks=123, socket_inode=77))
        # Unrelated process arguments may be huge/private. They are not read.
        unrelated = self.process(43)
        (unrelated / 'exe').unlink()
        (unrelated / 'exe').symlink_to(self.platform / probe.NAMES['worker'])
        (unrelated / 'cmdline').write_bytes(b'private' * 10000)
        self.assertEqual(self.observe()['pid'], 42)
        for role in probe.NAMES:
            path = self.platform / probe.NAMES[role]
            original = path.read_bytes()
            path.chmod(0o700)
            path.write_bytes(b'!' + original[1:])
            with self.subTest(role=role), self.assertRaises(ValueError):
                self.observe()
            path.write_bytes(original)
            path.chmod(0o500 if role in ('service', 'worker') else 0o400)

    def test_installed_bytes_alone_cannot_identify_another_process_or_socket(self):
        executable = self.proc / '42/exe'
        executable.unlink()
        executable.symlink_to(self.platform / probe.NAMES['worker'])
        with self.assertRaises(ValueError):
            self.observe()
        executable.unlink()
        # Even a byte-identical different inode is not the installed daemon.
        other = self.root / 'other-service'
        other.write_bytes((self.platform / probe.NAMES['service']).read_bytes())
        other.chmod(0o500)
        executable.symlink_to(other)
        with self.assertRaises(ValueError):
            self.observe()
        executable.unlink()
        executable.symlink_to(self.platform / probe.NAMES['service'])
        for address, state, inode in [('0100007F', '0A', 77), ('00000000', '01', 77),
                                      ('00000000', '0A', 88)]:
            self.listener(address=address, state=state, inode=inode)
            with self.subTest(address=address, state=state, inode=inode), self.assertRaises(ValueError):
                self.observe()
        self.listener()
        self.process(43)
        with self.assertRaises(ValueError):
            self.observe()

    def test_process_exec_identity_arguments_and_start_ticks_are_rechecked(self):
        original = probe.owns_socket
        for change in ('ticks', 'arguments', 'executable', 'listener'):
            with self.subTest(change=change):
                calls = 0
                def change_after_first_observation(directory, inode):
                    nonlocal calls
                    result = original(directory, inode)
                    calls += 1
                    if calls == 1:
                        if change == 'ticks':
                            self.ticks(42, 124)
                        elif change == 'arguments':
                            (self.proc / '42/cmdline').write_bytes(b'wrong\0')
                        elif change == 'executable':
                            (self.proc / '42/exe').unlink()
                            (self.proc / '42/exe').symlink_to(self.platform / probe.NAMES['worker'])
                        else:
                            self.listener(inode=88)
                    return result
                with patch.object(probe, 'owns_socket', side_effect=change_after_first_observation):
                    with self.assertRaises(ValueError):
                        self.observe()
                shutil.rmtree(self.proc / '42')
                self.process(42)
                self.listener()

    def test_link_permission_size_and_scanning_bounds_fail_closed(self):
        service = self.platform / probe.NAMES['service']
        for mode in (0o600, 0o777):
            service.chmod(mode)
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                self.observe()
        service.chmod(0o500)
        configuration = self.platform / probe.NAMES['configuration']
        configuration.chmod(0o640)
        with self.assertRaises(ValueError):
            self.observe()
        configuration.chmod(0o400)
        os.link(service, self.root / 'hardlink')
        with self.assertRaises(ValueError):
            self.observe()
        (self.root / 'hardlink').unlink()
        with patch.object(probe, 'PROCESS_LIMIT', 0), self.assertRaises(ValueError):
            self.observe()
        with patch.object(probe, 'FD_LIMIT', 0), self.assertRaises(ValueError):
            self.observe()
        with patch.object(probe, 'PROGRAM_LIMIT', 8), self.assertRaises(ValueError):
            probe.command(self.expected)
        with self.assertRaises(ValueError):
            probe.small_file(self.proc / '42/cmdline', 2)
        (self.proc / '42/stat').write_text('42 (missing closing parenthesis ' + '1 ' * 25)
        with self.assertRaises(ValueError):
            self.observe()
        self.ticks(42, 123)
        service.rename(self.root / 'moved')
        service.symlink_to(self.root / 'moved')
        with self.assertRaises(OSError):
            self.observe()

    def test_exact_report_shape_and_program_do_not_accept_boolean_or_extra_evidence(self):
        good = dict(schema_version=1, files=self.expected,
                    daemon=dict(pid=42, start_ticks=123, socket_inode=77))
        result = lambda report: SimpleNamespace(exit_code=0, stdout=json.dumps(report), stderr='')
        self.assertEqual(probe.receipt(result(good), self.expected), good)
        for field in ('pid', 'start_ticks', 'socket_inode'):
            bad = copy.deepcopy(good)
            bad['daemon'][field] = True
            with self.subTest(field=field), self.assertRaises(ValueError):
                probe.receipt(result(bad), self.expected)
        bad = copy.deepcopy(good)
        bad['files']['worker']['sha256'] = '0' * 64
        with self.assertRaises(ValueError):
            probe.receipt(result(bad), self.expected)
        for bad in (dict(good, extra='payload'), dict(good, schema_version=True)):
            with self.assertRaises(ValueError):
                probe.receipt(result(bad), self.expected)
        for bad in (SimpleNamespace(exit_code=1, stdout='', stderr=''),
                    SimpleNamespace(exit_code=0, stdout='{}' * 4096, stderr=''),
                    SimpleNamespace(exit_code=0, stdout=json.dumps(good), stderr='warning')):
            with self.assertRaises(ValueError):
                probe.receipt(bad, self.expected)
        args = shlex.split(probe.command(self.expected))
        self.assertEqual(args[:3], ['/usr/bin/python3', '-I', '-c'])
        self.assertEqual(json.loads(args[4]), self.expected)
        self.assertLessEqual(len(probe.command(self.expected).encode()), probe.COMMAND_LIMIT)
        # Execute only the invalid-argument path of the actual sent program.
        # This fails before reading /proc, installed paths or invoking an SDK.
        for flags in ([], ['-O']):
            checked = subprocess.run([sys.executable, '-I', *flags, '-c', args[3], '{}'],
                capture_output=True, text=True, timeout=5)
            self.assertEqual(checked.returncode, 1)
            self.assertEqual(checked.stdout, '')
            self.assertEqual(checked.stderr, 'guest_identity_failed\n')

    def test_descriptor_observation_rejects_replacement_and_changes_during_hash(self):
        service = self.platform / probe.NAMES['service']
        original = service.read_bytes()
        read = os.read
        for change in ('growth', 'same-size', 'replacement'):
            service.chmod(0o700)
            service.write_bytes(original)
            service.chmod(0o500)
            before = service.stat()
            changed = False
            def mutate(fd, size):
                nonlocal changed
                block = read(fd, size)
                if block and not changed:
                    changed = True
                    if change == 'replacement':
                        service.rename(self.root / 'old-service')
                        service.write_bytes(original)
                        service.chmod(0o500)
                    else:
                        service.chmod(0o700)
                        service.write_bytes(original + b'!' if change == 'growth' else b'!' + original[1:])
                        service.chmod(0o500)
                        if change == 'same-size':
                            # All original bytes have already been read. This
                            # case tests a changed metadata snapshot, not an
                            # atomic read guarantee. Rapid writes may share the
                            # same Linux timestamp tick; establish the fault
                            # explicitly instead of sleeping or retrying.
                            os.utime(service, ns=(before.st_atime_ns,
                                                 before.st_mtime_ns + 2_000_000_000))
                            self.assertNotEqual(service.stat().st_mtime_ns, before.st_mtime_ns)
                return block
            with self.subTest(change=change), patch.object(probe.os, 'read', side_effect=mutate):
                with self.assertRaises(ValueError):
                    probe.installed(self.platform, self.expected, self.owner)
            self.assertTrue(changed, 'the intended read-time fault was not reached')

    def test_local_release_and_config_pins_are_checked_before_building_a_guest_command(self):
        release = self.root / 'release'
        release.mkdir(mode=0o700)
        value = manifest(release)
        write_once(release / 'manifest.json', value)
        config = self.root / 'configuration.json'
        write_once(config, dict(version=2, worker=artifacts.worker_configuration(value)))
        manifest_hash = hashlib.sha256((release / 'manifest.json').read_bytes()).hexdigest()
        config_hash = hashlib.sha256(config.read_bytes()).hexdigest()
        identity, expected = probe.load_expected(release, manifest_hash, config, config_hash)
        self.assertEqual(identity['source_commit'], value['source']['commit'])
        self.assertEqual(expected['manifest']['sha256'], manifest_hash)
        for pins in (('0'*64, config_hash), (manifest_hash, '0'*64)):
            with self.subTest(pins=pins), self.assertRaises(ValueError):
                probe.load_expected(release, pins[0], config, pins[1])

    def test_fixed_go_observation_uses_same_real_executable_and_socket_checks(self):
        path = self.platform / 'envd'
        path.write_bytes(b'private Go fixture')
        path.chmod(0o755)
        expected = artifacts.fingerprint(path, probe.LIMITS['service'])
        executable = self.proc / '42/exe'
        executable.unlink()
        executable.symlink_to(path)
        argv = (probe.GO_PATH + '\0-verbose\0').encode()
        (self.proc / '42/cmdline').write_bytes(argv)
        installed = probe.installed_file(path, expected, self.owner, 0o755)
        def observe():
            return probe.observe_process(self.proc, expected, installed, self.owner, argv)
        self.assertEqual(observe(), dict(pid=42, start_ticks=123, socket_inode=77))
        for changed in (argv[:-1], argv + b'-no-cgroups\0', b'other\0-verbose\0'):
            (self.proc / '42/cmdline').write_bytes(changed)
            with self.subTest(argv=changed), self.assertRaises(ValueError):
                observe()
        (self.proc / '42/cmdline').write_bytes(argv)
        path.chmod(0o777)
        with self.assertRaises(ValueError):
            probe.installed_file(path, expected, self.owner, 0o755)

    def test_go_pin_and_receipt_cannot_accept_rust_shape_wrong_bytes_or_boolean_identity(self):
        pinned = probe.load_go_expected()
        self.assertEqual(pinned['source_commit'], 'f455fd58fa3f289e78ce1de690c35968d8d29322')
        expected = pinned['file']
        report = dict(schema_version=1, fixed_go=expected,
                      daemon=dict(pid=42, start_ticks=123, socket_inode=77))
        result = lambda item: SimpleNamespace(exit_code=0, stdout=json.dumps(item), stderr='')
        self.assertEqual(probe.go_receipt(result(report), expected), report)
        for bad in (dict(report, files=self.expected), dict(report, schema_version=True),
                    dict(report, fixed_go=dict(expected, bytes=1)),
                    dict(report, daemon=dict(pid=True, start_ticks=123, socket_inode=77))):
            with self.subTest(report=bad), self.assertRaises(ValueError):
                probe.go_receipt(result(bad), expected)
        args = shlex.split(probe.go_command(expected))
        self.assertEqual(json.loads(args[4]), {'fixed_go': expected})
        self.assertEqual(args[:3], ['/usr/bin/python3', '-I', '-c'])
        for flags in ([], ['-O']):
            checked = subprocess.run([sys.executable, '-I', *flags, '-c', args[3],
                                      '{"fixed_go":{"bytes":true,"sha256":"wrong"}}'],
                capture_output=True, text=True, timeout=5)
            self.assertEqual((checked.returncode, checked.stdout, checked.stderr),
                             (1, '', 'guest_identity_failed\n'))

    def resource_files(self):
        directory = self.proc / '42'
        (directory / 'status').write_bytes(
            b'Name:\tprivate\nPid:\t42\nTgid:\t42\nThreads:\t3\nVmRSS:\t1 kB\nVmHWM:\t512 kB\n')
        (directory / 'smaps_rollup').write_bytes(
            b'0000-ffff ---p 00000000 00:00 0 [rollup]\nRss:\t768 kB\nPss:\t123 kB\n')
        return directory

    def test_resource_sample_uses_rollup_rss_and_separate_kernel_hwm_and_fd_count(self):
        self.resource_files()
        daemon = self.observe()
        with patch.object(probe.time, 'monotonic_ns', side_effect=[100, 150]):
            sample = probe.read_resources(self.proc, daemon)
        self.assertEqual(sample, dict(rss_bytes=768*1024, reported_peak_rss_bytes=512*1024,
                                      threads=3, open_fds=1, elapsed_ns=50))
        # VmHWM and the later rollup snapshot are neither atomic nor equally
        # precise. The observer must not silently turn one into the other.
        (self.proc / '42/fd/7').symlink_to('pipe:[88]')
        self.assertEqual(probe.read_resources(self.proc, daemon)['open_fds'], 2)
        with patch.object(probe, 'FD_LIMIT', 1), self.assertRaises(ValueError):
            probe.read_resources(self.proc, daemon)

    def test_resource_files_refuse_missing_malformed_duplicate_large_or_linked_fields(self):
        directory = self.resource_files()
        daemon = self.observe()
        for name, content in [
            ('status', b'Pid: 42\nTgid: 43\nThreads: 3\nVmHWM: 512 kB\n'),
            ('status', b'Pid: 42\nTgid: 42\nThreads: 0\nVmHWM: 512 kB\n'),
            ('status', b'Pid: 42\nTgid: 42\nThreads: 4097\nVmHWM: 512 kB\n'),
            ('smaps_rollup', b'Rss: -1 kB\n'),
            ('smaps_rollup', b'Rss: 1 KB\n'),
            ('smaps_rollup', b'Rss: 1.0 kB\n'),
            ('smaps_rollup', b'Rss: true kB\n'),
            ('smaps_rollup', b'Rss: 1 kB\nRss: 2 kB\n'),
            ('smaps_rollup', b'Pss: 1 kB\n'),
            ('smaps_rollup', b'Rss: 18446744073709551616 kB\n'),
            ('smaps_rollup', b'Rss: 1 kB\n' + b' ' * (16 * 1024)),
        ]:
            with self.subTest(name=name, content=content[:60]):
                self.resource_files()
                (directory / name).write_bytes(content)
                with self.assertRaises(ValueError):
                    probe.read_resources(self.proc, daemon)
        self.resource_files()
        (directory / 'smaps_rollup').unlink()
        with self.assertRaises(OSError):
            probe.read_resources(self.proc, daemon)
        (directory / 'smaps_rollup').symlink_to('status')
        with self.assertRaises(OSError):
            probe.read_resources(self.proc, daemon)

    def test_resource_read_refuses_process_exec_arguments_or_listener_changes(self):
        self.resource_files()
        daemon = self.observe()
        read = probe.proc_snapshot
        for change in ('ticks', 'arguments', 'executable', 'listener'):
            changed = []
            def mutate(name, directory):
                raw = read(name, directory)
                if name == 'smaps_rollup':
                    changed.append(change)
                    if change == 'ticks':
                        self.ticks(42, 124)
                    elif change == 'arguments':
                        (self.proc / '42/cmdline').write_bytes(b'other\0')
                    elif change == 'executable':
                        (self.proc / '42/exe').unlink()
                        (self.proc / '42/exe').symlink_to(self.platform / probe.NAMES['worker'])
                    else:
                        self.listener(inode=88)
                return raw
            with self.subTest(change=change), patch.object(probe, 'proc_snapshot', side_effect=mutate):
                with self.assertRaises(ValueError):
                    probe.read_resources(self.proc, daemon)
            self.assertEqual(changed, [change])
            shutil.rmtree(self.proc / '42')
            self.process(42)
            self.resource_files()
            self.listener()

    def test_resource_observation_binds_original_target_before_and_after_the_sample(self):
        self.resource_files()
        daemon = self.observe()
        read = probe.read_resources
        for expected in (self.expected, {'fixed_go': probe.load_go_expected()['file']}):
            before = dict(schema_version=1, daemon=daemon, files=expected)
            def sample(proc, original):
                self.assertEqual(proc, Path('/proc'))
                self.assertEqual(original, daemon)
                return read(self.proc, original)
            request = dict(expected=expected, daemon=daemon)
            with patch.object(probe, 'observe_target', side_effect=[before, before]) as observe, \
                    patch.object(probe, 'read_resources', side_effect=sample) as resources:
                result = probe.observe_resources(request)
            self.assertEqual([call.args for call in observe.call_args_list], [(expected,), (expected,)])
            self.assertEqual(resources.call_count, 1)
            self.assertEqual((result['expected'], result['daemon']), (expected, daemon))
            after = dict(before, daemon=dict(daemon, start_ticks=124))
            with patch.object(probe, 'observe_target', side_effect=[before, after]), \
                    patch.object(probe, 'read_resources', side_effect=sample), self.assertRaises(ValueError):
                probe.observe_resources(request)
            with patch.object(probe, 'observe_target', return_value=after), \
                    patch.object(probe, 'read_resources') as resources, self.assertRaises(ValueError):
                probe.observe_resources(request)
            resources.assert_not_called()

    def test_resource_receipts_reject_wrong_target_identity_shape_or_unbounded_metrics(self):
        self.resource_files()
        daemon = self.observe()
        sample = probe.read_resources(self.proc, daemon)
        result = lambda value: SimpleNamespace(exit_code=0, stdout=json.dumps(value), stderr='')
        for expected in (self.expected, {'fixed_go': probe.load_go_expected()['file']}):
            good = dict(schema_version=1, expected=expected, daemon=daemon, sample=sample)
            self.assertEqual(probe.resource_receipt(result(good), expected, daemon), good)
            bad_reports = [dict(good, schema_version=True), dict(good, extra=True),
                           dict(good, daemon=dict(daemon, pid=43)),
                           dict(good, daemon=dict(daemon, pid=True)),
                           dict(good, expected={'fixed_go': dict(bytes=1, sha256='0'*64)})]
            for field in sample:
                for value in (True, -1, 2**64, '1'):
                    bad_reports.append(dict(good, sample=dict(sample, **{field: value})))
            for bad in bad_reports:
                with self.subTest(report=bad), self.assertRaises(ValueError):
                    probe.resource_receipt(result(bad), expected, daemon)
            for bad in (SimpleNamespace(exit_code=1, stdout=json.dumps(good), stderr=''),
                        SimpleNamespace(exit_code=0, stdout=json.dumps(good), stderr='warning'),
                        SimpleNamespace(exit_code=0, stdout='{}' * 4096, stderr='')):
                with self.assertRaises(ValueError):
                    probe.resource_receipt(bad, expected, daemon)

    def test_resource_program_has_exact_pins_bounded_shell_and_optimized_failure_path(self):
        daemon = self.observe()
        for expected in (self.expected, {'fixed_go': probe.load_go_expected()['file']}):
            command = probe.resource_command(expected, daemon)
            args = shlex.split(command)
            self.assertEqual(args[:3], ['/usr/bin/python3', '-I', '-c'])
            self.assertEqual(json.loads(args[4]), {'resources': dict(expected=expected, daemon=daemon)})
            self.assertLessEqual(len(command.encode()), probe.COMMAND_LIMIT)
            for flags in ([], ['-O']):
                checked = subprocess.run([sys.executable, '-I', *flags, '-c', args[3],
                    '{"resources":{"expected":{},"daemon":{}}}'],
                    capture_output=True, text=True, timeout=5)
                self.assertEqual((checked.returncode, checked.stdout, checked.stderr),
                                 (1, '', 'guest_identity_failed\n'))
        with patch.object(probe, 'COMMAND_LIMIT', 8), self.assertRaises(ValueError):
            probe.resource_command(self.expected, daemon)
        with self.assertRaises(ValueError):
            probe.resource_command(self.expected, dict(daemon, pid=True))


class LinuxResourceSnapshotTests(unittest.TestCase):
    """Real procfs parsing, selected only by the required Linux tool suite."""

    def test_actual_proc_rollup_and_status_use_the_observed_current_process(self):
        self.assertEqual(sys.platform, 'linux')
        pid = os.getpid()
        directory = os.open(f'/proc/{pid}', os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            ticks = probe.start_ticks(directory, pid)
            status = probe.proc_values(probe.proc_snapshot('status', directory),
                                       {b'Pid': 1, b'Tgid': 1, b'Threads': 1, b'VmHWM': 1024})
            memory = probe.proc_values(probe.proc_snapshot('smaps_rollup', directory), {b'Rss': 1024})
            self.assertEqual((status[b'Pid'], status[b'Tgid']), (pid, pid))
            self.assertGreater(status[b'Threads'], 0)
            self.assertGreater(status[b'VmHWM'], 0)
            self.assertGreater(memory[b'Rss'], 0)
            self.assertEqual(probe.start_ticks(directory, pid), ticks)
        finally:
            os.close(directory)


if __name__ == '__main__':
    unittest.main()
