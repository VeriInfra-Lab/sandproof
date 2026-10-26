"""Current rendered host inputs for the existing drain oracle; no VM or SDK."""
import copy
from contextlib import contextmanager
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import _center_host_services as roles
import lifecycle_unload_data_drain as drain
import lifecycle_unload_runtime as runtime
from tools.template_build_config import host_uid_range


def rendered_native(state_directory):
    # The original Rust deployment tests deserialize/validate these samples.
    # Feed their current raw native/capture shapes through the same role render
    # used by the real lifecycle scenario, rather than inventing a NodeConfig.
    host = json.loads((ROOT / 'deploy/host.example.json').read_text())
    center = json.loads((ROOT / 'deploy/center.example.json').read_text())
    native = host['native']
    native['node_state_dir'] = str(state_directory)
    _, hosts = roles.render([native],
        registered_catalogs=[native['templates']],
        capture_settings=[host['capture']],
        deployment=host['inbox']['node']['deployment_id'],
        center_root='/controlled/center', host_tls_roots=['/controlled/host/tls'],
        host_addresses=['127.0.0.1:9001'], public_address='127.0.0.1:9000',
        artifact_address='127.0.0.1:9002', redis_address='127.0.0.1:9003',
        password='TESTONLYREDISKEY', key='TESTONLYPUBLICKEY',
        retained_sessions=host['inbox']['max_sessions'],
        registry=center['template_registry'],
        versions={'internal_protocol': center['nodes'][0]['registration']['protocol_version']})
    return hosts[0]['native']


class DataDrainRouteSchemaTests(unittest.TestCase):
    def test_formal_rendered_native_scans_routes_without_derived_input_fields(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            routes = root / 'helper-gates'
            routes.mkdir(mode=0o700)
            native = rendered_native(root)
            self.assertEqual(set(native['bounded_host']), {
                'console_policy', 'maintenance', 'physical_storage', 'uid_base', 'uplink_base'})
            self.assertGreater(len(host_uid_range(native)), 0)
            before = copy.deepcopy(native)
            running = {'context': {'position': 0, 'binding': {'vm': 7}}}
            # Only root ownership is controlled here. No marker, process, data
            # request or cleanup is manufactured by this empty-route check.
            with patch.object(drain, '_private_directory', return_value=routes):
                self.assertEqual(drain._live_routes(native, running), {})
            self.assertEqual(native, before)

    @contextmanager
    def route_cell(self, maximum=5):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            directory = root / 'helper-gates'
            directory.mkdir(mode=0o700)
            native = rendered_native(root)
            native['data_transfer']['max_transfers_per_vm'] = maximum
            path = directory / '0.lock'
            path.touch(mode=0o600)
            actual = path.stat()
            # Match the original RuntimeHostSlot/InflightRecord serialization.
            # Only private-root ownership is supplied as a controlled fact;
            # the process below is this real test's current Linux process.
            context = dict(position=0, vm=7,
                uplink_index=native['bounded_host']['uplink_base'],
                helper_gate=dict(path=str(path), device=actual.st_dev, inode=actual.st_ino,
                    max_data_routes_per_slot=maximum))
            fields = Path(f'/proc/{os.getpid()}/stat').read_text().rsplit(')', 1)[1].split()
            process = dict(pid=os.getpid(), start_time_ticks=int(fields[19]),
                boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip())
            record = dict(version=1, context=context, process=process,
                command_digest=[1]*32, route_index=maximum-1, control=False)
            marker = directory / f'0.lock.route-{maximum-1}.json'
            marker.write_text(json.dumps(record))
            metadata = SimpleNamespace(st_dev=actual.st_dev, st_ino=actual.st_ino,
                st_mode=stat.S_IFREG | 0o600, st_uid=0, st_nlink=1)
            def read_marker(value, _limit):
                return json.loads(Path(value).read_text())
            running = {'context': {'position': 0, 'binding': {'vm': 7}}}
            with patch.object(drain, '_private_directory', return_value=directory), \
                    patch.object(drain.Path, 'lstat', return_value=metadata), \
                    patch.object(runtime, 'original_json', side_effect=read_marker):
                yield native, running, marker, record

    def test_actual_transfer_limit_includes_the_last_route_cell_and_preserves_inputs(self):
        with self.route_cell() as (native, running, marker, record):
            before = copy.deepcopy(native)
            self.assertEqual(native['data_transfer']['max_transfers_per_vm'], 5)
            self.assertNotIn('max_data_routes_per_slot', native['bounded_host'])
            self.assertEqual(drain._live_routes(native, running), {
                drain._route_key(record['process']): (marker, record)})
            self.assertEqual(native, before)

    def test_persisted_capacity_and_original_route_identity_cannot_be_substituted(self):
        for field in ('capacity', 'position', 'vm', 'uplink', 'inode', 'path'):
            with self.subTest(field=field), self.route_cell() as (native, running, marker, record):
                context = record['context']
                if field == 'capacity': context['helper_gate']['max_data_routes_per_slot'] = 4
                elif field == 'position': context['position'] = 1
                elif field == 'vm': context['vm'] = 8
                elif field == 'uplink': context['uplink_index'] += 1
                elif field == 'inode': context['helper_gate']['inode'] += 1
                else: context['helper_gate']['path'] += '.foreign'
                marker.write_text(json.dumps(record))
                with self.assertRaises(ValueError):
                    drain._live_routes(native, running)

    def test_control_or_unobserved_process_is_not_an_inflight_data_route(self):
        with self.route_cell() as (native, running, marker, record):
            record['control'] = True
            marker.write_text(json.dumps(record))
            self.assertEqual(drain._live_routes(native, running), {})
            record['control'] = False
            marker.write_text(json.dumps(record))
            with patch.object(drain, '_alive', return_value=False):
                self.assertEqual(drain._live_routes(native, running), {})

    def test_missing_untyped_or_inconsistent_transfer_limits_fail_before_marker_reads(self):
        for fault in ('missing', 'incomplete', 'boolean', 'reserve', 'global'):
            with self.subTest(fault=fault), self.route_cell() as (native, running, _marker, _record):
                if fault == 'missing': del native['data_transfer']
                elif fault == 'incomplete': del native['data_transfer']['max_transfers_per_vm']
                elif fault == 'boolean': native['data_transfer']['max_transfers_per_vm'] = True
                elif fault == 'reserve': native['data_transfer']['control_reserve'] = 5
                else: native['data_transfer']['max_transfers'] = 4
                with patch.object(runtime, 'original_json', side_effect=AssertionError('invalid input reached a route cell')), \
                        self.assertRaises(ValueError):
                    drain._live_routes(native, running)


if __name__ == '__main__':
    unittest.main()
