"""Complete-field representation checks; no complete Serde/byte proof claim."""
from pathlib import Path
import argparse
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/envelope-schema')
PROOF = 'SandboxModel/Lifecycle/Protocol/EnvelopeSchema.lean'
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('envelope_source_helpers', ROOT / HERE.parent / 'schema-leaves/formal.py')
sha, snapshot = base.sha, base.snapshot


def inventory(root):
    data = json.loads((root / HERE / 'inventory.json').read_text())
    if (data.get('schema') != 1 or data.get('representation_only') is not True
            or data.get('a6_instance_adopted') is not False):
        raise ValueError('only the unadopted complete-field representation is supported')
    for name, digest in data['inputs'].items():
        if sha(root / name) != digest:
            raise ValueError('reviewed input changed: ' + name)
    proof = (root / HERE / 'EnvelopeSchema.lean.in').read_text()
    audits = re.findall(r'^#audit_controller_proof (\S+)$', proof, re.M)
    if audits != data['audits'] or len(audits) != 24 or len(audits) != len(set(audits)):
        raise ValueError('missing or duplicate complete-field audit')
    if set(data['structs']) != {'NodeIdentity', 'Resources', 'CreateBinding', 'OperationKey',
            'OperationBinding', 'LifecycleRequest', 'CenterIdentity', 'RetentionBinding',
            'AcceptedLifecycleIntent', 'RecordedLifecycleIntent'}:
        raise ValueError('missing actual schema structure')
    for name, row in data['structs'].items():
        source = (root / row['source']).read_text()
        match = re.search(r'#\[serde\(deny_unknown_fields\)\]\s*pub struct ' + name
                          + r'\s*\{([^}]+)\}', source)
        if match is None:
            raise ValueError('actual standard derive changed: ' + name)
        fields = re.findall(r'pub\s+(\w+)\s*:\s*([^,\n]+)', re.sub(r'//[^\n]*', '', match[1]))
        fields = [[key, re.sub(r'\s+', '', ty)] for key, ty in fields]
        if fields != row['fields_in_declaration_order']:
            raise ValueError('field type/order/coverage changed: ' + name)
    if set(data['operations']) != {'Create', 'Unload', 'Resume', 'Renew', 'Destroy'}:
        raise ValueError('missing actual operation branch')
    source = (root / 'sandbox-protocol/src/lifecycle/request.rs').read_text()
    for name, fields in data['operations'].items():
        match = re.search(r'\b' + name + r'\s*\{([^}]+|)\}', source)
        actual = re.findall(r'(\w+)\s*:\s*([^,\n]+)', match[1]) if match else None
        if actual is None or [[key, re.sub(r'\s+', '', ty)] for key, ty in actual] != fields:
            raise ValueError('actual operation fields changed: ' + name)
    return data


def run(output, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='protocol4-complete-field-representation', full_schema_proof=False,
                  full_json_proof=False, full_v3_proof=False, full_acceptance=False,
                  a6_instance_adopted=False, native_checks='delegated-to-ning', commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, name, argv, allow_failure=False):
        begin = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if result.returncode and not allow_failure:
            raise ValueError('candidate command failed: ' + str(log))
        return result.returncode

    try:
        data = inventory(ROOT)
        inputs = snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, digest in inputs.items():
            target = project / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
            if sha(target) != digest:
                raise ValueError('source changed while copying: ' + name)
        command(project, output, 'candidate-patch', ['patch', '-p1', '--fuzz=0', '--batch',
            '--input=' + str(ROOT / HERE / 'shared-candidate.patch')])
        expected = dict(inputs)
        for name, digest in data['candidate_sources'].items():
            if sha(project / name) != digest:
                raise ValueError('candidate patch differs: ' + name)
            expected[name] = digest
        (project / PROOF).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / HERE / 'EnvelopeSchema.lean.in', project / PROOF)
        expected[PROOF] = sha(project / PROOF)
        if prepare_only:
            record.update(prepared=True, complete=False)
        else:
            (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
                if sha(ROOT / '.tools' / relative) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed tool changed: ' + name)
            record['tools'] = {name: fixed[name] for name in ('charon', 'aeneas')}
            wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
            command(project, output, 'request-extract', wrapper + [sys.executable,
                str(HERE.parent / 'extract.py'), '--profile', 'request', '--output',
                'artifacts/envelope-request-extract'])
            extraction = project / 'artifacts/envelope-request-extract'
            fresh = json.loads((extraction / 'result.json').read_text())
            historical = json.loads((ROOT / HERE.parent / 'frozen.json').read_text())['generations']['request']
            frozen = data['current_request_generation']
            if fresh.get('success') is not True or fresh.get('sources_unchanged') is not True:
                raise ValueError('incomplete actual request extraction')
            row = fresh['profiles']['request']
            if (row['llbc_canonical_sha256'] != frozen['llbc']
                    or row['generated_sha256'] != dict(frozen['generated'], **frozen['templates'])):
                raise ValueError('current Rust does not match retained request generation')
            for name, digest in frozen['generated'].items():
                if sha(project / 'verification/aeneas/stage1-protocol/request' / name) != digest:
                    raise ValueError('retained actual generated file changed: ' + name)
            record['request_extraction'] = fresh
            record['historical_freeze'] = dict(unchanged=True,
                matches_current=historical == frozen,
                historical_llbc=historical['llbc'], current_llbc=frozen['llbc'],
                migration='PR #686: protocol constant 3 to 4; shared freeze remains pending')
            # These preflights record translation capability, never a property counterexample.
            record['serde_preflight'] = {}
            for kind in ('encode', 'decode'):
                raw = project / ('artifacts/envelope-' + kind + '.llbc')
                command(project, output, kind + '-charon', wrapper + [
                    '.tools/aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default', '--include', 'sandproof_protocol',
                    '--start-from', 'crate::accepted_' + kind, '--dest-file',
                    '../../../../artifacts/envelope-' + kind + '.llbc', '--', '--manifest-path',
                    str(HERE / 'Cargo.toml'), '--lib', '--locked', '--offline'])
                ir = json.loads(raw.read_text())
                if ir.get('has_errors') is not False:
                    raise ValueError('incomplete actual Serde preflight')
                code = command(project, output, kind + '-aeneas', wrapper + [
                    '.tools/aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error',
                    '-warnings-as-errors', '-split-files', '-emit-json', '-namespace',
                    'SandproofEnvelopeSchemaRust', '-subdir', 'SandproofEnvelopeSchema', '-dest',
                    'artifacts/envelope-' + kind + '-generated', 'artifacts/envelope-' + kind + '.llbc'],
                    allow_failure=True)
                record['serde_preflight'][kind] = dict(llbc_sha256=sha(raw),
                    aeneas_exit_code=code, translation_succeeded=code == 0, property_proved=False)
            command(project, output, 'build', ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.EnvelopeSchema'])
            formal = module('envelope_actual_formal', ROOT / HERE.parent / 'formal.py')
            resolver = module('envelope_closure', ROOT / 'tools/proof_batch.py')
            files = sorted(resolver.closure(project, [PROOF]))
            actual = formal.actual_files(project, files, output, env, command)
            if not set(data['audits']).issubset(actual['audits']):
                raise ValueError('missing representation dependency audits')
            record.update(actual=actual, complete=True)
        for name, digest in expected.items():
            if sha(project / name) != digest:
                raise ValueError('candidate/source file changed: ' + name)
        if inputs != snapshot(ROOT):
            raise ValueError('original inputs changed during candidate check')
    except BaseException as error:
        record.pop('prepared', None)
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.prepare_only)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'prepared', 'seconds', 'error']}))
    raise SystemExit(0 if result.get('complete') or result.get('prepared') else 1)
