"""Capture the adopted reply helper and four existing transparent value roots."""
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
HERE = Path('verification/experiments/stage1-protocol/record-reply')
spec = importlib.util.spec_from_file_location('reply_capture_support', ROOT/HERE/'support.py')
support = importlib.util.module_from_spec(spec)
spec.loader.exec_module(support)
sha = support.sha


def run(output, variant):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='actual-adopted-record-reply-capture', variant=variant,
        capture_complete=False, translation_complete=False, proof_complete=False,
        commands=[], generated_bodies_modified=False, axiom_templates_imported=False,
        private_business_replacement=False, production_adopted=True,
        ordinary_checks_run=False, native_checks_run=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true')

    def command(project, label, argv):
        begin = time.monotonic()
        log = output/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=done.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if done.returncode:
            raise ValueError('capture failure is not a property rejection: '+label)

    try:
        data = support.inventory()
        if variant not in support.CASES:
            raise ValueError('unknown reply source view')
        inputs = support.base.snapshot(ROOT)
        modes = {name: support.mode(ROOT/name) for name in inputs}
        record.update(inputs=inputs, input_modes=modes,
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed proof tool changed: '+name)
        record['tools'] = fixed
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, target)
            if sha(target) != digest or support.mode(target) != modes[name]:
                raise ValueError('source or mode changed during copying: '+name)
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        mutation = support.mutate(project, data['variants'][variant]['mutation'])
        expected[support.MODEL] = sha(project/support.MODEL)
        lib = project/'sandbox-internal/src/lib.rs'
        if b'proof_record_reply_roots' in lib.read_bytes():
            raise ValueError('transparent reply roots were already appended')
        lib.write_bytes(lib.read_bytes()+(ROOT/HERE/'roots.rs.in').read_bytes())
        expected['sandbox-internal/src/lib.rs'] = sha(lib)
        record.update(project=str(project), source_mutation=mutation,
            private_inputs=expected, independent_cargo_target=True, independent_lake_build=True)
        paths = [Path.home(), support.runtime.tool_root(ROOT), support.runtime.package_root(ROOT)]
        for key in ('CARGO_HOME', 'RUSTUP_HOME', 'ELAN_HOME', 'SANDPROOF_PROOF_RUNTIME_ROOT'):
            if os.environ.get(key):
                paths.append(Path(os.environ[key]).resolve(strict=True))
        virtual = support.runtime.VIRTUAL
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home',
            *[arg for path in dict.fromkeys(paths) for arg in ('--ro-bind', str(path), str(path))],
            '--bind', str(project), str(virtual), '--chdir', str(virtual),
            '--setenv', 'CARGO_TARGET_DIR', str(virtual/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        roots = data['roots']
        include = ['sandproof_protocol', 'sandproof_internal']
        args = [str(virtual/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default']
        for name in include:
            args += ['--include', name]
        for name in roots:
            args += ['--start-from', name]
        args += ['--dest-file', str(virtual/'artifacts/record-reply.llbc'), '--',
            '--manifest-path', 'sandbox-internal/Cargo.toml', '--lib', '--locked', '--offline',
            '--target', fixed['target']]
        command(project, 'charon', wrapper+args)
        compiler_log = re.sub(r'\x1b\[[0-9;]*m', '', (output/'charon.log').read_text())
        for package, folder in [('sandproof-protocol', 'sandbox-protocol'), ('sandproof-internal', 'sandbox-internal')]:
            if not re.search(r'Compiling '+re.escape(package)+r' v[^\n]+\('+
                    re.escape(str(virtual/folder))+r'\)', compiler_log):
                raise ValueError('actual own crate was not freshly compiled: '+package)
        ir = project/'artifacts/record-reply.llbc'
        if not ir.is_file() or ir.is_symlink() or ir.stat().st_size > 16*2**20:
            raise ValueError('missing or oversized original LLBC')
        raw = json.loads(ir.read_text())
        options = raw['translated']['options']
        if (raw.get('has_errors') is not False or options['start_from'] != roots
                or options['include'] != include or options['opaque'] or options['exclude']
                or options['skip_borrowck'] or options['no_typecheck'] or options['monomorphize']
                or options['remove_unused_clauses']):
            raise ValueError('unreviewed capture options or incomplete LLBC')
        record.update(capture_complete=True, llbc_sha256=sha(ir), ir_options=options,
            fresh_actual_packages=['sandproof-protocol', 'sandproof-internal'])
        generated = output/'generated'
        generated.mkdir()
        command(project, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofStage1RequestRust', '-subdir', 'SandproofStage1Request',
            '-dest', str(generated), str(ir)])
        functions = json.loads((generated/'translation.json').read_text())['functions']
        actual = [row for row in functions if row['lean_name'] ==
            'SandproofStage1RequestRust.server.lifecycle.record_reply']
        if len(actual) != 1 or actual[0]['is_opaque'] is not False:
            raise ValueError('actual reply helper was hidden or missing')
        externals = sorted(row['lean_name'] for row in functions if row['is_opaque'])
        if externals != data['external_value_interfaces']:
            raise ValueError('new external interface or hidden business function')
        record.update(translation_complete=True, actual_helper=actual[0], external_value_interfaces=externals,
            generated={str(path.relative_to(generated)): sha(path)
                for path in generated.rglob('*') if path.is_file()})
        if any(sha(project/name) != digest for name, digest in expected.items()):
            raise ValueError('private capture source changed')
        if any(support.mode(project/name) != value for name, value in modes.items()):
            raise ValueError('private capture source mode changed')
        if support.base.snapshot(ROOT) != inputs or any(support.mode(ROOT/name) != value for name, value in modes.items()):
            raise ValueError('public capture source or mode changed')
        if support.inventory() != data:
            raise ValueError('reply input inventory changed during capture')
    except BaseException as error:
        record.update(translation_complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=support.CASES, default='actual')
    parser.add_argument('--lock', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    with args.lock.open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        result = run(args.output.absolute(), args.variant)
    print(json.dumps({k: result.get(k) for k in ('capture_complete', 'translation_complete', 'seconds', 'error')}))
    raise SystemExit(not result['translation_complete'])
