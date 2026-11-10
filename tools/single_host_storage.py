"""Read-only capacity planning for a single runtime pool and its storage volume."""
import argparse
import json
import os
from pathlib import Path
import sys

from tools.system_acceptance import json_file, private_path
from tools.template_physical_storage import activation_budget, activation_capacity


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


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', type=Path, required=True, help='complete bound physical profile')
    parser.add_argument('--directory', type=Path, required=True, help='existing backing or XFS directory')
    parser.add_argument('--image-bytes', type=int, default=0, help='proposed image including format overhead')
    parser.add_argument('--host-reserve-bytes', type=int, default=0)
    args = parser.parse_args(argv)
    try:
        report = volume_plan(json_file(private_path(args.profile)), args.directory,
                             image_bytes=args.image_bytes, host_reserve_bytes=args.host_reserve_bytes)
    except (OSError, ValueError, KeyError) as error:
        print(f'Storage plan rejected: {error}', file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['fits'] else 1


if __name__ == '__main__':
    sys.exit(main())
