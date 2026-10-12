"""Owned component-test child for the original double-mTLS relay, not a VM test.

Rust supplies synthetic fixed certificate files and loopback addresses. This
child reuses RelayMaterials and InternalFaultRelay unchanged; it does not create
a second TLS/HTTP implementation or load deployed credentials.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import json
from pathlib import Path
import ssl
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from _internal_fault_relay import InternalFaultRelay, RelayMaterials


def main():
    # A single finite test command from our owned Rust parent, never a public
    # control endpoint. EOF or a stop command closes precisely this child.
    source = sys.stdin.buffer.readline(8193)
    if len(source) > 8192 or not source.endswith(b'\n'):
        raise ValueError('finite component test input required')
    value = json.loads(source)
    materials = RelayMaterials(Path(value['operator']), Path(value['control']), value['receipt'])
    # Test the supported Python 3.13 stricter verification contract also when
    # the test selector is Python 3.12. Do not disable root/SAN/leaf verification.
    materials.client.verify_flags |= ssl.VERIFY_X509_STRICT
    materials.server.verify_flags |= ssl.VERIFY_X509_STRICT
    relay = InternalFaultRelay(listen=('127.0.0.1', value['listen_port']),
        backend=('127.0.0.1', value['backend_port']), node=value['node'], materials=materials)
    relay.start()
    try:
        print('READY', flush=True)
        command = sys.stdin.buffer.readline(65)
        if command not in (b'', b'STOP\n'):
            raise ValueError('fixed component stop required')
    finally:
        relay.stop()
    if relay.failed.is_set():
        raise ValueError('original relay rejected authenticated component data')
    # The Rust TempDir owns synthetic test files under its current UID. The
    # root-only DK material cleanup receipt is intentionally not impersonated.
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
