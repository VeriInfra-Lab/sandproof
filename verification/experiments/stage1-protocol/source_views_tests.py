"""Reject business replacement and missing mutations in adopted-source views."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[3]
AREA = Path('verification/experiments/stage1-protocol')
PACKAGES = (
    'inbox-entry-validation', 'inbox-retired-identities', 'inbox-origin-selection',
    'inbox-config', 'inbox-config-validation', 'terminal-binding', 'retention-lane',
    'native-retirement', 'session-shape', 'inbox-validation', 'inbox-save',
    'inbox-record-validation', 'inbox-load',
)


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


class AdoptedSourceViews(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        (ROOT / 'artifacts').mkdir(exist_ok=True)
        cls.temporary = tempfile.TemporaryDirectory(prefix='v3-source-views-', dir=ROOT / 'artifacts')
        cls.project = Path(cls.temporary.name)
        shutil.copytree(ROOT / AREA, cls.project / AREA,
                        ignore=shutil.ignore_patterns('__pycache__'))
        names = {str(AREA / 'inbox-record/lib.rs')}
        for package in PACKAGES:
            inventory = json.loads((ROOT / AREA / package / 'inventory.json').read_text())
            names.update(inventory['production_inputs'])
            names.update(inventory.get('original_inputs', {}))
        cls.original = {name: (ROOT / name).read_bytes() for name in names}
        cls.views = module('tested_source_views', ROOT / AREA / 'source_views.py')

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def restore(self):
        for name, data in self.original.items():
            path = self.project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)

    def test_all_correct_and_mutated_views_report_every_source_change(self):
        for package in PACKAGES:
            preparation = module('tested_' + package.replace('-', '_'),
                                 ROOT / AREA / package / 'prepare.py')
            for variant in preparation.VARIANTS:
                with self.subTest(package=package, variant=variant):
                    self.restore()
                    changed, metadata = preparation.prepare(self.project, variant)
                    observed = {name: self.views.sha(self.project / name)
                                for name, data in self.original.items()
                                if (self.project / name).read_bytes() != data}
                    self.assertEqual(changed, observed)
                    if variant in ('original', 'candidate', 'restored'):
                        self.assertIsNone(metadata['mutation'])
                        for name in changed:
                            self.assertTrue((self.project / name).read_bytes().startswith(
                                self.original[name].rstrip()),
                                'correct view replaced production: ' + name)
                    else:
                        self.assertTrue(metadata['isolated_production_mutation'])
                        self.assertIn(metadata['mutation']['path'], changed)

    def test_changed_source_is_rejected_before_roots_are_added(self):
        self.restore()
        name = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
        with (self.project / name).open('a') as stream:
            stream.write('\n// changed input\n')
        with self.assertRaisesRegex(ValueError, 'adopted source input differs'):
            self.views.inputs(self.project, AREA / 'inbox-load')

    def test_mutation_requires_a_unique_expression_in_its_named_function(self):
        path = self.project / 'mutation.rs'
        path.write_text('fn one() { old(); old(); }\nfn two() { other(); }\n')
        with self.assertRaisesRegex(ValueError, 'nonunique actual mutation expression'):
            self.views.edit(self.project, 'mutation.rs', [('old()', 'new()')], 'fn one(')
        with self.assertRaisesRegex(ValueError, 'nonunique actual mutation expression'):
            self.views.edit(self.project, 'mutation.rs', [('old()', 'new()')], 'fn two(')
        self.assertNotIn('new()', path.read_text())


if __name__ == '__main__':
    os.umask(0o077)
    unittest.main()
