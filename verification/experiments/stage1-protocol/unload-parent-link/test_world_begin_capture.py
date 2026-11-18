"""Admission checks for the selected original WorldTransaction::begin root."""
from copy import deepcopy
import json
from pathlib import Path
import tempfile
import unittest

import world_begin_capture as capture
from test_prepare_transaction_capture import call, method, name


class WorldBeginAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.cfg = capture.configuration(dict(opaque=['sandproof_controller_service', 'rusqlite']))
        cfg = deepcopy(self.cfg)
        high_water = {'Structured': {'body': [call(6), {'Str': 'input'}, {'Str': ''}]}}
        definitions = [
            (method(capture.TRANSACTION, 'begin'),
             {'Structured': {'body': [call(i) for i in range(1, 6)]}}),
            (method(capture.TRANSACTION, 'world'), 'Opaque'),
            (name(capture.CONTROLLER + '::checkpoint::prepare_write'), 'Opaque'),
            (method(capture.SQLITE_TRANSACTION, 'transaction_with_behavior'), 'Opaque'),
            (name(capture.HIGH_WATER), high_water),
            (method(capture.TRANSACTION, 'from_started_transaction'), 'Opaque'),
            (name(capture.CONTROLLER + '::allocators::read'), 'Opaque'),
        ]
        functions = [dict(def_id=i, body=body, item_meta=dict(
            name=path, opacity='Opaque' if body == 'Opaque' else 'Transparent',
        )) for i, (path, body) in enumerate(definitions)]
        self.raw = dict(has_errors=False, translated=dict(
            options=dict(skip_borrowck=False, no_typecheck=False, remove_unused_clauses=False,
                         monomorphize=False, exclude=[], start_from=cfg['roots'],
                         start_from_if_exists=cfg['retained_roots'],
                         include=cfg['include'], opaque=cfg['opaque']),
            target_information=[{'key': 'x86_64-unknown-linux-gnu'}],
            fun_decls=functions,
            type_decls=[dict(item_meta=dict(name=name(capture.BEHAVIOR), opacity='Transparent'),
                             kind={'Enum': [dict(name=x) for x in ('Deferred', 'Immediate', 'Exclusive')]})],
        ))
        self.mappings = [
            dict(def_id=0, rust_name=capture.RUST_METHOD, is_opaque=False,
                 source={'file': capture.SOURCE}, lean_file=capture.MODULE + '/Funs.lean',
                 lean_name=capture.MODULE + '.controller.transaction.WorldTransaction.begin'),
            dict(def_id=1, rust_name=capture.COMMITTED_WORLD),
            dict(def_id=3, rust_name=capture.SQLITE_BEGIN,
                 lean_name='rusqlite.Connection.transaction_with_behavior'),
        ]
        # Match the fixed generator's multiline definition and call layout.
        self.generated = (
            'def\n  controller.transaction.WorldTransaction.begin\n'
            '  (conn : rusqlite.Connection) := do\n'
            '  let world ← controller.transaction.CommittedWorld.world conn\n'
            '  let r ←\n    rusqlite.Connection.transaction_with_behavior\n'
            '      conn rusqlite.transaction.TransactionBehavior.Immediate\n'
            '  let entered ← controller.allocators.input_high_water r\n'
            '  controller.transaction.WorldTransaction.from_started_transaction r world entered\n'
        )

    def admit(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'actual.llbc').write_text(json.dumps(self.raw))
            (root / capture.MODULE).mkdir()
            (root / capture.MODULE / 'Funs.lean').write_text(self.generated)
            (root / 'translation.json').write_text(json.dumps(dict(functions=self.mappings)))
            return capture.admit(root / 'actual.llbc', root, self.cfg, 'x86_64-unknown-linux-gnu')

    def test_reports_original_begin_mode_and_allocator_scope(self):
        report = self.admit()
        self.assertEqual(report['calls']['from_started'], 5)
        self.assertEqual(report['sqlite_behavior'], 'Immediate')
        self.assertEqual((report['input_kind'], report['input_scope']), ('input', ''))

    def test_non_immediate_and_mention_outside_the_call_are_rejected(self):
        original = self.generated
        for mode in ('Deferred', 'Exclusive'):
            with self.subTest(mode=mode):
                self.generated = original.replace('.Immediate', '.' + mode) + (
                    '\ndef another (b := rusqlite.transaction.TransactionBehavior.Immediate) := b\n'
                )
                with self.assertRaisesRegex(ValueError, 'must pass TransactionBehavior.Immediate'):
                    self.admit()

    def test_allocator_kind_scope_and_original_callee_are_required(self):
        body = self.raw['translated']['fun_decls'][4]['body']['Structured']['body']
        for replacement, index in (({'Str': 'output'}, 1), ({'Str': 'foreign'}, 2), (call(2), 0)):
            with self.subTest(index=index):
                original, body[index] = body[index], replacement
                with self.assertRaisesRegex(ValueError, 'original allocator kind and empty scope'):
                    self.admit()
                body[index] = original

    def test_from_started_must_be_the_original_single_call(self):
        body = self.raw['translated']['fun_decls'][0]['body']['Structured']['body']
        body.append(call(5))
        with self.assertRaisesRegex(ValueError, 'call site differs: from_started'):
            self.admit()

    def test_other_world_method_cannot_supply_the_candidate(self):
        self.mappings[1]['rust_name'] = capture.COMMITTED_WORLD.replace('CommittedWorld', 'WorldTransaction')
        with self.assertRaisesRegex(ValueError, 'original CommittedWorld'):
            self.admit()

    def test_the_sqlite_receiver_must_be_the_original_connection(self):
        self.mappings[2]['rust_name'] = capture.SQLITE_BEGIN.replace('::Connection', '::OtherConnection')
        with self.assertRaisesRegex(ValueError, 'original rusqlite method'):
            self.admit()


if __name__ == '__main__':
    unittest.main()
