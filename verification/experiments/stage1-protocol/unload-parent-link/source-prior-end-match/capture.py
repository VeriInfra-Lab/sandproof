"""Capture the original prior-end caller with its transparent original late guard."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-request-end-match'
sys.path.insert(0, str(PARENT.parent / 'source-remove-retired'))
spec = importlib.util.spec_from_file_location('source_prior_end_match_capture_parent', PARENT / 'capture.py')
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
retire = parent.retire
ROOT = 'sandproof_controller_service::controller::lifecycle_inbox::source_work::full::drain::prior_matches'
NAMESPACE = 'SandproofSourcePriorEndMatch'
VARIANTS = ('original', 'allow-completed-no-request')
END = Path('controller-service/src/controller/lifecycle_inbox/source_work/full/drain.rs')
BASE_STAGED = parent.STAGED
STAGED = (*BASE_STAGED, END)
OPTIONS = dict(parent.OPTIONS, roots=[ROOT])
PORTS = parent.PORTS
LIBRARIES = {
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
    'capture.work.digest_full_work_binding',
    'capture.work.encode_full_work_binding',
    'core.ops.range.RangeInclusive.contains',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'core.option.Option.as_deref',
    'core.option.Option.as_ref',
}
OLD = '''            )? && !(capture.completed.is_some()
                && matches!(
                    observation.outcome,
                    crate::capture::CaptureRequestEndOutcome::NoRequest
                ))'''
NEW = '            )?'
MATCH = OLD[OLD.index('matches!('):-1]
CLAUSE = OLD[OLD.index('capture.completed.is_some()'):-1]
HELPER = ('\nfn prior_completed_no_request(completed: bool, observation: &crate::capture::CaptureRequestEndObservation) -> bool {\n    '
          + CLAUSE.replace('capture.completed.is_some()', 'completed', 1) + '\n}\n')


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown prior-end view')
    start = text.index('fn prior_matches(')
    stop = text.index('\nfn terminal_end(', start)
    body = text[start:stop]
    if body.count(OLD) != 1:
        raise ValueError('one exact original completed/NoRequest check is required')
    if variant == 'original':
        return text
    return text[:start] + body.replace(OLD, NEW, 1) + text[stop:]


def reconstruct(archive, source, variant):
    try:
        parent.reconstruct(archive, source, 'original')
    finally:
        retire.STAGED = STAGED
    path = source / END
    text = mutate(path.read_text(), variant)
    if variant == 'original':
        start = text.index('fn prior_matches(')
        stop = text.index('\nfn terminal_end(', start)
        body = text[start:stop]
        if body.count(CLAUSE) != 1:
            raise ValueError('one exact original completed/NoRequest clause required')
        text = text[:start] + body.replace(CLAUSE, 'prior_completed_no_request(capture.completed.is_some(), observation)', 1) + text[stop:] + HELPER
    path.write_text(text)


def admit(ir, generated, target):
    tree = retire.checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = dict(business={}, libraries=[])
    for suffix in ('controller.lifecycle_inbox.source_work.full.drain.prior_matches',
            'capture.CaptureRequestEndObservation.matches',
            'capture.work.FullCaptureWorkRequest.binding_digest'):
        rows = [row for row in mapping if row['lean_name'] == NAMESPACE + '.' + suffix]
        if len(rows) != 1 or rows[0]['is_opaque']:
            raise ValueError('one actual transparent method is required: ' + suffix)
        functions = [row for row in tree['fun_decls'] if row and row['def_id'] == rows[0]['def_id']]
        if len(functions) != 1 or 'Structured' not in (functions[0]['body'] or {}):
            raise ValueError('the complete original method is required: ' + suffix)
        result['business'][suffix] = rows[0]['def_id']
    for row in mapping:
        if row['is_opaque']:
            if row['source']['file'].startswith(('controller-service/', 'controller-core/', 'sandbox-protocol/')) and not row['lean_name'].endswith(tuple('.' + name.rsplit('::', 1)[1] for name in PORTS)):
                raise ValueError('no additional business opacity: ' + row['lean_name'])
            result['libraries'].append(row['lean_name'])
    result['libraries'].sort()
    if set(result['libraries']) != LIBRARIES or len(result['libraries']) != 7:
        raise ValueError('the exact seven per-symbol interfaces are required')
    helper = [row for row in mapping if row['lean_name'] == NAMESPACE +
              '.controller.lifecycle_inbox.source_work.full.drain.prior_completed_no_request']
    if helper:
        if len(helper) != 1 or helper[0]['is_opaque']:
            raise ValueError('one original late guard helper must be transparent')
        functions = [row for row in tree['fun_decls'] if row and row['def_id'] == helper[0]['def_id']]
        if len(functions) != 1 or 'Structured' not in (functions[0]['body'] or {}):
            raise ValueError('the complete actual late guard helper body is required')
        result['guard_helper'] = helper[0]['def_id']
    return result


def capture(output, tools, lock, variant):
    retire.ROOT, retire.NAMESPACE, retire.OPTIONS, retire.STAGED = ROOT, NAMESPACE, OPTIONS, STAGED
    retire.reconstruct, retire.admit = reconstruct, admit
    try:
        retire.capture(output, tools, lock, variant)
    finally:
        path = output / 'result.json'
        if path.is_file():
            r = json.loads(path.read_bytes())
            r.update(base_driver=r['driver'], driver=sha256(Path(__file__).read_bytes()).hexdigest(),
                capture_root=ROOT, direct_original_caller_with_transparent_guard=True, whole_receipt_proved=False, public_acceptance=False,
                parent_driver=sha256(Path(parent.__file__).read_bytes()).hexdigest(),
                original_completed_no_request_clause_moved_to_transparent_helper_with_original_bool_argument=(variant == 'original'),
                mutation=None if variant == 'original' else 'omit only completed/NoRequest rejection after actual request-end match')
            path.write_text(json.dumps(r, indent=2) + '\n')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
