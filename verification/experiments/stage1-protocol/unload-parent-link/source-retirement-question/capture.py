"""Capture the complete original selected-record retirement-question tail.

The original Iterator::find root retains its fixed-tool lifetime failure.
Only its verbatim request-check/return tail is factored; no search algorithm
or whole-business lookup is substituted or hidden behind a trusted port.
"""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))
_spec = importlib.util.spec_from_file_location('source_question_capture_parent', PARENT / 'extract.py')
retire = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(retire)
VARIANTS = ('original', 'sandbox-only-request')
ROOT = retire.ports.WORK + '::retirement::check_question_record'
NAMESPACE = 'SandproofSourceRetirementQuestion'
OPTIONS = dict(retire.OPTIONS, roots=[ROOT])
BODY = '''    if request(record)? != question.request {
        return Err(JournalError::Conflict);
    }
    Ok(record)
'''
WRONG = BODY.replace('request(record)? != question.request',
                     'request(record)?.sandbox() != question.request.sandbox()')
_base_stage = retire.reconstruct


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown original retirement-question view')
    start = text.index("fn find<'a>(")
    stop = text.index('\nimpl ControllerJournal {', start)
    actual = text[start:stop]
    if actual.count(BODY) != 1 or not actual.endswith(BODY + '}\n'):
        raise ValueError('the complete original find tail must occur exactly once')
    return text[:start] + actual.replace(BODY, WRONG if variant != 'original' else BODY) + text[stop:]


def factor(text, variant):
    text = mutate(text, variant)
    start = text.index("fn find<'a>(")
    stop = text.index('\nimpl ControllerJournal {', start)
    actual = text[start:stop]
    chosen = BODY if variant == 'original' else WRONG
    helper = ("fn check_question_record<'a>(record: &'a SourceWorkRecord, question: &MaterialRetirementQuestion) -> Result<&'a SourceWorkRecord, JournalError> {\n"
              + chosen + '}\n\n')
    parent = actual.replace(chosen, '    check_question_record(record, question)\n')
    if actual.count(chosen) != 1 or parent.count('check_question_record(record, question)') != 1:
        raise ValueError('move the entire selected-record tail once; keep the original search')
    return text[:start] + helper + parent + text[stop:]


def reconstruct(archive, source, variant):
    _base_stage(archive, source, 'original')
    path = source / retire.ports.RETIREMENT
    path.write_text(factor(path.read_text(), variant))


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = dict(business={}, libraries=[])
    for name in (ROOT, retire.ports.WORK + '::retirement::request'):
        entry = retire.declaration(tree['fun_decls'], name)
        functions = [item for item in mapping if item['def_id'] == entry['def_id']]
        if ('Structured' not in (entry['body'] or {}) or not functions
                or any(item['is_opaque'] for item in functions)):
            raise ValueError('the entire question tail and original request must be transparent: ' + name)
        result['business'][name] = entry['def_id']
    for item in mapping:
        if item['is_opaque']:
            if item['source']['file'].startswith('controller-service/'):
                raise ValueError('no owned business declaration or complete lookup may be opaque')
            result['libraries'].append(item['lean_name'])
    result['libraries'].sort()
    return result


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS = ROOT, NAMESPACE, OPTIONS
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            result = json.loads(path.read_bytes())
            result.update(base_driver=result['driver'], driver=digest(Path(__file__)),
                capture_root=ROOT, original_find_proved=False,
                tail_transformation='verbatim original request-check/return tail with optional sandbox-only mutation',
                mutation=None if variant == 'original' else 'compare only sandbox instead of the complete request')
            path.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
