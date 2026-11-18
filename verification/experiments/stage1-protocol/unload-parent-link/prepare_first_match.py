"""Check the original tool outputs and prepare an independent first-match proof root."""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare_world_begin import closure, read_exact
from source_first_match import ARCHIVE, CODEC, DIGESTS, HERE, MODES, NAMESPACE, ROOT, SOURCE, TREE
from source_first_match import admit, digest


ROOTS = {'factored': 'SourceFirstMatch', 'binding-only': 'SourceFirstMatchBindingOnly'}
SUPPORT = ('SandboxModel/ProofAuditCommand.lean', 'SandboxModel/Lifecycle/Protocol/ReplayContract.lean')
BODIES = ('Types.lean', 'Funs.lean')


def captured(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    expected = DIGESTS['factored' if mode == 'parent' else mode]
    if (report['mode'], report['source_commit'], report['source_tree'], report['codec_sha256'],
            report['public_acceptance']) != (mode, SOURCE, TREE, expected, False):
        raise ValueError('exact current source and original extraction receipt required')
    if report['complete'] is not True or report['driver_sha256'] != digest(HERE.parent / 'source_first_match.py'):
        raise ValueError('completed receipt from the current extraction driver required')
    if ([row['label'] for row in report['commands']] != ['charon', 'aeneas']
            or any(type(row['exit_code']) is not int or row['exit_code'] != 0 or row['bounded_stop'] is not False
                   for row in report['commands'])):
        raise ValueError('both original tool invocations must have completed')
    read_exact(path / 'source.tar', ARCHIVE)
    read_exact(path / 'source' / CODEC, expected)
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = path / 'generated'
    env = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
    original = admit(ir, generated, env['target'], mode)
    names = {'translation.json', *(f'{NAMESPACE}/{name}' for name in BODIES)}
    if mode == 'parent':
        names.update(f'{NAMESPACE}/{name}External_Template.lean' for name in ('Types', 'Funs'))
    found = {str(file.relative_to(generated)): digest(file) for file in generated.rglob('*') if file.is_file()}
    if found.keys() != names or found != report['generated_sha256'] or original != report['admission']:
        raise ValueError('original generated inventory or check differs')
    bodies = {name: read_exact(generated / f'{NAMESPACE}/{name}', found[f'{NAMESPACE}/{name}']) for name in BODIES}
    return bodies, dict(ir=report['ir_sha256'], original_driver=report['driver_sha256'],
                        original_complete=report['complete'], generated=found, admission=original)


def steps(data):
    names = rb'retains_(?:inventory|record)_using(?:_loop(?:\.body)?)?'
    marker = rb'^def\s+controller\.lifecycle_inbox\.source_work\.codec\.'
    matches = re.findall(rb'(' + marker + names + rb'\n.*?)(?=\n\n(?:/--|end ))', data, re.M | re.S)
    if len(matches) != 6:
        raise ValueError('both actual scan definitions and four loop definitions required')
    return tuple(matches)


def materialize(capture, correct, parent, output, packages, mcp, mode):
    candidate, chosen = captured(capture, mode)
    good = candidate if mode == 'factored' else captured(correct, 'factored')[0]
    caller, actual = captured(parent, 'parent')
    if steps(caller['Funs.lean']) != steps(good['Funs.lean']):
        raise ValueError('isolated loops must be identical to the original parent loops')
    if mode == 'binding-only' and candidate['Types.lean'] != good['Types.lean']:
        raise ValueError('same complete comparison contract types required')
    root = ROOTS[mode]
    output.mkdir(mode=0o700)
    for folder, target in (('.lake', packages), ('.tools', mcp)):
        (output / folder).mkdir()
        (output / folder / ('packages' if folder == '.lake' else 'lean-mcp')).symlink_to(target, target_is_directory=True)
    for name in (*SUPPORT, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh', 'lean-toolchain', 'lake-manifest.json'):
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / name, target)
    with (output / 'lakefile.toml').open('x') as lake:
        lake.write('name = "source-first-match"\n')
        pinned = (ROOT / 'lakefile.toml').read_text()
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', pinned)
        lake.write(dependency)
        for library in ('SandboxModel', NAMESPACE, 'FirstMatchInputs', root):
            lake.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    (output / NAMESPACE).mkdir()
    for name, data in candidate.items():
        (output / NAMESPACE / name).write_bytes(data)
    for module in ('FirstMatchInputs', root):
        shutil.copyfile(HERE / f'{module}.lean.in', output / f'{module}.lean')
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(SUPPORT) | {f'{NAMESPACE}/{name}' for name in BODIES} | {'FirstMatchInputs.lean', root + '.lean'}
    if files.keys() != expected:
        raise ValueError('unexpected actual first-match Lean inputs')
    plan = dict(root=root, mode=mode, source=SOURCE, capture=chosen, parent=actual, files=files,
                strict_files=sorted(expected), modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=MODES, required=True)
    for name in ('capture', 'correct', 'parent', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    materialize(args.capture.resolve(strict=True), args.correct.resolve(strict=True), args.parent.resolve(strict=True),
                args.output.resolve(), args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True), args.mode)
