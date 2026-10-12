"""Unprivileged pressure-observation regressions; no SDK, mount or VM.

Load only the probe's small diagnostic functions, not a second controller.
The actual ENOSPC and physical cleanup remain Linux/KVM acceptance requirements.
"""

import ast
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock


SYSTEM = Path(__file__).resolve().parents[2] / 'tests/system'


def load_definition(filename, name, **bindings):
    parsed = ast.parse((SYSTEM / filename).read_text())
    definition = next(node for node in parsed.body
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)) and node.name == name)
    namespace = dict(bindings)
    exec(compile(ast.Module(body=[definition], type_ignores=[]), filename, 'exec'), namespace)
    return namespace[name]


class StoragePressureObserversTests(unittest.TestCase):
    def setUp(self):
        self.node = dict(deployment_id='controlled-deployment', node_id='runtime', node_generation=1)
        self.services = SimpleNamespace(
            host_configs=[dict(inbox=dict(node=self.node))], planned_audit=Mock())

    def test_only_the_registered_original_host_zero_is_a_pre_admission_veto(self):
        first = dict(nodes=[dict(identity=self.node, free=dict(disk_bytes=4096)),
            dict(identity=self.node | dict(node_id='foreign'), free=dict(disk_bytes=0))])
        original = dict(nodes=[dict(identity=self.node, free=dict(disk_bytes=0))])
        self.services.planned_audit.side_effect = [first, original]
        clock = Mock(monotonic=Mock(return_value=0), sleep=Mock())
        observe = load_definition('server_disk_reserve.py', 'wait_disk_veto', time=clock)
        self.assertIs(observe(self.services), original)
        self.assertEqual(self.services.planned_audit.call_count, 2)
        clock.sleep.assert_called_once_with(.1)

    def test_missing_or_duplicate_original_node_is_not_a_low_disk_pass(self):
        observe = load_definition('server_disk_reserve.py', 'wait_disk_veto',
            time=Mock(monotonic=Mock(return_value=0)))
        for nodes in ([], [dict(identity=self.node), dict(identity=self.node)]):
            self.services.planned_audit.return_value = dict(nodes=nodes)
            with self.subTest(nodes=len(nodes)), self.assertRaises(AssertionError):
                observe(self.services)

    def test_unobserved_veto_times_out_without_creating_or_replaying_work(self):
        self.services.planned_audit.return_value = dict(
            nodes=[dict(identity=self.node, free=dict(disk_bytes=1))])
        clock = Mock(monotonic=Mock(side_effect=[0, 0, 31]), sleep=Mock())
        observe = load_definition('server_disk_reserve.py', 'wait_disk_veto', time=clock)
        with self.assertRaisesRegex(TimeoutError, 'no creation replay'):
            observe(self.services)
        self.services.planned_audit.assert_called_once_with('center')

    def test_filler_releases_only_its_created_objects_and_never_a_foreign_entry(self):
        filler_type = load_definition('host_enospc.py', 'Filler')
        with tempfile.TemporaryDirectory() as temporary:
            filler = filler_type(Path(temporary))
            filler.release()
            filler.path.mkdir()
            (filler.path / 'owned').touch()
            (filler.path / 'foreign').touch()
            filler.names, filler.created = ['owned'], True
            with self.assertRaises(OSError):
                filler.release()
            self.assertFalse((filler.path / 'owned').exists())
            self.assertTrue((filler.path / 'foreign').exists())
            self.assertTrue(filler.created, 'unknown cleanup cannot be called complete')

    def test_successful_filler_cleanup_is_idempotent_without_a_broad_delete(self):
        filler_type = load_definition('host_enospc.py', 'Filler')
        with tempfile.TemporaryDirectory() as temporary:
            filler = filler_type(Path(temporary))
            filler.path.write_bytes(b'own-small-file')
            filler.created = True
            filler.release()
            filler.release()
            self.assertFalse(filler.path.exists())
            self.assertFalse(filler.created)


if __name__ == '__main__':
    unittest.main()
