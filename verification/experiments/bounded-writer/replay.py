"""Host-side formal component; caller holds a6000's existing formal-heavy lock.

Both physical variants get the same private virtual paths. No shared source,
compiler target or Lean build is overwritten. No native tests run here.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_bounded_writer import ASSETS, FROZEN, LIBRARY, check_frozen, check_pair, check_sources, expected_audits
from audit_messages import component_messages
from prepare import prepare


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def crate_archive(root, output):
    """Stable headers; identity still comes from every frozen source file."""
    with output.open('xb') as raw, gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode='w') as package:
            base = root / 'verification/extract-bounded-writer'
            for path in [base] + sorted(base.rglob('*')):
                if path.is_symlink() or not (path.is_file() or path.is_dir()):
                    raise ValueError('unregistered crate filesystem object')
                info = package.gettarinfo(str(path), arcname=str(path.relative_to(root)))
                info.uid = info.gid = info.mtime = 0
                info.uname = info.gname = ''
                info.mode = 0o755 if path.is_dir() else 0o644
                if path.is_file():
                    with path.open('rb') as stream:
                        package.addfile(info, stream)
                else:
                    package.addfile(info)


def run(root, archive, output, target, seed):
    root = root.resolve(strict=True)
    target = target.resolve(strict=True)
    seed = seed.resolve(strict=True)
    for name in (root, output.resolve(), target, seed):
        name.relative_to('/data1/zn')
    runtime_env = dict(os.environ, SANDPROOF_WRITER_TARGET=str(target), SANDPROOF_WRITER_SEED=str(seed),
                       PYTHONDONTWRITEBYTECODE='1')

    def launch(base, command, log):
        args = ['bash', str(root / ASSETS / 'runtime.sh'), str(base)] + command
        with log.open('xb') as stream:
            code = subprocess.run(args, env=runtime_env, stdout=stream, stderr=subprocess.STDOUT,
                                  timeout=1200).returncode
        return args, code

    return component(root, archive, output, launch, True, {'runtime_kind': 'host-fixed-runtime'})


def component(root, archive, output, launch, complete, metadata, verify_mapping=None):
    """Common admission; full entry has no optional proof/audit skipping switches."""
    if output.exists() or output.is_symlink():
        raise ValueError('new private component directory required')
    frozen = check_frozen(root)
    env = check_sources(root)
    check_pair(root)
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    kind = ('bounded-writer-formal-component-not-full-project-suite' if complete else
            'bounded-writer-extract-only-not-proof-acceptance')
    record = dict(kind=kind, variants={}, runtime=metadata)
    checked_messages = []
    try:
        crate = output / 'crate.tar.gz'
        crate_archive(root, crate)
        record.update(source_archive_sha256=sha(archive), crate_archive_sha256=sha(crate),
                      frozen_inputs=frozen)
        for variant in ('correct', 'no-limit'):
            base = output / variant
            base.mkdir(mode=0o700)
            source = base / 'source'
            prepare(archive, source, crate, variant)
            # The source transformer owns only the three declared production edits.
            # Overlay proof/tool assets, never replace a transformed production file.
            for name in frozen:
                if name in env['source_files']:
                    continue
                destination = source / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(root / name, destination)
            shutil.copyfile(root / FROZEN, source / FROZEN)
            if variant == 'no-limit':
                for name in ('SandproofBoundedWriter/Types.lean', 'SandproofBoundedWriter/Funs.lean', 'translation.json'):
                    shutil.copyfile(root / ASSETS / 'negative/generated' / name, source / LIBRARY / name)
            (source / '.lake').mkdir()
            (source / '.lake/packages').symlink_to(env['runtime']['lean_packages_virtual'], target_is_directory=True)
            (source / '.tools').symlink_to(env['runtime']['tool_root_virtual'], target_is_directory=True)
            (source / 'lakefile.toml').write_text(env['runtime']['private_lakefile'])
            for name in ('tmp', 'target-formal', 'cargo-home', 'evidence'):
                (base / name).mkdir(mode=0o700)
            (base / 'cargo-home/registry').symlink_to('/home/ning/.cargo/registry')
            row = {'component_root': str(base), 'virtual_source': env['runtime']['virtual_source'], 'steps': []}
            record['variants'][variant] = row
            commands = [('extract', ['bash', 'scripts/python-entry.sh', 'verification/experiments/bounded-writer/translate.py',
                    '--variant', variant, '--tool-root', env['runtime']['tool_root_virtual']])]
            if complete:
                commands.append(('proof', ['bash', 'scripts/python-entry.sh', 'verification/experiments/bounded-writer/proof.py',
                    '--variant', variant, '--output', '/tmp/bounded-writer/evidence/proof',
                    '--tool-root', env['runtime']['tool_root_virtual']]))
            for name, command in commands:
                began = time.monotonic()
                args, code = launch(base, command, base / (name + '-launch.log'))
                row['steps'].append(dict(name=name, command=args, exit_code=code,
                    seconds=time.monotonic() - began, log_sha256=sha(base / (name + '-launch.log'))))
                (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
                if code:
                    raise ValueError(variant + '/' + name + ' failed; preserve all prior evidence')
            extraction = json.loads((base / 'evidence/preflight/policy/result.json').read_text())
            if extraction.get('success') is not True:
                raise ValueError('missing complete component evidence')
            if complete:
                proof = json.loads((base / 'evidence/proof/result.json').read_text())
                checked_messages += component_messages(source, base / 'evidence/proof', proof,
                                                        expected_audits(root, variant == 'no-limit'))
                row['audits'] = proof['audits']
                row['physical_files'] = len(proof['files'])
        if check_frozen(root) != frozen:
            raise ValueError('component source/frozen closure changed during execution')
        if verify_mapping:
            verify_mapping()
        record['success'] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic() - start
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    # Emit only messages actually present in fully admitted original service logs,
    # after both complete variants. Extract-only never emits proof audit lines.
    if record['success'] and complete:
        for message in checked_messages:
            print(message, flush=True)
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT)
    parser.add_argument('--source-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--runtime-seed', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.root, args.source_archive.resolve(strict=True), args.output.absolute(), args.target, args.runtime_seed)
    print(json.dumps({k: v for k, v in result.items() if k != 'frozen_inputs'}))
    raise SystemExit(0 if result['success'] else 1)
