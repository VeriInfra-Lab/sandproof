"""Check fresh raw captures, the original source audits and exact parent bodies."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tokens(data):
    # Comments are ignored, strings stay byte-for-byte tokens. Never normalize
    # whitespace inside SQL/string literals or join neighboring identifiers.
    text = data.decode()
    parts = []
    i = 0
    while i < len(text):
        if text[i] == '"':
            start = i
            i += 1
            while i < len(text):
                if text[i] == '\\':
                    i += 2
                elif text[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
            parts.append(text[start:i])
        elif text.startswith('--', i):
            end = text.find('\n', i)
            i = len(text) if end < 0 else end
        elif text.startswith('/-', i):
            depth = 1
            i += 2
            while depth and i < len(text):
                if text.startswith('/-', i):
                    depth += 1
                    i += 2
                elif text.startswith('-/', i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise ValueError('unterminated comment')
            parts.append(' ')
        else:
            parts.append(text[i])
            i += 1
    return re.findall(r'"(?:\\.|[^"\\])*"|\w+|[^\s]', ''.join(parts))


def check(source, parent, project, output):
    output.mkdir(mode=0o700)
    results = {}
    for mode, directory in [('source', source), ('parent', parent)]:
        result = json.loads((directory / 'result.json').read_bytes())
        if not result['complete'] or result.get('native_acceptance') is not False:
            raise ValueError('not a complete development capture')
        if sha(directory / 'actual.llbc') != result['ir_sha256']:
            raise ValueError('changed raw capture')
        for name, expected in result['generated'].items():
            if sha(directory / 'generated' / name) != expected:
                raise ValueError('changed original generated output')
        if len(result['commands']) != 2 or any(row['exit_code'] for row in result['commands']):
            raise ValueError('capture command did not complete')
        for row in result['commands']:
            if sha(directory / (row['label'] + '.log')) != row['log_sha256']:
                raise ValueError('changed raw extraction log')
        results[mode] = dict(source_base=result['head'], ir_sha256=result['ir_sha256'], generated=result['generated'])
    if results['source']['source_base'] != results['parent']['source_base']:
        raise ValueError('source and parent were captured from different candidates')
    generated = ROOT / 'verification/aeneas/stage1-protocol/unload-parent-link/SandproofOriginalSourceRows'
    for name in ['Types.lean', 'Funs.lean']:
        if (generated / name).read_bytes() != (source / 'generated/SandproofOriginalSourceRows' / name).read_bytes():
            raise ValueError('published source rows are not the raw current tool output')
    rows = HERE.parent / 'original-source-rows/original-capture'
    raw = json.loads((source / 'actual.llbc').read_bytes())
    cfg = json.loads((rows / 'precheck-config.json').read_bytes())
    for name, function in [('admit-original-source.py', 'audit'), ('actual-source-audit.py', 'check')]:
        spec = importlib.util.spec_from_file_location('original_audit', rows / name)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        report = getattr(module, function)(raw, cfg, output)
        if report.get('complete') is not True:
            raise ValueError('original full source audit did not complete')
    for name in ['Types.lean', 'Funs.lean']:
        raw_file = parent / 'generated/SandproofUnloadFirstObservation' / name
        view_file = project / 'SandproofUnloadFirstObservation' / name
        original = raw_file.read_bytes()
        view = view_file.read_bytes()
        view = re.sub(rb'(?m)^variable \[Representations\](?: \[ParentCalls\])?\n', b'', view)
        view = view.replace(b' [Representations] [ParentCalls]', b'')
        if name == 'Types.lean':
            # The exact three helper-attribute isolations are already checked
            # by prepare.py's original #772 reversible metadata inventory.
            original = original.replace(b'@[discriminant isize]\n', b'')
            original = re.sub(rb'(?m)^@\[discriminant isize, (rust_type[^\]]+)\]\n',
                              rb'@[\1]\n', original)
        if tokens(original) != tokens(view):
            raise ValueError('parent computational tokens differ: ' + name)
    results.update(development_only=True, public_acceptance=False, complete=True,
                   parent_computational_tokens_identical=True, original_source_audits_complete=True)
    (output / 'result.json').write_text(json.dumps(results, indent=2) + '\n')
    print('raw generated files, full original source audits and parent computational tokens checked')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'parent', 'project', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    check(args.source.resolve(strict=True), args.parent.resolve(strict=True), args.project.resolve(strict=True), args.output.resolve())
