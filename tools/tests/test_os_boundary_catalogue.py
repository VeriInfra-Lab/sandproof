"""Review-catalogue negative cases; not Rust semantic counterexamples."""
import copy
import unittest
from pathlib import Path
from tools.check_os_boundary_catalogue import CATALOGUE,read,validate
ROOT=Path(__file__).resolve().parents[2]

class OsBoundaryCatalogue(unittest.TestCase):
    def setUp(self):self.data=read(ROOT/CATALOGUE)
    def test_exact_current_source_and_registered_families(self):
        result=validate(ROOT,self.data)
        self.assertEqual(result['families'],25)
        self.assertEqual(result['compiled_variants'],60)
        self.assertEqual(result['new_program_theorems'],0)
    def test_missing_family_or_required_guarantee_rejected(self):
        bad=copy.deepcopy(self.data);bad['families'].pop()
        with self.assertRaises(ValueError):validate(ROOT,bad)
        for key in ['original_object_and_config','normal_result','failure_and_partial_effects',
                    'cancel_panic_crash','progress_conditions','caller_obligations']:
            bad=copy.deepcopy(self.data);bad['families'][0][key]=''
            with self.subTest(key=key),self.assertRaises(ValueError):validate(ROOT,bad)
    def test_stale_source_cfg_and_ambiguous_locator_rejected(self):
        for field in ['native_inputs_sha256']:
            bad=copy.deepcopy(self.data);bad[field]='0'*64
            with self.assertRaises(ValueError):validate(ROOT,bad)
        for field in ['index_sha256','capture_receipt_sha256','config_sha256']:
            bad=copy.deepcopy(self.data);bad['compiled_source_evidence'][field]='0'*64
            with self.subTest(field=field),self.assertRaises(ValueError):validate(ROOT,bad)
        bad=copy.deepcopy(self.data);bad['versions']['redis_server']='latest'
        with self.assertRaises(ValueError):validate(ROOT,bad)
        bad=copy.deepcopy(self.data);bad['families'][0]['locations'][0]['line']+=1
        with self.assertRaises(ValueError):validate(ROOT,bad)
    def test_trust_and_proof_promotion_rejected(self):
        bad=copy.deepcopy(self.data);bad.pop('status_scope')
        with self.assertRaises(ValueError):validate(ROOT,bad)
        bad=copy.deepcopy(self.data);bad['families'][0]['public_status']='business_proof'
        with self.assertRaises(ValueError):validate(ROOT,bad)
        for field in ['new_program_theorems','new_trusted_guarantees']:
            bad=copy.deepcopy(self.data);bad[field]=1
            with self.assertRaises(ValueError):validate(ROOT,bad)
        bad=copy.deepcopy(self.data);bad['families'][0]['contracts']=['A13']
        with self.assertRaises(ValueError):validate(ROOT,bad)
        for field in self.data['denied_inferences']:
            bad=copy.deepcopy(self.data);bad['denied_inferences'][field]=True
            with self.subTest(field=field),self.assertRaises(ValueError):validate(ROOT,bad)
    def test_pending_and_single_tls_proposal_not_hidden(self):
        for name in ['helper-child','tls-original-provenance']:
            bad=copy.deepcopy(self.data);next(r for r in bad['families'] if r['id']==name)['public_status']='registered_contract'
            with self.assertRaises(ValueError):validate(ROOT,bad)
        bad=copy.deepcopy(self.data);bad['public_completion']['one_proposal']['new_trust']=True
        with self.assertRaises(ValueError):validate(ROOT,bad)

if __name__=='__main__':unittest.main()
