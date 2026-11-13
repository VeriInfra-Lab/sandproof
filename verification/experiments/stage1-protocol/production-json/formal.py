"""Read back the fixed JSON proof, keeping its original execution identity.

This is one finite convergence adapter, not a new proof executor or a cache.
Changed proof inputs or missing original evidence are rejected for new review.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import time

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/production-json')
ENV = 'SANDPROOF_PRODUCTION_JSON_ORIGINAL_EVIDENCE'
ENTRY = 'production-json'


def sha(value):
    return hashlib.sha256(value).hexdigest()


def file_bytes(path):
    path = Path(path)
    if not path.is_absolute() or not path.is_file() or path.resolve() != path:
        raise ValueError('expected an absolute regular non-link input: '+str(path))
    return path.read_bytes()


def load(root, name):
    return json.loads(file_bytes(root/HERE/name))


def current(root):
    root = Path(root).resolve()
    view, interface, fixed = (load(root, name) for name in
                              ('view.json', 'interface.json', 'readback.json'))
    if (view.get('schema') != 1 or fixed.get('schema') != 1
            or fixed.get('kind') != 'fixed-production-json-original-evidence'):
        raise ValueError('unknown finite JSON view')
    evidence = {}

    def checked(name, expected, mode=None):
        relative = Path(name)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('unsafe JSON source path')
        path = root/relative
        data = file_bytes(path)
        if sha(data) != expected:
            raise ValueError('current JSON input differs: '+name)
        if mode is not None and ('100755' if path.stat().st_mode & 0o111 else '100644') != mode:
            raise ValueError('current JSON file mode differs: '+name)
        evidence[str(path)] = expected
        return data

    for section in ('source_bindings', 'encoding_transform_inputs'):
        for name, expected in view[section].items():
            checked(name, expected)
    checked(view['raw_provenance']['source'], view['raw_provenance']['sha256'])
    installed, contents = {}, {}
    for name, row in view['files'].items():
        contents[name] = checked(row['source'], row['sha256'], row['mode'])
        installed[name] = row.get('installed_sha256', row['sha256'])
    # The service fingerprint includes every local Lean source, in this order.
    # This pins the imports too, not just the ten explicitly checked files.
    fingerprint = hashlib.sha256()
    lean_files = sorted(n for n in installed if n.startswith('SandboxModel/') and n.endswith('.lean'))
    lean_files += sorted(n for n in installed if n.startswith('verification/aeneas/') and n.endswith('.lean'))
    for name in lean_files:
        encoded = name.encode()
        fingerprint.update(len(encoded).to_bytes(8, 'big'))
        fingerprint.update(encoded)
        fingerprint.update(bytes.fromhex(installed[name]))
    if fingerprint.hexdigest() != fixed['revision']['sources_sha256']:
        raise ValueError('current complete JSON Lean input differs from original service')
    if (contents['lean-toolchain'].decode().strip() != fixed['revision']['toolchain']
            or installed['lake-manifest.json'] != fixed['revision']['manifest_sha256']
            or view['lake_view_mapping']['historical_config_sha256'] != fixed['revision']['lakefile_sha256']):
        raise ValueError('fixed JSON toolchain or consumed library context differs')
    files = fixed['physical_files']
    audits = [name for path in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', contents[path].decode(), re.M)]
    if (len(files) != 10 or len(set(files)) != len(files)
            or len(audits) != 30 or len(set(audits)) != len(audits)
            or audits[-9:] != interface['audits']):
        raise ValueError('fixed physical JSON files or declarations differ')
    for name in ('view.json', 'interface.json', 'readback.json', 'formal.py', 'materialize.py'):
        path = root/HERE/name
        evidence[str(path)] = sha(file_bytes(path))
    return view, fixed, installed, files, audits, evidence


def plan(root):
    _, _, _, files, audits, _ = current(root)
    return dict(views={'current-writer': dict(files=files, audits=audits)})


def original(root, directory):
    view, fixed, installed, files, audits, consumed = current(root)
    directory = Path(directory)
    if not directory.is_absolute() or directory.resolve() != directory:
        raise ValueError('original JSON evidence must be an absolute non-link directory')
    values = {}
    for name, expected in fixed['files'].items():
        path = directory/name
        value = file_bytes(path)
        if sha(value) != expected:
            raise ValueError('original JSON evidence changed: '+name)
        values[name] = value
        consumed[str(path)] = expected
    result = json.loads(values['result.json'])
    actual = result.get('actual', {})
    if (result.get('kind') != fixed['original_kind'] or result.get('complete') is not True
            or actual.get('files') != {name: installed[name] for name in files}
            or actual.get('audits') != audits
            or actual.get('service_log_sha256') != fixed['files']['actual-files/physical-files/service.log']):
        raise ValueError('original JSON proof result is incomplete or has a different view')
    expected_commands = [('build-writer', ['lake', 'build',
                                          'SandboxModel.Lifecycle.Protocol.EnvelopeWriter'])]
    expected_commands += [('kernel-'+str(i), ['lake', 'env', 'lean', name])
                          for i, name in enumerate(files)]
    commands = result.get('commands', [])
    if len(commands) != len(expected_commands) or any(
            row.get('label') != label or row.get('argv') != argv or row.get('exit_code') != 0
            for row, (label, argv) in zip(commands, expected_commands)):
        raise ValueError('original kernel/build completion differs')
    requests = [json.loads(line) for line in values['actual-files/physical-files/requests.jsonl'].decode().splitlines()]
    wanted = [dict(tool='lean_diagnostic_messages', arguments={'file_path': name},
                   expected_sha256=installed[name]) for name in files]
    if requests != wanted+[{'tool': 'exit'}]:
        raise ValueError('original service request list differs')
    rows = []
    for line in values['actual-files/physical-files/service.log'].decode().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            rows.append(row)
    if len(rows) != len(wanted):
        raise ValueError('missing original physical service result')
    observed = []
    for request, row in zip(wanted, rows):
        revision = dict(fixed['revision'], file_sha256=request['expected_sha256'])
        if row.get('request') != request or row.get('stale') is not False or row.get('revision') != revision:
            raise ValueError('original physical service identity differs')
        diagnostic = row.get('result', {})
        for _ in range(3):
            if 'success' in diagnostic:
                break
            diagnostic = diagnostic.get('structuredContent', diagnostic.get('result', {}))
        if (diagnostic.get('success') is not True or diagnostic.get('partial') is not False
                or diagnostic.get('timed_out') is not False or diagnostic.get('failed_dependencies')
                or diagnostic.get('still_elaborating_lines') not in (None, [])
                or any(item.get('severity') in ('warning', 'error') for item in diagnostic.get('items', []))):
            raise ValueError('original physical service has incomplete diagnostics')
        for item in diagnostic.get('items', []):
            message = str(item.get('message', item.get('data', '')))
            match = re.fullmatch(r'Checked ([\w.]+): \d+ standard logical dependencies', message)
            if match:
                observed.append(match[1])
    if sorted(observed) != sorted(audits):
        raise ValueError('original complete dependency audit differs')
    return result, consumed


def check(result, root):
    if (result.get('kind') != 'fixed-production-json-evidence-readback'
            or result.get('convergence_entry') != ENTRY or result.get('complete') is not True
            or result.get('execution') != 'REUSE' or result.get('proofs_rerun') is not False):
        raise ValueError('not a complete finite JSON readback')
    raw, consumed = original(root, result['original_directory'])
    if (result.get('actuals') != [raw['actual']] or result.get('evidence') != consumed
            or result.get('original_result_sha256') != consumed[
                str(Path(result['original_directory'])/'result.json')]
            or result.get('original_seconds') != raw['seconds']
            or result.get('original_actual_cpus') != raw['actual_cpus']):
        raise ValueError('original JSON proof identity or consumed evidence changed')
    project = Path(result['materialized_project'])
    _, _, installed, _, _, _ = current(root)
    for name, expected in installed.items():
        if sha(file_bytes(project/name)) != expected:
            raise ValueError('materialized current JSON input differs: '+name)
    return True


def evidence(result, root):
    check(result, root)
    consumed = dict(result['evidence'])
    _, _, installed, _, _, _ = current(root)
    consumed.update({str(Path(result['materialized_project'])/name): expected
                     for name, expected in installed.items()})
    return consumed


def run(output):
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    result = dict(kind='fixed-production-json-evidence-readback', convergence_entry=ENTRY,
                  complete=False, execution='REUSE', proofs_rerun=False,
                  formal_batch_acceptance=False)
    try:
        directory = os.environ.get(ENV)
        if not directory:
            raise ValueError('missing original JSON evidence: '+ENV)
        raw, consumed = original(ROOT, directory)
        spec = importlib.util.spec_from_file_location('finite_json_materialize', ROOT/HERE/'materialize.py')
        materialize = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(materialize)
        project = output/'project'
        materialize.install(project)
        result.update(original_directory=directory, materialized_project=str(project),
                      original_result_sha256=consumed[str(Path(directory)/'result.json')],
                      original_seconds=raw['seconds'], original_actual_cpus=raw['actual_cpus'],
                      actuals=[raw['actual']], evidence=consumed, complete=True)
        check(result, ROOT)
        # Forward the already admitted original messages, never synthesize a
        # fresh kernel/service success for this readback command.
        for line in file_bytes(Path(directory)/'actual-files/actual-service.stdout').decode().splitlines():
            if line.startswith('Checked '):
                print(line, flush=True)
    except BaseException as error:
        result.update(complete=False, error=repr(error))
        raise
    finally:
        result['seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(run(args.output), indent=2))
