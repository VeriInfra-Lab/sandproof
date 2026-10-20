"""Forward actual admitted service messages, never generate audit messages."""
from __future__ import annotations

import hashlib
import json
import re

PATTERN = re.compile(r'Checked (\S+): \d+ standard logical dependencies')


def read_messages(source, log, evidence):
    digest = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    if (evidence.get('complete') is not True or digest(source) != evidence['source_sha256']
            or digest(log) != evidence['log_sha256']):
        raise ValueError('actual physical file/log evidence changed')
    rows = []
    for line in log.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get('request', {}).get('tool') == 'lean_diagnostic_messages':
            rows.append(row)
    if len(rows) != 1:
        raise ValueError('missing/duplicate original service result')
    row = rows[0]
    request = {'tool': 'lean_diagnostic_messages', 'arguments': {'file_path': evidence['file']},
               'expected_sha256': evidence['source_sha256']}
    if (row['request'] != request or row['stale'] is not False
            or row['revision']['file_sha256'] != evidence['source_sha256']):
        raise ValueError('service request/source mismatch')
    result = row['result']
    for _ in range(3):
        if 'success' in result:
            break
        result = result.get('structuredContent', result.get('result', {}))
    if (result.get('success') is not True or result.get('partial') is not False
            or result.get('timed_out') is not False or result.get('failed_dependencies')
            or result.get('still_elaborating_lines') not in (None, [])):
        raise ValueError('incomplete original service result')
    messages = []
    for item in result.get('items', []):
        if item.get('severity') in ('error', 'warning'):
            raise ValueError('original service reports unresolved diagnostics')
        text = item.get('message', item.get('data', ''))
        if PATTERN.fullmatch(text):
            messages.append(text)
    expected = re.findall(r'^#audit_controller_proof (\S+)', source.read_text(), re.M)
    observed = [PATTERN.fullmatch(message)[1] for message in messages]
    if (len(set(expected)) != len(expected) or sorted(expected) != sorted(observed)
            or len(expected) != evidence['audits']):
        raise ValueError('actual audit messages missing, duplicated or extra')
    return messages


def component_messages(source, proof_output, record, expected):
    if record.get('success') is not True:
        raise ValueError('component proof did not complete')
    messages = []
    for index, evidence in enumerate(record['files']):
        messages += read_messages(source / evidence['file'],
                                  proof_output / ('file-' + str(index) + '.log'), evidence)
    observed = [PATTERN.fullmatch(message)[1] for message in messages]
    if sorted(observed) != sorted(expected) or record['audits'] != len(expected):
        raise ValueError('component exact audit inventory differs from real service messages')
    return messages
