"""Materialize the actual finish body with the checked #936 and #770 inputs."""
import argparse
import json
from pathlib import Path
import re
import shutil

from check_revision_pair import checked_commands
from finish_capture import ARCHIVE, MODULES, SOURCE, TREE, admit, configuration
from prepare_started_transaction import PROVIDER
from prepare_unload_frontier import capture_inputs as captured_frontier
from prepare_world_begin import HERE, ROOT, closure, prepare as materialize_begin, read_exact
import prepare_transaction_capture as transaction


VIEW = 'verification/aeneas/unload-finish'
INPUTS = 'UnloadFinishInputs'
ROOTS = {'tail': 'UnloadFinish', 'reordered': 'UnloadFinishReordered'}
CANDIDATE = 'a3d5c9085b7e6f76de877b67a99bf1bf55e014891c2af090fc42be575060f173'
REORDERED = '135ccf6216a2e394ccedf512c441848645de517d92ae667ea06466fc844ec7cf'


def captured(path, mode, *, identity=None):
    report = json.loads((path / 'result.json').read_bytes())
    commit, tree, archive = (SOURCE, TREE, ARCHIVE) if identity is None else (
        identity['head'], identity['tree'], identity['source_archive_sha256'])
    candidate = REORDERED if mode == 'reordered' else CANDIDATE
    if (report['complete'] is not True or report['public_acceptance'] is not False
            or (report['mode'], report['source_commit'], report['source_tree'],
                report['candidate_source_sha256']) != (mode, commit, tree, candidate)):
        raise ValueError('exact complete finish capture required')
    if ([row['label'] for row in report['commands']] != ['charon', 'aeneas']
            or any(type(row['exit_code']) is not int or row['exit_code'] != 0 or row['bounded_stop'] is not False
                   for row in report['commands'])):
        raise ValueError('both original extractor results must complete')
    read_exact(HERE / 'finish_capture.py', report['driver_sha256'])
    read_exact(path / 'source.tar', archive)
    read_exact(path / 'source' / transaction.SOURCE, candidate)
    for name, sha in transaction.SOURCE_INPUTS.items():
        if name != transaction.SOURCE:
            read_exact(path / 'source' / name, sha)
    binding = json.loads((PROVIDER / 'rust-source-binding.json').read_bytes())['files']
    for name in ('controller-service/src/controller/transaction.rs',
                 'controller-service/src/controller/transaction/memo.rs', 'Cargo.lock'):
        read_exact(path / 'source' / name, binding[name])
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = path / 'generated'
    module = MODULES[mode]
    names = {'translation.json', *(f'{module}/{name}.lean' for name in
                                  ('Funs', 'Types', 'FunsExternal_Template', 'TypesExternal_Template'))}
    if set(report['generated_sha256']) != names:
        raise ValueError('complete generated output inventory required')
    files = {name: read_exact(generated / name, report['generated_sha256'][name]) for name in names}
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    cfg = configuration(mode)
    checked_commands(path, report['commands'], cfg, fixed['target'])
    if admit(ir, generated, cfg, fixed['target'], mode) != report['admission']:
        raise ValueError('actual finish admission differs')
    return files, report


def materialize(tail, parent, prefix, prefix_parent, begin, output, packages, mcp, mode, *, identity=None):
    generated, actual = captured(tail, mode, identity=identity)
    parent_files, whole = captured(parent, 'parent', identity=identity)
    _, admitted_prefix = captured_frontier(prefix, 'prefix', identity=identity)
    captured_frontier(prefix_parent, 'parent', identity=identity)
    opening = b'fn open_source_work_inputs'
    preceding = (prefix / 'source' / transaction.SOURCE).read_bytes().split(opening, 1)[1]
    full = (parent / 'source' / transaction.SOURCE).read_bytes().split(opening, 1)[1]
    if preceding.split(b'\n}\n', 1)[0] != full.split(b'\n}\n', 1)[0]:
        raise ValueError('the admitted original prefix body must stay exact')
    root_name = b'controller.lifecycle_inbox.source_work.ControllerJournal.prepare_lifecycle_source_work\n'
    _, caller = parent_files[MODULES['parent'] + '/Funs.lean'].split(root_name, 1)
    opened = re.search(
        rb'source_work\.open_source_work_inputs\n.*?\| core\.ops\.control_flow\.ControlFlow\.Continue (\w+) =>\n'
        rb'    let \((\w+), (\w+), \w+\) := \1\n', caller, re.S)
    finishing = re.search(rb'source_work\.finish_source_work\n +[^\n]+\n +CrashInjectorInst (\w+)\n'
                          rb'.*?\} (\w+)\.config\.max_sessions crashes\n', caller, re.S)
    if opened is None or finishing is None or opened.groups()[1:] != finishing.groups():
        raise ValueError('the generated parent must pass the opened owner and the original Inbox capacity')
    materialize_begin(begin, output, packages, mcp, identity=identity)
    plan_path = output / 'plan.json'
    plan = json.loads(plan_path.read_bytes())
    root, module = ROOTS[mode], MODULES[mode]
    with (output / 'lakefile.toml').open('a') as view:
        for library in (module, INPUTS, root):
            view.write(f'\n[[lean_lib]]\nname = "{library}"\nsrcDir = "{VIEW}"\n')
    target = output / VIEW / module
    target.mkdir(parents=True)
    for file in ('Funs.lean', 'Types.lean'):
        (target / file).write_bytes(generated[f'{module}/{file}'])
    local = HERE / 'finish'
    for file in ('FunsExternal.lean', 'TypesExternal.lean'):
        shutil.copyfile(local / (file + '.in'), target / file)
    proof = f'{VIEW}/{root}.lean'
    added = {f'{VIEW}/{module}/{file}' for file in ('Funs.lean', 'Types.lean', 'FunsExternal.lean', 'TypesExternal.lean')}
    for name in (INPUTS, root):
        file = f'{VIEW}/{name}.lean'
        shutil.copyfile(local / (name + '.lean.in'), output / file)
        added.add(file)
    reachable = closure(output, [proof], active_view='lakefile.toml')
    original_provider = {row['path']: row['sha256'] for row in
                         json.loads((PROVIDER / 'materialization.json').read_bytes())['modules'].values()}
    for name, sha in plan['files'].items():
        if reachable.get(name) != sha:
            raise ValueError('the admitted original begin closure must stay exact')
    if (added - reachable.keys() or any(original_provider.get(name) != sha
                                       for name, sha in reachable.items()
                                       if name not in plan['files'] and name not in added)):
        raise ValueError('unexpected original commit dependencies')
    plan.update(source=actual['source_commit'], root=root, files=reachable, mode=mode,
                finish_ir=actual['ir_sha256'], factored_parent_ir=whole['ir_sha256'],
                prior_prefix_ir=admitted_prefix['ir_sha256'],
                candidate_source=actual['candidate_source_sha256'],
                strict_files=sorted(added | set(plan['strict_files'])),
                modules={name: {'sha256': sha} for name, sha in reachable.items()},
                build_order=[name.removesuffix('.lean') for name in reachable])
    plan_path.write_text(json.dumps(plan, indent=2) + '\n')
    print(f'{root}: {len(reachable)} exact Lean inputs')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=tuple(ROOTS), required=True)
    for name in ('tail', 'parent', 'prefix', 'prefix-parent', 'begin', 'output', 'lake-packages', 'lean-mcp'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    materialize(args.tail.resolve(strict=True), args.parent.resolve(strict=True),
                args.prefix.resolve(strict=True), args.prefix_parent.resolve(strict=True),
                args.begin.resolve(strict=True), args.output.resolve(), args.lake_packages.resolve(strict=True),
                args.lean_mcp.resolve(strict=True), args.mode)
