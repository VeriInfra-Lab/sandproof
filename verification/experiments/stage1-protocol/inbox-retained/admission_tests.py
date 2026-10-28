"""Reject incomplete real extraction metadata before any retained proof is admitted."""
import argparse
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('retained_admission_formal', HERE/'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class Admission(unittest.TestCase):
    raw = None
    translation = None

    def generated(self, value):
        with tempfile.TemporaryDirectory(prefix='retained-metadata-') as name:
            directory = Path(name)
            (directory/'translation.json').write_text(json.dumps(value))
            return formal.admit_generated(directory)

    def test_original_complete_metadata_is_admitted(self):
        formal.admit_ir(self.raw, ['rusqlite::error::Error'])
        self.generated(self.translation)

    def test_missing_actual_retained_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if not f['lean_name'].endswith('.renewal_confirmation.follows_retained')]
        with self.assertRaisesRegex(ValueError, 'root/body missing'): self.generated(changed)

    def test_protocol_root_missing_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if f['lean_name'] != 'SandproofStage1RequestRust.accepted_validate']
        with self.assertRaisesRegex(ValueError, 'root/body missing'): self.generated(changed)

    def test_retained_opaque_is_rejected(self):
        changed = deepcopy(self.translation)
        for f in changed['functions']:
            if f['lean_name'].endswith('.renewal_confirmation.follows_retained'): f['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'root/body missing'): self.generated(changed)

    def test_bound_callback_missing_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if not f['lean_name'].endswith('.follows_retained.closure.Insts.CoreOpsFunctionFnOnceTupleU64Bool.call_once')]
        with self.assertRaisesRegex(ValueError, 'root/body missing'): self.generated(changed)

    def test_option_call_missing_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'] = [f for f in changed['functions'] if f['lean_name'] != 'core.option.Option.is_some_and']
        with self.assertRaisesRegex(ValueError, 'business opacity'): self.generated(changed)

    def test_bound_callback_opaque_is_rejected(self):
        changed = deepcopy(self.translation)
        for f in changed['functions']:
            if f['lean_name'].endswith('.follows_retained.closure.Insts.CoreOpsFunctionFnOnceTupleU64Bool.call_once'): f['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'root/body missing'): self.generated(changed)

    def test_business_opaque_is_rejected(self):
        changed = deepcopy(self.translation)
        changed['functions'].append(dict(lean_name='business_authorization', is_opaque=True))
        with self.assertRaisesRegex(ValueError, 'business opacity'): self.generated(changed)

    def test_unreviewed_opaque_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['options']['opaque'].append('sandproof_controller_service::controller')
        with self.assertRaisesRegex(ValueError, 'options'): formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_wrong_target_is_rejected(self):
        changed = deepcopy(self.raw)
        changed['translated']['target_information'][0]['key'] = 'different-target'
        with self.assertRaisesRegex(ValueError, 'target'): formal.admit_ir(changed, ['rusqlite::error::Error'])

    def test_missing_real_forwarder_is_rejected(self):
        changed = deepcopy(self.raw)
        for f in changed['translated']['fun_decls']:
            if f and f['item_meta']['name'][-1] == {'Ident':['proof_retained_history',0]}:
                f['item_meta']['name'][-1] = {'Ident':['different_entry',0]}
        with self.assertRaisesRegex(ValueError, 'forwarding root'): formal.admit_ir(changed, ['rusqlite::error::Error'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--llbc', type=Path, required=True)
    parser.add_argument('--generated', type=Path, required=True)
    args = parser.parse_args()
    Admission.raw = json.loads(args.llbc.read_text())
    Admission.translation = json.loads((args.generated/'translation.json').read_text())
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Admission))
    raise SystemExit(not result.wasSuccessful())
