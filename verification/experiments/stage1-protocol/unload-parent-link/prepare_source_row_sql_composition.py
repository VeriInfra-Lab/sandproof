"""Bind the actual composite generated namespace to the existing A4/A9 models."""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare_source_row_sql_select import VALUE_DATA, VALUE_SHA
from prepare_source_row_sql_update import AUDIT, PROVIDERS, STDLIB
from prepare_world_begin import closure, read_exact
from source_row_sql_composition import ARCHIVE, CODEC, MODES, NAMESPACE, ROOT, SOURCE, TREE, admit, digest
from source_row_sql_select import VACANT


HERE = Path(__file__).resolve().parent / 'row-sql-composition'
SELECT = HERE.with_name('row-sql-select')
DIGEST = HERE.parents[1] / 'standard-digest'
ROOTS = {'core': 'SourceRowSqlComposition', 'greatest-vacancy': 'SourceRowSqlCompositionWrong'}
WRITERS = {
    'core': '7809df9a648357a43d525605dc0d2ba36cf29a0b6aed7c956e9d92d0a93c41c5',
    'greatest-vacancy': 'e2bb0663f47a31171b7b6adb2b9d66f40e4c4c295eebde21c06ec20d58d4eec0',
}

SELECT_SHA = {
    'FunsExternal': '5e96cb45a18b6011bcbc0a6ec89591c2d61bedd467b88a71bd6dfba48b6f91d6',
    'TypesExternal': '4358d75a607ed5939f1e23f08456062e53df5d0c125df7969a9d4d7e906e2596',
    'SourceRowSelectInputs': '76ffe126e53608680352be2fae777169e8ccf99b5e7e94fda59a5d724a164942',
    'SourceRowSqlSelect': 'bd4d1f7b4c5d1510e1771d276460a081a9b5a91cfd2b217990c4fb9f54ccf7f1',
    'SourceRowSqlGreatestVacancy': 'eef76547d86570de26d8b21327f0bdc785764e036ef4c1380ccdb5b851a76ab0',
}


def select_template(name):
    return read_exact(SELECT / (name + '.lean.in'), SELECT_SHA[name]).decode()


def captured(directory, mode):
    receipt = json.loads((directory / 'result.json').read_bytes())
    driver = HERE.parent / 'source_row_sql_composition.py'
    if (receipt['mode'], receipt['source_commit'], receipt['source_tree'],
            receipt['driver_sha256'], receipt['complete'], receipt['public_acceptance']) != (
            mode, SOURCE, TREE, digest(driver), True, False):
        raise ValueError('an actual complete composite capture is required')
    if [(row['label'], row['exit_code'], row['bounded_stop']) for row in receipt['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed captures must finish')
    read_exact(directory / 'source.tar', ARCHIVE)
    read_exact(directory / 'source' / CODEC, receipt['codec_sha256'])
    if receipt['codec_sha256'] != WRITERS[mode]:
        raise ValueError('actual composed source changed')
    ir = directory / 'actual.llbc'
    read_exact(ir, receipt['ir_sha256'])
    for row in receipt['commands']:
        read_exact(directory / (row['label'] + '.log'), row['log_sha256'])
    generated = directory / 'generated'
    files = {'translation.json', *(f'{NAMESPACE}/{name}' for name in (
        'Types.lean', 'Funs.lean', 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    if (files != receipt['generated_sha256'].keys()
            or files != {str(path.relative_to(generated)) for path in generated.rglob('*') if path.is_file()}):
        raise ValueError('unexpected generated inventory')
    sources = {name: read_exact(generated / name, receipt['generated_sha256'][name]) for name in files}
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(ir, generated, target) != receipt['admission']:
        raise ValueError('the chosen parent, concrete dictionary or A9 call changed')
    return {name: sources[f'{NAMESPACE}/{name}'] for name in ('Types.lean', 'Funs.lean')}


def reuse_select():
    ext = select_template('FunsExternal')
    ext = ext.replace('import SandproofSourceRowSqlSelect.Types\n',
                      'import SandproofSourceRowSqlSelect.Types\n'
                      'import SandproofStandardDigest.FunsExternal\n'
                      'import SandproofFoundations.IdentifierLibrary\n', 1)
    # In this view the actual reference ToSql and slice ToSql bodies are present.
    ext = ext[:ext.index('def Shared0T.Insts.RusqliteTypesTo_sqlToSql.to_sql')] + ext[ext.index('def bindBlobReply'):]
    first = ext.index('-- The firstRow provider owns')
    ext = ext[:first] + ext[ext.index('def rusqlite.row.Row.get', first):]
    ext = ext[:ext.index('def SliceU8.Insts.RusqliteTypesTo_sqlToSql.to_sql')]
    ext = ext.replace('let converted ← fromSqlDictionary.column_result value',
                      'let converted ← fromSqlDictionary.column_result (asValueRef value)', 1)
    addition = (HERE / 'JoinedBinding.lean.in').read_text()
    cast, body = addition.split('def alloc.string.String.as_bytes', 1)
    at = ext.index('def rusqlite.row.Row.get')
    ext = ext[:at] + cast + ext[at:] + 'def alloc.string.String.as_bytes' + body
    types = select_template('TypesExternal').replace(
        'rusqlite.types.value_ref.ValueRef', 'SandproofSourceSlotRead.rusqlite.types.value_ref.ValueRef')
    types = types.replace('namespace SandproofSourceRowSqlSelect\n', 'namespace SandproofSourceRowSqlSelect\n\n'
                          'abbrev F64 := SandproofFoundationsRust.F64\n', 1)
    proof = select_template('SourceRowSelectInputs')
    proof = proof.replace('SliceU8.Insts.RusqliteTypesTo_sqlToSql.to_sql,', '''SliceU8.Insts.RusqliteTypesTo_sqlToSql.to_sql,
    rusqlite.types.to_sql.ToSqlOutput.Insts.CoreConvertFromSharedAT.from,
''', 1)
    proof = proof.replace('bindOne, bindBlobReply,', 'bindOne,', 1)
    proof = proof.replace('simp [Slice.val, Slice.length, Aeneas.Data.ListN.ListN.toList]',
        '''simp [Slice.val, Slice.length, Aeneas.Data.ListN.ListN.toList,
    rusqlite.types.value_ref.ValueRef.Insts.CoreConvertFromSharedASliceU8.from, bindBlobReply]''', 1)
    proof = proof.replace('columnZero, h, value', 'columnZero, h, value, asValueRef')
    value = read_exact(VALUE_DATA, VALUE_SHA).decode()
    value = value.replace('SandproofLoadBlobRust', 'SandproofSourceSlotRead')
    value = re.sub(r'^@\[discriminant isize, rust_type [^\n]+\]\n', '', value, flags=re.M)
    return types, ext + '\n\nend SandproofSourceRowSqlSelect\n', proof, value


def adopt_sha(copy):
    provenance = json.loads((DIGEST / 'reuse-provenance.json').read_bytes())
    for name, expected in provenance['math_and_generated_data_unchanged'].items():
        source = DIGEST / 'reuse' / name
        data = read_exact(source, expected)
        if name.endswith('.lean.in'):
            copy('SandboxModel/Lifecycle/Protocol/' + name.removesuffix('.in'), data)
        elif name.endswith('/SandproofDigestArray32/Types.lean'):
            copy('SandproofDigestArray32/Types.lean', data)
    inventory = json.loads((DIGEST / 'inventory.json').read_bytes())
    board = 'SandboxModel/Foundations/ByteArray.lean'
    copy(board, read_exact(ROOT / board, '32c29657733cb72eb74e1657e816b00dc1e74f27f2926b94a1b1a788cda78bc9'))
    external = 'external/FunsExternal.lean.in'
    data = read_exact(DIGEST / external, inventory['package_inputs'][external]).replace(
        b'import SandproofStandardDigest.Types\n', b'', 1)
    copy('SandproofStandardDigest/FunsExternal.lean', data)


def materialize(mode, source, reference, output, packages, mcp):
    generated = captured(source, mode)
    if mode != 'core':
        correct = captured(reference, 'core')
        sql = VACANT.encode()
        if (correct['Types.lean'] != generated['Types.lean'] or correct['Funs.lean'].count(sql) != 1
                or correct['Funs.lean'].replace(sql, sql.replace(b'min(', b'max('), 1) != generated['Funs.lean']):
            raise ValueError('the sole generated difference must be the actual vacancy statement')
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        destination = output / name
        destination.parent.mkdir()
        destination.symlink_to(target, target_is_directory=True)

    def copy(name, body):
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as file:
            file.write(body)

    for name in (AUDIT, 'lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        copy(name, (ROOT / name).read_bytes())
    for name, expected in PROVIDERS.items():
        copy(name.split('/', 1)[1], read_exact(ROOT / STDLIB / name, expected))
    adopt_sha(copy)
    for name, body in generated.items():
        copy(f'{NAMESPACE}/{name}', body)
    types, funs, proof, value = reuse_select()
    for name, code in (('TypesExternal', types), ('FunsExternal', funs), ('ReadValueData', value)):
        copy(f'{NAMESPACE}/{name}.lean', code.encode())
    copy('SourceRowSelectInputs.lean', proof.encode())
    root = ROOTS[mode]
    read_root = 'SourceRowSqlSelect' if mode == 'core' else 'SourceRowSqlGreatestVacancy'
    actual_read = select_template(read_root)
    if mode != 'core':
        original = select_template('SourceRowSqlSelect')
        existing = original[original.index('theorem actual_existing'):original.index('theorem actual_vacant')]
        actual_read = actual_read.replace('theorem actual_greatest', existing + 'theorem actual_greatest', 1)
        actual_read += '#audit_controller_proof SandboxModel.Stage1Protocol.SourceRowSqlSelect.actual_existing\n'
    copy('ActualRead.lean', actual_read.encode())
    for name in ('ComposedEffects', 'RowComposition', root):
        copy(name + '.lean', (HERE / (name + '.lean.in')).read_bytes())
    with (output / 'lakefile.toml').open('x') as lake:
        aeneas, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n',
                             (ROOT / 'lakefile.toml').read_text())
        lake.write('name = "source-row-sql-composition"\n' + aeneas)
        for library in ('SandboxModel', 'SandproofSqlite', 'SandproofSqliteStatements', 'SandproofFoundations',
                        'SandproofDigestArray32', 'SandproofStandardDigest', NAMESPACE,
                        'SourceRowSelectInputs', 'ActualRead', 'ComposedEffects', 'RowComposition', root):
            lake.write(f'\n[[lean_lib]]\nname = "{library}"\n')
            if library == NAMESPACE:
                lake.write('leanOptions = { weak.linter.style.nameCheck = false }\n')
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    physical = {str(path.relative_to(output)) for path in output.rglob('*.lean') if not path.is_symlink()}
    if files.keys() != physical:
        raise ValueError('the actual closure must contain every physical proof file')
    plan = dict(root=root, mode=mode, source=SOURCE, files=files, strict_files=sorted(physical),
                modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact actual Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=MODES, required=True)
    for name in ('capture', 'reference', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    materialize(args.mode, args.capture.resolve(strict=True), args.reference.resolve(strict=True),
                args.output.resolve(), args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
