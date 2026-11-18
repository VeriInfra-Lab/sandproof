"""Admit the actual gate mutation/output contract without Cargo or a VM."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('uncommitted_stop_native',
    ROOT / 'verification/experiments/uncommitted-stop/native.py')
native = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native)


class StopGateMutationTests(unittest.TestCase):
    def source(self, root):
        for name in (native.SOURCE,native.FIXTURE):
            path=root/name;path.parent.mkdir(parents=True,exist_ok=True)
            path.write_bytes((ROOT/name).read_bytes())
        return root/native.SOURCE

    def test_only_whole_snapshot_gate_changes_and_original_source_checks_and_tests_remain(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);path=self.source(root);text=path.read_text()
            fixture=(root/native.FIXTURE).read_bytes();line=native.rejection_line(root)
            self.assertEqual(fixture.splitlines()[line-1].strip(),b'assert_eq!(')
            native.mutate(root)
            self.assertEqual(path.read_text(),text.replace(native.BEFORE,native.AFTER,1))
            self.assertEqual((root/native.FIXTURE).read_bytes(),fixture)
            self.assertEqual(native.fixture_digest(root),hashlib.sha256(fixture).hexdigest())
            self.assertEqual(native.rejection_line(root),line)
            self.assertIn('let (_, plan) = current_plan(world, sandbox, record)?;',path.read_text())
            with self.assertRaises(ValueError):native.mutate(root)

    def test_missing_duplicate_or_moved_gate_never_selects_another_region(self):
        for fault in ('missing','duplicate','moved'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);path=self.source(root);text=path.read_text()
                if fault=='missing':text=text.replace(native.BEFORE,'')
                if fault=='duplicate':text=text.replace(native.BEFORE,native.BEFORE*2)
                if fault=='moved':text=text.replace(native.BEFORE,'')+native.BEFORE
                path.write_text(text)
                with self.assertRaises(ValueError):native.mutate(root)
                self.assertEqual(path.read_text(),text)

    def test_exact_original_equality_assertion_location_cannot_be_replaced(self):
        marker='    assert_eq!(\n        handle_worker(&dispatched, &id),\n        Err(WorkerReject::SnapshotNotReady)\n    );'
        for fault in ('property','assertion','duplicate'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as directory:
                root=Path(directory);self.source(root);path=root/native.FIXTURE;text=path.read_text()
                if fault=='property':text=text.replace(native.PROPERTY.rsplit('::',1)[1],'other_test')
                if fault=='assertion':text=text.replace(marker,'')
                if fault=='duplicate':text=text.replace(marker,marker+'\n'+marker)
                path.write_text(text)
                with self.assertRaises(ValueError):native.rejection_line(root)


class StopGateExecutionTests(unittest.TestCase):
    def output(self,rejected,line=405):
        text='\nrunning 1 test\ntest '+native.PROPERTY+(' ... FAILED\n' if rejected else ' ... ok\n')
        if rejected:
            text+=(f"thread '{native.PROPERTY}' panicked at {native.FIXTURE}:{line}:5:\n"
                'assertion `left == right` failed\n'
                '  left: Ok(WorkerDecision { event: StopVm { sandbox: SandboxId(1), vm: VmId(1) }, next: World {} })\n'
                ' right: Err(SnapshotNotReady)\n')
        text+=(f'test result: {"FAILED" if rejected else "ok"}. '
            f'{0 if rejected else 1} passed; {1 if rejected else 0} failed; '
            '0 ignored; 0 measured; 100 filtered out; finished in 0.1s\n')
        return text.encode()

    def test_actual_stop_success_must_be_rejected_at_the_original_worker_assertion(self):
        native.checked_execution(self.output(False),b'',0,native.PROPERTY,False,405)
        native.checked_execution(self.output(True),b'',101,native.PROPERTY,True,405)
        for name in native.TESTS[1:]:
            native.checked_execution(self.output(False).replace(native.PROPERTY.encode(),name.encode()),
                b'',0,name,False,405)
        self.assertEqual(len(set(native.TESTS)),3)

    def test_other_error_wrong_location_compile_failure_skip_or_timeout_is_not_the_counterexample(self):
        bad=self.output(True)
        for out,code in ((b'',101),(bad,124),(bad,0),
            (bad.replace(b'running 1 test',b'running 0 tests'),101),
            (bad.replace(b'0 ignored;',b'1 ignored;'),101),
            (bad.replace(b':405:5:',b':406:5:'),101),
            (bad.replace(str(native.FIXTURE).encode(),b'other.rs'),101),
            (bad.replace(b'assertion `left == right` failed',b'called Result::unwrap()'),101),
            (bad.replace(b'event: StopVm',b'event: FreezeVm'),101),
            (bad.replace(b'left: Ok(WorkerDecision',b'left: Err(WorkerDecision'),101),
            (bad.replace(b'right: Err(SnapshotNotReady)',b'right: Err(WrongPhase)'),101),
            (bad.replace(b'right: Err(SnapshotNotReady)',b'right: Err(WrongPhase)')
             + b'\nright: Err(SnapshotNotReady)\n',101),
            (bad.replace(native.PROPERTY.encode(),b'other_test'),101),(self.output(False),0)):
            with self.subTest(output=out,code=code),self.assertRaises(ValueError):
                native.checked_execution(out,b'',code,native.PROPERTY,True,405)
        with self.assertRaises(ValueError):native.checked_execution(bad,b'',101,native.PROPERTY,False,405)

    def test_core_artifact_uses_the_same_strict_shared_package_and_profile_admission(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);project=root/'source';target=root/'cache';target.mkdir()
            binary=target/'core-tests';binary.write_bytes(b'controlled compiler metadata fixture, not a Rust binary')
            record=dict(reason='compiler-artifact',executable=str(binary),
                package_id='path+file://'+str(project/native.PACKAGE_DIR)+'#sandbox-controller-core@0.1.0',
                manifest_path=str(project/native.PACKAGE_DIR/'Cargo.toml'),
                target=dict(src_path=str(project/native.PACKAGE_DIR/'src/lib.rs'),name=native.CRATE,kind=['lib']),
                profile=dict(test=True,debug_assertions=True,overflow_checks=True),fresh=False)
            def admit(value):
                return native.runner.checked_binary(json.dumps(value).encode(),project,target,
                    package_dir=native.PACKAGE_DIR,crate=native.CRATE)
            self.assertEqual(admit(record)['path'],str(binary))
            for field in ('test','debug_assertions','overflow_checks'):
                for value in (False,1,'true',None):
                    changed=dict(record,profile=dict(record['profile'],**{field:value}))
                    with self.subTest(field=field,value=value),self.assertRaises(ValueError):admit(changed)
            changed=dict(record,target=dict(record['target'],name='sandproof_controller_service'))
            with self.assertRaises(ValueError):admit(changed)


if __name__ == '__main__':
    unittest.main()
