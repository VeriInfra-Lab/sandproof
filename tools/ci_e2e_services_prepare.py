"""Uninstalled offline center/host preparation helper; no runtime pass claim.

Not imported by the active preparation/supervision entry. The atomic production
cutover must replace the existing preparation entry and delete its former
implementation; this helper is neither a runtime mode nor a pass attestation.
"""
import argparse
import json
import os
from pathlib import Path
import selectors
import subprocess
import time

from tools import ci_e2e_artifacts as artifacts
from tools.ci_e2e_supervisor import git
from tools.check_kvm_host import unique_keys
from tools.system_acceptance import json_file, private_path


def build_environment(config):
    """Reuse existing private-path protection; no implicit online cache fill.

    Do not inherit the supervisor environment: neither App credentials nor
    incidental user proxy/configuration can enter candidate build processes.
    The operator prepares the fixed public caches in a separate reviewed step.
    """
    caches = {name: str(private_path(config[name], directory=True))
              for name in ('cargo_home', 'uv_cache', 'rustup_home', 'cargo_target')}
    tools = str(private_path(config['toolchain_bin'], directory=True))
    return dict(PATH=tools + ':/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                CARGO_BUILD_JOBS='2', CARGO_NET_OFFLINE='true', UV_OFFLINE='true',
                CARGO_TARGET_DIR=caches['cargo_target'], CARGO_HOME=caches['cargo_home'],
                UV_CACHE_DIR=caches['uv_cache'], RUSTUP_HOME=caches['rustup_home'],
                GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')


def service_paths(config):
    """Only exact output names in this run's approved independent target."""
    target = private_path(config['cargo_target'], directory=True) / 'release'
    paths = {role: config[role] for role in artifacts.BINARIES}
    if (any(paths[role] != str(target / name) for role, name in artifacts.BINARIES.items())
            or config['window_helper'] != str(target / 'sandproof-window-reducer')):
        raise ValueError('service outputs must belong to this exact candidate build target')
    return paths


def contract_versions(binary, env):
    """Bounded metadata from the reviewed child; no configuration or secrets."""
    process = subprocess.Popen([binary, 'contract-versions'], env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL)
    result = bytearray()
    deadline = time.monotonic() + 5
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise TimeoutError('candidate contract metadata deadline')
                data = os.read(process.stdout.fileno(), 1025 - len(result))
                if not data:
                    break
                result.extend(data)
                if len(result) > 1024:
                    raise ValueError('candidate contract metadata exceeds its fixed size')
        if process.wait(timeout=max(.001, deadline - time.monotonic())) != 0:
            raise ValueError('candidate contract metadata command failed')
        return json.loads(result, object_pairs_hook=unique_keys)
    finally:
        process.stdout.close()
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)


def prepare(config):
    checkout = private_path(config['checkout'], directory=True)
    if git('rev-parse', 'HEAD', cwd=checkout) != config['candidate'] or git('status', '--porcelain', cwd=checkout):
        raise ValueError('candidate checkout changed')
    env = build_environment(config)
    paths = service_paths(config)
    manifest_path = Path(config['candidate_manifest'])
    private_path(manifest_path.parent, directory=True)
    if manifest_path.exists() or manifest_path.is_symlink():
        raise ValueError('candidate artifact receipt already exists; observe original preparation')
    subprocess.run([config['uv'], 'sync', '--locked', '--offline', '--no-python-downloads',
                    '--no-managed-python', '--no-config', '--project', str(checkout / 'benchmarks')],
                   check=True, env=env, timeout=900)
    for package, binary in (('sandproof-center-service', 'sandproof-center'),
                            ('sandproof-server', 'sandproof-host'),
                            ('sandproof-controller-service', 'sandproof-window-reducer')):
        subprocess.run([config['cargo'], 'build', '--release', '--locked', '--offline',
                        '-p', package, '--bin', binary], cwd=checkout, env=env, check=True, timeout=900)
    # Publish no fake runtime result. The original system harness owns runtime
    # provenance, binary fingerprints, task inputs and actual host admission.
    # These format numbers have one Rust source, not a separately maintained
    # Python copy. This metadata command never initializes or serves anything.
    versions = contract_versions(paths['center'], env)
    if (git('rev-parse', 'HEAD', cwd=checkout) != config['candidate']
            or git('status', '--porcelain', cwd=checkout)):
        raise ValueError('candidate modified during preparation')
    return artifacts.publish(manifest_path, candidate=config['candidate'], paths=paths,
                             versions=versions)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    prepare(json_file(private_path(args.config)))
