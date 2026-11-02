"""Recapture the real standard digest and check every retained physical file.

The direct original method remains a source-binding result, with its backend
failure retained separately. No public acceptance is granted by this entry.
"""
from pathlib import Path
import argparse
import fcntl
import importlib.util
import json
import os
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-save-digest')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('current_standard_save_digest_support', ROOT/HERE/'support.py')
capture = module('current_standard_save_digest_capture', ROOT/HERE/'capture.py')
blob_capture = module('current_standard_save_digest_blob_capture', ROOT/HERE.parent/'standard-blob/capture.py')
sha = support.sha


def run(output, lock_path, prepare_only=False, lock_fd=None):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='current-standard-save-digest-and-original-save-development', complete=False,
        commands=[], views={}, independent_sources=[], source_adopted=True,
        library_provider_adopted=True, original_direct_method_machine_refined=False,
        generated_bodies_modified=False, new_protocol_or_save_algorithm=False,
        ordinary_checks_run=False, native_checks_run=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    for name in ('LEAN_NUM_THREADS', 'LEAN_STACK_SIZE_KB', 'CARGO_NET_OFFLINE'):
        os.environ[name] = env[name]

    def save():
        (output/'progress.json').write_text(json.dumps(record, indent=2)+'\n')

    def command(project, folder, label, argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            folder=str(folder), exit_code=done.returncode, seconds=time.monotonic()-begin,
            log_sha256=sha(log)))
        save()
        if done.returncode:
            raise ValueError('compiler/kernel/service failure, not a contract rejection: '+label)

    def project_at(folder):
        project = folder/'source'
        project.mkdir(mode=0o700, parents=True)
        for name, digest in inputs.items():
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, target)
            if sha(target) != digest or support.mode(target) != modes[name]:
                raise ValueError('source changed while preparing an independent view')
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        return project

    def keep_source(project, changed, purpose):
        expected = {**inputs, **changed}
        row = dict(project=str(project), purpose=purpose, inputs=expected,
            modes={name:support.mode(project/name) for name in expected})
        record['independent_sources'].append(row)
        return row

    def compare_named(row, variant):
        index = json.loads((ROOT/HERE/'generated-index.json').read_text())
        expected = index['captures'][variant]
        if row['ir_value_sha256'] != expected['ir_value_sha256']:
            raise ValueError('complete original SHA IR value differs from the reviewed capture')
        generated = Path(row['generated_directory'])
        for name, entry in expected['published'].items():
            fresh, retained = generated/name, ROOT/entry['path']
            if (fresh.is_symlink() or retained.is_symlink() or sha(fresh) != entry['sha256']
                    or sha(retained) != entry['sha256'] or fresh.read_bytes() != retained.read_bytes()):
                raise ValueError('actual original generated SHA body changed: '+name)
        return dict(complete=True, index_sha256=sha(ROOT/HERE/'generated-index.json'),
            full_original_ir_value_sha256=row['ir_value_sha256'], published=expected['published'])

    save()
    waiting = time.monotonic()
    try:
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
            expected, actual = lock_path.stat(), os.fstat(lock.fileno())
            if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino):
                raise ValueError('inherited coordination descriptor differs')
            fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if lock_fd is not None else 0))
            started = time.monotonic()
            cpus = sorted(os.sched_getaffinity(0))
            if not 1 <= len(cpus) <= 4:
                raise ValueError('explicit admission of at most four CPUs is required')
            disk = os.statvfs(ROOT)
            if disk.f_bavail*disk.f_frsize < 72*2**30:
                raise ValueError('existing disk reserve plus declared work budget refused')
            record.update(lock_wait_seconds=started-waiting, cpus=cpus,
                coordination_lock=dict(inherited=lock_fd is not None,
                    device=actual.st_dev, inode=actual.st_ino))
            data = support.inventory()
            inputs = support.base.snapshot(ROOT)
            modes = {name:support.mode(ROOT/name) for name in inputs}
            support.resolver.ROOT, support.resolver.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output/'runtime-cache'))
            runtime, registry = support.resolver.runtime(), support.registry_module.registry()
            record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'],
                cwd=ROOT, text=True).strip(), inputs=inputs, input_modes=modes,
                runtime_before=runtime, registry_before=registry)
            captures = []
            folder = output/'original'
            original_project = project_at(folder)
            derivation, changed = support.preparation.view(original_project, 'original')
            keep_source(original_project, changed, 'unchanged-original-method-plus-transparent-root')
            record['phase'] = 'original-direct-library-source-binding'; save()
            original = capture.run(original_project, folder, 'original', support, command, 'original')
            record['original'] = dict(preparation=derivation, capture=original)
            captures.append(original)
            # Recapture the adopted unchanged generic Save and validation roots.
            folder = output/'generic-save'
            generic_project = project_at(folder)
            derivation,changed=support.preparation.generic_view(generic_project)
            keep_source(generic_project, changed, 'unchanged-generic-Save-with-original-transparent-roots')
            record['generic_save'] = dict(preparation=derivation, captures={})
            for label in ('original','canonical','primitive'):
                record['phase']='capture-current-generic-Save-'+label;save()
                captured=capture.generic_save(generic_project,folder,label,support,command)
                record['generic_save']['captures'][label]=captured;captures.append(captured)
                # Preserve exact independent generated bodies and complete IR.
                index=json.loads((ROOT/HERE/'generated-index.json').read_text())
                expected=index['captures']['generic-save-'+label]
                if captured['ir_value_sha256']!=expected['ir_value_sha256']:
                    raise ValueError('actual generic Save IR changed')
                generated=Path(captured['generated_directory'])
                for n,e in expected['published'].items():
                    if sha(generated/n)!=e['sha256'] or sha(ROOT/e['path'])!=e['sha256'] or (generated/n).read_bytes()!=(ROOT/e['path']).read_bytes():
                        raise ValueError('unedited generic Save output differs')
                captured['published_comparison']=dict(complete=True,full_ir_value_sha256=captured['ir_value_sha256'],published=expected['published'])
            error_binding=support.module('save_complete_existing_error_binding',
                ROOT/HERE.parent/'envelope-writer/binding.py')
            original_ir=Path(record['generic_save']['captures']['original']['ir'])
            foundation_ir=ROOT/'verification/aeneas/llbc/sandproof_foundations.llbc'
            shape=error_binding.contract_error(json.loads(original_ir.read_text()))
            if shape!=error_binding.contract_error(json.loads(foundation_ir.read_text())):
                raise ValueError('complete original ContractError differs from canonical data')
            record['generic_save']['canonical_error_binding']=dict(shape=shape,
                original_IR_sha256=sha(original_ir),canonical_IR_sha256=sha(foundation_ir))
            common, all_audits = None, set()
            for variant in support.VIEWS:
                folder = output/variant
                project = project_at(folder)
                derivation, changed = support.preparation.view(project, variant)
                record['phase'] = 'capture-current-SHA-'+variant; save()
                captured = capture.run(project, folder, 'named', support, command)
                captured['published_comparison'] = compare_named(captured, variant)
                captures.append(captured)
                row = dict(complete=False, project=str(project), preparation=derivation,
                    capture=captured, property_rejected=False, correct_core_imported=variant == 'actual')
                record['views'][variant] = row
                if variant == 'actual':
                    record['phase'] = 'candidate-pure-library-source-binding'; save()
                    leaf = capture.run(project, folder, 'leaf', support, command, 'leaf')
                    captures.append(leaf)
                    if leaf['source_binding']['exact_fixed_interfaces'] != original['source_binding']['exact_fixed_interfaces']:
                        raise ValueError('original/candidate library dictionaries or complete data differ')
                    row['leaf_binding'] = leaf
                installed = support.materialize(project, variant, Path(captured['generated_directory']),
                    record['generic_save']['captures'] if variant=='actual' else None)
                keep_source(project, {**changed, **installed}, 'checked-independent-SHA-'+variant)
                seeds = support.retained_roots(variant)
                files = sorted(support.resolver.closure(project,
                    [str(support.PROOFS/(stem+'.lean')) for stem in seeds], active_view='lakefile.toml'))
                hashes = {name:sha(project/name) for name in files}
                audits = [name for filename in files for name in re.findall(
                    r'^#audit_controller_proof (\S+)$', (project/filename).read_text(), re.M)]
                if (len(audits) != len(set(audits)) or files != data['views'][variant]['files']
                        or sorted(audits) != data['views'][variant]['audits']):
                    raise ValueError('missing, extra, duplicate or weakened actual file/audit inventory')
                shared = {name:hashes[name] for name in files if
                    name in [str(support.PROOFS/(stem+'.lean')) for stem in support.parent.MATH+support.parent.COMMON+support.COMMON+support.blob.COMMON]
                    or name in [str(support.GEN/(stem+'.lean')) for stem in ('Types', 'TypesExternal', 'FunsExternal')]}
                if common is not None and shared != common:
                    raise ValueError('correct/wrong complete property, fixture, library, math or types differ')
                common = shared
                all_audits.update(audits)
                row.update(files=hashes, audits=audits, installed=installed, seeds=seeds)
                record['phase'] = variant+'-build-all-retained-roots'; save()
                command(project, folder, 'roots', ['lake', 'build',
                    *['SandboxModel.Lifecycle.Protocol.'+stem for stem in seeds]])
                if prepare_only:
                    row['prepared'] = True; save()
                    continue
                record['phase'] = variant+'-all-real-files-kernel-service-dependencies'; save()
                physical = folder/'actual-files'; physical.mkdir()
                row['actual'] = support.checker.actual_files(project, files, physical, env, command)
                if row['actual']['files'] != hashes or row['actual']['audits'] != audits:
                    raise ValueError('checked actual file/audit inventory differs')
                support.reader._saved_physical_view(row['actual'], project,
                    physical/'physical-files', record['commands'])
                row.update(complete=True, property_rejected=variant != 'actual',
                    all_original_kernel_service_dependencies_read_back=True)
                (folder/'result.json').write_text(json.dumps(row, indent=2)+'\n'); save()
            for source in record['independent_sources']:
                project = Path(source['project'])
                if any((project/n).is_symlink() or sha(project/n) != h
                        or support.mode(project/n) != source['modes'][n] for n,h in source['inputs'].items()):
                    raise ValueError('independent source bytes/modes changed')
            for captured in captures:
                ir = Path(captured['ir'])
                if ir.is_symlink() or sha(ir) != captured['ir_sha256']:
                    raise ValueError('original full compiler IR changed')
                if 'generated_directory' in captured:
                    generated = Path(captured['generated_directory'])
                    if any(p.is_symlink() for p in generated.rglob('*')) or {
                        str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()
                    } != captured['generated_files']:
                        raise ValueError('original complete generated capture changed')
            for row in record['commands']:
                log = Path(row['folder'])/(row['label']+'.log')
                if log.is_symlink() or sha(log) != row['log_sha256']:
                    raise ValueError('original command log changed')
            if (support.base.snapshot(ROOT) != inputs
                    or any(support.mode(ROOT/n) != m for n,m in modes.items())
                    or support.registry_module.registry() != registry or support.resolver.runtime() != runtime):
                raise ValueError('original source/mode/runtime/locked-library completion guard failed')
            record.update(prepared=prepare_only, complete=not prepare_only,
                phase='current-component-prepared-not-accepted' if prepare_only else 'current-local-component-complete',
                all_postguards_complete=True, all_published_generated_comparisons_complete=True,
                runtime_after=runtime, registry_after=registry,
                common_model_values_library_and_types=common, union_audits=sorted(all_audits))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started if 'started' in locals() else None
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps({k:record.get(k) for k in ('complete', 'prepared', 'phase',
        'execution_seconds', 'error')}), flush=True)
    if not record.get('prepared') and not record['complete']:
        raise SystemExit(1)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    run(args.output.resolve(), args.lock.resolve(strict=True), args.prepare_only, args.lock_fd)
