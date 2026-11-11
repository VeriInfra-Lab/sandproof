"""Plan, provision and reopen the storage for one Sandproof runtime pool."""
import argparse
import json
import os
import re
import stat
import subprocess
from pathlib import Path
import sys

from tools.capture_acceptance import validate_settings
from tools.check_kvm_host import CheckFailed, trusted_path
from tools.system_acceptance import Failure, json_file, private_path, write_once
from tools.template_physical_storage import activation_budget, activation_capacity
from tools.xfs_project_quota import ENV, capture, project_hard_limit


GIB = 1024**3
RECEIPT = 'storage.json'
IMAGE = 'volume.xfs'
CONTROL = 'control'
DATA = 'data'


def volume_plan(profile, directory, *, image_bytes, host_reserve_bytes):
    """Keep backing-disk admission distinct from the eventual filesystem check."""
    if profile['public_build'] is not None:
        raise ValueError('single-host volume planning requires the runtime-only profile')
    if image_bytes < 0 or host_reserve_bytes < 0:
        raise ValueError('nonnegative image and host-reserve bytes required')
    if not image_bytes:
        observed = activation_capacity(profile, directory)
        missing = max(0, observed['required_bytes'] + host_reserve_bytes - observed['available_bytes'])
        return dict(mode='existing_filesystem', volume=observed,
                    host_reserve_bytes=host_reserve_bytes, missing_host_bytes=missing,
                    fits=not (missing or observed['missing_inodes']))

    budget = activation_budget(profile)
    free = os.statvfs(directory)
    outer_available = free.f_bavail * free.f_frsize
    outer_missing = max(0, image_bytes + host_reserve_bytes - outer_available)
    inner_lower_bound_missing = max(0, budget['required_bytes'] - image_bytes)
    fits = not (outer_missing or inner_lower_bound_missing or free.f_favail < 1)
    return dict(mode='file_volume', backing_available_bytes=outer_available,
                backing_available_inodes=free.f_favail, image_bytes=image_bytes,
                host_reserve_bytes=host_reserve_bytes, missing_host_bytes=outer_missing,
                activation=budget, missing_image_bytes_lower_bound=inner_lower_bound_missing,
                fits=fits, volume_check_pending=True)


def run(argv, *, pass_fds=()):
    return subprocess.run(argv, capture_output=True, text=True, check=True,
                          env=ENV, pass_fds=pass_fds, timeout=60).stdout


def deployment_directory(path):
    directory = trusted_path(path)
    if (directory.resolve(strict=True) != directory
            or stat.S_IMODE(directory.stat().st_mode) != 0o711
            or any(not parent.is_dir() or not parent.stat().st_mode & stat.S_IXOTH
                   for parent in (directory, *directory.parents))):
        raise ValueError('storage requires a root-owned 0711 directory and traversable parents')
    return directory


def mount_info(path):
    entries = json.loads(run(['/usr/bin/findmnt', '--json', '--target', str(path),
                              '--output', 'TARGET,SOURCE,FSTYPE,OPTIONS,UUID']))['filesystems']
    if len(entries) != 1:
        raise ValueError('expected one filesystem for the designated storage path')
    return entries[0]


def quota_mount(entry):
    options = set(entry['options'].split(','))
    return (entry['fstype'] == 'xfs' and bool(entry['uuid']) and 'rw' in options
            and bool({'prjquota', 'pquota'} & options)
            and not {'pqnoenforce', 'nodev', 'noexec'} & options)


def reflink_enabled(entry):
    # xfs_info is a shell wrapper that looks up xfs_spaceman on PATH.
    # Query the mounted filesystem directly in the same restricted environment.
    tool = trusted_path(Path('/usr/sbin/xfs_spaceman'))
    info = run([str(tool), '-c', 'info', entry['target']])
    return re.search(r'\breflink=1(?:\s|$)', info) is not None


def plan(profile, directory, *, image_bytes, host_reserve_bytes):
    outer = mount_info(directory)
    if image_bytes is None:
        image_bytes = 0 if quota_mount(outer) and reflink_enabled(outer) else (
            activation_budget(profile)['required_bytes'] + GIB)
    if not image_bytes and (not quota_mount(outer) or not reflink_enabled(outer)):
        raise ValueError('existing filesystem lacks writable XFS project quotas or reflink')
    return volume_plan(profile, directory, image_bytes=image_bytes,
                       host_reserve_bytes=host_reserve_bytes)


def original_project(profile, settings, data):
    if profile['public_build'] is not None or len(profile['pools']) != 1:
        raise ValueError('installation requires one runtime pool')
    pool = profile['pools'][0]
    capture_settings = validate_settings(settings, 1)[0]
    project = capture_settings['material_project']
    parent = Path(capture_settings['layout_parent'])
    if (parent / 'v' != Path(pool['chroot_base']) or project['hard_bytes'] % 1024
            or not any(item['root'] == str(parent / 's') and item['owner_uid'] == 0
                       and item['peak_bytes'] == project['hard_bytes']
                       and item['inode_budget'] == project['hard_inodes'] for item in profile['service_data'])):
        raise ValueError('capture layout and project must match the full physical budget')
    roots = [profile['coordination_file'], pool['database'], pool['node_state_dir'], pool['chroot_base'],
             *(item['root'] for item in profile['host_template_caches']),
             *(item['root'] for item in profile['service_data'])]
    if any(Path(path) == data or not Path(path).is_relative_to(data) for path in roots):
        raise ValueError('all managed profile paths must be children of the designated storage root')
    return project_limits(project)


def project_limits(project):
    return {key: project[key] for key in ('project_id', 'hard_bytes', 'hard_inodes')}


def quota_limits(entry, project, *, configure=False):
    if not quota_mount(entry) or not reflink_enabled(entry):
        raise ValueError('storage mount lacks writable XFS project quotas or reflink')
    mount = Path(entry['target'])
    tool = trusted_path(Path('/usr/sbin/xfs_quota'))
    state = capture(tool, 'state -p', mount)
    if (state.count('Project quota state on ') != 1 or str(mount) not in state
            or state.count('Accounting: ON') != 1 or state.count('Enforcement: ON') != 1):
        raise ValueError('actual project accounting/enforcement not confirmed ON')
    project_id = project['project_id']
    if configure:
        # Only a freshly formatted private image may have its quota written here.
        capture(tool, f"limit -p bhard={project['hard_bytes']//1024}k "
                      f"ihard={project['hard_inodes']} {project_id}", mount)
    window = f' -L {project_id} -U {project_id}'
    blocks = capture(tool, 'report -p -b -N -n' + window, mount)
    inodes = capture(tool, 'report -p -i -N -n' + window, mount)
    if (project_hard_limit(blocks, project_id)*1024 != project['hard_bytes']
            or project_hard_limit(inodes, project_id) != project['hard_inodes']):
        raise ValueError('actual kernel hard limits differ from protected capture budget')


def image_identity(image):
    info = image.lstat()
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != 0
            or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1
            or info.st_size <= 0 or info.st_blocks*512 < info.st_size):
        raise ValueError('private preallocated storage image required')
    backing = mount_info(image)
    if not backing['uuid']:
        raise ValueError('storage image backing filesystem requires a stable UUID')
    return dict(backing_uuid=backing['uuid'], inode=info.st_ino, size_bytes=info.st_size)


def mount_image(image, data):
    run(['/usr/bin/mount', '-t', 'xfs', '-o', 'loop,prjquota', str(image), str(data)])


def check_loop(entry, data, image):
    if entry['target'] != str(data) or not re.fullmatch(r'/dev/loop[0-9]+', entry['source']):
        raise ValueError('storage root is not the expected loop mount')
    observed = json.loads(run(['/usr/sbin/losetup', '--list', '--json',
                               '--output', 'BACK-FILE,OFFSET,SIZELIMIT', entry['source']]))['loopdevices']
    if (len(observed) != 1 or observed[0]['back-file'] != str(image)
            or int(observed[0]['offset']) != 0 or int(observed[0]['sizelimit']) != 0):
        raise ValueError('loop mount does not refer to the whole original image')


def require_capacity(profile, data):
    observed = activation_capacity(profile, data)
    if observed['missing_bytes'] or observed['missing_inodes']:
        raise ValueError('formatted storage lacks the full activation budget: '
                         + json.dumps(observed, sort_keys=True))
    return observed


def provision(directory, profile, project, *, image_bytes, host_reserve_bytes):
    data = directory / DATA
    control = directory / CONTROL
    if any(os.path.lexists(path) for path in (control, data)):
        raise ValueError('first installation requires fresh reserved storage paths')
    control.mkdir(mode=0o700)
    image = control / IMAGE
    if image_bytes:
        flags = os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW
        with os.fdopen(os.open(image, flags, 0o600), 'r+b') as stream:
            fd = stream.fileno()
            os.posix_fallocate(fd, 0, image_bytes)
            os.fsync(fd)
            identity = image_identity(image)
            # Format only the descriptor exclusively created above; preserve its preallocation.
            run(['/usr/sbin/mkfs.xfs', '-f', '-K', '-m', 'reflink=1',
                 '/proc/self/fd/' + str(fd)], pass_fds=(fd,))
            os.fsync(fd)
            if identity != image_identity(image):
                raise ValueError('storage image identity changed during formatting')
        free = os.statvfs(directory)
        if free.f_bavail * free.f_frsize < host_reserve_bytes:
            raise ValueError('preallocated image leaves less than the required host reserve')
    else:
        identity = None
    data.mkdir(mode=0o700)
    if identity is not None:
        mount_image(image, data)
        check_loop(mount_info(data), data, image)
        data.chmod(0o700)
    entry = mount_info(data)
    capacity = require_capacity(profile, data)
    quota_limits(entry, project, configure=identity is not None)
    if identity is not None and image_identity(image) != identity:
        raise ValueError('original image changed during installation')
    # Roles can reach their own 0700 children. The original image and receipt
    # are still behind a separate root-only directory.
    data.chmod(0o711)
    receipt = dict(version=1, data_inode=data.stat().st_ino, uuid=entry['uuid'],
                   image=identity, project=project)
    for path in (data, directory):
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    write_once(control / RECEIPT, receipt)
    return dict(receipt=receipt, capacity=capacity)


def reopen(directory, project, *, remount=False):
    data = directory / DATA
    control = private_path(directory / CONTROL, directory=True)
    image = control / IMAGE
    receipt = json_file(private_path(control / RECEIPT))
    if receipt.get('version') != 1 or receipt.get('project') != project:
        raise ValueError('storage receipt does not bind the original project')
    trusted_path(data)
    identity = receipt['image']
    if identity is not None:
        if image_identity(image) != identity:
            raise ValueError('storage image differs from the recorded image')
        if mount_info(data)['target'] != str(data):
            if not remount or any(data.iterdir()):
                raise ValueError('original storage mount is missing or mountpoint is not empty')
            uuid = run(['/usr/sbin/blkid', '-p', '-s', 'UUID', '-o', 'value', str(image)]).strip()
            if uuid != receipt['uuid']:
                raise ValueError('storage image UUID changed')
            mount_image(image, data)
    entry = mount_info(data)
    if identity is not None:
        check_loop(entry, data, image)
    if (entry['uuid'] != receipt['uuid'] or data.stat().st_ino != receipt['data_inode']
            or stat.S_IMODE(data.stat().st_mode) != 0o711):
        raise ValueError('storage filesystem or data-root identity changed')
    quota_limits(entry, project)
    return receipt


def check_acceptance_volume(directory, settings, scratch, staging):
    """Use the installed volume and its quota before a prepared runtime run."""
    directory = deployment_directory(directory)
    data = directory / DATA
    scratch, staging = (trusted_path(path) for path in (scratch, staging))
    capture = validate_settings(settings, 1)[0]
    parent = Path(capture['layout_parent'])
    if (any(path == data or not path.is_relative_to(data) for path in (scratch, staging))
            or not parent.is_relative_to(scratch)):
        raise ValueError('prepared runtime roots must belong to the original installed data directory')
    receipt = reopen(directory, project_limits(capture['material_project']))
    target = mount_info(data)['target']
    if any(mount_info(path)['target'] != target for path in (scratch, staging)):
        raise ValueError('prepared runtime roots must use the same original XFS mount')
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('plan', 'install', 'remount', 'check'))
    parser.add_argument('--profile', type=Path, required=True, help='complete bound physical profile')
    parser.add_argument('--directory', type=Path, required=True,
                        help='designated root-owned 0711 storage directory')
    parser.add_argument('--capture-settings', type=Path, help='approved single-host capture settings')
    parser.add_argument('--image-bytes', type=int, help='override proposed image including format overhead')
    parser.add_argument('--host-reserve-bytes', type=int, default=GIB)
    args = parser.parse_args(argv)
    try:
        profile = json_file(private_path(args.profile))
        directory = deployment_directory(args.directory)
        if args.action == 'plan':
            report = plan(profile, directory, image_bytes=args.image_bytes,
                          host_reserve_bytes=args.host_reserve_bytes)
            print(json.dumps(report, indent=2, sort_keys=True))
            return 0 if report['fits'] else 1
        if os.geteuid() != 0 or args.capture_settings is None:
            raise ValueError('authorized root and approved capture settings required')
        activation_budget(profile)
        settings = json_file(private_path(args.capture_settings), limit=65536)
        project = original_project(profile, settings, directory / DATA)
        if args.action == 'install':
            report = plan(profile, directory, image_bytes=args.image_bytes,
                          host_reserve_bytes=args.host_reserve_bytes)
            if not report['fits']:
                print(json.dumps(report, indent=2, sort_keys=True))
                return 1
            result = provision(directory, profile, project, image_bytes=report.get('image_bytes', 0),
                               host_reserve_bytes=args.host_reserve_bytes)
        else:
            result = reopen(directory, project, remount=args.action == 'remount')
    except (OSError, ValueError, KeyError, CheckFailed, Failure, subprocess.SubprocessError) as error:
        print(f'Storage {args.action} rejected: {error}', file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
