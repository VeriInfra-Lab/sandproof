"""Materialize the complete retained source-observation parent declaration."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import shutil

import prepare_observation_suffix as suffix
from prepare_observation_slot import captured, definitions
from prepare_retention_providers import declarations
from prepare_world_begin import closure


HERE = Path(__file__).with_name('observation-parent')
ROOT = 'ObservationParent'
GENERATED = 'ObservationParentGenerated'
CURRENT = suffix.PREFIX + 'snapshot.CurrentSource'


def actual_declarations(files):
    end = b'\n\nend ' + suffix.slot.NAMESPACE.encode() + b'\n'
    return {name: declarations(body.removesuffix(end)) for name, body in files.items()}


def prepare(original, prefix, scan, tail, output, packages, service):
    parent, receipt = captured(original, 'parent')
    core, core_receipt = captured(scan, 'core')
    if any(definitions(parent[name], name) != definitions(core[name], name) for name in parent):
        raise ValueError('the complete original caller must use the same retained scan and decision')
    exact = actual_declarations(parent)
    prefix_report, prefix_bodies = suffix.verified(prefix, 'parent')
    prior = {name: suffix.generated_declarations(body) for name, body in prefix_bodies.items()}
    if any(exact['Types.lean'][name] != prior['Types.lean'][name] for name, _ in suffix.TYPES):
        raise ValueError('the original parent and continuation must have the same complete carrier declarations')
    members = (suffix.FINISH, suffix.FINISH_SOURCE, suffix.SCAN_DICTIONARY,
               suffix.SCAN_DICTIONARY + '.same_work', suffix.SAVE_DICTIONARY,
               suffix.SAVE_DICTIONARY + '.save')
    if any(exact['Funs.lean'][name] != prior['Funs.lean'][name] for name in members):
        raise ValueError('the original parent and continuation must have the same actual helpers')
    parent_def = exact['Funs.lean'][suffix.PARENT]
    begin = b'  let (r, c) \xe2\x86\x90\n      controller.transaction.WorldTransaction.begin'
    if parent_def.count(begin) != 1:
        raise ValueError('the original direct Begin with connection writeback is required')
    blocks = {
        b'-- GENERATED CURRENT SOURCE': exact['Types.lean'][CURRENT],
        b'-- GENERATED WHOLE PARENT': parent_def,
    }

    suffix.prepare(prefix, scan, tail, output, packages, service)
    plan_path = output / 'plan.json'
    plan = json.loads(plan_path.read_bytes())
    if plan['parent'] != prefix_report['generated_sha256'] or plan['scan'] != core_receipt:
        raise ValueError('the retained inputs changed while materializing the continuation')
    document = (HERE / (GENERATED + '.lean.in')).read_bytes()
    for marker, block in blocks.items():
        document = suffix.replace_once(document, marker, block)
    (output / (GENERATED + '.lean')).write_bytes(document)
    shutil.copyfile(HERE / (ROOT + '.lean.in'), output / (ROOT + '.lean'))
    with (output / 'lakefile.toml').open('a') as file:
        for module in (GENERATED, ROOT):
            file.write(f'\n[[lean_lib]]\nname = "{module}"\n')
    files = closure(output, [ROOT + '.lean'], active_view='lakefile.toml')
    expected = set(plan['files']) | {ROOT + '.lean', GENERATED + '.lean'}
    if files.keys() != expected:
        raise ValueError('the original parent source closure changed')
    plan.update(root=ROOT, direct_parent=receipt, direct_blocks={
        name.decode(): sha256(body).hexdigest() for name, body in blocks.items()},
        prefix_parent=prefix_report['ir_sha256'], files=files, strict_files=sorted(expected),
        modules={name: {'sha256': digest} for name, digest in files.items()},
        build_order=[name.removesuffix('.lean') for name in files])
    plan_path.write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('original', 'prefix', 'scan', 'tail', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(args.original.resolve(strict=True), args.prefix.resolve(strict=True),
            args.scan.resolve(strict=True), args.tail.resolve(strict=True), args.output.resolve(),
            args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
