"""Check registration limits without invoking a proof server or compiler."""
import json
from pathlib import Path
import tempfile
import unittest

import registration_preflight as preflight


class RegistrationPreflight(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name).resolve()
        self.files = {
            'Root.lean': b'import Nested.Child\n#audit_controller_proof Exact.root\n',
            'Nested/Child.lean': b'#audit_controller_proof Exact.child\n',
        }
        self.modules = {name: preflight.sha(data) for name, data in self.files.items()}
        self.view = dict(root='Root', modules=self.modules)
        self.plan = dict(schema=1, development_only=True, public_acceptance=False,
                         variant='correct', roots=['Root'], build_order=['Nested.Child', 'Root'],
                         modules={name: {'sha256': digest} for name, digest in self.modules.items()})
        for name, data in self.files.items():
            self.write(name, data)
        self.write_plan()

    def write(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def write_plan(self):
        self.write('plan.json', json.dumps(self.plan).encode())

    def test_missing_or_linked_freeze_refuses_before_project_checks(self):
        parent = preflight.PARENT / 'input-closure.json'
        with self.assertRaisesRegex(ValueError, 'missing or linked proof input'):
            preflight.collect(self.root, {})
        target = self.write('same-contents.json', b'{}')
        link = self.root / parent
        link.parent.mkdir(parents=True)
        link.symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'missing or linked proof input'):
            preflight.collect(self.root, {})

    def test_candidate_audits_must_match_frozen_template_and_declared_names(self):
        package = Path('candidate')
        template = package / 'Root.lean.in'
        self.write(template, self.files['Root.lean'])
        recipe = dict(root='Root', physical_files=2, own_modules=['Root.lean'],
                      audited_declarations=['Exact.root'])
        view = preflight.candidate_view(self.root, package, 'Root', recipe, self.modules)
        self.assertEqual(view['own_audits_by_file'], {'Root.lean': ['Exact.root']})
        with self.assertRaisesRegex(ValueError, 'declarations differ'):
            preflight.candidate_view(self.root, package, 'Root',
                                     dict(recipe, audited_declarations=['Exact.other']), self.modules)
        self.write(template, b'#audit_controller_proof Exact.other\n')
        with self.assertRaisesRegex(ValueError, 'changed proof input'):
            preflight.candidate_view(self.root, package, 'Root', recipe, self.modules)

    def test_physical_view_requires_all_frozen_files_and_collects_each_audit(self):
        self.write('.lake/build/generated.lean', b'cache file')
        self.assertEqual(preflight.check_project(self.root, 'correct', self.view, {}),
                         {'Root.lean': ['Exact.root'], 'Nested/Child.lean': ['Exact.child']})
        self.write('Unexpected.lean', b'an undeclared physical file')
        with self.assertRaisesRegex(ValueError, 'physical Lean file set differs'):
            preflight.check_project(self.root, 'correct', self.view, {})

    def test_shortened_plan_cannot_certify_a_shortened_physical_view(self):
        (self.root / 'Nested/Child.lean').unlink()
        del self.plan['modules']['Nested/Child.lean']
        self.plan['build_order'] = ['Root']
        self.write_plan()
        with self.assertRaisesRegex(ValueError, 'project plan differs'):
            preflight.check_project(self.root, 'correct', self.view, {})

    def test_variant_order_and_support_file_must_match(self):
        tool = 'tools/lean_session.py'
        support = {tool: preflight.sha(b'reviewed')}
        self.write(tool, b'reviewed')
        with self.assertRaisesRegex(ValueError, 'project plan differs'):
            preflight.check_project(self.root, 'attempt-zero', self.view, support)
        self.plan['build_order'].append('Root')
        self.write_plan()
        with self.assertRaisesRegex(ValueError, 'exact files once'):
            preflight.check_project(self.root, 'correct', self.view, support)
        self.plan['build_order'].pop()
        self.write_plan()
        self.write(tool, b'changed')
        with self.assertRaisesRegex(ValueError, 'changed proof input'):
            preflight.check_project(self.root, 'correct', self.view, support)


if __name__ == '__main__':
    unittest.main()
