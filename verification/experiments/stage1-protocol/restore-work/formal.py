"""Reextract all current restore-work views and check every physical proof file."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

import support

ROOT, HERE, sha, require = support.ROOT, support.HERE, support.sha, support.require


def run_view(output, case, data, inputs, modes, env):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    record = dict(kind='actual-restore-work-development-view', case=case, complete=False,
                  property_rejected=False, business_algorithm_replaced=False, new_axioms=False,
                  business_opacity_added=False, generated_bodies_modified=False, commands=[])

    def command(project, folder, label, argv):
        log = folder / (label + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(label=label, argv=list(map(str, argv)), cwd=str(project),
            exit_code=result.returncode, seconds=time.monotonic()-begin,
            log=str(log), log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        require(result.returncode == 0, 'command failed: ' + str(log))

    try:
        project = output / 'source'
        support.copy_source(project, inputs)
        expected = dict(inputs)
        mutation = support.mutate(project, case)
        if mutation:
            expected[mutation['source']] = mutation['mutated_sha256']
        record.update(source_inputs=expected, mutation=mutation)
        ir, evidence = support.capture(project, output, command)
        record['capture'] = evidence
        generated = output / 'generated'
        generated.mkdir()
        command(project, output, 'aeneas', [str(ROOT / '.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofRestoreWorkRust', '-subdir', 'SandproofRestoreWork',
            '-dest', str(generated), str(ir)])
        record['private_public_lake_seed'] = support.seed_public_lake(project)
        installed, files, audits = support.render(project, case, generated, data)
        require(files == data['cases'][case]['files'] and audits == data['cases'][case]['audits'],
                'missing, extra or changed physical file/audit inventory')
        expected.update(installed)
        record.update(installed=installed, files={name: sha(project / name) for name in files}, audits=audits)
        seeds = [name[:-5].replace('/', '.') for name in installed if name.startswith(str(support.PROOFS))]
        command(project, output, 'build', ['lake', 'build', *seeds])
        actual = output / 'actual-files'
        actual.mkdir()
        record['actual'] = support.checker.actual_files(project, files, actual, env, command)
        require(record['actual']['files'] == record['files'] and record['actual']['audits'] == audits,
                'complete original physical checker inventory differs')
        require(all(not (project / name).is_symlink() and sha(project / name) == digest
                    for name, digest in expected.items()), 'checked private source or proof changed')
        require(all(support.mode(project / name) == mode for name, mode in modes.items()),
                'private input mode changed')
        if mutation:
            path = project / mutation['source']
            original = (ROOT / mutation['source']).read_text()
            require(original.count(mutation['before']) == 1 and
                    path.read_text() == original.replace(mutation['before'], mutation['after']),
                    'real wrong source no longer equals the exact original mutation')
        record.update(complete=True, property_rejected=case != 'correct',
                      service_log_sha256=record['actual']['service_log_sha256'])
    except BaseException as error:
        record.update(complete=False, property_rejected=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    record['generated_files'] = {str(path.relative_to(output / 'generated')): sha(path)
                                for path in (output / 'generated').rglob('*') if path.is_file()}
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


def run(output, lock_path, lock_fd=None):
    output.mkdir(parents=True, mode=0o700)
    record = dict(kind='actual-restore-work-development-batch', complete=False, cases={},
                  native_checks_run=False, formal_acceptance=False, ning_acceptance=False,
                  shared_acceptance=False, full_v3=False, source_base=support.BASE)
    waiting = time.monotonic()
    started = None
    try:
        with (lock_path.open('r+') if lock_fd is None else os.fdopen(os.dup(lock_fd), 'r+')) as lock:
            actual, expected = os.fstat(lock.fileno()), lock_path.stat()
            require((actual.st_dev, actual.st_ino) == (expected.st_dev, expected.st_ino),
                    'inherited descriptor identifies another lock')
            fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if lock_fd is not None else 0))
            started = time.monotonic()
            require(1 <= len(os.sched_getaffinity(0)) <= 4, 'proof CPU allocation differs')
            require(os.statvfs(ROOT).f_bavail * os.statvfs(ROOT).f_frsize >= 72 * 2**30,
                    'insufficient private capture space')
            record.update(lock_wait_seconds=started-waiting,
                          source_commit=support.source_guard(), cpus=sorted(os.sched_getaffinity(0)),
                          coordination_lock=dict(inherited=lock_fd is not None, device=actual.st_dev,
                                                 inode=actual.st_ino, path=str(lock_path)))
            data = support.inventory()
            inputs = support.native.base.snapshot(ROOT)
            modes = {name: support.mode(ROOT / name) for name in inputs}
            support.resolver.ROOT, support.resolver.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output / 'runtime-cache'))
            runtime = support.resolver.runtime()
            libraries = support.native.registry.registry()
            record.update(inputs=inputs, input_modes=modes, runtime=runtime, registry=libraries)
            env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
                       LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536', CARGO_BUILD_JOBS='4')
            for case in support.CASES:
                row = run_view(output / case, case, data, inputs, modes, env)
                record['cases'][case] = dict(row, result_path=str(output / case / 'result.json'),
                                             result_sha256=sha(output / case / 'result.json'))
                (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
                require(row['complete'] and row['property_rejected'] == (case != 'correct'),
                        'restore work development view failed: ' + case)
            require(support.native.base.snapshot(ROOT) == inputs and
                    all(support.mode(ROOT / name) == value for name, value in modes.items()),
                    'original source or modes changed during proof')
            require(support.resolver.runtime() == runtime and support.native.registry.registry() == libraries and
                    support.inventory() == data, 'fixed runtime, libraries or inventory changed')
            require(all(sha(Path(command['log'])) == command['log_sha256'] for row in record['cases'].values()
                        for command in row['commands']), 'original command log changed')
            record.update(complete=True, all_three_views_complete=True,
                          all_source_mode_tool_library_log_postguards_complete=True)
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started if started is not None else None
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared heavy-proof lock')
    parser.add_argument('--lock-fd', type=int, help='inherited open description of the already held lock')
    args = parser.parse_args()
    os.umask(0o077)
    cpus = sorted(os.sched_getaffinity(0))
    if len(cpus) > 4:
        os.sched_setaffinity(0, set(cpus[:4]))
    os.environ.update(LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')
    result = run(args.output.absolute(), args.lock.resolve(strict=True), args.lock_fd)
    print(json.dumps({name: result.get(name) for name in ('complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
