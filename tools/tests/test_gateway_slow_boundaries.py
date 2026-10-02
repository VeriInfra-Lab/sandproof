"""Guards and deterministic workload/cleanup checks, never SDK/KVM acceptance."""
import asyncio
import copy
import hashlib
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[2] / 'tests/system/gateway_slow_boundaries.py'
SPEC = importlib.util.spec_from_file_location('gateway_slow_probe', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


def memory_snapshot():
    return dict(identity=dict(device=1, inode=2), limits={'memory.max': '268435456'},
                memory_current=100, memory_peak=200,
                memory_events=dict(oom=0, oom_kill=0, oom_group_kill=0))


class ConfigurationTests(unittest.TestCase):
    def test_complete_final_configuration_required_without_timeout_override(self):
        settings = dict(max_active_global=2, data_transfer=probe.FINAL_TRANSFER.copy())
        self.assertEqual(probe.validate_configuration(settings), probe.FINAL_TRANSFER)
        for name in probe.FINAL_TRANSFER:
            invalid = copy.deepcopy(settings)
            invalid['data_transfer'][name] -= 1
            with self.subTest(name=name), self.assertRaises(ValueError):
                probe.validate_configuration(invalid)
            invalid = copy.deepcopy(settings)
            del invalid['data_transfer'][name]
            with self.assertRaises(ValueError):
                probe.validate_configuration(invalid)
        for value in ({}, None, probe.FINAL_TRANSFER | {'control_reserve': True},
                      probe.FINAL_TRANSFER | {'unexpected': 1}):
            with self.assertRaises(ValueError):
                probe.validate_configuration(dict(max_active_global=2, data_transfer=value))
        with self.assertRaises(ValueError):
            probe.validate_configuration(settings | {'max_active_global': 1})

    def test_only_explicit_local_origin_is_allowed(self):
        self.assertEqual(probe.loopback_url('http://127.0.0.1:3000/'), 'http://127.0.0.1:3000')
        for url in ('https://127.0.0.1:3000', 'http://example.com:3000',
                    'http://localhost:3000', 'http://127.0.0.1',
                    'http://key:secret@127.0.0.1:3000', 'http://127.0.0.1:3000/a',
                    'http://127.0.0.1:3000?key=secret', 'http://127.0.0.1:3000#fragment'):
            with self.subTest(url=url), self.assertRaises(ValueError):
                probe.loopback_url(url)

    def test_service_peak_not_rss_and_oom_are_checked(self):
        baseline = memory_snapshot()
        probe.assert_memory(baseline, copy.deepcopy(baseline))
        for key, value in (('memory_peak', 268435457), ('memory_current', 268435457),
                           ('identity', dict(device=1, inode=3)),
                           ('limits', {'memory.max': '536870912'})):
            with self.subTest(key=key), self.assertRaises(AssertionError):
                probe.assert_memory(baseline, baseline | {key: value})
        for event in ('oom', 'oom_kill', 'oom_group_kill'):
            invalid = copy.deepcopy(baseline)
            invalid['memory_events'][event] += 1
            with self.subTest(event=event), self.assertRaises(AssertionError):
                probe.assert_memory(baseline, invalid)

    def test_actual_pid_start_and_exact_server_config_argv_are_required(self):
        meta = SimpleNamespace(st_dev=4, st_ino=5)
        with patch.object(probe, 'start_ticks', return_value=12), \
                patch.object(Path, 'read_bytes', return_value=b'/opt/server\0/private/config.json\0'), \
                patch.object(Path, 'stat', return_value=meta), \
                patch.object(probe, 'digest', return_value='a' * 64):
            binding = probe.process_binding(10, 12, Path('/private/config.json'))
            self.assertEqual(binding['start_ticks'], 12)
            self.assertEqual(binding['executable_inode'], 5)
            with self.assertRaises(ValueError):
                probe.process_binding(10, 13, Path('/private/config.json'))
            with self.assertRaises(ValueError):
                probe.process_binding(10, 12, Path('/other/config.json'))

    def test_observer_checks_membership_and_original_process_each_time(self):
        baseline = memory_snapshot() | dict(pids=[10, 11], path='/sys/fs/cgroup/accept/svc-test',
                                           cpu_stat={}, populated=1)
        with patch.object(probe.service_cgroup, 'observe', return_value=baseline) as observe, \
                patch.object(probe, 'start_ticks', side_effect=lambda pid: {10: 12, 11: 20}[pid]):
            observer = probe.ServiceObserver(Path('/private/record'), 10, 12)
            row = observer.sample('hold')
            self.assertEqual(row['process_identities'], [dict(pid=10, start_ticks=12), dict(pid=11, start_ticks=20)])
            self.assertEqual(observe.call_args.kwargs, {'pids': [10]})
        with patch.object(probe, 'start_ticks', return_value=99), self.assertRaises(ValueError):
            observer.sample('replaced')

    def test_failed_memory_snapshot_is_preserved_before_asserting(self):
        baseline = memory_snapshot() | dict(pids=[10], path='/sys/fs/cgroup/accept/svc-test',
                                           cpu_stat={}, populated=1)
        exceeded = baseline | {'memory_peak': 268435457}
        with patch.object(probe.service_cgroup, 'observe', side_effect=[baseline, exceeded]), \
                patch.object(probe, 'start_ticks', return_value=12):
            observer = probe.ServiceObserver(Path('/private/record'), 10, 12)
            with self.assertRaises(AssertionError):
                observer.sample('failed-peak')
            self.assertEqual(observer.samples[0]['memory_peak'], 268435457)


class WorkloadTests(unittest.TestCase):
    def test_upload_has_six_chunks_five_real_eight_second_intervals(self):
        elapsed = [0]
        writes, sleeps, chunks = [], [], []

        def sleep(seconds):
            sleeps.append(seconds)
            elapsed[0] += seconds

        class Socket:
            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return None

            def sendall(self, value):
                writes.append(value)

        helpers = SimpleNamespace(request_head=lambda *_args: None,
                                  chunk=lambda _sock, body: chunks.append(body),
                                  read_response=lambda _sock: 200)
        with patch.object(probe, 'connection', return_value=Socket()):
            result = probe.progressing_upload(None, 'http://127.0.0.1:3000', '/tmp/progress', helpers,
                                              clock=lambda: elapsed[0], sleep=sleep)
        self.assertEqual(sleeps, [8] * 5)
        self.assertEqual(len(chunks), 6)
        self.assertTrue(all(chunk == b'X' * 65536 for chunk in chunks))
        self.assertEqual(writes, [b'0\r\n\r\n'])
        self.assertEqual(result['elapsed_seconds'], 40)
        self.assertEqual(result['sha256'], hashlib.sha256(b'X' * (6 * 65536)).hexdigest())
        helpers.read_response = lambda _sock: 504
        with patch.object(probe, 'connection', return_value=Socket()), self.assertRaises(AssertionError):
            probe.progressing_upload(None, 'http://127.0.0.1:3000', '/tmp/progress', helpers,
                                     clock=lambda: elapsed[0], sleep=sleep)

    def test_resume_checks_full_contents_and_no_truncation(self):
        with patch.object(probe, 'DOWNLOAD_BYTES', 6):
            result = probe.consume(iter([b'XX', b'XX']), b'XX')
            self.assertEqual(result, dict(bytes=6, sha256=hashlib.sha256(b'XXXXXX').hexdigest()))
            for chunks in ([b'X'], [b'XXXXX'], [b'YY', b'XX'], [b'', b'XXXX']):
                with self.subTest(chunks=chunks), self.assertRaises(AssertionError):
                    probe.consume(iter(chunks), b'XX')


class CleanupTests(unittest.IsolatedAsyncioTestCase):
    async def test_real_prefix_barriers_and_exact_close_on_resume_failure(self):
        trace = []
        elapsed = [0]

        async def advance(seconds):
            elapsed[0] += seconds

        async def call(function, *args, **kwargs):
            return function(*args, **kwargs)

        class Stream:
            def __init__(self, number):
                self.number = number

            def __enter__(self):
                trace.append(('prefix', self.number))
                return iter([b'X', b'bad'])

            def __exit__(self, *_args):
                trace.append(('close', self.number))

        streams = []

        def read(*_args, **kwargs):
            self.assertEqual(kwargs['request_timeout'], 180)
            stream = Stream(len(streams))
            streams.append(stream)
            return stream

        target = SimpleNamespace(commands=SimpleNamespace(run=lambda *_args, **_kwargs: None),
                                 files=SimpleNamespace(get_info=lambda *_args: SimpleNamespace(size=probe.DOWNLOAD_BYTES), read=read))

        async def quiet(_target, _marker, tracked, _helpers):
            item = dict(handle=SimpleNamespace(send_stdin=lambda *_args, **_kwargs: trace.append(('input',))), done=False)
            tracked.append(item)
            return item

        async def limited(*_args):
            trace.append(('429',))

        async def healthy(*_args):
            trace.append(('healthy-control',))

        async def finish(*_args):
            trace.append(('quiet-finished',))

        async def cleanup(tracked):
            self.assertEqual(len(tracked), 1)
            trace.append(('quiet-cleanup',))

        helpers = SimpleNamespace(call=call, limited=limited, healthy=healthy, health=healthy,
                                  cleanup_handles=cleanup)
        observer = SimpleNamespace(sample=lambda label: trace.append(('sample', label)))
        with patch.object(probe, 'quiet_input', side_effect=quiet), \
                patch.object(probe, 'finish_quiet', side_effect=finish), \
                patch.object(probe, 'time', SimpleNamespace(monotonic=lambda: elapsed[0])), \
                patch.object(probe.asyncio, 'sleep', side_effect=advance):
            with self.assertRaises(AssertionError):
                await probe.paused_downloads(target, object(), '/tmp/exact', 'http://127.0.0.1:3000', helpers, observer)
        self.assertEqual([entry for entry in trace if entry[0] == 'prefix'], [('prefix', index) for index in range(6)])
        self.assertGreater(trace.index(('429',)), trace.index(('prefix', 5)))
        self.assertEqual([entry for entry in trace if entry[0] == 'close'], [('close', index) for index in range(6)])
        self.assertEqual(trace[-1], ('quiet-cleanup',))
        self.assertEqual(elapsed[0], 60)
        self.assertEqual(trace.count(('sample', 'paused-downloads')), 60)
        self.assertEqual(probe.PAUSE_SECONDS, 60)


if __name__ == '__main__':
    unittest.main()
