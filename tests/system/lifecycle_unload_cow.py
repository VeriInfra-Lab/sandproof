"""Read-only source isolation checks for the EXISTING normal lifecycle scenario.

The caller obtains the anchor from UnloadRuntime.unload_evidence, constructs this
probe before Resume, and supplies record_running plus assert_resumed_workload
after the latter has actually advanced the original guest RAM/disk witness.
No VM, manager, guest command, snapshot or lifecycle operation is created here.
Actual source bytes are reread using the existing protected checked_file reader.
This checks one observed guest write, not all future writes or shared extents.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import copy
import hashlib
import os
from pathlib import Path
import stat

from lifecycle_unload_runtime import checked_file, remaining
from tools.system_acceptance import private_path


ARTIFACTS = ('state.bin', 'memory.bin', 'disk-0.img')
BACKING_FIELDS = ('materials', 'manifest', 'completion', 'material_paths')


def _positive(value):
    return type(value) is int and 0 < value < 2**64


def _digest(value):
    return (type(value) is list and len(value) == 32
            and all(type(byte) is int and 0 <= byte <= 255 for byte in value))


def _file_stamp(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns,
            value.st_ctime_ns, value.st_mode, value.st_uid, value.st_gid,
            value.st_nlink)


def _directory_stamp(value, *, sealed):
    identity = (value.st_dev, value.st_ino, value.st_mode, value.st_uid, value.st_gid)
    # Other original bundles can be created in the store; this sealed bundle
    # itself must keep the exact directory entries and metadata.
    return identity + ((value.st_size, value.st_mtime_ns, value.st_ctime_ns,
                        value.st_nlink) if sealed else ())


class SealedSourceProbe:
    """Hold the original directories until a single after-write observation.

    Always close in the scenario's finally block if Resume/workload fails before
    verify_after_guest_write. That method also closes on either success or error.
    Closing descriptors never removes or changes any source material.
    """

    def __init__(self, physical, *, session, witness_before, deadline):
        self._directories = []
        self._closed = False
        try:
            remaining(deadline)
            self._deadline = deadline
            if not isinstance(session, str) or not session:
                raise ValueError('original source session required')
            self.session = session
            self._backing = copy.deepcopy({name: physical[name] for name in BACKING_FIELDS})
            facts = copy.deepcopy(physical['facts'])
            if (not _digest(facts['manifest_digest'])
                    or facts['manifest_digest'] != self._backing['manifest']['sha256']
                    or not all(_positive(facts[name]) for name in (
                        'capture', 'source_incarnation', 'material_bytes'))):
                raise ValueError('original adopted manifest digest required')
            self._facts = facts
            witness = copy.deepcopy(witness_before)
            nonce = witness['nonce']
            if (type(nonce) is not str or len(nonce) != 64
                    or any(char not in '0123456789abcdef' for char in nonce)
                    or not all(_positive(witness[name]) for name in (
                        'pid', 'start_time_ticks', 'counter'))):
                raise ValueError('original pre-resume RAM/disk witness required')
            self._guest = dict(ram_nonce_sha256=hashlib.sha256(nonce.encode()).hexdigest(),
                guest_pid=witness['pid'], guest_start_time_ticks=witness['start_time_ticks'],
                counter=witness['counter'] + 1)
            paths = self._backing['material_paths']
            if type(paths) is not list or len(paths) != 2:
                raise ValueError('one original bundle and completion path required')
            self._bundle, completion = (Path(path) for path in paths)
            key = self._bundle.name.removeprefix('capture-')
            if (not self._bundle.name.startswith('capture-') or len(key) != 64
                    or any(char not in '0123456789abcdef' for char in key)
                    or completion != self._bundle.parent / ('completed-' + key)):
                raise ValueError('original bundle/completion directory key required')
            for path, sealed in ((self._bundle.parent, False), (self._bundle, True)):
                private_path(path, directory=True)
                before = path.lstat()
                descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
                stamp = _directory_stamp(before, sealed=sealed)
                self._directories.append((path, descriptor, stamp, sealed))
                if _directory_stamp(os.fstat(descriptor), sealed=sealed) != stamp:
                    raise ValueError('original source directory replaced before anchor')
            materials = self._backing['materials']
            if (type(materials) is not list or len(materials) != len(ARTIFACTS)
                    or [entry['name'] for entry in materials] != list(ARTIFACTS)
                    or sum(entry['bytes'] for entry in materials) != facts['material_bytes']):
                raise ValueError('original complete manifest-bound material table required')
            self._files = [(self._bundle / 'manifest.json', self._backing['manifest'])]
            self._files.extend((self._bundle / name, entry)
                               for name, entry in zip(ARTIFACTS, materials, strict=True))
            self._files.append((completion, self._backing['completion']))
            self._stamps = []
            for path, measurement in self._files:
                before = path.lstat()
                if (not stat.S_ISREG(before.st_mode) or stat.S_IMODE(before.st_mode) != 0o400
                        or before.st_uid != os.geteuid() or before.st_nlink != 1
                        or not _positive(measurement['bytes']) or not _digest(measurement['sha256'])
                        or (before.st_dev, before.st_ino, before.st_size)
                        != (measurement['device'], measurement['inode'], measurement['bytes'])):
                    raise ValueError('original sealed source file differs from unload evidence')
                self._stamps.append(_file_stamp(before))
            self._check_anchor()
            remaining(deadline)
        except BaseException:
            self.close()
            raise

    def _check_anchor(self):
        if self._closed:
            raise ValueError('original source observation already closed')
        for path, descriptor, stamp, sealed in self._directories:
            private_path(path, directory=True)
            if (_directory_stamp(path.lstat(), sealed=sealed) != stamp
                    or _directory_stamp(os.fstat(descriptor), sealed=sealed) != stamp):
                raise ValueError('original source directory changed')
        if {path.name for path in self._bundle.iterdir()} != set(ARTIFACTS) | {'manifest.json'}:
            raise ValueError('original sealed source file set changed')
        for (path, _), stamp in zip(self._files, self._stamps, strict=True):
            if _file_stamp(path.lstat()) != stamp:
                raise ValueError('original sealed source file changed')

    def verify_after_guest_write(self, running, continuity, deadline):
        """Called only after the existing SDK witness write and readback succeed."""
        try:
            remaining(deadline)
            if deadline > self._deadline:
                raise ValueError('original source observation deadline cannot be extended')
            if (running['session'] != self.session or running.get('material_use_pin') is not True
                    or {name: running['running_backing'][name] for name in BACKING_FIELDS}
                    != self._backing):
                raise ValueError('original restored source and backing pin required')
            if (any(type(continuity.get(name)) is not type(expected)
                    or continuity.get(name) != expected for name, expected in self._guest.items())
                    or continuity.get('new_authorization') is not True
                    or continuity.get('old_authorizations_rejected') is not True):
                raise ValueError('original process RAM/disk advance evidence required')
            self._check_anchor()
            observed = []
            for path, expected in self._files:
                remaining(deadline)
                actual = checked_file(path, expected['bytes'], mode=0o400)
                if any(actual[name] != expected[name] for name in (
                        'bytes', 'sha256', 'device', 'inode')):
                    raise ValueError('source bytes differ from original adopted manifest evidence')
                observed.append(dict(name=path.name, **actual))
                remaining(deadline)
            # Recheck ALL files after ALL reads, including earlier measured files.
            self._check_anchor()
            return dict(scope='one observed guest RAM/disk advance; full source reread',
                capture=self._facts['capture'], source_incarnation=self._facts['source_incarnation'],
                manifest_digest=copy.deepcopy(self._facts['manifest_digest']),
                source_unchanged=True, original_backing_pin=True,
                directories=[dict(device=stamp[0], inode=stamp[1])
                             for _, _, stamp, _ in self._directories],
                files=observed, guest=copy.deepcopy(self._guest))
        finally:
            self.close()

    def close(self):
        if not self._closed:
            self._closed = True
            for _, descriptor, _, _ in self._directories:
                os.close(descriptor)

    def __enter__(self):
        if self._closed:
            raise ValueError('original source observation already closed')
        return self

    def __exit__(self, *_):
        self.close()
