"""Materialize the fixed source SELECT views with the finite A4 read interface."""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare_source_row_sql_update import AUDIT, PROVIDERS, STDLIB
from prepare_world_begin import closure, read_exact
from source_row_sql_select import ARCHIVE, CODEC, HERE, NAMESPACE, ROOT, SOURCE, TREE, VACANT, admit, digest


ROOTS = {'core': 'SourceRowSqlSelect', 'greatest-vacancy': 'SourceRowSqlGreatestVacancy'}
PROBE_SHA = {
    'core': 'cff1f9c3340d75b2813585ea7fb2af669a01e057df45048b6dc90207456ac20c',
    'greatest-vacancy': '55abb8c8bd072f752282ad65241e379e325b2d61f38479bc851f122cb7841577',
}
VALUE_DATA = HERE.parents[1] / 'standard-blob/external/ReadValueData.lean.in'
VALUE_SHA = 'd05b9a6e9219ddf5a9d429bdaf77280e569b4cd8c7475c7f38e77a38f81b3db9'


def captured(directory, mode):
    receipt = json.loads((directory / 'result.json').read_bytes())
    driver = HERE.parent / 'source_row_sql_select.py'
    if (receipt['mode'], receipt['source_commit'], receipt['source_tree'], receipt['codec_sha256'],
            receipt['driver_sha256'], receipt['complete'], receipt['public_acceptance']) != (
            mode, SOURCE, TREE, PROBE_SHA[mode], digest(driver), True, False):
        raise ValueError('actual fixed SELECT adapter capture required')
    if [(row['label'], row['exit_code'], row['bounded_stop']) for row in receipt['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed extraction tools must complete')
    read_exact(directory / 'source.tar', ARCHIVE)
    read_exact(directory / 'source' / CODEC, PROBE_SHA[mode])
    ir = directory / 'actual.llbc'
    read_exact(ir, receipt['ir_sha256'])
    for command in receipt['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    generated = directory / 'generated'
    files = {'translation.json', *(f'{NAMESPACE}/{name}' for name in (
        'Types.lean', 'Funs.lean', 'TypesExternal_Template.lean', 'FunsExternal_Template.lean'))}
    if (files != receipt['generated_sha256'].keys()
            or files != {str(path.relative_to(generated)) for path in generated.rglob('*') if path.is_file()}):
        raise ValueError('actual generated SELECT inventory differs')
    contents = {name: read_exact(generated / name, receipt['generated_sha256'][name]) for name in files}
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(ir, generated, target) != receipt['admission']:
        raise ValueError('actual SELECT call admission differs')
    return {name: contents[f'{NAMESPACE}/{name}'] for name in ('Types.lean', 'Funs.lean')}


def materialize(mode, capture, reference, output, packages, lean_mcp):
    generated = captured(capture, mode)
    if mode != 'core':
        correct = captured(reference, 'core')
        sql = VACANT.encode()
        if (generated['Types.lean'] != correct['Types.lean'] or correct['Funs.lean'].count(sql) != 1
                or generated['Funs.lean'] != correct['Funs.lean'].replace(sql, sql.replace(b'min(', b'max('), 1)):
            raise ValueError('greatest-vacancy output must only replace the actual aggregate')
    output.mkdir(mode=0o700)
    for name, destination in (('.lake/packages', packages), ('.tools/lean-mcp', lean_mcp)):
        entry = output / name
        entry.parent.mkdir()
        entry.symlink_to(destination, target_is_directory=True)
    for name in (AUDIT, 'lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        entry = output / name
        entry.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, entry)
    shared = set()
    for name, expected in PROVIDERS.items():
        if name.endswith('IdentifierLibrary.lean'):
            continue
        visible = name.split('/', 1)[1]
        entry = output / visible
        entry.parent.mkdir()
        entry.write_bytes(read_exact(ROOT / STDLIB / name, expected))
        shared.add(visible)
    root = ROOTS[mode]
    with (output / 'lakefile.toml').open('x') as lake:
        aeneas, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n',
                             (ROOT / 'lakefile.toml').read_text())
        lake.write('name = "source-row-sql-select"\n' + aeneas)
        for library in ('SandboxModel', 'SandproofSqlite', 'SandproofSqliteStatements', 'SandproofFoundations',
                        NAMESPACE, 'SourceRowSelectInputs', root):
            lake.write(f'\n[[lean_lib]]\nname = "{library}"\n')
            if library == NAMESPACE:
                lake.write('leanOptions = { weak.linter.style.nameCheck = false }\n')
    namespace = output / NAMESPACE
    namespace.mkdir()
    for name, body in generated.items():
        (namespace / name).write_bytes(body)
    for name in ('TypesExternal', 'FunsExternal'):
        shutil.copyfile(HERE / (name + '.lean.in'), namespace / (name + '.lean'))
    value_data = read_exact(VALUE_DATA, VALUE_SHA)
    (namespace / 'ReadValueData.lean').write_bytes(value_data.replace(b'SandproofLoadBlobRust', NAMESPACE.encode()))
    for name in ('SourceRowSelectInputs', root):
        shutil.copyfile(HERE / (name + '.lean.in'), output / (name + '.lean'))
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    exact = {AUDIT, 'SourceRowSelectInputs.lean', root + '.lean', *shared,
             *(f'{NAMESPACE}/{name}.lean' for name in ('Types', 'Funs', 'TypesExternal', 'FunsExternal', 'ReadValueData'))}
    if files.keys() != exact:
        raise ValueError('unexpected physical SELECT proof closure')
    plan = dict(root=root, mode=mode, source=SOURCE, files=files, strict_files=sorted(exact),
                modules={name: {'sha256': sha} for name, sha in files.items()},
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
