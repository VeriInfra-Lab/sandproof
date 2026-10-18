"""Trusted retirement of explicitly managed, disposable CI artifacts.

No discovery/adoption of home directories, proof environments, image caches or
unfinished runs. Only the trusted operator can seal a terminal cleanup receipt.
This is storage housekeeping, never a test result or resource-cleanup proof.
"""
import argparse
from contextlib import contextmanager
import fcntl
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

    def collect(self, *, now, apply=False, keep_days=7, max_count=20, max_bytes=100 * 1024**3):
        if (type(now) is not int or now <= 0 or type(keep_days) is not int or not 1 <= keep_days <= 365
                or type(max_count) is not int or not 0 <= max_count <= 1000
                or type(max_bytes) is not int or not 0 <= max_bytes <= 1024**5):
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
            eligible = (value['state'] == 'deleting' or age >= DAY and
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


def fsync_directory(path):
    fd = os.open(path, os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


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
    collect = commands.add_parser('collect')
    collect.add_argument('--apply', action='store_true')
    collect.add_argument('--keep-days', type=int, default=7)
    collect.add_argument('--max-count', type=int, default=20)
    collect.add_argument('--max-bytes', type=int, default=100 * 1024**3)
    args = parser.parse_args(argv)
    store = Store(args.root)
    with store.lock():
        if args.command == 'allocate':
            print(store.allocate(kind=args.kind, candidate=args.candidate))
        elif args.command == 'retire':
            store.retire(args.id, args.receipt, args.evidence, now=int(time.time()))
        else:
            import json
            print(json.dumps(store.collect(now=int(time.time()), apply=args.apply,
                keep_days=args.keep_days, max_count=args.max_count, max_bytes=args.max_bytes)))


if __name__ == '__main__':
    main()
