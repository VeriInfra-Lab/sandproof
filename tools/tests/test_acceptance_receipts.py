"""Admission regressions, not Rust/Lean proof evidence or forged CI statuses."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from tools import acceptance_receipts as receipts
from tools import proof_batch
from tools import proof_native_suite


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        temp=tempfile.TemporaryDirectory(); self.addCleanup(temp.cleanup)
        self.base=Path(temp.name); self.source=self.base/'source'; self.source.mkdir()
        (self.source/'Cargo.toml').write_text('[workspace]\n')
        (self.source/'Cargo.lock').write_text('version = 4\n')
        (self.source/'lib.rs').write_text('pub fn keep() {}\n')
        subprocess.run(['git','init','--quiet',str(self.source)],check=True)
        subprocess.run(['git','add','.'],cwd=self.source,check=True)
        self.cache=self.base/'evidence';self.cache.mkdir()
        receipts.write_json(self.cache/'runtime.json',{'unit_test_fixture':True})
        self.env=mock.patch.dict(os.environ,{'SANDPROOF_NATIVE_EVIDENCE':str(self.cache),
            'SANDPROOF_ACCEPTANCE_ROLE':'native'})
        self.env.start();self.addCleanup(self.env.stop)
        self.command=[sys.executable,'-c','print("actual native fixture ran")']
        self.log=self.base/'command.log'

    def publish(self, expected=0):
        command=self.command if expected==0 else [sys.executable,'-c','print("semantic fixture");raise SystemExit(101)']
        receipts.native_run('fixture',command,self.source,self.log,expected=expected)
        key=receipts.native_key('fixture',command,self.source)
        entry=self.cache/'commands'/receipts.digest(key)
        receipts.write_json(self.cache/'complete.json',dict(complete=True,exit_code=0,
            rules=receipts.rules(),runtime={'unit_test_fixture':True},
            commands={entry.name:receipts.sha(entry/'receipt.json')}))
        return command,key,entry

    def test_identical_native_command_executes_once(self):
        with mock.patch.object(receipts.subprocess,'run',wraps=subprocess.run) as run:
            self.publish()
            first=self.log.read_bytes();self.log.unlink()
            receipts.native_run('fixture',self.command,self.source,self.log)
            executions=[c for c in run.call_args_list if c.args[0]==self.command]
            self.assertEqual(len(executions),1)
            self.assertEqual(self.log.read_bytes(),first)

    def test_formal_reads_complete_evidence_without_executing_native(self):
        command,_,_=self.publish(expected=101)
        os.environ['SANDPROOF_ACCEPTANCE_ROLE']='formal'
        actual_run=subprocess.run
        def git_only(command,*args,**kwargs):
            if command[0]!='git':raise AssertionError('native on proof host: '+repr(command))
            return actual_run(command,*args,**kwargs)
        with mock.patch.object(receipts.subprocess,'run',side_effect=git_only):
            self.assertEqual(receipts.native_run('fixture',command,self.source,self.log,expected=101),101)
        self.assertIn('semantic fixture',self.log.read_text())

    def test_missing_or_incomplete_seal_cannot_replay(self):
        command,_,_=self.publish();os.environ['SANDPROOF_ACCEPTANCE_ROLE']='formal'
        seal=json.loads((self.cache/'complete.json').read_text())
        for field,value in [('complete',False),('exit_code',17),('rules',{})]:
            bad=copy.deepcopy(seal);bad[field]=value
            receipts.write_json(self.cache/'complete.json',bad)
            with self.subTest(field=field),self.assertRaises(ValueError):
                receipts.native_run('fixture',command,self.source,self.log)
        (self.cache/'complete.json').unlink()
        with self.assertRaises(OSError):receipts.native_run('fixture',command,self.source,self.log)

    def test_changed_source_dependency_configuration_and_runtime_invalidate(self):
        command,key,_=self.publish();os.environ['SANDPROOF_ACCEPTANCE_ROLE']='formal'
        for file in ['lib.rs','Cargo.toml','Cargo.lock']:
            path=self.source/file;original=path.read_bytes();path.write_bytes(original+b'\n')
            with self.subTest(file=file),self.assertRaises(ValueError):
                receipts.native_run('fixture',command,self.source,self.log)
            path.write_bytes(original)
        with mock.patch.dict(os.environ,{'CARGO_BUILD_TARGET':'different-target'}):
            self.assertNotEqual(receipts.native_key('fixture',command,self.source),key)
        receipts.write_json(self.cache/'runtime.json',{'unit_test_fixture':'changed tool'})
        with self.assertRaises(ValueError):receipts.native_run('fixture',command,self.source,self.log)

    def test_missing_corrupt_log_receipt_and_wrong_exit_are_not_success(self):
        _,key,entry=self.publish()
        receipt=json.loads((entry/'receipt.json').read_text())
        for field,value in [('complete',False),('exit_code',101),('exit_code',False),('key',{}),('files',{}),('files',[])]:
            bad=copy.deepcopy(receipt);bad[field]=value
            receipts.write_json(entry/'receipt.json',bad)
            with self.subTest(field=field):self.assertIsNone(receipts.load_success(entry,key))
        receipts.write_json(entry/'receipt.json',receipt)
        (entry/'command.log').write_text('changed')
        self.assertIsNone(receipts.load_success(entry,key))
        with self.assertRaises(ValueError):receipts.validate_native(self.cache)
        (entry/'command.log').unlink()
        self.assertIsNone(receipts.load_success(entry,key))

    def test_one_missing_component_blocks_the_whole_native_packet(self):
        _,_,entry=self.publish()
        self.assertTrue(receipts.validate_native(self.cache)['complete'])
        (entry/'command.log').unlink()
        with self.assertRaises(ValueError):receipts.validate_native(self.cache)

    def test_failure_or_interruption_never_publishes_success(self):
        command=[sys.executable,'-c','raise SystemExit(17)']
        with self.assertRaises(ValueError):receipts.native_run('bad',command,self.source,self.log)
        entry=self.cache/'commands'/receipts.digest(receipts.native_key('bad',command,self.source))
        self.assertFalse((entry/'receipt.json').exists())

    def test_fresh_component_failure_does_not_publish_a_case(self):
        def fail():raise RuntimeError('fresh binary or semantic assertion failed')
        with self.assertRaises(RuntimeError):receipts.native_case('bad-case',self.source,self.base,fail)
        key=receipts.native_key('bad-case',['component-case'],self.source)
        self.assertFalse((self.cache/'commands'/receipts.digest(key)/'receipt.json').exists())

    def test_path_escape_in_evidence_is_rejected(self):
        _,key,entry=self.publish()
        receipt=json.loads((entry/'receipt.json').read_text())
        for path in ['../outside.log','/tmp/outside.log']:
            receipt['files']={path:'0'*64};receipts.write_json(entry/'receipt.json',receipt)
            self.assertIsNone(receipts.load_success(entry,key))

    def test_new_untracked_candidate_and_cargo_config_are_inputs(self):
        before=receipts.rust_inputs(self.source)
        path=self.source/'sandbox-added/src/control.rs';path.parent.mkdir(parents=True);path.write_text('pub fn f() {}')
        config=self.source/'.cargo/config.toml';config.parent.mkdir();config.write_text('[build]\ntarget="x"')
        after=receipts.rust_inputs(self.source)
        self.assertNotEqual(before,after)
        self.assertIn('sandbox-added/src/control.rs',after)
        self.assertIn('.cargo/config.toml',after)

    def test_functional_acceptance_cannot_reuse_stale_documents(self):
        doc=self.source/'README.md';doc.write_text('[valid](Cargo.toml)\n')
        subprocess.run(['git','add','README.md'],cwd=self.source,check=True)
        before=receipts.native_key('functional',['component-case'],self.source)
        native_before=receipts.native_key('fixture',self.command,self.source)
        doc.write_text('[broken](missing.md)\n')
        self.assertNotEqual(receipts.native_key('functional',['component-case'],self.source),before)
        self.assertEqual(receipts.native_key('fixture',self.command,self.source),native_before)

    def test_embedded_resources_and_inherited_configuration_are_inputs(self):
        asset=self.source/'sandbox-example/query.sql';asset.parent.mkdir();asset.write_text('SELECT 1')
        deployment=self.source/'deploy/example.json';deployment.parent.mkdir();deployment.write_text('{}')
        subprocess.run(['git','add','sandbox-example/query.sql','deploy/example.json'],cwd=self.source,check=True)
        before=receipts.rust_inputs(self.source)
        asset.write_text('SELECT 2');deployment.write_text('{"different":true}')
        self.assertNotEqual(before,receipts.rust_inputs(self.source))
        config=self.base/'.cargo/config.toml';config.parent.mkdir();config.write_text('[build]\ntarget="other"')
        self.assertTrue(any(n.startswith('@configuration:') for n in receipts.rust_inputs(self.source)))

    def test_undeclared_owned_build_script_fails_closed(self):
        path=self.source/'sandbox-example/build.rs';path.parent.mkdir();path.write_text('fn main() {}')
        with self.assertRaisesRegex(ValueError,'generated-input registration'):
            receipts.rust_inputs(self.source)

    def test_unsealed_command_is_executed_again_in_a_new_batch(self):
        with mock.patch.object(receipts.subprocess,'run',wraps=subprocess.run) as run:
            with mock.patch.dict(os.environ,{'SANDPROOF_NATIVE_BATCH':'attempt-one'}):
                receipts.native_run('fixture',self.command,self.source,self.log)
            with mock.patch.dict(os.environ,{'SANDPROOF_NATIVE_BATCH':'attempt-two','SANDPROOF_NATIVE_PREVIOUS':''}):
                receipts.native_run('fixture',self.command,self.source,self.log)
            self.assertEqual(len([c for c in run.call_args_list if c.args[0]==self.command]),2)
        key=receipts.native_key('fixture',self.command,self.source)
        entry=self.cache/'commands'/receipts.digest(key)
        self.assertEqual(len(list((entry/'history').glob('*/receipt.json'))),1)

    def test_untrusted_checkout_refuses_without_changing_permissions(self):
        proof_native_suite.trusted_checkout(self.source)
        self.source.chmod(0o775)
        with self.assertRaisesRegex(ValueError,'untrusted checkout ancestry'):
            proof_native_suite.trusted_checkout(self.source)
        self.assertEqual(self.source.stat().st_mode&0o777,0o775)


class ClosureTests(unittest.TestCase):
    def test_transitive_lean_imports_and_added_inventory_are_not_ignored(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);(root/'SandboxModel').mkdir()
            (root/'lakefile.toml').write_text('[[lean_lib]]\nname="SandboxModel"\n')
            a=root/'SandboxModel/A.lean';b=root/'SandboxModel/B.lean';c=root/'SandboxModel/C.lean'
            a.write_text('import SandboxModel.B\n');b.write_text('import SandboxModel.C\n');c.write_text('def x := 1\n')
            before=proof_batch.closure(root,['SandboxModel/A.lean'])
            self.assertEqual(set(before),{'SandboxModel/A.lean','SandboxModel/B.lean','SandboxModel/C.lean'})
            c.write_text('def x := 2\n')
            self.assertNotEqual(before,proof_batch.closure(root,['SandboxModel/A.lean']))
            c.unlink()
            with self.assertRaises(OSError):proof_batch.closure(root,['SandboxModel/A.lean'])

    def test_manifest_cannot_hide_a_parent_or_escape_the_repository(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);path=root/'inputs.sha256'
            path.write_text('0'*64+'  ../outside\n')
            with self.assertRaises(ValueError):proof_batch.closure(root,['inputs.sha256'])

    def test_formal_task_list_preserves_all_original_modules(self):
        self.assertEqual(set(proof_batch.MODULES),{'json-request','tls-pin','json-identifiers','json-counter',
            'task-contract','revocation-contract','tunnel','data-drain','sqlite','foundations','network',
            'first-chain-linux','first-chain','storage'})
        root=Path(__file__).resolve().parents[2]
        retained=(root/'scripts/verify-retained-proofs.sh').read_text()
        self.assertIn("rg --files SandboxModel -g '*.lean'",retained)
        self.assertIn('verification/ProofAudit.lean',retained)
        self.assertIn('tools/proof_services.py',retained)


class FormalReuseTests(unittest.TestCase):
    def setUp(self):
        ReceiptTests.setUp(self)
        verification=self.source/'verification';verification.mkdir()
        (verification/'network-inputs.sha256').write_text('')
        (verification/'acceptance-inputs.sha256').write_text('')
        stamp=self.base/'formal-runtime.json';receipts.write_json(stamp,{'unit_test_fixture':True})
        receipts.write_json(self.cache/'complete.json',{'unit_test_fixture':True})
        self.formal_env=mock.patch.dict(os.environ,{'SANDPROOF_FORMAL_RUNTIME':str(stamp),
            'SANDPROOF_FORMAL_CACHE':str(self.base/'formal-cache'),
            'TMPDIR':str(self.base),'SANDPROOF_PROOF_RECOMPUTE':'0',
            'SANDPROOF_PROOF_BATCH':'unit-batch'})
        self.formal_env.start();self.addCleanup(self.formal_env.stop)

    def test_same_batch_and_next_batch_reuse_exact_success(self):
        first=proof_batch.step('network',self.command,self.source)
        second=proof_batch.step('network',self.command,self.source)
        self.assertFalse(first['reused']);self.assertTrue(second['reused'])
        with mock.patch.dict(os.environ,{'SANDPROOF_PROOF_BATCH':'next-batch'}):
            self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])

    def test_full_recompute_still_deduplicates_one_batch(self):
        with mock.patch.dict(os.environ,{'SANDPROOF_PROOF_RECOMPUTE':'1'}):
            self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])
            self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])
            with mock.patch.dict(os.environ,{'SANDPROOF_PROOF_BATCH':'next-batch'}):
                self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])

    def test_corrupt_log_and_missing_raw_evidence_force_reexecution(self):
        row=proof_batch.step('network',self.command,self.source)
        entry=Path(row['receipt']).parent
        (entry/'command.log').write_text('corrupt')
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])

        data=json.loads((entry/'receipt.json').read_text())
        data['evidence']={str(self.base/'deleted-raw.llbc'):'0'*64}
        receipts.write_json(entry/'receipt.json',data)
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])

    def test_unrelated_lean_stays_reusable_but_imported_contract_changes_miss(self):
        model=self.source/'Library.lean';model.write_text('def x := 1\n')
        (self.source/'verification/network-inputs.sha256').write_text(receipts.sha(model)+'  Library.lean\n')
        proof_batch.step('network',self.command,self.source)
        (self.source/'Unrelated.lean').write_text('def y := 1\n')
        self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])
        model.write_text('def x := 2\n')
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])

    def test_checker_negative_fixture_and_tool_changes_force_reexecution(self):
        fixture=self.source/'verification/fixtures/negative.rs';fixture.parent.mkdir();fixture.write_text('fixture one')
        subprocess.run(['git','add','verification/fixtures/negative.rs'],cwd=self.source,check=True)
        proof_batch.step('network',self.command,self.source)
        fixture.write_text('fixture two')
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])
        runtime=Path(os.environ['SANDPROOF_FORMAL_RUNTIME'])
        receipts.write_json(runtime,{'unit_test_fixture':'tool changed'})
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])

    def test_new_functional_document_check_does_not_discard_an_unchanged_proof(self):
        seal=dict(rules={},rust={},runtime={},commands={})
        receipts.write_json(self.cache/'complete.json',seal)
        proof_batch.step('network',self.command,self.source)
        identity='a'*64;entry=self.cache/'commands'/identity;entry.mkdir(parents=True)
        receipts.write_json(entry/'receipt.json',{'key':{'label':'functional','rules':{},'runtime':{}}})
        seal['commands'][identity]=receipts.sha(entry/'receipt.json')
        receipts.write_json(self.cache/'complete.json',seal)
        self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])


class FixedRuntimeDigestTests(unittest.TestCase):
    def test_unchanged_fixed_file_is_not_read_again(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'fixed-tool';path.write_text('fixed version')
            # Model a quiescent installed tool, rather than a just-written file
            # whose timestamp can still share the writer's filesystem tick.
            with mock.patch.object(proof_batch.time,'time_ns',return_value=path.stat().st_ctime_ns+3_000_000_000):
                current={};expected=proof_batch.fixed_runtime_sha(path,{'files':{}},current,allow_metadata=True)
                with mock.patch('hashlib.file_digest',side_effect=AssertionError('unnecessary content reread')):
                    self.assertEqual(proof_batch.fixed_runtime_sha(path,{'files':current},{},allow_metadata=True),expected)

    def test_preserving_size_and_mtime_does_not_hide_content_change(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'fixed-tool';path.write_text('version one')
            current={};before=proof_batch.fixed_runtime_sha(path,{'files':{}},current,allow_metadata=True)
            stat=path.stat();path.write_text('version two')
            os.utime(path,ns=(stat.st_atime_ns,stat.st_mtime_ns))
            self.assertNotEqual(proof_batch.fixed_runtime_sha(path,{'files':current},{},allow_metadata=True),before)

    def test_inode_replacement_and_broken_digest_force_content_read(self):
        with tempfile.TemporaryDirectory() as temporary:
            path=Path(temporary)/'fixed-tool';path.write_text('version one')
            current={};before=proof_batch.fixed_runtime_sha(path,{'files':{}},current,allow_metadata=True)
            other=Path(temporary)/'replacement';other.write_text('version two');other.replace(path)
            self.assertNotEqual(proof_batch.fixed_runtime_sha(path,{'files':current},{},allow_metadata=True),before)
            current[str(path.resolve())]['sha256']='not-a-digest'
            self.assertEqual(proof_batch.fixed_runtime_sha(path,{'files':current},{},allow_metadata=True),receipts.sha(path))


if __name__=='__main__':unittest.main()
