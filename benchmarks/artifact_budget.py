"""Monotonic trial evidence accounting; not a host or network capacity promise."""
import asyncio
from contextlib import asynccontextmanager, contextmanager
import json
import os
from pathlib import Path
import stat

from tools.benchmark_tasks import InvalidInput


MAX_FILE_BYTES = 96 * 1024**2

DEFAULT_ARTIFACT_LIMITS = {
    'max_bytes': 128 * 1024**2,
    'max_entries': 4096,
    'max_depth': 16,
    'max_file_bytes': MAX_FILE_BYTES,
}


def add_artifact_arguments(parser):
    group = parser.add_argument_group('per-trial retained evidence limits')
    for key, default in DEFAULT_ARTIFACT_LIMITS.items():
        group.add_argument('--artifact-' + key.replace('_', '-'), type=int, default=default)


def artifact_limits_from_args(args):
    limits = {key: getattr(args, 'artifact_' + key) for key in DEFAULT_ARTIFACT_LIMITS}
    return ArtifactBudget(limits).limits


class ArtifactBudgetExceeded(InvalidInput):
    pass


class ArtifactBudget:
    def __init__(self, limits):
        required = {'max_bytes', 'max_entries', 'max_depth', 'max_file_bytes'}
        if (set(limits) != required
                or any(type(value) is not int or value <= 0 for value in limits.values())
                or limits['max_file_bytes'] > min(limits['max_bytes'], MAX_FILE_BYTES)
                or limits['max_depth'] > 64):
            raise InvalidInput('invalid trial artifact limits')
        self.limits = dict(limits)
        self.bytes = 0
        self.entries = 0
        self.failure = None
        self.violation = None
        self.stream_close_failure = None

    def note_failure(self, reason):
        # Only constant reason codes enter reports; guest paths/errors are not
        # retained here and repeated failures cannot grow the evidence state.
        if self.failure is None:
            self.failure = reason

    def note_close_failure(self, reason):
        self.note_failure(reason)
        if self.stream_close_failure is None:
            self.stream_close_failure = reason

    def reject(self, reason):
        self.note_failure(reason)
        if self.violation is None:
            self.violation = reason
        raise ArtifactBudgetExceeded(self.violation)

    def check(self):
        if self.violation is not None:
            raise ArtifactBudgetExceeded(self.violation)

    def entry(self, depth=0):
        self.check()
        if depth > self.limits['max_depth']:
            self.reject('artifact_depth_exceeded')
        if self.entries >= self.limits['max_entries']:
            self.reject('artifact_entries_exceeded')
        self.entries += 1

    def charge(self, size):
        self.check()
        if type(size) is not int or size < 0:
            self.reject('invalid_artifact_charge')
        if size > self.limits['max_bytes'] - self.bytes:
            self.reject('artifact_bytes_exceeded')
        self.bytes += size

    def report(self):
        return {'limits': dict(self.limits), 'charged_bytes': self.bytes,
                'charged_entries': self.entries, 'complete': self.failure is None,
                'failure': self.failure, 'violation': self.violation,
                'stream_close_failure': self.stream_close_failure}


class ArtifactStream:
    """Own SDK iteration and close across cancellation, using public APIs only.

    Both HTTPX and the pinned SDK close internally at EOF. Shielding aclose
    alone is insufficient: cancellation must not orphan their in-progress
    cleanup. Keep each read in its own task, then join it and explicit close
    under one five-second cleanup deadline before propagating cancellation.
    """
    def __init__(self, reader, budget, *, timeout=5):
        self.reader, self.budget, self.timeout = reader, budget, timeout
        self.next_task = None
        self.close_task = None
        self.task = None
        self.deadline = None

    def __aiter__(self):
        return self

    async def __anext__(self):
        self.next_task = asyncio.create_task(anext(self.reader))
        return await asyncio.shield(self.next_task)

    async def close(self):
        loop = asyncio.get_running_loop()
        if self.task is None:
            self.deadline = loop.time() + self.timeout
            self.close_task = asyncio.create_task(self.reader.aclose())
            owned = [self.close_task]
            if self.next_task is not None:
                owned.append(self.next_task)
            self.task = asyncio.gather(*owned, return_exceptions=True)
        interrupted = False
        try:
            while True:
                try:
                    if not self.task.done():
                        remaining = self.deadline - loop.time()
                        if remaining <= 0:
                            raise TimeoutError('artifact stream close unconfirmed')
                        await asyncio.wait_for(asyncio.shield(self.task), remaining)
                    # Read exceptions belong to the caller. Explicit close
                    # failures are separate missing evidence, never success.
                    self.close_task.result()
                    return
                except asyncio.CancelledError:
                    if self.close_task.cancelled():
                        self.budget.note_close_failure('artifact_stream_close_failed')
                        raise
                    interrupted = True
                    # A second cancel cannot abandon the same owned cleanup
                    # or reset its deadline. No SDK operation is replayed.
        except TimeoutError:
            self.budget.note_close_failure('artifact_stream_close_unconfirmed')
            raise
        except Exception:
            self.budget.note_close_failure('artifact_stream_close_failed')
            raise
        finally:
            if interrupted:
                raise asyncio.CancelledError


@asynccontextmanager
async def closing_artifact_stream(reader, budget):
    owner = ArtifactStream(reader, budget)
    try:
        yield owner
    except BaseException:
        try:
            await owner.close()
        except asyncio.CancelledError:
            # A newly delivered cancellation outranks an ordinary read/budget
            # failure; only secondary cleanup errors may be suppressed.
            raise
        except BaseException:
            # The owner records failed/unconfirmed cleanup. Preserve the
            # original cancellation/read failure instead of a cleanup timeout.
            pass
        raise
    else:
        await owner.close()


def json_string_bytes(value):
    """Exact ensure_ascii JSON string size, without copying a huge escaped log."""
    if value is None:
        return 4  # JSON null
    return 2 + sum(len(json.dumps(value[start:start + 16384])) - 2
                   for start in range(0, len(value), 16384))


def validate_destination(root, target, budget, *, directory=False):
    root, target = Path(root).absolute(), Path(target).absolute()
    if ('..' in target.parts or root.is_symlink() or root.resolve() != root
            or not target.is_relative_to(root) or (target == root and not directory)):
        budget.reject('unsafe_artifact_destination')
    info = root.stat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.geteuid() or info.st_mode & 0o077:
        budget.reject('trial_directory_not_private')
    relative = target.relative_to(root)
    if len(relative.parts) > budget.limits['max_depth']:
        budget.reject('artifact_depth_exceeded')
    current = root
    for component in relative.parts:
        current /= component
        if current.is_symlink():
            budget.reject('unsafe_artifact_destination')
    return root, target


@contextmanager
def artifact_output(root, target, budget):
    """Write only inside a private trial, publishing an owned partial on success.

    Directory descriptors and O_NOFOLLOW prevent host symlinks from redirecting
    writes. A pre-existing .partial is never adopted or deleted. Only our exact
    temporary inode can be removed on failure; successful prior targets survive.
    """
    root, target = validate_destination(root, target, budget)
    relative = target.relative_to(root)
    descriptors = []
    partial = target.name + '.partial'
    owned = None
    directory = None
    try:
        directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        descriptors.append(directory)
        info = os.fstat(directory)
        if info.st_uid != os.geteuid() or info.st_mode & 0o077:
            budget.reject('trial_directory_not_private')
        for component in relative.parts[:-1]:
            try:
                os.stat(component, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                budget.entry()
                os.mkdir(component, mode=0o700, dir_fd=directory)
            directory = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                dir_fd=directory)
            descriptors.append(directory)

        def check_target():
            try:
                info = os.stat(target.name, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                return
            if not stat.S_ISREG(info.st_mode):
                budget.reject('unsafe_artifact_destination')

        check_target()
        fd = os.open(partial, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                     0o600, dir_fd=directory)
        info = os.fstat(fd)
        owned = (info.st_dev, info.st_ino)
        with os.fdopen(fd, 'wb') as output:
            yield output
            output.flush()
        check_target()
        current = os.stat(partial, dir_fd=directory, follow_symlinks=False)
        if (current.st_dev, current.st_ino) != owned:
            budget.reject('artifact_partial_replaced')
        os.replace(partial, target.name, src_dir_fd=directory, dst_dir_fd=directory)
        owned = None
    except OSError:
        budget.note_failure('artifact_local_io_failed')
        raise
    finally:
        try:
            if owned is not None:
                try:
                    current = os.stat(partial, dir_fd=directory, follow_symlinks=False)
                    if (current.st_dev, current.st_ino) == owned:
                        os.unlink(partial, dir_fd=directory)
                except FileNotFoundError:
                    pass
        finally:
            for fd in reversed(descriptors):
                os.close(fd)
