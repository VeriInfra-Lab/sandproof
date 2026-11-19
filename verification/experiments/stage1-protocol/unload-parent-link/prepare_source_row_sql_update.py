"""Materialize exact actual source UPDATE outputs with the existing A4 execute binding."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import shutil

from prepare_world_begin import closure, read_exact
from source_row_sql_update import ARCHIVE, CODEC, HERE, NAMESPACE, ROOT, SOURCE, TREE, admit, digest
from stage_row_sql_update import STAGED


ROOTS = {'core': 'SourceRowSqlUpdate', 'swap-key-digest': 'SourceRowSqlUpdateSwapped'}
OWN = ('Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean', 'SqliteCommonBinding.lean')
AUDIT = 'SandboxModel/ProofAuditCommand.lean'
STDLIB = Path('verification/aeneas')
PROVIDERS = {
    'sqlite-statements/SandproofSqliteStatements/TypesExternal.lean': 'b22b9a544018ac114c7d992acfdc2ab28b0379cf4ddb82b59955abc92048e7ab',
    'sqlite/SandproofSqlite/TypesExternal.lean': '4bd78eb7f545377f7fea06ccb5a89f2b2b10e9f0e2a2d171284c063fa338b673',
    'foundations/SandproofFoundations/TypesExternal.lean': '10deee0bfc54132835d05b064231b13a9e4806a7dddea94b8d584561d66fc3f8',
    'foundations/SandproofFoundations/IdentifierLibrary.lean': 'e4d24aa239104ab0c362ba246d53072398b8f9be79109e015f1b1d68523f4c9c',
}
BLOB = HERE.parents[1] / 'standard-save-digest/blob'
COMMON = HERE.parents[1] / 'create-lifetime-sql/materialize.py'


def captured(path, mode):
    receipt = json.loads((path / 'result.json').read_bytes())
    driver = HERE.parent / 'source_row_sql_update.py'
    if (receipt['mode'], receipt['source_commit'], receipt['source_tree'], receipt['codec_sha256'],
            receipt['driver_sha256'], receipt['complete'], receipt['public_acceptance']) != (
            mode, SOURCE, TREE, STAGED[mode], digest(driver), True, False):
        raise ValueError('actual checked SQLite adapter capture required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in receipt['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tool invocations must finish')
    read_exact(path / 'source.tar', ARCHIVE)
    read_exact(path / 'source' / CODEC, STAGED[mode])
    for item in receipt['commands']:
        read_exact(path / (item['label'] + '.log'), item['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, receipt['ir_sha256'])
    generated = path / 'generated'
    names = {'translation.json', *(f'{NAMESPACE}/{name}' for name in ('Types.lean', 'Funs.lean',
        'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    observed = {str(file.relative_to(generated)) for file in generated.rglob('*') if file.is_file()}
    if observed != names or observed != receipt['generated_sha256'].keys():
        raise ValueError('concrete SQLite adapter physical inventory differs')
    for name in names:
        read_exact(generated / name, receipt['generated_sha256'][name])
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(ir, generated, target) != receipt['admission']:
        raise ValueError('the actual original SQLite call must be used')
    return receipt, {name: (generated / NAMESPACE / name).read_bytes() for name in ('Types.lean', 'Funs.lean')}


def original_common():
    inventory = json.loads((BLOB / 'inventory.json').read_bytes())
    read_exact(COMMON, inventory['providers'][str(COMMON.relative_to(ROOT))])
    source = BLOB / 'TypesExternal.lean.in'
    types = read_exact(source, inventory['templates'][str(source.relative_to(ROOT))]).decode()
    module = importlib.util.spec_from_file_location('row_sql_existing_a4', COMMON)
    common = importlib.util.module_from_spec(module)
    module.loader.exec_module(common)
    if common.hashes(ROOT) != inventory['exact_common_fragments']:
        raise ValueError('the original A4 and string providers changed')
    fragments = common.fragments(ROOT)
    body = '\n'.join(fragments[name] for name in ('sql_to_str', 'sql_string_deref', 'sql_bind_values', 'sql_execute'))
    adapter = f'''import {NAMESPACE}.Types
import SandproofFoundations.IdentifierLibrary

open Aeneas Aeneas.Std
namespace SandproofSqliteStatementsRust
abbrev rusqlite.params.Params := {NAMESPACE}.rusqlite.params.Params

''' + body + '\nend SandproofSqliteStatementsRust\n'
    return types.replace('SandproofInboxSaveBlobRust', NAMESPACE), adapter


def materialize(mode, actual, reference, output, packages, mcp):
    receipt, contents = captured(actual, mode)
    if mode != 'core':
        correct = captured(reference, 'core')[1]
        ordered = 'let s ← lift (Array.to_slice sha)\n  let s1 ← lift (Array.to_slice key)'.encode()
        swapped = 'let s ← lift (Array.to_slice key)\n  let s1 ← lift (Array.to_slice sha)'.encode()
        if (contents['Types.lean'] != correct['Types.lean'] or correct['Funs.lean'].count(ordered) != 1
                or contents['Funs.lean'] != correct['Funs.lean'].replace(ordered, swapped, 1)):
            raise ValueError('the generated error case must only exchange the digest and key inputs')
    types, binding = original_common()
    root = ROOTS[mode]
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        destination = output / name
        destination.parent.mkdir()
        destination.symlink_to(target, target_is_directory=True)
    for name in (AUDIT, 'lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, destination)
    provider_files = set()
    for name, expected in PROVIDERS.items():
        source = ROOT / STDLIB / name
        relative = name.split('/', 1)[1]
        dest = output / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_bytes(read_exact(source, expected))
        provider_files.add(relative)
    with (output / 'lakefile.toml').open('x') as config:
        aeneas, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', (ROOT / 'lakefile.toml').read_text())
        config.write('name = "source-row-sql-update"\n' + aeneas)
        for library in ('SandboxModel', 'SandproofSqlite', 'SandproofSqliteStatements', 'SandproofFoundations',
                        NAMESPACE, 'SourceRowSqlInputs', root):
            config.write(f'\n[[lean_lib]]\nname = "{library}"\n')
            if library == NAMESPACE:
                # Use the existing setting for the generated rusqlite __bind_in ABI.
                config.write('leanOptions = { weak.linter.style.nameCheck = false }\n')
    generated = output / NAMESPACE
    generated.mkdir()
    for name, content in contents.items():
        (generated / name).write_bytes(content)
    (generated / 'TypesExternal.lean').write_text(types)
    (generated / 'SqliteCommonBinding.lean').write_text(binding)
    shutil.copyfile(HERE / 'FunsExternal.lean.in', generated / 'FunsExternal.lean')
    for name in ('SourceRowSqlInputs', root):
        shutil.copyfile(HERE / (name + '.lean.in'), output / (name + '.lean'))
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = {AUDIT, 'SourceRowSqlInputs.lean', root + '.lean', *(f'{NAMESPACE}/{name}' for name in OWN), *provider_files}
    if files.keys() != expected:
        raise ValueError('unexpected SQLite adapter Lean closure')
    plan = dict(root=root, mode=mode, source=SOURCE, generated=receipt['generated_sha256'],
                files=files, strict_files=sorted(expected), modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('capture', 'reference', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    materialize(args.mode, args.capture.resolve(strict=True), args.reference.resolve(strict=True), args.output.resolve(),
                args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
