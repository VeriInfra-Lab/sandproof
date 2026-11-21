"""Reproduce all current correct or actual wrong Full/Pause proof inputs."""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location('source_full_work_guard_validation', HERE / 'validation.py')
validation = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(validation)
capture = validation.capture
CORE = tuple('SourceFullWorkGuard' + name for name in (
    'Types', 'Std', 'Library', 'View', 'LoopWitness', 'ManifestPathModel', 'ManifestPath',
    'ManifestProfileModel', 'ManifestProfile', 'Spec', 'Pause', 'Proof', 'Values'))
WRONG = ('SourceFullWorkGuardWrong', 'SourceFullWorkGuardNegative')


def replace(source, marker, body):
    if source.count(marker) != 1:
        raise ValueError('one complete exact insertion is required: ' + marker.decode())
    return source.replace(marker, body + b'\n')


def prepare(args):
    original, reference = validation.current(args.reference, 'original')
    wrong = args.variant == 'allow-empty-state'
    if wrong:
        actual, generated = validation.current(args.actual, args.variant)
        wrong_root = validation.same_interfaces(reference, generated)
    else:
        if args.variant != 'original' or args.actual != args.reference:
            raise ValueError('the single exact current correct capture is required')
        actual, generated, wrong_root = original, reference, None
    parent, evidence, added = validation.parent(args.reads, args.reads_audit,
        args.retention, args.retention_audit, reference)
    std, std_evidence = validation.reuse.standards(validation.REPO)
    generic, generic_evidence = validation.reuse.generic(validation.REPO,
        set(validation.parsed(reference['Funs'])) | capture.LIBRARIES)
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
            raise ValueError('an inherited actual source changed during copying: ' + name)
    for name in validation.SUPPORT:
        source = (validation.REPO / name).read_bytes()
        if source != (args.reads / name).read_bytes():
            raise ValueError('the unchanged exact common support is required: ' + name)
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(source)
    modules = (*CORE, *WRONG) if wrong else CORE
    lake = (args.reads / 'lakefile.toml').read_text()
    for name in modules:
        if name + '.lean' in generic:
            source = generic[name + '.lean']
        else:
            source = (HERE / (name + '.lean.in')).read_bytes()
            if name == 'SourceFullWorkGuardTypes':
                body = b'\n\n'.join(validation.declaration(reference, 'Types', item, added['Types'][item])
                    for item in ('sandproof_protocol.lifecycle.manifest.ManifestError',
                                 'capture.work.PauseWorkRequest.valid.closure'))
                source = replace(source, b'-- ACTUAL CURRENT MANIFEST ERROR AND PAUSE CALLBACK TYPE\n', body)
            elif name == 'SourceFullWorkGuardStd':
                source = replace(source, b'-- EXACT EXISTING STANDARD VALUE BINDINGS\n', std)
            elif name == 'SourceFullWorkGuardView':
                source = replace(source, b'-- ACTUAL COMPLETE CURRENT FULL AND PAUSE FUNCTIONS\n',
                                 validation.raw_functions(reference))
            elif name == 'SourceFullWorkGuardWrong':
                source = replace(source, b'-- ACTUAL WRONG FULL GUARD\n',
                    validation.declaration(generated, 'Funs', validation.ROOT, wrong_root))
        (args.output / (name + '.lean')).write_bytes(source)
        if 'name = "' + name + '"' not in lake:
            lake += '\n[[lean_lib]]\nname = "' + name + '"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    root = 'SourceFullWorkGuardNegative' if wrong else 'SourceFullWorkGuardValues'
    files = validation.original_inputs.closure(args.output, [root + '.lean'], active_view='lakefile.toml')
    expected = set(parent['files']) | {name + '.lean' for name in modules}
    if set(files) != expected or any(files[name] != wanted for name, wanted in parent['files'].items()):
        raise ValueError('every inherited and current actual file must remain in the complete closure')
    audits = []
    for name, wanted in files.items():
        if validation.digest(args.output / name) != wanted:
            raise ValueError('materialized actual source changed')
        audits.extend(re.findall(r'^#audit_controller_proof (\S+)$', (args.output / name).read_text(), re.M))
    if len(audits) != len(set(audits)):
        raise ValueError('the complete audit inventory must be unique')
    template_paths = [*(p for p in sorted(HERE.iterdir()) if p.is_file()),
                      *(validation.REPO / name for name in validation.extra_inputs())]
    plan = dict(root=root, variant=args.variant, source=capture.retire.SOURCE, tree=capture.retire.TREE,
        reference=original, actual=actual, parent=evidence,
        selected_declarations={group: list(rows) for group, rows in added.items()},
        exact_standard_bindings=std_evidence, generic_instantiations=generic_evidence,
        original_full_functions_materialized=29, canonical_parent_types_reused=24,
        wrong_reuses_exact_28_identical_functions=wrong,
        templates={str(p.relative_to(validation.REPO)): validation.digest(p) for p in template_paths},
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
