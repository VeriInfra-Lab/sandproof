"""Prepare a private Lean view from the actual source observation slot captures."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import shutil
import tarfile

import observation_slot_capture as slot
from prepare_world_begin import closure, read_exact
from source_first_match import borrowed_paths


ROOTS = {'core': 'ObservationSlot', 'binding-only': 'ObservationSlotBindingOnly'}
SUPPORT = ('SandboxModel/ProofAuditCommand.lean', 'SandboxModel/Lifecycle/Protocol/ReplayContract.lean')
BODIES = ('Types.lean', 'Funs.lean')


def parent_relation(translated, admitted):
    functions = translated['fun_decls']
    raw = slot.indexed(functions, admitted['parent'])['body']['Structured']
    body = raw['body']
    types = translated['type_decls']
    variants = {row['def_id']: {item['id']: item['name'] for item in row['kind']['Enum']}
                for row in types if row and isinstance(row['kind'], dict) and 'Enum' in row['kind']}
    (lookup, _, lookup_guards), = slot.finish.routes(body, admitted['lookup'], variants)
    (guard, _, decision_guards), = slot.finish.routes(body, admitted['guard'], variants)
    (tail, steps, tail_guards), = slot.finish.routes(body, admitted['tail'], variants)
    local = slot.finish.local
    if ([branch for _, branch in lookup_guards] != [True, *(['Continue'] * 7), False]
            or decision_guards[:-1] != lookup_guards or decision_guards[-1][1] != 'Continue'
            or tail_guards != (*decision_guards, (local(guard['dest']), 'Fresh'))):
        raise ValueError('only the original successful prefix and a fresh selected slot may reach save')
    declarations = {row['name']: row['index'] for row in raw['locals']['locals'] if row['name']}
    tx, sources, inbox, work, observation, crashes, returned = (
        declarations[name] for name in ('tx', 'sources', 'inbox', 'work', 'observation', 'crashes', 'record'))
    aliases, calls, values = {}, [], {}
    for step in steps:
        if 'Assign' in step:
            destination, value = step['Assign']
            if 'Local' in destination['kind']:
                key = local(destination)
                operand = value['Use'][0] if 'Use' in value else {}
                source = value.get('Ref', {}).get('place') or operand.get('Move') or operand.get('Copy')
                if source is not None:
                    aliases[key] = source
                values[key] = value
        elif 'Call' in step:
            call = step['Call']['call']
            identifier = slot.call_kind(call).get('Fun')
            if identifier is not None:
                member = slot.indexed(functions, identifier)['item_meta']['name'][-1].get('Ident', [''])[0]
                calls.append((member, call))
                if member in ('deref', 'deref_mut'):
                    aliases[local(call['dest'])] = call['args'][0]['Move']
    lookup_id = local(lookup['dest'])
    to_result, = [call for member, call in calls if member == 'ok_or'
                  and call['args'][0] == {'Move': lookup['dest']}]
    branch, = [call for member, call in calls if member == 'branch'
               and call['args'] == [{'Move': to_result['dest']}]]
    index_gate = local(branch['dest'])
    index, = [call for member, call in calls if member == 'index_mut']
    selected = local(index['dest'])
    anchors = {tx, sources, inbox, work, observation, crashes, returned, lookup_id, index_gate, selected}

    def field(type_name, field_name):
        struct = slot.declaration(types, type_name)['kind']['Struct']
        number, = [i for i, entry in enumerate(struct) if entry['name'] == field_name]
        return number

    def path(place):
        return slot.finish.origin(place, aliases, anchors)

    def inputs(call):
        return [path(arg['Move']) for arg in call['args']]

    records = (sources, field(slot.WORK + '::codec::Sources', 'records'))
    saved = (selected, field(slot.WORK + '::SourceWorkRecord', 'observation'))
    if (decision_guards[-1][0] != index_gate or inputs(lookup) != [records, (work,)]
            or inputs(index) != [records, (index_gate, 0)] or inputs(guard) != [saved, (observation,)]):
        raise ValueError('the original complete scan index must select the saved and incoming observation')
    clone, = [call for member, call in calls if member == 'clone' and local(call['dest']) == returned]
    write, = [value for step in steps if 'Assign' in step for place, value in [step['Assign']]
              if 'Projection' in place['kind'] and path(place) == saved]
    tag, arguments = values[local(write['Use'][0]['Move'])]['Aggregate']
    option = slot.declaration(types, 'core::option::Option')['def_id']
    if (tag['Adt'][0]['id'] != option or variants[option][tag['Adt'][1]] != 'Some'
            or [path(arg['Move']) for arg in arguments] != [(observation,)]
            or inputs(clone) != [(selected,)]
            or inputs(tail) != [(tx,), (sources,), (inbox, 0, 3), (crashes,), (returned,)]
            or local(tail['dest']) != 0):
        raise ValueError('the fresh branch must update that selected slot and pass the original owner to save')
    comparison = slot.indexed(functions, admitted['comparison'])['body']['Structured']['body']
    member, = [entry['call'] for entry in slot.ir_values(comparison, 'Call')]
    equality = slot.indexed(functions, slot.call_kind(member)['Fun'])
    if (local(member['dest']) != 0 or equality['item_meta']['name'][-1] != {'Ident': ['eq', 0]}
            or borrowed_paths(comparison, member) != [
                (1, field(slot.WORK + '::SourceWorkRecord', 'work')), (2,)]):
        raise ValueError('the selected comparison must borrow the whole original SourceWork')
    return dict(records=records, original_work=work, selected_slot=selected, saved=saved,
                observation=observation, tail_guard='Fresh', whole_work_eq=equality['def_id'])


def captured(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    if (report['mode'], report['source_commit'], report['source_tree'], report['complete']) != (
            mode, slot.finish.SOURCE, slot.finish.TREE, True):
        raise ValueError('the completed exact observation slot source capture is required')
    read_exact(Path(slot.__file__), report['driver_sha256'])
    read_exact(slot.HERE / 'find.rs.in', report['candidate_sha256'])
    archive = path / 'source.tar'
    read_exact(archive, slot.finish.ARCHIVE)
    with tarfile.open(archive) as source:
        for name, sha in slot.finish.prepare.SOURCE_INPUTS.items():
            handle = source.extractfile(name)
            if handle is None:
                raise ValueError('missing fixed source input: ' + name)
            with handle:
                data = handle.read()
            if sha256(data).hexdigest() != sha:
                raise ValueError('fixed source input changed: ' + name)
            if name == slot.finish.prepare.SOURCE:
                expected = slot.factor(data.decode(), mode).encode()
                copy = read_exact(path / 'source' / name, report['candidate_source_sha256'])
                if expected != copy:
                    raise ValueError('captured source differs from the exact slot factorization')
            else:
                read_exact(path / 'source' / name, sha)
    if ([(row['label'], row['exit_code'], row['bounded_stop']) for row in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the original extraction commands must complete')
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = path / 'generated'
    environment = json.loads((slot.ROOT / 'verification/foundations-environment.json').read_bytes())
    names = {'translation.json', *(f'{slot.NAMESPACE}/{name}' for name in BODIES)}
    if mode == 'parent':
        names.update(f'{slot.NAMESPACE}/{name}External_Template.lean' for name in ('Types', 'Funs'))
    found = {str(file.relative_to(generated)): slot.digest(file) for file in generated.rglob('*') if file.is_file()}
    if (found.keys() != names or found != report['generated_sha256']
            or slot.admit(ir, generated, environment['target'], mode) != report['admission']):
        raise ValueError('the exact generated inventory and original admission are required')
    summary = dict(ir=report['ir_sha256'], original_driver=report['driver_sha256'],
                   generated=found, admission=report['admission'])
    if mode == 'parent':
        summary['same_parent'] = parent_relation(json.loads(ir.read_bytes())['translated'], report['admission'])
    return {name: read_exact(generated / f'{slot.NAMESPACE}/{name}', found[f'{slot.NAMESPACE}/{name}'])
            for name in BODIES}, summary


def definitions(data, kind):
    prefix = rb'controller\.lifecycle_inbox\.source_work\.'
    names = {'Funs.lean': rb'def ' + prefix + rb'observation_slot_(?:using(?:_loop(?:\.body)?)?|decision)',
             'Types.lean': rb'(?:structure|inductive) ' + prefix + rb'Observation(?:Work|SlotDecision)'}
    blocks = re.findall(rb'^(' + names[kind] + rb')(?=[ \n])([^\n]*\n.*?)(?=\n\n(?:/--|end ))',
                        data, re.M | re.S)
    if len(blocks) != (4 if kind == 'Funs.lean' else 2):
        raise ValueError('all actual observation slot declarations must occur once')
    return dict(blocks)


def materialize(captures, output, packages, mcp, mode):
    candidate, chosen = captured(captures / mode, mode)
    good = candidate if mode == 'core' else captured(captures / 'core', 'core')[0]
    caller, actual = captured(captures / 'parent', 'parent')
    if any(definitions(caller[name], name) != definitions(good[name], name) for name in BODIES):
        raise ValueError('the actual parent and isolated positive slot definitions differ')
    if mode == 'binding-only':
        expected = good['Funs.lean'].replace(b'ObservationWorkInst.same_work t work',
                                             b'ObservationWorkInst.same_binding t work')
        if candidate['Types.lean'] != good['Types.lean'] or candidate['Funs.lean'] != expected:
            raise ValueError('the generated negative must change only the original comparison')
    root = ROOTS[mode]
    output.mkdir(mode=0o700)
    for path, link in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        target = output / path
        target.parent.mkdir()
        target.symlink_to(link, target_is_directory=True)
    for name in (*SUPPORT, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh', 'lean-toolchain', 'lake-manifest.json'):
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(slot.ROOT / name, target)
    with (output / 'lakefile.toml').open('x') as lake:
        lake.write('name = "source-observation-slot"\n')
        pinned = (slot.ROOT / 'lakefile.toml').read_text()
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', pinned)
        lake.write(dependency)
        for library in ('SandboxModel', slot.NAMESPACE, 'ObservationSlotInputs', root):
            lake.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    (output / slot.NAMESPACE).mkdir()
    for name, data in candidate.items():
        (output / slot.NAMESPACE / name).write_bytes(data)
    for module in ('ObservationSlotInputs', root):
        shutil.copyfile(slot.HERE / f'{module}.lean.in', output / f'{module}.lean')
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(SUPPORT) | {f'{slot.NAMESPACE}/{name}' for name in BODIES} | {
        'ObservationSlotInputs.lean', root + '.lean'}
    if files.keys() != expected:
        raise ValueError('the actual observation slot Lean inputs differ')
    plan = dict(root=root, mode=mode, source=slot.finish.SOURCE, capture=chosen, parent=actual,
                files=files, strict_files=sorted(expected),
                modules={name: {'sha256': sha} for name, sha in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('captures', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    materialize(args.captures.resolve(strict=True), args.output.resolve(),
                args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True), args.mode)
