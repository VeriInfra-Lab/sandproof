"""Reject missing original roots, hidden effects and falsely expanded query claims."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools.check_loop_callbacks_contract import (ROOT,DEST,LLBC,CRATE,METHOD,
    check,check_extraction,digest,llbc_digest)


class SharedQueryExtractionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.llbc=json.loads((ROOT/LLBC).read_text())
        cls.inventory=json.loads((ROOT/DEST/'translation.json').read_text())
        cls.env=json.loads((ROOT/'verification/loop-callbacks-environment.json').read_text())

    def admit(self,llbc=None,inventory=None):
        return check_extraction(ROOT,self.llbc if llbc is None else llbc,
            self.inventory if inventory is None else inventory,self.env)

    def test_actual_original_roots_are_complete(self):
        self.assertEqual(self.admit(),dict(original_functions=6,loop_artifacts=2,custom_drop_nodes=0))

    def test_missing_success_is_rejected(self):
        for bad in [True,None]:
            value=copy.deepcopy(self.llbc);value['has_errors']=bad
            with self.assertRaisesRegex(ValueError,'successful'):
                self.admit(llbc=value)

    def test_no_root_can_disappear(self):
        for index in range(len(self.inventory['functions'])):
            value=copy.deepcopy(self.inventory);del value['functions'][index]
            with self.assertRaises(ValueError):self.admit(inventory=value)

    def test_duplicate_inventory_is_rejected(self):
        value=copy.deepcopy(self.inventory);value['functions'].append(value['functions'][0])
        with self.assertRaisesRegex(ValueError,'duplicate'):self.admit(inventory=value)

    def test_loop_roles_cannot_be_collapsed(self):
        value=copy.deepcopy(self.inventory)
        for function in value['functions']:
            if 'loop' in function:function['loop']['is_body']=True
        with self.assertRaisesRegex(ValueError,'loop/body'):self.admit(inventory=value)

    def test_loop_cannot_bind_to_another_function(self):
        value=copy.deepcopy(self.inventory)
        next(f for f in value['functions'] if 'loop' in f)['parent_lean_name']='wrong.parent'
        with self.assertRaisesRegex(ValueError,'loop/body'):self.admit(inventory=value)

    def test_owned_query_cannot_be_an_opaque_assumption(self):
        for name in [METHOD+'get',METHOD+'contains_key',CRATE+'::lookup']:
            value=copy.deepcopy(self.inventory)
            next(f for f in value['functions'] if f['rust_name']==name)['is_opaque']=True
            with self.assertRaisesRegex(ValueError,'real structured body'):self.admit(inventory=value)

    def test_actual_owned_body_is_required(self):
        value=copy.deepcopy(self.llbc)
        ident=next(f for f in self.inventory['functions'] if f['rust_name']==METHOD+'get')['def_id']
        next(f for f in value['translated']['fun_decls'] if f and f['def_id']==ident)['body']='Opaque'
        with self.assertRaises(ValueError):self.admit(llbc=value)

    def test_ordered_original_bodies_cannot_be_dropped(self):
        value=copy.deepcopy(self.llbc);value['translated']['ordered_decls']=[]
        with self.assertRaisesRegex(ValueError,'structured body'):self.admit(llbc=value)

    def test_new_observable_drop_is_not_erased(self):
        value=copy.deepcopy(self.llbc)
        ident=next(f for f in self.inventory['functions'] if f['rust_name']==METHOD+'get')['def_id']
        body=next(f for f in value['translated']['fun_decls'] if f and f['def_id']==ident)['body']
        body['Structured']['unreviewed_effect']={'Drop':{'resource':'permit'}}
        with self.assertRaisesRegex(ValueError,'destruction/error'):self.admit(llbc=value)

    def test_frontend_safety_checks_cannot_be_disabled(self):
        for flag in ['skip_borrowck','no_typecheck','no_normalize','no_reorder_decls']:
            value=copy.deepcopy(self.llbc);value['translated']['options'][flag]=True
            with self.assertRaisesRegex(ValueError,'compiler checks'):self.admit(llbc=value)

    def test_pointer_target_cannot_change(self):
        value=copy.deepcopy(self.llbc)
        value['translated']['target_information'][0]['value']['target_pointer_size']=4
        with self.assertRaisesRegex(ValueError,'pointer representation'):self.admit(llbc=value)

    def test_unreviewed_library_signature_is_rejected(self):
        value=copy.deepcopy(self.llbc)
        foreign=next(f for f in value['translated']['fun_decls'] if f and f['body']=='Opaque')
        foreign['signature']['unreviewed_input']='resource-bearing'
        with self.assertRaisesRegex(ValueError,'library function/signature'):self.admit(llbc=value)

    def test_unexpected_generated_function_file_is_rejected(self):
        value=copy.deepcopy(self.inventory);value['functions'][0]['lean_file']='missing.lean'
        with self.assertRaisesRegex(ValueError,'structured body'):self.admit(inventory=value)

    def test_store_type_cannot_disappear(self):
        value=copy.deepcopy(self.inventory);value['types']=[]
        with self.assertRaisesRegex(ValueError,'Store representation'):self.admit(inventory=value)

    def test_complete_ast_digest_retains_semantic_fields(self):
        value=copy.deepcopy(self.llbc)
        value['translated']['options']['precise_drops']=not value['translated']['options']['precise_drops']
        self.assertNotEqual(llbc_digest(value),llbc_digest(self.llbc))

    def test_only_short_names_map_order_is_canonicalized(self):
        value=copy.deepcopy(self.llbc);value['translated']['short_names'].reverse()
        self.assertEqual(llbc_digest(value),llbc_digest(self.llbc))


class SharedQueryFrozenInputsTests(unittest.TestCase):
    def setUp(self):
        tmp=tempfile.TemporaryDirectory();self.addCleanup(tmp.cleanup);self.root=Path(tmp.name)/'source'
        self.env=json.loads((ROOT/'verification/loop-callbacks-environment.json').read_text())
        for name in self.env['inputs']:
            destination=self.root/name;destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(ROOT/name,destination)
        self.seal()

    def seal(self):
        (self.root/'verification/loop-callbacks-inputs.sha256').write_text(''.join(
            digest(self.root/name)+'  '+name+'\n' for name in sorted(self.env['inputs'])))

    def test_actual_frozen_module_is_admitted(self):
        self.assertEqual(len(check(self.root)['theorems']),21)

    def test_changed_source_is_not_old_proof_evidence(self):
        p=self.root/'controller-core/src/store.rs';p.write_text(p.read_text()+'\n// changed\n')
        with self.assertRaisesRegex(ValueError,'Frozen shared query input changed'):check(self.root)

    def test_missing_audit_is_rejected_even_after_resealing(self):
        p=self.root/'SandboxModel/Foundations/LoopCallbacks/Audit.lean'
        p.write_text('\n'.join(p.read_text().splitlines()[:-1])+'\n');self.seal()
        with self.assertRaisesRegex(ValueError,'theorem/audit'):check(self.root)

    def test_caller_business_cannot_be_reclassified_as_trusted(self):
        p=self.root/'verification/experiments/loop-callbacks/source-map.json';value=json.loads(p.read_text())
        row=next(x for x in value['use_points'] if x['id']=='checkpoint.duplicate-row-decision')
        row['status']='conditional-mechanical';row['provider']='B4';p.write_text(json.dumps(value));self.seal()
        with self.assertRaisesRegex(ValueError,'business obligation'):check(self.root)

    def test_new_template_or_external_file_is_rejected(self):
        p=self.root/DEST/'SandproofLoopCallbacks/FunsExternal.lean';p.write_text('-- unreviewed\n')
        with self.assertRaisesRegex(ValueError,'generated query file/template'):check(self.root)

    def test_placeholder_is_rejected_even_after_resealing(self):
        p=self.root/'SandboxModel/Foundations/LoopCallbacks/Examples.lean'
        p.write_text(p.read_text()+'\naxiom forged : True\n');self.seal()
        with self.assertRaisesRegex(ValueError,'placeholder'):check(self.root)

    def test_duplicate_frozen_input_is_rejected(self):
        p=self.root/'verification/loop-callbacks-inputs.sha256'
        p.write_text(p.read_text()+p.read_text().splitlines()[0]+'\n')
        with self.assertRaisesRegex(ValueError,'duplicate shared query input'):check(self.root)

    def test_missing_negative_variant_is_rejected(self):
        p=self.root/'verification/loop-callbacks-environment.json';value=json.loads(p.read_text())
        del value['wrong_variants']['skip-first'];p.write_text(json.dumps(value));self.seal()
        with self.assertRaisesRegex(ValueError,'wrong Rust variant'):check(self.root)

    def test_wrong_version_must_refute_the_same_contract(self):
        p=self.root/self.env['wrong_variants']['skip-first']['template']
        p.write_text(p.read_text().replace(chr(172)+' QueryContract',chr(172)+' MembershipContract'));self.seal()
        with self.assertRaisesRegex(ValueError,'Same-contract'):check(self.root)

    def test_parent_input_cannot_be_silently_removed(self):
        self.env['inputs'].remove('Cargo.toml')
        (self.root/'verification/loop-callbacks-environment.json').write_text(json.dumps(self.env));self.seal()
        with self.assertRaisesRegex(ValueError,'input closure'):check(self.root)

    def test_independent_lake_view_cannot_drop_old_libraries(self):
        p=self.root/'lake-loop-callbacks.toml'
        p.write_text(p.read_text().replace('name = "SandproofNetwork"','name = "UnreviewedNetwork"'));self.seal()
        with self.assertRaisesRegex(ValueError,'retain all original libraries'):check(self.root)


if __name__=='__main__':
    unittest.main()
