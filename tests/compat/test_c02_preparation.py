"""Original C02 inputs reach the shared renderer; no service, device or VM."""
import copy
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path[:0] = [str(ROOT), str(ROOT / 'tests/system')]
import sdk_c02_acceptance as runner
from tools.runtime_acceptance_budget import static_cache
from tools import system_acceptance as reports
from tools.tests import test_center_host_fixture as fixture


class C02PreparationTests(unittest.TestCase):
    def test_original_pool_reaches_real_renderer_without_implicit_defaults(self):
        class Rendered(RuntimeError):
            pass

        with tempfile.TemporaryDirectory(prefix='c02-', dir=Path('/tmp').resolve()) as temporary:
            parent = Path(temporary)
            root = parent / 'r'
            root.mkdir()
            inputs = {}
            for name in ('kernel', 'rootfs', 'firecracker'):
                path = parent / name
                path.write_bytes(b'explicit non-executable component fixture')
                inputs[name] = path
            args = SimpleNamespace(**inputs, jailer=parent / 'jailer', uid_base=3200000,
                                   cgroup_parent=parent / 'not-a-real-cgroup',
                                   service_cgroup_record=parent / 'not-executed.json')
            report = reports.new_report('sp12-abcdefgh')
            report['hashes'] = {name: reports.digest(inputs[name]) for name in ('kernel', 'rootfs')}

            def render(_args, actual_root, pools, catalogs, *_args_rest, **_kwargs):
                self.assertEqual(actual_root, root)
                native = copy.deepcopy(pools[0])
                # The original assembly supplies this cache before the pure
                # renderer. Reuse its constructor; never start a service/VM.
                native['bounded_host']['physical_storage']['host_template_caches'] = [
                    static_cache(native, catalogs[0], root / 'template-cache')]
                before = copy.deepcopy(native)
                center, hosts = fixture.FixtureTests().render([native])
                self.assertEqual(center['startup_timeout_secs'], 300)
                self.assertIs(center['public_ipv4_enabled'], False)
                self.assertEqual(hosts[0]['native']['host_resources'], dict(
                    cgroup_parent=str(args.cgroup_parent),
                    memory_max_bytes=catalogs[0]['templates'][0]['resources']['host_memory_bytes']))
                self.assertEqual(hosts[0]['native'], before)
                raise Rendered

            with patch.object(runner, 'provision_runtime'), \
                    patch.object(runner.RuntimeServices, 'prepare', side_effect=render), \
                    self.assertRaises(Rendered):
                runner.prepare(args, root, '1234567890abcdef', 'TESTOWNER', 'TESTFOREIGN', 3100, report)
