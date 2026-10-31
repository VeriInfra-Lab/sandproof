"""Reproduce all actual candidate/read/domain views with the original checker."""
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
HERE = Path('verification/experiments/stage1-protocol/standard-blob')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('current_standard_blob_support', ROOT/HERE/'support.py')
capture = module('current_standard_blob_capture', ROOT/HERE/'capture.py')
sha = support.sha


def run(output, lock_path, prepare_only=False, lock_fd=None):
    output.mkdir(mode=0o700, parents=True)
    record = dict(kind='current-standard-blob-and-original-load-domain-development', complete=False,
        commands=[], views={}, source_candidate_adopted=False, finite_library_provider_adopted=False,
        generated_bodies_modified=False, new_protocol_or_load_algorithm=False,
        whole_callback_assumption_added=False, ordinary_checks_run=False, native_checks_run=False,
        formal_acceptance=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    # Runtime admission and the same-service client observe these same options.
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
            raise ValueError('development failure, not a contract counterexample: '+label)

    def project_at(folder):
        project = folder/'source'
        project.mkdir(mode=0o700, parents=True)
        for name, digest in inputs.items():
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, target)
            if sha(target) != digest or support.mode(target) != modes[name]:
                raise ValueError('source changed while preparing actual view')
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        return project

    def install_raw(row, destination, project):
        source = Path(row['generated_directory'])/destination.name
        changed = {}
        for name in ('Types.lean', 'Funs.lean'):
            target = project/destination/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source/name, target)
            changed[str(target.relative_to(project))] = sha(target)
        return changed

    save()
    waiting = time.monotonic()
    try:
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
            expected, actual = lock_path.stat(), os.fstat(lock.fileno())
            if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino):
                raise ValueError('inherited descriptor is not the same coordination lock')
            fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if lock_fd is not None else 0))
            started = time.monotonic()
            cpus = sorted(os.sched_getaffinity(0))
            if not 1 <= len(cpus) <= 4:
                raise ValueError('this serial proof task requires an explicitly granted group of at most four CPUs')
            disk = os.statvfs(ROOT)
            if disk.f_bavail*disk.f_frsize < 64*2**30+2*4*2**30:
                raise ValueError('original disk reserve and declared work budget refused')
            record.update(lock_wait_seconds=started-waiting, coordination_lock=dict(
                inherited=lock_fd is not None, device=actual.st_dev, inode=actual.st_ino), cpus=cpus)
            data = support.inventory()
            inputs = support.base.snapshot(ROOT)
            modes = {name: support.mode(ROOT/name) for name in inputs}
            support.resolver.ROOT, support.resolver.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output/'runtime-cache'))
            runtime = support.resolver.runtime()
            registry = support.registry_module.registry()
            record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'],
                cwd=ROOT, text=True).strip(), inputs=inputs, input_modes=modes,
                fixed_runtime_before=runtime, registry=registry)
            folder = output/'load'
            load_project = project_at(folder)
            derivation, load_changed = support.preparation.load_view(load_project)
            record['load'] = dict(project=str(load_project), preparation=derivation,
                                  prepared_inputs=load_changed, captures={})
            for label, kind in [('original', 'load-original'), ('canonical', 'load-canonical'),
                                ('primitive', 'primitive')]:
                record['phase'] = 'capture-current-'+kind
                save()
                row = capture.run(load_project, folder, label, support, command, kind)
                row['published_comparison'] = support.check_capture(row, kind)
                record['load']['captures'][label] = row
                save()
            error_binding = support.module('blob_existing_complete_error_binding',
                ROOT/HERE.parent/'envelope-writer/binding.py')
            original_ir = Path(record['load']['captures']['original']['ir'])
            original = error_binding.contract_error(json.loads(original_ir.read_text()))
            foundation_ir = ROOT/'verification/aeneas/llbc/sandproof_foundations.llbc'
            if original != error_binding.contract_error(json.loads(foundation_ir.read_text())):
                raise ValueError('current complete original ContractError differs from canonical data')
            record['load']['canonical_error_binding'] = dict(complete_original_shape=original,
                original_IR_sha256=sha(original_ir), canonical_IR_sha256=sha(foundation_ir))
            all_audits = set()
            common = None
            for variant in support.VIEWS:
                folder = output/variant
                project = project_at(folder)
                derivation, changed = support.preparation.read_view(project, variant)
                record['phase'] = 'capture-current-standard-'+variant
                save()
                standard = capture.run(project, folder, 'standard', support, command, 'standard')
                standard['published_comparison'] = support.check_capture(standard, 'standard-'+variant)
                row = dict(complete=False, project=str(project), preparation=derivation,
                    prepared_inputs=changed, captures=dict(standard=standard), property_rejected=False,
                    correct_core_imported=variant == 'actual')
                record['views'][variant] = row
                expected_inputs = {**inputs, **changed}
                if variant == 'actual':
                    row['load_captures'] = record['load']['captures']
                    # This is a Lean source view of independently captured actual
                    # methods. Their complete original Rust source views remain
                    # separate and are each bound above, not relabelled.
                    load_inputs = {**inputs, **load_changed}
                    if any(sha(load_project/name) != digest for name, digest in load_inputs.items()):
                        raise ValueError('original Load capture source changed')
                installed = support.materialize(project, variant)
                installed.update(install_raw(standard, support.READ_GEN, project))
                if variant == 'actual':
                    installed.update(install_raw(record['load']['captures']['canonical'], support.LOAD_GEN, project))
                    installed.update(install_raw(record['load']['captures']['primitive'], support.GRAPHIC_GEN, project))
                expected_inputs.update(installed)
                shared = {name: installed[name] for name in installed
                    if name.startswith(str(support.READ_GEN)) and not name.endswith('/Funs.lean')}
                shared.update({str(support.PROOFS/(stem+'.lean')): installed[str(support.PROOFS/(stem+'.lean'))]
                    for stem in support.COMMON})
                if common is not None and shared != common:
                    raise ValueError('correct/wrong complete model, fixture, types or library differs')
                common = shared
                seeds = [str(support.PROOFS/(stem+'.lean')) for stem in support.VIEWS[variant]]
                files = sorted(support.resolver.closure(project, seeds, active_view='lakefile.toml'))
                hashes = {name: sha(project/name) for name in files}
                audits = [name for filename in files for name in re.findall(
                    r'^#audit_controller_proof (\S+)$', (project/filename).read_text(), re.M)]
                if len(audits) != len(set(audits)):
                    raise ValueError('duplicate actual proof audit')
                if (files != data['views'][variant]['files']
                        or sorted(audits) != data['views'][variant]['audits']):
                    raise ValueError('missing, extra or weakened retained view/audit inventory')
                all_audits.update(audits)
                source_modes = {name: support.mode(project/name) for name in expected_inputs}
                row.update(installed=installed, source_files=expected_inputs,
                    source_modes=source_modes, files=hashes, audits=audits)
                record['phase'] = variant+'-build-current-real-roots'
                save()
                command(project, folder, 'view-build', ['lake', 'build',
                    *['SandboxModel.Lifecycle.Protocol.'+stem for stem in support.VIEWS[variant]]])
                if prepare_only:
                    row['prepared'] = True
                    save()
                    continue
                record['phase'] = variant+'-all-real-files-kernel-service-dependencies'
                save()
                physical = folder/'actual-files'
                physical.mkdir()
                row['actual'] = support.checker.actual_files(project, files, physical, env, command)
                if row['actual']['files'] != hashes or row['actual']['audits'] != audits:
                    raise ValueError('actual file audit inventory differs')
                support.reader._saved_physical_view(row['actual'], project,
                    physical/'physical-files', record['commands'])
                if any(sha(project/name) != digest for name, digest in expected_inputs.items()):
                    raise ValueError('actual checked source changed')
                if any(support.mode(project/name) != value for name, value in modes.items()):
                    raise ValueError('actual source mode changed')
                row.update(complete=True, property_rejected=variant != 'actual',
                    all_original_kernel_service_dependencies_read_back=True)
                (folder/'result.json').write_text(json.dumps(row, indent=2)+'\n')
                save()
            for row in record['views'].values():
                project = Path(row['project'])
                for name, digest in row['source_files'].items():
                    if ((project/name).is_symlink() or sha(project/name) != digest
                            or support.mode(project/name) != row['source_modes'][name]):
                        raise ValueError('actual independent source/mode completion guard failed')
            for name, digest in {**inputs, **load_changed}.items():
                if ((load_project/name).is_symlink() or sha(load_project/name) != digest
                        or support.mode(load_project/name) != modes[name]):
                    raise ValueError('original independent Load source/mode completion guard failed')
            captures = list(record['load']['captures'].values())
            captures += [row['captures']['standard'] for row in record['views'].values()]
            for captured in captures:
                ir = Path(captured['ir'])
                generated = Path(captured['generated_directory'])
                if ir.is_symlink() or sha(ir) != captured['ir_sha256']:
                    raise ValueError('original captured IR changed')
                if any(p.is_symlink() for p in generated.rglob('*')):
                    raise ValueError('linked original generated capture')
                current = {str(p.relative_to(generated)): sha(p)
                           for p in generated.rglob('*') if p.is_file()}
                if current != captured['generated_files']:
                    raise ValueError('original generated capture changed')
            for row in record['commands']:
                log = Path(row['folder'])/(row['label']+'.log')
                if log.is_symlink() or sha(log) != row['log_sha256']:
                    raise ValueError('original command log changed')
            if (support.base.snapshot(ROOT) != inputs
                    or any(support.mode(ROOT/name) != value for name, value in modes.items())
                    or support.registry_module.registry() != registry
                    or support.resolver.runtime() != runtime):
                raise ValueError('source/mode/fixed-runtime/registry completion guard failed')
            record.update(prepared=prepare_only, complete=not prepare_only,
                phase='current-component-prepared-not-accepted' if prepare_only else 'current-local-component-complete',
                fixed_runtime_after=runtime, all_postguards_complete=True,
                all_published_generated_comparisons_complete=True,
                common_model_values_library_and_types=common, union_audits=sorted(all_audits))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['execution_seconds'] = time.monotonic()-started if 'started' in locals() else None
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps({name: record.get(name) for name in ('complete', 'prepared', 'phase',
        'execution_seconds', 'error')}), flush=True)
    if not record.get('prepared') and not record['complete']:
        raise SystemExit(1)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    parser.add_argument('--prepare-only', action='store_true')
    arguments = parser.parse_args()
    run(arguments.output.resolve(), arguments.lock.resolve(strict=True), arguments.prepare_only,
        arguments.lock_fd)
