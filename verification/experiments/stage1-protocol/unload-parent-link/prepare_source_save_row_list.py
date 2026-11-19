"""Bind the unchanged save and Vec loop to the actual single-row SQL view."""
import argparse
from hashlib import sha256
import json
from pathlib import Path

import prepare_source_row_sql_composition as sql
import source_save_row_list_capture as capture
from prepare_observation_suffix import replace_once
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('save-row-list')
PREFIX = 'controller.lifecycle_inbox.source_work.codec.'
GENERATED = 'SaveRowsGenerated'
ROOT = 'SaveRows'


def generated(directory):
    receipt = json.loads((directory / 'result.json').read_bytes())
    if (receipt['source'], receipt['tree'], receipt['driver'], receipt['options'],
            receipt['complete']) != (capture.SOURCE, capture.TREE,
            capture.row.digest(Path(capture.__file__)), capture.CFG, True):
        raise ValueError('expected the exact completed current capture')
    if [(step['label'], step['exit_code'], step['bounded_stop']) for step in receipt['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('the original capture tools must finish')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    read_exact(directory / 'source' / sql.CODEC, capture.STAGED)
    for step in receipt['commands']:
        read_exact(directory / (step['label'] + '.log'), step['log_sha256'])
    ir = directory / 'actual.llbc'
    read_exact(ir, receipt['ir_sha256'])
    output = directory / 'generated'
    files = {'translation.json', *(f'{capture.NAMESPACE}/{part}.lean' for part in (
        'Funs', 'Types', 'FunsExternal_Template', 'TypesExternal_Template'))}
    if ({str(p.relative_to(output)) for p in output.rglob('*') if p.is_file()} != files
            or receipt['generated_sha256'].keys() != files):
        raise ValueError('actual save generation inventory changed')
    content = {n: read_exact(output / n, receipt['generated_sha256'][n]) for n in files}
    target = json.loads((sql.ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if capture.admit(ir, output, target) != receipt['admission']:
        raise ValueError('the original save capture admission changed')
    return {f: content[f'{capture.NAMESPACE}/{f}'] for f in ('Funs.lean', 'Types.lean')}, receipt


def definitions(files, namespace):
    return {n: declarations(body.removesuffix(b'\n\nend ' + namespace.encode() + b'\n'))
            for n, body in files.items()}


def fragments(current, original):
    sections = definitions(current, capture.NAMESPACE)
    prior = definitions(original, sql.NAMESPACE)
    for key, old in prior.items():
        if any(sections[key].get(name) != data for name, data in old.items()):
            raise ValueError('the complete earlier single-row SQL declarations differ: ' + key)
    types, functions = sections['Types.lean'], sections['Funs.lean']
    instance, = (name for name in functions if name.startswith(PREFIX + 'JournalSourceSave.Insts.')
                 and name.rsplit('.', 1)[-1].startswith('Sandproof_controller_service'))
    prelude = [types[PREFIX + part] for part in ('Sources', 'SourceSaveEffects', 'JournalSourceSave')]
    prelude += [functions[PREFIX + part] for part in ('write_candidate_records_loop.body',
                 'write_candidate_records_loop', 'write_candidate_records', 'save_using')]
    adapters = []
    for part in ('read', 'limit', 'encode', 'retains', 'write'):
        name = f'{instance}.{part}'
        adapters.append(functions[name])
        if part != 'write':
            adapters.append(f'local notation "{name}" => {name} inputs'.encode())
    adapters.extend((functions[instance], f'abbrev saveEffects := {instance} inputs'.encode(),
                     f'local notation "{instance}" => saveEffects inputs'.encode(), functions[PREFIX + 'save']))
    return {b'-- GENERATED TYPES AND LOOP': b'\n\n'.join(prelude),
            b'-- GENERATED CONCRETE SAVE': b'\n\n'.join(adapters)}


def prepare(current, prior, output, packages, service):
    data, receipt = generated(current)
    old = sql.captured(prior, 'core')
    blocks = fragments(data, old)
    sql.materialize('core', prior, prior, output, packages, service)
    plan_path = output / 'plan.json'
    plan = json.loads(plan_path.read_bytes())
    template = (HERE / f'{GENERATED}.lean.in').read_bytes()
    for marker, block in blocks.items():
        template = replace_once(template, marker, block)
    (output / f'{GENERATED}.lean').write_bytes(template)
    (output / f'{ROOT}.lean').write_bytes((HERE / f'{ROOT}.lean.in').read_bytes())
    with (output / 'lakefile.toml').open('a') as file:
        for name in (GENERATED, ROOT):
            file.write(f'\n[[lean_lib]]\nname = "{name}"\n')
    files = closure(output, [ROOT + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(plan['files']) | {ROOT + '.lean', GENERATED + '.lean'}:
        raise ValueError('the original save source closure differs')
    prior_plan = plan.copy()
    plan.update(root=ROOT, source=capture.SOURCE, original_row=prior_plan,
                current=receipt, files=files, strict_files=sorted(files),
                blocks={name.decode(): sha256(body).hexdigest() for name, body in blocks.items()},
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    plan_path.write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('current', 'prior', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(args.current.resolve(strict=True), args.prior.resolve(strict=True), args.output.resolve(),
            args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True))
