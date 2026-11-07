"""Fail-closed admission checks for real identifier leaves and their library domain."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools import check_json_identifiers_extraction as audit

from tools.tests.proof_input_fixture import reference_root

ROOT = reference_root()


class JsonIdentifierExtractionTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        for relative in (
            audit.ENVIRONMENT, audit.ENTRY, audit.SOURCE,
            'sandbox-protocol/src/internal.rs',
            'verification/json-counter-environment.json',
            'verification/foundations-environment.json',
            'scripts/regenerate-foundations-lean.sh',
            'verification/extract-foundations/src/lib.rs',
            'verification/extract-foundations/src/numeric.rs',
            'verification/extract-foundations/src/receipts.rs',
            'verification/extract-foundations/src/json.rs',
            'verification/extract-foundations/src/json_request.rs',
            'verification/json-request-binding.json',
            'sandbox-protocol/src/internal/lifecycle.rs',
            'sandbox-internal/src/client.rs',
            'sandbox-internal/src/server.rs',
            'SandboxModel/Boundaries/JsonRequest/SliceBinding.lean',
            audit.DEST + '/SandproofFoundations/Funs.lean',
        ):
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        self.llbc = json.loads((ROOT / audit.LLBC).read_text())
        self.inventory = json.loads((ROOT / audit.DEST / 'translation.json').read_text())

    def check(self):
        return audit.check(self.root, self.llbc, self.inventory)

    def test_all_eight_real_leaves_are_transparent(self):
        self.assertEqual(self.check(), 38)

    def test_cannot_hide_real_identifier_deserialize(self):
        function = next(f for f in self.inventory['functions']
                        if f['lean_name'].endswith(
                            'TenantId.Insts.Serde_coreDeDeserialize.deserialize'))
        function['is_opaque'] = True
        with self.assertRaisesRegex(ValueError, 'owned body missing'):
            self.check()

    def test_cannot_substitute_another_identifier_call(self):
        path = self.root / audit.ENTRY
        text = path.read_text()
        path.write_text(text.replace(
            'sandproof_protocol::TenantId::deserialize(input)',
            'sandproof_protocol::SandboxId::deserialize(input)', 1))
        with self.assertRaisesRegex(ValueError, 'call-through changed'):
            self.check()

    def test_cannot_rebind_owned_source(self):
        function = next(f for f in self.inventory['functions']
                        if f['lean_name'].endswith(
                            'TenantId.Insts.Serde_coreDeDeserialize.deserialize'))
        function['source']['file'] = 'sandbox-protocol/src/internal.rs'
        with self.assertRaisesRegex(ValueError, 'source rebound'):
            self.check()

    def test_cannot_expand_registered_format_domain(self):
        path = self.root / audit.ENVIRONMENT
        environment = json.loads(path.read_text())
        environment['format_domain']['arguments'] = ['any:Display']
        path.write_text(json.dumps(environment))
        with self.assertRaisesRegex(ValueError, 'formatting domain'):
            self.check()

    def test_cannot_omit_an_identifier_from_registry(self):
        path = self.root / audit.ENVIRONMENT
        environment = json.loads(path.read_text())
        environment['functions'].remove(
            'sandproof_protocol.TenantId.Insts.Serde_coreDeDeserialize.deserialize')
        path.write_text(json.dumps(environment))
        with self.assertRaisesRegex(ValueError, 'function registry'):
            self.check()

    def test_cannot_add_observable_drop(self):
        function = next(f for f in self.inventory['functions']
                        if f['lean_name'].endswith(
                            'TenantId.Insts.Serde_coreDeDeserialize.deserialize'))
        decl = next(d for d in self.llbc['translated']['fun_decls']
                    if d is not None and d['def_id'] == function['def_id'])
        decl['body']['Structured']['unreviewed_cleanup'] = {'Drop': {'kind': 'Static'}}
        with self.assertRaises(ValueError):
            self.check()

    def test_cannot_remove_a_transparent_body(self):
        function = next(f for f in self.inventory['functions']
                        if f['lean_name'].endswith(
                            'TenantId.Insts.Serde_coreDeDeserialize.deserialize'))
        next(d for d in self.llbc['translated']['fun_decls']
             if d is not None and d['def_id'] == function['def_id'])['body'] = None
        with self.assertRaisesRegex(ValueError, 'owned body missing'):
            self.check()


if __name__ == '__main__':
    unittest.main()
