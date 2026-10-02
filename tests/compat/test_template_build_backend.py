"""Use the real pinned SDK constructor, synthetic guest transport only."""
import contextlib
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.template_build_service import Backend
from tools.template_build_store import BuildError
from e2b import CommandExitException, FileNotFoundException, TimeoutException


class BuildSdkSession(unittest.TestCase):
    def test_existing_private_vm_session_uses_real_sdk_without_second_create(self):
        self.exercise("dockerfile")

    def test_public_import_uploads_fetcher_and_seventh_tool_without_from_build(self):
        self.exercise("oci")

    def test_sparse_candidate_digest_mismatch_still_refuses_without_second_create(self):
        self.exercise('oci', bad_digest=True)

    def test_worker_preflight_failure_retains_bounded_cause_without_status_file(self):
        self.exercise('dockerfile', early_failure=True)

    def test_failed_status_and_empty_tool_logs_retain_conversion_cause(self):
        for kind in ('oci', 'dockerfile'):
            with self.subTest(kind=kind):
                output = self.exercise(kind, failure={'logs': b''})
                self.assertEqual(output, b'\n[worker command failed]\nInvalidTemplate: OCI expanded byte limit\n')

    def test_diagnostics_share_full_log_budget_and_survive_long_stdout(self):
        output = self.exercise('oci', failure={'logs': b'a' * 32768, 'stdout': 'x' * 100000})
        self.assertEqual(len(output), 65536)
        self.assertIn(b'[worker command failed]', output)
        self.assertTrue(output.endswith(b'InvalidTemplate: OCI expanded byte limit\n'))

    def test_tool_log_prefix_is_kept_when_failure_needs_only_part_of_budget(self):
        output = self.exercise('dockerfile', failure={'logs': b'a' * 32768})
        self.assertEqual(len(output), 65536)
        self.assertTrue(output.startswith(b'a' * 64000))
        self.assertTrue(output.endswith(b'InvalidTemplate: OCI expanded byte limit\n'))

    def test_small_byte_budget_retains_stderr_tail_without_utf8_expansion(self):
        output = self.exercise('oci', failure={'logs': b'', 'limit': 31,
                                              'stderr': '\u754c' * 10000 + '\nreason\n'})
        self.assertEqual(len(output), 31)
        self.assertTrue(output.endswith(b'\nreason\n'))

    def test_failed_worker_missing_tool_logs_still_exposes_its_cause(self):
        output = self.exercise('oci', failure={'missing_logs': True})
        self.assertIn(b'OCI expanded byte limit', output)

    def test_nonzero_worker_cannot_publish_even_if_it_wrote_candidate_status(self):
        self.exercise('oci', failure={'state': 'candidate', 'reason': 'WorkerFailed:CommandExit'})

    def test_command_timeout_retains_tool_logs_without_reading_candidate_or_status(self):
        for kind in ('oci', 'dockerfile'):
            with self.subTest(kind=kind):
                output = self.exercise(kind, command_timeout={})
                self.assertIn(b'[worker command wait timed out; partial logs]', output)
                self.assertIn(b'[build.log]\nbounded log\n', output)
                self.assertIn(b'[daemon.log]\nbounded log\n', output)

    def test_timeout_log_cap_stops_reading_and_closes_the_stream(self):
        closed = []
        extra_reads = []
        @contextlib.contextmanager
        def read(_path, **_opts):
            def blocks():
                yield b'x' * 100000
                extra_reads.append(True)
                yield b'unexpected extra bytes'
            try:
                yield blocks()
            finally:
                closed.append(True)
        output = self.exercise('oci', command_timeout={'read': read})
        self.assertEqual(len(output), 65536)
        self.assertEqual(closed, [True])
        self.assertEqual(extra_reads, [])

    def test_timeout_keeps_partial_logs_when_transport_fails_without_leaking_exception(self):
        def read(path, **_opts):
            if path.endswith('daemon.log'):
                raise FileNotFoundException('builder-key:private-token')
            def blocks():
                yield b'last completed build step\n'
                raise TimeoutException('builder-key:private-token')
            return contextlib.nullcontext(blocks())
        output = self.exercise('dockerfile', command_timeout={'read': read})
        self.assertIn(b'last completed build step\n', output)
        self.assertEqual(output.count(b'[log unavailable]'), 2)

    def test_timeout_log_window_is_shared_and_never_starts_another_read_after_expiry(self):
        now = [10.0]
        reads = []
        closed = []
        late_reads = []
        @contextlib.contextmanager
        def read(path, **opts):
            reads.append(path)
            self.assertEqual(opts['request_timeout'], 1)
            def blocks():
                now[0] += 6
                yield b'last bytes from in-flight read\n'
                late_reads.append(True)
                yield b'unexpected late bytes'
            try:
                yield blocks()
            finally:
                closed.append(True)
        with patch('tools.template_build_service.time.monotonic', side_effect=lambda: now[0]):
            output = self.exercise('oci', command_timeout={'read': read})
        self.assertEqual(reads, ['/output/job/build.log'])
        self.assertEqual(closed, [True])
        self.assertEqual(late_reads, [])
        self.assertIn(b'last bytes from in-flight read', output)

    def test_tiny_timeout_log_budget_does_not_start_a_guest_read(self):
        reads = []
        def read(*_args, **_opts):
            reads.append(True)
            return contextlib.nullcontext(iter([b'unexpected bytes']))
        output = self.exercise('oci', command_timeout={'read': read, 'limit': 1})
        self.assertEqual(output, b'[')
        self.assertEqual(reads, [])

    def test_failed_timeout_log_write_preserves_the_original_timeout(self):
        with patch('tools.template_build_service.durable_write', side_effect=OSError('disk full')):
            self.exercise('oci', command_timeout={'write_failed': True})

    def test_expired_build_never_starts_status_download(self):
        self.exercise('oci', download_deadline={'phase': 'before'})

    def test_artifact_download_rejects_late_chunks_and_closes_stream(self):
        for name in ('status.json', 'candidate.ext4', 'build.log', 'daemon.log'):
            with self.subTest(name=name):
                self.exercise('oci', download_deadline={'phase': 'block', 'name': name})

    def test_artifact_download_rejects_eof_arriving_after_original_deadline(self):
        self.exercise('dockerfile', download_deadline={'phase': 'eof', 'name': 'candidate.ext4'})

    def exercise(self, kind, early_failure=False, failure=None, bad_digest=False, command_timeout=None,
                 download_deadline=None):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source = directory / "source.tar"
            source.write_bytes(b"opaque input")
            candidate = b"\0" * (1024 * 1024)
            status = {"state": "candidate", "rootfs_sha256": hashlib.sha256(candidate).hexdigest()}
            if bad_digest:
                status['rootfs_sha256'] = 'f' * 64
            if failure is not None:
                status.update(state=failure.get('state', 'failed'), error_type='InvalidTemplate')
            profile = {"runtime": {"credentials": [{"tenant": 1, "secret": "runtime-key"}]},
                "builder": {"bind": "127.0.0.1:1", "domain": "example.invalid", "credentials": [{"secret": "builder-key"}]},
                "tools": {name: {"path": str(source), "sha256": "a" * 64} for name in
                    ("buildctl", "buildkitd", "buildkit-runc", "umoci", "envd", "busybox", "regctl")},
                "dns_servers": ["1.1.1.1", "8.8.8.8"]}
            backend = Backend(profile)
            row = {"id": "a" * 32, "input_hash": hashlib.sha256(source.read_bytes()).hexdigest(), "deadline": 1000,
                "request": json.dumps({"kind": kind, "image": "docker.io/library/ubuntu:24.04",
                    "resources": {"diskMiB": 1, "minFreeMiB": 16}})}
            now = [0]
            store = SimpleNamespace(directory=lambda _: directory, clock=lambda: now[0],
                limits={"max_input_bytes": 65536, "max_expanded_bytes": 65536, "max_entries": 100, "max_log_bytes": 65536})
            if failure is not None or command_timeout is not None:
                options = failure if failure is not None else command_timeout
                store.limits['max_log_bytes'] = options.get('limit', 65536)
            session = {"sandboxID": "sb-test-1", "envdVersion": "0.9.0", "envdAccessToken": "private-token"}
            timeout_error = TimeoutException('host-connection-context:private-token:builder-key')
            closed, extra_reads = [], []
            if download_deadline is not None:
                (directory / 'candidate.ext4').write_bytes(b'previous artifact')
            @contextlib.contextmanager
            def delayed(data):
                def blocks():
                    if download_deadline['phase'] == 'eof':
                        yield data
                        now[0] = 1000
                        return
                    yield data[:1]
                    now[0] = 1000
                    yield data[1:]
                    extra_reads.append(True)
                try:
                    yield blocks()
                finally:
                    closed.append(True)
            def stream(path, **_opts):
                if command_timeout is not None and 'read' in command_timeout:
                    return command_timeout['read'](path, **_opts)
                if early_failure and path.endswith('status.json'):
                    raise FileNotFoundException('no early status')
                if failure is not None and path.endswith('.log') and failure.get('missing_logs'):
                    raise FileNotFoundException('no tool log')
                log = failure.get('logs', b'bounded log\n') if failure is not None else b'bounded log\n'
                data = json.dumps(status).encode() if path.endswith("status.json") else candidate if path.endswith("candidate.ext4") else log
                if download_deadline is not None and path.endswith(download_deadline.get('name', '/never')):
                    return delayed(data)
                return contextlib.nullcontext(iter([data]))
            def execute(command, **_opts):
                if command_timeout is not None and command.startswith('python3 /input/template_worker.py'):
                    raise timeout_error
                if early_failure and command.startswith('python3 /input/template_worker.py'):
                    raise CommandExitException(stderr='x' * 100000 + '\nbuild disk budget unavailable\n',
                                               stdout='', exit_code=1, error=None)
                if failure is not None and command.startswith('python3 /input/template_worker.py'):
                    raise CommandExitException(stdout=failure.get('stdout', ''),
                        stderr=failure.get('stderr', 'InvalidTemplate: OCI expanded byte limit\n'), exit_code=1,
                        error='host-connection-context:private-token:builder-key')
                if download_deadline is not None and command.startswith('python3 /input/template_worker.py'):
                    now[0] = 1000 if download_deadline['phase'] == 'before' else 999.75
                return SimpleNamespace(stdout='12345678-1234-1234-1234-123456789abc\n')
            # Keep Sandbox itself real: incorrect kwargs/version/configuration
            # must fail here, not hide behind a constructor test double.
            with patch.object(backend, "vm", return_value={"session": session}) as identity, \
                    patch("e2b.sandbox_sync.main.Filesystem") as files, \
                    patch("e2b.sandbox_sync.main.Commands") as commands:
                files.return_value.read.side_effect = stream
                commands.return_value.run.side_effect = execute
                if download_deadline is not None:
                    with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'):
                        backend.build(store, row)
                    identity.assert_called_once_with(row, create=True)
                    self.assertEqual(row['deadline'], 1000)
                    self.assertEqual((directory / 'candidate.ext4').read_bytes(), b'previous artifact')
                    self.assertEqual(list(directory.glob('*.partial')), [])
                    self.assertEqual(extra_reads, [])
                    reads = files.return_value.read.call_args_list
                    if download_deadline['phase'] == 'before':
                        self.assertEqual(reads, [])
                        self.assertEqual(closed, [])
                    else:
                        self.assertEqual(closed, [True])
                        self.assertEqual(reads[-1].args[0], '/output/job/' + download_deadline['name'])
                        self.assertTrue(all(call.kwargs['request_timeout'] == 0.25 for call in reads))
                    return
                if command_timeout is not None:
                    with self.assertRaises(TimeoutException) as caught:
                        backend.build(store, row)
                    self.assertIs(caught.exception, timeout_error)
                    identity.assert_called_once_with(row, create=True)
                    self.assertEqual(row['deadline'], 1000)
                    self.assertFalse((directory / 'candidate.ext4').exists())
                    self.assertFalse((directory / 'status.json').exists())
                    self.assertEqual(sum(c.args[0].startswith('python3 /input/template_worker.py')
                                         for c in commands.return_value.run.call_args_list), 1)
                    self.assertTrue(all(c.args[0] in ('/output/job/build.log', '/output/job/daemon.log')
                                        and c.kwargs['request_timeout'] == 1
                                        for c in files.return_value.read.call_args_list))
                    if command_timeout.get('write_failed'):
                        self.assertFalse((directory / 'build.log').exists())
                        return
                    output = (directory / 'build.log').read_bytes()
                    self.assertLessEqual(len(output), store.limits['max_log_bytes'])
                    self.assertNotIn(b'private-token', output)
                    self.assertNotIn(b'builder-key', output)
                    return output
                if failure is not None:
                    with self.assertRaises(BuildError) as caught:
                        backend.build(store, row)
                    self.assertEqual(caught.exception.reason, failure.get('reason', 'WorkerFailed:InvalidTemplate'))
                    self.assertFalse((directory / 'candidate.ext4').exists())
                    self.assertNotIn('/output/job/candidate.ext4', [call.args[0] for call in files.return_value.read.call_args_list])
                    identity.assert_called_once_with(row, create=True)
                    output = (directory / 'build.log').read_bytes()
                    self.assertLessEqual(len(output), store.limits['max_log_bytes'])
                    self.assertNotIn(b'private-token', output)
                    self.assertNotIn(b'builder-key', output)
                    return output
                if early_failure:
                    with self.assertRaises(BuildError) as caught:
                        backend.build(store, row)
                    self.assertEqual(caught.exception.reason, 'WorkerPreflightFailed')
                    self.assertEqual((directory / 'build.log').stat().st_size, 65536)
                    self.assertTrue((directory / 'build.log').read_text().endswith('build disk budget unavailable\n'))
                    self.assertFalse((directory / 'candidate.ext4').exists())
                    identity.assert_called_once_with(row, create=True)
                    return
                if bad_digest:
                    with self.assertRaisesRegex(BuildError, 'CandidateDigestMismatch'):
                        backend.build(store, row)
                else:
                    self.assertEqual(backend.build(store, row), status)
                identity.assert_called_once_with(row, create=True)
                config = files.call_args.args[2]
                self.assertEqual(config.sandbox_headers["X-Access-Token"], "private-token")
                self.assertEqual((directory / "candidate.ext4").stat().st_size, len(candidate))
                self.assertEqual((directory / "candidate.ext4").read_bytes(), candidate)
                if hasattr((directory / "candidate.ext4").stat(), 'st_blocks'):
                    self.assertLess((directory / "candidate.ext4").stat().st_blocks * 512, len(candidate) // 4)
                writes = {call.args[0]: call.args[1] for call in files.return_value.write.call_args_list}
                self.assertIn("/input/template_fetch.py", writes)
                guest_profile = json.loads(writes["/input/profile.json"])
                self.assertEqual("regctl" in guest_profile, kind == "oci")
                worker = json.loads(writes["/input/request.json"])
                self.assertEqual(worker["kind"], "oci-reference" if kind == "oci" else "dockerfile")
                self.assertEqual(worker["dns_servers"], profile["dns_servers"])
                self.assertEqual(writes["/etc/resolv.conf"], "nameserver 1.1.1.1\nnameserver 8.8.8.8\n")
                self.assertNotIn("/input/Dockerfile", writes)
