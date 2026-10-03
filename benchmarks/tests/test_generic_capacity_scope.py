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

    def configuration(self, tasks, repetitions=1):
        capacity = plan(tasks, repetitions, 64 * GIB, 64 * MIB,
                        input_bytes=10 * GIB, expanded_bytes=32 * GIB)
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


if __name__ == '__main__':
    unittest.main()
