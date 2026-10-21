"""Re-extract the unchanged directory body and one real Rust bitmask mutant."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

from prepare import prepare, sha, SOURCE, VARIANTS

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
LIBRARY = Path('verification/aeneas/protected-directory')


def snapshot(root):
    return {str(p.relative_to(root)): sha(p) for p in sorted(root.rglob('*'))
            if p.is_file() and (p.suffix == '.rs' or p.name in
                ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'))
            and p.relative_to(root).parts[0] not in ('.git', '.lake', 'target', 'artifacts')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--tool-root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--publish', action='store_true',
                        help='initial development only: create previously absent generated files')
    args = parser.parse_args()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('registered formal Linux runtime and private target required')
    out, tools = args.output.resolve(), args.tool_root.resolve()
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    sys.path.insert(0, str(ROOT))
    from tools.check_first_chain_extraction import check_tools
    from tools.check_foundations_extraction import llbc_digest
    check_tools(ROOT, tools)
    record = {'kind': 'real-directory-component-extraction', 'variants': {},
              'tool_registry_sha256': sha(ROOT / 'verification/first-chain-linux-environment.json')}
    start = time.monotonic()
    for variant in VARIANTS:
        case = out / variant
        case.mkdir()
        source = case / 'source'
        entry = prepare(args.archive.resolve(), source, variant)
        before = snapshot(source)
        raw = case / 'raw'
        raw.mkdir()
        llbc = raw / 'original.llbc'
        env = dict(os.environ, CARGO_INCREMENTAL='0',
                   RUSTFLAGS='--remap-path-prefix=' + str(source) + '=.')
        prefix = 'SandproofProtectedDirectory' + ('Bad' if variant == 'bypass-writes' else '')
        commands = [
            ('charon', [str(tools / 'aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
             '--error-on-warnings', '--sysroot', 'default', '--targets',
             'x86_64-unknown-linux-gnu', '--start-from', 'crate::protected_files::directory',
             '--include', 'crate::protected_files', '--dest-file', '../raw/original.llbc',
             '--', '--manifest-path', 'sandbox-internal/Cargo.toml', '--lib',
             '--locked', '--offline', '--ignore-rust-version']),
            ('aeneas', [str(tools / 'aeneas/bin/aeneas'), '-backend', 'lean', '-abort-on-error',
             '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', prefix + 'Rust',
             '-subdir', prefix, '-dest', str(case / 'generated'), str(llbc)])]
        entry['commands'] = []
        for label, command in commands:
            log = raw / (label + '.log')
            began = time.monotonic()
            with log.open('xb') as stream:
                code = subprocess.run(command, cwd=source, env=env, stdout=stream,
                    stderr=subprocess.STDOUT, timeout=900).returncode
            entry['commands'].append({'label': label, 'command': command,
                'exit_code': code, 'seconds': time.monotonic() - began, 'log_sha256': sha(log)})
            (case / 'progress.json').write_text(json.dumps(entry, indent=2) + '\n')
            if code:
                raise ValueError('real ' + variant + ' ' + label + ' failed; preserve raw evidence')
            if label == 'charon':
                tree = json.loads(llbc.read_text())
                local = [f for f in tree['translated']['fun_decls']
                         if f and f['item_meta']['is_local']]
                if (tree.get('has_errors') is not False or len(local) != 1
                        or local[0]['item_meta']['name'][-1] != {'Ident': ['directory', 0]}
                        or not isinstance(local[0]['body'], dict)
                        or 'Structured' not in local[0]['body']
                        or local[0]['signature']['is_unsafe']):
                    raise ValueError('actual original transparent directory body is required')
                entry['llbc_sha256'] = sha(llbc)
                entry['selected_declaration_sha256'] = hashlib.sha256(
                    json.dumps(local[0], sort_keys=True).encode()).hexdigest()
                entry['source_inputs'] = before
        entry['source_unchanged'] = before == snapshot(source)
        if not entry['source_unchanged']:
            raise ValueError('real Rust inputs changed during extraction')
        destination = ROOT / LIBRARY / variant
        if args.publish:
            destination.mkdir(parents=True, exist_ok=False)
            shutil.copy2(llbc, destination / 'original.llbc')
        else:
            if not destination.is_dir():
                raise ValueError('frozen generated files missing; no silent publishing')
            shutil.copytree(destination, case / 'previous-published')
            if llbc_digest(json.loads(llbc.read_text())) != llbc_digest(
                    json.loads((destination / 'original.llbc').read_text())):
                raise ValueError('fresh complete IR differs; preserve both originals')
        generated = case / 'generated'
        entry['generated_files'] = {}
        for p in sorted(generated.rglob('*')):
            if p.is_file():
                relative = p.relative_to(generated)
                entry['generated_files'][str(relative)] = sha(p)
                # Keep templates in raw evidence only; never adopt their axioms.
                if '_Template' in p.name:
                    continue
                target = destination / relative
                if args.publish:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(p, target)
                elif not target.is_file() or sha(p) != sha(target):
                    raise ValueError('unmodified generated file differs: ' + str(relative))
        record['variants'][variant] = entry
        (out / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
    record.update(success=True, seconds=time.monotonic() - start)
    (out / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({'success': True, 'seconds': record['seconds'],
                      'variants': list(record['variants'])}))


if __name__ == '__main__':
    main()
