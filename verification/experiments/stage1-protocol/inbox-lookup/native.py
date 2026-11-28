"""Development checks of original/factored/key-only/restored real Inbox lookup.

Does not issue ning or shared acceptance. The same assertions run in every view.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import tomllib
import platform
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/inbox-lookup')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
TEST_ROOT = 'controller-service/src/controller/lifecycle_inbox/tests.rs'
VARIANTS = ('original', 'candidate', 'key-only', 'restored')
EXCLUSIVE_CPUS = 64
CARGO_BOOTSTRAP = """import json, os, sys, time
cpus = list(map(int, sys.argv[1].split(',')))
before = sorted(os.sched_getaffinity(0))
if not set(cpus) <= set(before) or len(cpus) != len(set(cpus)):
    raise RuntimeError('Cargo slot exceeds the task affinity')
os.sched_setaffinity(0, cpus)
actual = sorted(os.sched_getaffinity(0))
jobs = int(os.environ['CARGO_BUILD_JOBS'])
if actual != cpus or jobs != len(cpus):
    raise RuntimeError('Cargo slot or job count differs from admission')
with open(sys.argv[2], 'x') as stream:
    json.dump(dict(parent=before, actual=actual, cargo_jobs=jobs, pid=os.getpid(),
                   started_monotonic_ns=time.monotonic_ns()), stream)
os.execvp(sys.argv[3], sys.argv[3:])
"""


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('inbox_native_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
registry_module = module('inbox_native_registry', ROOT / HERE.parent / 'envelope-writer/native.py')
# Reuse the reviewed archive/source verifier, selecting this actual Cargo lock.
registry_module.HERE = HERE
sha = base.sha


def exclusive_slots():
    if os.environ.get('SANDPROOF_STAGE1_NATIVE_JOBS', '1') != '1':
        raise ValueError('the exclusive lookup cannot run under parallel Stage1 native dispatch')
    cpus = sorted(os.sched_getaffinity(0))
    if len(cpus) != EXCLUSIVE_CPUS:
        raise ValueError(f'exclusive lookup must be started with exactly {EXCLUSIVE_CPUS} CPUs')
    registered = module('lookup_native_lock', ROOT/HERE.parent/'registered.py')
    descriptor, lock = registered.inherited_lock()
    width = EXCLUSIVE_CPUS // len(VARIANTS)
    slots = {variant: cpus[i*width:(i+1)*width] for i, variant in enumerate(VARIANTS)}
    return slots, descriptor, lock


def run(output, *, exclusive=False):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='inbox-lookup-native-development', complete=False, commands=[],
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, host=platform.node())
    command_rows = {variant: [] for variant in VARIANTS}
    slots = {}
    try:
        if exclusive:
            slots, descriptor, lock = exclusive_slots()
            task_cpus = sorted(cpu for slot in slots.values() for cpu in slot)
            record['parallel_execution'] = dict(cpu_budget=EXCLUSIVE_CPUS, slots=slots, lock=lock)
        inputs = base.snapshot(ROOT)
        registry = registry_module.registry()
        packages = tomllib.loads((ROOT/HERE/'Cargo.lock').read_text())['package']
        locked_packages = {p['name']+'@'+p['version'] for p in packages if 'checksum' in p}
        record.update(inputs=inputs, registry=registry, variants={}, tools={},
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        for name, args in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version'])]:
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = dict(path=str(path), sha256=sha(path),
                version=subprocess.check_output([str(path), *args], text=True))

        def run_variant(variant):
            folder = output / variant
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            expected = dict(inputs)
            for name, digest in inputs.items():
                target = project / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, target)
                if sha(target) != digest:
                    raise ValueError('source changed while copying: ' + name)
            if (project / MODEL).read_text().count('fn find_entry(') != 1:
                raise ValueError('adopted production lookup helper missing or duplicated')
            if variant == 'key-only':
                path = project / MODEL
                text = path.read_text()
                old = 'if entry.intent != *intent {'
                new = 'if entry.intent.request.binding.key != intent.request.binding.key {'
                if text.count(old) != 1:
                    raise ValueError('expected exactly one actual full-intent guard')
                path.write_text(text.replace(old, new))
                record['mutation'] = dict(path=MODEL, old=old, new=new,
                    before=expected[MODEL], after=sha(path))
                expected[MODEL] = sha(path)
            with (project / MODEL).open('a') as source:
                source.write('\n' + (ROOT / HERE / 'extract-root.rs.in').read_text())
            expected[MODEL] = sha(project / MODEL)
            with (project / TEST_ROOT).open('a') as source:
                source.write('\n#[path = "../../../../verification/experiments/stage1-protocol/inbox-lookup/native_tests.rs"]\nmod proof_lookup;\n')
            expected[TEST_ROOT] = sha(project / TEST_ROOT)
            slot = slots.get(variant)
            jobs = str(len(slot)) if slot is not None else '2'
            env = dict(os.environ, CARGO_TARGET_DIR=str(folder / 'target'),
                       CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS=jobs)
            for label, args in [('test', ['--lib', '--message-format=json', 'proof_lookup::', '--', '--nocapture']),
                                ('clippy', ['--lib', '--tests', '--', '-D', 'warnings'])]:
                argv = ['cargo', label, '-p', 'sandproof-controller-service', '--locked', '--offline']
                if slot is not None:
                    argv += ['--jobs', jobs]
                argv += args
                command, pass_fds = argv, ()
                if slot is not None:
                    admission = folder / (label + '-cpu-admission.json')
                    command = [sys.executable, '-c', CARGO_BOOTSTRAP, ','.join(map(str, slot)),
                               str(admission), *argv]
                    pass_fds = (descriptor,)
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    done = subprocess.run(command, cwd=project, env=env, stdin=subprocess.DEVNULL,
                        stdout=stream, stderr=subprocess.STDOUT, timeout=1800, pass_fds=pass_fds)
                row = dict(variant=variant, label=label, argv=argv, exit_code=done.returncode,
                    seconds=time.monotonic()-begin, log_sha256=sha(log))
                command_rows[variant].append(row)
                if slot is not None:
                    admitted = json.loads(admission.read_bytes())
                    if (admitted['actual'] != slot or admitted['parent'] != task_cpus
                            or admitted['cargo_jobs'] != len(slot)):
                        raise ValueError('Cargo slot evidence differs from the admitted task')
                    row.update(cpu_admission_sha256=sha(admission),
                               finished_monotonic_ns=time.monotonic_ns())
                    (folder / 'progress.json').write_text(json.dumps(command_rows[variant], indent=2)+'\n')
                else:
                    record['commands'].append(row)
                    (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
                text = log.read_text()
                if label == 'test':
                    artifacts = []
                    for line in text.splitlines():
                        try:
                            value = json.loads(line)
                        except ValueError:
                            continue
                        if value.get('reason') == 'compiler-artifact' and value['package_id'].startswith('registry+'):
                            if value['package_id'].rsplit('#',1)[1] not in locked_packages:
                                raise ValueError('compiled registry package is outside the verified lock')
                        if value.get('reason') == 'compiler-artifact' and value['target']['name'] in (
                                'sandproof_protocol', 'sandproof_controller_service'):
                            artifacts.append(value)
                    if len(artifacts) != 2 or any(a['fresh'] for a in artifacts):
                        raise ValueError('actual protocol library and service test must be freshly compiled')
                    for artifact in artifacts:
                        source = Path(artifact['target']['src_path'])
                        folder_name = 'sandbox-protocol' if artifact['target']['name'] == 'sandproof_protocol' else 'controller-service'
                        if (source != project/folder_name/'src/lib.rs'
                                or artifact['manifest_path'] != str(project/folder_name/'Cargo.toml')
                                or artifact['profile']['test'] != (folder_name == 'controller-service')):
                            raise ValueError('native artifact links the wrong source/manifest/test target')
                    row['artifacts'] = [dict(message=a, files={n: sha(Path(n)) for n in a['filenames']
                        if Path(n).is_file()}) for a in artifacts]
                    if variant == 'key-only':
                        if (done.returncode != 101 or '2 passed; 2 failed; 0 ignored;' not in text or
                                'first_key_conflict_is_not_hidden_by_a_later_exact_replay ... FAILED' not in text or
                                'same_key_changed_full_envelope_is_always_a_conflict ... FAILED' not in text):
                            raise ValueError('wrong Rust did not reach the same full lookup assertions')
                        row['property_rejected'] = True
                    elif done.returncode or '4 passed; 0 failed; 0 ignored;' not in text:
                        raise ValueError('original/candidate/restored lookup assertions failed')
                elif done.returncode:
                    raise ValueError('Clippy failed: ' + variant)
            if any(sha(project / name) != digest for name, digest in expected.items()):
                raise ValueError('checked source changed: ' + variant)
            record['variants'][variant] = dict(inputs=expected, complete=True)
        if slots:
            with ThreadPoolExecutor(max_workers=len(slots)) as pool:
                list(pool.map(run_variant, VARIANTS))
        else:
            for variant in VARIANTS:
                run_variant(variant)
        if base.snapshot(ROOT) != inputs or registry_module.registry() != registry:
            raise ValueError('original sources or dependencies changed during native checking')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    if slots:
        record['commands'] = [row for variant in VARIANTS for row in command_rows[variant]]
        checked = record.get('variants', {})
        record['variants'] = {variant: checked[variant] for variant in VARIANTS if variant in checked}
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--exclusive-64-cpus', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), exclusive=args.exclusive_64_cpus)
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
