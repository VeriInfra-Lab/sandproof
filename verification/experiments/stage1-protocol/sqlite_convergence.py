"""Finite SQLite V3 handoff; preserve original evidence and use original checkers.

Only the fault's extraction and counterexample service run in formal mode. The
unchanged eight-page correct proof and the Ning native component are read back.
This is a component result, never a full native or V0 acceptance seal.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
HERE = Path('verification/experiments/sqlite-commit')
LIB = Path('verification/aeneas/sqlite/SandproofSqlite')
MEMO = 'controller-service/src/controller/transaction/memo.rs'
SOURCE_COMMIT = '8fc694185a4ccbe2ea181564ba860b14bea9f6e1'
INPUTS_ENV = 'SANDPROOF_SQLITE_CONVERGENCE_INPUTS'
FILES = [str(LIB/(name+'.lean')) for name in (
    'Audit', 'Examples', 'Funs', 'FunsExternal', 'Model', 'Results', 'Types', 'TypesExternal')]
FAILURES = sorted(['commit_unknown_exact', 'commit_unknown_abstract',
                   'commit_success_abstract', 'normal_commit_prefix'])
COUNTERS = ['SandproofSqliteRust.Counterexample.'+name for name in (
    'unknown_keeps_initial', 'unknown_violates_same_property', 'reachable_stale_witness')]
# Original seven Ning files, independently retained before this adapter existed.
# Locations are supplied privately; these hashes cannot be supplied by operators.
PACKET = {
    'correct/result.json': 'f0f59f18c173cfb39a3ba74a4c38e00aa4a7f82badd9e84b6d5d62fdfdb05051',
    'correct/format.log': 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    'correct/clippy.log': 'f87d899e348ab82e4a0f4b37eaa460737ed82c19db6f42726fda9b6382edcf8a',
    'correct/cache-tests.log': '326cfa9fbe7e4d287f157d5030fbccdc80d772f9bae1a7c04dc14ef196d555eb',
    'fault/summary.json': '238f49932a4a55b1a43d4a219af554a9007f9207161f9378346a22808f6e4bbc',
    'fault/receipt.json': '3f0b7839ff40903f9d8bec7e67a0ee93c08a9fd84838bb562b58e1d65f9cdc08',
    'fault/command.log': '04f9e7eb51b14135f8d062bbf1cdd6c6c7e2819ad640dc6dee07c4a94c65aa01',
}
NATIVE_COMMAND = ['cargo', 'test', '-p', 'sandproof-controller-service', '--locked',
    '--offline', '--lib', 'controller::cache_tests::uncertain_commit_reconciles_both_durable_outcomes_for_every_write_family',
    '--', '--exact', '--test-threads=1', '--nocapture']


def sha(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise ValueError('missing or linked original file: '+str(path))
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError('duplicate JSON key: '+key)
            result[key] = value
        return result
    sha(path)
    return json.loads(Path(path).read_text(), object_pairs_hook=unique)


def write(path, value):
    with Path(path).open('x') as stream:
        stream.write(json.dumps(value, indent=2)+'\n')


def module(name, path):
    sys.path.insert(0, str(ROOT))
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def original(root):
    # negative.py's original import is deliberately retained.
    sys.path.insert(0, str(root/HERE))
    return module('sqlite_convergence_original_negative', root/HERE/'negative.py')


def inventory(root):
    profile = read(root/HERE/'candidate.json')
    if profile.get('production_adopted') is not True or profile['current_capture_source_commit'] != SOURCE_COMMIT:
        raise ValueError('this finite adapter requires its reviewed adopted source')
    for name, digest in profile['source_files'].items():
        if sha(root/name) != digest:
            raise ValueError('current SQLite source closure changed: '+name)
    return profile


def plan(root=ROOT):
    audits = re.findall(r'^#audit_controller_proof (\S+)$', (root/LIB/'Audit.lean').read_text(), re.M)
    if len(audits) != 18 or len(set(audits)) != 18:
        raise ValueError('expected the complete original eighteen SQLite audits')
    return {'views': {'correct': {'files': FILES, 'audits': audits},
                      'skip-first-clear': {'files': ['Counterexample.lean'], 'audits': COUNTERS}}}


def _packet(folder):
    base = folder/'native-packet'
    for name, digest in PACKET.items():
        path = base/name
        if not path.resolve(strict=True).is_relative_to(base.resolve()) or sha(path) != digest:
            raise ValueError('original portable Ning packet differs: '+name)
    return base


def _native_readback(folder, root):
    from tools.acceptance_receipts import digest, load_success, rust_inputs
    profile = inventory(root)
    packet = _packet(folder)
    correct = read(packet/'correct/result.json')
    stages = [
        ('format', ['cargo', 'fmt', '--all', '--', '--check']),
        ('clippy', ['cargo', 'clippy', '-p', 'sandproof-controller-service', '--lib', '--tests',
                    '--locked', '--offline', '--', '-D', 'warnings']),
        ('cache-tests', ['cargo', 'test', '-p', 'sandproof-controller-service', '--locked', '--offline',
                        '--lib', 'controller::cache_tests', '--', '--test-threads=1'])]
    if (correct.get('base') != SOURCE_COMMIT or correct.get('exit_code') != 0
            or correct.get('source_unchanged') is not True or correct.get('tests') != 16
            or correct.get('ignored') != 0 or correct.get('patch_sha256') != profile['patch_sha256']
            or [(x['stage'], x['command']) for x in correct['stages']] != stages
            or any(x['exit_code'] != 0 for x in correct['stages'])
            or '16 passed; 0 failed; 0 ignored' not in (packet/'correct/cache-tests.log').read_text()):
        raise ValueError('original correct sixteen-test native entry did not pass')
    summary, receipt = read(packet/'fault/summary.json'), read(packet/'fault/receipt.json')
    key = receipt['key']
    expected = rust_inputs(root)
    old = (root/MEMO).read_text()
    line = '    cells.set_committed(None);\n'
    if old.count(line) != 2:
        raise ValueError('first-clear mutation is no longer unique')
    mutant = hashlib.sha256(old.replace(line, '    // Fault: preserve the old committed memo.\n', 1).encode()).hexdigest()
    expected[MEMO] = mutant
    used = [json.loads(row) for row in summary.get('used_receipts', '').splitlines()]
    compiler = read(root/'verification/first-chain-linux-environment.json')['production_compiler']
    if (summary.get('complete') is not True or summary.get('driver_exit_code') != 0
            or summary.get('production_commit') != SOURCE_COMMIT
            or summary.get('driver_commit') != profile['native_reference_driver_commit']
            or summary.get('actual_cargo_exit') != 101 or summary.get('original_assertion_rejected') is not True
            or summary.get('formal_tools_invoked') is not False or summary.get('mutated_memo_sha256') != mutant
            or summary.get('unchanged_source_files') != len(profile['source_files'])-1
            or summary.get('log_sha256') != sha(packet/'fault/command.log')
            or len(used) != 1 or used[0].get('identity') != digest(key)
            or used[0].get('sha256') != sha(packet/'fault/receipt.json')
            or key.get('label') != 'sqlite-commit-native-rejected' or key.get('command') != NATIVE_COMMAND
            or key.get('options') != {} or key.get('rust') != expected
            or key.get('rules', {}).get(str(HERE/'negative.py')) != profile['native_reference_driver_sha256']
            or key.get('runtime', {}).get('platform') != 'linux' or key['runtime'].get('uid') != 1000
            or key['runtime'].get('rustc', {}).get('version', '').splitlines()[:1] != [compiler]
            or load_success(packet/'fault', key) != receipt or receipt.get('actual_exit') != 101
            or receipt.get('expected') != 101):
        raise ValueError('exact-source original native fault receipt differs; no new execution claimed')
    text = (packet/'fault/command.log').read_text()
    if any(s not in text for s in ('running 1 test', '0 passed; 1 failed; 0 ignored',
            'controller/cache_tests.rs:102:', 'assertion `left == right` failed')):
        raise ValueError('fault did not reach the unchanged original native assertion')
    return {str((packet/name).resolve()): sha(packet/name) for name in PACKET}


def check_native(result, root=ROOT, folder=None):
    if folder is None:
        raise ValueError('portable native evidence requires its actual containing folder')
    if (result.get('convergence_entry') != 'sqlite-commit' or result.get('mode') != 'native'
            or result.get('complete') is not True or result.get('actuals') != []
            or result.get('source_commit') != SOURCE_COMMIT or result.get('native_executed_here') is not False
            or result.get('packet') != PACKET):
        raise ValueError('different finite SQLite native component result')
    return _native_readback(Path(folder), Path(root))


def native(output, config, root=ROOT):
    correct = Path(config['correct']).resolve(strict=True)
    fault = Path(config['fault_evidence']).resolve(strict=True)
    source = Path(config['fault_source']).resolve(strict=True)
    profile = inventory(root)
    actual_head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=correct/'source', text=True).strip()
    if actual_head != SOURCE_COMMIT:
        raise ValueError('original correct source has a different Git identity')
    # Check the complete current native closure against the preserved original,
    # not only the two memo files or a convenient list of proof inputs.
    from tools.acceptance_receipts import rust_inputs
    if rust_inputs(correct/'source') != rust_inputs(root):
        raise ValueError('original correct native Rust closure differs from current source')
    for name, digest in profile['source_files'].items():
        if sha(correct/'source'/name) != digest:
            raise ValueError('original correct adopted source differs: '+name)
    for name in PACKET:
        origin = correct/'evidence'/Path(name).name if name.startswith('correct/') else fault/Path(name).name
        target = output/'native-packet'/name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(origin, target)
    original(root).consume_native_fault(root, source, fault, output/'native-original-readback.log', profile, NATIVE_COMMAND)
    result = dict(convergence_entry='sqlite-commit', complete=True, mode='native', source_commit=SOURCE_COMMIT,
                  native_executed_here=False, actuals=[], packet=PACKET)
    result['reuse_evidence'] = check_native(result, root, output)
    return result


def _correct(config, root):
    project = Path(config['correct_project']).resolve(strict=True)
    parent_path = Path(config['correct_parent_result']).resolve(strict=True)
    service_path = Path(config['correct_service_result']).resolve(strict=True)
    parent, service = read(parent_path), read(service_path)
    view = plan(root)['views']['correct']
    actual = service['actual']
    expected_files = {name: sha(root/name) for name in view['files']}
    if (parent.get('complete') is not False or parent.get('error') != "FileNotFoundError(2, 'No such file or directory')"
            or service.get('complete') is not True or service.get('original_failure_sha256') != sha(parent_path)
            or Path(service['original_failure_path']).resolve() != parent_path
            or service.get('source_hashes') != expected_files or actual.get('files') != expected_files
            or actual.get('audits') != view['audits']):
        raise ValueError('original correct proof/service identity or complete inventory differs')
    kernels = [row for row in parent['commands'] if row['label'].startswith('kernel-')]
    if len(kernels) != 8 or len(service['kernel_reuse']) != 8 or any(row['exit_code'] != 0 for row in parent['commands']):
        raise ValueError('original correct kernels or original IR admission failed')
    commands, evidence = [], {str(parent_path): sha(parent_path), str(service_path): sha(service_path)}
    for index, (name, kernel, reused) in enumerate(zip(FILES, kernels, service['kernel_reuse'], strict=True)):
        argv = ['lake', 'env', 'lean', name]
        log = Path(reused['original_log'])
        if (kernel['label'] != 'kernel-'+str(index) or kernel['argv'] != argv
                or reused.get('use') != 'REUSE' or reused.get('argv') != argv or reused.get('exit_code') != 0
                or sha(log) != reused.get('original_log_sha256') or sha(project/name) != expected_files[name]):
            raise ValueError('original correct file/kernel reuse differs: '+name)
        commands.append(dict(argv=argv, cwd=str(project), exit_code=0, log_sha256=sha(log)))
        evidence[str(log.resolve())] = sha(log)
        evidence[str((project/name).resolve())] = sha(project/name)
    # Do not silently reuse an old import or a different Lean/package installation.
    for name in ['SandboxModel/ProofAuditCommand.lean', 'lean-toolchain', 'lake-manifest.json']:
        if sha(project/name) != sha(root/name):
            raise ValueError('correct proof dependency changed: '+name)
        evidence[str((project/name).resolve())] = sha(project/name)
    for name in FILES:
        imports = re.findall(r'^import (\S+)$', (root/name).read_text(), re.M)
        if any(n != 'Aeneas' and n != 'SandboxModel.ProofAuditCommand'
               and not n.startswith('SandproofSqlite.') for n in imports):
            raise ValueError('unregistered correct SQLite import: '+name)
    for name in ('.lake/packages', '.tools'):
        if not (project/name).samefile(root/name):
            raise ValueError('original package/tool installation differs: '+name)
    physical = service_path.parent/'physical-files'
    for name in ('requests.jsonl', 'service.log'):
        evidence[str((physical/name).resolve())] = sha(physical/name)
    return project, actual, commands, evidence, physical, service


def _replay(root, replay, readback):
    result = original(root).consume_current_replay(root, replay, readback)
    evidence = {str(readback.resolve()): sha(readback)}
    for name in ('result.json', 'sandproof_sqlite_core.llbc', 'sandproof_sqlite_wrapper.llbc',
                 'generated/SandproofSqlite/Types.lean', 'generated/SandproofSqlite/Funs.lean',
                 'generated/translation.json'):
        path = replay/'evidence'/name
        evidence[str(path.resolve())] = sha(path)
    return result, evidence


def check(result, root=ROOT):
    root = Path(root)
    if (result.get('convergence_entry') != 'sqlite-commit' or result.get('mode') != 'formal'
            or result.get('complete') is not True or result.get('source_commit') != SOURCE_COMMIT):
        raise ValueError('different finite SQLite formal component result')
    inventory(root)
    from tools.check_sqlite_extraction import check_libraries
    check_libraries(root, root/'.tools')
    context = result['readback']
    native_path = Path(context['native_result'])
    evidence = check_native(read(native_path), root, native_path.parent)
    evidence[str(native_path.resolve())] = sha(native_path)
    project, actual, commands, retained, physical, _ = _correct(context, root)
    if result['actuals'][0] != actual or len(result['actuals']) != 2:
        raise ValueError('correct actual-file object was edited instead of reused')
    reader = module('sqlite_original_physical_reader', root/'verification/experiments/stage1-protocol/registered.py')
    copied = Path(context['correct_readback'])
    for index, command in enumerate(commands):
        if sha(copied/('kernel-'+str(index)+'.log')) != command['log_sha256']:
            raise ValueError('kernel readback copy differs from original')
    for name in ('requests.jsonl', 'service.log'):
        if sha(copied/'physical-files'/name) != sha(physical/name):
            raise ValueError('service readback copy differs from original')
    reader._saved_physical_view(actual, project, copied/'physical-files', commands)
    _, replay_evidence = _replay(root, Path(context['correct_replay']), Path(context['registry_readback']))
    retained.update(replay_evidence)
    neg = Path(context['negative'])
    bad = read(neg/'evidence/result.json')
    if (bad.get('exit_code') != 0 or bad.get('actual_rust_mutant') is not True
            or bad.get('source_unchanged') is not True or bad.get('generated_types_identical') is not True
            or bad.get('same_property_failures') != FAILURES or bad.get('kernel_counterexample_count') != 3
            or bad.get('same_properties_sha256') != sha(root/LIB/'Results.lean')
            or sha(neg/'evidence/SameProperties.lean') != sha(root/LIB/'Results.lean')
            or sha(neg/'evidence/Counterexample.lean') != sha(root/HERE/'Counterexample.lean')
            or sha(neg/'lean/Counterexample.lean') != sha(root/HERE/'Counterexample.lean')
            or bad.get('native_readback', {}).get('native_executed_here') is not False):
        raise ValueError('original bad Rust did not fail the unchanged properties')
    required = ['charon', 'aeneas', 'generated-build', 'same-properties-rejected', 'counterexample']
    if ([s['stage'] for s in bad['stages']] != required or any(s['exit_code'] !=
            (1 if s['stage'] == 'same-properties-rejected' else 0) for s in bad['stages'])):
        raise ValueError('missing original formal fault stages')
    from tools.acceptance_receipts import rust_inputs
    fault_receipt = read(native_path.parent/'native-packet/fault/receipt.json')
    if (rust_inputs(neg/'source') != fault_receipt['key']['rust']
            or bad.get('mutant_files') != {name:sha(neg/'source'/name)
                for name in read(root/HERE/'candidate.json')['candidate_files']}):
        raise ValueError('formal fault Rust inputs differ from the actual Ning fault')
    negative_driver = original(root)
    errors = [x for x in negative_driver.diagnostics(neg/'evidence/same-properties-rejected.log')
              if x['severity'] == 'error']
    starts = [(i+1, re.match(r'theorem (\w+)', line).group(1))
              for i, line in enumerate((root/LIB/'Results.lean').read_text().splitlines())
              if line.startswith('theorem ')]
    owners = {next((name for line, name in reversed(starts) if line <= e['pos']['line']), None) for e in errors}
    if owners != set(FAILURES):
        raise ValueError('raw original diagnostics do not reject the four unchanged properties')
    if read(neg/'source/sandproof_sqlite_core.llbc').get('has_errors') is not False:
        raise ValueError('fault extraction was incomplete')
    if (sha(neg/'evidence/generated/SandproofSqlite/Types.lean') != sha(root/LIB/'Types.lean')
            or sha(neg/'evidence/generated/SandproofSqlite/Funs.lean') == sha(root/LIB/'Funs.lean')):
        raise ValueError('fault generated types or changed implementation differ')
    for name in ('tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        if sha(neg/'lean'/name) != sha(root/name):
            raise ValueError('counterexample service client differs: '+name)
    wrong = result['actuals'][1]
    if list(wrong['files']) != ['Counterexample.lean'] or wrong['audits'] != COUNTERS:
        raise ValueError('incomplete counterexample physical inventory')
    reader._saved_physical_view(wrong, neg/'lean', neg/'physical-check/physical-files', result['commands'])
    for name, digest in result['reuse_evidence'].items():
        if sha(Path(name)) != digest:
            raise ValueError('consumed original evidence changed: '+name)
    evidence.update(retained)
    required = [neg/'evidence/result.json', neg/'source/sandproof_sqlite_core.llbc',
                neg/'evidence/SameProperties.lean', neg/'evidence/same-properties-rejected.log',
                neg/'evidence/generated/SandproofSqlite/Types.lean',
                neg/'evidence/generated/SandproofSqlite/Funs.lean', neg/'source'/MEMO,
                neg/'lean/Counterexample.lean', neg/'lean/tools/lean_session.py',
                neg/'lean/scripts/lean-mcp-server.sh', neg/'physical-check/kernel-0.log',
                neg/'physical-check/physical-files/requests.jsonl', neg/'physical-check/physical-files/service.log']
    evidence.update({str(p.resolve()): sha(p) for p in required})
    if not evidence.items() <= result['reuse_evidence'].items():
        raise ValueError('original evidence was not bound into the final result')
    return dict(result['reuse_evidence'])


def evidence(result, root=ROOT):
    return check(result, root)


def formal(output, config, root=ROOT):
    inventory(root)
    native_path = Path(config['native_result']).resolve(strict=True)
    reused = check_native(read(native_path), root, native_path.parent)
    reused[str(native_path)] = sha(native_path)
    project, correct, _, retained, physical, service = _correct(config, root)
    reused.update(retained)
    copied = output/'correct-readback'
    copied.mkdir()
    shutil.copytree(physical, copied/'physical-files')
    for index, row in enumerate(service['kernel_reuse']):
        shutil.copy2(row['original_log'], copied/('kernel-'+str(index)+'.log'))
    replay = Path(config['correct_replay']).resolve(strict=True)
    baseline = read(replay/'evidence/result.json')
    registry = output/'current-registry-readback.json'
    write(registry, dict(complete=True, original_result_sha256=sha(replay/'evidence/result.json'),
        source_commit=baseline['base'], core_llbc_sha256=baseline['core_llbc_sha256'],
        wrapper_llbc_sha256=baseline['wrapper_llbc_sha256'], generated_files={name:sha(
            root/'verification/aeneas/sqlite'/name) for name in (
                'SandproofSqlite/Types.lean', 'SandproofSqlite/Funs.lean', 'translation.json')}))
    _, original_replay = _replay(root, replay, registry)
    reused.update(original_replay)
    commands = []

    def run(cwd, folder, label, argv):
        argv = list(map(str, argv))
        log = folder/(label+'.log')
        started = time.monotonic()
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=cwd, env=os.environ.copy(), stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT)
        commands.append(dict(label=label, argv=argv, cwd=str(cwd), exit_code=done.returncode,
                             log_sha256=sha(log), seconds=time.monotonic()-started))
        if done.returncode:
            raise ValueError('original SQLite formal stage failed: '+label)

    negative = output/'negative'
    run(root, output, 'original-negative', [sys.executable, root/HERE/'negative.py', '--formal-only',
        '--native-evidence', native_path.parent/'native-packet/fault', '--current-registry-readback', registry,
        '--source-base', root, '--destination', negative, '--correct-replay', replay,
        '--tool-root', root/'.tools'])
    child = negative/'lean'
    shutil.copy2(negative/'evidence/Counterexample.lean', child/'Counterexample.lean')
    # lean_session derives its project from its own real __file__. A symlink
    # would silently select the parent project instead of this counterexample.
    (child/'tools').mkdir()
    (child/'scripts').mkdir()
    shutil.copy2(root/'tools/lean_session.py', child/'tools/lean_session.py')
    shutil.copy2(root/'scripts/lean-mcp-server.sh', child/'scripts/lean-mcp-server.sh')
    (child/'.tools').symlink_to((root/'.tools').resolve(strict=True), target_is_directory=True)
    (child/'artifacts').mkdir()
    physical_output = negative/'physical-check'
    physical_output.mkdir()
    checker = module('sqlite_original_actual_files', root/'verification/experiments/stage1-protocol/formal.py')
    wrong = checker.actual_files(child, ['Counterexample.lean'], physical_output, os.environ.copy(), run)
    # Preserve the actual inputs consumed by the original bad-Rust pipeline.
    for name in ('result.json', 'SameProperties.lean', 'Counterexample.lean', 'native-rejected.log',
                 'charon.log', 'aeneas.log', 'generated-build.log', 'same-properties-rejected.log',
                 'counterexample.log', 'generated/SandproofSqlite/Types.lean',
                 'generated/SandproofSqlite/Funs.lean', 'generated/translation.json'):
        path = negative/'evidence'/name
        reused[str(path.resolve())] = sha(path)
    for path in [negative/'source/sandproof_sqlite_core.llbc', negative/'source'/MEMO,
                 child/'Counterexample.lean', child/'tools/lean_session.py', child/'scripts/lean-mcp-server.sh',
                 physical_output/'kernel-0.log',
                 physical_output/'physical-files/requests.jsonl', physical_output/'physical-files/service.log',
                 output/'original-negative.log']:
        reused[str(path.resolve())] = sha(path)
    for index in range(8):
        path = copied/('kernel-'+str(index)+'.log')
        reused[str(path.resolve())] = sha(path)
    for name in ('requests.jsonl', 'service.log'):
        path = copied/'physical-files'/name
        reused[str(path.resolve())] = sha(path)
    result = dict(convergence_entry='sqlite-commit', mode='formal', complete=True, source_commit=SOURCE_COMMIT,
        actuals=[correct, wrong], commands=commands, reuse_evidence=reused,
        readback={**config, 'native_result':str(native_path), 'correct_replay':str(replay),
                  'correct_readback':str(copied), 'registry_readback':str(registry), 'negative':str(negative)})
    check(result, root)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('native', 'formal'))
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != args.mode:
        raise ValueError('ordinary work belongs on Ning; formal work on the proof host')
    config = read(Path(os.environ[INPUTS_ENV]))
    if config.get('schema') != 1:
        raise ValueError('unknown private SQLite location manifest')
    output = args.output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    result = (native if args.mode == 'native' else formal)(output, config[args.mode])
    write(output/'result.json', result)
    print(json.dumps(dict(complete=True, convergence_entry='sqlite-commit', mode=args.mode)))


if __name__ == '__main__':
    main()
