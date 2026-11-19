"""Bind original Drop paths to the admitted memo core and same-property counterexample."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import re
import tarfile

import observation_drop_capture as capture
import prepare_observation_sql_save as joint
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('observation-drop')
ROOT = 'ObservationDrop'
NEGATIVE = 'ObservationDropNegative'
CORE_VIEW = 'ObservationDropCore'
CORE = 'controller.transaction.memo.rollback_if_live'
SQLITE = Path('verification/aeneas/sqlite/SandproofSqlite')
SQLITE_FILES = ('TypesExternal', 'Types', 'Funs', 'FunsExternal', 'SlotData', 'Model', 'Results')
CELL = 'controller.transaction.memo.Cells'


def checked(directory, baseline, mode):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['mode'], report['source'], report['tree'], report['driver'], report['options'], report['complete']) != (
            mode, capture.SOURCE, capture.TREE, capture.row.digest(Path(capture.__file__)), capture.CFG, True):
        raise ValueError('a completed exact original or rollback mutation capture is required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('actual original tools must finish')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    old_memo = read_exact(capture.row.ROOT / capture.MEMO_PATH, capture.MEMO_SHA)
    if read_exact(directory / 'source' / capture.MEMO_PATH, report['memo']) != capture.memo_source(old_memo, mode):
        raise ValueError('only original rollback failure invalidation can be removed')
    read_exact(directory / 'source' / capture.WORK, baseline['work_sha256'])
    read_exact(directory / 'source' / capture.row.CODEC, capture.STAGED)
    for command in report['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    ir = directory / 'actual.llbc'
    read_exact(ir, report['ir_sha256'])
    generated = directory / 'generated'
    expected = {'translation.json', *(f'{capture.NAMESPACE}/{name}.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (report['generated_sha256'].keys() != expected
            or {str(file.relative_to(generated)) for file in generated.rglob('*') if file.is_file()} != expected):
        raise ValueError('the original generated inventories are required')
    files = {name: read_exact(generated / name, report['generated_sha256'][name]) for name in expected}
    target = json.loads((capture.row.ROOT / 'verification/foundations-environment.json').read_bytes())['target']
    if json.dumps(capture.admit(ir, generated, target), sort_keys=True) != json.dumps(
            report['admission'], sort_keys=True):
        raise ValueError('actual normal Drop routes or original aliases differ')
    def extract(name):
        body = files[f'{capture.NAMESPACE}/{name}.lean']
        return declarations(body.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))
    return extract('Funs'), extract('Types'), report


def compare(correct, wrong, original):
    good, types, admitted = correct
    bad, other_types, _ = wrong
    baseline = joint.rows.definitions(original, joint.capture.NAMESPACE)
    for name in (joint.suffix.PARENT, joint.suffix.FINISH_SOURCE, joint.suffix.FINISH, joint.CODEC + 'save'):
        if good[name] != baseline['Funs.lean'][name]:
            raise ValueError('the original translated write/return chain differs')
    if good.keys() != bad.keys() or types != other_types:
        raise ValueError('the rollback mutation must retain the same declarations')
    old = b'    then\n      let cells1 \xe2\x86\x90 CellsInst.set_committed cells none\n      ok (transaction1, cells1)'
    new = b'    then ok (transaction1, cells)'
    if (good[CORE].count(old) != 1 or good[CORE].replace(old, new) != bad[CORE]
            or any(good[name] != bad[name] for name in good if name != CORE)):
        raise ValueError('the sole generated change must retain the stale cache on rollback failure')
    original_memo = capture.row.ROOT / SQLITE / 'Funs.lean'
    memo = declarations(original_memo.read_bytes().removesuffix(b'\n\nend SandproofSqliteRust\n'))
    if memo[CORE] != good[CORE]:
        raise ValueError('the original active rollback core differs from admitted memo math')
    original_types = (capture.row.ROOT / SQLITE / 'Types.lean').read_bytes()
    original_types = declarations(original_types.removesuffix(b'\n\nend SandproofSqliteRust\n'))
    if types[CELL] != original_types[CELL]:
        raise ValueError('the original memo cell dictionary changed')
    return {name: sha256(good[name]).hexdigest() for name in (
        CORE, 'controller.transaction.WorldTransaction.Insts.CoreOpsDropDrop.drop')}


def memo_view(correct, sqlite):
    equations = re.findall(rb'^theorem rollback_(?:absent|success|unknown)_exact .*?(?=\n\n(?:theorem|/-))',
                           sqlite['Results'], re.M | re.S)
    if len(equations) != 3:
        raise ValueError('the original three rollback equations are required')
    model = b'def slotOps ' + sqlite['Model'].split(b'\ndef slotOps ', 1)[1].removesuffix(
        b'\n\nend SandproofSqliteRust.Model\n')
    result_test = b'def core.result.Result.is_err ' + sqlite['FunsExternal'].split(
        b'\ndef core.result.Result.is_err ', 1)[1].removesuffix(b'\n\nend SandproofSqliteRust\n')
    template = (HERE / f'{CORE_VIEW}.lean.in').read_bytes()
    for marker, block in ((b'-- ACTUAL MEMO CORE', correct[1][CELL] + b'\n\n' + correct[0][CORE]),
                          (b'-- ORIGINAL RESULT TEST', result_test),
                          (b'-- ORIGINAL B2 MODEL', model),
                          (b'-- ORIGINAL ROLLBACK EQUATIONS', b'\n\n'.join(equations))):
        template = joint.suffix.replace_once(template, marker, block)
    return template


def prepare(core, mutant, joint_capture, prior, audit, output, packages, service):
    original, baseline = joint.joint_files(joint_capture)
    correct = checked(core, baseline, 'core')
    wrong = checked(mutant, baseline, 'keep-cache')
    matching = compare(correct, wrong, original)
    prior_plan = json.loads((prior / 'plan.json').read_bytes())
    diagnostics = json.loads((audit / 'result.json').read_bytes())
    if (prior_plan['root'] != 'ObservationSqlSave' or prior_plan['source'] != capture.SOURCE
            or prior_plan['joint'] != baseline or diagnostics['complete'] is not True
            or diagnostics['source_sha256'] != prior_plan['files'] or diagnostics['physical_files'] != 35
            or len(diagnostics['audited']) != 74):
        raise ValueError('the existing audited exact linked parent view is required')
    read_exact(audit / 'service.log', diagnostics['service_log_sha256'])
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)

    def write(name, data):
        path = output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            if path.read_bytes() != data:
                raise ValueError('the shared proof dependency differs: ' + name)
        else:
            path.write_bytes(data)

    for name, digest in prior_plan['files'].items():
        write(name, read_exact(prior / name, digest))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        write(name, (capture.row.ROOT / name).read_bytes())
    sqlite = {}
    with tarfile.open(core / 'source.tar') as archive:
        for part in SQLITE_FILES:
            path = SQLITE / f'{part}.lean'
            with archive.extractfile(str(path)) as original:
                sqlite[part] = original.read()
            if (capture.row.ROOT / path).read_bytes() != sqlite[part]:
                raise ValueError('the original memo proof source differs: ' + part)
    extra = {f'SandproofSqlite/{part}.lean' for part in ('SlotData', 'TypesExternal')}
    for name in extra:
        write(name, sqlite[Path(name).stem])
    write(f'{CORE_VIEW}.lean', memo_view(correct, sqlite))
    template = (HERE / f'{NEGATIVE}.lean.in').read_bytes()
    write(f'{NEGATIVE}.lean', joint.suffix.replace_once(template, b'-- ACTUAL WRONG ROLLBACK', wrong[0][CORE]))
    write(f'{ROOT}.lean', (HERE / f'{ROOT}.lean.in').read_bytes())
    configuration = (prior / 'lakefile.toml').read_text()
    with (output / 'lakefile.toml').open('x') as file:
        file.write(configuration)
        for name in (CORE_VIEW, NEGATIVE, ROOT):
            file.write(f'\n[[lean_lib]]\nname = "{name}"\n')
    files = closure(output, [ROOT + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(prior_plan['files']) | extra | {name + '.lean' for name in (ROOT, NEGATIVE, CORE_VIEW)}:
        raise ValueError('the actual normal Drop proof closure differs')
    plan = dict(root=ROOT, source=capture.SOURCE, original=correct[2], wrong=wrong[2], shared_core=matching,
                parent=prior_plan, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()},
                build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOT, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    names = ('core', 'mutant', 'joint-capture', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp')
    for name in names:
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in names))
