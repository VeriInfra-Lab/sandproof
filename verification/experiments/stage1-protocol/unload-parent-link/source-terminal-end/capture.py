"""Capture the complete original terminal end and its stop-only replay mutant."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_terminal_end_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'stop-only-replay')
ROOT = retire.ports.WORK + '::full::drain::terminal_end'
NAMESPACE = 'SandproofSourceTerminalEnd'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT])
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/drain.rs')
BASE_STAGE, BASE_STAGED = retire.reconstruct, retire.STAGED
STAGED = (*BASE_STAGED, END)
OMITTED = ' || **previous != observation'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown original terminal-end capture')
    start = text.index('fn terminal_end(')
    stop = text.index('\n/// Check saved bytes', start)
    body = text[start:stop]
    if body.count(OMITTED) != 1:
        raise ValueError('the one exact complete saved observation comparison is required')
    return text[:start] + (body if variant == 'original' else body.replace(OMITTED, '', 1)) + text[stop:]


def reconstruct(archive, source, variant):
    retire.STAGED = BASE_STAGED
    try:
        BASE_STAGE(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    path.write_text(mutate(path.read_text(), variant))


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, None)
    entry = retire.declaration(tree['fun_decls'], ROOT)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    rows = [r for r in mapping if r['def_id'] == entry['def_id']]
    if 'Structured' not in (entry['body'] or {}) or not rows or any(r['is_opaque'] for r in rows):
        raise ValueError('the whole original terminal_end must stay structured')
    if any(r['is_opaque'] and r['source']['file'].startswith(
            ('controller-service/', 'controller-core/', 'sandbox-protocol/')) for r in mapping):
        raise ValueError('no owned complete clone/equality or business root may be hidden')
    return dict(root=entry['def_id'], transparent_owned=True,
        libraries=sorted(r['lean_name'] for r in mapping if r['is_opaque']))


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS, retire.STAGED = ROOT, NAMESPACE, OPTIONS, STAGED
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            result = json.loads(path.read_bytes())
            result.update(base_driver=result['driver'], driver=digest(Path(__file__)), capture_root=ROOT,
                direct_original_root=True, whole_receipt_proved=False,
                mutation=None if variant == 'original' else 'omit only complete saved observation equality in terminal_end')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
