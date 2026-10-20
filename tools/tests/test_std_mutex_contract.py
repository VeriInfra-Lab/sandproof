"""Reject missing identity/provider, library drift and opaque original bodies."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools.check_std_mutex_contract import ROOT, SOURCE_FILES, check, extraction, digest
from tools.check_foundations_extraction import llbc_digest


class StdMutexAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name)/'source';self.sysroots={}
        self.env=json.loads((ROOT/'verification/std-mutex-environment.json').read_text())
        for name in self.env['inputs']:
            destination=self.root/name;destination.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(ROOT/name,destination)
        for role,library in self.env['standard_library'].items():
            self.sysroots[role]=Path(temporary.name)/role
            for name in library['sources']:
                path=self.sysroots[role]/'lib/rustlib/src/rust/library'/name
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('// synthetic admission fixture only: '+role+'/'+name+'\n')
                library['sources'][name]=digest(path)
        self.seal()

    def seal(self):
        self.env['production_sources']={n:digest(self.root/n) for n in SOURCE_FILES}
        (self.root/'verification/std-mutex-environment.json').write_text(json.dumps(self.env))
        (self.root/'verification/std-mutex-inputs.sha256').write_text(''.join(
            digest(self.root/n)+'  '+n+'\n' for n in self.env['inputs']))

    def test_actual_finite_registry_admitted(self):
        self.assertEqual(len(check(self.root,self.sysroots)['use_points']),13)

    def test_original_unlock_moved_after_diagnostic_rejected(self):
        path=self.root/'controller-service/src/application/diagnostics.rs'
        text=path.read_text();start=text.index('impl<T> Drop for SerialGuard')
        prefix=text[:start];body=text[start:]
        body=body.replace('        drop(self.guard.take());\n','',1)
        body=body.replace('            CURRENT.with(|current| {','            CURRENT.with(|current| {\n                drop(self.guard.take());',1)
        path.write_text(prefix+body);self.seal()
        with self.assertRaisesRegex(ValueError,'call/release order changed'):
            check(self.root,self.sysroots)

    def test_original_mutex_substitution_rejected(self):
        path=self.root/'controller-service/src/application/diagnostics.rs'
        path.write_text(path.read_text().replace('let guard = self.0.lock()?;','let guard = other.lock()?;'))
        self.seal()
        with self.assertRaisesRegex(ValueError,'call anchor changed'):
            check(self.root,self.sysroots)

    def test_no_business_callback_can_be_relabelled_trusted(self):
        site=next(s for s in self.env['use_points'] if s['id']=='meter.business-policy')
        site['status']='conditional-mechanical';site['public_results']=[self.env['theorems'][0]]
        self.seal()
        with self.assertRaisesRegex(ValueError,'business decision cannot be trusted'):
            check(self.root,self.sysroots)

    def test_public_rule_cannot_be_shifted_to_business(self):
        site=next(s for s in self.env['use_points'] if s['id']=='serial.drop')
        site['status']='business-obligation';site['public_results']=[];self.seal()
        with self.assertRaisesRegex(ValueError,'public standard Mutex rule cannot be moved'):
            check(self.root,self.sysroots)

    def test_original_object_required(self):
        self.env['use_points'][0]['original_object']='';self.seal()
        with self.assertRaisesRegex(ValueError,'lacks original object'):
            check(self.root,self.sysroots)

    def test_missing_frozen_input_rejected(self):
        path=self.root/'verification/std-mutex-inputs.sha256'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n')
        with self.assertRaisesRegex(ValueError,'Missing or duplicate standard Mutex input'):
            check(self.root,self.sysroots)

    def test_missing_audit_rejected(self):
        path=self.root/'SandboxModel/Boundaries/StdMutex/Audit.lean'
        path.write_text('\n'.join(path.read_text().splitlines()[:-1])+'\n');self.seal()
        with self.assertRaisesRegex(ValueError,'Missing, duplicate or extra standard Mutex audit'):
            check(self.root,self.sysroots)

    def test_poison_implementation_drift_rejected(self):
        path=self.sysroots['frontend']/'lib/rustlib/src/rust/library/std/src/sync/poison.rs'
        path.write_text('// changed poison observation\n')
        with self.assertRaisesRegex(ValueError,'Reviewed standard Mutex library source changed'):
            check(self.root,self.sysroots)

    def extraction_case(self):
        return (copy.deepcopy(self.env),
                json.loads((self.root/self.env['extraction']['llbc']).read_text()),
                json.loads((self.root/self.env['extraction']['inventory']).read_text()))

    def test_whole_ordered_llbc_checked(self):
        env,tree,inventory=self.extraction_case()
        tree['translated']['fun_decls'][11]['body']['Structured']['body']['statements'].reverse()
        with self.assertRaisesRegex(ValueError,'Full standard Mutex LLBC changed'):
            extraction(env,tree,inventory)

    def test_resealed_opaque_owned_body_rejected(self):
        env,tree,inventory=self.extraction_case()
        tree['translated']['fun_decls'][11]['body']='Opaque'
        env['extraction']['normalized_sha256']=llbc_digest(tree)
        with self.assertRaisesRegex(ValueError,'must remain transparent'):
            extraction(env,tree,inventory)

    def test_resealed_unreviewed_library_call_rejected(self):
        env,tree,inventory=self.extraction_case()
        inventory['functions'].append(dict(def_id=99,is_local=False,is_opaque=True))
        with self.assertRaisesRegex(ValueError,'unreviewed external function'):
            extraction(env,tree,inventory)
