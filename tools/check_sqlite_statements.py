"""Admit the actual SQLite UPDATE candidate, full original AST and proof records.

This never edits an LLBC or generated Lean file. The comparison canonicalizes
only Charon's unordered short-name map and the explicitly recorded output-file
argument; no body, field, Drop, source span or ordered array is discarded.
"""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

try:
    from tools.check_foundations_extraction import llbc_digest
    from tools.check_sqlite_extraction import read_json, surface, proof_sources_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest
    from check_sqlite_extraction import read_json, surface, proof_sources_digest

ASSETS = Path('verification/experiments/sqlite-statements')
LIBRARY = Path('verification/aeneas/sqlite-statements')
MODELS = Path('SandboxModel/Boundaries/SqliteStatements')
AUDIT = MODELS / 'Audit.lean'
ENVIRONMENT = Path('verification/sqlite-statements-environment.json')
NEGATIVE_PROOF = ASSETS / 'negative/Counterexample.lean'
NEGATIVE_VIEW_PROOF = Path('SandboxModel/SqliteStatementsNegative.lean')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def full_digest(data):
    data = copy.deepcopy(data)
    destination = data['translated']['options']['dest_file']
    if not isinstance(destination, str) or not destination.endswith('/original.llbc'):
        raise ValueError('unregistered original extraction destination')
    data['translated']['options']['dest_file'] = '<declared-invocation-destination>/original.llbc'
    return llbc_digest(data)


def audit_names(text):
    names = re.findall(r'^#audit_controller_proof ([A-Za-z0-9_.]+)$', text, re.M)
    if len(names) != len(set(names)):
        raise ValueError('duplicate theorem audit')
    return set(names)


def expected_audits(root, negative=False):
    if negative:
        expected = audit_names((root / NEGATIVE_PROOF).read_text())
        if len(expected) != 7:
            raise ValueError('wrong-count counterexample audit changed')
        return expected
    expected = set()
    for path in sorted((root / MODELS).glob('*.lean')):
        text = path.read_text()
        # Private helper lemmas stay dependencies of their public roots.
        declarations = re.findall(r'^theorem ([A-Za-z0-9_]+)', text, re.M)
        if declarations:
            namespaces = re.findall(r'^namespace ([A-Za-z0-9_.]+)$', text, re.M)
            if len(namespaces) != 1:
                raise ValueError('unreviewed theorem namespace: ' + str(path))
            expected.update(namespaces[0] + '.' + name for name in declarations)
    if len(expected) != 38 or audit_names((root / AUDIT).read_text()) != expected:
        raise ValueError('missing, additional or changed statement proof audit')
    return expected


def check_frozen(root):
    path = root / 'verification/sqlite-statements-inputs.sha256'
    listed = {}
    for line in path.read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  ([^\n]+)', line)
        if match is None:
            raise ValueError('malformed frozen input')
        digest, name = match.groups()
        item = Path(name)
        if name in listed or item.is_absolute() or '..' in item.parts:
            raise ValueError('duplicate or escaping frozen input')
        listed[name] = digest
        if sha(root / item) != digest:
            raise ValueError('frozen input changed: ' + name)
    required = {'tools/check_sqlite_statements.py', str(ENVIRONMENT), str(AUDIT),
                str(MODELS / 'Contract.lean'), str(NEGATIVE_PROOF),
                str(LIBRARY / 'SandproofSqliteStatements/Funs.lean'),
                str(LIBRARY / 'SandproofSqliteStatements/FunsExternal.lean'),
                str(ASSETS / 'negative/Funs.lean'), str(ASSETS / 'candidate.json'),
                str(ASSETS / 'candidate-core.patch'), str(ASSETS / 'proof.py'),
                'tools/lean_session.py', 'scripts/lean-mcp-server.sh',
                'SandboxModel/ProofAuditCommand.lean', 'lean-toolchain', 'lake-manifest.json'}
    required.update(read_json(root / ENVIRONMENT)['context_sources'])
    if not required <= set(listed):
        raise ValueError('frozen input closure omitted a required dependency')
    return listed


def check_sources(root):
    check_frozen(root)
    environment = read_json(root / ENVIRONMENT)
    profile = read_json(root / ASSETS / 'candidate.json')
    for name, digest in {**profile['source_files'], **environment['context_sources']}.items():
        if sha(root / name) != digest:
            raise ValueError('actual production source changed: ' + name)
    if sha(root / ASSETS / profile['patch']) != profile['patch_sha256']:
        raise ValueError('candidate patch changed')
    if sha(root / ASSETS / 'statement-core.rs') != profile['candidate_files']['controller-service/src/journal/statement_core.rs']:
        raise ValueError('candidate actual-call core changed')
    generated_paths = environment['generated_paths']
    for variant_name, variant in environment['variants'].items():
        for original_name, digest in variant['generated'].items():
            stored = generated_paths[original_name]
            if original_name.endswith('/Funs.lean') and variant_name == 'accept-zero-changes':
                stored = str(ASSETS / 'negative/Funs.lean')
            if sha(root / stored) != digest:
                raise ValueError('retained original generated bytes changed: ' + stored)
    if (root / 'SandboxModel/Boundaries/SqliteStatements.lean').read_text() != 'import SandboxModel.Boundaries.SqliteStatements.Audit\n':
        raise ValueError('unreviewed public root shim')
    # The source closure is frozen separately; the gate also checks declarations
    # rather than treating a matching file hash as a theorem audit.
    for path in [*(root / MODELS).glob('*.lean'), root / NEGATIVE_PROOF,
                 *(root / LIBRARY / 'SandproofSqliteStatements').glob('*.lean')]:
        if re.search(r'\b(sorry|admit|axiom)\b', path.read_text()):
            raise ValueError('placeholder or custom assumption: ' + str(path))
    expected_audits(root)
    expected_audits(root, True)
    return environment


def check_pair(root, correct, wrong):
    environment = check_sources(root)
    for variant, data in [('correct', correct), ('accept-zero-changes', wrong)]:
        frozen = environment['variants'][variant]
        if data.get('has_errors') is not False:
            raise ValueError('missing or erroneous actual Charon output')
        options = data['translated']['options']
        for flag in ('skip_borrowck', 'no_typecheck', 'no_normalize', 'no_reorder_decls'):
            if options.get(flag) is not False:
                raise ValueError('original compiler/AST check disabled: ' + flag)
        # The sealed tuple method is an input encoder inside the admitted
        # whole execute summary. It must never be used as a standalone raw
        # binding implementation by translated application code.
        def mentions_encoder(value):
            if isinstance(value, dict):
                return value.get('Fun') == 39 or any(mentions_encoder(v) for v in value.values())
            return isinstance(value, list) and any(mentions_encoder(v) for v in value)
        def direct_calls(value):
            if isinstance(value, dict):
                if 'Call' in value and mentions_encoder(value['Call']['call']['func']):
                    raise ValueError('raw tuple binding use is outside the admitted execute summary')
                for child in value.values():
                    direct_calls(child)
            elif isinstance(value, list):
                for child in value:
                    direct_calls(child)
        for function in data['translated']['fun_decls']:
            if function and isinstance(function['body'], dict):
                direct_calls(function['body'])
        if full_digest(data) != frozen['full_llbc_digest']:
            raise ValueError('complete actual AST or original compiler options changed')
        if surface(data) != frozen['surface']:
            raise ValueError('function/normal Drop/unwind Drop surface changed')
        embedded = {f['name']['Local']: hashlib.sha256(f['contents'].encode()).hexdigest()
                    for f in data['translated']['files'] if f and f.get('contents') is not None}
        if embedded != frozen['embedded_sources']:
            raise ValueError('embedded original candidate source changed')
    inventory = read_json(root / LIBRARY / 'translation.json')
    if inventory != environment['translation']:
        raise ValueError('full generated function/type/trait inventory changed')
    group = environment['interface_group']
    if group['boundary'] != 'rusqlite.Connection.execute' or group['encoder_only_functions'] != ['TupleABCDE.Insts.RusqliteParamsParams.__bind_in']:
        raise ValueError('unreviewed opaque execute summary group')
    opaque = {f['lean_name'] for f in inventory['functions'] if f['is_opaque']}
    if opaque != set(environment['interfaces']):
        raise ValueError('unregistered opaque interface')
    for name, contract in environment['interfaces'].items():
        if not all(contract.get(k) for k in ('provider', 'input_domain', 'object', 'normal_return',
                                             'first_failure', 'effects', 'unwind_crash', 'caller')):
            raise ValueError('incomplete interface guarantee: ' + name)
        text = (root / LIBRARY / 'SandproofSqliteStatements/FunsExternal.lean').read_text()
        if not re.search(r'\bdef\s+' + re.escape(name) + r'(?![\w.])', text):
            raise ValueError('opaque model missing: ' + name)
    return environment


def check_extraction(root, path, variant):
    result = read_json(path / 'result.json')
    environment = read_json(root / ENVIRONMENT)
    if (result.get('variant') != variant or result.get('success') is not True
            or result.get('sources_unchanged') is not True
            or result.get('charon_exit_code') != 0 or result.get('aeneas_exit_code') != 0
            or result.get('has_errors') is not False
            or result.get('charon_timed_out') or result.get('aeneas_timed_out')):
        raise ValueError('actual extraction failed, incomplete or changed sources')
    if result['tools'] != environment['tools'] or result['tool_binaries_sha256'] != environment['tool_binaries_sha256']:
        raise ValueError('original tool input changed')
    generated = path / 'generated'
    actual = {str(p.relative_to(generated)): sha(p) for p in generated.rglob('*') if p.is_file()}
    if actual != result['generated'] or actual != environment['variants'][variant]['generated']:
        raise ValueError('incomplete or changed original generated files, including templates')
    llbc = read_json(path / 'original.llbc')
    if sha(path / 'original.llbc') != result['llbc_sha256']:
        raise ValueError('raw original LLBC evidence damaged')
    command = result['charon_command']
    if command[command.index('--dest-file') + 1] != llbc['translated']['options']['dest_file']:
        raise ValueError('LLBC output argument differs from original command')
    if full_digest(llbc) != environment['variants'][variant]['full_llbc_digest']:
        raise ValueError('original regenerated complete AST differs')
    return llbc


def check_tools(root, tool_root):
    env = read_json(root / ENVIRONMENT)
    for name, revision in env['tools'].items():
        actual = subprocess.check_output(['git', '-C', str(tool_root / name), 'rev-parse', 'HEAD'], text=True).strip()
        dirty = subprocess.check_output(['git', '-C', str(tool_root / name), 'status', '--porcelain', '--untracked-files=no'], text=True)
        if actual != revision or dirty:
            raise ValueError('original tool revision/tracked code changed: ' + name)
    for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
        if sha(tool_root / relative) != env['tool_binaries_sha256'][name]:
            raise ValueError('original binary changed: ' + name)
    backend = root / '.lake/packages/aeneas'
    if subprocess.check_output(['git', '-C', str(backend), 'rev-parse', 'HEAD'], text=True).strip() != env['tools']['aeneas']:
        raise ValueError('original Lean backend revision changed')
    if subprocess.check_output(['git', '-C', str(backend), 'status', '--porcelain', '--untracked-files=no'], text=True):
        raise ValueError('original Lean backend tracked source changed')


def check_libraries(root, tool_root):
    env = read_json(root / ENVIRONMENT)
    charon = tool_root / 'aeneas/charon/bin/charon'
    roots = {'production': Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()),
             'extraction': Path(subprocess.check_output([str(charon), 'toolchain-path'], text=True).strip())}
    for role, base in roots.items():
        for name, digest in env['library_sources'][role].items():
            if sha(base / 'lib/rustlib/src/rust/library' / name) != digest:
                raise ValueError('fixed standard-library source changed: ' + role + '/' + name)
    registry = Path(os.environ['CARGO_HOME']) / 'registry/src'
    for name, digest in env['registry_sources'].items():
        matches = list(registry.glob('*/' + name))
        if len(matches) != 1 or sha(matches[0]) != digest:
            raise ValueError('fixed SQLite/rusqlite source changed: ' + name)
    version = subprocess.check_output(['rustc', '-vV'], text=True)
    if version != env['production_rustc']:
        raise ValueError('production compiler differs from the checked native version')
    extraction = subprocess.check_output([str(charon), 'toolchain-version'], text=True).strip()
    if extraction != env['extraction_toolchain']:
        raise ValueError('original extraction compiler changed')


def check_record(view, record, expected, name, versions):
    path = view / name
    request = record.get('request', {})
    if (request != dict(tool='lean_diagnostic_messages', arguments={'file_path': str(name)}, expected_sha256=sha(path))
            or record.get('stale') is not False):
        raise ValueError('unfiltered physical file request for exact source required')
    envelope = record['result']
    if set(envelope) != {'result'}:
        raise ValueError('service error or unexpected envelope')
    result = envelope['result']
    revision = record['revision']
    if (result.get('success') is not True or result.get('partial') is not False
            or result.get('timed_out') is not False
            or result.get('failed_dependencies') not in (None, [])
            or result.get('still_elaborating_lines') not in (None, [])
            or revision.get('file_sha256') != sha(path)
            or revision.get('sources_sha256') != proof_sources_digest(view)
            or revision.get('toolchain') != (view / 'lean-toolchain').read_text().strip()
            or revision.get('lakefile_sha256') != sha(view / 'lakefile.toml')
            or revision.get('manifest_sha256') != sha(view / 'lake-manifest.json')
            or any(revision.get(k) != value for k, value in versions.items())):
        raise ValueError('incomplete/stale/different-version physical proof record')
    items = result['items']
    if any(x.get('severity') in ('warning', 'error') or 'unsolvedGoals' in (x.get('lean_tags') or []) for x in items):
        raise ValueError('unresolved physical proof diagnostic')
    found = [m.group(1) for x in items
             if (m := re.fullmatch(r'Checked (\S+): [0-3] standard logical dependencies', x.get('message', '')))]
    if len(found) != len(expected) or set(found) != expected:
        raise ValueError('missing, duplicate or extra physical theorem audit')
    return len(expected)


def check_service(root, view, path, negative=False):
    name = NEGATIVE_VIEW_PROOF if negative else AUDIT
    records = [json.loads(s) for s in path.read_text().splitlines() if s.startswith('{')]
    records = [r for r in records if r.get('request', {}).get('tool') == 'lean_diagnostic_messages'
               and r['request'].get('arguments', {}).get('file_path') == str(name)]
    if len(records) != 1:
        raise ValueError('one exact physical service record required')
    if negative:
        if sha(view / name) != sha(root / NEGATIVE_PROOF):
            raise ValueError('wrong counterexample proof copied to view')
    elif sha(view / name) != sha(root / AUDIT):
        raise ValueError('wrong positive audit copied to view')
    return check_record(view, records[0], expected_audits(root, negative), name,
                        read_json(root / ENVIRONMENT)['service_versions'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--correct', type=Path)
    parser.add_argument('--wrong', type=Path)
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--view', type=Path)
    parser.add_argument('--service-log', type=Path)
    parser.add_argument('--negative', action='store_true')
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    correct = read_json(root / 'verification/aeneas/llbc/sandproof_sqlite_statements.llbc')
    wrong = read_json(root / 'verification/aeneas/llbc/sandproof_sqlite_statements_wrong_count.llbc')
    if args.correct:
        correct = check_extraction(root, args.correct, 'correct')
    if args.wrong:
        wrong = check_extraction(root, args.wrong, 'accept-zero-changes')
    check_pair(root, correct, wrong)
    if args.tool_root:
        check_tools(root, args.tool_root)
        check_libraries(root, args.tool_root)
    if bool(args.view) != bool(args.service_log):
        raise ValueError('physical view and log must be supplied together')
    count = check_service(root, args.view, args.service_log, args.negative) if args.view else 0
    print(json.dumps(dict(kind='sqlite-statements-admission-not-full-acceptance', admitted=True,
                          physical_audits=count, actual_regeneration=bool(args.correct and args.wrong))))


if __name__ == '__main__':
    main()
