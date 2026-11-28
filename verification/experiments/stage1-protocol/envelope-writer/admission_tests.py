"""Source/AST admission regressions, separate from the real Rust fault experiment."""
from pathlib import Path
import argparse
import copy
import hashlib
import json
import unittest
import binding


class OriginalCallAdmission(unittest.TestCase):
    def root(self, raw):
        return next(d for d in raw['translated']['fun_decls'] if d and d['item_meta']['name']==[
            {'Ident':['sandproof_envelope_writer_check',0]},{'Ident':['accepted_bounded',0]}])

    def statements(self, raw):
        return self.root(raw)['body']['Structured']['body']['statements']

    def test_original_complete_calls(self):
        self.assertEqual(set(binding.snapshot(RAW,SOURCE_SHA)['roots']),{'accepted_bounded','recorded_bounded'})

    def test_changed_original_limit_is_rejected(self):
        raw=copy.deepcopy(RAW)
        self.statements(raw)[4]['kind']['Assign'][1]['Use'][0]['Copy']['kind']={'Local':1}
        with self.assertRaises(ValueError):binding.snapshot(raw,SOURCE_SHA)

    def test_omitted_complete_value_is_rejected(self):
        raw=copy.deepcopy(RAW)
        self.statements(raw)[5]['kind']['Call']['call']['args'][0]['Move']['kind']={'Local':2}
        with self.assertRaises(ValueError):binding.snapshot(raw,SOURCE_SHA)

    def test_missing_real_dictionary_is_rejected(self):
        raw=copy.deepcopy(RAW)
        self.statements(raw)[5]['kind']['Call']['call']['func']['Regular']['generics']['trait_refs']=[]
        with self.assertRaises(ValueError):binding.snapshot(raw,SOURCE_SHA)

    def test_changed_unwind_is_rejected(self):
        raw=copy.deepcopy(RAW)
        self.statements(raw)[5]['kind']['Call']['on_unwind']['statements'][-1]['kind']='Return'
        with self.assertRaises(ValueError):binding.snapshot(raw,SOURCE_SHA)

    def test_borrow_check_bypass_is_rejected(self):
        raw=copy.deepcopy(RAW);raw['translated']['options']['skip_borrowck']=True
        with self.assertRaises(ValueError):binding.snapshot(raw,SOURCE_SHA)

    def test_old_embedded_source_is_rejected(self):
        with self.assertRaises(ValueError):binding.snapshot(RAW,'0'*64)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--llbc',type=Path,required=True)
    args=parser.parse_args();RAW=json.loads(args.llbc.read_text())
    SOURCE_SHA=hashlib.sha256(Path(__file__).with_name('roots.rs.in').read_bytes()).hexdigest()
    unittest.main(argv=['admission_tests'],verbosity=2)
