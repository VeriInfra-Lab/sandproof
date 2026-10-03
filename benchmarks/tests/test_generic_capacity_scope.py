"""Cohorts share admission budgets without broadening execution or source modes."""
import copy
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).resolve().parents[2]
SYSTEM = ROOT / 'tests/system'
sys.path.insert(0, str(SYSTEM))
try:
    from harbor_generic_chain import select_cohort
finally:
    sys.path.remove(str(SYSTEM))

from tools.benchmark_tasks import InvalidInput
from tools.generic_capacity import GIB, MIB, plan
from tools.template_acceptance import budgets


class CapacityScopeTests(unittest.TestCase):
    def setUp(self):
        self.tasks = json.loads((ROOT / 'benchmarks/terminal-bench-2.0.lock.json').read_text())['tasks']

    def args(self, names, **overrides):
        return SimpleNamespace(**({'task': names, 'all': False, 'capacity_scope': 'all',
            'oracle_mode': 'original', 'source_kind': 'oci', 'resource_pressure': False,
            'repetitions': 1} | overrides))

    def configuration(self, tasks, repetitions=1, job_limit=None):
        capacity = plan(tasks, repetitions, 64 * GIB, 64 * MIB,
                        input_bytes=10 * GIB, expanded_bytes=32 * GIB, job_limit=job_limit)
        return capacity, budgets(capacity, 64 * GIB, 64 * MIB)

    def test_modes_and_task_shapes_share_the_full_configuration(self):
        original = copy.deepcopy(self.tasks)
        expected = self.configuration(self.tasks)
        for names, mode, source in (
                (['regex-log'], 'original', 'oci'),
                (['torch-tensor-parallelism'], 'repaired', 'oci'),
                (['mteb-retrieve', 'mteb-leaderboard'], 'repaired', 'oci-pip-cache-pruned')):
            with self.subTest(names=names):
                args = self.args(names, oracle_mode=mode, source_kind=source)
                selected, actual_mode, entries, planning = select_cohort(args, self.tasks)
                self.assertEqual(selected, set(names))
                self.assertEqual(actual_mode, mode)
                self.assertEqual(entries, [t for t in original if t['name'] in names])
                self.assertEqual(self.configuration(planning), expected)
                self.assertFalse(args.all)
                self.assertEqual(args.task, names)
        self.assertEqual(self.tasks, original)

    def test_original_selected_capacity_and_two_repetitions_are_preserved(self):
        args = self.args(['regex-log'], capacity_scope='selected', repetitions=2)
        selected, _, entries, planning = select_cohort(args, self.tasks)
        self.assertEqual(selected, {'regex-log'})
        self.assertEqual(planning, entries)
        self.assertEqual(self.configuration(planning, 2)[0]['jobs'], 2)
        args.capacity_scope = 'all'
        planning = select_cohort(args, self.tasks)[3]
        self.assertEqual(self.configuration(planning, 2)[0]['jobs'], 178)

    def test_full_capacity_does_not_relax_source_or_oracle_validation(self):
        for args in (
                self.args(['regex-log'], oracle_mode='repaired'),
                self.args(['mteb-retrieve', 'regex-log'], source_kind='oci-pip-cache-pruned'),
                self.args(None, all=True, source_kind='oci-pip-cache-pruned')):
            with self.subTest(args=args), self.assertRaises(InvalidInput):
                select_cohort(args, self.tasks)
        for names in ([], ['unknown-task']):
            with self.subTest(names=names), self.assertRaises(RuntimeError):
                select_cohort(self.args(names), self.tasks)
        with self.assertRaises(ValueError):
            select_cohort(self.args(['regex-log'], capacity_scope='invalid'), self.tasks)

    def test_pressure_uses_selected_shapes_with_the_same_full_capacity(self):
        names = ['regex-log', 'polyglot-c-py']
        args = self.args(names, resource_pressure=True)
        selected, _, entries, planning = select_cohort(args, self.tasks)
        self.assertEqual(selected, set(names))
        self.assertEqual({e['name'] for e in entries}, set(names))
        self.assertEqual(self.configuration(planning), self.configuration(self.tasks))
        args.capacity_scope = 'selected'
        self.assertEqual(len(select_cohort(args, self.tasks)[3]), 4)
        args.repetitions = 2
        with self.assertRaises(ValueError):
            select_cohort(args, self.tasks)
        args.repetitions, args.capacity_scope = 1, 'all'
        with self.assertRaises(ValueError):
            select_cohort(args, entries)

    def test_fixed_cohort_limit_covers_every_locked_task_without_reducing_shapes(self):
        expected = self.configuration(self.tasks, job_limit=4)
        visited = []
        for offset in range(0, len(self.tasks), 4):
            names = [task['name'] for task in self.tasks[offset:offset + 4]]
            args = self.args(names, capacity_jobs=4)
            selected, mode, entries, planning = select_cohort(args, self.tasks)
            self.assertEqual(mode, 'original')
            self.assertEqual(selected, set(names))
            self.assertEqual(entries, self.tasks[offset:offset + 4])
            self.assertEqual(self.configuration(planning, job_limit=4), expected)
            visited.extend(task['name'] for task in entries)
        self.assertEqual(visited, [task['name'] for task in self.tasks])
        full = self.configuration(self.tasks)
        for field in ('runtime', 'builder', 'runtime_compute', 'builder_compute', 'builder_shape'):
            self.assertEqual(expected[1][field], full[1][field])
        for field in ('max_cpu', 'max_memory_mib', 'max_disk_mib', 'timeout_seconds',
                      'max_input_bytes', 'max_expanded_bytes'):
            self.assertEqual(expected[1]['limits'][field], full[1]['limits'][field])
        self.assertEqual(expected[1]['registry']['max_templates_global'], 4)
        self.assertEqual(expected[1]['limits']['max_jobs'], 4)

    def test_overflow_and_invalid_cohort_limits_fail_before_dispatch(self):
        for args in (
                self.args(['regex-log'], capacity_jobs=0),
                self.args(['regex-log'], capacity_jobs=True),
                self.args(['regex-log'], capacity_jobs=1.5),
                self.args(['regex-log'], capacity_jobs=90),
                self.args(['regex-log'], capacity_jobs=4, capacity_scope='selected'),
                self.args(None, all=True, capacity_jobs=4),
                self.args(['regex-log'], repetitions=2, capacity_jobs=1),
                self.args(['regex-log', 'polyglot-c-py'], resource_pressure=True, capacity_jobs=3)):
            with self.subTest(args=args), self.assertRaises(ValueError):
                select_cohort(args, self.tasks)
        selected, _, _, planning = select_cohort(
            self.args(['regex-log'], repetitions=2, capacity_jobs=2), self.tasks)
        self.assertEqual(selected, {'regex-log'})
        self.assertEqual(self.configuration(planning, 2, job_limit=2)[0]['jobs'], 2)
        args = self.args(['regex-log', 'polyglot-c-py'], resource_pressure=True, capacity_jobs=4)
        self.assertEqual(len(select_cohort(args, self.tasks)[3]), 89)


if __name__ == '__main__':
    unittest.main()
