"""Recheck original roots and strict physical files for parser error composition.

A6/B4 source binding remains conditional; this is not a full parser proof.
"""
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
HERE = Path('verification/experiments/stage1-protocol/parser-binding')
PROOF = 'SandboxModel/Lifecycle/Protocol/EnvelopeParser.lean'
TEMPLATE = HERE / 'EnvelopeParser.lean.in'
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('bytes_source_helpers', ROOT / HERE.parent / 'schema-leaves/formal.py')
schema = module('bytes_schema_helpers', ROOT / HERE.parent / 'envelope-schema/formal.py')
binding = module('parser_bytes_binding', ROOT / HERE.parent / 'envelope-bytes/binding.py')
parent = module('parser_parent', ROOT / HERE.parent / 'envelope-bytes/formal.py')
library = module('parser_library', ROOT / HERE / 'library.py')
dispatch = module('parser_dispatch', ROOT / HERE / 'binding.py')
canonical = module('parser_canonical', ROOT / 'tools/check_foundations_extraction.py').llbc_digest
sha, snapshot = base.sha, base.snapshot


def inventory(root):
    data = parent.inventory(root)
    audits = re.findall(r'^#audit_controller_proof (\S+)$', (root / TEMPLATE).read_text(), re.M)
    if len(audits) != 8 or len(set(audits)) != 8:
        raise ValueError('missing/duplicate parser composition audit')
    data['audits'] = data['audits'] + audits
    return data


def local_closure(project, seeds):
    # Load the reviewed candidate in the actual private project, never the
    # unchanged public helper from ROOT (which has no active-view parameter).
    resolver = module('bytes_original_closure', project / 'tools/proof_batch.py')
    return sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))


def run(output, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='protocol4-parser-error-composition',
        a6_instance_adopted=False, full_json_proof=False, full_v3_proof=False,
        active_lake_view='lakefile.toml', full_v0_proof=False,
        full_acceptance=False, native_checks='delegated-to-ning', commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, name, argv):
        begin = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if result.returncode:
            raise ValueError('candidate command failed: ' + str(log))

    try:
        data = inventory(ROOT)
        library_candidate = json.loads((ROOT / HERE / 'candidate.json').read_text())
        record['library_inputs'] = library.check(library_candidate, ROOT)
        inputs = snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, expected in inputs.items():
            destination = project / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
            if sha(destination) != expected:
                raise ValueError('source changed while copying: ' + name)
        expected = dict(inputs)
        expected.update(base.view.install(project, 'envelope-bytes'))
        for component, stem in [('schema-leaves', 'RoleLeaves'), ('envelope-schema', 'EnvelopeSchema'),
                                ('envelope-bytes', 'EnvelopeBytes'), ('parser-binding', 'EnvelopeParser')]:
            target = project / ('SandboxModel/Lifecycle/Protocol/' + stem + '.lean')
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / HERE.parent / component / (stem + '.lean.in'), target)
            expected[str(target.relative_to(project))] = sha(target)
        if prepare_only:
            record.update(prepared=True, complete=False)
        else:
            runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
            (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
                if sha(ROOT / '.tools' / relative) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed tool binary differs: ' + name)
            record['tools'] = {name: fixed[name] for name in ('charon', 'aeneas')}
            wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
                '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
            command(project, output, 'map-extract', wrapper + ['bash', str(HERE.parent / 'schema-leaves/extract-map.sh')])
            command(project, output, 'json-extract', wrapper + [sys.executable, str(HERE.parent / 'envelope-bytes/extract-json.py')])
            generated = {}
            for directory in ['verification/aeneas/stage1-protocol/json-foundations',
                              'verification/aeneas/stage1-protocol/schema-result']:
                for path in sorted((project / directory).rglob('*')):
                    if path.is_file():
                        if '_Template.lean' in path.name:
                            raise ValueError('unreviewed generated template')
                        generated[str(path.relative_to(project))] = sha(path)
            if generated != data['generated_sha256']:
                raise ValueError('fresh selected view differs from actual retained generation')
            canonical = module('bytes_original_ir_digest', ROOT / 'tools/check_foundations_extraction.py').llbc_digest
            record['json_extraction'] = json.loads((project / 'artifacts/envelope-json-extraction.json').read_text())
            if canonical(json.loads((project / 'artifacts/envelope-json-foundations.llbc').read_text())) != data['json_llbc_canonical_sha256']:
                raise ValueError('fresh JSON view full LLBC differs')
            command(project, output, 'request-extract', wrapper + [sys.executable,
                str(HERE.parent / 'extract.py'), '--profile', 'request', '--output', 'artifacts/bytes-request-extract'])
            fresh = json.loads((project / 'artifacts/bytes-request-extract/result.json').read_text())
            current = schema.inventory(ROOT)['current_request_generation']
            row = fresh['profiles']['request']
            if (fresh.get('success') is not True or fresh.get('sources_unchanged') is not True
                    or row['llbc_canonical_sha256'] != current['llbc']
                    or row['generated_sha256'] != dict(current['generated'], **current['templates'])):
                raise ValueError('current request complete generation differs')
            record['request_extraction'] = fresh
            record['byte_roots'] = {}
            for name in binding.ROOTS:
                command(project, output, name + '-charon', wrapper + ['.tools/aeneas/charon/bin/charon',
                    'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                    '--include', 'sandproof_protocol', '--start-from', 'crate::' + name,
                    '--dest-file', '../../../../artifacts/bytes-' + name + '.llbc', '--',
                    '--manifest-path', str(HERE.parent / 'envelope-bytes/Cargo.toml'), '--lib', '--locked', '--offline'])
                rawpath = project / 'artifacts' / ('bytes-' + name + '.llbc')
                raw = json.loads(rawpath.read_text())
                actual = binding.snapshot(raw, name, sha(project / HERE.parent / 'envelope-bytes/roots.rs.in'))
                registered = data['byte_roots'][name]
                if binding.digest(actual) != registered['snapshot_sha256'] or canonical(raw) != registered['llbc_canonical_sha256']:
                    raise ValueError('concrete byte AST/type/dictionary/unwind differs: ' + name)
                (output / (name + '-snapshot.json')).write_text(json.dumps(actual, indent=2)+'\n')
                record['byte_roots'][name] = dict(registered, llbc_sha256=sha(rawpath), parser_proved=False)
                if name.endswith('decode'):
                    report = dispatch.report(raw)
                    report_path = output / (name + '-dispatch.json')
                    report_path.write_text(json.dumps(report, indent=2)+'\n')
                    record['byte_roots'][name]['dispatch_sha256'] = sha(report_path)
            command(project, output, 'build', ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.EnvelopeParser'])
            original = module('bytes_actual_formal', ROOT / HERE.parent / 'formal.py')
            files = local_closure(project, [PROOF, *[n for n in generated if n.endswith('.lean')]])
            actual = original.actual_files(project, files, output, env, command)
            declared = [n for file in files for n in re.findall(r'^#audit_controller_proof (\S+)$',
                        (project / file).read_text(), re.M)]
            if sorted(actual['audits']) != sorted(declared) or not set(data['audits']).issubset(actual['audits']):
                raise ValueError('incomplete actual composition/import audit')
            record.update(actual=actual, generated_sha256=generated, complete=True)
        for name, wanted in expected.items():
            if sha(project / name) != wanted:
                raise ValueError('actual source changed during checking: ' + name)
        if record['library_inputs'] != library.check(library_candidate, ROOT):
            raise ValueError('actual parser libraries changed during checking')
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
