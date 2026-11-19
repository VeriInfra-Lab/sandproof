"""Join the parent and SQL proofs using one actual combined Rust extraction."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import tempfile

import observation_sql_save_capture as capture
import prepare_observation_parent as parent
import prepare_observation_suffix as suffix
import prepare_source_save_row_list as rows
from prepare_observation_slot import captured, definitions
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('observation-sql-save')
ROOT = 'ObservationSqlSave'
CODEC = rows.PREFIX
SOURCE_TYPE = CODEC + 'Sources'
TYPE_ARGUMENTS = (b'\n  capture.source.SourceObservationConfig\n'
                  b'  sandproof_protocol.internal.NodeIdentity\n'
                  b'  controller.lifecycle_inbox.source_work.SourceWorkRecord')
SAVE_INSTANCE_SUFFIX = 'SourceObservationConfigNodeIdentitySourceWorkRecord'


def joint_files(directory):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['source'], report['tree'], report['driver'], report['options'], report['complete']) != (
            capture.SOURCE, capture.TREE, capture.row.digest(Path(capture.__file__)), capture.CFG, True):
        raise ValueError('a completed current joint capture is required')
    commands = [(step['label'], step['exit_code'], step['bounded_stop']) for step in report['commands']]
    if commands != [('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed extraction commands must complete')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    read_exact(directory / 'source' / capture.row.CODEC, capture.STAGED)
    read_exact(directory / 'source' / capture.WORK, report['work_sha256'])
    for step in report['commands']:
        read_exact(directory / (step['label'] + '.log'), step['log_sha256'])
    ir = directory / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = directory / 'generated'
    expected = {'translation.json', *(f'{capture.NAMESPACE}/{name}.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (report['generated_sha256'].keys() != expected
            or {str(file.relative_to(generated)) for file in generated.rglob('*') if file.is_file()} != expected):
        raise ValueError('the current joint generated inventory differs')
    data = {name: read_exact(generated / name, report['generated_sha256'][name]) for name in expected}
    fixed = json.loads((capture.row.ROOT / 'verification/foundations-environment.json').read_bytes())
    if json.dumps(capture.admit(ir, generated, fixed['target']), sort_keys=True) != json.dumps(
            report['admission'], sort_keys=True):
        raise ValueError('the original combined parent admission differs')
    return {name: data[f'{capture.NAMESPACE}/{name}'] for name in ('Types.lean', 'Funs.lean')}, report


def compare_joint(combined, original, save):
    together = rows.definitions(combined, capture.NAMESPACE)
    caller = parent.actual_declarations(original)
    codec = rows.definitions(save, rows.capture.NAMESPACE)
    if any(definitions(combined[name], name) != definitions(original[name], name) for name in original):
        raise ValueError('the current direct parent must use the entire original scan and decision')
    direct, original_funs = together['Funs.lean'], caller['Funs.lean']

    prior = suffix.SAVE_DICTIONARY
    current = prior + SAVE_INSTANCE_SUFFIX
    expected = original_funs[suffix.PARENT]
    if expected.count(prior.encode()) != 1 or expected.replace(prior.encode(), current.encode()) != direct[suffix.PARENT]:
        raise ValueError('the full parent may only select the same save instance at typed Sources')
    source = SOURCE_TYPE.encode()
    # The only adapter change is the three arguments to the same Sources struct.
    bindings = {
        prior: (b'  ' + source + b' :=', b'  (' + source + TYPE_ARGUMENTS + b') :='),
        prior + '.save': (b'(sources : ' + source + b')', b'(sources : ' + source + TYPE_ARGUMENTS + b')'),
    }
    for name, (old, new) in bindings.items():
        template = original_funs[name].replace(prior.encode(), current.encode())
        if template.count(old) != 1 or template.replace(old, new) != direct[name.replace(prior, current)]:
            raise ValueError('the original writer adapter or dictionary changed beyond Sources' + name)
    glue = (suffix.FINISH, suffix.FINISH_SOURCE, suffix.SCAN_DICTIONARY, suffix.SCAN_DICTIONARY + '.same_work')
    if any(direct[name] != original_funs[name] for name in glue):
        raise ValueError('actual parent tail and scan adapters differ')
    if any(together['Types.lean'][name] != caller['Types.lean'][name]
           for name in [*(name for name, _ in suffix.TYPES if name != SOURCE_TYPE), parent.CURRENT]):
        raise ValueError('the remaining original parent carrier fields differ')
    if any(together['Types.lean'][CODEC + name] != codec['Types.lean'][CODEC + name]
           for name in ('Sources', 'SourceSaveEffects', 'JournalSourceSave')):
        raise ValueError('the original typed source collection or save effect dictionary changed')
    scope = ('save', 'save_using', 'write_candidate_records', 'write_candidate_records_loop',
             'write_candidate_records_loop.body', 'write_candidate_record')
    scope = [*(CODEC + n for n in scope),
             *(n for n in codec['Funs.lean'] if n.startswith(CODEC + 'JournalSourceSave.Insts.'))]
    if any(direct[name] != codec['Funs.lean'][name] for name in scope):
        raise ValueError('the actual combined save and iteration must be the same earlier proofs')
    return {name: sha256(direct[name]).hexdigest()
            for name in (suffix.PARENT, current, current + '.save', *glue, *scope)}


def append_project(output, other, previous, related):
    for name, digest in related['files'].items():
        source = read_exact(other / name, digest)
        dest = output / name
        if name in previous['files']:
            read_exact(dest, digest)
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(source)
    for name in ('lean-toolchain', 'lake-manifest.json'):
        if (output / name).read_bytes() != (other / name).read_bytes():
            raise ValueError('fixed Lean dependencies differ')
    modules = r'\[\[lean_lib\]\]\nname = "([^"]+)"'
    known = set(re.findall(modules, (output / 'lakefile.toml').read_text()))
    with (output / 'lakefile.toml').open('a') as lake:
        extra = [name for name in re.findall(modules, (other / 'lakefile.toml').read_text()) if name not in known]
        for name in (*extra, ROOT):
            lake.write(f'\n[[lean_lib]]\nname = "{name}"\n')


def prepare(joint, original, prefix, scan, tail, current_sql, row_sql, output, packages, service):
    combined, receipt = joint_files(joint)
    original_bodies, _ = captured(original, 'parent')
    original_receipt = json.loads((original / 'result.json').read_bytes())
    read_exact(joint / 'source' / capture.WORK, original_receipt['candidate_source_sha256'])
    save, _ = rows.generated(current_sql)
    compared = compare_joint(combined, original_bodies, save)
    rows.prepare(current_sql, row_sql, output, packages, service)
    plan_path = output / 'plan.json'
    plan = json.loads(plan_path.read_bytes())
    with tempfile.TemporaryDirectory(dir=output.parent) as workspace:
        other = Path(workspace) / 'parent'
        parent.prepare(original, prefix, scan, tail, other, packages, service)
        parent_plan = json.loads((other / 'plan.json').read_bytes())
        append_project(output, other, plan, parent_plan)
    (output / f'{ROOT}.lean').write_bytes((HERE / f'{ROOT}.lean.in').read_bytes())
    files = closure(output, [ROOT + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(plan['files']) | set(parent_plan['files']) | {ROOT + '.lean'}:
        raise ValueError('the complete parent-SQL Lean closure differs')
    plan = dict(root=ROOT, source=capture.SOURCE, joint=receipt, corresponding=compared, parent=parent_plan,
                storage=plan, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    plan_path.write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    names = ('joint', 'original', 'prefix', 'scan', 'tail', 'current-sql', 'row-sql', 'output',
             'lake-packages', 'lean-mcp')
    for name in names:
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in names))
