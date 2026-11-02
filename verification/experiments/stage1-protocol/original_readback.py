"""Finite readback of the ten reviewed original V3 proof packages.

This does not execute a proof, create an original checkpoint, or discover a
cache entry. The selection and current source applicability are fixed. Original
results, commands, CPUs, revisions and diagnostic messages remain unchanged.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import tarfile
import time

ROOT = Path(__file__).resolve().parents[3]
HERE = Path('verification/experiments/stage1-protocol')
ENV = 'SANDPROOF_V3_ORIGINAL_EVIDENCE'
KIND = 'fixed-original-proof-readback'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def bytes_at(path):
    path = Path(path)
    if not path.is_absolute() or not path.is_file() or path.resolve() != path:
        raise ValueError('missing or linked finite original input: '+str(path))
    return path.read_bytes()


def read(path):
    return json.loads(bytes_at(path))


def safe(name):
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise ValueError('unsafe relative evidence path: '+name)
    return path


def fixed(root):
    value = read(root/HERE/'fixed-originals.json')
    if value.get('schema') != 1 or value.get('kind') != 'fixed-ten-package-original-proof-selection':
        raise ValueError('unknown finite original selection')
    return value


def job_name(job):
    return job if isinstance(job, str) else job['name']


def supported(root, job):
    return job_name(job) in fixed(root)['jobs']


def _write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    with path.open('xb') as stream:
        stream.write(data)


def prepare(bundle, output, root=ROOT):
    """Copy only selected raw bytes once; no extraction tools or tests run."""
    root, bundle, output = Path(root).resolve(), Path(bundle).resolve(), Path(output).resolve()
    spec = fixed(root)
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    retained, original_paths = {}, {}
    archives = {}
    try:
        for name, expected in spec['archives'].items():
            path = bundle/safe(name)
            if digest(bytes_at(path)) != expected:
                raise ValueError('original archive differs: '+name)
            archives[name] = tarfile.open(path)
        def member(archive, name):
            item = archives[archive].getmember(str(safe(name)))
            if not item.isfile():
                raise ValueError('original archive entry is not a regular file')
            return archives[archive].extractfile(item).read()
        def retain(category, data, expected):
            if digest(data) != expected:
                raise ValueError('original selected bytes changed')
            name = category+'/'+expected
            if name not in retained:
                _write(output/name, data)
                retained[name] = expected
            return name
        results = {}
        for name, row in spec['jobs'].items():
            results[name] = retain('results', member(row['archive'], row['member']), row['sha256'])
        logs = json.loads(member('root-original-logs.tar.gz', 'index.json'))
        for row in logs:
            name = retain('logs', member('root-original-logs.tar.gz', row['member']), row['sha256'])
            if row['path'] in original_paths:
                raise ValueError('duplicate original log path')
            original_paths[row['path']] = dict(file=name, sha256=row['sha256'],
                original_digest_recorded=row['original_digest_recorded'])
        for expected, recipe in spec['physical_sources'].items():
            if 'archive' in recipe:
                retain('raw', member(recipe['archive'], recipe['member']), expected)
        index = dict(kind='finite-original-proof-bytes', schema=1,
                     selection_sha256=digest(bytes_at(root/HERE/'fixed-originals.json')),
                     archives=spec['archives'], files=retained, results=results,
                     original_paths=original_paths, proofs_executed=False)
        _write(output/'index.json', (json.dumps(index, indent=2)+'\n').encode())
        return index
    finally:
        for archive in archives.values():
            archive.close()


def _inputs(root, job, spec, evidence):
    row = spec['jobs'][job]
    for name in row['inputs']:
        expected = spec['inputs'][name]
        path = root/safe(name)
        value = bytes_at(path)
        mode = '100755' if path.stat().st_mode & 0o111 else '100644'
        if digest(value) != expected['sha256'] or mode != expected['mode']:
            raise ValueError('reviewed current consumed input differs: '+name)
        evidence[str(path)] = expected['sha256']
    for name in ('fixed-originals.json', 'original_readback.py'):
        path = root/HERE/name
        evidence[str(path)] = digest(bytes_at(path))


def _source(root, stage, spec, expected, evidence):
    recipe = spec['physical_sources'][expected]
    if 'source' in recipe:
        path = root/safe(recipe['source'])
        value = bytes_at(path)
        evidence[str(path)] = digest(value)
    elif 'recipe' in recipe:
        path = root/safe(recipe['recipe'])
        recipe_data = bytes_at(path)
        rule = json.loads(recipe_data)
        evidence[str(path)] = digest(recipe_data)
        path = root/safe(rule['provider'])
        value = bytes_at(path)
        if digest(value) != rule['provider_sha256']:
            raise ValueError('original clone provider differs')
        evidence[str(path)] = digest(value)
        text = value.decode()
        for before, after in rule['replacements']:
            if before not in text:
                raise ValueError('original clone-view replacement absent')
            text = text.replace(before, after)
        value = text.encode()
    else:
        path = stage/'raw'/expected
        value = bytes_at(path)
        evidence[str(path)] = digest(value)
    if digest(value) != expected:
        raise ValueError('current physical proof bytes differ from original')
    return value


def _original(stage, index, expected, job, evidence):
    name = index['results'][job]
    path = stage/safe(name)
    value = bytes_at(path)
    if digest(value) != expected['sha256']:
        raise ValueError('original result bytes differ')
    evidence[str(path)] = expected['sha256']
    result = json.loads(value)
    if result.get('complete') is not True:
        raise ValueError('original proof is incomplete')
    identity = result.get('source_commit', result.get('source_head'))
    if identity != expected['original_commit'] or result.get('cpus') != expected['original_cpus']:
        raise ValueError('original proof execution identity differs')
    return result, path


def _cases(result, expected):
    if 'cases' in result:
        if list(result['cases']) != list(expected['views']):
            raise ValueError('original cases differ from the fixed complete selection')
        return result['cases']
    variant = result['variant']
    if list(expected['views']) != [variant]:
        raise ValueError('original single variant differs')
    return {variant: result}


def _commands(result, case, row):
    commands = row.get('commands', result.get('commands', []))
    if 'cases' in result and not row.get('commands'):
        commands = [c for c in commands if case in Path(c['log']).parts]
    if not commands or any(c.get('exit_code') != 0 for c in commands):
        raise ValueError('missing or failed original execution commands')
    return commands


def _fingerprint(result, row):
    inputs = dict(row.get('source_inputs', result['inputs']))
    inputs.update(row.get('installed', {}))
    inputs.update(row['actual']['files'])
    value = hashlib.sha256()
    # Match lean_session's Path-component ordering, not plain string ordering:
    # Foo/Child.lean and Foo.lean have a different relative order.
    for prefix in ('SandboxModel/', 'verification/aeneas/'):
        for name in sorted((n for n in inputs if n.startswith(prefix) and n.endswith('.lean')), key=Path):
            encoded = name.encode()
            value.update(len(encoded).to_bytes(8, 'big'))
            value.update(encoded)
            value.update(bytes.fromhex(inputs[name]))
    return value.hexdigest(), inputs


def _validate(root, job, stage, output, *, install):
    root, stage, output = map(lambda p: Path(p).resolve(), (root, stage, output))
    name = job_name(job)
    spec = fixed(root)
    expected = spec['jobs'][name]
    evidence = {}
    _inputs(root, name, spec, evidence)
    index = read(stage/'index.json')
    if (index.get('kind') != 'finite-original-proof-bytes' or index.get('schema') != 1
            or index.get('selection_sha256') != digest(bytes_at(root/HERE/'fixed-originals.json'))
            or index.get('archives') != spec['archives'] or index.get('proofs_executed') is not False):
        raise ValueError('different finite original evidence preparation')
    evidence[str(stage/'index.json')] = digest(bytes_at(stage/'index.json'))
    result, result_path = _original(stage, index, expected, name, evidence)
    spec_import = importlib.util.spec_from_file_location('finite_original_registered', root/HERE/'registered.py')
    registered = importlib.util.module_from_spec(spec_import)
    spec_import.loader.exec_module(registered)
    checker_path = root/HERE/'registered.py'
    evidence[str(checker_path)] = digest(bytes_at(checker_path))
    messages, views = [], []
    def original_log(path, expected_sha=None):
        item = index['original_paths'][str(path)]
        file = stage/safe(item['file'])
        value = bytes_at(file)
        got = digest(value)
        if got != item['sha256'] or (expected_sha is not None and got != expected_sha):
            raise ValueError('original command or service log changed')
        evidence[str(file)] = got
        return value
    def placed(path, value):
        if install:
            _write(path, value)
        elif bytes_at(path) != value:
            raise ValueError('materialized proof/log readback changed')
        evidence[str(path)] = digest(value)
    for case, row in _cases(result, expected).items():
        view = expected['views'][case]
        actual = row['actual']
        if (row.get('complete') is not True or actual['files'] != view['files']
                or actual['audits'] != view['audits']
                or actual['service_log_sha256'] != view['service_log_sha256']):
            raise ValueError('original actual-file inventory differs')
        if expected['package'] == 'native-readiness':
            if case != 'correct' and (
                    len(actual['audits']) != 3 or
                    'SandboxModel.Stage1Protocol.NativeReadiness.wrong_status_refutes_full_contract'
                    not in actual['audits']):
                raise ValueError('missing original native-readiness counterexample')
        elif row.get('property_rejected') is not (case != 'correct'):
            raise ValueError('original rejection result differs')
        folder = output/'views'/case
        project, physical = folder/'source', folder/'physical-files'
        for target, h in actual['files'].items():
            placed(project/safe(target), _source(root, stage, spec, h, evidence))
        commands = _commands(result, case, row)
        kernels = [c for c in commands if Path(c['log']).name.startswith('kernel-')]
        if len(kernels) != len(actual['files']):
            raise ValueError('missing original kernel commands')
        for command in commands:
            original_log(command['log'], command['log_sha256'])
        projection = []
        for i, (target, command) in enumerate(zip(actual['files'], kernels)):
            if (Path(command['log']).name != 'kernel-'+str(i)+'.log'
                    or command['argv'] != ['lake', 'env', 'lean', target]):
                raise ValueError('original kernel command order differs')
            placed(folder/('kernel-'+str(i)+'.log'), original_log(command['log'], command['log_sha256']))
            # This is a path projection for checking identical retained bytes,
            # not a new compilation command or a rewrite of the original JSON.
            projection.append(dict(command, cwd=str(project)))
        old_physical = Path(kernels[0]['log']).parent/'physical-files'
        for file in ('requests.jsonl', 'service.log'):
            placed(physical/file, original_log(old_physical/file,
                actual['service_log_sha256'] if file == 'service.log' else None))
        registered._saved_physical_view(actual, project, physical, projection)
        source_fingerprint, old_inputs = _fingerprint(result, row)
        revisions, checked_messages = [], []
        for line in bytes_at(physical/'service.log').decode().splitlines():
            try:
                response = json.loads(line)
            except ValueError:
                continue
            if response.get('request', {}).get('tool') != 'lean_diagnostic_messages':
                continue
            revision = response['revision']
            common = {k:v for k,v in revision.items() if k != 'file_sha256'}
            if (revision.get('sources_sha256') != source_fingerprint
                    or revision.get('toolchain') != bytes_at(root/'lean-toolchain').decode().strip()
                    or revision.get('manifest_sha256') != old_inputs['lake-manifest.json']):
                raise ValueError('original service source/toolchain identity differs')
            if expected['package'] != 'native-readiness' and revision.get('lakefile_sha256') != old_inputs['lakefile.toml']:
                raise ValueError('original materialized Lake configuration differs')
            revisions.append(common)
            diagnostic = response['result']
            for _ in range(3):
                if 'success' in diagnostic:
                    break
                diagnostic = diagnostic.get('structuredContent', diagnostic.get('result', {}))
            for item in diagnostic.get('items', []):
                message = str(item.get('message', item.get('data', '')))
                if re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies', message):
                    checked_messages.append(message)
        if not revisions or any(r != revisions[0] for r in revisions):
            raise ValueError('original service changed revision during the view')
        messages.extend(checked_messages)
        views.append(dict(case=case, actual=actual, original_revision=revisions[0],
                          original_project=kernels[0]['cwd'], materialized_project=str(project)))
    return result, result_path, evidence, ('\n'.join(messages)+'\n').encode(), views


def reader(root, job, directory, output):
    """Return the unchanged original object, consumed evidence, original messages."""
    raw, _, evidence_files, stdout, _ = _validate(root, job, directory, output, install=True)
    return raw, evidence_files, stdout


def original(result):
    value = bytes_at(result['original_result_path'])
    if digest(value) != result['original_result_sha256']:
        raise ValueError('changed original result')
    return json.loads(value)


def check(result, root=ROOT, job=None):
    if (result.get('kind') != KIND or result.get('complete') is not True
            or result.get('execution') != 'REUSE' or result.get('proofs_executed') is not False):
        raise ValueError('not a complete fixed original readback')
    if job is not None and result['job'] != job_name(job):
        raise ValueError('fixed readback belongs to a different job')
    raw, path, consumed, stdout, views = _validate(root, result['job'],
        result['original_directory'], result['output_directory'], install=False)
    if (result['original_result_path'] != str(path) or result['original_result_sha256'] != digest(bytes_at(path))
            or result['actuals'] != [v['actual'] for v in views] or result['views'] != views
            or result['evidence'] != consumed or result['stdout_sha256'] != digest(stdout)
            or result['original_commit'] != raw.get('source_commit', raw.get('source_head'))
            or result['original_cpus'] != raw.get('cpus')):
        raise ValueError('original proof identity or current readback changed')
    return True


def evidence(result, root=ROOT):
    check(result, root)
    return dict(result['evidence'])


def run(output, job, root=ROOT, directory=None):
    root, output = Path(root).resolve(), Path(output).resolve()
    directory = Path(directory or os.environ[ENV]).resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    started = time.monotonic()
    result = dict(kind=KIND, job=job_name(job), complete=False, execution='REUSE',
                  proofs_executed=False, current_batch_acceptance=False)
    try:
        raw, path, consumed, stdout, views = _validate(root, job, directory, output, install=True)
        result.update(complete=True, original_directory=str(directory), output_directory=str(output),
            original_result_path=str(path), original_result_sha256=digest(bytes_at(path)),
            original_commit=raw.get('source_commit', raw.get('source_head')), original_cpus=raw.get('cpus'),
            actuals=[v['actual'] for v in views], views=views, evidence=consumed,
            stdout_sha256=digest(stdout), original_global_fingerprints_reconstructed=True)
        print(stdout.decode(), end='', flush=True)
    except BaseException as error:
        result.update(complete=False, error=repr(error))
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        _write(output/'result.json', (json.dumps(result, indent=2)+'\n').encode())
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='mode', required=True)
    stage = sub.add_parser('prepare')
    stage.add_argument('--bundle', type=Path, required=True)
    stage.add_argument('--output', type=Path, required=True)
    execute = sub.add_parser('run')
    execute.add_argument('--job', required=True)
    execute.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.mode == 'prepare':
        prepare(args.bundle, args.output)
    else:
        run(args.output, args.job)
