"""Render a Lean data module from the verified actual Charon receipts."""

import argparse
from hashlib import sha256
import json
from pathlib import Path


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def lean_string(value):
    if not isinstance(value, str):
        raise TypeError('expected string from Charon receipt')
    return json.dumps(value, ensure_ascii=False)


def render(window_path, retirement_path, window_llbc, retirement_llbc, commit):
    window = json.loads(window_path.read_bytes())
    retirement = json.loads(retirement_path.read_bytes())
    if window['source'] != commit or window['artifact_sha256'] != digest(window_llbc):
        raise ValueError('CloseWindow receipt does not bind the exact actual source capture')
    if retirement['capture_sha256'] != digest(retirement_llbc) or (
        retirement['branch_source_sha256'] not in window['source_inputs'].values()
    ):
        raise ValueError('retirement receipt does not bind the linked actual source capture')
    if retirement['original_owner_argument'] != 1 or retirement['separate_transaction_calls'] != 0:
        raise ValueError('retirement receipt changes the transaction owner')
    traces = [
        '  [' + ', '.join(map(lean_string, trace['events'])) + ']'
        for trace in window['traces']
    ]
    return '\n'.join([
        '/- Generated from the validated actual Charon receipts. -/',
        'namespace SandproofBranchBatch.WindowEvidence',
        '',
        'def candidate : String := ' + lean_string(commit),
        'def windowLlbcSha256 : String := ' + lean_string(window['artifact_sha256']),
        'def retirementLlbcSha256 : String := ' + lean_string(retirement['capture_sha256']),
        'def selectSql : String := ' + lean_string(retirement['select']),
        'def deleteSql : String := ' + lean_string(retirement['delete']),
        'def separateTransactionCalls : Nat := ' + str(retirement['separate_transaction_calls']),
        'def windowTraces : List (List String) := [',
        ',\n'.join(traces),
        ']',
        '',
        'end SandproofBranchBatch.WindowEvidence',
        '',
    ])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('window', type=Path)
    parser.add_argument('retirement', type=Path)
    parser.add_argument('window_llbc', type=Path)
    parser.add_argument('retirement_llbc', type=Path)
    parser.add_argument('commit')
    args = parser.parse_args()
    print(render(args.window, args.retirement, args.window_llbc, args.retirement_llbc,
                 args.commit), end='')
