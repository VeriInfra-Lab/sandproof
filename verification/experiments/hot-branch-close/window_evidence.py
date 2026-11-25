"""Render a Lean data module from the verified actual Charon receipts."""

import argparse
from hashlib import sha256
import json
from pathlib import Path


BRANCH_SOURCE = 'controller-service/src/controller/branching.rs'


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
        retirement['branch_source_sha256'] != window['source_inputs'].get(BRANCH_SOURCE)
    ):
        raise ValueError('retirement receipt does not bind the linked actual source capture')
    if retirement['original_owner_argument'] != 1 or retirement['separate_transaction_calls'] != 0:
        raise ValueError('retirement receipt changes the transaction owner')
    columns = retirement['row_callback_columns']
    outcomes = retirement['row_callback_outcomes']
    select = retirement['select_parameter']
    delete = retirement['delete_parameter']
    if (type(columns) is not list or any(type(column) is not int for column in columns)
            or columns != list(range(5))
            or type(select['add']) is not int or select['add'] != 1
            or type(delete['bytes']) is not int or delete['bytes'] != 32):
        raise ValueError('retirement receipt differs from the original SQL arguments or row columns')
    expected_errors = [dict(read=list(range(col + 1)), error_from=col) for col in columns]
    if (outcomes != [*expected_errors, dict(read=columns, stored_columns=columns)]
            or any(type(col) is not int for outcome in outcomes for col in outcome['read'])
            or any(type(outcome.get('error_from')) is not int for outcome in outcomes[:-1])
            or any(type(col) is not int for col in outcomes[-1]['stored_columns'])
            or type(select['slot']) is not int or select['slot'] != 1 or select['rust_type'] != 'u32'
            or select['source'] != 'HostBranchLimits.max_batches'
            or type(delete['slot']) is not int or delete['slot'] != 1 or delete['rust_type'] != '&[u8]'
            or delete['source'] != 'digest(loaded_record.key.operation)'
            or delete['through'] != 'original retirement vector'):
        raise ValueError('retirement receipt lost the original row outcomes or parameter sources')
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
        'def selectColumns : List Nat := [' + ', '.join(map(str, columns)) + ']',
        'def selectFirstErrors : List (List Nat × Nat) := [' + ', '.join(
            '(' + str(outcome['read']) + ', ' + str(outcome['error_from']) + ')'
            for outcome in outcomes[:-1]) + ']',
        'def selectStoredColumns : List Nat := ' + str(outcomes[-1]['stored_columns']),
        'def selectLimitOffset : Nat := ' + str(select['add']),
        'def selectLimitSource : String := ' + lean_string(select['source']),
        'def deleteDigestBytes : Nat := ' + str(delete['bytes']),
        'def deleteKeySource : String := ' + lean_string(delete['source']),
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
