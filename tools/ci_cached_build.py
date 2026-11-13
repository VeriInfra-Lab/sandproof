"""Offline Cargo preparation with a stable, exclusive cache and separate evidence.

This does not reuse test results or authorize retirement of runtime resources.
Cargo still checks current source/dependency fingerprints on every invocation.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import time

from tools.system_acceptance import digest, private_path, write_once


def build(source, cache, lane, evidence, arguments):
    source = Path(source).resolve(strict=True)
    cache = private_path(cache, directory=True)
    evidence = private_path(evidence, directory=True)
    if (not re.fullmatch('[a-z][a-z0-9-]{0,47}', lane)
            or not arguments or arguments[0] not in ('build', 'check', 'clippy')
            or any(a.split('=')[0] in ('--target-dir', '--message-format', '--manifest-path',
                                      '--config') for a in arguments)
            or '--' in arguments):
        raise ValueError('explicit Cargo build/check/clippy arguments; no target or source overrides')
    if evidence.is_relative_to(cache) or list(evidence.iterdir()):
        raise ValueError('new independent evidence directory required')
    compiler = subprocess.check_output(['rustc', '-vV'], cwd=source, text=True)
    key = hashlib.sha256(compiler.encode()).hexdigest()[:16]
    target = cache / (lane + '-' + key)
    target.mkdir(mode=0o700, exist_ok=True)
    private_path(target, directory=True)
    lock = cache / (lane + '-' + key + '.lock')
    with open(lock, 'a', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
        # One lane must not mix concurrent preparation; callers may allocate
        # another finite lane instead of creating a cache for every commit.
        fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE='true')
        command = ['cargo', *arguments, '--locked', '--offline', '--message-format=json-render-diagnostics']
        identity = dict(source=str(source), compiler=compiler,
            lock_sha256=digest(source/'Cargo.lock'), command=command,
            cargo_target=str(target), cargo_home=env.get('CARGO_HOME',str(Path.home()/'.cargo')),
            build_environment={k:v for k,v in env.items() if k.startswith('CARGO_PROFILE_')
                or k in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_TARGET')})
        write_once(evidence/'input.json', identity)
        started = time.monotonic()
        with (evidence/'cargo.jsonl').open('x') as output, (evidence/'cargo.stderr').open('x') as errors:
            code = subprocess.run(command, cwd=source, env=env, stdout=output, stderr=errors).returncode
        counts = {'fresh':0, 'compiled':0}
        for line in (evidence/'cargo.jsonl').read_text().splitlines():
            try: item = json.loads(line)
            except ValueError: continue
            if item.get('reason') == 'compiler-artifact':
                counts['fresh' if item.get('fresh') is True else 'compiled'] += 1
        result = dict(exit_code=code, seconds=time.monotonic()-started,
            artifacts=counts, cargo_target=str(target), offline=True,
            input_sha256=digest(evidence/'input.json'),
            test_or_proof_acceptance_claimed=False)
        write_once(evidence/'result.json',result)
        return result


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,required=True)
    parser.add_argument('--cache',type=Path,required=True)
    parser.add_argument('--lane',required=True)
    parser.add_argument('--evidence',type=Path,required=True)
    parser.add_argument('cargo_args',nargs=argparse.REMAINDER)
    args=parser.parse_args(argv)
    arguments=args.cargo_args[1:] if args.cargo_args[:1]==['--'] else args.cargo_args
    result=build(args.source,args.cache,args.lane,args.evidence,arguments)
    print(json.dumps(result))
    return result['exit_code'] if result['exit_code'] >= 0 else 1


if __name__=='__main__':
    raise SystemExit(main())
