"""Check the adopted Writer and mutate only copied production expressions."""
from pathlib import Path
import argparse
import difflib
import hashlib
import importlib.util
import json


def sha(data):
    return hashlib.sha256(data).hexdigest()


def prepare(project, variant='correct'):
    if variant not in ('correct', 'swallow-error'):
        raise ValueError('unknown writer variant')
    evidence = project / 'controller-service/src/template_registry/evidence.rs'
    before = evidence.read_text()
    for symbol in ('pub fn write_bounded<', 'pub fn finish_bounded<',
                   'pub fn encode_bounded_using<', 'pub trait BoundedSerialize<'):
        if before.count(symbol) != 1:
            raise ValueError('adopted writer missing or duplicated: ' + symbol)
    after = before
    if variant == 'swallow-error':
        start = before.index('pub fn encode_bounded_using<')
        end = before.index('pub fn encode_bounded_with_buffer<', start)
        body = before[start:end]
        call = '    finish_bounded(writer, result)'
        if body.count(call) != 1:
            raise ValueError('actual serializer-result mutation is not unique')
        body = body.replace(call, '    let _ = result;\n    finish_bounded(writer, Ok(()))', 1)
        after = before[:start] + body + before[end:]
        evidence.write_text(after)
    records = []
    for name in ('controller-service/src/template_registry/evidence.rs',
                 'sandbox-internal/src/server.rs', 'sandbox-internal/src/lib.rs'):
        current = (project / name).read_bytes()
        records.append(dict(path=name, before_sha256=sha(before.encode()) if name == 'controller-service/src/template_registry/evidence.rs' else sha(current),
                            after_sha256=sha(current)))
    out = project / 'artifacts/envelope-writer-source'
    out.mkdir(parents=True)
    record = dict(variant=variant, production_factorization_adopted=True,
                  correct_source_rewritten=False, files=records,
                  condition='Fixed Vec/Serde call contracts remain explicit; not a general old/new equivalence proof')
    (out / 'source-transform.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project', required=True, type=Path)
    parser.add_argument('--variant', choices=['correct', 'swallow-error'], default='correct')
    args = parser.parse_args()
    print(json.dumps(prepare(args.project.absolute(), args.variant)))
