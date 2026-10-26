"""Current original bounded calls, unchanged Writer policy and full trace contract.

Runs the existing strict actual-file checker; ning owns shared acceptance.
Concrete Serde/standard-call instance adoption remains an explicit obligation.
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
HERE = Path('verification/experiments/stage1-protocol/envelope-writer')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


prior = module('writer_budget', ROOT / HERE.parent / 'envelope-budget/formal.py')
base, sha, snapshot = prior.base, prior.sha, prior.snapshot
binding = module('writer_binding', ROOT / HERE / 'binding.py')
preparer = module('writer_preparer', ROOT / HERE / 'prepare.py')
canonical = module('writer_original_ir_digest', ROOT / 'tools/check_foundations_extraction.py').llbc_digest


def inventory(root):
    prior.inventory(root)
    data = json.loads((root / HERE / 'inventory.json').read_text())
    if (data.get('schema') != 1 or data.get('a6_instance_adopted') is not False
            or data.get('production_factorization_adopted') is not False
            or set(data['variants']) != {'correct', 'swallow-error'}):
        raise ValueError('unreviewed Writer boundary or variants')
    for name, expected in data['inputs'].items():
        if sha(root / name) != expected:
            raise ValueError('reviewed Writer input changed: ' + name)
    for variant, stems in [('correct', ['EnvelopeWriter', 'EnvelopeWriterWitnesses']),
                           ('swallow-error', ['EnvelopeWriterWrong', 'EnvelopeWriterWitnesses'])]:
        audits = [n for stem in stems for n in re.findall(r'^#audit_controller_proof (\S+)$',
            (root / HERE / (stem + '.lean.in')).read_text(), re.M)]
        if audits != data['variants'][variant]['audits'] or len(set(audits)) != len(audits):
            raise ValueError('incomplete/duplicate Writer audits: ' + variant)
    return data


def parent_generation(project, output, wrapper, command, record):
    """Reproduce the same current Request and three independent approved views."""
    budget = prior.inventory(ROOT)
    byte_data = prior.previous.inventory(ROOT)
    for name, argv in [
        ('map-extract', ['bash', str(HERE.parent / 'schema-leaves/extract-map.sh')]),
        ('json-extract', [sys.executable, str(HERE.parent / 'envelope-bytes/extract-json.py')]),
        ('budget-extract', [sys.executable, str(HERE.parent / 'envelope-budget/extract.py')])]:
        command(project, output, name, wrapper + argv)
    for name, expected in budget['generated_sha256'].items():
        if sha(project / name) != expected:
            raise ValueError('parent generated view changed: ' + name)
    json_raw = json.loads((project / 'artifacts/envelope-json-foundations.llbc').read_text())
    if canonical(json_raw) != byte_data['json_llbc_canonical_sha256']:
        raise ValueError('full parent JSON LLBC changed')
    if canonical(json.loads((project / 'artifacts/envelope-budget.llbc').read_text())) != budget['budget_llbc_canonical_sha256']:
        raise ValueError('original constants LLBC changed')
    command(project, output, 'request-extract', wrapper + [sys.executable,
        str(HERE.parent / 'extract.py'), '--profile', 'request', '--output', 'artifacts/bytes-request-extract'])
    fresh = json.loads((project / 'artifacts/bytes-request-extract/result.json').read_text())
    current = prior.schema.inventory(ROOT)['current_request_generation']
    row = fresh['profiles']['request']
    if (fresh.get('success') is not True or fresh.get('sources_unchanged') is not True
            or row['llbc_canonical_sha256'] != current['llbc']
            or row['generated_sha256'] != dict(current['generated'], **current['templates'])):
        raise ValueError('current protocol-4 Request generation changed')
    record['request_extraction'] = fresh
    record['byte_roots'] = {}
    for name in prior.binding.ROOTS:
        command(project, output, name + '-charon', wrapper + ['.tools/aeneas/charon/bin/charon',
            'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
            '--include', 'sandproof_protocol', '--start-from', 'crate::' + name,
            '--dest-file', '../../../../artifacts/bytes-' + name + '.llbc', '--',
            '--manifest-path', str(HERE.parent / 'envelope-bytes/Cargo.toml'), '--lib', '--locked', '--offline'])
        raw = json.loads((project / 'artifacts' / ('bytes-' + name + '.llbc')).read_text())
        actual = prior.binding.snapshot(raw, name, sha(project / HERE.parent / 'envelope-bytes/roots.rs.in'))
        registered = byte_data['byte_roots'][name]
        if prior.binding.digest(actual) != registered['snapshot_sha256'] or canonical(raw) != registered['llbc_canonical_sha256']:
            raise ValueError('concrete original byte/type/dictionary/unwind differs: ' + name)
        record['byte_roots'][name] = registered
    return json_raw


def run(output, variant, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='current-writer-complete-trace-contract', variant=variant, complete=False,
        full_acceptance=False, full_v3_proof=False, a6_instance_adopted=False,
        production_factorization_adopted=False, native_checks='delegated-to-ning', commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')
    def command(project, folder, name, argv):
        begin = time.monotonic(); log = folder / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if result.returncode: raise ValueError('candidate command failed: ' + str(log))
    try:
        data = inventory(ROOT)
        inputs = snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        prepared = prior.run(output / 'preparation', prepare_only=True)
        if prepared.get('prepared') is not True or prepared['inputs'] != inputs:
            raise ValueError('prior exact private preparation failed/changed')
        project = output / 'preparation/source'
        expected = dict(inputs)
        for part in ['schema-leaves', 'envelope-schema', 'envelope-bytes', 'envelope-budget']:
            expected.update(json.loads((ROOT / HERE.parent / part / 'inventory.json').read_text())['candidate_sources'])
        for part, stem in [('schema-leaves','RoleLeaves'),('envelope-schema','EnvelopeSchema'),
                           ('envelope-bytes','EnvelopeBytes'),('envelope-budget','EnvelopeBudget')]:
            expected['SandboxModel/Lifecycle/Protocol/' + stem + '.lean'] = sha(project / HERE.parent / part / (stem+'.lean.in'))
        command(project, output, 'writer-lake-patch', ['patch','-p1','--fuzz=0','--batch',
            '--input=' + str(ROOT / HERE / 'shared-candidate.patch')])
        expected['lakefile.toml'] = data['lake_candidate_sha256']
        if sha(project / 'lakefile.toml') != expected['lakefile.toml']:
            raise ValueError('incorrect active Lake view')
        target = 'EnvelopeWriter' if variant == 'correct' else 'EnvelopeWriterWrong'
        for stem in ['EnvelopeWriterContract', 'EnvelopeWriterWitnesses', target]:
            name = 'SandboxModel/Lifecycle/Protocol/' + stem + '.lean'
            shutil.copy2(ROOT / HERE / (stem+'.lean.in'), project / name)
            expected[name] = sha(project / name)
        if prepare_only:
            record.update(prepared=True, project=str(project))
        else:
            (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            for name, relative in [('charon','aeneas/charon/bin/charon'),('aeneas','aeneas/bin/aeneas')]:
                if sha(ROOT / '.tools' / relative) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed tool changed: ' + name)
            record['tools'] = {n: fixed[n] for n in ('charon','aeneas')}
            wrapper = ['bwrap','--ro-bind','/','/','--dev','/dev','--proc','/proc',
                '--tmpfs','/tmp','--tmpfs','/home','--ro-bind',str(Path.home()),str(Path.home()),
                '--bind',str(project),str(VIRTUAL),'--chdir',str(VIRTUAL),
                '--setenv','CARGO_TARGET_DIR',str(VIRTUAL / 'artifacts/extract-target'),
                '--setenv','CARGO_NET_OFFLINE','true','--setenv','CARGO_BUILD_JOBS','4',
                '--unsetenv','RUSTFLAGS','--unsetenv','CARGO_ENCODED_RUSTFLAGS','--']
            json_raw = parent_generation(project, output, wrapper, command, record)
            command(project, output, 'original-calls', wrapper + [sys.executable,str(HERE / 'extract.py'),
                '--mode','calls','--label','writer-original-calls-01'])
            raw = json.loads((project / 'artifacts/writer-original-calls-01.llbc').read_text())
            original = binding.snapshot(raw, sha(project / HERE / 'roots.rs.in'))
            if (canonical(raw) != data['original_calls']['llbc_canonical_sha256']
                    or binding.digest(original) != data['original_calls']['snapshot_sha256']):
                raise ValueError('original bounded calls/types/dictionaries/unwind changed')
            if binding.contract_error(raw) != binding.contract_error(json_raw):
                raise ValueError('ContractError alias is not the same complete original enum')
            record['original_calls'] = original
            command(project, output, 'binding-admission', [sys.executable, str(HERE / 'admission_tests.py'),
                '--llbc', 'artifacts/writer-original-calls-01.llbc'])
            transform = preparer.prepare(project, variant)
            record['source_transform'] = transform
            for row in transform['files']:
                expected[row['path']] = row['after_sha256']
            selected = data['variants'][variant]
            if {r['path']:r['after_sha256'] for r in transform['files']} != selected['candidate_sources']:
                raise ValueError('candidate source/fault differs')
            label = 'writer-current-05' if variant == 'correct' else 'swallow-error-01'
            command(project, output, 'writer-extract', wrapper + [sys.executable,str(HERE / 'extract.py'),
                '--mode','policy','--label',label])
            generated = {str(p.relative_to(project)):sha(p)
                for p in (project / 'verification/aeneas/stage1-protocol/envelope-writer').rglob('*') if p.is_file()}
            if generated != selected['generated_sha256']:
                raise ValueError('fresh Writer generated body differs')
            if canonical(json.loads((project / 'artifacts' / (label+'.llbc')).read_text())) != selected['llbc_canonical_sha256']:
                raise ValueError('full Writer LLBC differs')
            expected.update(generated)
            command(project, output, 'build', ['lake','build','SandboxModel.Lifecycle.Protocol.'+target,
                'SandboxModel.Boundaries.BoundedWriter.Audit'])
            seeds = ['SandboxModel/Lifecycle/Protocol/'+target+'.lean',
                     'SandboxModel/Boundaries/BoundedWriter/Audit.lean',
                     *[n for n in generated if n.endswith('.lean')]]
            files = prior.local_closure(project, seeds)
            original_checker = module('writer_actual_checker',ROOT / HERE.parent / 'formal.py')
            actual = original_checker.actual_files(project, files, output, env, command)
            declared = [n for f in files for n in re.findall(r'^#audit_controller_proof (\S+)$',(project/f).read_text(),re.M)]
            if sorted(actual['audits']) != sorted(declared) or not set(selected['audits']).issubset(actual['audits']):
                raise ValueError('incomplete actual-file audit')
            record.update(actual=actual, generated_sha256=generated, complete=True)
        for name, wanted in expected.items():
            if sha(project / name) != wanted: raise ValueError('source changed during checking: '+name)
        if inputs != snapshot(ROOT): raise ValueError('original source changed during checking')
    except BaseException as error:
        record.pop('prepared', None)
        record.update(complete=False,error=repr(error),traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--variant',choices=['correct','swallow-error'],default='correct')
    parser.add_argument('--prepare-only',action='store_true')
    args=parser.parse_args();os.umask(0o077)
    result=run(args.output.absolute(),args.variant,args.prepare_only)
    print(json.dumps({k:result.get(k) for k in ['variant','complete','prepared','seconds','error']}))
    raise SystemExit(0 if result.get('complete') or result.get('prepared') else 1)
