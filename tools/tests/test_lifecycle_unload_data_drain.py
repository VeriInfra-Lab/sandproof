"""Current rendered host inputs for the existing drain oracle; no VM or SDK."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import _center_host_services as roles
import lifecycle_unload_data_drain as drain
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


if __name__ == '__main__':
    unittest.main()
