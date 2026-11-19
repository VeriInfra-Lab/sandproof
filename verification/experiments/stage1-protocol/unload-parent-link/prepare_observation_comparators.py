"""Join #963 and the original complete comparison proofs on one generated type view."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import shutil
import tarfile

import observation_comparators_capture as capture
import observation_slot_capture as slot
import prepare_observation_slot as prior
from prepare_retention_providers import FULL_WORK_MODULES, PREFIX, checked_template, declarations
from prepare_world_begin import closure, read_exact


ROOTS = {'core': 'ObservationComparators', 'skip-executable': 'ObservationComparatorsSkipExecutable'}
EXTERNAL = ('Bool.Insts.CoreCmpPartialEqBool.ne',
            'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
            'alloc.string.String.Insts.CoreCmpPartialEqString.eq')
WORK_MODULES = ('Spec', 'RequestEquality', 'Containers', 'ScalarEquality', 'Values', *FULL_WORK_MODULES)
CASES = 'ObservationComparatorInputs'


def verified(path, mode):
    report = json.loads((path / 'result.json').read_bytes())
    if (report['mode'], report['source_commit'], report['source_tree'], report['complete']) != (
            mode, slot.finish.SOURCE, slot.finish.TREE, True):
        raise ValueError('the completed exact comparison capture is required')
    read_exact(Path(capture.__file__), report['driver_sha256'])
    if set(report['templates']) != {str(p.relative_to(slot.ROOT)) for p in capture.templates()}:
        raise ValueError('the comparison capture input templates differ')
    for name, digest in report['templates'].items():
        read_exact(slot.ROOT / name, digest)
    archive = path / 'source.tar'
    read_exact(archive, slot.finish.ARCHIVE)
    staged = {str(name) for name in (capture.WORK, capture.LIB, capture.RAW)}
    if set(report['staged']) != staged:
        raise ValueError('the original staged input set differs')
    with tarfile.open(archive) as source:
        for name, digest in capture.INPUTS.items():
            with source.extractfile(name) as handle:
                original = handle.read()
            if sha256(original).hexdigest() != digest:
                raise ValueError('the original comparator input differs: ' + name)
            if name in staged:
                actual = read_exact(path / 'source' / name, report['staged'][name])
                expected = capture.transformed(Path(name), original.decode(), mode).encode()
                if actual != expected:
                    raise ValueError('the staged comparison input differs: ' + name)
            else:
                read_exact(path / 'source' / name, digest)
    if ([(row['label'], row['exit_code'], row['bounded_stop']) for row in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the original fixed commands must complete')
    for row in report['commands']:
        read_exact(path / (row['label'] + '.log'), row['log_sha256'])
    ir = path / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = path / 'generated'
    files = {*prior.BODIES, 'FunsExternal_Template.lean'}
    names = {'translation.json', *(f'{capture.NAMESPACE}/{name}' for name in files)}
    if ({str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names
            or set(report['generated_sha256']) != names):
        raise ValueError('the original comparator generated output inventory differs')
    target = json.loads((slot.ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if capture.admit(ir, generated, target) != report['admission']:
        raise ValueError('the actual comparator provider admission changed')
    content = {name: read_exact(generated / name, digest) for name, digest in report['generated_sha256'].items()}
    present = re.findall(rb'^axiom ([^ \n]+)', content[f'{capture.NAMESPACE}/FunsExternal_Template.lean'], re.M)
    if set(present) != {name.encode() for name in EXTERNAL} or len(present) != len(EXTERNAL):
        raise ValueError('only the existing admitted primitive comparison symbols are expected')
    return report, {name: content[f'{capture.NAMESPACE}/{name}'] for name in prior.BODIES}


def selected_name(path, namespace, identifier, *, external=False):
    tree = json.loads((path / 'generated/translation.json').read_bytes())
    row, = [row for row in tree['functions'] if row['def_id'] == identifier]
    name = row['lean_name']
    if row['is_opaque'] is not external or name.startswith(namespace + '.') == external:
        raise ValueError('the originally selected function must belong to the actual generated view')
    return name.removeprefix(namespace + '.')


def match_parent(parent, candidate, report, bodies):
    old, evidence = prior.captured(parent, 'parent')
    if any(prior.definitions(old[name], name) != prior.definitions(bodies[name], name) for name in prior.BODIES):
        raise ValueError('the actual #963 helper declarations and the comparator view differ')
    comparable = ('controller.lifecycle_inbox.source_work.SourceWork',
                  'controller.lifecycle_inbox.source_work.SourceWorkRecord',
                  'capture.source.RawSourceObservation')
    old_types = declarations(old['Types.lean'].removesuffix(b'\n\nend ' + slot.NAMESPACE.encode() + b'\n'))
    new_types = declarations(bodies['Types.lean'].removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))
    if any(old_types[name] != new_types[name] for name in comparable):
        raise ValueError('the original parent comparison carriers differ')
    ir = json.loads((parent / 'actual.llbc').read_bytes())['translated']
    fns = ir['fun_decls']
    function = slot.indexed(fns, evidence['admission']['parent'])
    bound = evidence['admission']['guard']
    call, = [entry['call'] for entry in slot.ir_values(function['body']['Structured']['body'], 'Call')
             if slot.call_kind(entry['call']).get('Fun') == bound]
    eq = slot.declaration(ir['trait_decls'], 'core::cmp::PartialEq')['def_id']
    raw = capture.compare_member(ir, capture.reference(ir, call, eq), eq, 0)
    expected = (evidence['admission']['comparison'], evidence['same_parent']['whole_work_eq'], raw['def_id'])
    actual = tuple(report['admission'][key] for key in ('work_adapter', 'work_eq', 'raw_eq'))
    names = [selected_name(parent, slot.NAMESPACE, identifier, external=external)
             for identifier, external in zip(expected, (False, True, True), strict=True)]
    if names != [selected_name(candidate, capture.NAMESPACE, identifier) for identifier in actual]:
        raise ValueError('the actual comparator dictionaries selected by #963 changed')
    return dict(parent=evidence, matched_functions=names, matched_carriers=list(comparable))


def prepare(captures, parent, output, packages, mcp, mode):
    candidate = captures / mode
    report, bodies = verified(candidate, mode)
    previous = match_parent(parent, candidate, report, bodies)
    if mode == 'skip-executable':
        core = verified(captures / 'core', 'core')[1]
        if core['Types.lean'] != bodies['Types.lean']:
            raise ValueError('the actual mutated observation has different comparison carrier types')
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', mcp)):
        path = output / name
        path.parent.mkdir()
        path.symlink_to(target, target_is_directory=True)
    for name in (*prior.SUPPORT, 'tools/lean_session.py', 'scripts/lean-mcp-server.sh',
                 'lake-manifest.json', 'lean-toolchain'):
        path = output / name
        path.parent.mkdir(exist_ok=True, parents=True)
        shutil.copy2(slot.ROOT / name, path)
    root = ROOTS[mode]
    with (output / 'lakefile.toml').open('x') as file:
        pinned = (slot.ROOT / 'lakefile.toml').read_text()
        dependency, = re.findall(r'\[\[require\]\]\nname = "aeneas"\n[^\n]+\n[^\n]+\n', pinned)
        file.write('name = "observation-comparators"\n' + dependency)
        for library in ('SandboxModel', capture.NAMESPACE, 'ObservationSlotInputs', 'ObservationSlot', CASES, root):
            file.write(f'\n[[lean_lib]]\nname = "{library}"\n')
    generated = output / capture.NAMESPACE
    generated.mkdir()
    for name, body in bodies.items():
        (generated / name).write_bytes(body)
    provider = 'source-work-match'
    inventory = json.loads((capture.PROVIDERS / provider / 'inventory.json').read_bytes())
    originals = {}
    for name in (*WORK_MODULES, 'FunsExternal'):
        document = checked_template(provider, name, inventory)
        content = document.replace('SandproofSourceWorkMatchRust', capture.NAMESPACE).replace(
            'SandproofSourceWorkMatch.', capture.NAMESPACE + '.')
        path = (generated if name == 'FunsExternal' else output / PREFIX / 'SourceWorkMatch') / (name + '.lean')
        path.parent.mkdir(exist_ok=True, parents=True)
        path.write_text(content)
        originals[str(path.relative_to(output))] = sha256(document.encode()).hexdigest()
    for name in ('ObservationSlotInputs', 'ObservationSlot'):
        text = (slot.HERE / (name + '.lean.in')).read_text()
        (output / (name + '.lean')).write_text(text.replace(slot.NAMESPACE, capture.NAMESPACE))
    for name in (CASES, root):
        shutil.copyfile(capture.HERE / (name + '.lean.in'), output / (name + '.lean'))
    files = closure(output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(originals) | set(prior.SUPPORT) | {f'{capture.NAMESPACE}/{name}' for name in prior.BODIES}
    expected |= {name + '.lean' for name in ('ObservationSlotInputs', 'ObservationSlot', CASES, root)}
    if files.keys() != expected:
        raise ValueError('unexpected transitive observation comparator proof inputs: ' + str(files.keys() ^ expected))
    plan = dict(root=root, mode=mode, source=slot.finish.SOURCE, prior_parent=previous,
                captured=report['generated_sha256'], reused=originals, files=files, strict_files=sorted(expected),
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', choices=ROOTS, required=True)
    for name in ('captures', 'parent', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    prepare(args.captures.resolve(strict=True), args.parent.resolve(strict=True), args.output.resolve(),
            args.lake_packages.resolve(strict=True), args.lean_mcp.resolve(strict=True), args.mode)
