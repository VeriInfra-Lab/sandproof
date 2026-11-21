"""Materialize the factored first-Unload prefix with the original #930 provider."""
import argparse
import json
from pathlib import Path
import shutil

from check_current_prepare_capture import CURRENT
from check_revision_pair import checked_commands
from frontier_capture import MODULES, admit, configure
from prepare_world_begin import HERE, ROOT, closure, prepare as prepare_begin, read_exact
import prepare_transaction_capture as transaction
from probe_prepare_iterators import BASELINES, REQUEST_CARRIERS


MODULE = MODULES['prefix']
PROOF = 'UnloadFrontier'
NEGATIVE_PROOF = 'UnloadFrontierReordered'
INPUTS = 'UnloadFrontierInputs'
VIEW = 'verification/aeneas/unload-frontier'
AUTH_INPUTS = (
    'controller-service/src/controller/lifecycle_inbox.rs',
    'controller-service/src/controller/lifecycle_inbox/model.rs',
    'sandbox-protocol/src/lifecycle/authority.rs',
    'sandbox-protocol/src/lifecycle/identity.rs',
    'sandbox-protocol/src/lifecycle/request.rs',
)
CANDIDATE = 'da11e9e7e8f9aab90b4ef6911169f62ba2f5cab1b2066491903045d90a83dbf6'
REORDERED = '650362a8fc2dfb2811152c13b8514af907c05af3fb5dfdcec9ce6eccf31ab009'


def capture_inputs(path, mode, *, identity=None):
    report = json.loads((path / 'probe-result.json').read_bytes())
    commit = CURRENT if identity is None else identity['head']
    tree, archive = BASELINES[CURRENT] if identity is None else (
        identity['tree'], identity['source_archive_sha256'])
    source = REORDERED if mode == 'reordered' else CANDIDATE
    if ((report['source_commit'], report['source_tree'], report['variant'], report['frontier']) !=
            (commit, tree, 'json-boundary', mode)
            or report['wrapper_ir'] is not False or report['generated'] is not True
            or report['candidate_source_sha256'] != source or report['archive_sha256'] != archive
            or 'error' in report):
        raise ValueError('exact complete factored capture required')
    if ([row['label'] for row in report['commands']] != ['charon', 'aeneas']
            or any(type(row['exit_code']) is not int or row['exit_code'] != 0
                   or row['bounded_stop'] is not False for row in report['commands'])):
        raise ValueError('both original extraction steps must complete')
    read_exact(path / 'source.tar', archive)
    read_exact(path / 'source' / transaction.SOURCE, source)
    for name, sha in transaction.SOURCE_INPUTS.items():
        if name != transaction.SOURCE:
            read_exact(path / 'source' / name, sha)
    binding = ROOT / 'verification/experiments/create-transaction-link/current/rust-source-binding.json'
    bound = json.loads(binding.read_bytes())['files']
    for name in AUTH_INPUTS:
        read_exact(path / 'source' / name, bound[name])
    for name, field in (('probe_prepare_iterators.py', 'driver_sha256'),
                        ('frontier_capture.py', 'frontier_driver_sha256')):
        read_exact(HERE / name, report[field])
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    directory = path / 'generated'
    names = {'translation.json', *(f'{MODULES[mode]}/{name}.lean' for name in
                                   ('Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if set(report['generated_sha256']) != names:
        raise ValueError('complete generated file inventory required')
    files = {name: read_exact(directory / name, report['generated_sha256'][name]) for name in names}
    profile = ROOT / 'verification/experiments/stage1-protocol/unload-startup-source/capture-profile.json'
    cfg = transaction.configuration(json.loads(profile.read_bytes())['parent'])
    cfg['include'].extend(REQUEST_CARRIERS)
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    cfg = configure(cfg, mode)
    checked_commands(path, report['commands'], cfg, fixed['target'])
    if admit(ir, directory, cfg, fixed['target'], mode) != report['frontier_admission']:
        raise ValueError('original call admission differs')
    return files, report


def materialize(prefix, parent, begin, output, packages, lean_mcp, reordered, *, identity=None):
    generated, primary = capture_inputs(prefix, 'reordered' if reordered else 'prefix', identity=identity)
    _, whole = capture_inputs(parent, 'parent', identity=identity)
    prepare_begin(begin, output, packages, lean_mcp, identity=identity)
    plan_file = output / 'plan.json'
    plan = json.loads(plan_file.read_bytes())
    root = NEGATIVE_PROOF if reordered else PROOF
    with (output / 'lakefile.toml').open('a') as config:
        for library in (MODULE, INPUTS, root):
            config.write(f'\n[[lean_lib]]\nname = "{library}"\nsrcDir = "{VIEW}"\n')
    module = output / VIEW / MODULE
    module.mkdir(parents=True)
    for name in ('Types.lean', 'Funs.lean'):
        (module / name).write_bytes(generated[MODULE + '/' + name])
    for name in ('TypesExternal.lean', 'FunsExternal.lean'):
        shutil.copyfile(HERE / 'frontier' / (name + '.in'), module / name)
    proof = f'{VIEW}/{root}.lean'
    for name in (INPUTS, root):
        shutil.copyfile(HERE / 'frontier' / (name + '.lean.in'), output / VIEW / (name + '.lean'))
    added = {f'{VIEW}/{MODULE}/{name}' for name in ('Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean')}
    added.update((proof, f'{VIEW}/{INPUTS}.lean'))
    files = closure(output, [proof], active_view='lakefile.toml')
    if ({name: sha for name, sha in files.items() if name not in added} != plan['files']
            or added - files.keys()):
        raise ValueError('unexpected original dependency closure')
    plan.update(source=primary['source_commit'], root=root, files=files,
                strict_files=sorted(added | set(plan['strict_files'])),
                mode=primary['frontier'], candidate_source=primary['candidate_source_sha256'],
                prefix_ir=primary['ir_sha256'], parent_ir=whole['ir_sha256'],
                modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    plan_file.write_text(json.dumps(plan, indent=2) + '\n')
    print(f'{root}: {len(files)} exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('prefix', 'parent', 'begin', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--reordered', action='store_true')
    args = parser.parse_args()
    materialize(args.prefix.resolve(strict=True), args.parent.resolve(strict=True), args.begin.resolve(strict=True),
                args.output.resolve(), args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True),
                args.reordered)
