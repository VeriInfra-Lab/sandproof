"""Run only inside an isolated build VM with the worker modules in /input."""

import sys as _guard_sys
if _guard_sys.flags.optimize:
    raise RuntimeError("optimized_python")

from pathlib import Path
import hashlib
import json
import os
import stat
import sys
import time
from unittest.mock import patch

sys.path.insert(0, '/input')
from template_inputs import sha256
from template_worker import Commands, populate_ext4, unmount_ext4


def snapshot(root, names):
    rows, links = {}, {}
    for name in names:
        path = root / name
        meta = path.lstat()
        row = dict(type=stat.S_IFMT(meta.st_mode), mode=stat.S_IMODE(meta.st_mode),
                   uid=meta.st_uid, gid=meta.st_gid, mtime_ns=meta.st_mtime_ns,
                   xattrs={key: os.getxattr(path, key, follow_symlinks=False).hex()
                           for key in sorted(os.listxattr(path, follow_symlinks=False))})
        if stat.S_ISLNK(meta.st_mode):
            row['target'] = os.readlink(path)
        elif stat.S_ISREG(meta.st_mode):
            row.update(size=meta.st_size, sha256=sha256(path),
                       hardlink_to=links.setdefault((meta.st_dev, meta.st_ino), name))
        rows[name] = row
    return rows


def exercise():
    assert os.geteuid() == 0 and Path('/dev/vda').is_block_device()
    work = Path('/output/ext4-check')
    work.mkdir(mode=0o700)
    source = work / 'source'
    (source / 'data/sub').mkdir(parents=True)
    (source / 'data/value').write_bytes(b'original\x00\xff')
    os.chown(source / 'data/value', 123, 456)
    (source / 'data/value').chmod(0o4750)
    os.setxattr(source / 'data/value', 'user.fixture', b'value')
    os.setxattr(source / 'data/sub', 'user.directory', b'directory')
    os.link(source / 'data/value', source / 'data/hard')
    (source / 'data/absolute').symlink_to('/data/value')
    (source / 'data/sub/relative').symlink_to('../value')
    os.lchown(source / 'data/absolute', 123, 456)
    with (source / 'data/sparse').open('xb') as output:
        output.write(b'head')
        output.seek(4 * 1024**2)
        output.write(b'tail')
        output.truncate(8 * 1024**2)
    names = sorted(path.relative_to(source).as_posix() for path in source.rglob('*'))
    for name in names:
        os.utime(source / name, ns=(946684800123456789, 946684800123456789), follow_symlinks=False)
    expected = snapshot(source, names)
    evidence = []
    for failure in (None, 'slow-unmount', 'cp', 'mount'):
        case = work / (failure or 'success')
        case.mkdir()
        disk = case / 'candidate.ext4'
        with disk.open('xb') as output:
            output.truncate(512 * 1024**2)
        original = TimeoutError('injected original deadline')
        original_run = Commands.run

        def run_with_delayed_unmount(runner, args, **kwargs):
            if failure == 'slow-unmount' and Path(args[0]).name == 'umount':
                # Deterministic injected latency at the actual unmount: a
                # timer releases the real tool after the old five-second
                # grace, while the original build deadline still has time.
                release = ('import os,signal,sys; '
                           'signal.signal(signal.SIGALRM, '
                           'lambda *_: os.execv(sys.argv[1], sys.argv[1:])); '
                           'signal.setitimer(signal.ITIMER_REAL, 6); signal.pause()')
                args = [sys.executable, '-c', release, *args]
            return original_run(runner, args, **kwargs)

        class InjectedCommands(Commands):
            def run(self, args, **kwargs):
                if Path(args[0]).name == failure:
                    if failure == 'mount':
                        super().run(args, **kwargs)  # Mount succeeded before its late result.
                    self.deadline = time.monotonic() - 1
                    raise original
                return super().run(args, **kwargs)

        with (case / 'build.log').open('xb') as log:
            commands = InjectedCommands(time.monotonic() + 60, log, 1024**2)
            try:
                with patch.object(Commands, 'run', run_with_delayed_unmount):
                    populate_ext4(commands, source, disk)
            except TimeoutError as error:
                assert failure in ('cp', 'mount') and error is original
            else:
                assert failure in (None, 'slow-unmount')
            mount = case / 'candidate-mount'
            assert not mount.exists() and not os.path.ismount(mount)
            if failure in (None, 'slow-unmount'):
                commands.run(['/usr/sbin/e2fsck', '-fn', str(disk)])
                mount.mkdir()
                commands.run(['/usr/bin/mount', '-t', 'ext4', '-o',
                              'loop,ro,noload,nosuid,nodev,noexec', str(disk), str(mount)])
                try:
                    observed = snapshot(mount, names)
                    assert observed == expected, 'candidate content or metadata changed'
                    assert (mount / 'data/sparse').stat().st_blocks * 512 < 1024**2
                finally:
                    unmount_ext4(commands, mount)
                    mount.rmdir()
            assert not any(path.read_text().strip() == str(disk)
                           for path in Path('/sys/block').glob('loop*/loop/backing_file'))
        evidence.append(dict(case=failure or 'success', passed=True, mount_and_loop_gone=True))
        disk.unlink()
    return dict(passed=True, cases=evidence, entries=len(names),
                snapshot_sha256=hashlib.sha256(json.dumps(expected, sort_keys=True).encode()).hexdigest())


if __name__ == '__main__':
    print(json.dumps(exercise()))
