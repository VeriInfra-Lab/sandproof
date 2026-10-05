"""Candidate preparation in an independent trusted systemd unit; no App key."""
import argparse
import os
from pathlib import Path
import subprocess

from tools.benchmark_ci import private_path
from tools.ci_e2e_supervisor import git
from tools.system_acceptance import json_file


def prepare(config):
    checkout = private_path(config['checkout'], directory=True)
    if git('rev-parse', 'HEAD', cwd=checkout) != config['candidate'] or git('status', '--porcelain', cwd=checkout):
        raise ValueError('candidate checkout changed')
    env = dict(os.environ, HTTPS_PROXY=config['proxy'], HTTP_PROXY=config['proxy'],
               https_proxy=config['proxy'], http_proxy=config['proxy'], NO_PROXY='localhost,127.0.0.1',
               CARGO_BUILD_JOBS='2', CARGO_TARGET_DIR=config['cargo_target'],
               PATH=config['toolchain_bin'] + ':/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin')
    subprocess.run([config['uv'], 'sync', '--locked', '--project', str(checkout / 'benchmarks')],
                   check=True, env=env, timeout=900)
    subprocess.run([config['cargo'], 'build', '--release', '--locked', '-p', 'sandproof-server', '--bin', 'sandproof-server'],
                   cwd=checkout, env=env, check=True, timeout=900)
    subprocess.run([config['cargo'], 'build', '--release', '--locked', '-p', 'sandproof-controller-service',
                    '--bin', 'sandproof-window-reducer'], cwd=checkout, env=env, check=True, timeout=900)
    # Publish no fake runtime result. The original system harness owns runtime
    # provenance, binary fingerprints, task inputs and actual host admission.
    if git('status', '--porcelain', cwd=checkout):
        raise ValueError('candidate modified during preparation')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    prepare(json_file(private_path(args.config)))
