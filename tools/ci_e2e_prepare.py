"""Offline dual-service candidate preparation; no runtime or pass attestation.

One approved checkout produces the center, host and existing acceptance helpers.
No inherited credentials, network fallback or old single-server output is used.
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
    """Reviewed fixed caches only; never inherit the supervisor's App key."""
    caches = {name: str(private_path(config[name], directory=True))
              for name in ('cargo_home', 'uv_cache', 'rustup_home', 'cargo_target')}
    tools = str(private_path(config['toolchain_bin'], directory=True))
    return dict(PATH=tools + ':/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                CARGO_BUILD_JOBS='2', CARGO_NET_OFFLINE='true', UV_OFFLINE='true',
                # One controlled test build for DK and Harbor alike. Ordinary
                # deployment releases do not include these debug-only faults.
                CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS='true',
                CARGO_TARGET_DIR=caches['cargo_target'], CARGO_HOME=caches['cargo_home'],
                UV_CACHE_DIR=caches['uv_cache'], RUSTUP_HOME=caches['rustup_home'],
                GIT_TERMINAL_PROMPT='0', GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')


def service_paths(config):
    """Only exact output names in this approved candidate's build target."""
    target = private_path(config['cargo_target'], directory=True) / 'release'
    paths = {role: config[role] for role in artifacts.BINARIES}
    if ('server' in config
            or any(paths[role] != str(target / name) for role, name in artifacts.BINARIES.items())
            or config['window_helper'] != str(target / 'sandproof-window-reducer')
            or config['stale_probe'] != str(target / 'examples/stale_host_actions')):
        raise ValueError('service outputs must belong to this exact candidate build target')
    return paths


def bounded_json_command(argv, env, *, timeout, max_bytes):
    """One bounded, deadline-controlled JSON reader for metadata/read-only audit.

    Callers own authorization and argv. This never executes a shell or inherits
    credentials, and malformed/error/unknown output cannot become an empty view.
    """
    if (type(timeout) not in (int, float) or not 0 < timeout <= 60
            or type(max_bytes) is not int or not 0 < max_bytes <= 16 * 1024**2):
        raise ValueError('explicit finite command output and deadline required')
    process = subprocess.Popen(argv, env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    result = bytearray()
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise TimeoutError('candidate contract metadata deadline')
                data = os.read(process.stdout.fileno(), min(65536, max_bytes + 1 - len(result)))
                if not data:
                    break
                result.extend(data)
                if len(result) > max_bytes:
                    raise ValueError('candidate contract metadata exceeds its fixed size')
        if process.wait(timeout=max(.001, deadline - time.monotonic())) != 0:
            raise ValueError('candidate contract metadata command failed')
        return json.loads(result, object_pairs_hook=unique_keys)
    finally:
        process.stdout.close()
        if process.poll() is None:
            process.kill()
            process.wait(timeout=5)


def contract_versions(binary, env):
    """Bounded candidate metadata; no protected config or filesystem effects."""
    return bounded_json_command([binary, 'contract-versions'], env, timeout=5, max_bytes=1024)


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
    for package, binary in (('sandproof-center', 'sandproof-center'),
                            ('sandproof-server', 'sandproof-host'),
                            ('sandproof-controller-service', 'sandproof-window-reducer')):
        subprocess.run([config['cargo'], 'build', '--release', '--locked', '--offline',
                        '-p', package, '--bin', binary], cwd=checkout, env=env, check=True, timeout=900)
    subprocess.run([config['cargo'], 'build', '--release', '--locked', '--offline',
                    '-p', 'sandproof-node', '--example', 'stale_host_actions'],
                   cwd=checkout, env=env, check=True, timeout=900)
    # Helpers must exist in this same candidate target, not an external tool.
    for name in ('window_helper', 'stale_probe'):
        helper = artifacts.trusted_path(Path(config[name]))
        if not helper.is_file() or not os.access(helper, os.X_OK):
            raise ValueError('same-candidate acceptance helper is not executable')
    # Publish no fake runtime result. The original system harness owns runtime
    # provenance, binary fingerprints, task inputs and actual host admission.
    # One shared Rust source defines versions, but BOTH actual binaries must
    # agree. Neither a partial build nor a mixed-format host is sealed.
    versions = contract_versions(paths['center'], env)
    if contract_versions(paths['host'], env) != versions:
        raise ValueError('center and host contract versions differ')
    if (git('rev-parse', 'HEAD', cwd=checkout) != config['candidate']
            or git('status', '--porcelain', cwd=checkout)):
        raise ValueError('candidate modified during preparation')
    from tools.envd_acceptance import prepare as prepare_envd
    prepare_envd(config)
    return artifacts.publish(manifest_path, candidate=config['candidate'], paths=paths,
                             versions=versions)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    prepare(json_file(private_path(args.config)))
