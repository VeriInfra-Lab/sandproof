"""Structured admission fixtures for the prepare capture, without Charon or Lean."""
from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest

import prepare_transaction_capture as capture


def name(parts):
    return [{'Ident': [part, 0]} for part in parts.split('::')]


def method(declaration_name, member):
    return name(declaration_name) + [{'Impl': {'Ty': {}}}, {'Ident': [member, 0]}]


def call(identifier):
    return {'Call': {'call': {'func': {'Regular': {'kind': {'Fun': identifier}}}}}}


def fixture(cfg):
    callees = [
        method(capture.TRANSACTION, 'begin'),
        method(capture.TRANSACTION, 'world'),
        method(capture.TRANSACTION, 'commit'),
        name(capture.INBOX + '::codec::load'),
        name(capture.INBOX + '::authenticated'),
        name(capture.WORK + '::codec::load'),
        name(capture.WORK + '::snapshot::current'),
        name(capture.WORK + '::codec::records_per_session'),
        name(capture.WORK + '::codec::save'),
    ]
    calls = [call(i) for i in range(1, len(callees) + 1)]
    functions = [dict(def_id=0, item_meta=dict(
        name=method(capture.WORK, 'prepare_lifecycle_source_work'), opacity='Transparent'),
        body={'Structured': {'body': calls}},
    )]
    functions.extend(dict(def_id=i, item_meta=dict(name=path, opacity='Opaque'), body='Opaque')
                     for i, path in enumerate(callees, start=1))
    types = [dict(item_meta=dict(name=name(path), opacity='Transparent'),
                  kind={'Struct': [dict(name=field) for field in fields]})
             for path, fields in capture.SHAPES.items()]
    return dict(has_errors=False, translated=dict(
        options=dict(skip_borrowck=False, no_typecheck=False, remove_unused_clauses=False,
                     monomorphize=False, exclude=[], start_from=cfg['roots'],
                     start_from_if_exists=cfg['retained_roots'],
                     include=cfg['include'], opaque=cfg['opaque']),
        target_information=[{'key': 'x86_64-unknown-linux-gnu'}],
        fun_decls=functions, type_decls=types,
    ))


class PrepareAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.cfg = capture.configuration(dict(opaque=[capture.CRATE, 'rusqlite']))
        self.raw = fixture(deepcopy(self.cfg))
        self.translated = self.raw['translated']
        self.lean = 'controller.lifecycle_inbox.source_work.ControllerJournal.prepare_lifecycle_source_work'
        self.mapping = dict(def_id=0, rust_name=capture.ROOT, is_opaque=False,
                            source=dict(file=capture.SOURCE),
                            lean_name=capture.MODULE + '.' + self.lean,
                            lean_file=capture.MODULE + '/Funs.lean')
        self.generated_definition = f'def {self.lean} (x : Unit) : Unit := x\n'

    def admit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'actual.llbc').write_text(json.dumps(self.raw))
            (root / capture.MODULE).mkdir()
            (root / capture.MODULE / 'Funs.lean').write_text(self.generated_definition)
            (root / 'translation.json').write_text(json.dumps(dict(functions=[self.mapping])))
            return capture.admit(root / 'actual.llbc', root, self.cfg, 'x86_64-unknown-linux-gnu')

    def test_actual_root_and_distinct_call_ids_are_reported(self):
        report = self.admit()
        self.assertEqual(report['actual_root'], capture.ROOT)
        self.assertNotEqual(report['calls']['load_inbox'], report['calls']['load_sources'])
        self.assertEqual(report['calls']['current'], 7)
        self.assertNotIn(capture.WORK + '::snapshot::current', self.cfg['include'])

    def test_borrow_check_bypass_is_rejected(self):
        self.translated['options']['skip_borrowck'] = True
        with self.assertRaisesRegex(ValueError, 'capture options'):
            self.admit()

    def test_typed_lookup_without_retaining_the_impl_is_rejected(self):
        self.translated['options']['start_from_if_exists'] = []
        with self.assertRaisesRegex(ValueError, 'capture options'):
            self.admit()

    def test_same_named_plain_function_is_not_the_actual_method(self):
        self.translated['fun_decls'][0]['item_meta']['name'].pop(-2)
        with self.assertRaisesRegex(ValueError, 'actual inherent method'):
            self.admit()

    def test_reading_the_other_load_or_calling_current_twice_is_rejected(self):
        body = self.translated['fun_decls'][0]['body']['Structured']['body']
        body[5] = call(4)
        with self.assertRaisesRegex(ValueError, 'load_inbox'):
            self.admit()
        body[5] = call(6)
        body.append(call(7))
        with self.assertRaisesRegex(ValueError, 'current'):
            self.admit()

    def test_generated_mapping_must_bind_the_actual_receiver(self):
        self.mapping['rust_name'] = capture.ROOT.replace('ControllerJournal', 'OtherJournal')
        with self.assertRaisesRegex(ValueError, 'generated prepare identity'):
            self.admit()

    def test_mapping_without_a_generated_definition_is_rejected(self):
        self.generated_definition = f'axiom {self.lean} : Unit -> Unit\n'
        with self.assertRaisesRegex(ValueError, 'generated prepare definition'):
            self.admit()

    def test_source_work_carrier_omission_is_rejected(self):
        self.translated['type_decls'][0]['kind']['Struct'].pop()
        with self.assertRaisesRegex(ValueError, 'original prepare carrier'):
            self.admit()


if __name__ == '__main__':
    unittest.main()
