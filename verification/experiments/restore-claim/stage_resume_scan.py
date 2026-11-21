"""Expose the original loaded-source search and bind its registered library calls."""
import argparse
from pathlib import Path
import re
import shutil

from stage_decisions import RESTORE, RESTORE_SHA256
from stage_lean import HERE, MODULE, fixed_source


LIBRARY = 'SandproofRestoreResumeScan'
TYPES = {'core.io.error.Error', 'rusqlite.error.Error', 'serde_json.error.Error'}
FUNCTIONS = {
    'core.option.Option.as_ref',
    'core.option.Option.is_some_and',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any',
}


def stage_rust(project):
    source = fixed_source(project, RESTORE, RESTORE_SHA256)
    parent = source.index('pub(in crate::controller) fn permits_resume_input(')
    start = source.index('    let Some(row) = world.sandbox_record(request.sandbox) else {', parent)
    end = source.index('\n}', start)
    signature = '''

fn proof_permits_resume_loaded(
    sources: &codec::Sources,
    world: &core::World,
    request: &core::ResumeRequest,
) -> Result<bool, JournalError> {
'''
    (project / RESTORE).write_text(source[:start] + '    proof_permits_resume_loaded(&sources, world, request)'
                                  + source[end:end + 2] + signature + source[start:])


def definition(source, name):
    declaration = re.search(rf'^def {re.escape(name)}(?=\s)', source, re.MULTILINE)
    if declaration is None:
        raise ValueError(f'registered definition missing: {name}')
    return source[declaration.start():].split('\n\n', 1)[0]


def library_bindings(project):
    foundations = fixed_source(project, Path('verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'),
                               '23f9e47340b35666ebd55d920c01f6db44e269dcd09743aa3344a92789fe8a12')
    standard_read = fixed_source(project, Path('verification/experiments/stage1-protocol/protocol-record/standard-read/FunsExternal.lean.in'),
                                 '22f828972faacc28c7ea4a124bdd76c757910cfc4026753605d678727662f9bc')
    current = (HERE / 'External.lean.in').read_text()
    bindings = (
        definition(foundations, 'core.option.Option.as_ref'),
        definition(foundations, 'core.option.Option.is_some_and'),
        definition(current, 'core.option.Option.Insts.CoreCmpPartialEqOption.eq'),
        definition(standard_read, 'anyList'),
        definition(standard_read, 'core.slice.iter.Iter.Insts.CoreIterTraitsIteratorIteratorSharedAT.any'),
    )
    return (f'import {LIBRARY}.Types\n\nopen Aeneas Aeneas.Std\n'
            f'namespace {LIBRARY}Rust\n\n' + '\n\n'.join(bindings) + f'\n\nend {LIBRARY}Rust\n')


def stage_lean(project, generated):
    emitted = generated / LIBRARY
    for module, expected in (('TypesExternal', TYPES), ('FunsExternal', FUNCTIONS)):
        lines = (emitted / f'{module}_Template.lean').read_text().splitlines()
        actual = {line.split()[1] for line in lines if line.startswith('axiom ')}
        if actual != expected:
            raise ValueError(f'unexpected {module} symbols: {actual}')

    parent = Path('verification/aeneas/restore-resume-scan')
    library = project / parent / LIBRARY
    library.mkdir(parents=True)
    for module in ('Types', 'Funs'):
        shutil.copyfile(emitted / f'{module}.lean', library / f'{module}.lean')
    shutil.copyfile(HERE / 'ResumeScanTypesExternal.lean.in', library / 'TypesExternal.lean')
    (library / 'FunsExternal.lean').write_text(library_bindings(project))

    folder = project / MODULE
    folder.mkdir(parents=True)
    shutil.copyfile(HERE / 'ResumeScan.lean.in', folder / 'ResumeScan.lean')
    with (project / 'lakefile.toml').open('a') as output:
        output.write(f'\n[[lean_lib]]\nname = "{LIBRARY}"\nsrcDir = "{parent.as_posix()}"\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('--generated', type=Path)
    args = parser.parse_args()
    if args.generated is None:
        stage_rust(args.project)
    else:
        stage_lean(args.project, args.generated)
