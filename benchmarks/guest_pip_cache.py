"""Explicit image-variant helper; executed only in the isolated build VM."""
import json
import os
from pathlib import Path
import shutil
import stat


def prune(cache):
    cache = Path(cache)
    if not cache.is_absolute() or cache.name != 'pip' or cache.parent.name != '.cache':
        raise ValueError('fixed pip cache path required')
    for parent in (cache, *cache.parents):
        if not stat.S_ISDIR(parent.lstat().st_mode):
            raise ValueError('cache ancestors must be real directories')
    if not shutil.rmtree.avoids_symlink_attacks:
        raise ValueError('descriptor-based cache removal required')
    allowed = {'http', 'http-v2', 'wheels', 'selfcheck', 'CACHEDIR.TAG'}
    if set(p.name for p in cache.iterdir()) - allowed:
        raise ValueError('unrecognized pip cache contents')
    identity = cache.stat()
    entries, files, logical_bytes = 0, 0, 0
    for directory, names, filenames in os.walk(cache, followlinks=False):
        for name in [*names, *filenames]:
            entries += 1
            if entries > 50000:
                raise ValueError('cache entry limit exceeded')
            meta = (Path(directory) / name).lstat()
            if not (stat.S_ISREG(meta.st_mode) or stat.S_ISDIR(meta.st_mode)):
                raise ValueError('only regular cache files and directories supported')
            if stat.S_ISREG(meta.st_mode):
                files += 1
                logical_bytes += meta.st_size
    current = cache.stat()
    if (identity.st_dev, identity.st_ino) != (current.st_dev, current.st_ino):
        raise ValueError('cache identity changed')
    shutil.rmtree(cache)
    return {'repair': 'pip-download-cache', 'path': str(cache),
            'removed_files': files, 'removed_logical_bytes': logical_bytes,
            'removed': not cache.exists()}


if __name__ == '__main__':
    if os.geteuid() != 0:
        raise SystemExit('root build guest required')
    print(json.dumps(prune('/root/.cache/pip')))
