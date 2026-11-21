"""Test a source rewrite in a disposable f26 archive, with the fixed extractor."""
import argparse
import difflib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import tarfile
import time

import prepare_transaction_capture as prepare
import frontier_capture
from revision_capture import declaration, regular_calls


ROOT = Path(__file__).resolve().parents[4]
GIB = 2**30
BASELINES = {
    prepare.SOURCE_COMMIT: (prepare.SOURCE_TREE,
                            '5e3c46112d1becd898fb070b4fd10373874184aef6fe40b1727e3fe6381fb824'),
    'a044095cd65ea518b1f57f4d4b2239de023c2bd1': (
        '84ba51c05691a86e8bbf538397899f95b500bc57',
        'ed12d2e1671877b5a42de85879ff18216d0fd8c71ed2e62a9dba9666bda87772',
    ),
    '6fd0852025468e2583c436d37af18383bf2e71ea': (
        '1eb064834e310a2006b3df6c5981ab15a07afc12',
        '1fd6c33cf97f94bfea5afd6ae135c60a6427121f58188d517ec80059e0623042',
    ),
}
REPLAY = '''        if let Some(record) = sources.records.iter().find(|r| r.work.request() == intent) {
            return Ok(record.clone());
        }
'''
REPLAY_LOOP = '''        let mut index = 0usize;
        while index < sources.records.len() {
            if sources.records[index].work.request() == intent {
                break;
            }
            index += 1;
        }
        if index < sources.records.len() {
            return Ok(sources.records[index].clone());
        }
'''
CAPACITY = '''        if serde_json::to_vec(&work)?.len() as u64 > u64::from(sources.config.max_work_bytes)
            || sources
                .records
                .iter()
                .filter(|record| {
                    record.work.request().request.binding.session == intent.request.binding.session
                })
                .count() as u64
                >= codec::records_per_session(&tx)?
        {
            return Err(JournalError::MaintenanceCapacity);
        }
'''
CAPACITY_LOOP = '''        if serde_json::to_vec(&work)?.len() as u64 > u64::from(sources.config.max_work_bytes) {
            return Err(JournalError::MaintenanceCapacity);
        }
        let mut session_records = 0usize;
        let mut index = 0usize;
        while index < sources.records.len() {
            if sources.records[index]
                .work
                .request()
                .request
                .binding
                .session
                == intent.request.binding.session
            {
                session_records += 1;
            }
            index += 1;
        }
        if session_records as u64 >= codec::records_per_session(&tx)? {
            return Err(JournalError::MaintenanceCapacity);
        }
'''
REQUEST_CARRIERS = (
    'sandproof_protocol::lifecycle::request::LifecycleRequest',
    'sandproof_protocol::lifecycle::identity::OperationBinding',
)
ENCODER = prepare.WORK + '::encode_prepared_source_work'
ENCODER_RUST = '''fn encode_prepared_source_work(work: &SourceWork) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(work)
}

'''
VARIANTS = ('replay-only', 'replay-and-capacity', 'json-boundary')


def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def rewrite(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown source rewrite')
    first = text.index('    pub fn prepare_lifecycle_source_work<')
    last = text.index('    /// Inspect the original finite responsibilities', first)
    body = text[first:last]
    pairs = [(REPLAY, REPLAY_LOOP)]
    if variant != 'replay-only':
        pairs.append((CAPACITY, CAPACITY_LOOP))
    for old, new in pairs:
        if body.count(old) != 1:
            raise ValueError('exact reviewed first Unload body is required')
        body = body.replace(old, new, 1)
    if variant == 'json-boundary':
        guard = 'if serde_json::to_vec(&work)?.len() as u64 > u64::from(sources.config.max_work_bytes) {'
        if body.count(guard) != 1:
            raise ValueError('the original source work encoding must occur once')
        body = body.replace(guard, 'if encode_prepared_source_work(&work)?.len() as u64\n'
                            '            > u64::from(sources.config.max_work_bytes)\n        {', 1)
        insert = text.index('impl ControllerJournal {')
        return text[:insert] + ENCODER_RUST + text[insert:first] + body + text[last:]
    return text[:first] + body + text[last:]


def target_bytes(root):
    size = 0
    for path in root.rglob('*'):
        try:
            info = path.lstat()
        except FileNotFoundError:
            continue  # rustc removes intermediate objects as it builds.
        if not stat.S_ISLNK(info.st_mode):
            size += info.st_blocks * 512
    return size


def stop(child):
    for signum in (signal.SIGTERM, signal.SIGKILL):
        if child.poll() is not None:
            return
        try:
            os.killpg(child.pid, signum)
        except ProcessLookupError:
            pass
        try:
            child.wait(timeout=5)
            return
        except subprocess.TimeoutExpired:
            continue
    raise RuntimeError('bounded extraction process did not stop')


def run(label, argv, seconds, source, output, env):
    started = time.monotonic()
    log = output / (label + '.log')
    peak = 0
    bounded = False
    with log.open('xb') as results:
        child = subprocess.Popen(argv, cwd=source, env=env, stdout=results,
                                 stderr=subprocess.STDOUT, start_new_session=True)
        try:
            while child.poll() is None:
                peak = max(peak, target_bytes(output / 'target'))
                if time.monotonic() - started > seconds or peak > 4 * GIB or shutil.disk_usage(ROOT).free < 64 * GIB:
                    bounded = True
                    break
                time.sleep(0.5)
        finally:
            stop(child)
    return dict(label=label, argv=list(map(str, argv)), exit_code=child.returncode,
                seconds=time.monotonic() - started, bounded_stop=bounded,
                peak_target_bytes=peak, log_sha256=digest(log))


def check_encoder_wrapper(translated, encoder):
    to_vec = declaration(translated['fun_decls'], 'serde_json::ser::to_vec')
    if encoder['item_meta']['opacity'] != 'Transparent':
        raise ValueError('encoder must have a real transparent Rust body')
    body = encoder['body']['Structured']['body']
    steps = [row['kind'] for row in body['statements']]
    if ([step if isinstance(step, str) else next(iter(step)) for step in steps] !=
            ['StorageLive', 'StorageLive', 'Assign', 'Call', 'StorageDead', 'StorageDead', 'Return']
            or list(regular_calls(body)) != [to_vec['def_id']]):
        raise ValueError('encoder must make the one original serde_json call')
    input_ty, = encoder['signature']['inputs']
    _, source_ty, kind = input_ty['Value'][1]['Ref']
    source_type_id, source_type = source_ty['Value']
    source = declaration(translated['type_decls'], prepare.WORK + '::SourceWork')
    place, borrow = steps[2]['Assign']
    call = steps[3]['Call']['call']
    move, = call['args']
    input_place, projection = borrow['Ref']['place']['kind']['Projection']
    if (kind != 'Shared' or source_type['Adt']['id'] != source['def_id']
            or borrow['Ref']['kind'] != 'Shared' or projection != 'Deref' or input_place['kind'] != {'Local': 1}
            or move != {'Move': place} or call['dest']['kind'] != {'Local': 0}
            or call['func']['Regular']['generics']['types'] != [{'Deduplicated': source_type_id}]):
        raise ValueError('encoder must pass its complete original SourceWork and return the original result')
    return to_vec['def_id']


def probe(variant, output, tools, lock, wrapper_ir, source_commit, frontier=None):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('extraction probe belongs to the existing proof role')
    if shutil.disk_usage(ROOT).free < 80 * GIB:
        raise ValueError('at least 80 GiB workspace free required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    source_tree, original_archive = BASELINES[source_commit]
    report = dict(variant=variant, source_commit=source_commit, source_tree=source_tree,
                  cpus=cpus, development_probe=True, original_implementation=False,
                  commands=[], generated=False, wrapper_ir=wrapper_ir, native_acceptance=False,
                  driver_sha256=digest(Path(__file__)))
    if frontier is not None:
        report['frontier'] = frontier
        report['frontier_driver_sha256'] = digest(Path(frontier_capture.__file__))
    try:
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_bytes())
        charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
        for name, path in (('charon', charon), ('aeneas', aeneas)):
            if digest(path) != fixed[name]['binary_sha256']:
                raise ValueError('fixed extraction tool differs: ' + name)
        archive = output / 'source.tar'
        found_tree = subprocess.check_output(['git', 'rev-parse', source_commit + '^{tree}'], cwd=ROOT, text=True).strip()
        if found_tree != source_tree:
            raise ValueError('original integration tree differs')
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', source_commit], cwd=ROOT, stdout=stream, check=True)
        report['archive_sha256'] = digest(archive)
        if report['archive_sha256'] != original_archive:
            raise ValueError('original integration archive differs')
        with tarfile.open(archive) as tar:
            tar.extractall(source, filter='data')
        for name, expected in prepare.SOURCE_INPUTS.items():
            if digest(source / name) != expected:
                raise ValueError('original capture input differs: ' + name)
        path = source / prepare.SOURCE
        original = path.read_text()
        changed = rewrite(original, variant)
        if frontier is not None:
            changed = frontier_capture.factor(changed, reordered=frontier == 'reordered')
        path.write_text(changed)
        patch = ''.join(difflib.unified_diff(original.splitlines(keepends=True), changed.splitlines(keepends=True),
                        fromfile='a/' + prepare.SOURCE, tofile='b/' + prepare.SOURCE))
        (output / 'source.patch').write_text(patch)
        report['candidate_source_sha256'] = digest(path)
        report['patch_sha256'] = digest(output / 'source.patch')

        profile = ROOT / 'verification/experiments/stage1-protocol/unload-startup-source/capture-profile.json'
        cfg = prepare.configuration(json.loads(profile.read_bytes())['parent'])
        if variant != 'replay-only':
            cfg['include'].extend(REQUEST_CARRIERS)
            report['transparent_request_carriers'] = list(REQUEST_CARRIERS)
        if wrapper_ir:
            cfg.update(roots=[ENCODER], retained_roots=[], include=[ENCODER])
        if frontier is not None:
            cfg = frontier_capture.configure(cfg, frontier)
        selectors = [item for flag, key in [
            ('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
            ('--include', 'include'), ('--opaque', 'opaque'),
        ] for value in cfg[key] for item in (flag, value)]
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        ir = output / 'actual.llbc'
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            commands = [
                ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                            *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                            '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ]
            if not wrapper_ir:
                commands.append(('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                                             '-split-files', '-emit-json', '-namespace', cfg['namespace'],
                                             '-subdir', cfg['subdir'], '-dest', output / 'generated', ir], 120))
            for label, argv, seconds in commands:
                step = run(label, argv, seconds, source, output, env)
                report['commands'].append(step)
                if step['exit_code'] != 0 or step['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = digest(ir)
        if digest(ir) != report['ir_sha256']:
            raise ValueError('extracted IR changed')
        if variant == 'json-boundary' and frontier in (None, 'parent'):
            translated = json.loads(ir.read_bytes())['translated']
            encoder = declaration(translated['fun_decls'], ENCODER)
            report['encoder_id'] = encoder['def_id']
        if frontier is not None:
            report['frontier_admission'] = frontier_capture.admit(ir, output / 'generated', cfg, fixed['target'], frontier)
        elif wrapper_ir:
            report['serde_to_vec_id'] = check_encoder_wrapper(translated, encoder)
        else:
            loops = 1 if variant == 'replay-only' else 2
            admitted = prepare.admit(ir, output / 'generated', cfg, fixed['target'], expected_loops=loops)
            report['candidate_calls'] = admitted['calls']
            report['lean_name'] = admitted['lean_name']
            if variant == 'json-boundary':
                caller = prepare.method(translated['fun_decls'], prepare.WORK, 'prepare_lifecycle_source_work')
                calls = list(regular_calls(caller['body']['Structured']['body']))
                if calls.count(encoder['def_id']) != 1:
                    raise ValueError('parent must call the concrete encoding boundary once')
        if not wrapper_ir:
            report['generated_sha256'] = {
                str(path.relative_to(output / 'generated')): digest(path)
                for path in (output / 'generated').rglob('*') if path.is_file()
            }
            report['generated'] = True
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        report['error'] = str(error)
    finally:
        (output / 'probe-result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({key: value for key, value in report.items() if key != 'commands'}, indent=2))
    return not (report['generated'] or 'serde_to_vec_id' in report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=VARIANTS, required=True)
    parser.add_argument('--source-commit', choices=tuple(BASELINES), default=prepare.SOURCE_COMMIT)
    parser.add_argument('--wrapper-ir', action='store_true', help='check the concrete encoder body with Charon only')
    parser.add_argument('--frontier', choices=tuple(frontier_capture.MODULES))
    for name in ('output', 'tools', 'lock'):
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    if args.wrapper_ir and args.variant != 'json-boundary':
        parser.error('--wrapper-ir requires the json-boundary variant')
    if args.frontier is not None and (args.variant != 'json-boundary' or args.wrapper_ir):
        parser.error('--frontier requires an ordinary json-boundary capture')
    raise SystemExit(probe(args.variant, args.output.resolve(), args.tools.resolve(strict=True),
                           args.lock.resolve(strict=True), args.wrapper_ir, args.source_commit, args.frontier))
