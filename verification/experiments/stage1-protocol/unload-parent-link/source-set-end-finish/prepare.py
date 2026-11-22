"""Reproduce full-body development views with all rejected metadata retained.

This does not alter or satisfy the strict30 capture admission. The unknown
standard method is arbitrary data; its exact use certificate is checked.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('source_set_end_finish_validation', HERE/'validation.py')
validation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validation)
capture = validation.capture
CORE = ('SourceSetEndFinishView', 'SourceSetEndFinishProof', 'SourceSetEndBoxUseModel',
        'SourceSetEndBoxUseView', 'SourceSetEndBoxUseProof', 'SourceSetEndFinishValues', 'SourceSetEndFinish')
WRONG = ('SourceSetEndPrefixWrong', 'SourceSetEndPrefixNegative',
         'SourceSetEndFinishWrongView', 'SourceSetEndFinishNegative')


def insert(source, marker, body):
    if source.count(marker) != 1:
        raise ValueError('one exact original declaration insertion required')
    return source.replace(marker, body+b'\n')


def raw_body(reference, names, box=False):
    rows = validation.parsed(reference['Funs'])
    bodies = []
    for name, body in rows.items():
        if name not in names:
            continue
        raw = validation.declaration(reference, 'Funs', name, body)
        if box and name in {validation.PRIOR+'.eq', validation.END+'.eq'}:
            # Only these two derived dictionaries use the default ne. Their
            # complete representation equalities are checked in BoxUseProof.
            raw += ('\n\nlocal notation "'+name.removesuffix('.eq')+'" => '
                'SandboxModel.Stage1Protocol.SourceSetEndBoxUseModel.derivedDictionary '+name).encode()
        bodies.append(raw)
    if set(names) - rows.keys():
        raise ValueError('a complete original function body is missing')
    return b'\n\n'.join(bodies)


def prepare(args):
    original, reference = validation.raw(args.reference, 'original')
    wrong = args.variant == 'allow-completed-no-request'
    if wrong:
        actual, generated = validation.raw(args.actual, args.variant)
        difference = validation.same_interfaces(reference, generated)
    else:
        if args.variant != 'original' or args.actual != args.reference:
            raise ValueError('one exact original raw output is required')
        actual, generated, difference = original, reference, None
    parent, evidence, additions = validation.parent(args.reads, args.reads_audit,
        args.retention, args.retention_audit, reference)
    boxed = {validation.OBSERVATION, validation.PRIOR+'.eq', validation.END+'.eq', validation.ROOT}
    sites = {name for name, raw in validation.parsed(reference['Funs']).items()
             if b'core.cmp.PartialEqBox' in raw}
    if sites != {validation.PRIOR+'.eq', validation.END+'.eq'}:
        raise ValueError('all original Box dictionary construction sites must be quantified')
    args.output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        path = args.output/name
        path.parent.mkdir(exist_ok=True)
        path.symlink_to(target, target_is_directory=True)
    for name, wanted in parent['files'].items():
        path = args.output/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes((args.reads/name).read_bytes())
        if validation.digest(path) != wanted:
            raise ValueError('inherited current file changed: '+name)
    for name in validation.SUPPORT:
        source = (validation.REPO/name).read_bytes()
        if source != (args.reads/name).read_bytes():
            raise ValueError('original common support must remain unchanged: '+name)
        path = args.output/name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(source)
    lake = (args.reads/'lakefile.toml').read_text()
    modules = (*CORE, *WRONG) if wrong else CORE
    for name in modules:
        owner = HERE.parent/'source-set-end' if name in WRONG[:2] else HERE
        source = (owner/(name+'.lean.in')).read_bytes()
        if name in ('SourceSetEndFinishView', 'SourceSetEndFinishWrongView'):
            source = insert(source, b'-- EXACT ORIGINAL COMPLETE SET END DECLARATIONS\n',
                raw_body(generated if name.endswith('WrongView') else reference, validation.NEW))
        elif name == 'SourceSetEndBoxUseView':
            source = insert(source, b'-- ORIGINAL COMPLETE DECLARATIONS WITH UNCONSTRAINED BOX NE SLOT\n',
                raw_body(reference, boxed, box=True))
        elif name == 'SourceSetEndPrefixWrong':
            source = insert(source, b'-- ACTUAL ORIGINAL SET END PREFIX FUNCTIONS\n',
                raw_body(generated, validation.prefix.NEW['Funs']))
        (args.output/(name+'.lean')).write_bytes(source)
        if 'name = "'+name+'"' not in lake:
            lake += '\n[[lean_lib]]\nname = "'+name+'"\n'
    (args.output/'lakefile.toml').write_text(lake)
    root = 'SourceSetEndFinishNegative' if wrong else 'SourceSetEndFinish'
    files = validation.original_inputs.closure(args.output, [root+'.lean'], active_view='lakefile.toml')
    expected = set(parent['files']) | {n+'.lean' for n in modules}
    if set(files) != expected or any(files[n] != h for n, h in parent['files'].items()):
        raise ValueError('the entire inherited and new actual-file closure is required')
    audits = []
    for name in files:
        audits.extend(re.findall(r'^#audit_controller_proof (\S+)$', (args.output/name).read_text(), re.M))
    if len(audits) != len(set(audits)):
        raise ValueError('the full audit inventory must be unique')
    inputs = [p for p in sorted(HERE.iterdir()) if p.is_file()]
    inputs += [validation.REPO/n for n in validation.extra_inputs()]
    inputs += [HERE.parent/'source-set-end'/(n+'.lean.in') for n in WRONG[:2]]
    templates = {str(p.relative_to(validation.REPO)): validation.digest(p) for p in inputs}
    plan = dict(root=root, variant=args.variant, source=capture.retire.SOURCE, tree=capture.retire.TREE,
        reference=original, actual=actual, parent=evidence, wrong_view=difference,
        selected_declarations={g: list(a) for g, a in additions.items()},
        templates=templates, files=files, strict_files=sorted(files), audits=audits,
        modules={n: dict(sha256=h) for n, h in files.items()},
        build_order=[n.removesuffix('.lean') for n in files],
        development_only=True, public_acceptance=False, full_setter_admission_passed=False,
        original31_metadata_retained=True, box_ne_law_assumed=False,
        scope='exact full generated body, complete result/poststate and arbitrary-method use certificate')
    (args.output/'plan.json').write_text(json.dumps(plan, indent=2)+'\n')
    print(root, len(files), 'actual files;', len(audits), 'unique audits; capture admission still rejected')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=capture.VARIANTS, required=True)
    for name in ('actual', 'reference', 'reads', 'reads-audit', 'retention', 'retention-audit',
                 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--'+name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        if name != 'variant':
            setattr(args, name, path.resolve(strict=name != 'output'))
    prepare(args)
