"""Admission and scheduling regressions; no compiler, VM or proof is launched."""
import ast
import copy
import fcntl
import io
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sys
import tarfile
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('stage1_registered', HERE/'registered.py')
registered = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registered)


class CreateDelivery(unittest.TestCase):
    def plan(self):
        return registered.formal_plan(Path('/unused'), profile='create-delivery')

    def result(self, job):
        rows = {}
        for case, recipe in job['views'].items():
            files = {n:'a'*64 for n in (recipe['files'] or recipe['seeds'])}
            actual = dict(files=files, audits=list(recipe['audits']), service_log_sha256='b'*64)
            rows[case] = dict(complete=True, property_rejected=recipe['property_rejected'],
                             files=files, audits=list(recipe['audits']), actual=actual)
        return dict(kind=job['expected_kind'], complete=True, prepare_only=False,
            all_three_variants_complete=True, all_variants_complete=True,
            **{'views' if job['name'] == 'adopted-create' else 'variants':rows})

    def test_fixed_profile_preserves_default_full_and_every_view(self):
        with patch.dict(os.environ, SANDPROOF_STAGE1_PROFILE='create-delivery',
                        SANDPROOF_STAGE1_ONLY='create-selection'):
            full = registered.formal_plan(Path('/unused'))
            self.assertEqual(len(full), 138)
            self.assertEqual(len(registered.native_labels()), 53)
        jobs = self.plan()
        definition = registered.create_profile()
        self.assertEqual([j['name'] for j in jobs], list(registered.CREATE_CASES))
        self.assertEqual(len(jobs), 12)
        self.assertEqual(sum(len(j['cases']) for j in jobs), 42)
        self.assertEqual(sum(len(p['native_views']) for p in definition['packages']), 66)
        self.assertTrue(all(j['actual_files'] for j in jobs))
        for job in jobs:
            self.assertIn(job, [dict(j, profile='create-delivery') for j in full])
            self.assertNotIn('--prepare-only', job['args'])
        self.assertEqual(len(registered.native_labels('create-delivery')), 12)
        with self.assertRaises(ValueError):
            registered.formal_plan(Path('/unused'), profile='selection-only')

    def test_each_formal_view_requires_files_audits_and_its_own_property_outcome(self):
        for job in self.plan():
            good = self.result(job)
            registered._check_actual(good, job)
            field = 'views' if job['name'] == 'adopted-create' else 'variants'
            for case in job['cases']:
                for defect in ('omit', 'audit', 'duplicate-audit', 'file', 'property', 'incomplete'):
                    bad = copy.deepcopy(good)
                    row = bad[field][case]
                    if defect == 'omit':
                        del bad[field][case]
                    elif defect == 'audit':
                        row['actual']['audits'].pop()
                    elif defect == 'duplicate-audit':
                        row['actual']['audits'].append(row['actual']['audits'][0])
                    elif defect == 'file':
                        row['actual']['files'].pop(next(iter(row['actual']['files'])))
                    elif defect == 'property':
                        row['property_rejected'] = not row['property_rejected']
                    else:
                        row['complete'] = False
                    with self.subTest(package=job['name'], case=case, defect=defect), self.assertRaises(ValueError):
                        registered._check_actual(bad, job)
            for defect in ('prepare_only', 'selected_variant', 'kind'):
                bad = copy.deepcopy(good)
                bad[defect] = True if defect == 'prepare_only' else 'actual' if defect == 'selected_variant' else 'old-candidate-kind'
                with self.subTest(package=job['name'], defect=defect), self.assertRaises(ValueError):
                    registered._check_actual(bad, job)

    def test_native_property_rejection_is_not_a_mutant_clippy_exit(self):
        definition = registered.create_profile()
        for package in definition['packages']:
            expected = registered._create_native_plan(package['name'], registered._read(
                registered.ROOT/registered.HERE/package['name']/'inventory.json'))
            self.assertEqual(tuple(package['native_views']), tuple(expected))
            self.assertTrue(all(bool(v['expected_failures']) for k,v in expected.items()
                if k not in (next(iter(expected)), registered.CREATE_RESTORED[package['name']])))
        high = next(p for p in definition['packages'] if p['name'] == 'create-high-water')
        self.assertTrue(high['native_views']['wrong-seed']['expected_clippy_errors'])
        self.assertTrue(high['native_views']['wrong-seed']['expected_failures'])
        successor = next(p for p in definition['packages'] if p['name']=='create-owner-successor')
        adopted = next(p for p in definition['packages'] if p['name']=='adopted-create')
        self.assertEqual(len(successor['native_views']['wrong-overflow']['expected_clippy_errors']),2)
        self.assertEqual(successor['native_views']['wrong-overflow']['expected_clippy_errors'],
            adopted['native_views']['successor-wrong-overflow']['expected_clippy_errors'])
        for package in definition['packages']:
            for case in (next(iter(package['native_views'])),registered.CREATE_RESTORED[package['name']]):
                self.assertFalse(package['native_views'][case]['expected_clippy_errors'])

    def test_exact_mutant_clippy_keeps_duplicates_and_is_never_a_property_witness(self):
        first=dict(code='clippy::unnecessary_wraps',message='synthetic result diagnostic',source='Source.rs',line=5)
        second=dict(code='clippy::cast_possible_wrap',message='synthetic cast diagnostic',source='Source.rs',line=6)
        expected=[first,first,second,second]
        def document(error):
            return dict(reason='compiler-message',message=dict(level='error',code=dict(code=error['code']),
                message=error['message'],spans=[dict(is_primary=True,file_name=error['source'],line_start=error['line'])]))
        messages=[document(e) for e in (second,first,second,first)]
        def text(rows):
            return '\n'.join(json.dumps(r) for r in rows)
        row=dict(exit_code=101)
        registered.check_native_clippy(row,text(messages),Path('/project'),expected)
        self.assertEqual(len(row['clippy_errors']),4)
        self.assertNotIn('property_rejected',row)
        self.assertTrue(row['expected_mutant_lint_rejection'])
        for defect in ('missing','extra','code','message','source','line','primary'):
            bad=copy.deepcopy(messages)
            if defect=='missing':bad.pop()
            elif defect=='extra':bad.append(copy.deepcopy(bad[0]))
            elif defect=='code':bad[0]['message']['code']['code']='unreviewed-lint'
            elif defect=='message':bad[0]['message']['message']='different diagnostic'
            elif defect=='source':bad[0]['message']['spans'][0]['file_name']='Other.rs'
            elif defect=='line':bad[0]['message']['spans'][0]['line_start']=99
            else:bad[0]['message']['spans'].append(copy.deepcopy(bad[0]['message']['spans'][0]))
            with self.subTest(defect=defect),self.assertRaises(ValueError):
                registered.check_native_clippy(dict(exit_code=101),text(bad),Path('/project'),expected)
        for code,observed,wanted in ((0,messages,expected),(101,[],[]),(101,messages,[])):
            with self.assertRaises(ValueError):
                registered.check_native_clippy(dict(exit_code=code),text(observed),Path('/project'),wanted)
        clean=dict(exit_code=0)
        registered.check_native_clippy(clean,'ordinary compiler progress',Path('/project'),[])
        self.assertFalse(clean['expected_mutant_lint_rejection'])

    def test_native_original_logs_and_assertions_are_required_even_with_complete_true(self):
        for name in ('create-deadline','create-attachment-flow'):
            with self.subTest(package=name):
                self.native_log_fixture(name)

    def native_log_fixture(self, name):
        plan = registered._create_native_plan(name, registered._read(
            registered.ROOT/registered.HERE/name/'inventory.json'))
        with tempfile.TemporaryDirectory() as temporary:
            folder = Path(temporary)
            result = dict(kind=name+'-native-development', complete=True,
                inputs={'Source.rs':'a'*64, 'unrelated-report.md':'original-report'}, variants={}, commands=[])
            for variant, recipe in plan.items():
                (folder/variant).mkdir()
                result['variants'][variant] = dict(complete=True, inputs={'Source.rs':'derived-source'})
                labels = ('format-instrumentation','format','test','clippy') if name=='create-attachment-flow' else ('format','test','clippy')
                for label in labels:
                    failures = recipe['expected_failures'] if label == 'test' else []
                    errors = recipe.get('expected_clippy_errors',[]) if label=='clippy' else []
                    count = recipe['test_count']
                    text = 'synthetic admission fixture; no compiler was run\n'
                    if label == 'test':
                        text += '\nfailures:\n'+''.join('    controller::fixture::'+n+'\n' for n in failures)
                        text += f'\ntest result: {count-len(failures)} passed; {len(failures)} failed; 0 ignored;\n'
                    log = folder/variant/(label+'.log')
                    log.write_text(text)
                    result['commands'].append(dict(variant=variant, label=label,
                        exit_code=101 if failures or errors else 0, property_rejected=bool(failures),
                        clippy_errors=copy.deepcopy(errors),expected_mutant_lint_rejection=bool(errors),
                        log_sha256=registered._sha(log)))
            shared = SimpleNamespace(rust_inputs=lambda root:{'Source.rs':'a'*64},
                rules=lambda root:{}, contract_inputs=lambda root:{})
            with patch.object(registered, '_shared', return_value=shared):
                registered._check_create_native_result(registered.ROOT, name, folder, result)
                wrong = next(v for v,r in plan.items() if r['expected_failures'])
                defects = ['source','missing-view','clippy-instead-of-property','missing-command']
                if name=='create-attachment-flow':
                    defects += ['missing-preparation','bad-preparation-exit','preparation-is-not-property']
                if any(r.get('expected_clippy_errors') for r in plan.values()):
                    defects += ['missing-lint-diagnostic']
                for defect in defects:
                    bad = copy.deepcopy(result)
                    if defect == 'source':
                        bad['inputs']['Source.rs'] = 'changed'
                    elif defect == 'missing-view':
                        del bad['variants'][wrong]
                    elif defect == 'clippy-instead-of-property':
                        row = next(r for r in bad['commands'] if r['variant']==wrong and r['label']=='test')
                        row['property_rejected'] = False
                    elif defect=='missing-preparation':
                        bad['commands'].pop(0)
                    elif defect=='bad-preparation-exit':
                        bad['commands'][0]['exit_code']=101
                    elif defect=='preparation-is-not-property':
                        bad['commands'][0]['property_rejected']=True
                    elif defect=='missing-lint-diagnostic':
                        next(r for r in bad['commands'] if r['clippy_errors'])['clippy_errors'].pop()
                    else:
                        bad['commands'].pop()
                    with self.subTest(defect=defect), self.assertRaises(ValueError):
                        registered._check_create_native_result(registered.ROOT, name, folder, bad)
                (folder/'actual/test.log').write_text('corrupted original log')
                with self.assertRaisesRegex(ValueError, 'original Create native log'):
                    registered._check_create_native_result(registered.ROOT, name, folder, result)

    def test_create_worker_counts_and_four_cpu_slots_are_explicit(self):
        self.assertEqual(registered._execution_options(profile='create-delivery')['workers'], 1)
        self.assertEqual(registered._execution_options()['workers'], 1)
        for count in (8, 12):
            self.assertEqual(registered._execution_options(count, profile='create-delivery')['workers'], count)
            with self.assertRaises(ValueError):
                registered._execution_options(count)
            with patch.dict(os.environ, SANDPROOF_STAGE1_JOBS=str(count)):
                with self.assertRaises(ValueError):
                    registered._execution_options()
        for jobs in (1, 2, 4, 8, 12):
            slots = registered._cpu_slots('create-delivery', jobs, range(8, 8+4*jobs))
            self.assertEqual(len(slots), jobs)
            self.assertTrue(all(len(slot) == 4 for slot in slots))
            self.assertEqual(len({n for slot in slots for n in slot}), 4*jobs)
            with self.assertRaisesRegex(ValueError, 'admitted CPUs'):
                registered._cpu_slots('create-delivery', jobs, range(4*jobs-1))

    def test_child_bootstrap_checks_actual_affinity_before_exec(self):
        with tempfile.TemporaryDirectory() as temporary:
            admission = Path(temporary)/'cpu.json'
            state = {'cpus':set(range(48))}
            executed = []
            def set_affinity(pid, cpus):
                self.assertEqual(pid, 0)
                state['cpus'] = set(cpus)
            def execute(path, args):
                executed.append((path, args))
            fake_os = SimpleNamespace(sched_getaffinity=lambda pid:set(state['cpus']),
                sched_setaffinity=set_affinity, getpid=lambda:123, execv=execute)
            argv = ['bootstrap','8,9,10,11',str(admission),'/fixed/python','/original/driver.py','--lock-fd','9']
            with patch.dict(sys.modules, os=fake_os), patch.object(sys, 'argv', argv):
                exec(registered.CPU_BOOTSTRAP, {})
            self.assertEqual(state['cpus'], {8,9,10,11})
            self.assertEqual(executed, [('/fixed/python',argv[3:])])
            self.assertEqual(registered._check_cpu_admission(admission,[8,9,10,11])['actual'],[8,9,10,11])
            bad = registered._read(admission);bad['actual']=[8,9,10,12]
            admission.write_text(json.dumps(bad))
            with self.assertRaises(ValueError):
                registered._check_cpu_admission(admission)
            # The parent's subprocess call preserves the same already-open FD
            # through the fresh child/exec boundary, without preexec_fn.
            with patch.object(registered.subprocess,'run') as run, \
                    patch.object(registered.os,'sched_setaffinity',create=True) as parent_set:
                registered._invoke(Path(temporary),Path('driver.py'),['--lock-fd','9'],
                    pass_fds=(9,),cpu_affinity=(8,9,10,11),cpu_admission=admission)
            self.assertEqual(run.call_args.kwargs['pass_fds'],(9,))
            self.assertNotIn('preexec_fn',run.call_args.kwargs)
            parent_set.assert_not_called()
            self.assertEqual(run.call_args.args[0][1:4],['-c',registered.CPU_BOOTSTRAP,'8,9,10,11'])

    def test_create_pool_uses_distinct_slots_and_stops_dispatch_after_first_failure(self):
        for workers, fail in ((12,False),(4,True)):
            with self.subTest(workers=workers,fail=fail), tempfile.TemporaryDirectory() as temporary:
                output=Path(temporary);plan=registered.formal_plan(output,profile='create-delivery')
                barrier=threading.Barrier(workers);release=threading.Event();started={};mutex=threading.Lock()
                def launch(root,job,directory,paths,runtime_seed=None,cpu_affinity=None):
                    with mutex:started[job['name']]=cpu_affinity
                    directory.mkdir(parents=True);barrier.wait(timeout=10)
                    if fail and job['name']==plan[0]['name']:
                        raise ValueError('controlled first Create failure')
                    if fail and not release.wait(timeout=10):
                        raise ValueError('parent did not publish first failure')
                    folder=Path(job['output']);folder.mkdir()
                    result=folder/'result.json';result.write_text('{"synthetic_scheduler_only":true}')
                    log=directory/'command.log';log.write_text('synthetic scheduling; no compiler or proof\n')
                    return dict(name=job['name'],argv=['synthetic'],result_path=str(result),
                                seconds=0,command_log=str(log))
                def progress(rows,failures,misses):
                    if failures:release.set()
                with patch.object(registered.os,'sched_getaffinity',return_value=set(range(48)),create=True), \
                        patch.object(registered,'_launch_job',side_effect=launch), \
                        patch.object(registered,'_check_actual'), \
                        patch.object(registered,'_checkpoint_evidence',return_value={}), \
                        patch.object(registered,'_execution_helper',return_value=SimpleNamespace()):
                    rows,failures=registered.execute_formal_jobs(registered.ROOT,output,plan,
                        {},{},jobs=workers,profile='create-delivery',progress=progress)
                self.assertEqual(len(started), workers)
                self.assertEqual(len({n for slot in started.values() for n in slot}),workers*4)
                self.assertTrue(all(len(slot)==4 for slot in started.values()))
                self.assertEqual(bool(failures),fail)
                self.assertEqual(len(rows), workers-1 if fail else workers)

    def test_source_capsule_and_physical_leaf_bytes_are_both_required(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)/'source'
            root.mkdir()
            archive = Path(temporary)/'sources.tar'
            files = {}
            with tarfile.open(archive,'w') as tar:
                for name in registered.CREATE_SOURCE_MEMBERS:
                    data = ('-- synthetic readonly source '+name+'\n').encode()
                    info=tarfile.TarInfo(name);info.size=len(data);tar.addfile(info,io.BytesIO(data))
                    path=root/'.tools'/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(data)
                    files[name]=dict(sha256=registered._sha(path),bytes=len(data))
            definition=root/registered.HERE/'native-source-inputs.json';definition.parent.mkdir(parents=True)
            model=dict(schema=1,kind='stage1-create-fixed-readonly-sources',environment=registered.CREATE_SOURCE_ENV,
                archive_sha256=registered._sha(archive),archive_bytes=archive.stat().st_size,files=files,contains_tool_binaries=False)
            definition.write_text(json.dumps(model))
            with patch.dict(os.environ,{registered.CREATE_SOURCE_ENV:str(archive)}):
                registered.check_create_sources(root)
                with patch.dict(os.environ,{registered.CREATE_SOURCE_ENV:''}), self.assertRaises(ValueError):
                    registered.check_create_sources(root)
                leaf=root/'.tools'/registered.CREATE_SOURCE_MEMBERS[0];original=leaf.read_bytes()
                leaf.write_bytes(original+b'changed')
                with self.assertRaisesRegex(ValueError,'source bytes differ'):
                    registered.check_create_sources(root)
                leaf.write_bytes(original)
                with patch.dict(os.environ,SANDPROOF_ACCEPTANCE_ROLE='formal'), self.assertRaises(ValueError):
                    registered.prepare_create_sources(root)
                with patch.dict(os.environ,SANDPROOF_ACCEPTANCE_ROLE='native'):
                    leaf.unlink();registered.prepare_create_sources(root)
                    self.assertEqual(leaf.stat().st_mode&0o777,0o444)
                    leaf.chmod(0o644);leaf.write_bytes(b'existing different tool/source')
                    with self.assertRaisesRegex(ValueError,'never overwrite'):
                        registered.prepare_create_sources(root)
                    leaf.write_bytes(original)
                # Existing formal tool-root directory aliases remain legal;
                # only actual leaf bytes/type and declared archive are checked.
                seed=Path(temporary)/'fixed-seed';(root/'.tools/aeneas').rename(seed)
                (root/'.tools/aeneas').symlink_to(seed,target_is_directory=True)
                registered.check_create_sources(root)
                with archive.open('ab') as stream:stream.write(b'changed')
                with self.assertRaisesRegex(ValueError,'archive changed'):
                    registered.check_create_sources(root)

    def test_inherited_lock_survives_exec_and_child_does_not_unlock_parent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            lock = root/'coordination.lock'
            lock.write_text('')
            driver = root/'child.py'
            driver.write_text('import fcntl, os, sys\n'
                'fd=int(sys.argv[1])\nfcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)\n'
                'if len(sys.argv)>3:\n'
                ' assert sorted(os.sched_getaffinity(0))==[int(n) for n in sys.argv[3].split(",")]\n'
                'with open(sys.argv[2],"r+") as other:\n'
                ' try: fcntl.flock(other,fcntl.LOCK_EX|fcntl.LOCK_NB)\n'
                ' except BlockingIOError: pass\n'
                ' else: raise RuntimeError("parent lock was lost")\n')
            with lock.open('r+') as parent, patch.dict(os.environ,
                    SANDPROOF_LOCK_FILE=str(lock), SANDPROOF_LOCK_FD=str(parent.fileno())):
                fcntl.flock(parent, fcntl.LOCK_EX|fcntl.LOCK_NB)
                fd, identity = registered.inherited_lock()
                self.assertEqual(identity['inode'], lock.stat().st_ino)
                args = [fd, lock]
                options = {}
                if hasattr(os,'sched_getaffinity') and len(os.sched_getaffinity(0)) >= 4:
                    cpus = tuple(sorted(os.sched_getaffinity(0))[:4])
                    args += [','.join(map(str,cpus))]
                    options = dict(cpu_affinity=cpus,cpu_admission=root/'cpu-admission.json')
                registered._invoke(root, driver, args, pass_fds=(fd,), **options)
                if options:
                    self.assertEqual(registered._check_cpu_admission(root/'cpu-admission.json',cpus)['actual'],list(cpus))
                with lock.open('r+') as other, self.assertRaises(BlockingIOError):
                    fcntl.flock(other, fcntl.LOCK_EX|fcntl.LOCK_NB)
                wrong = root/'wrong-lock'
                wrong.write_text('')
                with patch.dict(os.environ, SANDPROOF_LOCK_FILE=str(wrong)), self.assertRaises(ValueError):
                    registered.inherited_lock()
                with patch.dict(os.environ, SANDPROOF_LOCK_FD='not-an-fd'), self.assertRaises(ValueError):
                    registered.inherited_lock()


class Registration(unittest.TestCase):
    def test_native_keys_are_the_original_formal_prerequisites(self):
        self.assertEqual(len(registered.NATIVE_CASES), 53)
        self.assertEqual(len(set(registered.NATIVE_CASES)), 53)
        self.assertEqual(registered.native_labels(), registered.NATIVE_CASES)
        for path in HERE.glob('inbox-*/formal.py'):
            calls = [node for node in ast.walk(ast.parse(path.read_text()))
                     if isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                     and node.func.id == 'native_case']
            labels = {node.args[0].value for node in calls if node.args
                      and isinstance(node.args[0], ast.Constant)}
            self.assertLessEqual(labels-{'functional'}, set(registered.NATIVE_CASES))
            for call in calls:
                self.assertEqual({kw.arg: ast.literal_eval(kw.value) for kw in call.keywords},
                                 {'scope': 'stage1-protocol'})
        self.assertNotIn('stage1-inbox-record-total', registered.NATIVE_CASES)

    def test_old_v0_and_retained_component_are_not_replaced(self):
        names = ast.parse((registered.ROOT/'tools/proof_batch.py').read_text())
        modules = next(ast.literal_eval(node.value) for node in names.body
                       if isinstance(node, ast.Assign) and any(
                           isinstance(t, ast.Name) and t.id == 'MODULES' for t in node.targets))
        self.assertEqual(len(modules), 33)
        self.assertIn('stage1-protocol', modules)
        self.assertNotIn('component.py', {Path(j['driver']).name for j in registered.formal_plan(Path('/unused'))})

    def test_full_independent_positive_and_negative_schedule(self):
        jobs = registered.formal_plan(Path('/unused'))
        names = {job['name'] for job in jobs}
        self.assertEqual(len(jobs), 138)
        self.assertEqual(len(jobs), len(names))
        self.assertLessEqual({'replay-replay', 'replay-validation', 'replay-envelope',
            'manifest-validation', 'access-identity', 'create-observation', 'create-conversion',
            'create-guard', 'envelope-writer-correct', 'envelope-writer-swallow-error',
            'serde-binding', 'serde-binding-negative', 'parser-binding', 'parser-binding-negative',
            'decode-assembly', 'decode-assembly-negative'}, names)
        for job in jobs:
            self.assertTrue((registered.ROOT/job['driver']).is_file())
            self.assertNotIn('--prepare-only', job['args'])
            self.assertNotIn('--development-native', job['args'])
        manifest = next(j for j in jobs if j['name'] == 'manifest-validation')
        self.assertEqual(len(manifest['cases']), 6)
        self.assertIn('skip-capture-binding', manifest['cases'])

    def test_total_only_subsumes_same_variant_append_formal(self):
        jobs = registered.formal_plan(Path('/unused'))
        inbox = [job for job in jobs if Path(job['driver']).parent.name.startswith('inbox-')]
        self.assertEqual(len(inbox), 76)
        self.assertFalse(any(j['name'].startswith('inbox-append-') for j in inbox))
        for variant in registered.INBOX_CASES['inbox-append']:
            job, = [j for j in inbox if j['name'] == 'inbox-record-total-'+variant]
            self.assertEqual(set(job['native']), {'stage1-inbox-record', 'stage1-inbox-append'})
        for job in inbox:
            self.assertIn('--require-native', job['args'])
        self.assertIn('stage1-inbox-append', registered.NATIVE_CASES)
        retained = next(j for j in inbox if j['name'] == 'inbox-retained-correct')
        self.assertNotIn('--fresh-native', retained['args'])

    def test_expanded_schedule_preserves_all_original_63_jobs(self):
        legacy_variants = {
            'inbox-lookup': ('correct', 'key-only'),
            'inbox-admission': ('correct', 'skip-node', 'skip-envelope'),
            'inbox-record': ('correct', 'skip-retention-window', 'wrong-recorded-version'),
            'inbox-record-total': ('correct', 'skip-sdk-gate', 'skip-prior-capture'),
            'inbox-state': ('correct', 'always-current', 'drop-operation'),
            'inbox-position': ('correct', 'always-current', 'drop-operation'),
            'inbox-lineage': ('correct', 'advance-unattached-resume', 'repeat-capture', 'skip-final-incarnation'),
            'inbox-retained': ('correct', 'reverse-compaction-bound', 'reverse-confirmation-bound', 'zero-version-adjacency'),
            'inbox-lifetimes': ('correct', 'skip-predecessor', 'keep-gap-predecessor', 'skip-final-deadline'),
            'inbox-renewal-session': ('correct', 'key-only-checkpoint', 'ignore-retained-frontier', 'ignore-unapplied-sdk'),
            'inbox-unload-resolution': ('correct', 'ignore-snapshot-phase', 'ignore-capture-operation', 'ignore-result-maximum'),
        }
        jobs = registered.formal_plan(Path('/unused'))
        legacy_names = {name+'-'+variant for name, variants in legacy_variants.items()
                        for variant in variants}
        legacy_names.update(('replay-replay', 'replay-validation', 'replay-envelope',
            'manifest-validation', 'result-confirmation', 'observation-bindings',
            'resume-observation', 'unload-observation', 'access-identity',
            'renewal-confirmation', 'create-observation', 'create-conversion', 'create-guard',
            'json-wire', 'schema-leaves', 'envelope-schema', 'envelope-bytes', 'envelope-budget',
            'envelope-writer-correct', 'envelope-writer-swallow-error',
            'serde-binding', 'serde-binding-negative', 'parser-binding', 'parser-binding-negative',
            'decode-assembly', 'decode-assembly-negative'))
        self.assertEqual(len(legacy_names), 63)
        self.assertLessEqual(legacy_names, {job['name'] for job in jobs})
        self.assertLessEqual({'stage1-'+name for name in legacy_variants
                             if name != 'inbox-record-total'} | {'stage1-inbox-append'},
                            set(registered.NATIVE_CASES))

    def test_lost_native_or_variant_registration_is_refused(self):
        with patch.object(registered, 'validate_metadata_inputs'):
            registered.validate_registration()
        with patch.object(registered, 'NATIVE_CASES', tuple(
                name for name in registered.NATIVE_CASES if name != 'stage1-inbox-state')):
            with self.assertRaisesRegex(ValueError, 'duplicate native registration|missing native registration'):
                registered.validate_registration()
        for name, dropped in (('inbox-state', 'drop-operation'),
                              ('inbox-save', 'wrong-budget'),
                              ('retention-lane', 'oversize-encoding')):
            with self.subTest(package=name):
                variants = dict(registered.INBOX_CASES)
                variants[name] = tuple(value for value in variants[name] if value != dropped)
                with patch.object(registered, 'INBOX_CASES', variants):
                    with self.assertRaisesRegex(ValueError, 'missing/extra original variant'):
                        registered.validate_registration()

    def test_wrong_machine_role_fails_before_any_work(self):
        with patch.object(registered, '_shared') as shared:
            with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='formal'):
                with self.assertRaisesRegex(ValueError, 'native host'):
                    registered.run_native(registered.ROOT, Path('/unused'))
            with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='native'):
                with self.assertRaisesRegex(ValueError, 'shared native'):
                    registered.run_formal(Path('/unused'), Path('/unused/retained/result.json'))
            shared.assert_not_called()

    def test_formal_execution_environment_is_strict_and_explicit_arguments_win(self):
        with patch.dict(os.environ, SANDPROOF_STAGE1_JOBS='2',
                        SANDPROOF_STAGE1_RESUME_FROM='/original/registered-result.json'):
            options = registered._execution_options()
            self.assertEqual(options['workers'],2)
            self.assertEqual(options['resume_from'],'/original/registered-result.json')
            self.assertEqual(options['sources']['workers'],'SANDPROOF_STAGE1_JOBS')
            self.assertEqual(registered._execution_options(4,Path('/explicit.json'))['workers'],4)
        with patch.dict(os.environ,SANDPROOF_STAGE1_JOBS='3'):
            with self.assertRaisesRegex(ValueError,'must be 1, 2 or 4'):
                registered._execution_options()
            self.assertEqual(registered._execution_options(1)['workers'],1)
        with patch.dict(os.environ,SANDPROOF_STAGE1_RESUME_FROM='relative.json'):
            with self.assertRaisesRegex(ValueError,'absolute original record'):
                registered._execution_options(1)

    def test_formal_refuses_missing_seal_or_native_case_before_driver(self):
        for failure in ('seal', 'case'):
            def validate_native(cache, *, scope):
                self.assertEqual(scope, 'stage1-protocol')
                if failure == 'seal':
                    raise ValueError('changed seal')
            def native_case(*args, scope):
                self.assertEqual(scope, 'stage1-protocol')
                raise ValueError('missing native case')
            shared = SimpleNamespace(validate_native=validate_native, native_case=native_case)
            # This fixture tests seal admission after structural registration.
            # Historical full-package input migration has its own real guard;
            # it must not intercept this synthetic missing-seal/case boundary.
            with patch.object(registered, '_shared', return_value=shared), \
                    patch.object(registered, 'validate_metadata_inputs'), \
                    patch.object(registered, '_invoke') as invoke:
                with patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='formal', SANDPROOF_NATIVE_EVIDENCE='/unused'):
                    with self.assertRaisesRegex(ValueError, 'seal|native case'):
                        registered.run_formal(Path('/unused'), Path('/unused/retained/result.json'))
                invoke.assert_not_called()

    def test_native_archive_and_result_remain_sealed_original_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            evidence = base/'commands/identity/evidence'
            evidence.mkdir(parents=True)
            raw = evidence/'source.tar'
            raw.write_bytes(b'original source bytes')
            receipt = dict(key={}, files={'evidence/source.tar': registered._sha(raw)})
            with patch.object(registered, '_shared', return_value=SimpleNamespace(digest=lambda _: 'identity')):
                with patch.dict(os.environ, SANDPROOF_NATIVE_EVIDENCE=str(base)):
                    self.assertEqual(registered._original_file(receipt, 'source.tar'), raw)
                    raw.write_bytes(b'changed source bytes')
                    with self.assertRaisesRegex(ValueError, 'changed sealed'):
                        registered._original_file(receipt, 'source.tar')

    def test_original_archive_driver_cannot_use_complete_instead_of_success(self):
        def native_case(label, root, output, action, *, scope):
            if label != 'stage1-protocol':
                action()
            return {}
        shared = SimpleNamespace(native_case=native_case)
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(registered, 'validate_registration'), \
                patch.object(registered, 'check_create_sources'), \
                patch.object(registered, '_shared', return_value=shared), \
                patch.object(registered, '_original_file', return_value=Path('/unused/source.tar')), \
                patch.object(registered, '_invoke'), \
                patch.object(registered, '_read', return_value=dict(complete=True, success=False)), \
                patch.dict(os.environ, SANDPROOF_ACCEPTANCE_ROLE='native', CARGO_TARGET_DIR='/unused'):
            with self.assertRaisesRegex(ValueError, 'native driver did not complete'):
                registered.run_native(registered.ROOT, Path(temporary))

    def test_bare_crate_zero_test_success_is_rejected(self):
        def native_run(label, command, root, log):
            log.write_text('test result: ok. 0 passed; 0 failed; 0 ignored;\n')
        shared = SimpleNamespace(native_run=native_run)
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(registered, '_shared', return_value=shared):
            with self.assertRaisesRegex(ValueError, 'crate test inventory'):
                registered._crate_native(registered.ROOT, Path(temporary)/'output', 'envelope-schema')

    def prepared_fixture(self, temporary):
        root = Path(temporary)/'root'
        root.mkdir()
        for name in ('verification/extract-foundations/src/lib.rs',
                     'verification/extract-foundations/Cargo.toml',
                     str(registered.HERE/'local_view.py'),
                     str(registered.HERE/'local-view.toml.in'),
                     str(registered.HERE/'schema-leaves/protocol_leaves.rs.in'),
                     str(registered.HERE/'schema-leaves/RoleLeaves.lean.in')):
            path = root/name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(registered.ROOT/name, path)
        (root/'unrelated.rs').write_text('pub const ORIGINAL: u8 = 1;\n')
        (root/'.cargo').mkdir()
        (root/'.cargo/config.toml').write_text('[build]\njobs = 4\n')
        inputs = {str(p.relative_to(root)): registered._sha(p) for p in root.rglob('*') if p.is_file()}
        output = Path(temporary)/'output'
        project = output/'preparation/source'
        shutil.copytree(root, project)
        helper = root/registered.HERE/'local_view.py'
        spec = importlib.util.spec_from_file_location('fixture_original_role_installer', helper)
        installer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installer)
        installer.install(project, 'schema-leaves')
        return root, output, project, dict(inputs=inputs, prepared=True, complete=False)

    def test_actual_role_preparation_binds_original_complete_private_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, output, project, prepared = self.prepared_fixture(temporary)
            changes = registered._role_preparation(root, output, prepared)
            self.assertEqual(set(changes), {'lakefile.toml', 'verification/extract-foundations/src/lib.rs',
                'verification/extract-foundations/src/protocol_leaves.rs',
                'SandboxModel/Lifecycle/Protocol/RoleLeaves.lean'})
            expected = registered._check_prepared_source(root, project, prepared['inputs'], changes)
            self.assertEqual(expected['unrelated.rs'], registered._sha(root/'unrelated.rs'))

    def test_any_unpermitted_prepared_source_change_or_extra_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, output, project, prepared = self.prepared_fixture(temporary)
            changes = registered._role_preparation(root, output, prepared)
            for name in sorted(set(prepared['inputs'])-set(changes)):
                with self.subTest(source=name):
                    path = project/name
                    before = path.read_bytes()
                    path.write_bytes(before+b'\nchanged after preparation\n')
                    with self.assertRaisesRegex(ValueError, 'private prepared source differs'):
                        registered._check_prepared_source(root, project, prepared['inputs'], changes)
                    path.write_bytes(before)
            extra = project/'unregistered.rs'
            extra.write_text('pub const EXTRA: u8 = 1;\n')
            with self.assertRaisesRegex(ValueError, 'private prepared source differs'):
                registered._check_prepared_source(root, project, prepared['inputs'], changes)
            extra.unlink()
            extra.symlink_to(root/'unrelated.rs')
            with self.assertRaisesRegex(ValueError, 'symlink'):
                registered._check_prepared_source(root, project, prepared['inputs'], changes)

    def test_incomplete_cases_or_missing_physical_audits_cannot_pass(self):
        job = dict(name='example', args=[], cases=['correct', 'wrong'], actual_files=True)
        actual = dict(complete=True, files={'Actual.lean': 'sha'}, audits=['actual.theorem'], service_log_sha256='log')
        valid = dict(complete=True, cases={'correct': actual, 'wrong': copy.deepcopy(actual)})
        registered._check_actual(valid, job)
        for field in ('files', 'audits', 'service_log_sha256'):
            broken = copy.deepcopy(valid)
            broken['cases']['wrong'].pop(field)
            with self.assertRaisesRegex(ValueError, 'physical-file'):
                registered._check_actual(broken, job)
        broken = copy.deepcopy(valid)
        broken['cases'].pop('wrong')
        with self.assertRaisesRegex(ValueError, 'missing/extra'):
            registered._check_actual(broken, job)
        broken = copy.deepcopy(valid)
        broken['cases']['wrong']['complete'] = False
        with self.assertRaisesRegex(ValueError, 'incomplete original'):
            registered._check_actual(broken, job)

    def test_wrong_variant_and_model_only_rejection_are_rejected(self):
        job = dict(name='inbox-state-drop-operation', args=['--variant', 'drop-operation'],
                   cases=['drop-operation'], actual_files=False)
        with self.assertRaisesRegex(ValueError, 'wrong actual variant'):
            registered._check_actual(dict(complete=True, variant='correct'), job)
        job = dict(name='serde-binding-negative', args=[], cases=[], actual_files=False)
        with self.assertRaisesRegex(ValueError, 'actual source-admission rejection'):
            registered._check_actual(dict(complete=True), job)

    def test_total_missing_parent_physical_seed_does_not_cover_append(self):
        result = dict(actual={'files': {}, 'audits': []})
        with self.assertRaisesRegex(ValueError, 'every original append seed'):
            registered._append_coverage(result, 'correct')

    def test_negative_template_audits_are_registered_without_claiming_success(self):
        templates = registered.audit_templates()
        names = {Path(path).name for path in templates}
        self.assertLessEqual({'NegativeFullCapture.lean.in', 'NegativeGuard.lean.in',
                             'InboxStateWrongCurrent.lean.in', 'EnvelopeWriterWrong.lean.in'}, names)
        self.assertTrue(all(audits for audits in templates.values()))

    def test_missing_or_empty_audit_source_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = str(registered.HERE/'Example.lean.in')
            path = root/name
            path.parent.mkdir(parents=True)
            with patch.object(registered.subprocess, 'check_output', return_value=(name+'\0').encode()), \
                    patch.object(registered, 'AUDIT_TEMPLATE_COUNT', 1), \
                    patch.object(registered, 'AUDIT_DECLARATION_COUNT', 1):
                with self.assertRaisesRegex(ValueError, 'missing original audit template'):
                    registered.audit_template_paths(root)
                path.write_text('#audit_controller_proof Example.actual\n')
                self.assertEqual(registered.audit_template_paths(root), (name,))
                path.write_text('example : True := True.intro\n')
                with self.assertRaisesRegex(ValueError, 'missing/extra finite audit'):
                    registered.audit_template_paths(root)

    def test_missing_changed_or_linked_parent_input_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            name = str(registered.HERE/'envelope-schema/inventory.json')
            metadata = root/name
            metadata.parent.mkdir(parents=True)
            parent_name = str(registered.HERE/'frozen.json')
            parent = root/parent_name
            parent.write_text('{"original":true}\n')
            original = parent.read_bytes()
            metadata.write_text(json.dumps({'inputs': {parent_name: registered._sha(parent)}}))
            with patch.object(registered, 'audit_template_paths', return_value=(str(
                    registered.HERE/'envelope-schema/EnvelopeSchema.lean.in'),)), \
                    patch.object(registered, 'formal_plan', return_value=[]), \
                    patch.object(registered.subprocess, 'check_output', return_value=(name+'\0').encode()):
                self.assertEqual(registered.validate_metadata_inputs(root), {name+':inputs': 1})
                parent.write_bytes(original+b'changed')
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)
                parent.unlink()
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)
                target = root/'same-bytes.json'
                target.write_bytes(original)
                parent.symlink_to(target)
                with self.assertRaisesRegex(ValueError, 'registered source digest differs'):
                    registered.validate_metadata_inputs(root)

    def retained_fixture(self, temporary, *, wrong=False):
        root = Path(temporary)/'root'
        inventory = root/registered.HERE/'retained-protocol.json'
        inventory.parent.mkdir(parents=True)
        names = ['Retained'+str(i) for i in range(62)]
        inventory.write_text(json.dumps(dict(schema=1, modules=names)))
        files = {}
        for name in names:
            relative = 'SandboxModel/Lifecycle/Protocol/'+name+'.lean'
            path = root/relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('-- original '+name+'\n')
            files[relative] = registered._sha(path)
        folder = Path(temporary)/'proof-output/fixture'
        service = folder/'physical-files/service.log'
        service.parent.mkdir(parents=True)
        service.write_text('controlled validator fixture; no Lean service was invoked\n')
        result_path = folder/'result.json'
        actual = dict(files=files, audits=['Fixture.original'], service_log_sha256=registered._sha(service))
        result = dict(complete=True, actual=actual, variant='wrong' if wrong else 'correct')
        result_path.write_text(json.dumps(result))
        job = dict(name='fixture', driver='unused.py', args=['--variant', 'wrong' if wrong else 'correct'],
                   cases=['wrong' if wrong else 'correct'], actual_files=True)
        record = dict(kind='registered-stage1-formal-drivers', complete=True,
            retained_inventory_sha256=registered._sha(inventory), results=[dict(name='fixture',
                result_path=str(result_path), result_sha256=registered._sha(result_path))])
        old_path = folder.parent/'retained/formal/result.json'
        views = {}
        for profile in ('carriers', 'request', 'manifest'):
            log = old_path.parent/'correct'/profile/'physical-files/service.log'
            log.parent.mkdir(parents=True)
            log.write_text('controlled original-component provenance fixture\n')
            views[profile] = dict(files={'FixtureUnrelated.lean': '0'*64}, audits=['Fixture.retained'],
                                  service_log_sha256=registered._sha(log))
        cases = {name: dict(complete=True) for name in ('correct', 'same-incarnation', 'key-only-replay',
                  'skip-capture-binding', 'session-only-envelope')}
        cases['correct']['views'] = views
        old_path.write_text(json.dumps(dict(complete=True, all_cases_complete=True, cases=cases)))
        record['retained_result'] = dict(path=str(old_path), sha256=registered._sha(old_path))
        return root, job, record, result_path, service

    def test_retained_requires_all_62_current_exact_sources_in_correct_view(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, result_path, _ = self.retained_fixture(temporary)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                coverage = registered.compute_retained_coverage(root, record)
                self.assertEqual(len(coverage), 62)
                record['retained_source_files'] = coverage
                self.assertEqual(registered.validate_retained_coverage(root, record), coverage)
                result = json.loads(result_path.read_text())
                missing = next(iter(result['actual']['files']))
                result['actual']['files'].pop(missing)
                result_path.write_text(json.dumps(result))
                record['results'][0]['result_sha256'] = registered._sha(result_path)
                with self.assertRaisesRegex(ValueError, 'lacks exact correct-view coverage'):
                    registered.compute_retained_coverage(root, record)

    def retained_audit_fixture(self, temporary):
        _, _, record, _, _ = self.retained_fixture(temporary)
        path = Path(record['retained_result']['path'])
        result = json.loads(path.read_text())
        expected = set()
        for case, row in result['cases'].items():
            field = 'views' if case == 'correct' else 'proofs'
            if field == 'proofs':
                row[field] = {'Negative': {}}
            for view in row[field]:
                audit = 'Fixture.' + case.replace('-', '_') + '.' + view
                expected.add(audit)
                log = path.parent/case/view/'physical-files/service.log'
                log.parent.mkdir(parents=True, exist_ok=True)
                log.write_text('Controlled fixture; no Lean service invoked.\n'
                    + 'Checked ' + audit + ': 3 standard logical dependencies\n')
                row[field][view] = dict(files={'Fixture.lean': '0'*64}, audits=[audit],
                                       service_log_sha256=registered._sha(log))
        path.write_text(json.dumps(result))
        record['retained_result']['sha256'] = registered._sha(path)
        return record, path, result, expected

    def test_retained_positive_and_negative_service_audits_are_both_collected(self):
        with tempfile.TemporaryDirectory() as temporary:
            record, _, _, expected = self.retained_audit_fixture(temporary)
            self.assertEqual(registered.retained_audits(record), expected)
            self.assertIn('Fixture.same_incarnation.Negative', expected)
            self.assertIn('Fixture.key_only_replay.Negative', expected)

    def test_retained_audit_record_does_not_replace_missing_or_changed_service(self):
        for change in ('missing', 'changed', 'linked', 'missing-message', 'missing-files', 'missing-views'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                record, path, result, _ = self.retained_audit_fixture(temporary)
                row = result['cases']['key-only-replay']
                log = path.parent/'key-only-replay/Negative/physical-files/service.log'
                if change == 'missing':
                    log.unlink()
                elif change == 'changed':
                    log.write_text('changed service')
                elif change == 'linked':
                    target = log.with_suffix('.original')
                    log.rename(target)
                    log.symlink_to(target)
                elif change == 'missing-message':
                    log.write_text('No target audit was actually reported.\n')
                    row['proofs']['Negative']['service_log_sha256'] = registered._sha(log)
                elif change == 'missing-files':
                    row['proofs']['Negative']['files'] = {}
                else:
                    row['proofs'] = {}
                path.write_text(json.dumps(result))
                record['retained_result']['sha256'] = registered._sha(path)
                with self.assertRaisesRegex(ValueError, 'retained'):
                    registered.retained_audits(record)

    def test_retained_wrong_view_cannot_back_current_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary, wrong=True)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                with self.assertRaisesRegex(ValueError, 'lacks exact correct-view coverage'):
                    registered.compute_retained_coverage(root, record)

    def test_extra_unregistered_protocol_source_is_rejected_even_on_reuse(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                record['retained_source_files'] = registered.compute_retained_coverage(root, record)
                (root/'SandboxModel/Lifecycle/Protocol/Unregistered.lean').write_text('-- extra source\n')
                with self.assertRaisesRegex(ValueError, 'unregistered or missing retained protocol'):
                    registered.validate_retained_coverage(root, record)

    def test_original_carriers_view_can_supply_its_own_exact_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, result_path, _ = self.retained_fixture(temporary)
            current = json.loads(result_path.read_text())
            name = next(iter(current['actual']['files']))
            digest = current['actual']['files'].pop(name)
            result_path.write_text(json.dumps(current))
            record['results'][0]['result_sha256'] = registered._sha(result_path)
            old_path = Path(record['retained_result']['path'])
            old = json.loads(old_path.read_text())
            old['cases']['correct']['views']['carriers']['files'][name] = digest
            old_path.write_text(json.dumps(old))
            record['retained_result']['sha256'] = registered._sha(old_path)
            with patch.object(registered, 'formal_plan', return_value=[job]):
                coverage = registered.compute_retained_coverage(root, record)
                self.assertEqual(coverage[name]['job'], 'retained-stage1-protocol')
                self.assertEqual(coverage[name]['view'], 'carriers')

    def test_incomplete_original_component_or_missing_correct_view_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root, job, record, _, _ = self.retained_fixture(temporary)
            path = Path(record['retained_result']['path'])
            original = json.loads(path.read_text())
            for field in ('case', 'view', 'complete'):
                changed = copy.deepcopy(original)
                if field == 'case':
                    changed['cases'].pop('same-incarnation')
                elif field == 'view':
                    changed['cases']['correct']['views'].pop('carriers')
                else:
                    changed['all_cases_complete'] = False
                path.write_text(json.dumps(changed))
                record['retained_result']['sha256'] = registered._sha(path)
                with patch.object(registered, 'formal_plan', return_value=[job]):
                    with self.assertRaisesRegex(ValueError, 'all original cases and correct views'):
                        registered.compute_retained_coverage(root, record)

    def test_retained_changed_result_service_source_or_coverage_is_rejected(self):
        for change in ('result', 'service', 'source', 'coverage'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                root, job, record, result_path, service = self.retained_fixture(temporary)
                with patch.object(registered, 'formal_plan', return_value=[job]):
                    record['retained_source_files'] = registered.compute_retained_coverage(root, record)
                    if change == 'result':
                        result_path.write_text(result_path.read_text()+'\n')
                    elif change == 'service':
                        service.write_text('changed original service log')
                    elif change == 'source':
                        (root/next(iter(record['retained_source_files']))).write_text('changed source')
                    else:
                        record['retained_source_files'].pop(next(iter(record['retained_source_files'])))
                    with self.assertRaises(ValueError):
                        registered.validate_retained_coverage(root, record)


class CheckpointExecution(unittest.TestCase):
    """Controlled child processes/service records; no proof tools invoked."""
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.shared = registered._shared()
        self.helper = registered._execution_helper(registered.ROOT)
        self.context = dict(schema=1, reuse_scope='same-source-tree', source_tree='fixture',
                            runtime={'fixed-tool':'original'}, native={'seal':'original'})
        self.called = []
        self.environments = {}
        self.mutex = threading.Lock()
        self.defect = None
        self.hook = None

    def plan(self, output, names=('fixture-first', 'fixture-second')):
        return [dict(name=name, driver='fixture.py', args=['--variant', 'correct'],
                     output=str(output/name), native=[], cases=['correct'], actual_files=True)
                for name in names]

    def invoke(self, root, driver, args, env=None, *, log=None):
        output = Path(args[args.index('--output')+1])
        name = output.name
        with self.mutex:
            self.called.append(name)
            self.environments[name] = dict(env)
        if self.hook is not None:
            self.hook(name)
        output.mkdir(mode=0o700)
        project = output/'source'
        (project/'artifacts').mkdir(parents=True)
        (project/'artifacts/original.llbc').write_text('{"controlled_fixture":true}')
        source = project/'Fixture.lean'
        audit = 'Fixture.'+name.replace('-', '_')
        source.write_text('-- Controlled bytes: no Lean service was invoked.\n'
                          '#audit_controller_proof '+audit+'\n')
        files = {'Fixture.lean':registered._sha(source)}
        physical = output/'physical-files'
        physical.mkdir()
        request = dict(tool='lean_diagnostic_messages', arguments={'file_path':'Fixture.lean'},
                       expected_sha256=files['Fixture.lean'])
        (physical/'requests.jsonl').write_text(json.dumps(request)+'\n{"tool":"exit"}\n')
        response = dict(request=request, revision={'file_sha256':files['Fixture.lean']}, stale=False,
            result=dict(success=True, partial=self.defect == 'partial', timed_out=False,
                        failed_dependencies=[], still_elaborating_lines=[], items=[dict(
                            severity='information', message='Checked '+audit+': 3 standard logical dependencies')]))
        (physical/'service.log').write_text(json.dumps(response)+'\n')
        kernel = output/'kernel-0.log'
        kernel.write_text('Controlled successful kernel fixture; no Lean invocation.\n')
        result = dict(complete=True, variant='correct', actual=dict(files=files, audits=[audit],
            service_log_sha256=registered._sha(physical/'service.log')), commands=[dict(
                argv=['lake','env','lean','Fixture.lean'], cwd=str(project), exit_code=0,
                log_sha256=registered._sha(kernel))])
        (output/'result.json').write_text(json.dumps(result))
        Path(log).write_text('Controlled child fixture.\nChecked '+audit+': 3 standard logical dependencies\n')
        return ['controlled-python',str(root/driver),*map(str,args)]

    def execute(self, label, *, jobs=1, resume=None, names=('fixture-first','fixture-second'), progress=None,
                runtime_seed=None, report_fixture=False):
        output = self.directory/label
        output.mkdir()
        plan = self.state_plan(output) if report_fixture else self.plan(output,names)
        with patch.object(registered,'formal_plan',return_value=plan), \
                patch.object(registered,'_invoke',side_effect=self.invoke_state if report_fixture else self.invoke):
            rows, failures = registered.execute_formal_jobs(registered.ROOT,output,plan,
                self.context,{},jobs=jobs,resume=resume,progress=progress,runtime_seed=runtime_seed)
        parent = output/'registered-result.json'
        self.shared.write_json(parent,dict(kind='registered-stage1-formal-drivers',
            execution_schema=1,complete=not failures,results=rows))
        return output, rows, failures, parent

    def state_plan(self, output):
        return [dict(name='inbox-state-'+variant, driver=str(registered.HERE/'inbox-state/formal.py'),
            args=['--require-native','--variant',variant], output=str(output/('inbox-state-'+variant)),
            native=['stage1-inbox-state'], cases=[variant], actual_files=True)
            for variant in ('correct','always-current')]

    def invoke_state(self, root, driver, args, env=None, *, log=None):
        command = self.invoke(root,driver,args,env,log=log)
        path = Path(args[args.index('--output')+1])/'result.json'
        value = registered._read(path)
        value.update(kind='inbox-state-formal-development', variant=args[args.index('--variant')+1],
                     inputs=copy.deepcopy(self.context['repository']['files']))
        self.shared.write_json(path,value)
        return command

    def report_context(self, *, report='original report', tree='original-tree'):
        files = {'docs/proof-coverage.md':registered._object_sha(report),
            'controller-service/src/controller/lifecycle_inbox/model.rs':'b'*64,
            'docs/proof-workflow.md':'c'*64}
        key = dict(label='functional', command=['component-case'], repository=copy.deepcopy(files))
        return dict(schema=2, reuse_scope='same-tree-or-reviewed-state-report', source_tree=tree,
            repository=dict(files=files,modes={name:'100644' for name in files}),
            rust={'model':'original'}, rules={'driver':'original'}, contracts={'contract':'original'},
            source_archives={'archive':'original'}, runtime={'tool':'original'},
            driver_python={'executable':'original'},
            native=dict(seal_sha256=registered._object_sha(key),receipts={
                'functional':dict(key=key,sha256=registered._object_sha(key)),
                'stage1-inbox-state':dict(key={'state':'original'},sha256='d'*64)}))

    def test_current_context_captures_regular_source_modes_and_requires_fresh_functional_inputs(self):
        root = self.directory/'source'
        report = root/'docs/proof-coverage.md'
        report.parent.mkdir(parents=True)
        report.write_text('controlled report fixture\n')
        report.chmod(0o600)
        files = {'docs/proof-coverage.md':registered._sha(report)}
        cache = self.directory/'native'
        evidence = cache/'commands/functional/evidence'
        evidence.mkdir(parents=True)
        (cache/'complete.json').write_text('controlled seal')
        (evidence.parent/'receipt.json').write_text('controlled complete receipt')
        receipts = {'functional':{'key':{'repository':dict(files)}}}
        shared = SimpleNamespace(repository_inputs=lambda root:dict(files),
            rust_inputs=lambda root:{},rules=lambda root:{},contract_inputs=lambda root:{},
            source_archive_inputs=lambda:{})
        helper = SimpleNamespace(runtime=lambda:{'tools':'controlled fixture'})
        def git(args, **kwargs):
            if args[1] == 'rev-parse': return 'controlled-tree\n'
            if args[1] == 'status': return b''
            self.assertEqual(args,['git','ls-files','--stage','-z'])
            return b'100644 0000000000000000000000000000000000000000 0\tdocs/proof-coverage.md\0'
        with patch.object(registered.subprocess,'check_output',side_effect=git), \
                patch.object(registered,'_evidence',return_value=evidence), \
                patch.object(registered,'inherited_lock',return_value=(9,{'path':'/fixture-lock','device':1,'inode':2})), \
                patch.object(registered,'create_profile',return_value={'profile':'create-delivery'}), \
                patch.dict(os.environ,SANDPROOF_NATIVE_EVIDENCE=str(cache)):
            context = registered._execution_context(root,shared,helper,receipts)
            self.assertEqual(context['schema'],2)
            self.assertEqual(context['repository'],dict(files=files,modes={'docs/proof-coverage.md':'100644'}))
            receipts['functional']['key']['repository'] = {'docs/proof-coverage.md':'old-digest'}
            with self.assertRaisesRegex(ValueError,'current complete functional admission'):
                registered._execution_context(root,shared,helper,receipts)
            receipts['functional']['key']['repository'] = dict(files)
            report.chmod(0o700)
            with self.assertRaisesRegex(ValueError,'regular committed files'):
                registered._execution_context(root,shared,helper,receipts)

    def test_reviewed_report_only_continuation_reuses_both_without_launching_a_driver(self):
        self.context = self.report_context()
        _, before, failures, parent = self.execute('state-original',report_fixture=True)
        self.assertFalse(failures)
        saved = registered._read(parent)
        saved['complete'] = False
        self.shared.write_json(parent,saved)
        original_bytes = parent.read_bytes()
        self.context = self.report_context(report='updated coverage report',tree='updated-tree')
        self.called.clear()
        _, rows, failures, _ = self.execute('report-only',resume=parent,report_fixture=True)
        self.assertFalse(failures)
        self.assertEqual(self.called,[])
        self.assertEqual([row['use'] for row in rows],['REUSE','REUSE'])
        self.assertEqual([row['result_path'] for row in rows],[row['result_path'] for row in before])
        self.assertEqual(parent.read_bytes(),original_bytes)
        for row in rows:
            self.assertEqual(row['reuse_basis']['kind'],'reviewed-state-coverage-report-only')
            self.assertFalse(row['reuse_basis']['report_checks_reused'])
            self.assertEqual(row['reuse_basis']['current_tree'],'updated-tree')

    def test_report_only_reuse_refuses_every_other_source_or_execution_change(self):
        original = self.report_context()
        accepted = self.report_context(report='new report',tree='new-tree')
        job = self.state_plan(self.directory)[0]
        result = dict(kind='inbox-state-formal-development',variant='correct',
                      inputs=original['repository']['files'])
        for change in ('rust','rules','contracts','source_archives','runtime','driver_python',
                'native-case','semantic-file','other-doc','new-file','deleted-file','mode',
                'report-mode','stale-functional','missing-snapshot','legacy-schema','wrong-inputs'):
            with self.subTest(change=change):
                current = copy.deepcopy(accepted)
                checked = copy.deepcopy(result)
                if change in ('rust','rules','contracts','source_archives','runtime','driver_python'):
                    current[change] = {'changed':'input'}
                elif change == 'native-case':
                    current['native']['receipts']['stage1-inbox-state']['sha256'] = 'e'*64
                elif change in ('semantic-file','other-doc'):
                    name = ('controller-service/src/controller/lifecycle_inbox/model.rs'
                            if change == 'semantic-file' else 'docs/proof-workflow.md')
                    current['repository']['files'][name] = 'e'*64
                    current['native']['receipts']['functional']['key']['repository'][name] = 'e'*64
                elif change == 'new-file':
                    current['repository']['files']['new.rs'] = 'e'*64
                    current['repository']['modes']['new.rs'] = '100644'
                    current['native']['receipts']['functional']['key']['repository']['new.rs'] = 'e'*64
                elif change == 'deleted-file':
                    current['repository']['files'].pop('docs/proof-workflow.md')
                    current['repository']['modes'].pop('docs/proof-workflow.md')
                    current['native']['receipts']['functional']['key']['repository'].pop('docs/proof-workflow.md')
                elif change in ('mode','report-mode'):
                    name = 'docs/proof-coverage.md' if change == 'report-mode' else 'docs/proof-workflow.md'
                    current['repository']['modes'][name] = '100755'
                elif change == 'stale-functional':
                    current['native']['receipts']['functional'] = copy.deepcopy(original['native']['receipts']['functional'])
                elif change == 'missing-snapshot':
                    current['repository'] = []
                elif change == 'legacy-schema':
                    current['schema'] = 1
                else:
                    checked['inputs'] = {}
                with self.assertRaises((ValueError,KeyError)):
                    registered._context_reuse_basis(original,current,job,checked)

    def test_report_scope_does_not_extend_to_another_producer_or_variant(self):
        original = self.report_context()
        current = self.report_context(report='new report',tree='new-tree')
        baseline = self.state_plan(self.directory)[0]
        result = dict(kind='inbox-state-formal-development',variant='correct',
                      inputs=original['repository']['files'])
        for field, value in [('name','inbox-state-drop-operation'),('driver','another.py'),
                ('args',['--require-native','--variant','drop-operation']),
                ('native',[]),('cases',['wrong']),('actual_files',False)]:
            with self.subTest(field=field):
                job = dict(baseline,**{field:value})
                with self.assertRaisesRegex(ValueError,'no reviewed report-independent closure'):
                    registered._context_reuse_basis(original,current,job,result)

    def test_report_change_does_not_relabel_a_damaged_proof_as_reusable(self):
        self.context = self.report_context()
        output, _, _, parent = self.execute('state-before-damage',report_fixture=True)
        original_bytes = parent.read_bytes()
        (output/'inbox-state-correct/source/artifacts/original.llbc').unlink()
        self.context = self.report_context(report='new report',tree='new-tree')
        self.called.clear()
        def require_current_native(name):
            raise ValueError('native and formal source inputs differ; current case evidence required')
        self.hook = require_current_native
        after, rows, failures, _ = self.execute('state-damaged',resume=parent,report_fixture=True)
        self.assertEqual(self.called,['inbox-state-correct'])
        self.assertEqual([(row['name'],row['use']) for row in rows],[('inbox-state-always-current','REUSE')])
        self.assertIn('current case evidence required',failures['inbox-state-correct']['error'])
        self.assertFalse((after/'execution/inbox-state-correct/checkpoint.json').exists())
        self.assertEqual(parent.read_bytes(),original_bytes)

    def test_same_candidate_checkpoints_resume_even_when_parent_aggregate_failed(self):
        output, rows, failures, parent = self.execute('first')
        self.assertFalse(failures)
        self.assertEqual([row['use'] for row in rows],['RUN','RUN'])
        old = registered._read(parent)
        old['complete'] = False
        self.shared.write_json(parent,old)
        original = parent.read_bytes()
        self.called.clear()
        _, reused, failures, _ = self.execute('resume',resume=parent)
        self.assertEqual(self.called,[])
        self.assertFalse(failures)
        self.assertEqual([row['use'] for row in reused],['REUSE','REUSE'])
        self.assertEqual(parent.read_bytes(),original)
        self.assertEqual([row['result_path'] for row in rows],[row['result_path'] for row in reused])

    def test_damaged_evidence_only_reruns_the_affected_job(self):
        for defect in ('raw','service','kernel','checkpoint','result','context'):
            with self.subTest(defect=defect):
                output, rows, _, parent = self.execute('first-'+defect)
                first = rows[0]
                if defect == 'raw': (output/'fixture-first/source/artifacts/original.llbc').unlink()
                elif defect == 'service': (output/'fixture-first/physical-files/service.log').write_text('changed')
                elif defect == 'kernel': (output/'fixture-first/kernel-0.log').unlink()
                elif defect == 'checkpoint': Path(first['checkpoint_path']).unlink()
                elif defect == 'result': Path(first['result_path']).write_text('{"complete":false}')
                else:
                    receipt = registered._read(Path(first['checkpoint_path']))
                    # The context is a shared immutable input: its damage invalidates both jobs.
                    Path(receipt['context_path']).write_text('{}')
                self.called.clear()
                _, after, failures, _ = self.execute('again-'+defect,resume=parent)
                self.assertFalse(failures)
                expected = ['fixture-first','fixture-second'] if defect == 'context' else ['fixture-first']
                self.assertEqual(sorted(self.called),expected)
                self.assertEqual(after[0]['use'],'RUN')
                self.assertEqual(after[1]['use'],'RUN' if defect == 'context' else 'REUSE')

    def test_changed_runtime_native_or_input_context_never_hits_old_checkpoint(self):
        _, _, _, parent = self.execute('original')
        for field in ('runtime','native','source_tree'):
            with self.subTest(field=field):
                before = copy.deepcopy(self.context)
                self.context[field] = 'changed'
                self.called.clear()
                _, rows, failures, _ = self.execute('changed-'+field,resume=parent)
                self.assertFalse(failures)
                self.assertEqual([row['use'] for row in rows],['RUN','RUN'])
                self.context = before

    def test_nonobject_saved_records_rerun_instead_of_aborting_resume(self):
        for defect in ('checkpoint', 'result', 'context'):
            with self.subTest(defect=defect):
                _, rows, _, parent = self.execute('nonobject-'+defect)
                first = rows[0]
                checkpoint = Path(first['checkpoint_path'])
                receipt = registered._read(checkpoint)
                path = (checkpoint if defect == 'checkpoint' else
                        Path(first['result_path']) if defect == 'result' else
                        Path(receipt['context_path']))
                path.write_text('[]')
                # Keep the outer digest coherent to exercise validation of the
                # saved record itself, rather than the earlier digest gate.
                if defect == 'checkpoint':
                    first['checkpoint_sha256'] = registered._sha(path)
                else:
                    field = 'result_sha256' if defect == 'result' else 'context_sha256'
                    receipt[field] = registered._sha(path)
                    if defect == 'result':
                        first[field] = receipt[field]
                    self.shared.write_json(checkpoint, receipt)
                    first['checkpoint_sha256'] = registered._sha(checkpoint)
                saved = registered._read(parent)
                saved['results'][0] = first
                self.shared.write_json(parent, saved)
                self.called.clear()
                _, after, failures, _ = self.execute('repaired-'+defect, resume=parent)
                self.assertFalse(failures)
                expected = ['fixture-first', 'fixture-second'] if defect == 'context' else ['fixture-first']
                self.assertEqual(sorted(self.called), expected)
                self.assertEqual(after[0]['use'], 'RUN')
                self.assertEqual(after[1]['use'], 'RUN' if defect == 'context' else 'REUSE')

    def test_partial_service_cannot_publish_a_successful_checkpoint(self):
        self.defect = 'partial'
        output, rows, failures, _ = self.execute('partial')
        self.assertEqual(rows,[])
        self.assertIn('fixture-first',failures)
        self.assertFalse((output/'execution/fixture-first/checkpoint.json').exists())
        self.assertTrue((output/'execution/fixture-first/failure.json').is_file())
        self.assertEqual(self.called,['fixture-first'])

    def test_two_workers_have_private_state_and_aggregate_in_original_order(self):
        barrier = threading.Barrier(2)
        second_admitted = threading.Event()
        def hook(name):
            barrier.wait(timeout=5)
            if name == 'fixture-first':
                self.assertTrue(second_admitted.wait(timeout=5))
        def progress(rows, failures, misses):
            if any(row['name']=='fixture-second' for row in rows): second_admitted.set()
        self.hook = hook
        before = dict(os.environ)
        _, rows, failures, _ = self.execute('parallel',jobs=2,progress=progress)
        self.assertFalse(failures)
        self.assertEqual([row['name'] for row in rows],['fixture-first','fixture-second'])
        self.assertEqual(os.environ,before)
        for variable in ('SANDPROOF_FORMAL_CACHE','CARGO_TARGET_DIR'):
            self.assertEqual(len({env[variable] for env in self.environments.values()}),2)

    def test_failure_stops_dispatch_but_an_inflight_success_remains_resumable(self):
        second_started, release_second = threading.Event(), threading.Event()
        def hook(name):
            if name == 'fixture-first':
                self.assertTrue(second_started.wait(timeout=5))
                raise ValueError('controlled first child failure')
            second_started.set()
            self.assertTrue(release_second.wait(timeout=5))
        def progress(rows, failures, misses):
            if failures: release_second.set()
        self.hook = hook
        names = ('fixture-first','fixture-second','fixture-third')
        output, rows, failures, parent = self.execute('failed',jobs=2,names=names,progress=progress)
        self.assertEqual(set(self.called),{'fixture-first','fixture-second'})
        self.assertEqual([row['name'] for row in rows],['fixture-second'])
        self.assertEqual(set(failures),{'fixture-first'})
        failed_bytes = (output/'execution/fixture-first/failure.json').read_bytes()
        self.hook = None
        self.called.clear()
        _, rows, failures, _ = self.execute('recovered',jobs=2,names=names,resume=parent)
        self.assertFalse(failures)
        self.assertEqual(set(self.called),{'fixture-first','fixture-third'})
        self.assertEqual([row['use'] for row in rows],['RUN','REUSE','RUN'])
        self.assertEqual((output/'execution/fixture-first/failure.json').read_bytes(),failed_bytes)

    def test_four_worker_limit_and_private_outputs(self):
        barrier = threading.Barrier(4)
        self.hook = lambda name: barrier.wait(timeout=5)
        names = ('fixture-first','fixture-second','fixture-third','fixture-fourth')
        _, rows, failures, _ = self.execute('four',jobs=4,names=names)
        self.assertFalse(failures)
        self.assertEqual(len(rows),4)
        self.assertEqual(len(set(row['result_path'] for row in rows)),4)

    def test_verified_parent_runtime_metadata_seeds_private_caches_without_rewriting_it(self):
        seed = self.directory/'runtime-file-hashes.json'
        seed.write_text('{"schema":1,"files":{"controlled":{"sha256":"original"}}}')
        original = seed.read_bytes()
        _, rows, failures, _ = self.execute('seeded',jobs=2,runtime_seed=seed)
        self.assertFalse(failures)
        copies = [Path(env['SANDPROOF_FORMAL_CACHE'])/'runtime-file-hashes.json'
                  for env in self.environments.values()]
        self.assertEqual(len(set(copies)),2)
        self.assertTrue(all(path.read_bytes()==original for path in copies))
        copies[0].write_text('{"schema":1,"files":{}}')
        self.assertEqual(copies[1].read_bytes(),original)
        self.assertEqual(seed.read_bytes(),original)
        self.assertTrue(all(row['runtime_seed_sha256']==registered._sha(seed) for row in rows))

    def test_original_public_cli_forwards_environment_configuration_to_the_executor(self):
        self.public_cli_fixture()

    def test_public_final_write_failure_preserves_the_original_progress_index(self):
        self.public_cli_fixture(interrupt_final=True)

    def test_public_source_guard_rejects_mode_change_even_when_bytes_and_seal_are_unchanged(self):
        self.public_cli_fixture(change_modes=True)

    def public_cli_fixture(self, *, interrupt_final=False, change_modes=False):
        cache = self.directory/'native'
        evidence = cache/'commands/fixture/evidence'
        evidence.mkdir(parents=True)
        (cache/'complete.json').write_text('{"controlled_fixture":true}')
        (evidence.parent/'receipt.json').write_text('{"controlled_fixture":true}')
        retained = self.directory/'retained.json'
        retained.write_text('{"controlled_fixture":true}')
        output = self.directory/'public'
        shared = SimpleNamespace(validate_native=lambda *a,**k: None,
            native_case=lambda *a,**k: {'key':{'label':a[0]}},
            repository_inputs=lambda root: {'fixture':'original'}, digest=registered._object_sha,
            write_json=self.shared.write_json)
        original_replace = os.replace
        progress_bytes = []
        writes = []
        context = dict(self.context,repository=dict(modes={'Fixture.rs':'100644'}))
        def replace(source, destination):
            if interrupt_final and Path(destination) == output/'registered-result.json' and progress_bytes:
                raise OSError('controlled final replacement failure')
            return original_replace(source, destination)
        def write_json(path, value):
            writes.append(Path(path))
            return self.shared.write_json(path, value)
        shared.write_json = write_json
        def execute_boundary(*args, **kwargs):
            if change_modes:
                kwargs['source_guard']()
                raise AssertionError('changed modes passed the public source guard')
            if interrupt_final:
                kwargs['progress']([dict(name='fixture', checkpoint_path='retained-original')], {}, {})
                progress_bytes.append((output/'registered-result.json').read_bytes())
            raise ValueError('controlled executor boundary')
        argv = ['registered.py','formal','--output',str(output),'--retained-result',str(retained)]
        before_umask = os.umask(0o077)
        try:
            with patch.object(registered,'validate_registration'), \
                    patch.object(registered,'check_create_sources'), \
                    patch.object(registered,'_shared',return_value=shared), \
                    patch.object(registered,'_evidence',return_value=evidence), \
                    patch.object(registered,'_original_file',return_value=retained), patch.object(registered,'_retained_result'), \
                    patch.object(registered,'_check_create_native_result'), \
                    patch.object(registered,'_execution_helper',return_value=self.helper), \
                    patch.object(registered,'_execution_context',return_value=context), \
                    patch.object(registered,'_source_modes',return_value={'Fixture.rs':'100755' if change_modes else '100644'}), \
                    patch.object(registered,'formal_plan',return_value=[]), \
                    patch.object(registered,'execute_formal_jobs',side_effect=execute_boundary) as execute, \
                    patch.object(os,'replace',side_effect=replace), \
                    patch.object(registered.sys,'argv',argv), \
                    patch.dict(os.environ,SANDPROOF_ACCEPTANCE_ROLE='formal',SANDPROOF_NATIVE_EVIDENCE=str(cache),
                        SANDPROOF_FORMAL_CACHE=str(self.directory/'runtime-cache'), SANDPROOF_STAGE1_JOBS='2',
                        SANDPROOF_STAGE1_RESUME_FROM='/original/registered-result.json'):
                error = OSError if interrupt_final else ValueError
                message = 'controlled final replacement failure' if interrupt_final else 'controlled executor boundary'
                if change_modes:
                    message = 'shared native seal or registered source changed'
                with self.assertRaisesRegex(error,message):
                    registered.main()
                self.assertEqual(execute.call_args.kwargs['jobs'],2)
                self.assertEqual(execute.call_args.kwargs['resume'],'/original/registered-result.json')
        finally:
            os.umask(before_umask)
        saved = registered._read(output/'registered-result.json')
        self.assertFalse(saved['complete'])
        self.assertEqual(saved['execution']['workers'],2)
        self.assertEqual(saved['execution']['sources']['workers'],'SANDPROOF_STAGE1_JOBS')
        self.assertEqual(writes, [output/'registered-result.json'] * (2 if interrupt_final else 1))
        if interrupt_final:
            self.assertEqual((output/'registered-result.json').read_bytes(),progress_bytes[0])
            self.assertEqual(saved['results'],[dict(name='fixture',checkpoint_path='retained-original')])
            self.assertEqual(list(output.glob('.receipt-*')),[])

    def test_old_raw_parent_cannot_silently_become_checkpoint_evidence(self):
        path = self.directory/'legacy.json'
        path.write_text('{"kind":"registered-stage1-formal-drivers","complete":true,"results":[]}')
        with self.assertRaisesRegex(ValueError,'checkpoint-producing'):
            self.execute('legacy',resume=path)


if __name__ == '__main__':
    unittest.main()
