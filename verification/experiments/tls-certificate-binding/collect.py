"""Recollect the four frozen original TLS bodies; no Aeneas/Lean pass claimed."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

REL = Path('verification/experiments/tls-certificate-binding')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    source, tools, output = args.source.resolve(), args.tool_root.resolve(), args.output.resolve()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('use the registered formal Linux environment and private target')
    output.mkdir(parents=True, exist_ok=False)
    sys.path.insert(0, str(source))
    from tools.check_first_chain_extraction import check_tools
    from tools.check_tls_certificate_binding import check
    check_tools(source, tools)
    check(source)
    frozen = json.loads((source/REL/'environment.json').read_text())
    before = {name: sha(source/name) for name in frozen['sources']}
    target = source/REL/'original.llbc'
    shutil.copy2(target, output/'previous.llbc')
    target.unlink()
    command = [str(tools/'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
               '--error-on-warnings', '--sysroot', 'default', '--targets', 'x86_64-unknown-linux-gnu',
               '--start-from', 'crate::tls::_::verify_peer', '--start-from', 'crate::tls::_::peer_pin',
               '--include', 'crate::tls']
    for name in ['rustls', 'rustls_pki_types', 'sha2', 'digest', 'generic_array', 'typenum', 'crypto_common']:
        command.extend(['--opaque', name])
    command.extend(['--dest-file', (REL/'original.llbc').as_posix(), '--', '--manifest-path',
                    'sandbox-internal/Cargo.toml', '--lib', '--locked', '--offline', '--ignore-rust-version'])
    environment = dict(os.environ, RUSTFLAGS='--remap-path-prefix='+str(source)+'=.', CARGO_INCREMENTAL='0')
    started = time.monotonic()
    with (output/'charon.log').open('xb') as stream:
        code = subprocess.run(command, cwd=source, env=environment, stdout=stream,
                              stderr=subprocess.STDOUT, timeout=900).returncode
    result = {'kind': 'original-Charon-source-admission-not-machine-refinement',
              'charon_exit': code, 'command': command, 'seconds': time.monotonic()-started,
              'sources_unchanged': before == {name: sha(source/name) for name in before},
              'charon_log_sha256': sha(output/'charon.log')}
    if target.is_file():
        shutil.copy2(target, output/'original.llbc')
        result['llbc_sha256'] = sha(target)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    if code or not target.is_file() or not result['sources_unchanged']:
        raise ValueError('actual original-body capture failed; preserve evidence')
    result['admission'] = check(source)
    (output/'result.json').write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({k:v for k,v in result.items() if k!='command'}))


if __name__ == '__main__':
    main()
