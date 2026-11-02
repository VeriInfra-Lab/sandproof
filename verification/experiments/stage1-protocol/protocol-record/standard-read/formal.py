"""The two actual StandardRecord reads, with the original capture and checker."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import time

ROOT = Path(__file__).resolve().parents[5]
HERE = Path('verification/experiments/stage1-protocol/protocol-record/standard-read')
PARENT = Path('verification/experiments/stage1-protocol')
ENTRY = 'standard-record-read'
CAPTURE_ENV = 'SANDPROOF_STANDARD_RECORD_CAPTURE_EVIDENCE'
REUSE_ENV = 'SANDPROOF_STANDARD_RECORD_REUSE_EVIDENCE'
PREPARED_ENV = 'SANDPROOF_STANDARD_RECORD_PREPARED_PROJECT'
CHECKED_FILES = (
    str(HERE/'generated/SandproofStandardRecord/TypesExternal.lean'),
    str(HERE/'generated/SandproofStandardRecord/Types.lean'),
    str(HERE/'generated/SandproofStandardRecord/FunsExternal.lean'),
    str(HERE/'generated/SandproofStandardRecord/Funs.lean'),
    'SandboxModel/Lifecycle/Protocol/StandardRecordCloningDelta.lean',
    'SandboxModel/Access/Cloning.lean',
    'SandboxModel/Lifecycle/Protocol/StandardRecordReads.lean',
)


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    path = Path(path)
    if not path.is_absolute() or not path.is_file() or path.resolve() != path:
        raise ValueError('missing or linked finite read input: '+str(path))
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    sha(path)
    return json.loads(Path(path).read_text())


def current(root):
    root = Path(root).resolve()
    view, capture = (read(root/HERE/name) for name in ('view.json', 'capture.json'))
    support = module('standard_read_fixed_support', root/HERE/'support.py')
    if (view.get('schema') != 1 or capture.get('schema') != 1
            or capture['roots'] != support.ROOTS or capture['include'] != support.INCLUDE
            or capture['opaque'] != support.OPAQUE
            or sha(root/HERE/'root.rs.in') != capture['root_sha256']):
        raise ValueError('actual StandardRecord scope changed')
    evidence, values = {}, {}
    for name, expected in capture['source_inputs'].items():
        path = root/name
        mode = '100755' if path.stat().st_mode & 0o111 else '100644'
        if sha(path) != expected or mode != capture['source_modes'][name]:
            raise ValueError('actual read source/mode changed: '+name)
        evidence[str(path)] = expected
    for name, row in view['files'].items():
        if Path(name).is_absolute() or '..' in Path(name).parts:
            raise ValueError('unsafe finite project path')
        path = root/row['source']
        if sha(path) != row['source_sha256']:
            raise ValueError('current read proof provider changed: '+str(path))
        evidence[str(path)] = row['source_sha256']
        content = path.read_bytes()
        for before, after in row.get('replace', []):
            if before.encode() not in content:
                raise ValueError('missing original namespace binding')
            content = content.replace(before.encode(), after.encode())
        if hashlib.sha256(content).hexdigest() != row['sha256']:
            raise ValueError('finite namespace view changed: '+name)
        values[name] = content
    for name in ('formal.py', 'view.json', 'capture.json', 'root.rs.in', 'support.py'):
        path = root/HERE/name
        evidence[str(path)] = sha(path)
    return view, capture, values, evidence


def plan(root):
    _, _, values, _ = current(root)
    files = list(CHECKED_FILES)
    audits = [audit for name in files for audit in re.findall(
        r'^#audit_controller_proof (\S+)$', values[name].decode(), re.M)]
    if len(audits) != len(set(audits)) or len(audits) != 9:
        raise ValueError('missing, duplicate or extra read declarations')
    return dict(views={'actual': dict(files=files, audits=audits)})


def capture_evidence(root, directory):
    _, fixed, _, evidence = current(root)
    directory = Path(directory)
    for name, expected in fixed['files'].items():
        path = directory/name
        if sha(path) != expected:
            raise ValueError('original read capture evidence changed: '+name)
        evidence[str(path)] = expected
    record = read(directory/'result.json')
    if (record.get('complete') is not True or record.get('capture_complete') is not True
            or record.get('translation_complete') is not True
            or record.get('generated_bodies_modified') is not False
            or record.get('source_postguard', {}).get('complete') is not True
            or record['target'] != fixed['target'] or record['tools'] != fixed['tools']
            or record['roots'] != fixed['roots'] or record['include'] != fixed['include']
            or record['opaque'] != fixed['opaque']
            or any(row.get('exit_code') != 0 for row in record['commands'])):
        raise ValueError('incomplete original two-method capture')
    return evidence


def install(root, project):
    _, _, values, _ = current(root)
    project = Path(project)
    project.mkdir(parents=True, exist_ok=False)
    for name, content in values.items():
        target = project/name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    (project/'artifacts').mkdir(exist_ok=True)
    return project


def check(result, root):
    if result.get('convergence_entry') != ENTRY or result.get('complete') is not True:
        raise ValueError('incomplete actual StandardRecord proof')
    if result.get('kind') == 'standard-record-proof-readback':
        original = Path(result['original_result'])
        if sha(original) != result['original_result_sha256']:
            raise ValueError('original complete read result changed')
        raw = read(original)
        if raw.get('kind') != 'actual-standard-record-read-proof':
            raise ValueError('readback must refer directly to an original execution')
        check(raw, root)
        if result.get('actuals') != raw['actuals'] or result.get('execution') != 'REUSE':
            raise ValueError('readback changed the original proof view')
        return True
    if result.get('kind') != 'actual-standard-record-read-proof' or result.get('execution') != 'RUN':
        raise ValueError('unknown StandardRecord proof producer')
    consumed = capture_evidence(root, result['capture_directory'])
    if consumed != result.get('source_evidence'):
        raise ValueError('current source or original capture no longer matches')
    view, _, _, _ = current(root)
    project = Path(result['project'])
    for name, row in view['files'].items():
        if sha(project/name) != row['sha256']:
            raise ValueError('actual materialized read input changed: '+name)
    recipe = plan(root)['views']['actual']
    actual, = result['actuals']
    if (set(actual['files']) != set(recipe['files'])
            or sorted(actual['audits']) != sorted(recipe['audits'])):
        raise ValueError('incomplete read physical file or dependency view')
    for command in result['commands']:
        if command['exit_code'] != 0 or sha(Path(command['log'])) != command['log_sha256']:
            raise ValueError('missing successful original read command')
    registered = module('original_read_service_validator', Path(root)/PARENT/'registered.py')
    registered._saved_physical_view(actual, project, Path(result['physical']), result['commands'])
    stdout = Path(result['physical']).parent/'actual-service.stdout'
    if (sha(stdout) != result['service_stdout_sha256']
            or sorted(re.findall(r'^Checked (\S+): \d+ standard logical dependencies$',
                                 stdout.read_text(), re.M)) != sorted(actual['audits'])):
        raise ValueError('original admitted read audit output changed')
    return True


def evidence(result, root):
    check(result, root)
    if result.get('kind') == 'standard-record-proof-readback':
        original = Path(result['original_result'])
        values = evidence(read(original), root)
        values[str(original)] = sha(original)
        return values
    values = dict(result['source_evidence'])
    view, _, _, _ = current(root)
    values.update({str(Path(result['project'])/name): row['sha256']
                   for name, row in view['files'].items()})
    for command in result['commands']:
        values[command['log']] = command['log_sha256']
    physical = Path(result['physical'])
    for name in ('requests.jsonl', 'service.log'):
        values[str(physical/name)] = sha(physical/name)
    values[str(physical.parent/'actual-service.stdout')] = result['service_stdout_sha256']
    return values


def run(output):
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    result = dict(kind='actual-standard-record-read-proof', convergence_entry=ENTRY,
                  complete=False, execution='RUN', commands=[], formal_batch_acceptance=False)
    try:
        reused = os.environ.get(REUSE_ENV)
        if reused:
            original = Path(reused)/'result.json'
            raw = read(original)
            check(raw, ROOT)
            result.update(kind='standard-record-proof-readback', execution='REUSE',
                          original_result=str(original), original_result_sha256=sha(original),
                          actuals=raw['actuals'], complete=True)
            check(result, ROOT)
            for line in Path(raw['physical']).parent.joinpath('actual-service.stdout').read_text().splitlines():
                if line.startswith('Checked '):
                    print(line, flush=True)
            return result
        if len(os.sched_getaffinity(0)) > 4:
            raise ValueError('the fixed read proof needs one admitted four-CPU slot')
        directory = os.environ.get(CAPTURE_ENV)
        if not directory:
            raise ValueError('missing original read capture: '+CAPTURE_ENV)
        source_evidence = capture_evidence(ROOT, directory)
        runtime = module('standard_read_original_runtime', ROOT/PARENT/'formal_runtime.py')
        prepared = os.environ.get(PREPARED_ENV)
        if prepared:
            project = Path(prepared)
            if not project.is_absolute() or project.resolve() != project:
                raise ValueError('prepared project must be an absolute private directory')
            view, _, _, _ = current(ROOT)
            for name, row in view['files'].items():
                if sha(project/name) != row['sha256']:
                    raise ValueError('prepared read project differs: '+name)
            if ((project/'.tools').resolve() != runtime.tool_root(ROOT).resolve()
                    or (project/'.lake/packages').resolve() != runtime.package_root(ROOT).resolve()):
                raise ValueError('prepared project has a different fixed tool or package binding')
        else:
            project = install(ROOT, output/'project')
            (project/'.tools').symlink_to(runtime.tool_root(ROOT))
            (project/'.lake').mkdir()
            (project/'.lake/packages').symlink_to(runtime.package_root(ROOT))
        result.update(capture_directory=directory, source_evidence=source_evidence,
                      project=str(project), actual_cpus=sorted(os.sched_getaffinity(0)))

        def command(where, folder, label, argv):
            begin = time.monotonic()
            log = folder/(label+'.log')
            with log.open('x') as stream:
                done = subprocess.run(argv, cwd=where, stdin=subprocess.DEVNULL,
                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
            result['commands'].append(dict(label=label, argv=argv, cwd=str(where),
                exit_code=done.returncode, log=str(log), log_sha256=sha(log),
                seconds=time.monotonic()-begin))
            (output/'progress.json').write_text(json.dumps(result,indent=2)+'\n')
            done.check_returncode()

        command(project, output, 'build', ['lake', 'build',
            'SandboxModel.Lifecycle.Protocol.StandardRecordReads'])
        checker = module('standard_read_original_checker', ROOT/PARENT/'formal.py')
        actual_directory = output/'actual-files'
        actual_directory.mkdir()
        actual = checker.actual_files(project, plan(ROOT)['views']['actual']['files'],
                                      actual_directory, os.environ.copy(), command)
        result.update(actuals=[actual], physical=str(actual_directory/'physical-files'),
                      service_stdout_sha256=sha(actual_directory/'actual-service.stdout'), complete=True)
        check(result, ROOT)
    except BaseException as error:
        result.update(complete=False, error=repr(error))
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(run(args.output), indent=2))
