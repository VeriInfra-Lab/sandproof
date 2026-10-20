"""Finite pathname observations of ORIGINAL service_data, not a disk quota.

Sampling cannot bound growth between observations or open-unlinked files. The
existing filesystem provision, Redis health and external storage conditions
remain required. No content, credentials or private paths enter the report.
"""
import copy
import os
from pathlib import Path
import stat
import threading
import time

FIELDS = frozenset({'root', 'owner_uid', 'peak_bytes', 'inode_budget'})
SCAN_SECONDS = 10
SAMPLE_SECONDS = .05
U64_MAX = 2**64 - 1
OBSERVATION = 'sampled_pathname_peak_not_hard_quota'
LIMITATIONS = 'No instantaneous peak, open-unlinked allocation or long-term disk guarantee.'
USAGE_FIELDS = frozenset({'logical_file_bytes', 'allocated_bytes', 'entries'})
DECLARATION_FIELDS = frozenset({'index', 'identity', 'owner_uid', 'budget'})
REJECTION_REASONS = {
    'service inventory owner, filesystem, type or alias changed': 'entry_identity_or_mode',
    'service inventory contains an inode alias': 'inode_alias',
    'service inventory exceeds original inode budget': 'entries_exceeded',
    'service inventory exceeds original logical byte budget': 'bytes_exceeded',
    'original service inventory root replaced': 'root_replaced',
}


def require_storage_declaration(value):
    """Original execution's finite immutable projection, without private paths."""
    if type(value) is not list or not 1 <= len(value) <= 8:
        raise ValueError('original finite service storage declaration required')
    identities = set()
    device = None
    for index, row in enumerate(value):
        if (type(row) is not dict or set(row) != DECLARATION_FIELDS
                or type(row['index']) is not int or row['index'] != index
                or type(row['owner_uid']) is not int or not 0 <= row['owner_uid'] < 2**32
                or type(row['identity']) is not list or len(row['identity']) != 2
                or any(type(number) is not int or not 0 <= number <= U64_MAX
                       for number in row['identity']) or row['identity'][1] == 0
                or tuple(row['identity']) in identities
                or (device is not None and row['identity'][0] != device)):
            raise ValueError('distinct original service storage identities required')
        identities.add(tuple(row['identity']))
        device = row['identity'][0]
        budget = row['budget']
        if (type(budget) is not dict or set(budget) != {'logical_file_bytes', 'entries'}
                or any(type(number) is not int or not 0 < number <= U64_MAX
                       for number in budget.values())):
            raise ValueError('original finite service storage budget required')
    return copy.deepcopy(value)


def require_storage_observation(value, *, expected):
    """Retain one finite, post-exit engineering summary, not a quota attestation.

    Expected is the original immutable projection from the checked execution
    receipt (or its already checked durable reducer result), not the summary's
    own budget. This validator never invents another budget,
    merges different cohorts, reads paths or attests an unobserved interval.
    """
    fields = {'schema_version', 'observation', 'limitations', 'sample_interval_ms',
              'successful_samples', 'unknown_samples', 'sampler_exited', 'roots'}
    if (type(value) is not dict or set(value) != fields
            or type(value['schema_version']) is not int or value['schema_version'] != 1
            or value['observation'] != OBSERVATION or value['limitations'] != LIMITATIONS
            or type(value['sample_interval_ms']) is not int
            or value['sample_interval_ms'] != int(SAMPLE_SECONDS * 1000)
            or value['sampler_exited'] is not True
            or type(value['successful_samples']) is not int
            or not 0 < value['successful_samples'] <= U64_MAX
            or type(value['unknown_samples']) is not int
            or not 0 <= value['unknown_samples'] <= U64_MAX
            or type(value['roots']) is not list or not 1 <= len(value['roots']) <= 8):
        raise ValueError('finite stable post-exit service storage observation required')
    original = require_storage_declaration(expected)
    for row in value['roots']:
        if (type(row) is not dict or set(row) != DECLARATION_FIELDS | {'observed_peak', 'last_stable'}):
            raise ValueError('exact original service storage observation fields required')
    observed = require_storage_declaration([
        {key: row[key] for key in DECLARATION_FIELDS} for row in value['roots']])
    if observed != original:
        raise ValueError('service storage observation differs from original execution declaration')
    for row in value['roots']:
        budget, peak, stable = row['budget'], row['observed_peak'], row['last_stable']
        for usage in (peak, stable):
            if (type(usage) is not dict or set(usage) != USAGE_FIELDS
                    or any(type(number) is not int or not 0 <= number <= U64_MAX
                           for number in usage.values()) or not usage['entries']
                    or any(usage[key] > budget[key] for key in budget)):
                raise ValueError('stable service storage observation exceeds original budget')
        if any(stable[key] > peak[key] for key in USAGE_FIELDS):
            raise ValueError('stable service inventory cannot exceed its observed peak')
    return copy.deepcopy(value)


class ObservationUnknown(Exception):
    """A changing/unreadable inventory is not an empty inventory."""


def _identity(info):
    return info.st_dev, info.st_ino


def _directory_stamp(info):
    return info.st_mtime_ns, info.st_ctime_ns


def _check(info, record, device):
    if (not (stat.S_ISDIR(info.st_mode) or stat.S_ISREG(info.st_mode))
            or info.st_uid != record['owner_uid'] or info.st_dev != device
            or info.st_mode & 0o077):
        raise ValueError('service inventory owner, filesystem, type or alias changed')
    if stat.S_ISREG(info.st_mode):
        if info.st_nlink == 0:
            raise ObservationUnknown('service inventory file unlinked during observation')
        if info.st_nlink != 1:
            raise ValueError('service inventory owner, filesystem, type or alias changed')


def inventory(record, expected, deadline):
    """Bound entries/FDs and verify identities without reading file contents."""
    directories, seen = [], set()
    usage = dict(logical_file_bytes=0, allocated_bytes=0, entries=0)
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK

    def visit(fd, depth, seen_size, seen_blocks):
        if time.monotonic() >= deadline or depth >= record['inode_budget']:
            raise ObservationUnknown('service inventory observation deadline/depth exceeded')
        before = os.fstat(fd)
        if stat.S_ISREG(before.st_mode) and before.st_nlink == 0:
            raise ObservationUnknown('service inventory file unlinked during observation')
        _check(before, record, expected[0])
        if _identity(before) in seen:
            # Actual hard links were rejected by _check. Atomic rename can
            # make two single-link descriptors refer to the same inode while
            # enumerating changing names. This scan cannot seal an inventory.
            raise ObservationUnknown('service inventory name reused an observed inode')
        seen.add(_identity(before))
        usage['entries'] += 1
        if usage['entries'] > record['inode_budget']:
            raise ValueError('service inventory exceeds original inode budget')
        usage['allocated_bytes'] += max(before.st_blocks, seen_blocks) * 512
        if stat.S_ISREG(before.st_mode):
            usage['logical_file_bytes'] += max(before.st_size, seen_size)
        else:
            # Depth is bounded by the original entry count, but avoid Python
            # recursion for arbitrary configured directory depth.
            directories.append((os.dup(fd), _identity(before), _directory_stamp(before)))
            with os.scandir(fd) as entries:
                for entry in entries:
                    if time.monotonic() >= deadline:
                        raise ObservationUnknown('service inventory observation deadline exceeded')
                    if len(seen) + len(pending) >= record['inode_budget']:
                        raise ValueError('service inventory exceeds original inode budget')
                    initial = os.stat(entry.name, dir_fd=fd, follow_symlinks=False)
                    _check(initial, record, expected[0])
                    child = os.open(entry.name, flags, dir_fd=fd)
                    try:
                        opened = os.fstat(child)
                        current = os.stat(entry.name, dir_fd=fd, follow_symlinks=False)
                        _check(opened, record, expected[0])
                        _check(current, record, expected[0])
                        if _identity(initial) != _identity(opened) or _identity(opened) != _identity(current):
                            raise ObservationUnknown('service inventory file changed during observation')
                        size = max(initial.st_size, opened.st_size, current.st_size)
                        if stat.S_ISREG(current.st_mode) and size > record['peak_bytes']:
                            raise ValueError('service inventory exceeds original logical byte budget')
                        pending.append((child, depth + 1, size,
                                        max(initial.st_blocks, opened.st_blocks, current.st_blocks)))
                    except BaseException:
                        os.close(child)
                        raise
        if usage['logical_file_bytes'] > record['peak_bytes']:
            raise ValueError('service inventory exceeds original logical byte budget')

    pending = []
    try:
        root = os.open(record['root'], flags | os.O_DIRECTORY)
        pending.append((root, 0, 0, 0))
        if _identity(os.fstat(root)) != expected:
            raise ValueError('original service inventory root replaced')
        while pending:
            fd, depth, size, blocks = pending.pop()
            try:
                visit(fd, depth, size, blocks)
            finally:
                os.close(fd)
        for fd, original, stamp in directories:
            now = os.fstat(fd)
            _check(now, record, expected[0])
            if _identity(now) != original or _directory_stamp(now) != stamp:
                raise ObservationUnknown('service inventory directory changed during observation')
        current = os.stat(record['root'], follow_symlinks=False)
        _check(current, record, expected[0])
        if _identity(current) != expected:
            raise ValueError('original service inventory root replaced')
        return usage
    except OSError as error:
        raise ObservationUnknown('service inventory unavailable') from error
    finally:
        for fd, _depth, _size, _blocks in pending:
            os.close(fd)
        for fd, _original, _stamp in directories:
            os.close(fd)


class ServiceStoragePeaks:
    """One bounded sampler across ORIGINAL roles and all their restarts."""
    def __init__(self, records):
        if type(records) is not list or not 1 <= len(records) <= 8:
            raise ValueError('original finite service_data declaration required')
        self.records = copy.deepcopy(records)
        self.identities, self.roots = [], []
        for row in self.records:
            if (type(row) is not dict or set(row) != FIELDS
                    or type(row['root']) is not str or not Path(row['root']).is_absolute()
                    or any(type(row[key]) is not int or not 0 < row[key] <= U64_MAX
                           for key in ('peak_bytes', 'inode_budget'))
                    or type(row['owner_uid']) is not int or not 0 <= row['owner_uid'] < 2**32):
                raise ValueError('invalid original service_data declaration')
            path = Path(row['root'])
            info = path.lstat()
            _check(info, row, info.st_dev)
            if not stat.S_ISDIR(info.st_mode) or _identity(info) in self.identities:
                raise ValueError('distinct original service directories required')
            if self.identities and info.st_dev != self.identities[0][0]:
                raise ValueError('original service roots must share the physical filesystem')
            if any(path == other or path in other.parents or other in path.parents for other in self.roots):
                raise ValueError('original service roots overlap')
            self.roots.append(path)
            self.identities.append(_identity(info))
        self.bound_declaration = require_storage_declaration([
            dict(index=index, identity=list(identity), owner_uid=row['owner_uid'],
                 budget=dict(logical_file_bytes=row['peak_bytes'], entries=row['inode_budget']))
            for index, (row, identity) in enumerate(zip(self.records, self.identities))])
        self.peaks = [dict(logical_file_bytes=0, allocated_bytes=0, entries=0) for _ in records]
        self.latest = None
        self.successful_samples = self.unknown_samples = 0
        self.failure = None
        self.failure_details = None
        self.lock, self.stopped = threading.Lock(), threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.closed = False

    def declaration(self):
        """Retain the first observed original roots, even after staging removal."""
        return copy.deepcopy(self.bound_declaration)

    def sample(self):
        with self.lock:
            if self.failure is not None:
                raise ValueError(self.failure)
            deadline = time.monotonic() + SCAN_SECONDS
            observed = []
            try:
                for index, (row, identity, peak) in enumerate(zip(self.records, self.identities, self.peaks)):
                    current = inventory(row, identity, deadline)
                    observed.append(current)
                    # A later root becoming unknown cannot erase this root's
                    # already stable observed peak. Whole-cohort final success
                    # still requires every original root to be observed.
                    for key in peak:
                        peak[key] = max(peak[key], current[key])
            except ObservationUnknown:
                self.unknown_samples = min(U64_MAX, self.unknown_samples + 1)
                self.latest = None
                return False
            except ValueError as error:
                self.failure = 'original service_data inventory/budget rejected'
                self.failure_details = dict(kind='inventory_rejected', root_index=index,
                    reason=REJECTION_REASONS.get(str(error), 'unexpected_value_error'))
                raise ValueError(self.failure) from None
            self.successful_samples = min(U64_MAX, self.successful_samples + 1)
            self.latest = observed
            return True

    def start(self):
        if self.thread.ident is not None or self.closed:
            raise ValueError('one original storage sampler lifetime required')
        self.sample()
        self.thread.start()

    def _run(self):
        while not self.stopped.wait(SAMPLE_SECONDS):
            try:
                self.sample()
            except ValueError:
                return
            except Exception as error:
                with self.lock:
                    self.failure = 'original storage sampler observation failed'
                    self.failure_details = dict(kind='background_observation_failed',
                                                error_type=type(error).__name__[:128])
                    self.latest = None
                return

    def diagnostics(self):
        """Bounded original history, including failure; never a PASS or new scan."""
        with self.lock:
            return dict(schema_version=1, sampler_exited=self.closed,
                successful_samples=self.successful_samples, unknown_samples=self.unknown_samples,
                failure=copy.deepcopy(self.failure_details),
                roots=[dict(row, observed_peak=copy.deepcopy(peak),
                    last_stable=copy.deepcopy(self.latest[index]) if self.latest is not None else None)
                    for index, (row, peak) in enumerate(zip(self.declaration(), self.peaks))])

    def _stable(self):
        deadline = time.monotonic() + SCAN_SECONDS
        while not self.sample():
            if time.monotonic() >= deadline:
                raise ObservationUnknown('last service inventory remains unknown')
            time.sleep(SAMPLE_SECONDS)

    def stop(self):
        self.stopped.set()
        if self.thread.ident is not None:
            self.thread.join(SCAN_SECONDS + 1)
            if self.thread.is_alive():
                raise ObservationUnknown('original storage sampler exit unobserved')
        if not self.closed:
            try:
                self._stable()
            except (ObservationUnknown, ValueError):
                pass  # The original failure path may drain; report still rejects.
            self.closed = True

    def report(self):
        if not self.closed:
            self._stable()
        with self.lock:
            if self.failure is not None or self.latest is None or not self.successful_samples:
                raise ObservationUnknown('successful final service inventory absent')
            return dict(schema_version=1, observation=OBSERVATION, limitations=LIMITATIONS,
                sample_interval_ms=int(SAMPLE_SECONDS * 1000),
                successful_samples=self.successful_samples, unknown_samples=self.unknown_samples,
                sampler_exited=self.closed,
                roots=[dict(declaration,
                    observed_peak=copy.deepcopy(peak), last_stable=copy.deepcopy(current))
                    for declaration, peak, current in zip(self.declaration(), self.peaks, self.latest)])
