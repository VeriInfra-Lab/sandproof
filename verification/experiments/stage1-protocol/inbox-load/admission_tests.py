"""Exercise refusal of truncated, opaque, or misdirected actual extraction evidence."""
import argparse
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('inbox_admission_formal',HERE/'formal.py')
formal=importlib.util.module_from_spec(spec);spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    raw=None
    translation=None
    variant='correct'

    def ir(self,raw):
        return formal.admit_ir(raw,primitive=self.variant=='primitive')

    def generated(self,value):
        with tempfile.TemporaryDirectory(prefix='inbox-admission-') as name:
            directory=Path(name)
            (directory/'translation.json').write_text(json.dumps(value))
            return formal.admit_generated(directory,self.variant)

    def test_actual_metadata(self):
        self.ir(self.raw);self.generated(self.translation)

    def test_every_generated_body_is_required(self):
        # Each leaf/loop is part of the reviewed complete view, including branches unused by witnesses.
        for index,function in enumerate(self.translation['functions']):
            if function['is_opaque']:continue
            with self.subTest(function=function['lean_name']):
                changed=deepcopy(self.translation);del changed['functions'][index]
                with self.assertRaisesRegex(ValueError,'root/body missing'):self.generated(changed)

    def test_business_opacity_is_rejected(self):
        changed=deepcopy(self.translation)
        function=next(f for f in changed['functions']if not f['is_opaque'])
        function['is_opaque']=True
        with self.assertRaisesRegex(ValueError,'root/body missing'):self.generated(changed)

    def test_new_external_model_is_rejected(self):
        changed=deepcopy(self.translation)
        changed['functions'].append(dict(lean_name='trusted_inbox_transaction',is_opaque=True))
        with self.assertRaisesRegex(ValueError,'business opacity'):self.generated(changed)

    def test_every_external_binding_is_required(self):
        for index,function in enumerate(self.translation['functions']):
            if not function['is_opaque']:continue
            with self.subTest(function=function['lean_name']):
                changed=deepcopy(self.translation);del changed['functions'][index]
                with self.assertRaisesRegex(ValueError,'business opacity'):self.generated(changed)

    def test_duplicate_body_is_rejected(self):
        changed=deepcopy(self.translation)
        changed['functions'].append(deepcopy(next(f for f in changed['functions']if not f['is_opaque'])))
        with self.assertRaisesRegex(ValueError,'root/body missing'):self.generated(changed)

    def test_wrong_generated_file_is_rejected(self):
        changed=deepcopy(self.translation)
        next(f for f in changed['functions']if not f['is_opaque'])['lean_file']='Template.lean'
        with self.assertRaisesRegex(ValueError,'unreviewed file'):self.generated(changed)

    def test_incomplete_charon_is_rejected(self):
        changed=deepcopy(self.raw);changed['has_errors']=True
        with self.assertRaisesRegex(ValueError,'incomplete Charon'):self.ir(changed)

    def test_weakened_options_are_rejected(self):
        for option in ('skip_borrowck','no_typecheck','monomorphize'):
            with self.subTest(option=option):
                changed=deepcopy(self.raw);changed['translated']['options'][option]=True
                with self.assertRaisesRegex(ValueError,'options'):self.ir(changed)

    def test_unreviewed_selection_is_rejected(self):
        for option in ('start_from','include','opaque','exclude'):
            with self.subTest(option=option):
                changed=deepcopy(self.raw);changed['translated']['options'][option].append('unreviewed')
                with self.assertRaisesRegex(ValueError,'options'):self.ir(changed)

    def test_wrong_target_is_rejected(self):
        changed=deepcopy(self.raw);changed['translated']['target_information'][0]['key']='different-target'
        with self.assertRaisesRegex(ValueError,'target'):self.ir(changed)

    def test_missing_or_opaque_forwarder_is_rejected(self):
        names=(['proof_standard_graphic_bytes','proof_result_is_error']if self.variant=='primitive'
               else['proof_inbox_validate_using','proof_inbox_load_using'])
        for name in names:
            for attribute,value in [('name',[{'Ident':['different',0]}]),('opacity','Opaque')]:
                with self.subTest(name=name,attribute=attribute):
                    changed=deepcopy(self.raw)
                    function=next(f for f in changed['translated']['fun_decls']if f and f['item_meta']['name'][-1]=={'Ident':[name,0]})
                    function['item_meta'][attribute]=value
                    with self.assertRaisesRegex(ValueError,'forwarding root'):self.ir(changed)

    def test_wrong_actual_method_is_rejected(self):
        changed=deepcopy(self.raw)
        name='is_err'if self.variant=='primitive'else'validate_using'
        function=next(f for f in changed['translated']['fun_decls']if f and f['item_meta']['name'][-1]=={'Ident':[name,0]})
        function['item_meta']['name'][-1]={'Ident':['different',0]}
        with self.assertRaisesRegex(ValueError,'actual transparent'):self.ir(changed)

    def test_opaque_actual_method_is_rejected(self):
        changed=deepcopy(self.raw)
        name='is_err'if self.variant=='primitive'else'validate_using'
        function=next(f for f in changed['translated']['fun_decls']if f and f['item_meta']['name'][-1]=={'Ident':[name,0]})
        function['item_meta']['opacity']='Opaque'
        with self.assertRaisesRegex(ValueError,'actual transparent'):self.ir(changed)

    def test_load_callee_is_required_and_transparent(self):
        if self.variant=='primitive':
            return
        for attribute,value in [('name',[{'Ident':['different',0]}]),('opacity','Opaque')]:
            with self.subTest(attribute=attribute):
                changed=deepcopy(self.raw)
                function=next(f for f in changed['translated']['fun_decls']if f and
                    f['item_meta']['name'][-1]=={'Ident':['load_using',0]})
                function['item_meta'][attribute]=value
                with self.assertRaisesRegex(ValueError,'actual transparent'):self.ir(changed)

    def test_duplicate_forwarder_is_rejected(self):
        name = 'proof_result_is_error' if self.variant == 'primitive' else 'proof_inbox_load_using'
        changed = deepcopy(self.raw)
        functions = changed['translated']['fun_decls']
        function = next(f for f in functions if f and f['item_meta']['name'][-1] == {'Ident': [name, 0]})
        functions.append(deepcopy(function))
        with self.assertRaisesRegex(ValueError, 'forwarding root'):
            self.ir(changed)

    def test_load_forwarder_cannot_add_a_second_call(self):
        name = 'proof_result_is_error' if self.variant == 'primitive' else 'proof_inbox_load_using'
        changed = deepcopy(self.raw)
        function = next(f for f in changed['translated']['fun_decls'] if f and
                        f['item_meta']['name'][-1] == {'Ident': [name, 0]})
        # The admission walk must inspect the entire body, including nested calls.
        function['body']['additional_call'] = deepcopy(function['body'])
        with self.assertRaisesRegex(ValueError, 'exactly one actual call'):
            self.ir(changed)

    def test_result_tags_are_admitted(self):
        formal.library_binding.result_tags(self.raw)

    def test_native_literal_default_is_rejected(self):
        binding = formal.library_binding
        root = HERE.parents[3]
        with tempfile.TemporaryDirectory(prefix='save-literal-admission-') as name:
            project = Path(name)
            source = project/binding.SOURCE
            source.parent.mkdir(parents=True)
            source.write_bytes((root/binding.SOURCE).read_bytes())
            literal = project/binding.HERE/'LiteralBinding.lean.in'
            literal.parent.mkdir(parents=True)
            literal.write_bytes((root/binding.HERE/'LiteralBinding.lean.in').read_bytes())
            binding.check_literal(project)
            old = 'by rw [U32.max_eq]; decide'
            if literal.read_text().count(old) != 1:
                raise ValueError('unexpected literal length proof')
            literal.write_text(literal.read_text().replace(old,'by decide +native'))
            with self.assertRaisesRegex(ValueError,'changed existing literal adapter'):
                binding.check_literal(project)

    def test_changed_result_tags_are_rejected(self):
        changed=deepcopy(self.raw)
        name=[{'Ident':[x,0]}for x in ['core','result','Result']]
        declaration=next(t for t in changed['translated']['type_decls']if t and t['item_meta']['name']==name)
        declaration['kind']['Enum'][1]['discriminant']={'Signed':['Isize','0']}
        with self.assertRaisesRegex(ValueError,'Result logical tags'):formal.library_binding.result_tags(changed)

    def test_changed_existing_model_is_rejected(self):
        with self.assertRaisesRegex(ValueError,'nonunique existing model'):
            formal.library_binding.fragment('def different := 0\n\n','alloc.vec.Vec.is_empty')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc',type=Path,required=True)
    parser.add_argument('--generated',type=Path,required=True)
    parser.add_argument('--variant',choices=['correct','primitive',*formal.native.preparation.FAILURES],required=True)
    args=parser.parse_args()
    Admission.raw=json.loads(args.llbc.read_text())
    Admission.translation=json.loads((args.generated/'translation.json').read_text())
    Admission.variant=args.variant
    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Admission))
    raise SystemExit(not result.wasSuccessful())
