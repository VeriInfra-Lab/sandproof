"""Capture current Rust and two real schema mutants with the fixed frontend.

This is source-binding evidence. Aeneas's original full-parser failure remains
separate; native assertions belong to ning and no Lean Rust counterexample is
claimed by a source-admission rejection.
"""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


source = module('accepted_current_source_admission', HERE / 'source_admission.py')
base = module('accepted_parser_original_snapshot', HERE.parent / 'schema-leaves/formal.py')
runtime = module('accepted_parser_fixed_runtime', HERE.parent / 'formal_runtime.py')
batch = module('accepted_parser_runtime_admission', ROOT / 'tools/proof_batch.py')
registry = module('accepted_parser_original_registry', HERE.parent / 'envelope-writer/native.py')
registry.HERE = Path('.')
sha = base.sha

MUTATIONS = {
    'swap-resource-keys': (
        'sandbox-protocol/src/internal/lifecycle.rs',
        '    pub cpu: u64,\n    pub memory_bytes: u64,',
        '    #[serde(rename = "memory_bytes")]\n    pub cpu: u64,\n'
        '    #[serde(rename = "cpu")]\n    pub memory_bytes: u64,'),
    'swap-resume-keys': (
        'sandbox-protocol/src/lifecycle/request.rs',
        '    Resume {\n        source_incarnation: IncarnationId,\n'
        '        capture: CaptureId,\n        target_incarnation: IncarnationId,',
        '    Resume {\n        #[serde(rename = "target_incarnation")]\n'
        '        source_incarnation: IncarnationId,\n        capture: CaptureId,\n'
        '        #[serde(rename = "source_incarnation")]\n        target_incarnation: IncarnationId,'),
}


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def guard_baseline():
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    source.require(subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, head], cwd=ROOT,
                                  check=False).returncode == 0, 'capture is not based on the fixed integration')
    # Other adopted V3 components do not have to remain at BASE. Freeze the
    # actual protocol/helper/dependency sources instead; the complete current
    # input snapshot and mode postguard below still bind every captured file.
    frozen = ['sandbox-protocol/src', 'sandbox-protocol/Cargo.toml',
              'sandbox-internal/src/lifecycle.rs', 'sandbox-internal/Cargo.toml',
              'Cargo.toml', 'Cargo.lock']
    names = subprocess.check_output(['git', 'ls-tree', '-r', '--name-only', '-z', BASE,
                                     '--', *frozen], cwd=ROOT).decode().split('\0')
    source.require(bool(list(filter(None, names))), 'missing fixed parser source inputs')
    for name in filter(None, names):
        original = subprocess.check_output(['git', 'show', BASE + ':' + name], cwd=ROOT)
        path = ROOT / name
        source.require(path.is_file() and not path.is_symlink() and path.read_bytes() == original,
                       'reviewed parser source changed: ' + name)
    # The private probe uses this exact production closure, not a replacement
    # parser. Changes elsewhere in the handler (e.g. full record reply) are
    # outside this source-binding claim and remain A's own proof obligations.
    call = ('crate::lifecycle::decode_accepted_with(&body.to_bytes(), &context, |input| {\n'
            '        serde_json::from_slice::<AcceptedLifecycleIntent>(input)\n    })')
    production = ROOT / 'sandbox-internal/src/server/lifecycle.rs'
    source.require(production.is_file() and not production.is_symlink()
                   and production.read_text().count(call) == 1,
                   'original production parser call/slice/context changed')
    return head


def run(output):
    output.mkdir(parents=True, mode=0o700)
    record = dict(kind='current-Accepted-parser-source-captures-not-Lean-proof',
                  source_evidence_complete=False, formal_proof=False,
                  native_acceptance=False, shared_acceptance=False, full_v3=False,
                  source_base=BASE, commands=[], cases={})
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               CARGO_BUILD_JOBS='4', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')
    os.environ.update({name: env[name] for name in (
        'CARGO_NET_OFFLINE', 'LEAN_NUM_THREADS', 'LEAN_STACK_SIZE_KB')})

    def save():
        (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')

    def command(project, folder, label, argv):
        started = time.monotonic()
        log = folder / (label + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        row = dict(argv=argv, cwd=str(project), exit_code=result.returncode,
                   seconds=time.monotonic()-started, log=str(log), log_sha256=sha(log))
        record['commands'].append(row)
        save()
        if result.returncode:
            raise ValueError('actual compiler/capture failed; not a property rejection')

    waiting = time.monotonic()
    save()
    with (ROOT.parent / '.proof-heavy.lock').open('r+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        started = time.monotonic()
        inputs = modes = tools = libraries = None
        try:
            source.require(1 <= len(os.sched_getaffinity(0)) <= 4, 'proof CPU allocation differs')
            disk = os.statvfs(ROOT)
            source.require(disk.f_bavail * disk.f_frsize >= 72 * 2**30, 'insufficient private capture space')
            record['source_head'] = guard_baseline()
            inputs = base.snapshot(ROOT)
            modes = {name: mode(ROOT / name) for name in inputs}
            batch.ROOT, batch.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output / 'runtime-cache'))
            tools, libraries = batch.runtime(), registry.registry()
            record.update(inputs=inputs, input_modes=modes, runtime=tools, library_registry=libraries,
                          cpus=sorted(os.sched_getaffinity(0)), lock_wait_seconds=started-waiting)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            source.require(sha(ROOT / '.tools/aeneas/charon/bin/charon') == fixed['charon']['binary_sha256'],
                           'fixed Charon changed')
            schema = json.loads((HERE.parent / 'envelope-schema/inventory.json').read_text())
            fragment = (HERE / 'roots.rs.in').read_bytes()
            for case in ('correct', *MUTATIONS):
                folder = output / case
                project = folder / 'source'
                project.mkdir(parents=True, mode=0o700)
                for name, wanted in inputs.items():
                    path = project / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(ROOT / name, path)
                    source.require(sha(path) == wanted, 'capture source changed while copying')
                (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
                (project / '.lake').mkdir()
                (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
                (project / 'artifacts/extract-target').mkdir(parents=True)
                expected = dict(inputs)
                root_name = 'sandbox-internal/src/server/lifecycle.rs'
                path = project / root_name
                original = path.read_bytes()
                source.require(original.count(b'serde_json::from_slice::<AcceptedLifecycleIntent>(input)') == 1
                               and b'proof_current_accepted_parser' not in original,
                               'current route callback no longer matches the transparent probe')
                path.write_bytes(original + fragment)
                expected[root_name] = sha(path)
                if case != 'correct':
                    name, before, after = MUTATIONS[case]
                    path = project / name
                    text = path.read_text()
                    source.require(text.count(before) == 1, 'real Rust mutation site changed')
                    path.write_text(text.replace(before, after))
                    expected[name] = sha(path)
                virtual = runtime.VIRTUAL
                argv = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                        '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                        '--bind', str(project), str(virtual), '--chdir', str(virtual),
                        '--setenv', 'CARGO_TARGET_DIR', str(virtual / 'artifacts/extract-target'),
                        '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                        '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--',
                        str(virtual / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                        '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
                        '--include', 'sandproof_internal', '--start-from',
                        'crate::server::lifecycle::proof_current_accepted_parser', '--dest-file',
                        str(virtual / 'artifacts/current-accepted-parser.llbc'), '--', '--manifest-path',
                        'sandbox-internal/Cargo.toml', '--lib', '--locked', '--offline', '--target', fixed['target']]
                record['phase'] = case
                command(project, folder, 'charon', argv)
                ir_path = project / 'artifacts/current-accepted-parser.llbc'
                raw = json.loads(ir_path.read_text())
                options = raw['translated']['options']
                source.require(raw['has_errors'] is False and not options['opaque'] and
                               not options['exclude'] and not options['skip_borrowck'] and
                               not options['no_typecheck'] and not options['monomorphize'] and
                               not options['remove_unused_clauses'], 'incomplete or weakened original capture')
                rejection = None
                try:
                    admitted = source.report(raw, schema)
                except ValueError as error:
                    if case == 'correct' or str(error) != 'original input field name routed to another field':
                        raise
                    rejection = str(error)
                    admitted = None
                source.require((admitted is not None) == (case == 'correct'),
                               'real Rust schema mutant was not rejected by the same source contract')
                case_result = dict(compiled_and_captured=True, llbc=str(ir_path), llbc_sha256=sha(ir_path),
                    source_inputs=expected, source_contract_accepted=admitted is not None,
                    source_contract_rejection=rejection, Lean_Rust_counterexample=False)
                if admitted is not None:
                    (folder / 'source-report.json').write_text(json.dumps(admitted, indent=2) + '\n')
                    case_result['source_report_sha256'] = sha(folder / 'source-report.json')
                for name, wanted in expected.items():
                    source.require(not (project / name).is_symlink() and sha(project / name) == wanted and
                                   mode(project / name) == modes[name], 'captured source/mode changed')
                source.require((project / root_name).read_bytes().removesuffix(fragment) == original,
                               'thin probe changed the original route')
                if case != 'correct':
                    name, before, after = MUTATIONS[case]
                    restored = (project / name).read_text().replace(after, before)
                    source.require(restored.encode() == (ROOT / name).read_bytes(),
                                   'schema mutation does not invert to the exact original Rust')
                record['cases'][case] = case_result
                save()
            record['source_evidence_complete'] = True
        except BaseException as error:
            record.update(source_evidence_complete=False, error=repr(error), traceback=traceback.format_exc())
        try:
            source.require(inputs is not None and modes is not None and tools is not None and libraries is not None,
                           'capture never completed its original input admission')
            source.require(base.snapshot(ROOT) == inputs and all(mode(ROOT / name) == value
                           for name, value in modes.items()), 'current source/modes changed during capture')
            source.require(batch.runtime() == tools and registry.registry() == libraries,
                           'fixed runtime/library registry changed during capture')
            source.require(all(sha(Path(row['log'])) == row['log_sha256'] for row in record['commands']),
                           'original capture log changed')
            record['all_source_mode_tool_library_log_postguards_complete'] = True
        except BaseException as error:
            record.update(source_evidence_complete=False, postguard_error=repr(error))
        record['seconds'] = time.monotonic()-started
        (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({key: result.get(key) for key in (
        'source_evidence_complete', 'seconds', 'error', 'postguard_error')}))
    raise SystemExit(0 if result['source_evidence_complete'] else 1)
