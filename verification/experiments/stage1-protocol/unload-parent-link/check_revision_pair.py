"""Read back the two original #893 captures before constructing a Lean bridge."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import stat
import subprocess

import revision_capture


HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
DRIVER = '476a100a70365dbddd1904ffac505f622074923b'


def digest(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError(f'original regular artifact required: {path.name}')
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT)


def source_identity(head, driver=DRIVER):
    if re.fullmatch(r'[0-9a-f]{40}', head) is None:
        raise ValueError('one exact source commit required')
    if git('rev-parse', '--verify', head + '^{commit}').decode().strip() != head:
        raise ValueError('source must be the exact requested commit')
    reviewed = git('show', f'{head}:{revision_capture.REVISION_SOURCE}')
    if hashlib.sha256(reviewed).hexdigest() != revision_capture.REVISION_SOURCE_SHA256:
        raise ValueError('reviewed current readiness source changed')
    # The original capture uses umask 077, which also affects tar.umask=user.
    with subprocess.Popen(['git', 'archive', head], cwd=ROOT, stdout=subprocess.PIPE, umask=0o077) as process:
        archive_digest = hashlib.file_digest(process.stdout, 'sha256').hexdigest()
        if process.wait() != 0:
            raise ValueError('independent Git archive could not be read')
    return dict(driver_head=driver, head=head,
                tree=git('rev-parse', head + '^{tree}').decode().strip(),
                source_archive_sha256=archive_digest)


def fixed_configuration(driver):
    if (re.fullmatch(r'[0-9a-f]{40}', driver) is None
            or git('rev-parse', '--verify', driver + '^{commit}').decode().strip() != driver):
        raise ValueError('one exact capture driver commit required')
    parent_path = HERE.parent.relative_to(ROOT) / 'unload-startup-source/capture-profile.json'
    fixed_path = Path('verification/foundations-environment.json')
    driver_dir = HERE.relative_to(ROOT)
    for path in (parent_path, fixed_path, driver_dir / 'capture.py'):
        if git('show', f'{driver}:{path}') != git('show', f'{DRIVER}:{path}'):
            raise ValueError(f'fixed capture control differs: {path.name}')
    admission_path = driver_dir / 'revision_capture.py'
    if git('show', f'{driver}:{admission_path}') != (ROOT / admission_path).read_bytes():
        raise ValueError('capture driver admission differs from the checker')
    parent = json.loads(git('show', f'{driver}:{parent_path}'))['parent']
    parent.update(namespace='SandproofUnloadFirstObservation', subdir='SandproofUnloadFirstObservation')
    fixed = json.loads(git('show', f'{driver}:{fixed_path}'))
    return parent, fixed['target']


def checked_commands(directory, rows, cfg, target):
    if [row['label'] for row in rows] != ['charon', 'aeneas']:
        raise ValueError('one original Charon command followed by one original Aeneas command required')
    for row in rows:
        if (row.get('bounded_stop', False) is not False or type(row['exit_code']) is not int
                or row['exit_code'] != 0 or digest(directory / (row['label'] + '.log')) != row['log_sha256']):
            raise ValueError('original command failed, was bounded, or its log changed')
        argv = row['argv']
        if (type(argv) is not list or not argv or any(type(arg) is not str for arg in argv)
                or not Path(argv[0]).is_absolute() or Path(argv[0]).name != row['label']):
            raise ValueError('original tool invocation required')
    charon, aeneas = (row['argv'][1:] for row in rows)
    if '--dest-file' not in charon or charon.index('--dest-file') + 1 == len(charon):
        raise ValueError('original Charon output path required')
    ir = Path(charon[charon.index('--dest-file') + 1])
    if not ir.is_absolute() or ir.name != 'actual.llbc':
        raise ValueError('original Charon output path required')
    selectors = [arg for flag, key in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                       ('--include', 'include'), ('--opaque', 'opaque'))
                 for value in cfg.get(key, ()) for arg in (flag, value)]
    if charon != ['cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default', *selectors,
                  '--dest-file', str(ir), '--', '--manifest-path', 'controller-service/Cargo.toml',
                  '--lib', '--locked', '--offline', '--target', target]:
        raise ValueError('original Charon arguments differ')
    if aeneas != ['-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
                  '-namespace', cfg['namespace'], '-subdir', cfg['subdir'],
                  '-dest', str(ir.parent / 'generated'), str(ir)]:
        raise ValueError('original Aeneas arguments differ or use another IR')


def generated_inventory(directory):
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('original generated directory required')
    inventory = {}
    for path in sorted(directory.rglob('*')):
        if not stat.S_ISDIR(path.lstat().st_mode):
            inventory[path.relative_to(directory).as_posix()] = digest(path)
    return inventory


def checked_capture(directory, identity, cfg, target, mode):
    record = directory / 'result.json'
    if not stat.S_ISREG(record.lstat().st_mode):
        raise ValueError('original regular result required')
    result = json.loads(record.read_bytes())
    if (result['complete'] is not True or result['development_only'] is not True
            or result['native_acceptance'] is not False or 'error' in result
            or any(result.get(key) != expected for key, expected in identity.items())):
        raise ValueError(f'{mode} capture differs from the complete expected driver/source identity')
    if digest(directory / 'source.tar') != identity['source_archive_sha256']:
        raise ValueError(f'{mode} original archive differs from Git')
    checked_commands(directory, result['commands'], cfg, target)
    ir, generated = directory / 'actual.llbc', directory / 'generated'
    if digest(ir) != result['ir_sha256'] or generated_inventory(generated) != result['generated']:
        raise ValueError(f'{mode} IR or complete generated file inventory changed')
    admission = revision_capture.admit(ir, generated, cfg, target, mode)
    if json.dumps(admission, sort_keys=True) != json.dumps(result['revision_admission'], sort_keys=True):
        raise ValueError(f'{mode} recorded admission differs from the original capture')
    if digest(ir) != result['ir_sha256'] or generated_inventory(generated) != result['generated']:
        raise ValueError(f'{mode} generated input changed during original admission')
    return dict(admission=admission, ir_sha256=result['ir_sha256'], generated=result['generated'],
                logs={row['label']: row['log_sha256'] for row in result['commands']})


def check_pair(base, parent, identity, cfg, target):
    if base.samefile(parent):
        raise ValueError('two distinct original capture directories required')
    reports = {mode: checked_capture(directory, identity, revision_capture.configuration(cfg, mode), target, mode)
               for mode, directory in (('base-revision', base), ('parent', parent))}
    if (reports['base-revision']['admission']['ready_candidate_fields']
            != reports['parent']['admission']['ready_candidate_fields']):
        raise ValueError('base and parent ReadyCandidate field inventories differ')
    return dict(development_pair_checked=True, native_acceptance=False, public_acceptance=False,
                source=identity, views=reports)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', type=Path, required=True)
    parser.add_argument('--parent', type=Path, required=True)
    parser.add_argument('--source-commit', required=True)
    parser.add_argument('--driver-commit', required=True)
    args = parser.parse_args()
    try:
        cfg, target = fixed_configuration(args.driver_commit)
        result = check_pair(args.base.resolve(strict=True), args.parent.resolve(strict=True),
                            source_identity(args.source_commit, args.driver_commit), cfg, target)
    except (KeyError, OSError, TypeError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Unload revision capture pair rejected: {error}\n')
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
