"""Verify the actual observation-save prefix and reuse the original decoded guard."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import shutil
import tarfile

from observation_comparators_capture import compare_member, reference
import observation_prefix_capture as capture
from prepare_retention_providers import PREFIX, checked_template, declarations
from prepare_world_begin import closure, read_exact


SLOT = capture.slot
ROOTS = {'core': 'ObservationPrefix', 'begin-first': 'ObservationPrefixBeginFirst'}
GUARD = ('Spec', 'Equality', 'SourceEquality', 'Core')
BODIES = ('Types.lean', 'Funs.lean')
AUDIT = 'SandboxModel/ProofAuditCommand.lean'
REPLAY = 'SandboxModel/Lifecycle/Protocol/ReplayContract.lean'
EXTERNAL = {b'alloc.string.String.Insts.CoreCmpPartialEqString.eq', b'alloc.vec.Vec.is_empty'}


def verified(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    if (report['mode'], report['source'], report['tree'], report['complete']) != (
            mode, SLOT.finish.SOURCE, SLOT.finish.TREE, True):
        raise ValueError('the completed exact observation prefix capture is required')
    read_exact(Path(capture.__file__), report['driver_sha256'])
    read_exact(capture.HERE / 'prefix.rs.in', report['template_sha256'])
    probe = read_exact(capture.GUARD / 'probe.rs.in', report['probe_sha256'])
    archive = path / 'source.tar'
    read_exact(archive, SLOT.finish.ARCHIVE)
    staged = {str(name) for name in (capture.WORK, capture.LIB)}
    if set(report['staged']) != staged:
        raise ValueError('the actual staged prefix file set differs')
    with tarfile.open(archive) as source:
        for name, digest in capture.INPUTS.items():
            with source.extractfile(name) as handle:
                original = handle.read()
            if sha256(original).hexdigest() != digest:
                raise ValueError('the original observation input differs: ' + name)
            if name not in staged:
                read_exact(path / 'source' / name, digest)
                continue
            expected = (original + probe if name == str(capture.LIB)
                        else capture.factor(original.decode(), mode).encode())
            if read_exact(path / 'source' / name, report['staged'][name]) != expected:
                raise ValueError('the actual factored observation input differs: ' + name)
    if ([(row['label'], row['exit_code'], row['bounded_stop']) for row in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the original capture tools must complete')
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    read_exact(path / 'actual.llbc', report['ir_sha256'])
    generated = path / 'generated'
    files = {*BODIES, 'FunsExternal_Template.lean'}
    if mode == 'parent':
        files.add('TypesExternal_Template.lean')
    names = {'translation.json', *(f'{capture.NAMESPACE}/{name}' for name in files)}
    if ({str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names
            or set(report['generated_sha256']) != names):
        raise ValueError('the original observation generated output inventory differs')
    target = json.loads((SLOT.ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if capture.admit(path / 'actual.llbc', generated, mode, target) != report['admission']:
        raise ValueError('the original observation prefix admission changed')
    content = {name: read_exact(generated / name, digest) for name, digest in report['generated_sha256'].items()}
    if mode != 'parent':
        template = content[f'{capture.NAMESPACE}/FunsExternal_Template.lean']
        symbols = re.findall(rb'^axiom ([^ \n]+)', template, re.M)
        if set(symbols) != EXTERNAL or len(symbols) != len(EXTERNAL):
            raise ValueError('only the original two admitted primitive guard operations are expected')
    return report, {name: content[f'{capture.NAMESPACE}/{name}'] for name in BODIES}


def generated_declarations(data):
    return declarations(data.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))


def match_parent(parent, candidate):
    prefix = 'controller.lifecycle_inbox.source_work.'
    select = {'Funs.lean': ('source_observation_prefix',),
              'Types.lean': ('SourceObservationPrefix', 'ObservationPrefixErrors')}
    for file, names in select.items():
        old = generated_declarations(parent[file])
        new = generated_declarations(candidate[file])
        if any(old[prefix + name] != new[prefix + name] for name in names):
            raise ValueError('the positive prefix definition or its carrier differs in the actual parent')


def parent_relation(ir, admitted):
    translated = json.loads(ir.read_bytes())['translated']
    functions, types, traits = (translated[key] for key in ('fun_decls', 'type_decls', 'trait_decls'))
    raw = SLOT.indexed(functions, admitted['parent'])['body']['Structured']
    body = raw['body']
    local, origin = SLOT.finish.local, SLOT.finish.origin
    variants = {row['def_id']: {value['id']: value['name'] for value in row['kind']['Enum']}
                for row in types if row and isinstance(row['kind'], dict) and 'Enum' in row['kind']}

    def id_of(call):
        return SLOT.call_kind(call)['Fun']

    def member(call):
        return SLOT.indexed(functions, id_of(call))['item_meta']['name'][-1]['Ident'][0]

    def field(name, label):
        fields = SLOT.declaration(types, name)['kind']['Struct']
        index, = (index for index, value in enumerate(fields) if value['name'] == label)
        return index

    def flow(steps, anchors):
        aliases, values, calls = {}, {}, []
        for step in steps:
            if 'Assign' in step:
                place, value = step['Assign']
                if 'Local' not in place['kind']:
                    continue
                operand = value.get('Use', [{}])[0]
                source = value.get('Ref', {}).get('place') or operand.get('Move') or operand.get('Copy')
                if source is not None:
                    aliases[local(place)] = source
                values[local(place)] = value
            if 'Call' in step and 'Fun' in SLOT.call_kind(step['Call']['call']):
                call = step['Call']['call']
                calls.append(call)
                if member(call) in ('deref', 'deref_mut'):
                    aliases[local(call['dest'])] = call['args'][0]['Move']

        def paths(arguments):
            return [origin(value.get('Move') or value['Copy'], aliases, anchors) for value in arguments]

        return paths, values, calls

    def route(name):
        identifier = SLOT.declaration(functions, name)['def_id']
        found, = SLOT.finish.routes(body, identifier, variants)
        return found

    prefix, before, guards = route(capture.ROOT)
    lookup, _, selected = route(SLOT.FINDER)
    tail, steps, saved = route(SLOT.finish.OBSERVE)
    guard, _, checking = route(SLOT.DECISION)
    gate, = (position for position, branch in selected if branch == 'Continue')
    if (guards or selected != ((gate, 'Continue'),) or checking[:-1] != selected
            or checking[-1][1] != 'Continue' or saved != (*checking, (local(guard['dest']), 'Fresh'))):
        raise ValueError('only a successful prefix may search and only a fresh row may reach save')
    named = {row['name']: row['index'] for row in raw['locals']['locals'] if row['name']}
    self, work, context, observation, time, crashes, returned = (
        named[name] for name in ('self', 'work', 'context', 'observation', 'now_ms', 'crashes', 'record'))
    paths, values, calls = flow(steps, {self, work, context, observation, time, crashes, returned, gate})
    branched, = (call for call in calls if local(call['dest']) == gate)
    if (member(branched) != 'branch' or branched['args'] != [{'Move': prefix['dest']}]
            or any('Call' in step for step in before[:before.index(
                next(step for step in before if step.get('Call', {}).get('call') == prefix))])):
        raise ValueError('the original first operation must branch on the prefix result')
    input_paths = paths(prefix['args'])
    start, error = input_paths[0], input_paths[-1]
    controller = SLOT.finish.prepare.CONTROLLER + '::ControllerJournal'
    crate = SLOT.finish.prepare.CRATE
    journal = field(controller, 'journal')
    connection = field(crate + '::journal::Journal', 'connection')
    committed = field(controller, 'committed_world')
    if (input_paths[1:7] != [
            (self, field(controller, 'namespace')), (self, field(controller, 'active_host_epoch')),
            (work,), (context,), (observation,), (time,)]
            or paths(values[start[0]]['Aggregate'][1]) != [(self, journal, connection), (self, committed)]):
        raise ValueError('the prefix must receive the same connection, cached World and original request operands')
    errors_type = SLOT.declaration(types, capture.ERRORS)['def_id']
    journal_error = SLOT.declaration(types, crate + '::JournalError')['def_id']
    error_tag, error_arguments = values[error[0]]['Aggregate']
    tags = [values[path[0]]['Aggregate'][0]['Adt'] for path in paths(error_arguments)]
    if (error_tag['Adt'][0]['id'] != errors_type or any(tag[0]['id'] != journal_error for tag in tags)
            or [variants[journal_error][tag[1]] for tag in tags] != [
                'InvalidHostEvidence', 'UnsupportedStore', 'UnsupportedStore', 'Conflict']):
        raise ValueError('the parent must pass the exact original error constructors')
    record_field = field(SLOT.WORK + '::codec::Sources', 'records')
    inbox_type = SLOT.finish.prepare.INBOX + '::model::Inbox'
    inbox_config = field(inbox_type, 'config')
    inbox_capacity = field(SLOT.finish.prepare.INBOX + '::model::LifecycleInboxConfig', 'max_sessions')
    if (paths(lookup['args']) != [(gate, 0, 2, record_field), (work,)]
            or paths(tail['args']) != [(gate, 0, 0), (gate, 0, 2),
                                        (gate, 0, 1, inbox_config, inbox_capacity), (crashes,), (returned,)]):
        raise ValueError('the same return transaction, inbox, and loaded records must reach scan and save')

    anchor = SLOT.finish.prepare
    eq_trait = SLOT.declaration(traits, 'core::cmp::PartialEq')['def_id']
    prefix_eq = compare_member(translated, reference(translated, prefix, eq_trait), eq_trait, 0)['def_id']
    scan_trait = SLOT.declaration(traits, SLOT.TRAIT)['def_id']
    scan_member = compare_member(translated, reference(translated, lookup, scan_trait), scan_trait, 0)
    scan_eq, = SLOT.regular_calls(scan_member['body']['Structured']['body'])
    if scan_eq != prefix_eq:
        raise ValueError('the current source and the record scan must use the same complete work equality')
    trait = SLOT.declaration(traits, capture.TRAIT)['def_id']
    dictionary = reference(translated, prefix, trait)
    requirements = anchor.method(functions, SLOT.WORK, 'requirement')['def_id']
    request = anchor.method(functions, SLOT.WORK, 'request')['def_id']
    matches = anchor.method(functions, crate + '::capture::source', 'matches')['def_id']
    begin = anchor.method(functions, anchor.TRANSACTION, 'begin')['def_id']
    world = anchor.method(functions, anchor.TRANSACTION, 'world')['def_id']
    inbox = SLOT.declaration(functions, anchor.INBOX + '::codec::load')['def_id']
    auth = SLOT.declaration(functions, anchor.INBOX + '::authenticated')['def_id']
    sources = SLOT.declaration(functions, SLOT.WORK + '::codec::load')['def_id']
    current = SLOT.declaration(functions, SLOT.WORK + '::snapshot::current')['def_id']
    adapter_arity = (2, 1, 1, 3, 1, 7)
    port_names = ('matches', 'begin', 'inbox', 'authenticated', 'sources', 'current')
    actual_ports = {}
    operations = ((requirements, matches), (begin,), (inbox,), (request, auth), (sources,), (request, world, current))
    for index, expected in enumerate(operations):
        adapter = compare_member(translated, dictionary, trait, index)
        adapter_body = adapter['body']['Structured']['body']
        operations_path, arguments, invoked = flow(
            [row['kind'] for row in adapter_body['statements'] if isinstance(row['kind'], dict)],
            set(range(1, adapter_arity[index] + 1)))
        relevant = [call for call in invoked if member(call) not in ('deref', 'as_ref')]
        if tuple(map(id_of, relevant)) != expected or local(relevant[-1]['dest']) != 0:
            raise ValueError('the selected adapter changed the named operation or returned an unrelated result')
        read = local(relevant[0]['dest'])
        paths_expected = ([(1,), (read,)], [(1, 0), (1, 1)], [(1,)],
                          [(1,), (read,), (3,)], [(1,)])
        sent = operations_path(relevant[-1]['args'])
        if index < 5:
            if sent != paths_expected[index] or (index in (0, 3)
                    and operations_path(relevant[0]['args']) != [(2,)]):
                raise ValueError('the selected adapter changed the original comparison, owner or context operands')
        else:
            view, = (call for call in invoked if member(call) == 'as_ref')
            current_input = sent[-1][0]
            config = field(SLOT.WORK + '::codec::Sources', 'config')
            if (sent[:-1] != [(1,), (2,), (read,), (4,)]
                    or operations_path(relevant[0]['args']) != [(3,)]
                    or operations_path(relevant[1]['args']) != [(1,)]
                    or operations_path(view['args']) != [(7,)]
                    or operations_path(arguments[current_input]['Aggregate'][1]) != [
                        (local(relevant[1]['dest']),), (5,), (6, config), (local(view['dest']),)]):
                raise ValueError('current must receive the same transaction and exact snapshot inputs')
        actual_ports[port_names[index]] = expected[-1]
    return dict(original_parent=admitted['parent'], original_prefix=admitted['prefix'],
                result_gate=gate, complete_work_eq=prefix_eq, ports=actual_ports,
                error_constructors=[variants[journal_error][tag[1]] for tag in tags])


def prepare(captures, output, packages, mcp, mode):
    report, bodies = verified(captures / mode, mode)
    parent, original = verified(captures / 'parent', 'parent')
    positive = bodies if mode == 'core' else verified(captures / 'core', 'core')[1]
    match_parent(original, positive)
    relation = parent_relation(captures / 'parent/actual.llbc', parent['admission'])
    if mode == 'begin-first':
        good = generated_declarations(positive['Funs.lean'])
        modified = generated_declarations(bodies['Funs.lean'])
        changed = [name for name in good if good[name] != modified.get(name)]
        if (bodies['Types.lean'] != positive['Types.lean'] or good.keys() != modified.keys()
                or changed != ['controller.lifecycle_inbox.source_work.source_observation_prefix']):
            raise ValueError('the wrong view may move only Begin in the original prefix')
    output.mkdir(mode=0o700)
    for name, dependency in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        path = output / name
        path.parent.mkdir()
        path.symlink_to(dependency, target_is_directory=True)
    support = (AUDIT, REPLAY) if mode == 'core' else (AUDIT,)
    for name in (*support, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh', 'lake-manifest.json', 'lean-toolchain'):
        path = output / name
        path.parent.mkdir(exist_ok=True, parents=True)
        shutil.copy2(SLOT.ROOT / name, path)
    root = ROOTS[mode]
    with (output / 'lakefile.toml').open('x') as file:
        pinned = (SLOT.ROOT / 'lakefile.toml').read_text()
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', pinned)
        file.write('name = "source-observation-first-error"\n' + dependency)
        for library in ('SandboxModel', capture.NAMESPACE, 'ObservationPrefixInputs', root):
            file.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    generated = output / capture.NAMESPACE
    generated.mkdir()
    for name, body in bodies.items():
        (generated / name).write_bytes(body)
    originals = {}
    inventory = json.loads((capture.GUARD / 'inventory.json').read_bytes())
    for name in (*GUARD, 'FunsExternal'):
        original = checked_template('source-observation', name, inventory)
        content = original.replace('SandproofSourceObservationRust', capture.NAMESPACE).replace(
            'SandproofSourceObservation.', capture.NAMESPACE + '.')
        path = (generated if name == 'FunsExternal' else output / PREFIX / 'SourceObservation') / (name + '.lean')
        path.parent.mkdir(exist_ok=True, parents=True)
        path.write_text(content)
        originals[str(path.relative_to(output))] = sha256(original.encode()).hexdigest()
    for name in ('ObservationPrefixInputs', root):
        shutil.copyfile(capture.HERE / (name + '.lean.in'), output / (name + '.lean'))
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(originals) | {*support, *(f'{capture.NAMESPACE}/{name}' for name in BODIES),
                                 'ObservationPrefixInputs.lean', root + '.lean'}
    if files.keys() != expected:
        raise ValueError('unexpected transitive source guard proof inputs: ' + str(files.keys() ^ expected))
    plan = dict(root=root, mode=mode, source=SLOT.finish.SOURCE, parent=parent['generated_sha256'], relation=relation,
                captured=report['generated_sha256'], reused=originals, files=files, strict_files=sorted(expected),
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('captures', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    prepare(args.captures.resolve(strict=True), args.output.resolve(),
            args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True), args.mode)
