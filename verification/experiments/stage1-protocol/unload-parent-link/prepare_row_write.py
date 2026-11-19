"""Admit immutable row-writer outputs and build one independent Lean closure."""
import argparse
import json
from pathlib import Path
import re
import shutil

from prepare_world_begin import closure, read_exact
from source_row_write import ARCHIVE, CODEC, CODEC_DIGESTS, HERE, NAMESPACE, ROOT, SOURCE, TREE, admit, digest


ROOTS = {'core': 'SourceRowWrite', 'zero-ok': 'SourceRowWriteZeroAccepted'}
FILES = ('Types.lean', 'Funs.lean')
AUDIT = 'SandboxModel/ProofAuditCommand.lean'


def captured(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    source = CODEC_DIGESTS['zero-ok' if mode == 'zero-ok' else 'core']
    current = digest(HERE.parent / 'source_row_write.py')
    if (report['mode'], report['source_commit'], report['source_tree'], report['codec_sha256'],
            report['driver_sha256'], report['complete'], report['public_acceptance']) != (
            mode, SOURCE, TREE, source, current, True, False):
        raise ValueError('actual sealed row writer capture required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both original tool invocations must have finished')
    read_exact(path / 'source.tar', ARCHIVE)
    read_exact(path / 'source' / CODEC, source)
    for item in report['commands']:
        read_exact(path / (item['label'] + '.log'), item['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    gen = path / 'generated'
    names = {'translation.json', *(f'{NAMESPACE}/{name}' for name in FILES)}
    if mode == 'parent':
        names |= {f'{NAMESPACE}/{name}External_Template.lean' for name in ('Funs', 'Types')}
    found = {str(file.relative_to(gen)) for file in gen.rglob('*') if file.is_file()}
    if found != names or found != report['generated_sha256'].keys():
        raise ValueError('unexpected generated physical files')
    for name in names:
        read_exact(gen / name, report['generated_sha256'][name])
    target = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if admit(ir, gen, target, mode) != report['admission']:
        raise ValueError('row call and operand admission changed')
    return report, {name: (gen / NAMESPACE / name).read_bytes() for name in FILES}


def row_body(raw):
    marker = b'def controller.lifecycle_inbox.source_work.codec.write_candidate_record_using\n'
    body, = re.findall(rb'(^' + marker + rb'.*?)(?=\n\n(?:/--|end ))', raw, re.M | re.S)
    return body


def materialize(mode, capture, parent, reference, output, packages, mcp):
    own, bodies = captured(capture, mode)
    caller, parent_bodies = captured(parent, 'parent')
    correct = bodies if mode == 'core' else captured(reference, 'core')[1]
    original = row_body(correct['Funs.lean'])
    if original != row_body(parent_bodies['Funs.lean']):
        raise ValueError('actual concrete parent must use the proved row writer')
    if mode == 'zero-ok':
        if (original.count(b' != 1#usize') != 2
                or correct['Types.lean'] != bodies['Types.lean']
                or row_body(bodies['Funs.lean']) != original.replace(b' != 1#usize', b' > 1#usize')):
            raise ValueError('only the two generated copies of the same count condition may differ')

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
    with (output / 'lakefile.toml').open('x') as lake:
        aeneas, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', (ROOT / 'lakefile.toml').read_text())
        lake.write('name = "source-row-write"\n' + aeneas)
        for library in ('SandboxModel', NAMESPACE, 'RowWriteInputs', root):
            lake.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    (output / NAMESPACE).mkdir()
    for name, content in bodies.items():
        (output / NAMESPACE / name).write_bytes(content)
    for name in ('RowWriteInputs', root):
        shutil.copy2(HERE / (name + '.lean.in'), output / (name + '.lean'))
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = {AUDIT, 'RowWriteInputs.lean', root + '.lean', *(f'{NAMESPACE}/{name}' for name in FILES)}
    if files.keys() != expected:
        raise ValueError('unexpected actual row-writer Lean inputs')
    plan = dict(root=root, mode=mode, source=SOURCE, generated=own['generated_sha256'],
                parent=caller['generated_sha256'], files=files, strict_files=sorted(expected),
                modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('capture', 'parent', 'reference', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    materialize(args.mode, args.capture.resolve(strict=True), args.parent.resolve(strict=True),
                args.reference.resolve(strict=True), args.output.resolve(), args.lake_packages.resolve(strict=True),
                args.lean_mcp.resolve(strict=True))
