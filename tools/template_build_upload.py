"""Deadline-owned uploads through the pinned SDK, for the opt-in Rust builder.

There is one awaited upload at a time, no detached worker or replay. Cancellation
is not a VM cleanup receipt; the original supervisor still reclaims that VM.
Regular host filesystem I/O and the SDK's shared transport remain dependencies.
"""
import asyncio
from concurrent.futures import ThreadPoolExecutor
import hashlib
import io
import os
import stat

if __package__:
    from .template_build_store import BuildError
else:
    from template_build_store import BuildError


def remaining(deadline, clock):
    value = deadline - clock()
    if value <= 0:
        raise BuildError(408, 'BuildDeadlineExceeded')
    return value


class PinnedReader(io.RawIOBase):
    """Hash the actual streamed inode with fixed reads and a one-byte probe."""
    def __init__(self, path, expected, deadline, clock):
        super().__init__()
        self.path, self.expected = path, expected
        self.deadline, self.clock = deadline, clock
        self.total, self.digest, self.complete = 0, hashlib.sha256(), False
        self.fd = None
        self.fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        try:
            self.opened = os.fstat(self.fd)
            if (not stat.S_ISREG(self.opened.st_mode) or self.opened.st_size != expected['bytes']
                    or self.opened.st_nlink != 1 or self.opened.st_uid not in (0, os.geteuid())
                    or self.opened.st_mode & 0o022):
                raise ValueError('untrusted upload inode')
            self._same_inode()
        except BaseException:
            self.close()
            raise

    def _same_inode(self):
        def identity(meta):
            return (meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns,
                    meta.st_mode, meta.st_uid, meta.st_gid, meta.st_nlink)
        if (identity(os.fstat(self.fd)) != identity(self.opened)
                or identity(self.path.lstat()) != identity(self.opened)):
            raise ValueError('upload inode changed')

    def read(self, size=-1):
        remaining(self.deadline, self.clock)
        if size == 0:
            return b''
        count = min(65536, self.expected['bytes'] - self.total + 1)
        block = os.read(self.fd, count if size < 0 else min(count, size))
        remaining(self.deadline, self.clock)
        self.total += len(block)
        if self.total > self.expected['bytes']:
            raise ValueError('upload grew')
        self.digest.update(block)
        if not block:
            self._same_inode()
            if self.total != self.expected['bytes'] or self.digest.hexdigest() != self.expected['sha256']:
                raise ValueError('uploaded bytes differ from protected pin')
            self.complete = True
        return block

    def close(self):
        if not self.closed and self.fd is not None:
            os.close(self.fd)
        super().close()


async def _write(sandbox, version, destination, data, deadline, clock):
    from e2b.api.client_async import get_envd_api
    from e2b.sandbox_async.filesystem.filesystem import Filesystem
    # Reuse the original protected center URL, helper/tenant credentials and VM
    # access token. Constructing a filesystem client never creates another VM.
    remaining(deadline, clock)
    async with get_envd_api(sandbox.connection_config, sandbox.envd_api_url) as client:
        files = Filesystem(sandbox.envd_api_url, version, sandbox.connection_config, client)
        left = remaining(deadline, clock)
        try:
            async with asyncio.timeout(left):
                await files.write(destination, data, user='root', request_timeout=left)
        except TimeoutError:
            raise BuildError(408, 'BuildDeadlineExceeded') from None
        remaining(deadline, clock)  # A late response cannot authorize the worker.


def upload(sandbox, version, destination, data, deadline, clock, *, expected=None):
    def run(body):
        # SDK to_thread reads use exactly one owned worker. Python's runner has
        # a finite executor shutdown wait; this outer owner still joins after
        # that wait, before the file can close or a later upload can begin.
        with ThreadPoolExecutor(max_workers=1, thread_name_prefix='guest-upload') as executor:
            async def send():
                asyncio.get_running_loop().set_default_executor(executor)
                await _write(sandbox, version, destination, body, deadline, clock)
            asyncio.run(send())
    remaining(deadline, clock)
    if expected is None:
        if not isinstance(data, (str, bytes)):
            raise ValueError('explicit upload fingerprint required')
        run(data)
        return
    # The upload owner joins the SDK's file-reader executor before this descriptor
    # closes, including on cancellation. Kernel-blocked local I/O is not made
    # interruptible by an asyncio timeout.
    with PinnedReader(data, expected, deadline, clock) as incoming:
        run(incoming)
        if not incoming.complete:
            raise ValueError('upload returned before verified EOF')
        incoming._same_inode()
        remaining(deadline, clock)
