"""Bounded parallel transport for one public archive; downloaded bytes are never executed."""
import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import tempfile
import time
import urllib.request
from pathlib import Path

CHUNK_BYTES = 8 * 1024**2


def fetch(url, destination, size, sha256):
    if (not url.startswith('https://') or not 0 < size <= 1024**3
            or not re.fullmatch('[0-9a-f]{64}', sha256)):
        raise ValueError('expected HTTPS, bounded size and a fixed SHA-256')
    destination = Path(destination)
    if destination.exists() or destination.is_symlink():
        raise FileExistsError('destination must not already exist')
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='.sandproof-fetch-', dir=destination.parent) as temp:
        root = Path(temp)
        ranges = [(start, min(start + CHUNK_BYTES, size) - 1)
                  for start in range(0, size, CHUNK_BYTES)]

        def part(bounds):
            start, end = bounds
            path = root / str(start)
            request = urllib.request.Request(url, headers={
                'Range': f'bytes={start}-{end}', 'Accept-Encoding': 'identity'})
            with urllib.request.urlopen(request, timeout=30) as response, path.open('xb') as out:
                if (response.status != 206
                        or response.headers.get('Content-Range') != f'bytes {start}-{end}/{size}'):
                    raise ValueError('server did not return the exact requested range')
                count = 0
                while chunk := response.read(min(1024**2, end - start + 2 - count)):
                    count += len(chunk)
                    if count > end - start + 1:
                        raise ValueError('oversized range')
                    out.write(chunk)
                if count != end - start + 1:
                    raise ValueError('truncated range')
            return path

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(part, ranges))
        digest = hashlib.sha256()
        output = root / 'complete'
        with output.open('xb') as out:
            for start, _ in ranges:
                with (root / str(start)).open('rb') as stream:
                    while chunk := stream.read(1024**2):
                        out.write(chunk)
                        digest.update(chunk)
                (root / str(start)).unlink()
            out.flush()
            os.fsync(out.fileno())
        if digest.hexdigest() != sha256:
            raise ValueError('SHA-256 mismatch')
        os.link(output, destination)
        return {'url': url, 'size': size, 'sha256': digest.hexdigest(),
                'seconds': round(time.monotonic() - started, 3), 'parallel_connections': 4}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--size', type=int, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--proof', type=Path, required=True)
    args = parser.parse_args()
    result = fetch(args.url, args.output, args.size, args.sha256)
    with args.proof.open('x') as stream:
        json.dump(result, stream, indent=2)
        stream.write('\n')
    print(json.dumps(result), flush=True)
