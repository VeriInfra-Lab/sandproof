#!/usr/bin/env python3
"""Explicit repaired-task TCP policy; run only inside the task VM."""
import json
from pathlib import Path


def configure(setting, evidence):
    """Apply the existing browser recipe's policy and record its readback."""
    before = int(setting.read_text())
    if evidence.exists():
        raise RuntimeError('TCP connection policy evidence already exists')
    setting.write_text('2\n')
    observed = int(setting.read_text())
    if observed != 2:
        raise RuntimeError('guest TCP connection policy did not take effect')
    result = {
        'scope': 'guest TCP connection retry policy; no host changes or HTTP response substitution',
        'tcp_syn_retries_before': before,
        'tcp_syn_retries': observed,
        'limitation': 'Unreachable external subresources still fail; no claim that their content was tested.',
    }
    evidence.parent.mkdir(parents=True, exist_ok=True)
    with evidence.open('x') as output:
        output.write(json.dumps(result) + '\n')
    return result


if __name__ == '__main__':
    configure(Path('/proc/sys/net/ipv4/tcp_syn_retries'),
              Path('/logs/verifier/tcp-connect-policy.json'))
