"""Reproduce the complete original match or its actual omit-source-work-binding view."""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location('source_full_binding_validation', HERE / 'validation.py')
validation = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(validation)
capture = validation.capture
CORE = tuple('SourceFullBinding' + name for name in ('Spec', 'Library', 'View', 'Proof', 'Values'))
WRONG = ('SourceFullBindingWrong', 'SourceFullBindingNegative')


def replace(source, marker, body):
    if source.count(marker) != 1:
        raise ValueError('one complete original declaration insertion required')
    return source.replace(marker, body + b'\n')


def prepare(args):
    original, reference = validation.current(args.reference, 'original')
    wrong = args.variant == 'omit-source-work-binding'
    if wrong:
        actual, generated = validation.current(args.actual, args.variant)
        wrong_view = validation.same_interfaces(reference, generated)
    else:
        if args.variant != 'original' or args.actual != args.reference:
            raise ValueError('one exact current correct capture is required')
        actual, generated, wrong_view = original, reference, None
    parent, evidence, additions = validation.parent(args.reads, args.reads_audit,
        args.retention, args.retention_audit, reference)
    args.output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        path = args.output / name
        path.parent.mkdir(exist_ok=True)
        path.symlink_to(target, target_is_directory=True)
    for name, wanted in parent['files'].items():
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((args.reads / name).read_bytes())
        if validation.digest(path) != wanted:
            raise ValueError('inherited source changed during copying: ' + name)
    for name in validation.SUPPORT:
        source = (validation.REPO / name).read_bytes()
        if source != (args.reads / name).read_bytes():
            raise ValueError('the exact common support must remain unchanged: ' + name)
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(source)
    modules = (*CORE, *WRONG) if wrong else CORE
    lake = (args.reads / 'lakefile.toml').read_text()
    for name in modules:
        source = (HERE / (name + '.lean.in')).read_bytes()
        if name == 'SourceFullBindingLibrary':
            if source != validation.library():
                raise ValueError('the same exact standard bindings are mandatory')
        elif name == 'SourceFullBindingView':
            source = replace(source, b'-- ACTUAL COMPLETE FULL BINDING FUNCTIONS\n',
                validation.raw_functions(reference))
        elif name == 'SourceFullBindingWrong':
            source = replace(source, b'-- ACTUAL COMPLETE FULL BINDING FUNCTIONS\n',
                validation.raw_functions(generated))
        (args.output / (name + '.lean')).write_bytes(source)
        if 'name = "' + name + '"' not in lake:
            lake += '\n[[lean_lib]]\nname = "' + name + '"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    root = 'SourceFullBindingNegative' if wrong else 'SourceFullBindingValues'
    files = validation.original_inputs.closure(args.output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(parent['files']) | {name + '.lean' for name in modules}
    if set(files) != expected or any(files[name] != wanted for name, wanted in parent['files'].items()):
        raise ValueError('all inherited and current actual files must remain in the complete closure')
    audits = []
    for name, wanted in files.items():
        if validation.digest(args.output / name) != wanted:
            raise ValueError('materialized source changed')
        audits.extend(re.findall(r'^#audit_controller_proof (\S+)$', (args.output / name).read_text(), re.M))
    if len(audits) != len(set(audits)):
        raise ValueError('complete audit inventory must be unique')
    plan = dict(root=root, variant=args.variant, source=capture.retire.SOURCE, tree=capture.retire.TREE,
        reference=original, actual=actual, parent=evidence,
        selected_declarations={group: list(rows) for group, rows in additions.items()},
        templates={str(p.relative_to(validation.REPO)): validation.digest(p)
                   for p in [*(p for p in sorted(HERE.iterdir()) if p.is_file()),
                             *(validation.REPO / name for name in validation.extra_inputs())]},
        files=files, strict_files=sorted(files), audits=audits,
        modules={name: dict(sha256=wanted) for name, wanted in files.items()},
        build_order=[name.removesuffix('.lean') for name in files],
        development_only=True, public_acceptance=False, whole_receipt_proved=False)
    (args.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(root, len(files), 'complete actual files;', len(audits), 'unique audit obligations')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=capture.VARIANTS, required=True)
    for name in ('actual', 'reference', 'reads', 'reads-audit', 'retention', 'retention-audit',
                 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, name, value.resolve(strict=name != 'output'))
    prepare(args)
