"""Scoped original-tool replay and actual-file checks; not the full V0 acceptance."""
from pathlib import Path
import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
HERE = ROOT / 'verification/experiments/stage1-protocol'
LIBRARY = Path('verification/aeneas/stage1-protocol')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
POSITIVE = ['Model', 'Carriers', 'RequestValues', 'Request', 'LoopWitness',
            'ManifestWitnesses', 'Manifest']
NEGATIVE = {
    'same-incarnation': ('NegativeResume', ['Model', 'RequestValues'], 2),
    'key-only-replay': ('NegativeReplay', ['Model', 'RequestValues'], 2),
    'skip-capture-binding': ('NegativeManifest', ['Model', 'LoopWitness', 'ManifestWitnesses'], 25),
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def generated_files(profile):
    return [str(p.relative_to(ROOT)) for p in sorted((ROOT / LIBRARY / profile).rglob('*.lean'))]


def actual_files(project, files, output, env, run):
    """Use the existing strict batch client, with an explicit component inventory."""
    before = {n: sha(project / n) for n in files}
    for index, name in enumerate(files):
        run(project, output, 'kernel-' + str(index), ['lake', 'env', 'lean', name])
    service = load_module('stage1_actual_service', project / 'tools/proof_services.py')
    # Only select this component's physical files; do not change validator logic.
    service.ROOT, service.FILES = project, tuple(files)
    previous = set((project / 'artifacts').glob('physical-service-*'))
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout):
        service.main()
    created = set((project / 'artifacts').glob('physical-service-*')) - previous
    if len(created) != 1:
        raise ValueError('expected one original batch-service evidence directory')
    physical = output / 'physical-files'
    shutil.move(str(created.pop()), physical)
    expected = [n for name in files for n in re.findall(
        r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M)]
    observed = re.findall(r'^Checked (\S+): \d+ standard logical dependencies$', stdout.getvalue(), re.M)
    if len(set(expected)) != len(expected) or sorted(expected) != sorted(observed):
        raise ValueError('missing, duplicate or extra component audit messages')
    if before != {n: sha(project / n) for n in files}:
        raise ValueError('physical files changed during checking')
    (output / 'actual-service.stdout').write_text(stdout.getvalue())
    # Forward only the original service's admitted audit messages.
    for line in stdout.getvalue().splitlines():
        if line.startswith('Checked '):
            print(line, flush=True)
    return dict(files=before, audits=expected, service_log_sha256=sha(physical / 'service.log'))


def replay(output, archive, native, native_archive, selected):
    output.mkdir(mode=0o700, parents=True)
    started = time.monotonic()
    record = dict(kind='stage1-protocol-scoped-formal-component', cases={}, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536')
    frozen = json.loads((HERE / 'frozen.json').read_text())

    def check_inputs(project):
        for name, digest in frozen['inputs'].items():
            if sha(project / name) != digest:
                raise ValueError('frozen module input differs: ' + name)

    def run(project, folder, name, argv):
        begin = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        row = dict(argv=argv, cwd=str(project), exit_code=code,
                   seconds=time.monotonic()-begin, log_sha256=sha(log))
        record['commands'].append(row)
        if code:
            raise ValueError('component command failed: ' + name)

    try:
        check_inputs(ROOT)
        if 'correct' in selected:
            folder = output / 'correct'
            folder.mkdir(mode=0o700)
            run(ROOT, folder, 'extract', [sys.executable, str(HERE / 'extract.py'),
                                        '--output', str(folder / 'extraction')])
            sys.path.insert(0, str(ROOT))
            from tools.check_foundations_extraction import llbc_digest
            for profile, expected in frozen['generations'].items():
                fresh = folder / 'extraction' / profile
                if (llbc_digest(json.loads((fresh / 'original.llbc').read_text())) != expected['llbc']
                        or llbc_digest(json.loads((ROOT / LIBRARY / profile / 'original.llbc').read_text())) != expected['llbc']):
                    raise ValueError('complete original IR differs: ' + profile)
                for name, digest in expected['generated'].items():
                    if sha(fresh / 'generated' / name) != digest or sha(ROOT / LIBRARY / profile / name) != digest:
                        raise ValueError('unmodified generated output differs: ' + name)
            files = [str(PROOFS / (name + '.lean')) for name in POSITIVE]
            files += [n for p in ['carriers', 'request', 'manifest'] for n in generated_files(p)]
            run(ROOT, folder, 'build', ['lake', 'build', *['SandboxModel.Lifecycle.Protocol.' + n for n in POSITIVE]])
            actual = actual_files(ROOT, files, folder, env, run)
            if len(actual['audits']) != 38:
                raise ValueError('correct component requires exactly 38 audit declarations')
            record['cases']['correct'] = dict(complete=True, **actual)
        wrong = [name for name in selected if name != 'correct']
        if wrong:
            if archive is None or native is None or native_archive is None:
                raise ValueError('isolated source archive and original native mutant evidence required')
            native_record = json.loads(native.read_text())
            if native_record.get('success') is not True:
                raise ValueError('native component is incomplete')
            if sha(native_archive) != native_record['archive_sha256']:
                raise ValueError('original native source archive differs')
            with tarfile.open(native_archive) as native_tar:
                for file, digest in frozen['native_rust_inputs'].items():
                    entry = native_tar.getmember(file)
                    if not entry.isfile() or hashlib.sha256(native_tar.extractfile(entry).read()).hexdigest() != digest:
                        raise ValueError('native baseline source differs: ' + file)
            for row in native_record['variants']:
                folder = native.parent / row['name']
                if (row['build']['exit_code'] != 0 or row['assertion']['exit_code'] != 101
                        or row.get('rejected_by_unchanged_assertion') is not True
                        or sha(folder / 'build.log') != row['build']['log_sha256']
                        or sha(folder / 'assertions.log') != row['assertion']['log_sha256']
                        or sha(folder / 'mutation.rs') != row['wrong_source_sha256']):
                    raise ValueError('original native mutation evidence missing or changed')
                if any(item['fresh'] is not False for item in row['compiler_artifacts']):
                    raise ValueError('native actual compiler artifact was stale')
            record['native_evidence'] = dict(record_sha256=sha(native),
                original_archive_sha256=sha(native_archive),
                baseline_rust_inputs=frozen['native_rust_inputs'])
            native_rows = {v['name']: v for v in native_record['variants']}
            mutations = load_module('stage1_native_mutations', HERE / 'negative_native.py').MUTATIONS
            for mutation in mutations:
                name = mutation['name']
                if name not in wrong:
                    continue
                folder = output / name
                folder.mkdir(mode=0o700)
                project = folder / 'source'
                project.mkdir(mode=0o700)
                with tarfile.open(archive) as tar:
                    tar.extractall(project, filter='data')
                check_inputs(project)
                (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
                (project / '.lake').mkdir()
                (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
                source = project / mutation['source']
                text = source.read_text()
                if text.count(mutation['old']) != 1 or sha(source) != native_rows[name]['original_source_sha256']:
                    raise ValueError('original Rust differs from native case')
                source.write_text(text.replace(mutation['old'], mutation['new'], 1))
                if sha(source) != native_rows[name]['wrong_source_sha256']:
                    raise ValueError('wrong Rust differs from actually tested native source')
                if (project / PROOFS / 'Model.lean').read_bytes() != (ROOT / PROOFS / 'Model.lean').read_bytes():
                    raise ValueError('wrong case changed the independent property')
                run(project, folder, 'extract', [sys.executable, 'verification/experiments/stage1-protocol/extract.py',
                    '--profile', mutation['profile'], '--output', str(folder / 'extraction')])
                profile = mutation['profile']
                generated = folder / 'extraction' / profile / 'generated'
                for path in generated.rglob('*'):
                    if path.is_file() and '_Template' not in path.name:
                        target = project / LIBRARY / profile / path.relative_to(generated)
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(path, target)
                shutil.copy2(folder / 'extraction' / profile / 'original.llbc',
                             project / LIBRARY / profile / 'original.llbc')
                proof_adaptation = None
                if name == 'skip-capture-binding':
                    # The real mutant duplicates the suffix into loop0/loop1.
                    # Binding mismatch is the true branch and actually uses loop0.
                    # Adapt proof references only; never edit generated functions or Model.
                    witness = project / PROOFS / 'ManifestWitnesses.lean'
                    original_proof = witness.read_text()
                    before = sha(witness)
                    old_loop = 'sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop'
                    new_loop = old_loop + '0'
                    raw = (project / LIBRARY / 'manifest/SandproofStage1Manifest/Funs.lean').read_text()
                    if (original_proof.count(old_loop) != 6 or
                            ('def ' + new_loop + '.body\n') not in raw or
                            ('def ' + new_loop + '\n') not in raw):
                        raise ValueError('unexpected real mutant loop declarations/proof references')
                    (folder / 'original-witness.lean').write_bytes(witness.read_bytes())
                    witness.write_text(original_proof.replace(old_loop, new_loop))
                    proof_adaptation = dict(path=str(PROOFS / 'ManifestWitnesses.lean'),
                        before_sha256=before, after_sha256=sha(witness),
                        original_reference=old_loop, actual_reference=new_loop, replacements=6,
                        scope='proof helper references for actual binding-mismatch branch only')
                proof, imports, count = NEGATIVE[name]
                relative = 'artifacts/stage1-protocol/' + proof + '.lean'
                (project / relative).parent.mkdir(parents=True, exist_ok=True)
                (project / relative).write_bytes((HERE / (proof + '.lean.in')).read_bytes())
                targets = ['SandboxModel.Lifecycle.Protocol.' + n for n in imports]
                targets += ['SandproofStage1' + profile.title()]
                run(project, folder, 'build', ['lake', 'build', *targets])
                files = [str(PROOFS / (n + '.lean')) for n in imports]
                files += generated_files(profile)
                files += [relative]
                actual = actual_files(project, files, folder, env, run)
                if len(actual['audits']) != count:
                    raise ValueError('wrong-case exact audit inventory differs')
                record['cases'][name] = dict(complete=True, rust_sha256=sha(source),
                    model_sha256=sha(project / PROOFS / 'Model.lean'),
                    proof_adaptation=proof_adaptation, **actual)
                (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        check_inputs(ROOT)
        record['complete'] = True
        record['all_cases_complete'] = set(selected) == {'correct', *NEGATIVE}
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--source-archive', type=Path)
    parser.add_argument('--native-mutants', type=Path)
    parser.add_argument('--native-source-archive', type=Path)
    parser.add_argument('--case', choices=['all', 'correct', *NEGATIVE], default='all')
    args = parser.parse_args()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('fixed isolated formal Linux runtime required')
    os.umask(0o077)
    selected = ['correct', *NEGATIVE] if args.case == 'all' else [args.case]
    result = replay(args.output.absolute(), args.source_archive, args.native_mutants, args.native_source_archive, selected)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_cases_complete', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
