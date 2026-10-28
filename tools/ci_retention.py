"""Trusted retirement of explicitly managed, disposable CI artifacts.

No discovery/adoption of home directories, proof environments, image caches or
unfinished runs. Only the trusted operator can seal a terminal cleanup receipt.
This is storage housekeeping, never a test result or resource-cleanup proof.
"""
import argparse
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import re
import stat
import time
import uuid

from tools.benchmark_ci import remove_private_tree
from tools.ci_runner_supervisor import atomic_json
from tools.system_acceptance import digest, json_file, private_path, write_once

DAY = 86400
KINDS = {'build', 'acceptance'}


class Store:
    def __init__(self, root):
        self.root = private_path(root, directory=True)
        for name in ('payloads', 'records'):
            path = self.root / name
            path.mkdir(mode=0o700, exist_ok=True)
            private_path(path, directory=True)

    @contextmanager
    def lock(self):
        with open(self.root / 'lock', 'a',
                  opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as stream:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield

    def paths(self, identity):
        if not isinstance(identity, str) or not re.fullmatch('[0-9a-f]{32}', identity):
            raise ValueError('exact managed artifact identity required')
        return self.root / 'payloads' / identity, self.root / 'records' / (identity + '.json')

    def read(self, identity):
        payload, record = self.paths(identity)
        value = json_file(private_path(record))
        if (set(value) != {'id', 'kind', 'candidate', 'identity', 'state', 'finished', 'receipt'}
                or value['id'] != identity or value['kind'] not in KINDS
                or not re.fullmatch('[0-9a-f]{40}', str(value['candidate']))
                or value['state'] not in ('active', 'retired', 'deleting', 'removed')
                or not isinstance(value['identity'], list) or len(value['identity']) != 2
                or any(type(n) is not int or n < 0 for n in value['identity'])
                or type(value['finished']) is not int or value['finished'] < 0):
            raise ValueError('invalid managed artifact record')
        if value['state'] == 'active':
            if value['finished'] != 0 or value['receipt'] is not None:
                raise ValueError('active artifact cannot have a terminal receipt')
        else:
            self.receipt(value['receipt'], value)
        return payload, record, value

    @staticmethod
    def receipt(receipt, record):
        if (not isinstance(receipt, dict)
                or set(receipt) != {'candidate', 'status', 'resource_cleanup_confirmed', 'evidence_sha256'}
                or receipt['candidate'] != record['candidate']
                or receipt['status'] not in ('passed', 'failed', 'cancelled')
                or receipt['resource_cleanup_confirmed'] is not True
                or not re.fullmatch('[0-9a-f]{64}', str(receipt['evidence_sha256']))):
            raise ValueError('original terminal status and exact cleanup receipt required')

    def allocate(self, *, kind, candidate):
        if kind not in KINDS or not re.fullmatch('[0-9a-f]{40}', candidate):
            raise ValueError('disposable artifact kind and exact candidate required')
        identity = uuid.uuid4().hex
        payload, record = self.paths(identity)
        payload.mkdir(mode=0o700)
        info = payload.stat()
        # A crash before this record leaves an unregistered directory: never GC.
        write_once(record, dict(id=identity, kind=kind, candidate=candidate,
            identity=[info.st_dev, info.st_ino], state='active', finished=0, receipt=None))
        return payload

    def retire(self, identity, receipt_path, evidence_path, *, now):
        payload, record, value = self.read(identity)
        if value['state'] != 'active':
            raise ValueError('artifact already sealed; observe original retirement')
        info = private_path(payload, directory=True).stat()
        if [info.st_dev, info.st_ino] != value['identity']:
            raise ValueError('artifact identity changed')
        receipt = json_file(private_path(receipt_path), limit=4096)
        self.receipt(receipt, value)
        evidence = private_path(evidence_path)
        # Evidence stays OUTSIDE disposable payloads and is not collected here.
        if evidence.is_relative_to(self.root) or digest(evidence) != receipt['evidence_sha256']:
            raise ValueError('independent preserved evidence does not match receipt')
        if type(now) is not int or now <= 0:
            raise ValueError('finite positive retirement time required')
        value.update(state='retired', finished=now, receipt=receipt)
        atomic_json(record, value)

    def record_paths(self):
        for path in sorted((self.root / 'records').iterdir()):
            if re.fullmatch('[0-9a-f]{32}\\.tmp', path.name):
                # atomic_json's uncommitted scratch after a crash has NO
                # authority. Never replay it over the durable JSON record.
                private_path(path)
                continue
            if not re.fullmatch('[0-9a-f]{32}\\.json', path.name):
                raise ValueError('unknown retention record; inspect without deletion')
            yield path

    def collect(self, *, now, apply=False, keep_days=7, max_count=20, max_bytes=100 * 1024**3,
                keep_latest=0):
        if (type(now) is not int or now <= 0 or type(keep_days) is not int or not 1 <= keep_days <= 365
                or type(max_count) is not int or not 0 <= max_count <= 1000
                or type(max_bytes) is not int or not 0 <= max_bytes <= 1024**5
                or type(keep_latest) is not int or not 0 <= keep_latest <= 1000):
            raise ValueError('finite retention policy required; minimum age is one day')
        rows = []
        for path in self.record_paths():
            payload, record, value = self.read(path.stem)
            if value['state'] == 'removed' and (payload.exists() or payload.is_symlink()):
                raise ValueError('removed artifact unexpectedly present')
            if value['state'] in ('active', 'removed'):
                continue
            if payload.is_symlink():
                raise ValueError('artifact replaced by symlink')
            if not payload.exists():
                if value['state'] != 'deleting':
                    raise ValueError('retired payload missing without durable deletion intent')
                size = 0
            else:
                info = private_path(payload, directory=True).stat()
                if [info.st_dev, info.st_ino] != value['identity']:
                    raise ValueError('artifact identity changed')
                size = allocated_bytes(payload)
            rows.append((payload, record, value, size))
        rows.sort(key=lambda row: (-row[2]['finished'], row[2]['id']))
        retained = 0
        plan = []
        for index, (payload, record, value, size) in enumerate(rows):
            age = now - value['finished']
            eligible = (value['state'] == 'deleting' or index >= keep_latest and age >= DAY and
                        (age >= keep_days * DAY or index >= max_count or retained + size > max_bytes))
            if not eligible:
                retained += size
                continue
            plan.append(dict(id=value['id'], allocated_bytes=size, state=value['state']))
            if apply:
                # Receipt and intent are fsynced BEFORE the first destructive action.
                value['state'] = 'deleting'
                atomic_json(record, value)
                if payload.exists():
                    reject_mounts(payload)
                    remove_private_tree(payload, value['identity'])
                fsync_directory(payload.parent)
                value['state'] = 'removed'
                atomic_json(record, value)
        if apply:
            self.prune_removed()
        return plan

    def prune_removed(self):
        # Only tiny, fully removed tombstones; unresolved/active evidence is never
        # counted away. Original independent reports are not part of this store.
        records = []
        for path in self.record_paths():
            payload, record, value = self.read(path.stem)
            if value['state'] == 'removed':
                if payload.exists() or payload.is_symlink():
                    raise ValueError('removed artifact unexpectedly present')
                records.append((value['finished'], record))
        for _, record in sorted(records, reverse=True)[40:]:
            record.unlink()
        fsync_directory(self.root / 'records')


def allocated_bytes(root):
    """Read-only size estimate, not authorization; deletion rechecks every inode."""
    reject_mounts(root)
    info = root.stat()
    total = info.st_blocks * 512
    seen = {(info.st_dev, info.st_ino)}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            child = (Path(directory) / name).lstat()
            if child.st_dev != info.st_dev or child.st_uid != os.geteuid():
                raise ValueError('unknown owner or mount in disposable artifact')
            if not (stat.S_ISREG(child.st_mode) or stat.S_ISDIR(child.st_mode)):
                raise ValueError('unknown artifact type; preserve for inspection')
            identity = (child.st_dev, child.st_ino)
            if identity not in seen:
                total += child.st_blocks * 512
                seen.add(identity)
    return total


def reject_mounts(root, mountinfo=None):
    # st_dev alone cannot detect a same-filesystem bind mount. A retired
    # payload must have NO remaining mounts, even when their device matches.
    if mountinfo is None:
        mountinfo = Path('/proc/self/mountinfo').read_text()
    if not mountinfo.strip():
        raise ValueError('mount observation is empty')
    for line in mountinfo.splitlines():
        fields = line.split()
        if len(fields) < 10 or '-' not in fields[6:]:
            raise ValueError('mount observation is incomplete')
        name = re.sub(r'\\([0-7]{3})', lambda match: chr(int(match[1], 8)), fields[4])
        mount = Path(name)
        if not mount.is_absolute():
            raise ValueError('mount observation is invalid')
        if mount == root or mount.is_relative_to(root):
            raise ValueError('mount remains inside retired artifact')


def fsync_directory(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class NativeBuildRun:
    """Record Cargo targets born inside one new native run, retaining its inputs.

    Existing run-* directories cannot be adopted. A successful original native
    seal produces its own manifest; failed/cancelled runs require the separate
    explicit operator CLI. Publishing either never moves or deletes data.
    The trusted operator must separately establish complete reference/mount
    absence and supply an exact cleanup receipt before an explicit handoff.
    """

    def __init__(self, work, candidate):
        self.work = private_path(work, directory=True)
        if list(self.work.iterdir()):
            raise ValueError('only a newly allocated empty native run can be registered')
        if not re.fullmatch('[0-9a-f]{40}', candidate):
            raise ValueError('exact native candidate required')
        self.candidate = candidate
        self.identity = [self.work.stat().st_dev, self.work.stat().st_ino]
        self.allocation = self.work / 'build-allocation.json'
        write_once(self.allocation, dict(candidate=candidate, identity=self.identity))
        self.allocation_sha = digest(self.allocation)

    def finish(self, evidence):
        if (self.work / 'build-terminal-targets.json').exists():
            raise ValueError('failed/cancelled build record cannot become native success')
        if ([self.work.stat().st_dev, self.work.stat().st_ino] != self.identity
                or digest(self.allocation) != self.allocation_sha):
            raise ValueError('native run allocation changed')
        evidence = private_path(evidence)
        if evidence.parent != self.work:
            raise ValueError('original native completion must remain in the run')
        complete = json_file(evidence, limit=16 * 1024**2)
        used = [json.loads(line) for line in (self.work / 'commands-used.jsonl').read_text().splitlines()]
        if (complete.get('complete') is not True or type(complete.get('exit_code')) is not int
                or complete['exit_code'] != 0 or complete.get('scope_complete') is not True
                or not used or complete.get('uses') != used):
            raise ValueError('original successful native completion and exact command uses required')
        targets = native_targets(self.work)
        manifest = self.work / 'build-targets.json'
        write_once(manifest, dict(kind='native-cargo-targets-v1', candidate=self.candidate,
            work=str(self.work), identity=self.identity, allocation_sha256=self.allocation_sha,
            completion=str(evidence), completion_sha256=digest(evidence),
            commands_used_sha256=digest(self.work / 'commands-used.jsonl'), targets=targets,
            source_and_logs_retained=True, product_resource_cleanup_claimed=False))
        return manifest


def native_targets(work):
    """Inventory only marked Cargo targets; never include source or evidence."""
    reject_mounts(work)
    targets = []
    for directory, dirs, _ in os.walk(work, followlinks=False):
        for name in list(dirs):
            path = Path(directory) / name
            if name in ('.git', '.lake') or path.is_symlink():
                if name == 'target':
                    raise ValueError('Cargo target replaced by symlink')
                dirs.remove(name)
            elif name == 'target':
                private_path(path, directory=True)
                for marker in ('CACHEDIR.TAG', '.rustc_info.json'):
                    info = (path / marker).lstat()
                    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
                        raise ValueError('original Cargo target markers required')
                if not (path / 'CACHEDIR.TAG').read_bytes().startswith(
                        b'Signature: 8a477f597d28d172789f06886806bc55'):
                    raise ValueError('original Cargo cache signature required')
                info = path.stat()
                targets.append(dict(path=str(path), identity=[info.st_dev, info.st_ino],
                                    allocated_bytes=allocated_bytes(path)))
                dirs.remove(name)
    return targets


def terminal_exit(work, candidate, identity, record_path):
    """Read a trusted original executor record, not a cancellation request.

    The operator must have waited for ALL original work, including descendants.
    This tool neither observes a privileged process namespace nor stops work.
    """
    record_path = private_path(record_path)
    record = json_file(record_path, limit=4096)
    if (record_path.parent != work
            or set(record) != {'kind', 'candidate', 'identity', 'status', 'exit_code',
                               'processes_exited', 'observation_complete', 'observation_error'}
            or record['kind'] != 'native-process-exit-v1' or record['candidate'] != candidate
            or record['identity'] != identity or record['status'] not in ('failed', 'cancelled')
            or type(record['exit_code']) is not int or record['exit_code'] == 0
            or record['processes_exited'] is not True
            or record['observation_complete'] is not True or record['observation_error'] is not None):
        raise ValueError('original failed/cancelled exit and complete child termination required')
    if any((work / name).exists() for name in ('native-complete.json', 'build-targets.json')):
        raise ValueError('native success and failure records must remain separate')
    return record['status']


def command_uses_digest(work):
    path = work / 'commands-used.jsonl'
    # Early failure may precede the first command-use registration. Pin absence
    # too: a new record appearing later invalidates the terminal inventory.
    return digest(private_path(path)) if path.exists() or path.is_symlink() else None


def record_native_terminal(work, exit_record_path):
    """Explicit operator inventory for an already registered failed/cancelled run.

    No success seal, automatic recovery/adoption, move or collection is performed.
    Even an empty inventory is recorded truthfully; it cannot be handed off.
    """
    work = private_path(work, directory=True)
    allocation = private_path(work / 'build-allocation.json')
    value = json_file(allocation, limit=4096)
    identity = [work.stat().st_dev, work.stat().st_ino]
    if (set(value) != {'candidate', 'identity'} or value['identity'] != identity
            or not re.fullmatch('[0-9a-f]{40}', str(value['candidate']))):
        raise ValueError('original native run allocation required')
    exit_record_path = private_path(exit_record_path)
    status = terminal_exit(work, value['candidate'], identity, exit_record_path)
    targets = native_targets(work)
    manifest = work / 'build-terminal-targets.json'
    write_once(manifest, dict(kind='native-terminal-cargo-targets-v1',
        candidate=value['candidate'], work=str(work), identity=identity,
        allocation_sha256=digest(allocation), terminal_status=status,
        exit_record=str(exit_record_path), exit_record_sha256=digest(exit_record_path),
        commands_used_sha256=command_uses_digest(work), targets=targets,
        source_and_logs_retained=True, product_resource_cleanup_claimed=False))
    return manifest


def handoff_native_builds(store, manifest_path, receipt_path, evidence_path, *, now):
    """Explicit trusted-operator handoff under Store.lock; never automatic GC.

    Reference absence is supplied by the original trusted operator, not inferred
    from a test exit or an incomplete nonprivileged /proc view. Unknown/failed
    observations cannot authorize a move. No subprocess, sudo or process signal
    is used here. This settles only disposable build-byte custody.
    """
    manifest_path = private_path(manifest_path)
    value = json_file(manifest_path, limit=16 * 1024**2)
    if (value.get('kind') not in ('native-cargo-targets-v1', 'native-terminal-cargo-targets-v1')
            or not re.fullmatch('[0-9a-f]{40}', str(value.get('candidate')))):
        raise ValueError('original native target manifest required')
    work = private_path(Path(value['work']), directory=True)
    if manifest_path.parent != work or [work.stat().st_dev, work.stat().st_ino] != value['identity']:
        raise ValueError('native run identity changed')
    allocation = work / 'build-allocation.json'
    if (digest(allocation) != value['allocation_sha256']
            or json_file(allocation) != dict(candidate=value['candidate'], identity=value['identity'])
            or command_uses_digest(work) != value['commands_used_sha256']):
        raise ValueError('native allocation, original completion or command uses changed')
    status = 'passed'
    if value['kind'] == 'native-cargo-targets-v1':
        completion = private_path(Path(value['completion']))
        if completion.parent != work or digest(completion) != value['completion_sha256']:
            raise ValueError('original native completion changed')
    else:
        exit_record = private_path(Path(value['exit_record']))
        status = terminal_exit(work, value['candidate'], value['identity'], exit_record)
        if digest(exit_record) != value['exit_record_sha256'] or status != value['terminal_status']:
            raise ValueError('original native failed/cancelled exit changed')
    receipt_path = private_path(receipt_path)
    receipt = json_file(receipt_path, limit=4096)
    Store.receipt(receipt, value)
    evidence_path = private_path(evidence_path)
    observed = json_file(evidence_path)
    if (set(observed) != {'manifest_sha256', 'references_absent', 'mounts_absent',
                         'observation_complete', 'observation_error'}
            or receipt['status'] != status or evidence_path.is_relative_to(store.root)
            or digest(evidence_path) != receipt['evidence_sha256']
            or observed.get('manifest_sha256') != digest(manifest_path)
            or observed.get('references_absent') is not True
            or observed.get('mounts_absent') is not True
            or observed.get('observation_complete') is not True
            or observed.get('observation_error') is not None):
        raise ValueError('complete trusted cleanup observation required; unknown references stay active')
    if type(now) is not int or now <= 0:
        raise ValueError('finite positive retirement time required')
    if any(store.read(path.stem)[2]['finished'] >= now for path in store.record_paths()):
        # Do not choose the latest/rollback by UUID when completion seconds tie
        # or the host clock moves backwards. The operator keeps both in place.
        raise ValueError('native handoff time must follow earlier retirements')
    targets = []
    for item in value['targets']:
        path = private_path(Path(item['path']), directory=True)
        info = path.stat()
        if (path.name != 'target' or not path.is_relative_to(work)
                or any(part in ('.lake', '.git') for part in path.relative_to(work).parts)
                or [info.st_dev, info.st_ino] != item['identity']):
            raise ValueError('Cargo target identity changed before handoff')
        allocated_bytes(path)  # Existing owner/type/mount checks, before any move.
        targets.append(path)
    if not targets or any(evidence_path.is_relative_to(path) or receipt_path.is_relative_to(path)
                          for path in targets):
        raise ValueError('nonempty build list and independently preserved cleanup evidence required')
    if len(set(targets)) != len(targets) or any(
            a != b and a.is_relative_to(b) for a in targets for b in targets):
        raise ValueError('duplicate or overlapping Cargo targets')
    payload = store.allocate(kind='build', candidate=value['candidate'])
    handoff = work / 'build-handoff.json'
    write_once(handoff, dict(manifest_sha256=digest(manifest_path), payload=str(payload),
        receipt_sha256=digest(receipt_path), trusted_evidence_sha256=digest(evidence_path)))
    for index, path in enumerate(targets):
        path.rename(payload / str(index))
        fsync_directory(path.parent)
    fsync_directory(payload)
    store.retire(payload.name, receipt_path, evidence_path, now=now)
    return payload


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    commands = parser.add_subparsers(dest='command', required=True)
    allocate = commands.add_parser('allocate')
    allocate.add_argument('--kind', choices=sorted(KINDS), required=True)
    allocate.add_argument('--candidate', required=True)
    retire = commands.add_parser('retire')
    retire.add_argument('--id', required=True)
    retire.add_argument('--receipt', type=Path, required=True)
    retire.add_argument('--evidence', type=Path, required=True)
    terminal = commands.add_parser('record-native-terminal')
    terminal.add_argument('--work', type=Path, required=True)
    terminal.add_argument('--exit-record', type=Path, required=True)
    handoff = commands.add_parser('handoff-native')
    handoff.add_argument('--manifest', type=Path, required=True)
    handoff.add_argument('--receipt', type=Path, required=True)
    handoff.add_argument('--evidence', type=Path, required=True)
    collect = commands.add_parser('collect')
    collect.add_argument('--apply', action='store_true')
    collect.add_argument('--keep-days', type=int, default=7)
    collect.add_argument('--max-count', type=int, default=20)
    collect.add_argument('--max-bytes', type=int, default=100 * 1024**3)
    collect.add_argument('--keep-latest', type=int, default=0)
    args = parser.parse_args(argv)
    store = Store(args.root)
    with store.lock():
        if args.command == 'allocate':
            print(store.allocate(kind=args.kind, candidate=args.candidate))
        elif args.command == 'retire':
            store.retire(args.id, args.receipt, args.evidence, now=int(time.time()))
        elif args.command == 'record-native-terminal':
            print(record_native_terminal(args.work, args.exit_record))
        elif args.command == 'handoff-native':
            print(handoff_native_builds(store, args.manifest, args.receipt, args.evidence,
                now=int(time.time())))
        else:
            import json
            print(json.dumps(store.collect(now=int(time.time()), apply=args.apply,
                keep_days=args.keep_days, max_count=args.max_count, max_bytes=args.max_bytes,
                keep_latest=args.keep_latest)))


if __name__ == '__main__':
    main()
