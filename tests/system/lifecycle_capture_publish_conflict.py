"""One test-owned empty publication target; original capture and GC stay native.

No existing directory is adopted or replaced. The original sync gate is released
normally. Only after the exact worker End/kernel exit and original continuation
may this observer remove its own unchanged empty inode. Any unknown state keeps
the obstacle and all original source/work/material responsibilities for review.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import os
from pathlib import Path
import stat

from lifecycle_unload_capture_faults import CaptureHandoffProbe
from lifecycle_unload_runtime import checked_file, original_json, original_process, pidfd_exited, remaining
from tools.system_acceptance import private_path


def _stamp(value):
    return (value.st_dev, value.st_ino, value.st_mode, value.st_uid, value.st_gid,
            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


class EmptyCaptureTarget:
    """Own an exclusive new empty directory, never a preexisting product object."""
    def __init__(self, store, capture_key):
        if (type(capture_key) is not str or len(capture_key) != 64
                or any(char not in '0123456789abcdef' for char in capture_key)):
            raise ValueError('original Rust capture selector required')
        self.store = private_path(store, directory=True)
        self.name = 'capture-' + capture_key
        self.store_fd = self.fd = None
        self.created = self.removed = False
        self.identity = None
        self.owner = os.geteuid()
        self.parent_identity = self.store.stat().st_dev, self.store.stat().st_ino

    def _parent(self):
        if self.store_fd is None:
            raise ValueError('original publication parent is not held')
        held = os.fstat(self.store_fd); current = self.store.lstat()
        for value in (held, current):
            if (not stat.S_ISDIR(value.st_mode) or value.st_uid != self.owner
                    or stat.S_IMODE(value.st_mode) != 0o700
                    or (value.st_dev, value.st_ino) != self.parent_identity):
                raise ValueError('original protected publication parent changed')

    def create(self):
        if self.created or self.store_fd is not None:
            raise ValueError('only one exclusive obstacle creation is allowed')
        self.store_fd = os.open(self.store, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC)
        self._parent()
        os.mkdir(self.name, 0o700, dir_fd=self.store_fd)  # EEXIST is a failure, never adoption.
        self.created = True
        self.fd = os.open(self.name, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC,
                          dir_fd=self.store_fd)
        self.identity = _stamp(os.fstat(self.fd))
        self.unchanged()
        os.fsync(self.store_fd)
        return self.evidence()

    def unchanged(self):
        self._parent()
        if not self.created or self.removed or self.fd is None or self.identity is None:
            raise ValueError('original new obstacle must still be held')
        held = os.fstat(self.fd)
        entry = os.stat(self.name, dir_fd=self.store_fd, follow_symlinks=False)
        if (_stamp(held) != self.identity or _stamp(entry) != self.identity
                or not stat.S_ISDIR(held.st_mode) or held.st_uid != self.owner
                or stat.S_IMODE(held.st_mode) != 0o700 or held.st_nlink != 2
                or os.listdir(self.fd)):
            raise ValueError('changed or nonempty publication obstacle preserved')

    def remove(self):
        self.unchanged()
        os.rmdir(self.name, dir_fd=self.store_fd)
        self.removed = True
        if os.fstat(self.fd).st_nlink != 0:
            raise ValueError('original empty obstacle unlink is not confirmed')
        os.fsync(self.store_fd)
        return self.evidence()

    def evidence(self):
        return dict(capture_target=self.name, created=self.created, removed=self.removed,
                    identity=self.identity, parent_identity=self.parent_identity,
                    scope='test-owned empty target only; no material or capacity release')

    def close(self):
        for name in ('fd', 'store_fd'):
            descriptor = getattr(self, name)
            if descriptor is not None:
                os.close(descriptor); setattr(self, name, None)


class PublishConflictProbe(CaptureHandoffProbe):
    def __init__(self, runtime, client, physical_running, gate_dir):
        super().__init__(runtime, client, physical_running, 'sync-after', gate_dir)
        self.obstacle = None
        self.conflict = None

    def after_acceptance(self, accepted, deadline):
        result = super().after_acceptance(accepted, deadline)
        self.obstacle = EmptyCaptureTarget(
            Path(self.runtime.captures[0]['layout_parent']) / 's', self.question['capture_key'])
        result['publication_obstacle'] = self.obstacle.create()
        return result

    def _capture_gate_command(self):
        if self.stage != 'sync-after' or self.obstacle is None:
            raise ValueError('one original post-sync publication obstacle required')
        self.obstacle.unchanged()
        return 'release'  # The callback succeeds; this is not gate failure injection.

    def finish(self, deadline):
        result = super().finish(deadline)
        self.obstacle.unchanged()
        remaining(deadline)
        full = self.record['capture']['full']
        stage = self.obstacle.store / ('staging-' + self.question['capture_key'])
        before = stage.lstat()
        if (not stat.S_ISDIR(before.st_mode) or before.st_uid != 0
                or stat.S_IMODE(before.st_mode) != 0o500
                or set(os.listdir(stage)) != {'state.bin', 'memory.bin', 'disk-0.img', 'manifest.json'}):
            raise ValueError('complete original stage must remain after publication refusal')
        manifest_file = stage / 'manifest.json'
        manifest = original_json(manifest_file, 262144)
        if manifest['binding'] != full['pause']['binding'] or manifest['compatibility'] != full['compatibility']:
            raise ValueError('original staged manifest differs from accepted Full')
        files = []
        for entry, name, limit in zip(manifest['files'], ('state.bin', 'memory.bin', 'disk-0.img'),
                (full['state_bytes'], full['memory_bytes'], full['disk_bytes']), strict=True):
            remaining(deadline)
            measured = checked_file(stage / name, limit, mode=0o400)
            if entry['relative_path'] != name or entry['length'] != measured['bytes'] or entry['digest'] != measured['sha256']:
                raise ValueError('original refused stage differs from its complete file table')
            files.append(dict(name=name, **measured))
        if ((stage.stat().st_dev, stage.stat().st_ino) != (before.st_dev, before.st_ino)
                or (self.obstacle.store / ('completed-' + self.question['capture_key'])).exists()):
            raise ValueError('original stage changed or acquired a success completion')
        self.obstacle.unchanged()
        self.conflict = dict(original_returned_end=result, complete_stage_files=files,
            manifest=checked_file(manifest_file, 262144, mode=0o400),
            obstacle=self.obstacle.evidence(), callback_released_normally=True,
            syscall_errno='not separately traced in system scene; exact EEXIST checked by original Runtime unit test',
            full_adopted=False, capacity_released=False)
        return self.conflict

    def remove_after_continued(self, deadline):
        if self.conflict is None or not self.recovered or self.obstacle is None:
            raise ValueError('original failed worker and refused complete stage required')
        retained = self.assert_sync_continued(deadline)
        if not pidfd_exited(self.worker_pidfd) or not original_process(self.prior['source']):
            raise ValueError('original worker exit and live original VM required before obstacle removal')
        return dict(retained_responsibility=retained, obstacle=self.obstacle.remove())

    def cleanup(self):
        if self.obstacle is None or not self.obstacle.removed:
            raise ValueError('unknown publication obstacle preserved')
        return super().cleanup()

    def close(self):
        if self.obstacle is not None:
            self.obstacle.close()  # Never remove the obstacle on an unknown outcome.
        super().close()
