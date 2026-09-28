"""Run #12 get/list and #9 lifetime probes on an isolated real KVM server.

This is a privileged, operator-selected acceptance harness. It creates only a
new private root and preserves it on failure for exact-resource inspection.
"""

import argparse
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import secrets
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request

from server_process_crash import free_loopback_port, start_server, stop_server, wait_ready


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def exact_processes(prefix):
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / 'cmdline').read_bytes()
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        if prefix.encode() in command:
            yield int(entry.name)


def run(args):
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise RuntimeError('requires root on an isolated Linux/KVM host')
    assert importlib.metadata.version('e2b') == '2.51.0'
    assert args.max_total > args.max_timeout >= 300
    parent = args.scratch_parent.resolve(strict=True)
    disk_bytes = args.rootfs.stat().st_size
    if shutil.disk_usage(parent).free < disk_bytes * 5 + 256 * 1024 * 1024:
        raise RuntimeError('insufficient private disk for four VM reservations and cleanup')
    root = Path(tempfile.mkdtemp(prefix='sp12-', dir=parent))
    namespace = secrets.token_hex(8)
    longest_socket = root / 'jails' / args.firecracker.name / f'vm-{namespace}-1000' / 'root/api.socket'
    if len(os.fsencode(longest_socket)) >= 108:
        root.rmdir()
        raise RuntimeError('private test root exceeds Firecracker socket limit')
    key, foreign_key = secrets.token_hex(32), secrets.token_hex(32)
    port = free_loopback_port()
    for name in ('db', 'state', 'jails'):
        (root / name).mkdir(mode=0o700)
    binary = root / 'sandproof-server'
    kernel = root / 'vmlinux'
    rootfs = root / 'rootfs.ext4'
    for source, target in ((args.server, binary), (args.kernel, kernel), (args.rootfs, rootfs)):
        shutil.copyfile(source, target)
    binary.chmod(0o700)
    kernel.chmod(0o644)
    rootfs.chmod(0o644)
    base_digest = digest(rootfs)
    config = root / 'config.json'
    config.write_text(json.dumps({
        'namespace': namespace,
        'database': str(root / 'db/controller.db'),
        'node_state_dir': str(root / 'state'),
        'jailer': str(args.jailer.resolve(strict=True)),
        'firecracker': str(args.firecracker.resolve(strict=True)),
        'chroot_base': str(root / 'jails'),
        'kernel': str(kernel),
        'rootfs': str(rootfs),
        'uid_base': args.uid_base,
        'max_vm_id': 1000,
        'vcpu_count': 1,
        'memory_mib': 256,
        'file_policy_digest': 1,
        'bind': f'127.0.0.1:{port}',
        'domain': 'example.invalid',
        'credentials': [
            {'tenant': 1, 'secret': key},
            {'tenant': 2, 'secret': foreign_key},
        ],
        'max_active_global': 4,
        'max_active_per_tenant': 3,
        'max_disk_bytes_global': disk_bytes * 5,
        'max_disk_bytes_per_tenant': disk_bytes * 4,
        'cleanup_reserve_bytes': disk_bytes,
        'max_timeout_secs': args.max_timeout,
        'max_total_lifetime_secs': args.max_total,
        'default_timeout_secs': 300,
        'expiry_scan_seconds': args.scanner_seconds,
    }))
    config.chmod(0o600)
    base_url = f'http://127.0.0.1:{port}'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    process = None
    completed = False
    try:
        process = start_server(binary, config, root / 'server.log', None)
        wait_ready(opener, base_url, key, process)
        env = dict(os.environ, SANDPROOF_TEST_API_KEY=key,
                   SANDPROOF_TEST_FOREIGN_API_KEY=foreign_key)
        scripts = (
            ('sdk_list_contract.py', []),
            ('sdk_lifetime_target.py', [
                '--max-timeout', str(args.max_timeout),
                '--max-total', str(args.max_total),
                '--scanner-seconds', str(args.scanner_seconds),
            ]),
        )
        for script, extra in scripts:
            subprocess.run([sys.executable, str(Path(__file__).with_name(script)),
                            '--api-url', base_url, '--sandbox-url', base_url, *extra],
                           env=env, check=True, timeout=600)
        with sqlite3.connect(root / 'db/controller.db') as database:
            states = [json.loads(row[0]) for row in database.execute('SELECT state_blob FROM sandboxes')]
        assert states and all(state['phase'] == 'Destroyed' for state in states), 'live durable sandbox'
        prefix = f'vm-{namespace}-'
        assert not list(exact_processes(prefix)), 'test VMM process remains'
        assert not any(path.name.startswith(prefix) for path in Path('/run/netns').iterdir()), 'test netns remains'
        jail_root = root / 'jails' / args.firecracker.name
        assert not jail_root.exists() or not any(path.name.startswith(prefix) for path in jail_root.iterdir()), 'test jail remains'
        claims = root / 'jails/.claims'
        assert not claims.exists() or not any(path.name.startswith(prefix) for path in claims.iterdir()), 'test claim remains'
        assert digest(rootfs) == base_digest, 'fixed base changed'
        completed = True
        print(json.dumps({'server_sha256': digest(binary), 'base_sha256': base_digest,
                          'sdk_version': '2.51.0', 'durable_destroyed': len(states),
                          'exact_process_netns_jail_claim': 'none', 'fixed_base': 'unchanged'}))
    finally:
        stop_server(process)
        if completed:
            shutil.rmtree(root)
        else:
            print(f'Exact failed-run artifacts retained: {root}', file=sys.stderr)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('server', 'jailer', 'firecracker', 'kernel', 'rootfs', 'scratch-parent'):
        parser.add_argument(f'--{name}', required=True, type=Path)
    parser.add_argument('--uid-base', required=True, type=int)
    parser.add_argument('--max-timeout', type=int, default=600)
    parser.add_argument('--max-total', type=int, default=1200)
    parser.add_argument('--scanner-seconds', type=int, default=1)
    run(parser.parse_args())
