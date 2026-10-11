"""Record exact real-VM identities before a C02 child can destroy or expire them.

The private C02 supervisor supplies its owned directory explicitly. Other SDK
probes do no host I/O when that variable is absent. These are transient test
observations, not product tombstones or worker success receipts.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
import json
import os
from pathlib import Path
import re
import stat
import time


def record_resource_identity(sandbox):
    configured = os.environ.get("SANDPROOF_TEST_EVIDENCE_ROOT")
    if configured is None:
        return
    root = Path(configured)
    metadata = root.lstat()
    if (not root.is_absolute() or not stat.S_ISDIR(metadata.st_mode)
            or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
        raise RuntimeError("private owned C02 evidence root required")
    if (type(sandbox) is not str or re.fullmatch(r'sandbox-[1-9][0-9]{0,19}',sandbox) is None
            or int(sandbox.removeprefix('sandbox-'))>=2**64):
        raise RuntimeError('exact public center sandbox id required, not a native suffix')
    requests=root/'resource-requests'
    metadata = requests.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077):
        raise RuntimeError("private resource-evidence directory required")
    name=hashlib.sha256(sandbox.encode()).hexdigest()
    descriptor = os.open(requests/(name+'.request.json'), os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump({'sandbox':sandbox},output)
        output.flush()
        os.fsync(output.fileno())
    directory = os.open(requests, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)
    # This is only a bounded private test callback. The parent owns the actual
    # services, performs a planned offline original host audit, saves physical
    # evidence and restarts that same host. The child never signals product PIDs
    # or reads raw SQL/configuration as another authority.
    deadline=time.monotonic()+120
    acknowledgement=requests/(name+'.ack.json')
    while time.monotonic()<deadline:
        try:
            fd=os.open(acknowledgement,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
        except FileNotFoundError:
            time.sleep(.05)
            continue
        with os.fdopen(fd,'rb') as source:
            info=os.fstat(source.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_nlink!=1 or info.st_uid!=os.geteuid()
                    or info.st_mode&0o077 or info.st_size>256):
                raise RuntimeError('private exact resource observation reply required')
            value=json.loads(source.read(257))
        if (type(value) is not dict or set(value)!={'sandbox','recorded'}
                or value['sandbox']!=sandbox or value['recorded'] is not True):
            raise RuntimeError('original physical identity recording unknown')
        return
    raise TimeoutError('original parent resource observation unknown; retain request')
