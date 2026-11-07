"""Admission regressions on complete original LLBC, never synthetic success data."""
import copy
import json
import os
from pathlib import Path
import unittest

from binding import report
from negative import validate_native_commands


class DecoderAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = json.loads(Path(os.environ['SANDPROOF_PARSER_BINDING_LLBC']).read_text())

    def test_complete_original_dispatch(self):
        value = report(self.raw)
        self.assertEqual(value['operation_unknown_visitors'], 10)
        self.assertEqual(len(value['bodies']), 26)

    def test_incomplete_capture_is_not_a_rejection(self):
        changed = copy.deepcopy(self.raw)
        changed['has_errors'] = True
        with self.assertRaisesRegex(ValueError, 'incomplete Charon input'):
            report(changed)

    def test_missing_original_operation_visitor_is_rejected(self):
        changed = copy.deepcopy(self.raw)
        value = report(changed)
        operation = value['bodies']['sandproof_protocol::lifecycle::request::LifecycleOperation']
        victim = next(v['name'] for v in operation['derived_subtree']
                      if any(c['callee'] == 'serde_core::de::Error::unknown_field' for c in v['calls']))
        for index, f in enumerate(changed['translated']['fun_decls']):
            if f and f['item_meta']['name'] == victim:
                changed['translated']['fun_decls'][index] = None
                break
        with self.assertRaisesRegex(ValueError, 'operation unknown-field rejection visitor missing'):
            report(changed)

    def test_opaque_own_decoder_is_rejected(self):
        changed = copy.deepcopy(self.raw)
        operation = report(changed)['bodies']['sandproof_protocol::lifecycle::request::LifecycleOperation']
        function = next(f for f in changed['translated']['fun_decls']
                        if f and f['def_id'] == operation['function_id'])
        function['item_meta']['opacity'] = 'Opaque'
        with self.assertRaisesRegex(ValueError, 'original own Deserialize body missing'):
            report(changed)


class NativeReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.receipt = json.loads((Path(os.environ['SANDPROOF_PARSER_BINDING_NATIVE']) / 'result.json').read_text())

    def test_actual_complete_receipt(self):
        validate_native_commands(self.receipt)

    def test_missing_original_or_restored_log_is_rejected(self):
        for index in (0, 1, 4, 5):
            partial = copy.deepcopy(self.receipt)
            del partial['commands'][index]
            with self.assertRaisesRegex(ValueError, 'all six'):
                validate_native_commands(partial)

    def test_duplicate_command_does_not_replace_the_missing_log(self):
        partial = copy.deepcopy(self.receipt)
        partial['commands'][5] = copy.deepcopy(partial['commands'][3])
        with self.assertRaisesRegex(ValueError, 'all six'):
            validate_native_commands(partial)
