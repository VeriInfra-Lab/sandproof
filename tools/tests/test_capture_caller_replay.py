"""Caller diagnostic admission only; no extraction, Rust execution or Lean run."""
import copy
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

from tools import check_capture_caller_components as component


path = component.HERE/'caller_replay.py'
# The original CLI uses sibling imports. Restore the import state so this
# test's registration cannot select sibling modules for another component.
saved = {name: sys.modules.pop(name, None) for name in ('native', 'replay')}
sys.path.insert(0, str(path.parent))
try:
    caller = component.load('test_capture_caller_replay', path)
finally:
    sys.path.pop(0)
    for name, module in saved.items():
        sys.modules.pop(name, None)
        if module is not None:
            sys.modules[name] = module


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class CallerServiceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        names = ['Synthetic.'+name for name in ('first', 'second', 'third', 'fourth')]
        self.files = {'Proof.lean': names}
        (self.root/'Proof.lean').write_text(''.join(
            '#audit_controller_proof '+name+'\n' for name in names))
        digest = sha(self.root/'Proof.lean')
        self.row = dict(request=dict(tool='lean_diagnostic_messages',
            arguments=dict(file_path='Proof.lean'), expected_sha256=digest),
            revision=dict(file_sha256=digest), stale=False,
            result=dict(success=True, items=[dict(severity='info', line=i+1, column=1,
                message='Checked '+name+': 0 standard logical dependencies')
                for i, name in enumerate(names)]))

    def check(self, rows):
        return caller.check_service(self.root, self.files, '\n'.join(map(json.dumps, rows)), sha)

    def test_reverse_and_local_shift_preserve_the_original_response(self):
        for order in ((3, 2, 1, 0), (1, 2, 3, 0), (0, 2, 1, 3)):
            row = copy.deepcopy(self.row)
            row['result']['items'] = [row['result']['items'][i] for i in order]
            before = copy.deepcopy(row)
            with self.subTest(order=order):
                self.assertEqual(self.check([row]), [before])
                self.assertEqual(row, before)

    def test_columns_order_messages_but_source_order_still_requires_exact_names(self):
        row = copy.deepcopy(self.row)
        row['result']['items'][1].update(line=1, column=4)
        row['result']['items'].reverse()
        self.assertEqual(self.check([row]), [row])
        row = copy.deepcopy(self.row)
        first, second = row['result']['items'][:2]
        first['message'], second['message'] = second['message'], first['message']
        with self.assertRaisesRegex(ValueError, 'actual proof audit'):
            self.check([row])

    def test_missing_duplicate_extra_changed_or_malformed_audits_are_rejected(self):
        variants = []
        row = copy.deepcopy(self.row); row['result']['items'].pop(); variants.append(row)
        row = copy.deepcopy(self.row)
        row['result']['items'].append(copy.deepcopy(row['result']['items'][0]))
        variants.append(row)
        row = copy.deepcopy(self.row)
        row['result']['items'].append(dict(severity='info', line=5, column=1,
            message='Checked Synthetic.extra: 0 standard logical dependencies'))
        variants.append(row)
        for message in ('Checked Synthetic.changed: 0 standard logical dependencies',
                        'Checked Synthetic.second: 4 standard logical dependencies',
                        'Checked Synthetic.second: 0 standard logical dependencies trailing',
                        'Checked Synthetic.second: malformed'):
            row = copy.deepcopy(self.row); row['result']['items'][1]['message'] = message
            variants.append(row)
        for index, row in enumerate(variants):
            with self.subTest(variant=index), self.assertRaisesRegex(ValueError, 'actual proof audit'):
                self.check([row])

    def test_checked_audits_require_nonbool_positive_integer_coordinates(self):
        for axis in ('line', 'column'):
            for value in (None, 0, -1, True, False, 1.0, '1'):
                row = copy.deepcopy(self.row); row['result']['items'][1][axis] = value
                with self.subTest(axis=axis, value=value), \
                        self.assertRaisesRegex(ValueError, 'source position'):
                    self.check([row])
            row = copy.deepcopy(self.row); row['result']['items'][1].pop(axis)
            with self.subTest(axis=axis, missing=True), \
                    self.assertRaisesRegex(ValueError, 'source position'):
                self.check([row])

    def test_file_completeness_identity_and_diagnostics_remain_strict(self):
        for rows in ([], [self.row, self.row]):
            with self.subTest(count=len(rows)), self.assertRaises(ValueError):
                self.check(rows)
        variants = []
        for container, field, value in ((None, 'stale', True), ('result', 'success', False),
                ('result', 'partial', True), ('result', 'timed_out', True),
                ('result', 'failed_dependencies', ['missing']),
                ('revision', 'file_sha256', 'changed'), ('request', 'expected_sha256', 'changed')):
            row = copy.deepcopy(self.row)
            (row if container is None else row[container])[field] = value
            variants.append(row)
        row = copy.deepcopy(self.row); row['request']['arguments']['file_path'] = 'Other.lean'
        variants.append(row)
        for severity in ('error', 'warning'):
            row = copy.deepcopy(self.row)
            row['result']['items'].append(dict(severity=severity, message='diagnostic'))
            variants.append(row)
        for index, row in enumerate(variants):
            with self.subTest(variant=index), self.assertRaises(ValueError):
                self.check([row])


if __name__ == '__main__':
    unittest.main()
