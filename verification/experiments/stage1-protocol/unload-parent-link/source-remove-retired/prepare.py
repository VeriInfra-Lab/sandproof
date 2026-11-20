"""Materialize both complete retirement views from exact current captures."""
import argparse
import json
from pathlib import Path
import re

import extract
import inputs
from prepare_world_begin import closure

HERE = Path(__file__).resolve().parent
CORE = ('SourceRetireTypes', 'SourceRetireEqualityView', 'SourceRetireEquality',
        'SourceRetireLibrary', 'SourceRetireView', 'SourceRetireSpec', 'SourceRetireCases')
MARKERS = {
    'SourceRetireTypes': b'-- ACTUAL ADDITIONAL TYPES\n',
    'SourceRetireEqualityView': b'-- ACTUAL COMPLETE RECORD COMPARISONS\n',
    'SourceRetireView': b'-- ACTUAL REMAINING FUNCTIONS\n',
    'SourceRetireWrongView': b'-- ACTUAL WRONG CLEAR ROOT\n',
}


def inserted(name, body):
    template = (HERE / (name + '.lean.in')).read_bytes()
    marker = MARKERS[name]
    if template.count(marker) != 1:
        raise ValueError('one complete actual-declaration insertion is required: ' + name)
    return template.replace(marker, body + b'\n')


def views(reference, actual, selected, wrong):
    data = selected['data']
    generated = {}
    for module, group in (('SourceRetireTypes', 'types'), ('SourceRetireEqualityView', 'equalities')):
        generated[module] = inserted(module, b'\n\n'.join(
            inputs.declaration(reference, name, data[name]) for name in selected[group]))
    template = (HERE / 'SourceRetireView.lean.in').read_bytes()
    ports = set(re.findall(rb'local notation "([^"]+)" => io\.\w+', template))
    models = set(re.findall(rb'local notation "([^"]+)" => SourceRetireLibrary\.\w+', template))
    known = {n.decode() for n in ports | models} | {
        'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
        'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
        'core.option.Option.is_some_and', 'controller.lifecycle_inbox.sha256.digest'}
    if set(selected['libraries']) != known or len(ports) != 13 or len(models) != 19:
        raise ValueError('all exact typed ports and registered library interfaces are required')
    dependent = {n.decode() for n in ports}
    bodies = []
    for name in selected['functions']:
        body = inputs.declaration(reference, name, data[name])
        if any(n in dependent for n in selected['pattern'].findall(data[name].decode())):
            dependent.add(name)
        bodies.append(body)
        if name == 'capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES':
            bodies.append(('local notation "' + name + '" => _root_.SandboxModel.Stage1Protocol.'
                           'SourceRetireActual.' + name).encode())
        if name in dependent:
            bodies.append(('local notation "' + name + '" => ' + name + ' io').encode())
    generated['SourceRetireView'] = inserted('SourceRetireView', b'\n\n'.join(bodies))
    for name in CORE:
        if name not in generated:
            generated[name] = (HERE / (name + '.lean.in')).read_bytes()
    if wrong:
        body = inputs.parsed(actual['Funs'], extract.NAMESPACE)[inputs.ROOT]
        generated['SourceRetireWrongView'] = inserted('SourceRetireWrongView', body)
        generated['SourceRetireNegative'] = (HERE / 'SourceRetireNegative.lean.in').read_bytes()
    return generated


def prepare(args):
    original, reference = inputs.capture(args.reference, 'original')
    wrong = args.variant == 'allow-zero-update'
    if wrong:
        selected_capture, actual = inputs.capture(args.actual, args.variant)
    else:
        if args.actual != args.reference:
            raise ValueError('the correct project must use the single original capture')
        selected_capture, actual = original, reference
    for name in ('Types', 'External'):
        if actual[name] != reference[name]:
            raise ValueError('all actual types and library declarations must remain identical')
    original_funs = inputs.parsed(reference['Funs'], extract.NAMESPACE)
    actual_funs = inputs.parsed(actual['Funs'], extract.NAMESPACE)
    if original_funs.keys() != actual_funs.keys():
        raise ValueError('the complete generated function inventory must remain identical')
    changed = {n for n in original_funs if original_funs[n] != actual_funs[n]}
    if changed != ({inputs.ROOT} if wrong else set()):
        raise ValueError('only the actual exact-row-count check may change')
    if wrong:
        correct = original_funs[inputs.ROOT]
        if (correct.count(b'if val5 != 1#usize') != 4
                or correct.replace(b'if val5 != 1#usize', b'if val5 > 1#usize') != actual_funs[inputs.ROOT]):
            raise ValueError('the four expansions of the sole zero-row mutation are required')
    reads, reads_evidence = inputs.prior(args.reads, args.reads_audit, 'SourceInitReads', 99, 483)
    retention, retention_evidence = inputs.prior(
        args.retention, args.retention_audit, 'RetentionProviders', 26, 127)
    selected = inputs.select(reference, args.retention)
    inherited = dict(reads['files'])
    for name, wanted in retention['files'].items():
        if name in inherited and inherited[name] != wanted:
            raise ValueError('the original inherited projects must agree on every shared file')
        inherited[name] = wanted
    if len(inherited) != 123:
        raise ValueError('all original physical files from both complete projects are required')
    args.output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        link = args.output / name
        link.parent.mkdir(exist_ok=True)
        link.symlink_to(target, target_is_directory=True)
    for project, plan in ((args.reads, reads), (args.retention, retention)):
        for name, wanted in plan['files'].items():
            destination = args.output / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((project / name).read_bytes())
            if inputs.digest(destination) != wanted:
                raise ValueError('inherited source changed during copying: ' + name)
    for name in inputs.SUPPORT:
        body = (extract.REPO / name).read_bytes()
        if body != (args.reads / name).read_bytes() or body != (args.retention / name).read_bytes():
            raise ValueError('the original common fixed support is required: ' + name)
        destination = args.output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(body)
    modules = views(reference, actual, selected, wrong)
    lake = (args.reads / 'lakefile.toml').read_text()
    for name in ('SandproofSourceRetentionProviders', 'FirstMatchInputs', 'RetentionProviderCases',
                 'RetentionProviders', 'SourceFirstMatch', *modules):
        if f'name = "{name}"' not in lake:
            lake += f'\n[[lean_lib]]\nname = "{name}"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    for name, body in modules.items():
        (args.output / (name + '.lean')).write_bytes(body)
    root = 'SourceRetireNegative' if wrong else 'SourceRetireCases'
    files = closure(args.output, [root + '.lean'], active_view='lakefile.toml')
    if (set(files) != set(inherited) | {name + '.lean' for name in modules}
            or any(files[n] != wanted for n, wanted in inherited.items())):
        raise ValueError('all inherited/current actual files, proofs and cases must be in the closure')
    proofs = []
    for name, wanted in files.items():
        path = args.output / name
        if inputs.digest(path) != wanted:
            raise ValueError('current physical source changed during materialization')
        proofs.extend(re.findall(r'^#audit_controller_proof (\S+)$', path.read_text(), re.M))
    if len(proofs) != len(set(proofs)):
        raise ValueError('the complete audit inventory cannot contain duplicates')
    plan = dict(root=root, variant=args.variant, source=extract.SOURCE, tree=extract.TREE,
        reference=original, actual=selected_capture,
        parents=dict(reads=reads_evidence, retention=retention_evidence),
        shared=selected['shared'], reused=selected['reused'], libraries=selected['libraries'],
        declarations={k: selected[k] for k in ('types', 'equalities', 'functions')},
        templates={str(path.relative_to(extract.REPO)): inputs.digest(path)
                   for path in sorted(HERE.iterdir()) if path.is_file()},
        files=files, strict_files=sorted(files), audits=proofs,
        modules={n: dict(sha256=wanted) for n, wanted in files.items()},
        build_order=[n.removesuffix('.lean') for n in files],
        development_only=True, public_acceptance=False)
    (args.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'complete actual files;', len(proofs), 'unique audit obligations')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=extract.VARIANTS, required=True)
    for name in ('actual', 'reference', 'reads', 'reads-audit', 'retention', 'retention-audit',
                 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, name, value.resolve(strict=name != 'output'))
    prepare(args)
