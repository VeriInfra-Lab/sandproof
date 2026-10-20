"""Admit the frozen Writer candidate and complete original IR; never edit it."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import shutil
import tarfile

try:
    from tools.check_foundations_extraction import llbc_digest
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest

ASSETS = Path('verification/experiments/bounded-writer')
LIBRARY = Path('verification/aeneas/bounded-writer')
MODELS = Path('SandboxModel/Boundaries/BoundedWriter')
ENVIRONMENT = Path('verification/bounded-writer-environment.json')
FROZEN = Path('verification/bounded-writer-inputs.sha256')
NEGATIVE_PROOF = ASSETS / 'negative/Counterexample.lean'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    return json.loads(path.read_text())


def relative_file(root, name):
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise ValueError('escaping input path')
    target = root / path
    if target.is_symlink() or not target.is_file():
        raise ValueError('missing or symlink input: ' + name)
    target.resolve(strict=True).relative_to(root.resolve(strict=True))
    return target


def check_map(root, expected):
    for name, expected_sha in expected.items():
        if sha(relative_file(root, name)) != expected_sha:
            raise ValueError('source changed: ' + name)


def check_frozen(root, variant=None):
    files = {}
    for line in (root / FROZEN).read_text().splitlines():
        match = re.fullmatch(r'([0-9a-f]{64})  (.+)', line)
        if match is None or match[2] in files:
            raise ValueError('malformed or duplicate frozen input')
        files[match[2]] = match[1]
    required = {str(ENVIRONMENT), str(MODELS / 'Model.lean'), str(MODELS / 'Contract.lean'),
                str(MODELS / 'Refinement.lean'), str(MODELS / 'Library.lean'),
                str(MODELS / 'Examples.lean'), str(MODELS / 'Audit.lean'),
                str(NEGATIVE_PROOF), str(ASSETS / 'prepare.py'), str(ASSETS / 'native.py'),
                str(ASSETS / 'translate.py'), str(ASSETS / 'proof.py'),
                str(ASSETS / 'replay.py'), str(ASSETS / 'runtime.sh'), str(ASSETS / 'native_artifacts.py'),
                str(ASSETS / 'inside.py'), str(ASSETS / 'inside-runtime.sh'),
                str(ASSETS / 'lease.py'), str(ASSETS / 'audit_messages.py'),
                str(ASSETS / 'candidate.json'), str(ASSETS / 'candidate-diff.json'),
                str(LIBRARY / 'SandproofBoundedWriter/Funs.lean'),
                str(LIBRARY / 'SandproofBoundedWriter/TypesExternal.lean'),
                'tools/check_bounded_writer.py', 'tools/tests/test_bounded_writer.py',
                'SandboxModel/ProofAuditCommand.lean', 'tools/lean_session.py',
                'scripts/lean-mcp-server.sh', 'lean-toolchain', 'lake-manifest.json'}
    if not required <= set(files):
        raise ValueError('frozen input closure missing required source')
    if variant is not None:
        if variant not in ('correct', 'no-limit'):
            raise ValueError('unknown prepared variant')
        replacements = read(root / ENVIRONMENT)['variants'][variant]['candidate_files']
        if not set(replacements) <= set(files):
            raise ValueError('candidate replacement omitted from source freeze')
        # Explicit reviewed source variants, not IR/path normalization.
        files.update(replacements)
        if variant == 'no-limit':
            generated = read(root / ENVIRONMENT)['variants'][variant]['generated']
            for name in ('SandproofBoundedWriter/Types.lean', 'SandproofBoundedWriter/Funs.lean', 'translation.json'):
                files[str(LIBRARY / name)] = generated[name]
    check_map(root, files)
    return files


def check_sources(root, variant=None):
    environment = read(root / ENVIRONMENT)
    originals = dict(environment['source_files'])
    if variant is not None:
        if variant not in ('correct', 'no-limit'):
            raise ValueError('unknown Writer variant')
        originals.update(environment['variants'][variant]['candidate_files'])
    check_map(root, originals)
    for package in environment['source_roots']:
        name = package + '/build.rs'
        if (root / name).exists() and name not in environment['source_files']:
            raise ValueError('unregistered project build script: ' + name)
    config = root / '.cargo'
    if config.exists():
        for path in config.rglob('*'):
            if path.is_file() and str(path.relative_to(root)) not in environment['source_files']:
                raise ValueError('unregistered Cargo configuration')
    check_map(root, environment['crate_files'])
    crate = root / 'verification/extract-bounded-writer'
    actual = {str(p.relative_to(root)) for p in crate.rglob('*') if p.is_file()
              and not any(x in ('target', '.git', '__pycache__') for x in p.relative_to(crate).parts)}
    if actual != set(environment['crate_files']):
        raise ValueError('unregistered extraction-crate input')
    required = ('provider', 'input_domain', 'object', 'normal_return', 'failure', 'effects', 'unwind_crash_drop', 'caller')
    if len(environment['interfaces']) != 10 or any(not all(row.get(k) for k in required)
        for row in environment['interfaces'].values()):
        raise ValueError('incomplete primitive/adapter guarantee')
    if environment['scope'] != 'ordinary-value-isolated-candidate-existing-A6-B4':
        raise ValueError('unreviewed semantic scope')
    if environment['runtime']['virtual_source'] != '/tmp/bounded-writer/source':
        raise ValueError('unreviewed virtual source layout')
    return environment


def check_crate_archive(root, archive):
    expected = read(root / ENVIRONMENT)['crate_files']
    actual = {}
    with tarfile.open(archive) as package:
        for member in package.getmembers():
            name = member.name
            path = Path(name)
            if path.is_absolute() or '..' in path.parts or not (member.isfile() or member.isdir()):
                raise ValueError('unsafe crate archive member')
            if member.isfile():
                if name not in expected or name in actual:
                    raise ValueError('unregistered/duplicate crate source')
                stream = package.extractfile(member)
                actual[name] = hashlib.sha256(stream.read()).hexdigest()
            elif name != 'verification/extract-bounded-writer' and not name.startswith('verification/extract-bounded-writer/'):
                raise ValueError('unregistered crate directory')
    if actual != expected:
        raise ValueError('crate archive does not contain the exact frozen sources')


def check_ir(root, llbc, variant):
    env = read(root / ENVIRONMENT)
    if llbc.get('has_errors') is not False:
        raise ValueError('Charon errors or incomplete IR')
    options = llbc['translated']['options']
    if options != env['variants'][variant]['options']:
        raise ValueError('original compiler options changed')
    for key in ('skip_borrowck', 'no_typecheck', 'no_normalize', 'monomorphize'):
        if options[key]:
            raise ValueError('forbidden extraction switch: ' + key)
    if options['opaque'] != ['rusqlite::error::Error'] or options['exclude']:
        raise ValueError('unreviewed opaque/excluded interface')
    if llbc_digest(llbc) != env['variants'][variant]['full_llbc_digest']:
        raise ValueError('complete IR, source position, Drop or ordered data changed')
    return env


def audit_names(text):
    names = re.findall(r'^#audit_controller_proof ([A-Za-z0-9_.]+)$', text, re.M)
    if len(names) != len(set(names)):
        raise ValueError('duplicate proof audit')
    return set(names)


def expected_audits(root, negative=False):
    env = read(root / ENVIRONMENT)
    expected = set(env['audits']['no-limit' if negative else 'correct'])
    path = NEGATIVE_PROOF if negative else MODELS / 'Audit.lean'
    if not expected or audit_names((root / path).read_text()) != expected:
        raise ValueError('missing/additional proof audit target')
    return expected


def check_generated(root, generated, variant):
    env = read(root / ENVIRONMENT)
    actual = {str(p.relative_to(generated)): sha(p) for p in generated.rglob('*') if p.is_file()}
    if actual != env['variants'][variant]['generated']:
        raise ValueError('full generated files, including templates, changed')
    prefix = LIBRARY if variant == 'correct' else ASSETS / 'negative/generated'
    for name in ('SandproofBoundedWriter/Types.lean', 'SandproofBoundedWriter/Funs.lean', 'translation.json'):
        if (root / prefix / name).read_bytes() != (generated / name).read_bytes():
            raise ValueError('promoted generation differs from original tool: ' + name)
    inventory = read(generated / 'translation.json')
    if inventory != env['variants'][variant]['translation']:
        raise ValueError('full type/function/trait inventory changed')
    return inventory


def check_pair(root):
    env = read(root / ENVIRONMENT)
    correct = read(root / ASSETS / 'positive/original.llbc')
    wrong = read(root / ASSETS / 'negative/original.llbc')
    check_ir(root, correct, 'correct')
    check_ir(root, wrong, 'no-limit')
    if env['variants']['correct']['full_llbc_digest'] == env['variants']['no-limit']['full_llbc_digest']:
        raise ValueError('wrong source was not independently extracted')
    type_file = 'SandproofBoundedWriter/Types.lean'
    if env['variants']['correct']['generated'][type_file] != env['variants']['no-limit']['generated'][type_file]:
        raise ValueError('correct/wrong Rust types differ')
    for name in ('Model.lean', 'Contract.lean', 'Library.lean'):
        if name not in env['same_property_files']:
            raise ValueError('independent specification not registered')
        if sha(root / MODELS / name) != env['same_property_files'][name]:
            raise ValueError('correct/wrong property changed: ' + name)
    for path in list((root / MODELS).glob('*.lean')) + [root / NEGATIVE_PROOF,
            root / LIBRARY / 'SandproofBoundedWriter/TypesExternal.lean']:
        if re.search(r'^\s*(?:axiom|sorry|admit)\b|\b(?:sorry|admit)\s*$', path.read_text(), re.M):
            raise ValueError('unproved declaration: ' + str(path))
    for variant in ('correct', 'no-limit'):
        names = env['variants'][variant]['translation']['functions']
        declarations = {f['def_id']: f for f in (correct if variant == 'correct' else wrong)['translated']['fun_decls'] if f}
        for f in names:
            if f['rust_name'].startswith(('sandproof_bounded_writer_extract::',
                    'sandproof_controller_service::', 'sandproof_internal::')):
                body = declarations.get(f['def_id'], {}).get('body')
                if f['is_opaque'] or not isinstance(body, dict) or 'Structured' not in body:
                    raise ValueError('owned policy/conversion omitted or opaque')
    expected_audits(root)
    expected_audits(root, True)
    return env


def check_tools(root, tool_root):
    env = read(root / ENVIRONMENT)
    for name, revision in env['tools'].items():
        actual = subprocess.check_output(['git', '-C', str(tool_root / name), 'rev-parse', 'HEAD'], text=True).strip()
        dirty = subprocess.check_output(['git', '-C', str(tool_root / name), 'status', '--porcelain', '--untracked-files=no'], text=True)
        if actual != revision or dirty:
            raise ValueError('original tool revision/source changed: ' + name)
    for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
        if sha(tool_root / relative) != env['tool_binaries_sha256'][name]:
            raise ValueError('original binary changed: ' + name)
    backend = root / '.lake/packages/aeneas'
    if subprocess.check_output(['git', '-C', str(backend), 'rev-parse', 'HEAD'], text=True).strip() != env['tools']['aeneas']:
        raise ValueError('Lean backend revision changed')
    if subprocess.check_output(['git', '-C', str(backend), 'status', '--porcelain', '--untracked-files=no'], text=True):
        raise ValueError('Lean backend tracked source changed')
    if sha(root / 'lakefile.toml') != env['runtime']['private_lakefile_sha256']:
        raise ValueError('private Lean configuration changed')
    charon = tool_root / 'aeneas/charon/bin/charon'
    roots = {'production': Path(subprocess.check_output(['rustc', '--print', 'sysroot'], text=True).strip()),
             'extraction': Path(subprocess.check_output([str(charon), 'toolchain-path'], text=True).strip())}
    for role, base in roots.items():
        for name, digest in env['library_sources'][role].items():
            if sha(base / 'lib/rustlib/src/rust/library' / name) != digest:
                raise ValueError('fixed library source changed: ' + role + '/' + name)
    registry = Path(os.environ['CARGO_HOME']) / 'registry/src'
    for name, digest in env['registry_sources'].items():
        matches = list(registry.glob('*/' + name))
        if len(matches) != 1 or sha(matches[0]) != digest:
            raise ValueError('fixed registry source changed: ' + name)
    for name in ('lean', 'lake'):
        executable = shutil.which(name)
        if executable is None or sha(Path(executable)) != env[name + '_binary_sha256']:
            raise ValueError('fixed Lean tool binary changed: ' + name)
    if subprocess.check_output(['lean', '--version'], text=True).strip() != env['lean_version']:
        raise ValueError('Lean version differs')
    if subprocess.check_output(['rustc', '-vV'], text=True) != env['production_rustc']:
        raise ValueError('production compiler differs')
    if subprocess.check_output([str(charon), 'toolchain-version'], text=True).strip() != env['extraction_toolchain']:
        raise ValueError('extraction compiler differs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--tool-root', type=Path)
    parser.add_argument('--prepared-variant', choices=['correct', 'no-limit'])
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    check_frozen(root, args.prepared_variant)
    check_sources(root, args.prepared_variant)
    check_pair(root)
    if args.tool_root:
        check_tools(root, args.tool_root.resolve(strict=True))
    print('Bounded Writer source, complete IR, ordinary-value scope and exact audit inventory admitted.')


if __name__ == '__main__':
    main()
