"""Read existing T0 receipts/raw MIR on the proof host; never capture or compile.

The original published census remains responsible for complete compilation
coverage. This bounded locator checks its identities and preserves all matching
literal headers, including repeated const/runtime headers. It does not resolve
dispatch, infer reachability, interpret MIR or establish Aeneas support.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import time


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def locate(capture, catalogue, selection):
    start = time.monotonic()
    receipt_file = capture / 'receipt.json'
    receipt = json.loads(receipt_file.read_text())
    expected = catalogue['t0']
    if (sha(receipt_file) != expected['published_evidence']['receipt_sha256']
            or receipt['index_sha256'] != expected['published_evidence']['index_sha256']
            or receipt['source_commit'] != expected['capture_commit']
            or len(receipt['units']) != expected['units']):
        raise ValueError('original published capture identity differs')
    config_path = expected['config']
    if receipt['config_sha256'] != catalogue['sources'][config_path]:
        raise ValueError('original T0 configuration differs')
    if sha(capture / 'index.json') != receipt['index_sha256']:
        raise ValueError('original full index digest differs')
    expected_stages = [p + '-' + r for p in expected['profiles'] for r in expected['roots']]
    if [row['id'] for row in receipt['stages']] != expected_stages:
        raise ValueError('missing original root/profile')
    selected = {name: value for name, value in catalogue['sources'].items() if name.endswith('.rs')}
    if any(receipt['sources'].get(name) != digest for name, digest in selected.items()):
        raise ValueError('finite current Rust sources differ from original capture')
    rows = {query['id']: [] for query in selection['mir_queries']}
    units = []
    for index, unit in enumerate(receipt['units']):
        queries = [q for q in selection['mir_queries'] if q['crate'] == unit['crate']]
        if not queries:
            continue
        raw = Path(unit['raw'])
        if raw.is_absolute() or '..' in raw.parts:
            raise ValueError('escaping original raw MIR path')
        path = capture / raw
        if sha(path) != unit['sha256'] or path.stat().st_size != unit['bytes']:
            raise ValueError('changed or truncated original MIR')
        command = path.parent / 'command.json'
        if sha(command) != unit['command_sha256']:
            raise ValueError('changed original compiler command')
        command_data = json.loads(command.read_text())
        if command_data['exit'] != 0:
            raise ValueError('failed original compiler unit')
        units.append(dict(receipt_unit=index, crate=unit['crate'], source=unit['source'],
            cfg=unit['cfg'], codegen=unit['codegen'], mir_sha256=unit['sha256'],
            command_sha256=unit['command_sha256']))
        with path.open() as stream:
            for line, text in enumerate(stream, 1):
                if not text.startswith('fn '):
                    continue
                for query in queries:
                    if all(token in text for token in query['tokens']):
                        rows[query['id']].append(dict(receipt_unit=index, line=line,
                            header=text.rstrip('\n'), mir_sha256=unit['sha256']))
    if any(not rows[q['id']] for q in selection['mir_queries']):
        raise ValueError('finite queried original function header was not found')
    return dict(schema=1, kind='existing-t0-literal-header-locations-not-a-new-proof-or-extraction',
        baseline=catalogue['baseline'], receipt_sha256=sha(receipt_file),
        index_sha256=receipt['index_sha256'], source_commit=receipt['source_commit'],
        source_count=len(receipt['sources']), all_units=len(receipt['units']),
        original_stages=expected_stages, selected_source_sha256=selected, units=units,
        queries=selection['mir_queries'], observations=rows, seconds=time.monotonic() - start,
        limits=['Original T0 check-published remains the full admission gate.',
                'Headers are locations into raw compiler output, not resolved runtime calls.',
                'Duplicate const/runtime headers are preserved, not silently merged.',
                'MIR presence does not establish Charon/Aeneas support or any safety theorem.'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--capture', type=Path, required=True)
    parser.add_argument('--catalogue', type=Path, required=True)
    parser.add_argument('--selection', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.capture.resolve(strict=True).relative_to('/data1/zn')
    args.output.absolute().resolve().relative_to('/data1/zn')
    if args.output.exists() or args.output.is_symlink():
        raise ValueError('new private proof-host evidence file required')
    result = locate(args.capture, json.loads(args.catalogue.read_text()), json.loads(args.selection.read_text()))
    with args.output.open('x') as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write('\n')
    print(json.dumps(dict(kind=result['kind'], seconds=result['seconds'],
                         queries={k: len(v) for k, v in result['observations'].items()})))
