"""Independent JSON view of the unchanged V0 roots plus the reviewed role roots.

The lane root stays in the public full-V0 script. Its current RetentionLane type
pulls unrelated protocol discriminants into that view, which collide with the
independent Request view. This component selects all the same options and roots
except lane_current; no translated definition is edited or replaced.
"""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import shlex
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/envelope-bytes')
DEST = Path('verification/aeneas/stage1-protocol/json-foundations')
IR = Path('artifacts/envelope-json-foundations.llbc')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def selection(script):
    start = script.index('"$CHARON" cargo ')
    end = script.index('\ntest -s "$LLBC"', start)
    args = shlex.split(script[start:end].replace('\\\n', ' '))
    if args[0] != '$CHARON' or args.count('crate::lane_current') != 1:
        raise ValueError('reviewed original invocation changed')
    at = args.index('crate::lane_current')
    if args[at-1] != '--start-from':
        raise ValueError('lane root is not independently selectable')
    del args[at-1:at+1]
    target = args.index('--dest-file')+1
    if args[target] != '../aeneas/llbc/sandproof_foundations.llbc':
        raise ValueError('unexpected original Charon destination')
    args[target] = '../../' + str(IR)
    roots = [args[i+1] for i, flag in enumerate(args) if flag == '--start-from']
    if any(r.startswith('crate::protocol_leaves::') for r in roots):
        raise ValueError('the public extraction script must stay unchanged')
    roles = ['crate::protocol_leaves::' + role + '_decode' for role in (
        'session', 'center', 'incarnation', 'capture', 'request_window',
        'request_sequence', 'retention_window')]
    insertion = args.index('--')
    args[insertion:insertion] = [item for role in roles for item in ('--start-from', role)]
    roots += roles
    return args[1:], roots


def run():
    data = json.loads((ROOT / HERE / 'inventory.json').read_text())
    source = ROOT / 'scripts/regenerate-foundations-lean.sh'
    if sha(source) != data['inputs']['scripts/regenerate-foundations-lean.sh']:
        raise ValueError('unreviewed original generation script')
    args, roots = selection(source.read_text())
    fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
    charon = ROOT / '.tools/aeneas/charon/bin/charon'
    aeneas = ROOT / '.tools/aeneas/bin/aeneas'
    for name, tool in [('charon', charon), ('aeneas', aeneas)]:
        if sha(tool) != fixed[name]['binary_sha256']:
            raise ValueError('fixed binary differs: ' + name)
    ir = ROOT / IR
    if ir.exists():
        raise ValueError('fresh output required; retain the previous LLBC separately')
    ir.parent.mkdir(exist_ok=True)
    env = dict(os.environ, RUSTFLAGS='--remap-path-prefix=' + str(ROOT) + '=.')
    if os.environ.get('RUSTFLAGS') or os.environ.get('CARGO_ENCODED_RUSTFLAGS'):
        raise ValueError('unexpected Rust compiler flags')
    subprocess.run([str(charon), *args], cwd=ROOT, env=env, check=True)
    raw = json.loads(ir.read_text())
    if raw.get('has_errors') is not False or {x['key'] for x in
            raw['translated']['target_information']} != {fixed['target']}:
        raise ValueError('incomplete or wrong-target actual Charon input')
    destination = ROOT / DEST
    destination.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(aeneas), '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
        '-split-files', '-emit-json', '-namespace', 'SandproofFoundationsRust',
        '-subdir', 'SandproofFoundations', '-dest', str(destination), str(ir)], cwd=ROOT, check=True)
    # These are the unchanged reviewed provider files, never generated templates.
    for name in ['FunsExternal.lean', 'TypesExternal.lean', 'IdentifierLibrary.lean']:
        original = ROOT / 'verification/aeneas/foundations/SandproofFoundations' / name
        shutil.copy2(original, destination / 'SandproofFoundations' / name)
    spec = importlib.util.spec_from_file_location('protocol_independent_view',
        ROOT / HERE.parent / 'local_view.py')
    view = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(view)
    original = ROOT / 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'
    (destination / 'SandproofFoundations/FunsExternal.lean').write_text(view.role_provider(original))
    templates = list(destination.rglob('*_Template.lean'))
    archive = ROOT / 'artifacts/envelope-json-templates'
    archive.mkdir()
    for path in templates:
        shutil.move(path, archive / path.name)
    if 'LifecycleOperation' in (destination / 'SandproofFoundations/Types.lean').read_text():
        raise ValueError('unrelated protocol type still enters scoped JSON view')
    record = dict(scope='independent JSON/role view; full V0 lane stays unchanged',
        original_script_sha256=sha(source), omitted_roots=['crate::lane_current'],
        selected_roots=roots, charon_arguments=args, llbc_sha256=sha(ir),
        generated_sha256={str(p.relative_to(ROOT)): sha(p) for p in sorted(destination.rglob('*')) if p.is_file()})
    (ROOT / 'artifacts/envelope-json-extraction.json').write_text(json.dumps(record, indent=2)+'\n')


if __name__ == '__main__':
    run()
