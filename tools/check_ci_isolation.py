#!/usr/bin/env python3
"""Trusted pre-job probe installed by the host, never supplied by a PR."""
import grp
import os
from pathlib import Path
import socket
import subprocess
import urllib.error
import urllib.request


def main():
    if os.geteuid() == 0:
        raise RuntimeError('job must not run as root')
    groups = {grp.getgrgid(value).gr_name for value in os.getgroups()}
    if groups & {'sudo', 'docker', 'kvm', 'lxd', 'libvirt'}:
        raise RuntimeError('job has a privileged group')
    if subprocess.run(['sudo', '-n', 'true'], stdout=subprocess.DEVNULL,
                      stderr=subprocess.DEVNULL, check=False).returncode == 0:
        raise RuntimeError('job can elevate to host/guest administrator')
    for forbidden in ('/dev/kvm', '/home/ning', '/var/lib/sandproof-ci', '/var/lib/sandproof'):
        if Path(forbidden).exists():
            raise RuntimeError('unexpected host device or shared directory')
    try:
        with socket.create_connection(('10.0.2.2', 22), timeout=2):
            raise RuntimeError('host SSH is reachable')
    except OSError:
        pass
    proxy = urllib.request.ProxyHandler({'http': 'http://10.0.2.100:3128',
                                        'https': 'http://10.0.2.100:3128'})
    # This probe deliberately tests proxy rejection even for loopback names.
    previous = {key: os.environ.pop(key, None) for key in ('no_proxy', 'NO_PROXY')}
    try:
        opener = urllib.request.build_opener(proxy)
        for target in ('http://127.0.0.1/', 'http://169.254.169.254/', 'https://example.com/'):
            try:
                opener.open(target, timeout=10)
            except urllib.error.HTTPError as error:
                if error.code != 403:
                    raise RuntimeError('egress rejection returned an unexpected status') from error
            except urllib.error.URLError as error:
                # CONNECT errors are represented as a wrapped tunnel error.
                if '403 Forbidden' not in str(error.reason):
                    raise RuntimeError('egress refusal was not established') from error
            else:
                raise RuntimeError('restricted egress target was allowed')
    finally:
        # Importing/testing the probe must not reroute later loopback tests.
        for key, value in previous.items():
            if value is not None:
                os.environ[key] = value
    print('SANDPROOF_CI_ISOLATION_OK', flush=True)


if __name__ == '__main__':
    main()
