"""Formal-host-only Status component; the official entry also requires sealed native evidence."""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from tools.check_foundations_extraction import llbc_digest
from tools.check_schema_domains import check_sources, check_guards, check_catalogue, load_environment
from tools.check_schema_domains_binding import check_byte_actual, check_byte_registry, check_library, check_features

GENERATED = 'verification/aeneas/schema-domains'
LLBC = 'verification/aeneas/llbc/sandproof_schema_domains.llbc'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args, root, log, env):
    with log.open('xb') as stream:
        code = subprocess.run(args, cwd=root, env=env, stdout=stream,
                              stderr=subprocess.STDOUT, timeout=1200).returncode
    if code:
        raise ValueError('formal command failed: ' + repr(args) + '; log=' + str(log))
    return {'command': args, 'exit_code': code, 'log_sha256': sha(log)}


def diagnostic(project, relative, output, env):
    source = project / relative
    req = {'tool': 'lean_diagnostic_messages', 'arguments': {'file_path': relative},
           'expected_sha256': sha(source)}
    text = json.dumps(req) + '\n{"tool":"exit"}\n'
    output.with_suffix('.requests.jsonl').write_text(text)
    log = output.with_suffix('.log')
    with log.open('xb') as stream:
        completed = subprocess.run([str(project / '.tools/lean-mcp/bin/python'),
            'tools/lean_session.py'], cwd=project, env=env, input=text, text=True,
            stdout=stream, stderr=subprocess.STDOUT, timeout=600)
    if completed.returncode:
        raise ValueError('original service client failed: ' + str(log))
    rows = []
    for line in log.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            rows.append(row)
    if len(rows) != 1:
        raise ValueError('missing/duplicate actual-file result')
    row = rows[0]
    if (row['request'] != req or row['stale'] is not False
            or row['revision']['file_sha256'] != req['expected_sha256']):
        raise ValueError('stale actual-file result')
    result = row['result']
    for _ in range(3):
        if 'success' in result:
            break
        result = result.get('structuredContent', result.get('result', {}))
    if (result.get('success') is not True or result.get('partial') is not False
            or result.get('timed_out') is not False or result.get('failed_dependencies')
            or result.get('still_elaborating_lines') not in (None, [])):
        raise ValueError('incomplete actual-file result: ' + repr(result))
    items = result.get('items', [])
    if any(item.get('severity') in ('error', 'warning') for item in items):
        raise ValueError('unresolved actual-file diagnostics: ' + repr(items))
    expected = re.findall(r'^#audit_controller_proof (\S+)', source.read_text(), re.M)
    observed = [match[1] for item in items if (match := re.fullmatch(
        r'Checked (\S+): \d+ standard logical dependencies', item.get('message', item.get('data', ''))))]
    if len(set(expected)) != len(expected) or sorted(expected) != sorted(observed):
        raise ValueError('audit targets missing, duplicated or extra')
    return {'file': relative, 'source_sha256': sha(source), 'audits': len(expected),
            'complete': True, 'log_sha256': sha(log)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[3])
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    root, out, tool = args.root.resolve(), args.output.resolve(), args.tool_root.resolve()
    out.mkdir(mode=0o700)
    os.umask(0o077)
    start = time.monotonic()
    record = {'kind': 'schema-domains-formal-component-not-full-project-acceptance', 'variants': {}}
    try:
        if subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=no'],
                                   cwd=root, text=True):
            raise ValueError('formal replay requires clean committed source')
        if not Path(os.environ['CARGO_TARGET_DIR']).resolve().is_dir():
            raise ValueError('provide the allocated stable formal Cargo target')
        env = dict(os.environ, SANDPROOF_PROOF_TOOL_ROOT=str(tool), CARGO_NET_OFFLINE='true',
                   CARGO_INCREMENTAL='0', LEAN_NUM_THREADS='2', LEAN_STACK_SIZE_KB='65536',
                   PYTHONDONTWRITEBYTECODE='1')
        registry = load_environment(root)
        check_sources(root, registry)
        check_catalogue(root)
        check_byte_registry(root)
        commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip()
        archive = subprocess.check_output(['git', 'archive', commit], cwd=root)
        record.update(commit=commit, archive_sha256=hashlib.sha256(archive).hexdigest())
        if not (root / '.lake/packages').is_dir():
            raise ValueError('fixed Lean packages must already be configured')
        spec = importlib.util.spec_from_file_location('schema_native',
            root / 'verification/experiments/schema-domains/native.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        if set(module.MUTATIONS) != {'node-id-only', 'no-upper-bound'}:
            raise ValueError('native/formal mutation inventory changed')
        model = (root / 'SandboxModel/Boundaries/SchemaDomains/Model.lean').read_bytes()
        for variant in ('correct', 'node-id-only', 'no-upper-bound'):
            started = time.monotonic()
            project = out / (variant + '-source')
            project.mkdir(mode=0o700)
            with tarfile.open(fileobj=io.BytesIO(archive)) as package:
                package.extractall(project, filter='data')
            (project / '.tools').symlink_to(tool, target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(root / '.lake/packages', target_is_directory=True)
            shutil.copyfile(project / 'lake-schema-domains.toml', project / 'lakefile.toml')
            if (root / '.lake/build').is_dir():
                local = sorted((root / 'SandboxModel').rglob('*.lean'))
                local += sorted((root / 'verification/aeneas').rglob('*.lean'))
                for path in local:
                    other = project / path.relative_to(root)
                    if not other.is_file() or sha(path) != sha(other):
                        raise ValueError('local import source drifted before cache reuse')
                def own_artifacts(folder, names):
                    return [name for name in names if name in ('SchemaDomains', 'SandproofSchemaDomains')
                            or name.startswith(('SchemaDomains.', 'SandproofSchemaDomains.'))]
                shutil.copytree(root / '.lake/build', project / '.lake/build', ignore=own_artifacts)
            status = project / 'sandbox-internal/src/status.rs'
            if sha(status) != registry['source_files']['sandbox-internal/src/status.rs']:
                raise ValueError('original production Status source drifted')
            if variant != 'correct':
                before, after, _, _ = module.MUTATIONS[variant]
                text = status.read_text()
                if text.count(before) != 1:
                    raise ValueError('actual Rust mutation no longer matches')
                status.write_text(text.replace(before, after))
                if sha(status) != registry['semantic_negatives'][variant]['rust_sha256']:
                    raise ValueError('native/formal mutant source differs')
            row = {'commands': [run(['bash', 'scripts/regenerate-schema-domains-lean.sh'], project,
                                   out / (variant + '-extraction.log'), env)]}
            llbc = json.loads((project / LLBC).read_text())
            inventory = json.loads((project / GENERATED / 'translation.json').read_text())
            if llbc.get('has_errors') is not False:
                raise ValueError('compile failure is not a counterexample')
            row.update(rust_sha256=sha(status), full_llbc_sha256=llbc_digest(llbc),
                       generated_sha256={n: sha(project / GENERATED / n) for n in
                         ('SandproofSchemaDomains/Types.lean', 'SandproofSchemaDomains/Funs.lean',
                          'translation.json')})
            if (project / 'SandboxModel/Boundaries/SchemaDomains/Model.lean').read_bytes() != model:
                raise ValueError('mutant changed the independent property')
            if variant == 'correct':
                check_guards(project, llbc, inventory, registry)
                row['commands'].append(run(['bash', 'scripts/regenerate-schema-domains-binding.sh'],
                    project, out / 'byte-extraction.log', env))
                check_byte_actual(project, project / 'artifacts/schema-domains-binding')
                libraries = list((Path(env['CARGO_HOME']) / 'registry/src').glob('*/serde_json-1.0.142'))
                if len(libraries) != 1:
                    raise ValueError('fixed JSON registry missing or ambiguous')
                check_library(libraries[0].parent, registry['byte_binding'])
                check_features(project, registry['byte_binding'])
                targets = ['SandboxModel.Boundaries.SchemaDomains',
                           'SandboxModel.Boundaries.SchemaDomains.Audit']
                files = ['SandboxModel/Boundaries/SchemaDomains.lean'] + [
                    'SandboxModel/Boundaries/SchemaDomains/' + n + '.lean'
                    for n in ('Model', 'Guard', 'Bytes', 'Audit')] + [
                    GENERATED + '/SandproofSchemaDomains/' + n + '.lean'
                    for n in ('Types', 'Funs', 'FunsExternal')]
                count = 46
            else:
                if row['generated_sha256'] != registry['semantic_negatives'][variant]['generated_sha256']:
                    raise ValueError('mutant generated output drifted; preserve and review')
                targets = ['SandboxModel.Boundaries.SchemaDomains.Model',
                           'SandproofSchemaDomains.Funs', 'SandboxModel.ProofAuditCommand']
                files = ['verification/experiments/schema-domains/' + (
                    'NodeOnly.lean' if variant == 'node-id-only' else 'NoUpperBound.lean')]
                count = 2
            if sum(len(re.findall(r'^#audit_controller_proof (\S+)', (project / n).read_text(), re.M))
                   for n in files) != count:
                raise ValueError('proof audit inventory changed')
            row['commands'].append(run(['lake', 'build', *targets], project,
                                       out / (variant + '-build.log'), env))
            row['physical_files'] = []
            for index, name in enumerate(files):
                stem = out / (variant + '-file-' + str(index))
                row['commands'].append(run(['lake', 'env', 'lean', name], project,
                                           stem.with_suffix('.kernel.log'), env))
                row['physical_files'].append(diagnostic(project, name, stem, env))
            row.update(complete=True, seconds=time.monotonic() - started)
            record['variants'][variant] = row
            (out / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
        record.update(exit_code=0, complete=True)
    except BaseException as error:
        record.update(exit_code=1, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    if record.get('complete') is True:
        (out / 'complete.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps(record))
    raise SystemExit(record['exit_code'])


if __name__ == '__main__':
    main()
