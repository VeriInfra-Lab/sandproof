"""Finite tree/source admission negatives; not Rust semantic counterexamples."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools import check_tls_certificate_binding as binding

ROOT = Path(__file__).resolve().parents[2]


class TlsCertificateBindingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tree = json.loads((ROOT/binding.REL/'original.llbc').read_text())
        cls.frozen = json.loads((ROOT/binding.REL/'environment.json').read_text())

    def rejected(self, mutate):
        tree = copy.deepcopy(self.tree)
        mutate(tree['translated'])
        with self.assertRaises((ValueError, KeyError)):
            binding.check_shape(tree, self.frozen)

    def test_exact_reviewed_input_and_only_existing_unordered_normalization(self):
        self.assertEqual(binding.check(ROOT)['local_bodies'], 14)
        tree = copy.deepcopy(self.tree)
        tree['translated']['short_names'].reverse()
        self.assertEqual(binding.check_shape(tree, self.frozen), 14)

    def test_missing_or_opaque_root_is_rejected(self):
        for index in (11, 12, 13, 23, 27, 32, 38, 40, 41, 42, 43, 44, 143):
            with self.subTest(index=index):
                self.rejected(lambda t: t['fun_decls'][index].update(body=None))

    def test_first_to_last_is_rejected_before_frozen_tree_comparison(self):
        def mutate(t):
            call = binding.calls(t['fun_decls'][41])[0]
            item = t['fun_decls'][call['func']['Regular']['kind']['Fun']]
            item['item_meta']['name'][-1]['Ident'][0] = 'last'
        self.rejected(mutate)

    def test_wrong_pin_field_is_rejected(self):
        def mutate(t):
            for n in binding.nodes(t['fun_decls'][11]['body']):
                if n.get('Field') == [None, 1]:
                    n['Field'] = [None, 0]
                    return
            self.fail('original pin projection missing')
        self.rejected(mutate)

    def test_peer_pin_cannot_call_the_other_tls_object_facade(self):
        def mutate(t):
            binding.calls(t['fun_decls'][13])[0]['func']['Regular']['kind']['Fun'] = 12
        self.rejected(mutate)

    def test_digest_and_contains_order_and_full_arguments_are_retained(self):
        def mutate(t):
            body = t['fun_decls'][42]['body']['Structured']['body']['statements']
            locations = [i for i, s in enumerate(body) if isinstance(s['kind'], dict) and 'Call' in s['kind']]
            self.assertEqual(len(locations), 2)
            left, right = locations
            body[left], body[right] = body[right], body[left]
        self.rejected(mutate)

    def test_missing_unwind_cleanup_is_rejected(self):
        def mutate(t):
            for n in binding.nodes(t['fun_decls'][13]['body']):
                if 'on_unwind' in n:
                    n['on_unwind'] = None
                    return
            self.fail('original unwind branch missing')
        self.rejected(mutate)

    def test_new_local_body_or_indirect_call_is_rejected(self):
        def mutate(t):
            item = copy.deepcopy(t['fun_decls'][23])
            item['def_id'] = len(t['fun_decls'])
            t['fun_decls'].append(item)
        self.rejected(mutate)
        def indirect(t):
            binding.calls(t['fun_decls'][23])[0]['func'] = {'UnexpectedIndirect': 0}
        self.rejected(indirect)

    def test_unsafe_is_limited_to_the_exact_compiler_generated_glue(self):
        for index, unsafe in [(23, True), (34, False)]:
            with self.subTest(index=index):
                self.rejected(lambda t: t['fun_decls'][index]['signature'].update(is_unsafe=unsafe))

    def test_source_and_candidate_drift_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in [*self.frozen['sources'], str(binding.REL/'original.llbc'),
                         str(binding.REL/'environment.json')]:
                dest = root/name
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT/name, dest)
            for name in ['sandbox-internal/src/protected_server.rs',
                         'verification/aeneas/tls/SandproofTls/Funs.lean']:
                path = root/name
                before = path.read_bytes()
                path.write_bytes(before+b'\n')
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, 'changed'):
                    binding.check(root)
                path.write_bytes(before)


if __name__ == '__main__':
    unittest.main()
