"""Checker regression on real captured IR/metadata; never business counterexamples."""
import argparse
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('native_readiness_admission_under_test', HERE/'admission.py')
admission = importlib.util.module_from_spec(spec)
spec.loader.exec_module(admission)


class AdmissionTests(unittest.TestCase):
    def test_actual_ir(self):
        self.assertIn('actual_method', admission.admit_ir(RAW))

    def test_actual_generated(self):
        self.assertIn('functions', admission.admit_generated(GENERATED, VARIANT))

    def test_incomplete_capture(self):
        value = copy.deepcopy(RAW)
        value['has_errors'] = True
        with self.assertRaises(ValueError):
            admission.admit_ir(value)

    def test_wrong_options(self):
        for key in ['skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses']:
            with self.subTest(key=key):
                value = copy.deepcopy(RAW)
                value['translated']['options'][key] = True
                with self.assertRaises(ValueError):
                    admission.admit_ir(value)
        for key in ['opaque','exclude','include','start_from']:
            with self.subTest(key=key):
                value = copy.deepcopy(RAW)
                value['translated']['options'][key] = ['not-the-original']
                with self.assertRaises(ValueError):
                    admission.admit_ir(value)

    def test_omitted_or_reordered_snapshot_fields(self):
        for operation in ['omit','reorder']:
            value = copy.deepcopy(RAW)
            snapshot = next(t for t in value['translated']['type_decls'] if t and
                t['item_meta']['name'][-1] == {'Ident':['NativeCreateSnapshot',0]})
            fields = snapshot['kind']['Struct']
            if operation == 'omit':
                fields.pop()
            else:
                fields[0],fields[1] = fields[1],fields[0]
            with self.assertRaises(ValueError):
                admission.admit_ir(value)

    def test_opaque_original_checker(self):
        value = copy.deepcopy(RAW)
        method = next(f for f in value['translated']['fun_decls'] if f and
            f['item_meta']['name'][-1] == {'Ident':['has_completed_original_execution',0]})
        method['item_meta']['opacity'] = 'Opaque'
        with self.assertRaises(ValueError):
            admission.admit_ir(value)

    def test_different_original_checker(self):
        value = copy.deepcopy(RAW)
        method = next(f for f in value['translated']['fun_decls'] if f and
            f['item_meta']['name'][-1] == {'Ident':['has_completed_original_execution',0]})
        method['item_meta']['name'][-1] = {'Ident':['another_checker',0]}
        with self.assertRaises(ValueError):
            admission.admit_ir(value)

    def test_generated_missing_method_or_extra_opacity(self):
        original = json.loads((GENERATED/'translation.json').read_text())
        for operation in ['omit','opaque','rename_external']:
            value = copy.deepcopy(original)
            if operation == 'omit':
                value['functions'] = [f for f in value['functions'] if not f['lean_name'].endswith(admission.METHOD)]
            elif operation == 'opaque':
                next(f for f in value['functions'] if f['lean_name'].endswith(admission.METHOD))['is_opaque'] = True
            else:
                next(f for f in value['functions'] if f['is_opaque'])['lean_name'] = 'unknown.primitive'
            with tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                (directory/'translation.json').write_text(json.dumps(value))
                with self.assertRaises(ValueError):
                    admission.admit_generated(directory, VARIANT)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc',type=Path,required=True)
    parser.add_argument('--generated',type=Path,required=True)
    parser.add_argument('--variant',choices=('correct','ignore-status'),required=True)
    args = parser.parse_args()
    RAW = json.loads(args.llbc.read_text())
    GENERATED,VARIANT = args.generated,args.variant
    unittest.main(argv=['admission_tests'])
