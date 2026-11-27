import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools import ci_lookup_acceptance as wrapper, formal_builds, native_task
from tools.system_acceptance import digest, json_file, write_once


RUST_VARIANTS = ('original', 'candidate', 'key-only', 'restored')


class LookupCloseoutTests(unittest.TestCase):
    def setUp(self):
        self.addCleanup(os.umask, os.umask(0o077))
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name).resolve()
        self.checkout = self.root/'checkout'
        self.checkout.mkdir()
        self.observer = self.root/'installed-observer'
        self.observer.write_text('actual observation replaced in the test\n')
        self.candidate = dict(commit='a'*40, tree='b'*40)

    def run_fixture(self, output, terminal, native=None, *, lookup_only=False):
        output.mkdir()
        script = self.root/'original-driver.py'
        script.write_text(f'''import argparse, json, os, signal
from pathlib import Path
p = argparse.ArgumentParser()
p.add_argument('--output', type=Path, required=True)
p.add_argument('--base', required=True)
p.add_argument('--formal-lock', required=True)
p.add_argument('--reuse-native')
p.add_argument('--reuse-native-sha256')
p.add_argument('--lookup-only', action='store_true')
a = p.parse_args()
assert Path.cwd() == Path({str(self.checkout)!r})
assert a.base == 'reviewed-base'
assert a.formal_lock == '/installed/formal.lock'
assert a.lookup_only is {lookup_only!r}
assert (a.reuse_native, a.reuse_native_sha256) == ({None if native is None else native['path']!r},
                                                       {None if native is None else native['sha256']!r})
if not a.reuse_native:
    for variant in {RUST_VARIANTS!r}:
        native = a.output / 'original-native' / variant
        (native/'target').mkdir(parents=True)
        (native/'target/private-build').write_bytes(b'r'*4096)
        (native/'test.log').write_text('original Rust log')
for variant in ('correct', 'key-only'):
    project = a.output / ('formal-' + variant) / 'source'
    target = project / 'artifacts/extract-target'
    target.mkdir(parents=True)
    (target/'private-build').write_bytes(b'b'*4096)
    (project/'.lake').mkdir()
    (project/'.lake/preserved.olean').write_text('existing compiled Lean')
    (project/'actual.llbc').write_text('original LLBC')
result = dict(candidate={self.candidate!r}, complete={terminal == 0!r})
if a.lookup_only:
    result.update(complete=False, lookup_only_complete={terminal == 2!r})
path = a.output/('lookup-local-result.json' if a.lookup_only else 'module-result.json')
path.write_text(json.dumps(result))
print('logged by the original module executor', flush=True)
if {terminal} < 0:
    os.kill(os.getpid(), -{terminal})
raise SystemExit({terminal})
''')
        module = SimpleNamespace(clean_candidate=lambda: self.candidate, NATIVE_VARIANTS=RUST_VARIANTS)
        with (patch.object(wrapper, 'ROOT', self.checkout), patch.object(wrapper, 'DRIVER', script),
              patch.object(wrapper, 'lookup', return_value=module)):
            return wrapper.run(output, self.observer, 'reviewed-base', Path('/installed/formal.lock'),
                               native, lookup_only=lookup_only)

    def tangible_evidence(self, output, _invocation, terminal, _module):
        self.assertEqual(terminal['returncode'], 0)
        paths = [output/'module/module-result.json']
        for variant in ('correct', 'key-only'):
            project = output/'module'/f'formal-{variant}'/'source'
            paths.extend((project/'actual.llbc', project/'.lake/preserved.olean'))
        return {path: digest(path) for path in paths}

    def test_original_execution_then_cleanup_only_retry(self):
        output = self.root/'managed'
        reuse = dict(path='/original/lookup-native-receipt.json', sha256='a'*64)
        with (patch.object(wrapper, 'stage_files', side_effect=self.tangible_evidence),
              patch.object(native_task, 'observe', side_effect=ValueError('live reference'))):
            self.assertEqual(self.run_fixture(output, 0, reuse), 75)
        self.assertTrue(all((output/path).is_dir() for path in wrapper.TARGETS))
        original = self.tangible_evidence(output, None, {'returncode': 0}, None)
        original_log = digest(output/formal_builds.DIRECTORY/'native-task.log')
        with (patch.object(wrapper, 'stage_files', side_effect=self.tangible_evidence),
              patch.object(native_task, 'observe'), patch.object(formal_builds, 'execute') as execute,
              patch.object(wrapper, 'lookup', return_value=SimpleNamespace(clean_candidate=lambda: self.candidate))):
            result = wrapper.closeout(output, self.observer)
        execute.assert_not_called()
        self.assertEqual((result['target_count'], result['remaining_build_bytes']), (2, 0))
        self.assertTrue(all(not (output/path).exists() for path in wrapper.TARGETS))
        self.assertEqual({path: digest(path) for path in original}, original)
        self.assertEqual(digest(output/formal_builds.DIRECTORY/'native-task.log'), original_log)
        self.assertIn('logged by the original module', (output/formal_builds.DIRECTORY/'native-task.log').read_text())
        self.assertEqual(len(list(output.glob('module/formal-*/source/.lake/preserved.olean'))), 2)
        self.assertEqual(len(list(output.glob('module/formal-*/source/actual.llbc'))), 2)

    def test_fresh_run_retires_four_rust_builds_and_keeps_their_logs(self):
        output = self.root/'fresh'

        def readback(directory, invocation, terminal, module):
            evidence = self.tangible_evidence(directory, invocation, terminal, module)
            for variant in RUST_VARIANTS:
                path = directory/'module/original-native'/variant/'test.log'
                self.assertEqual(path.read_text(), 'original Rust log')
                evidence[path] = digest(path)
            return evidence

        with (patch.object(wrapper, 'stage_files', side_effect=readback), patch.object(native_task, 'observe')):
            self.assertEqual(self.run_fixture(output, 0), 0)
        result = json_file(output/formal_builds.DIRECTORY/'retired.json')
        self.assertEqual((result['target_count'], result['remaining_build_bytes']), (6, 0))
        for variant in RUST_VARIANTS:
            native = output/'module/original-native'/variant
            self.assertFalse((native/'target').exists())
            self.assertEqual((native/'test.log').read_text(), 'original Rust log')

    def test_known_failure_and_cancel_preserve_status_sigkill_preserves_targets(self):
        for code, shell in ((1, 1), (-15, 143), (137, 137)):
            with self.subTest(code=code), patch.object(native_task, 'observe'):
                output = self.root/str(code)
                self.assertEqual(self.run_fixture(output, code), shell)
                state = output/formal_builds.DIRECTORY
                self.assertEqual(json_file(state/'native-task-exit.json')['returncode'], code)
                self.assertIs(json_file(output/'module/module-result.json')['complete'], False)
                self.assertTrue(all((output/path).is_dir() == (code == 137) for path in wrapper.TARGETS))
                for variant in RUST_VARIANTS:
                    native = output/'module/original-native'/variant
                    self.assertEqual((native/'target').is_dir(), code == 137)
                    self.assertTrue((native/'test.log').is_file())

    def test_local_exit_two_and_receipt_survive_cleanup(self):
        output = self.root/'local'
        receipt = output/'module/lookup-local-result.json'

        def readback(directory, invocation, terminal, _module):
            self.assertEqual(directory, output)
            self.assertIs(invocation['lookup_only'], True)
            self.assertEqual(terminal['returncode'], 2)
            self.assertEqual(json_file(receipt), dict(candidate=self.candidate, complete=False, lookup_only_complete=True))
            return {receipt: digest(receipt)}

        native = dict(path='/original/lookup-native-receipt.json', sha256='a'*64)
        with (patch.object(wrapper, 'stage_files', side_effect=readback),
              patch.object(native_task, 'observe')):
            self.assertEqual(self.run_fixture(output, 2, native, lookup_only=True), 2)
        self.assertFalse((receipt.parent/'module-result.json').exists())
        result = json_file(output/formal_builds.DIRECTORY/'retired.json')
        self.assertEqual((result['returncode'], result['target_count'], result['remaining_build_bytes']), (2, 2, 0))
        self.assertTrue(all(not (output/path).exists() for path in wrapper.TARGETS))
        incomplete = self.root/'unreadable-local'
        with patch.object(wrapper, 'stage_files', side_effect=KeyError('native')):
            self.assertEqual(self.run_fixture(incomplete, 2, native, lookup_only=True), 2)
        self.assertTrue(all((incomplete/path).is_dir() for path in wrapper.TARGETS))
        self.assertFalse((incomplete/formal_builds.DIRECTORY/'retired.json').exists())

    def test_local_readback_requires_exact_pending_jobs_and_both_original_views(self):
        output = self.root/'local-evidence'
        mod = output/'module'
        mod.mkdir(parents=True)
        native_dir = self.root/'native-proof'
        (native_dir/'original-native').mkdir(parents=True)
        native_result = native_dir/'original-native/result.json'
        native_result.write_text('{}\n')
        native_log = native_dir/'original-native.log'
        native_log.write_text('original native executor\n')
        key = {'scoped_native_tools': True}
        native_receipt = native_dir/'lookup-native-receipt.json'
        write_once(native_receipt, dict(key=key, result_sha256=digest(native_result),
                                       launcher_log_sha256=digest(native_log)))
        invocation = dict(schema=1, candidate=self.candidate, lookup_only=True, base='reviewed',
                          native=dict(path=str(native_receipt), sha256=digest(native_receipt)))
        write_once(output/wrapper.INVOCATION, invocation)
        base = dict(commit='c'*40, tree='d'*40)
        paths = ['positive.lean.in', 'frozen.sha256']
        pending = [{'name': 'required-downstream-job'}]
        impact = {'pending_jobs': pending, 'unmapped': ['unmapped-dependent']}
        module = SimpleNamespace(MODULE='inbox-lookup', NATIVE_RECEIPT='lookup-native-receipt.json',
            changed_files=Mock(return_value=(base['commit'], paths)), local_lookup_impact=Mock(return_value=impact),
            load_native_reuse=Mock(return_value=({'commands': []}, {'result_sha256': digest(native_result)})))
        views = {case: dict(case=case) for case in wrapper.CASES}
        report = dict(schema=1, kind='inbox-lookup-only-proof', module=module.MODULE,
            complete=False, lookup_only_complete=True, full_stage1_acceptance=False, fresh_runs=False,
            candidate=self.candidate, formal=views,
            native=dict(receipt=str(native_receipt), use='REUSE', receipt_sha256=digest(native_receipt),
                        result_sha256=digest(native_result), original_candidate=base, source_differences=sorted(paths)),
            plan=dict(status='lookup-only', base_commit=base['commit'], changed_paths=paths,
                      native_reuse_paths=sorted(paths), pending_downstream=pending, unmapped_downstream=impact['unmapped'],
                      checks=['reuse-original-native', 'formal-correct', 'formal-key-only']))
        local = mod/'lookup-local-result.json'
        local.write_text(json.dumps(report))
        terminal = {'returncode': 2}
        with patch.object(wrapper, 'proof_files') as proofs:
            files = wrapper.stage_files(output, invocation, terminal, module)
            self.assertIn(local, files)
            self.assertEqual([call.args[2] for call in proofs.call_args_list], list(wrapper.CASES))
            self.assertTrue(all(call.kwargs['permitted'] == sorted(paths) for call in proofs.call_args_list))
            module.load_native_reuse.assert_called_once_with(native_receipt, digest(native_receipt), base, key, [])
            report['plan']['pending_downstream'] = []
            local.write_text(json.dumps(report))
            with self.assertRaisesRegex(ValueError, 'pending downstream list differs'):
                wrapper.stage_files(output, invocation, terminal, module)
            local.unlink()
            with self.assertRaises(FileNotFoundError):
                wrapper.stage_files(output, invocation, terminal, module)
            (mod/'module-result.json').write_text('{}\n')
            with self.assertRaisesRegex(ValueError, 'different acceptance scope'):
                wrapper.stage_files(output, invocation, terminal, module)

    def test_saved_proof_readback_rejects_changed_type_and_llbc(self):
        folder = self.root/'formal-key-only'
        project = folder/'source'
        real_module = wrapper.lookup()
        _, library, name = real_module.VARIANTS['key-only']
        theorem = 'SandboxModel.Stage1Protocol.' + name
        model = project/real_module.MODEL
        model.parent.mkdir(parents=True)
        model.write_text('actual Rust source\n')
        (project/'artifacts').mkdir()
        llbc = {label: project/f'artifacts/{label}.llbc' for label in ('original', 'candidate')}
        for path in llbc.values():
            path.write_text('original raw bytes\n')
        proof = 'Proof.lean'
        (project/proof).write_text(f'#audit_controller_proof {theorem}\n')
        check = project/'artifacts/ModuleAcceptance.lean'
        check.write_text(f'import SandboxModel.Lifecycle.Protocol.{library}\n#check ({theorem} : ¬ '
                         + real_module.THEOREM_TYPE + real_module.LOOKUP + ')\n')
        type_log = folder/'contract-type.log'
        type_log.write_text('validated type\n')
        kernel = folder/'kernel-0.log'
        kernel.touch()
        physical = folder/'physical-files'
        physical.mkdir()
        request = dict(tool='lean_diagnostic_messages', arguments=dict(file_path=proof),
                       expected_sha256=digest(project/proof))
        (physical/'requests.jsonl').write_text(json.dumps(request)+'\n'+json.dumps({'tool': 'exit'})+'\n')
        response = dict(request=request, stale=False,
            revision={'file_sha256': digest(project/proof)}, result=dict(success=True, partial=False, timed_out=False,
            items=[dict(message=f'Checked {theorem}: 1 standard logical dependencies')]))
        (physical/'service.log').write_text(json.dumps(response)+'\n')
        (folder/'actual-service.stdout').write_text(f'Checked {theorem}: 1 standard logical dependencies\n')
        (folder.parent/'formal-key-only.log').write_text('actual executor\n')
        write_once(folder/'result.json', dict(kind='inbox-lookup-formal-development', complete=True,
            source_commit=self.candidate['commit'], variant='key-only', candidate_model_sha256=digest(model),
            llbc={label: dict(path=str(path.relative_to(project)), sha256=digest(path)) for label, path in llbc.items()},
            actual=dict(files={proof: digest(project/proof)}, audits=[theorem],
                        service_log_sha256=digest(physical/'service.log')), raw_generated={},
            commands=[dict(label='kernel-0', argv=['lake', 'env', 'lean', proof], cwd=str(project),
                           exit_code=0, log_sha256=digest(kernel))]))
        view = dict(theorem=theorem, property_rejected=True, rust_formal_inputs_match=True, registry_content_match=True,
                    result_sha256=digest(folder/'result.json'), candidate_model_sha256=digest(model),
                    contract_type_source_sha256=digest(check), contract_type_log_sha256=digest(type_log))
        native = {'variants': {'key-only': {'inputs': {real_module.MODEL: digest(model)}}}}

        def validate():
            return wrapper.proof_files(real_module, view, 'key-only', folder, self.candidate, native, {})

        with patch('subprocess.run', side_effect=AssertionError('readback must not run Lean or Cargo')):
            self.assertIsNone(validate())
            with patch.dict(view, contract_type_source_sha256=digest(model)):
                with self.assertRaisesRegex(ValueError, 'evidence changed'):
                    validate()
            llbc['candidate'].write_text('changed original\n')
            with self.assertRaisesRegex(ValueError, 'evidence changed'):
                validate()


if __name__ == '__main__':
    unittest.main()
