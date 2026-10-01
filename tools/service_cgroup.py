"""Explicit root-owned cgroup v2 budget for services and inherited relays.

The operator and per-VM groups must remain outside this leaf. No global
configuration, process migration by PID, automatic killing, or guessed cleanup.
"""
import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys

ROOT = Path('/sys/fs/cgroup')
BOOT = Path('/proc/sys/kernel/random/boot_id')
OWNER_UID = 0


def private(path, *, directory=True):
    path = Path(path)
    if not path.is_absolute() or path.resolve(strict=True) != path:
        raise ValueError('absolute nonsymlink operator path required')
    meta = path.lstat()
    if (meta.st_uid != OWNER_UID or meta.st_mode & 0o077
            or not (stat.S_ISDIR(meta.st_mode) if directory else stat.S_ISREG(meta.st_mode))
            or (not directory and (meta.st_nlink != 1 or meta.st_size > 16384))):
        raise ValueError('private root-owned operator path required')
    return meta


def identity(path):
    meta = private(path)
    return {'device': meta.st_dev, 'inode': meta.st_ino}


def require_mount():
    if os.geteuid() != 0 or sys.platform != 'linux':
        raise ValueError('root Linux cgroup v2 required')
    mounts = [line.split(' - ', 1) for line in Path('/proc/self/mountinfo').read_text().splitlines()]
    if not any(left.split()[4] == str(ROOT) and right.split()[0] == 'cgroup2' for left, right in mounts):
        raise ValueError('cgroup v2 mount required')


def filesystem(parent):
    require_mount()
    if parent == ROOT or not parent.is_relative_to(ROOT):
        raise ValueError('exclusive delegated cgroup parent required')
    private(parent)
    if parent.stat().st_dev != ROOT.stat().st_dev:
        raise ValueError('cgroup filesystem changed')
    if ((parent / 'cgroup.type').read_text().strip() != 'domain'
            or not {'cpu', 'memory'} <= set((parent / 'cgroup.subtree_control').read_text().split())
            or (parent / 'cgroup.procs').read_text().strip()):
        raise ValueError('empty parent with CPU/memory already delegated required')


def limits(cpu_quota_us, memory_bytes):
    if (type(cpu_quota_us) is not int or not 1000 <= cpu_quota_us <= 25600000
            or type(memory_bytes) is not int or memory_bytes < 16 * 1024**2
            or memory_bytes % os.sysconf('SC_PAGE_SIZE')):
        raise ValueError('positive bounded CPU quota and page-aligned memory budget required')
    return {'cpu.max': f'{cpu_quota_us} 100000', 'cpu.max.burst': '0', 'memory.max': str(memory_bytes),
            'memory.swap.max': '0', 'memory.oom.group': '1'}


def write_new(path, value):
    private(path.parent)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as output:
        json.dump(value, output, indent=2)
        output.write('\n')
        output.flush()
        os.fsync(output.fileno())
    fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def prepare(parent, name, record, *, cpu_quota_us, memory_bytes):
    parent, record = Path(parent), Path(record)
    filesystem(parent)
    if not re.fullmatch(r'svc-[a-z0-9-]{1,64}', name):
        raise ValueError('invalid service cgroup name')
    path = parent / name
    policy = limits(cpu_quota_us, memory_bytes)
    value = {'schema_version': 1, 'boot_id': BOOT.read_text().strip(), 'path': str(path),
             'parent_identity': identity(parent), 'limits': policy}
    # An interrupted create cannot be adopted by name. Retain its intent for
    # operator inspection; only a durable inode record authorizes use/removal.
    if path.exists() or record.exists():
        raise ValueError('service cgroup or record already exists')
    write_new(record.with_suffix('.intent.json'), value)
    create_leaf(path)
    for field, setting in policy.items():
        (path / field).write_text(setting)
    value['identity'] = identity(path)
    write_new(record, value)
    checked(record)
    return value


def create_leaf(path):
    path.mkdir(mode=0o700)


def checked(record):
    record = Path(record)
    private(record.parent)
    private(record, directory=False)
    value = json.loads(record.read_text())
    if (set(value) != {'schema_version', 'boot_id', 'path', 'parent_identity', 'limits', 'identity'}
            or value['schema_version'] != 1 or value['boot_id'] != BOOT.read_text().strip()):
        raise ValueError('service cgroup record or boot identity changed')
    path = Path(value['path'])
    filesystem(path.parent)
    if not re.fullmatch(r'svc-[a-z0-9-]{1,64}', path.name):
        raise ValueError('invalid recorded cgroup path')
    if identity(path.parent) != value['parent_identity'] or identity(path) != value['identity']:
        raise ValueError('service cgroup identity changed')
    policy = value['limits']
    expected = limits(int(policy['cpu.max'].split()[0]), int(policy['memory.max']))
    if policy != expected:
        raise ValueError('invalid recorded cgroup limits')
    if (any((path / field).read_text().strip() != setting for field, setting in policy.items())
            or (path / 'cgroup.type').read_text().strip() != 'domain'
            or (path / 'cgroup.subtree_control').read_text().strip()
            or any(p.is_dir() for p in path.iterdir())):
        raise ValueError('service cgroup limits or leaf topology changed')
    return value


def counters(path):
    return {key: int(value) for key, value in (line.split() for line in path.read_text().splitlines())}


def observe(record, *, pids=()):
    value = checked(record)
    path = Path(value['path'])
    expected = '0::/' + str(path.relative_to(ROOT))
    for pid in pids:
        if Path(f'/proc/{pid}/cgroup').read_text().strip() != expected:
            raise ValueError('service process outside its recorded cgroup')
    return {'path': str(path), 'identity': value['identity'], 'limits': value['limits'],
            'pids': [int(pid) for pid in (path / 'cgroup.procs').read_text().split()],
            'cpu_stat': counters(path / 'cpu.stat'), 'memory_events': counters(path / 'memory.events'),
            'memory_current': int((path / 'memory.current').read_text()),
            'memory_peak': int((path / 'memory.peak').read_text()),
            'populated': counters(path / 'cgroup.events')['populated']}


def remove(record):
    value = checked(record)
    snapshot = observe(record)
    if snapshot['pids'] or snapshot['populated']:
        raise ValueError('service cgroup still contains processes; retain original identity')
    Path(value['path']).rmdir()
    return snapshot | {'removed': True}


def join_exec(record, command):
    if not command or not Path(command[0]).is_absolute():
        raise ValueError('explicit absolute service executable required')
    value = checked(record)
    path = Path(value['path'])
    if os.sched_getscheduler(0) != os.SCHED_OTHER or Path('/proc/self/oom_score_adj').read_text().strip() == '-1000':
        raise ValueError('ordinary scheduling and OOM-eligible service required')
    # This short, single-threaded launcher migrates only itself before exec.
    # Children inherit membership; no preexec_fn or racy external PID migration.
    (path / 'cgroup.procs').write_text('0')
    observe(record, pids=[os.getpid()])
    os.execv(command[0], command)


def prefix(record):
    return [sys.executable, str(Path(__file__).resolve()), 'exec', '--record', str(record), '--']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='action', required=True)
    create = commands.add_parser('prepare')
    create.add_argument('--parent', type=Path, required=True)
    create.add_argument('--name', required=True)
    create.add_argument('--cpu-quota-us', type=int, required=True)
    create.add_argument('--memory-bytes', type=int, required=True)
    for command in (create, commands.add_parser('check'), commands.add_parser('remove'), commands.add_parser('exec')):
        command.add_argument('--record', type=Path, required=True)
    command.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.action == 'prepare':
        result = prepare(args.parent, args.name, args.record, cpu_quota_us=args.cpu_quota_us, memory_bytes=args.memory_bytes)
    elif args.action == 'check':
        result = observe(args.record)
    elif args.action == 'remove':
        result = remove(args.record)
    else:
        join_exec(args.record, args.command[1:] if args.command[:1] == ['--'] else args.command)
        raise AssertionError('exec returned')
    print(json.dumps(result))
