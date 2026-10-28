"""Admission regressions, not Rust/Lean proof evidence or forged CI statuses."""
import copy
import json
import os
from pathlib import Path
import re
import shutil
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
        self.base=Path(temp.name); self.source=self.base/'source'; self.source.mkdir(mode=0o700)
        (self.source/'Cargo.toml').write_text('[workspace]\n')
        (self.source/'Cargo.lock').write_text('version = 4\n')
        (self.source/'lib.rs').write_text('pub fn keep() {}\n')
        subprocess.run(['git','init','--quiet',str(self.source)],check=True)
        subprocess.run(['git','add','.'],cwd=self.source,check=True)
        self.cache=self.base/'evidence';self.cache.mkdir()
        receipts.write_json(self.cache/'runtime.json',{'unit_test_fixture':True})
        self.env=mock.patch.dict(os.environ,{'SANDPROOF_NATIVE_EVIDENCE':str(self.cache),
            'SANDPROOF_ACCEPTANCE_ROLE':'native',
            **{name:'' for name in receipts.SOURCE_ARCHIVES}})
        self.env.start();self.addCleanup(self.env.stop)
        self.command=[sys.executable,'-c','print("actual native fixture ran")']
        self.log=self.base/'command.log'

    def publish(self, expected=0):
        command=self.command if expected==0 else [sys.executable,'-c','print("semantic fixture");raise SystemExit(101)']
        receipts.native_run('fixture',command,self.source,self.log,expected=expected)
        key=receipts.native_key('fixture',command,self.source)
        entry=self.cache/'commands'/receipts.digest(key)
        receipts.write_json(self.cache/'complete.json',dict(schema=receipts.NATIVE_SCHEMA,
            scope='all',scope_complete=True,full_native_suite_complete=True,complete=True,exit_code=0,
            rules=receipts.rules(),runtime={'unit_test_fixture':True},
            contracts=receipts.contract_inputs(receipts.ROOT),source_archives=receipts.source_archive_inputs(),
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

    def test_standalone_defaults_to_reading_evidence_never_native_execution(self):
        command,_,_=self.publish()
        os.environ.pop('SANDPROOF_ACCEPTANCE_ROLE')
        actual_run=subprocess.run
        def git_only(command,*args,**kwargs):
            if command[0]!='git':raise AssertionError('standalone executed native: '+repr(command))
            return actual_run(command,*args,**kwargs)
        with mock.patch.object(receipts.subprocess,'run',side_effect=git_only):
            self.assertEqual(receipts.native_run('fixture',command,self.source,self.log),0)

    def test_missing_or_incomplete_seal_cannot_replay(self):
        command,_,_=self.publish();os.environ['SANDPROOF_ACCEPTANCE_ROLE']='formal'
        seal=json.loads((self.cache/'complete.json').read_text())
        for field,value in [('complete',False),('exit_code',17),('rules',{}),
                            ('schema',1),('contracts',{}),('source_archives',{'changed':'0'*64})]:
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

    def add_contract(self, name='fixture-environment.json', value='first'):
        path=self.source/'verification'/name
        path.parent.mkdir(exist_ok=True)
        path.write_text(json.dumps({'normal_result':value}))
        subprocess.run(['git','add',str(path.relative_to(self.source))],cwd=self.source,check=True)
        return path

    def test_root_contract_change_reexecutes_the_actual_command(self):
        path=self.add_contract()
        command=[sys.executable,'-c',
            'import json;print(json.load(open("verification/fixture-environment.json"))["normal_result"])']
        receipts.native_run('contract',command,self.source,self.log)
        self.assertEqual(self.log.read_text().strip(),'first')
        path.write_text('{"normal_result":"second"}')
        receipts.native_run('contract',command,self.source,self.log)
        self.assertEqual(self.log.read_text().strip(),'second')
        path.unlink()
        with self.assertRaisesRegex(ValueError,'native contract input'):
            receipts.native_run('contract',command,self.source,self.log)

    def test_component_case_reuses_identical_contract_and_reexecutes_changed_contract(self):
        path=self.add_contract();output=self.base/'component';output.mkdir();calls=[]
        def execute():
            calls.append(json.loads(path.read_text())['normal_result'])
            (output/'original.log').write_text(calls[-1])
        receipts.native_case('contract',self.source,output,execute)
        receipts.native_case('contract',self.source,output,execute)
        self.assertEqual(calls,['first'])
        path.write_text('{"normal_result":"second"}')
        receipts.native_case('contract',self.source,output,execute)
        self.assertEqual(calls,['first','second'])

    def test_review_and_candidate_contracts_addition_removal_and_bytes_are_inputs(self):
        path=self.add_contract();key=receipts.native_key('fixture',self.command,self.source,root=self.source)
        self.assertIn('verification/fixture-environment.json',key['contracts']['review'])
        self.assertIn('verification/fixture-environment.json',key['contracts']['source'])
        added=self.source/'verification/new-contract.json';added.write_text('{}')
        self.assertNotEqual(key,receipts.native_key('fixture',self.command,self.source,root=self.source))
        added.unlink()
        self.assertEqual(key,receipts.native_key('fixture',self.command,self.source,root=self.source))
        other=self.base/'review';shutil.copytree(self.source,other)
        before=receipts.native_key('fixture',self.command,self.source,root=other)
        (other/'verification/fixture-environment.json').write_text('{"normal_result":"other"}')
        self.assertNotEqual(before,receipts.native_key('fixture',self.command,self.source,root=other))
        original=path.read_bytes();path.unlink();path.symlink_to(other/'verification/fixture-environment.json')
        with self.assertRaisesRegex(ValueError,'native contract input'):
            receipts.native_key('fixture',self.command,self.source)
        path.unlink();path.write_bytes(original)

    def test_same_archive_bytes_reuse_but_changed_bytes_execute_again(self):
        archive=self.base/'original.tar';archive.write_text('first')
        name=receipts.SOURCE_ARCHIVES[0];os.environ[name]=str(archive)
        command=[sys.executable,'-c','import os;print(open(os.environ["'+name+'"]).read())']
        with mock.patch.object(receipts.subprocess,'run',wraps=subprocess.run) as run:
            receipts.native_run('archive',command,self.source,self.log)
            relocated=self.base/'relocated.tar';shutil.copyfile(archive,relocated)
            os.environ[name]=str(relocated)
            receipts.native_run('archive',command,self.source,self.log)
            self.assertEqual(len([c for c in run.call_args_list if c.args[0]==command]),1)
            relocated.write_text('second')
            receipts.native_run('archive',command,self.source,self.log)
            self.assertEqual(len([c for c in run.call_args_list if c.args[0]==command]),2)
            self.assertEqual(self.log.read_text().strip(),'second')
            relocated.unlink()
            with self.assertRaisesRegex(ValueError,'native source archive'):
                receipts.native_run('archive',command,self.source,self.log)
            relocated.symlink_to(archive)
            with self.assertRaisesRegex(ValueError,'native source archive'):
                receipts.native_run('archive',command,self.source,self.log)

    def test_formal_admission_rejects_old_seal_schema_and_changed_archive(self):
        archive=self.base/'original.tar';archive.write_text('first')
        os.environ[receipts.SOURCE_ARCHIVES[0]]=str(archive)
        self.publish();self.assertTrue(receipts.validate_native(self.cache)['complete'])
        seal=json.loads((self.cache/'complete.json').read_text());seal['schema']=1
        receipts.write_json(self.cache/'complete.json',seal)
        with self.assertRaises(ValueError):receipts.validate_native(self.cache)
        seal['schema']=receipts.NATIVE_SCHEMA;receipts.write_json(self.cache/'complete.json',seal)
        archive.write_text('changed')
        with self.assertRaises(ValueError):receipts.validate_native(self.cache)

    def test_native_contract_binding_does_not_turn_proof_only_edits_into_native_inputs(self):
        self.add_contract();key=receipts.native_key('fixture',self.command,self.source,root=self.source)
        (self.source/'verification/fixture-inputs.sha256').write_text('not consumed execution configuration')
        (self.source/'README.md').write_text('updated report')
        (self.source/'verification/Proof.lean').write_text('-- changed proof')
        subprocess.run(['git','add','.'],cwd=self.source,check=True)
        self.assertEqual(key,receipts.native_key('fixture',self.command,self.source,root=self.source))

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

    def test_json_mutant_copy_preserves_tracked_inventory_without_a_commit(self):
        from tools.check_json_request_negative import materialize
        (self.source/'.gitignore').write_text('ignored.rs\n')
        (self.source/'ignored.rs').write_text('pub fn retained() {}\n')
        subprocess.run(['git','add','--force','.gitignore','ignored.rs'],cwd=self.source,check=True)
        candidate=self.base/'mutant'
        materialize(self.source,candidate)
        self.assertEqual(receipts.rust_inputs(self.source),receipts.rust_inputs(candidate))
        (candidate/'ignored.rs').write_text('pub fn changed() {}\n')
        self.assertNotEqual(receipts.rust_inputs(self.source),receipts.rust_inputs(candidate))


class ClosureTests(unittest.TestCase):
    def test_all_published_json_negative_drivers_have_a_native_phase(self):
        actual={p.name for p in (receipts.ROOT/'tools').glob('check_json_*negative.py')}
        self.assertEqual(actual,set(proof_native_suite.JSON_NEGATIVE_DRIVERS))

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
            'task-contract','revocation-contract','owned-message','notify','joinset','std-mutex','condvar','tokio-rwlock','tunnel','data-drain','sqlite','foundations','network',
            'first-chain-linux','first-chain','storage','schema-domains','loop-callbacks','sqlite-statements','child-observation','callback-state','bounded-writer','btree-set-binding','protected-directory','tokio-mutex','stage1-protocol'})
        self.assertEqual(len(proof_batch.MODULES),len(set(proof_batch.MODULES)))
        root=Path(__file__).resolve().parents[2]
        retained=(root/'scripts/verify-retained-proofs.sh').read_text()
        self.assertIn("rg --files SandboxModel -g '*.lean'",retained)
        self.assertIn('verification/ProofAudit.lean',retained)
        self.assertIn('tools/proof_services.py',retained)

    def test_every_formal_component_requirement_is_reachable_through_cli(self):
        root = Path(__file__).resolve().parents[2]
        required = set()
        for entry in (root / 'scripts').glob('verify-*.sh'):
            required.update(re.findall(r'tools/proof_native_suite\.py require ([a-z][a-z0-9-]*)',
                                       entry.read_text()))
        self.assertIn('condvar', required)
        self.assertIn('tokio-rwlock', required)
        self.assertIn('child-observation', required)
        self.assertIn('callback-state', required)
        self.assertIn('bounded-writer', required)
        self.assertIn('stage1-protocol', required)
        for case in sorted(required):
            with self.subTest(case=case), mock.patch.object(proof_native_suite, 'native_case') as admit:
                proof_native_suite.main(['require', case])
                admit.assert_called_once_with(case, proof_native_suite.ROOT, None, scope='all')

    def test_sqlite_view_retains_the_complete_parent_without_duplicate_libraries(self):
        from tools.check_sqlite_extraction import audit_surface
        repository=Path(__file__).resolve().parents[2]
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            shutil.copytree(repository/'verification/aeneas/sqlite',root/'verification/aeneas/sqlite')
            base=(repository/'lake-first-chain.toml').read_text()
            view=root/'lake-sqlite.toml'
            (root/'lake-first-chain.toml').write_text(base)
            view.write_text(base)
            self.assertEqual(len(audit_surface(root)),18)
            extension='\n[[lean_lib]]\nname = "SandproofSqlite"\nsrcDir = "verification/aeneas/sqlite"\n'
            view.write_text(base.rstrip()+'\n'+extension)
            with self.assertRaisesRegex(ValueError,'preserve the full'):
                audit_surface(root)
            view.write_text(base.replace('name = "SandproofSchemaDomains"','name = "WrongLibrary"'))
            with self.assertRaisesRegex(ValueError,'preserve the full'):
                audit_surface(root)
            without=base.replace(extension,'')
            self.assertNotEqual(without,base)
            (root/'lake-first-chain.toml').write_text(without)
            view.write_text(without.rstrip()+'\n'+extension)
            self.assertEqual(len(audit_surface(root)),18)
            invalid=base.replace('srcDir = "verification/aeneas/sqlite"','srcDir = "wrong-source"')
            (root/'lake-first-chain.toml').write_text(invalid)
            view.write_text(invalid)
            with self.assertRaisesRegex(ValueError,'original unique source'):
                audit_surface(root)

    def test_audit_inventory_includes_separate_negative_roots_and_requires_their_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            fixtures={
                'verification/ProofAudit.lean':'Root.accepts',
                'SandboxModel/Actual/Audit.lean':'Actual.safe',
                'verification/aeneas/Actual/Audit.lean':'Generated.refines',
                'verification/experiments/stage1-protocol/ScopedWrong.lean.in':'ScopedWrong.rejected',
                **{name:'Wrong.rejected'+str(index) for index,name in enumerate(proof_batch.EXTRA_AUDIT_FILES)}}
            for name, declaration in fixtures.items():
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True)
                path.write_text('#audit_controller_proof '+declaration+'\n')
            (root/'verification/experiments/stage1-protocol/registered.py').write_text(
                'def audit_template_paths(root):\n'
                '    return ("verification/experiments/stage1-protocol/ScopedWrong.lean.in",)\n')
            self.assertEqual(proof_batch.expected_audits(root),set(fixtures.values()))
            (root/proof_batch.EXTRA_AUDIT_FILES[0]).unlink()
            with self.assertRaises(OSError):proof_batch.expected_audits(root)


class ScopedRetainedReceiptTests(unittest.TestCase):
    """Operator-local admission fixtures, never real proof success records."""

    def test_scoped_files_need_current_seal_and_unchanged_original_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base/'source'; root.mkdir()
            cache = base/'cache'
            key = {'test_fixture': 'current-inputs'}
            entry = cache/'modules/stage1-protocol'/receipts.digest(key)
            entry.mkdir(parents=True)
            (entry/'command.log').write_text('fixture command completed\n')
            result = base/'original/registered/registered-result.json'
            result.parent.mkdir(parents=True)
            receipts.write_json(result, {'complete': True})
            original = base/'original/actual-service.log'
            original.write_text('fixture raw diagnostic bytes\n')
            receipt = dict(complete=True, exit_code=0, key=key,
                files={'command.log': receipts.sha(entry/'command.log')},
                evidence={str(result): receipts.sha(result), str(original): receipts.sha(original)})
            receipts.write_json(entry/'receipt.json', receipt)
            helper = root/'verification/experiments/stage1-protocol/registered.py'
            helper.parent.mkdir(parents=True)
            helper.write_text('def validate_retained_coverage(root, record):\n'
                              '    return {"SandboxModel/Scoped.lean": {"fixture": True}}\n')
            with mock.patch.dict(os.environ, SANDPROOF_FORMAL_CACHE=str(cache)), \
                    mock.patch.object(proof_batch, 'module_key', return_value=key):
                covered, binding = proof_batch.stage1_retained_coverage(root)
                self.assertEqual(set(covered), {'SandboxModel/Scoped.lean'})
                self.assertEqual(binding['registered_result_sha256'], receipts.sha(result))
                self.assertEqual(binding['module_receipt_sha256'], receipts.sha(entry/'receipt.json'))
                saved = original.read_bytes()
                for value in (b'changed raw diagnostics', None):
                    if value is None:
                        original.unlink()
                    else:
                        original.write_bytes(value)
                    with self.assertRaisesRegex(ValueError, 'original formal evidence changed'):
                        proof_batch.stage1_retained_coverage(root)
                    original.write_bytes(saved)
                receipt['key'] = {'test_fixture': 'older-inputs'}
                receipts.write_json(entry/'receipt.json', receipt)
                with self.assertRaisesRegex(ValueError, 'current complete stage1 module receipt'):
                    proof_batch.stage1_retained_coverage(root)


class ExecutionMappingTests(unittest.TestCase):
    def test_required_namespace_files_are_content_bound_and_cannot_be_missing(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)
            environment={}
            for name in ['SANDPROOF_FORMAL_WRAPPER','SANDPROOF_WRITER_LEASE','SANDPROOF_PROOF_BWRAP']:
                path=root/name;path.write_text('fixture for '+name)
                environment[name]=str(path)
            with mock.patch.object(proof_batch,'MODULES',('bounded-writer',)), mock.patch.dict(os.environ,environment):
                before=proof_batch.execution_runtime_inputs()
                self.assertEqual(set(before),{'outer_wrapper','directory_lease','namespace_binary'})
                for name in environment:
                    path=Path(environment[name]);original=path.read_bytes()
                    path.write_bytes(original+b' changed')
                    self.assertNotEqual(before,proof_batch.execution_runtime_inputs())
                    path.write_bytes(original)
                    path.unlink()
                    with self.assertRaises((OSError,ValueError)):
                        proof_batch.execution_runtime_inputs()
                    path.write_bytes(original)
            with mock.patch.object(proof_batch,'MODULES',('bounded-writer',)), mock.patch.dict(os.environ,{},clear=True):
                with self.assertRaises(KeyError):proof_batch.execution_runtime_inputs()
            with mock.patch.object(proof_batch,'MODULES',('network',)):
                self.assertEqual(proof_batch.execution_runtime_inputs(),{})


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

    def test_external_raw_source_and_archive_are_required_for_reuse(self):
        work=self.base/'external-module-work';work.mkdir()
        archive=work/'baseline.tar.gz';archive.write_bytes(b'fixture source archive')
        code=('from pathlib import Path; import tempfile; '
              'p=Path(tempfile.mkdtemp(prefix="run-",dir='+repr(str(work))+')); '
              '(p/"wrong.rs").write_text("pub fn wrong() {}\\n"); '
              '(p/"original.llbc").write_text("actual raw fixture"); '
              'print("external fixture executed")')
        command=[sys.executable,'-c',code]
        def inputs(*_):return [work],{str(archive):receipts.sha(archive)}
        with mock.patch.object(proof_batch,'external_component_paths',side_effect=inputs):
            first=proof_batch.step('network',command,self.source)
            data=json.loads(Path(first['receipt']).read_text())
            raws=[Path(n) for n in data['evidence'] if n.endswith('wrong.rs')]
            self.assertEqual(len(raws),1)
            self.assertIn(str(archive),data['evidence'])
            self.assertTrue(proof_batch.step('network',command,self.source)['reused'])
            raws[0].write_text('corrupt actual wrong source')
            self.assertFalse(proof_batch.step('network',command,self.source)['reused'])
            archive.write_bytes(b'changed archive')
            self.assertFalse(proof_batch.step('network',command,self.source)['reused'])

    def test_new_required_external_evidence_cannot_reuse_an_unbound_record(self):
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])
        raw=self.base/'reviewed-stdlib.rs';raw.write_text('actual reviewed source')
        required={str(raw):receipts.sha(raw)}
        with mock.patch.object(proof_batch,'external_component_paths',return_value=([],required)):
            current=proof_batch.step('network',self.command,self.source)
            self.assertFalse(current['reused'])
            self.assertEqual(json.loads(Path(current['receipt']).read_text())['evidence'][str(raw)],receipts.sha(raw))
            self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])

    def test_external_evidence_change_during_run_never_publishes_success(self):
        raw=self.base/'reviewed-stdlib.rs';raw.write_text('before execution')
        expected=receipts.sha(raw)
        command=[sys.executable,'-c','from pathlib import Path;Path('+repr(str(raw))+').write_text("changed while running")']
        with mock.patch.object(proof_batch,'external_component_paths',return_value=([],{str(raw):expected})):
            with self.assertRaisesRegex(ValueError,'external raw evidence changed'):
                proof_batch.step('network',command,self.source)
            key=proof_batch.module_key('network',command,self.source)
            entry=Path(os.environ['SANDPROOF_FORMAL_CACHE'])/'modules/network'/receipts.digest(key)
            self.assertFalse((entry/'receipt.json').exists())

    def test_existing_fixed_output_is_snapshotted_and_required_for_reuse(self):
        relative='artifacts/existing/source.llbc'
        raw=self.source/relative;raw.parent.mkdir(parents=True);raw.write_text('original raw fixture')
        with mock.patch.object(proof_batch,'FIXED_EVIDENCE',{'network':(relative,)},create=True):
            row=proof_batch.step('network',self.command,self.source)
            snapshot=Path(row['receipt']).parent/'fixed-output'/relative
            self.assertTrue(snapshot.is_file(),'pre-existing raw output was omitted')
            self.assertEqual(snapshot.read_bytes(),raw.read_bytes())
            raw.unlink()
            self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])
            raw.write_text('original raw fixture');snapshot.unlink()
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

    def test_sealed_contract_archive_and_schema_changes_reexecute_the_module(self):
        # This fixture exercises the internal cache step, not native admission.
        seal=dict(schema=2,rules={},rust={},runtime={},commands={},contracts={},source_archives={})
        receipts.write_json(self.cache/'complete.json',seal)
        self.assertFalse(proof_batch.step('network',self.command,self.source)['reused'])
        self.assertTrue(proof_batch.step('network',self.command,self.source)['reused'])
        for field,value in [('contracts',{'verification/fixture.json':'1'*64}),
                            ('source_archives',{'original':'2'*64}),('schema',3)]:
            seal[field]=value;receipts.write_json(self.cache/'complete.json',seal)
            with self.subTest(field=field):
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
