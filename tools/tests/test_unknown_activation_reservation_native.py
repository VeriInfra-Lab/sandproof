"""Mutation and admission checks; controlled output is not Rust execution."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('unknown_activation_reservation_native',
    ROOT / 'verification/experiments/unknown-activation-reservation/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class UnknownActivationMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE,native.FIXTURE):
            path=root/name;path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes((ROOT/name).read_bytes())
        return root/native.SOURCE

    def test_one_unknown_retry_policy_keeps_identity_freshness_tests_and_other_handlers(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);path=self.source(root);before=path.read_text()
            fixture=(root/native.FIXTURE).read_bytes();digest=native.fixture_digest(root)
            line=native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line-1].strip(),b'assert!(')
            native.mutate(root);after=path.read_text()
            self.assertEqual(after,before.replace(native.BEFORE,native.AFTER,1))
            self.assertEqual((root/native.FIXTURE).read_bytes(),fixture)
            self.assertEqual(native.fixture_digest(root),digest)
            self.assertEqual(native.rejection_line(root),line)
            self.assertEqual(after[:after.index(native.FIRST)],before[:before.index(native.FIRST)])
            self.assertEqual(after[after.index(native.LAST):],before[before.index(native.LAST):])
            first='    if !world.controller_up {'
            old=before[before.index(first,before.index(native.FIRST)):before.index(native.BEFORE)]
            self.assertIn(old,after)
            tail='    if plan.restore_from != record.restore_from {'
            self.assertEqual(after[after.index(tail):],before[before.index(tail):])
            with self.assertRaises(ValueError):native.mutate(root)
            self.assertEqual(path.read_text(),after)

    def test_missing_duplicate_or_moved_gate_is_rejected_before_any_write(self):
        for fault in ('missing','duplicate','moved'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);path=self.source(root);text=path.read_text()
                if fault=='missing':text=text.replace(native.BEFORE,'',1)
                if fault=='duplicate':text=text.replace(native.BEFORE,native.BEFORE*2,1)
                if fault=='moved':text=text.replace(native.BEFORE,'',1)+native.BEFORE
                path.write_text(text)
                with self.assertRaises(ValueError):native.mutate(root)
                self.assertEqual(path.read_text(),text)

    def test_whole_scheduler_test_body_is_required_and_bound_independently(self):
        for fault in ('missing','duplicate','test-change','resume-change'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);path=self.source(root);digest=native.fixture_digest(root)
                text=path.read_text()
                if fault=='missing':text=text.replace(native.TEST_BODY,'',1)
                if fault=='duplicate':text+=native.TEST_BODY
                if fault in ('missing','duplicate'):
                    path.write_text(text)
                    with self.assertRaises(ValueError):native.mutate(root)
                    self.assertEqual(path.read_text(),text)
                else:
                    changed=path if fault=='test-change' else root/native.FIXTURE
                    changed.write_bytes(changed.read_bytes()+b'\n// changed original test input\n')
                    self.assertNotEqual(native.fixture_digest(root),digest)

    def test_only_original_final_reservation_refusal_can_supply_the_panic_line(self):
        for fault in ('missing','duplicate','moved','other-test'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);self.source(root);path=root/native.FIXTURE;text=path.read_text()
                if fault=='missing':text=text.replace(native.MARKER,'',1)
                if fault=='duplicate':text=text.replace(native.MARKER,native.MARKER*2,1)
                if fault=='moved':text=text.replace(native.MARKER,'',1)+native.MARKER
                if fault=='other-test':text=text.replace(native.PROPERTY.rsplit('::',1)[1],'other_test',1)
                path.write_text(text)
                with self.assertRaises(ValueError):native.rejection_line(root)


class UnknownActivationExecutionTests(unittest.TestCase):
    def output(self,rejected,name=None,line=684):
        name=native.PROPERTY if name is None else name
        text='\nrunning 1 test\ntest '+name+(' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text+=(f"thread '{name}' panicked at {native.FIXTURE}:{line}:5:\n"
                'assertion failed: handle_controller(&world, ControllerInput::Scheduler('
                'SchedulerInput::ReserveStartup(OperationKey { sandbox: SID, operation: OperationId(3) }))).is_err()\n')
        text+=(f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 900 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_original_refusal_and_four_legal_controls_share_the_same_execution_admission(self):
        native.checked_execution(self.output(False),b'',0,native.PROPERTY,False,684)
        native.checked_execution(self.output(True),b'',101,native.PROPERTY,True,684)
        self.assertEqual(len(set(native.TESTS)),5)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False,name),b'',0,name,False,684)
            with self.assertRaises(ValueError):
                native.checked_execution(self.output(True,name),b'',101,name,True,684)

    def test_earlier_rejection_other_panic_compile_error_skip_timeout_or_foreign_test_is_not_evidence(self):
        good=self.output(True)
        for out,code in ((b'',101),(good,124),(good,0),
                (good.replace(b'running 1 test',b'running 0 tests'),101),
                (good.replace(b'0 ignored;',b'1 ignored;'),101),
                (good.replace(b':684:5:',b':683:5:'),101),
                (good.replace(str(native.FIXTURE).encode(),b'other.rs'),101),
                (good.replace(b'assertion failed:',b'called Result::unwrap(): CorruptStore'),101),
                (good.replace(native.ASSERTION,b'SchedulerInput::DispatchPrepare'),101),
                (good.replace(b'.is_err()',b'.is_ok()'),101),
                (good.replace(b'.is_err()',b'.is_ok()')+b'\n.is_err()\n',101),
                (good.replace(native.PROPERTY.encode(),b'other_test'),101),(self.output(False),0)):
            with self.subTest(code=code,output=out),self.assertRaises(ValueError):
                native.checked_execution(out,b'',code,native.PROPERTY,True,684)

    def test_real_compiler_artifact_must_be_the_fresh_core_libtest_with_all_checks_enabled(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);project=root/'source';target=root/'cache';target.mkdir()
            binary=target/'core-tests';binary.write_bytes(b'controlled artifact record; not a Rust program')
            row=dict(reason='compiler-artifact',executable=str(binary),
                package_id='path+file://'+str(project/native.PACKAGE_DIR)+'#sandbox-controller-core@0.1.0',
                manifest_path=str(project/native.PACKAGE_DIR/'Cargo.toml'),
                target=dict(src_path=str(project/native.PACKAGE_DIR/'src/lib.rs'),name=native.CRATE,kind=['lib']),
                profile=dict(test=True,debug_assertions=True,overflow_checks=True),fresh=False)
            def admit(record):
                return native.runner.checked_binary(json.dumps(record).encode(),project,target,
                    package_dir=native.PACKAGE_DIR,crate=native.CRATE)
            self.assertEqual(admit(row)['path'],str(binary))
            for key in ('test','debug_assertions','overflow_checks'):
                for value in (False,1,'true',None):
                    with self.subTest(key=key,value=value),self.assertRaises(ValueError):
                        admit(dict(row,profile=dict(row['profile'],**{key:value})))
            for changed in (dict(row,fresh=True),dict(row,target=dict(row['target'],name='sandproof_controller_service'))):
                with self.assertRaises(ValueError):admit(changed)


if __name__=='__main__':
    unittest.main()
