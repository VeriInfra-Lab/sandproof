"""Offline Rust guest bundle from one exact Git commit; no VM or installation.

Run as an authorized builder in an isolated, disposable Linux x86_64 environment.
Cargo/build scripts execute approved source. External build-image/toolchain/cache
trust is required; this is neither an adversarial build sandbox nor attestation.
Failures retain only this newly created run directory for inspection; no retry.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import time
import tomllib

from tools import guest_artifacts as artifacts
from tools.build_base_guest import verified_copy
from tools.check_kvm_host import absolute_path, require_keys, unique_keys
from tools.ci_e2e_prepare import bounded_json_command
from tools.system_acceptance import json_file, private_path, write_once
from tools.template_fetch import capture
from tools.template_inputs import extract_archive
from tools.template_worker import Commands

ROOT = Path(__file__).resolve().parents[1]
CONFIG_KEYS = {'candidate', 'checkout', 'output', 'cargo_home', 'toolchain_bin', 'timeout_seconds', 'max_log_bytes'}


def configuration(config):
    require_keys(config, CONFIG_KEYS, 'guest preparation configuration')
    artifacts.hex_digest(config['candidate'], 40)
    for key, maximum in (('timeout_seconds', 3600), ('max_log_bytes', 16 * 1024**2)):
        if type(config[key]) is not int or not 0 < config[key] <= maximum:
            raise ValueError('explicit finite build budget required')
    paths = {key: private_path(absolute_path(config[key]), directory=True)
             for key in ('checkout', 'cargo_home', 'toolchain_bin')}
    output = absolute_path(config['output'])
    private_path(output.parent, directory=True)
    if output.exists() or output.is_symlink():
        raise ValueError('new guest output required; observe the original build')
    if any(output == path or path in output.parents or output in path.parents for path in paths.values()):
        raise ValueError('build output must be separate from checkout, cache and compiler')
    return paths, output


def capture_bytes(arguments, destination, limit, commands):
    capture(arguments, destination, limit, commands)
    return artifacts.fingerprint(destination, limit, content=True)[1]


def archive_source(config, paths, output, commands):
    git = ['/usr/bin/git', '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false', '-c', 'tar.umask=0022',
           '-c', 'credential.helper=', '-C', str(paths['checkout'])]
    candidate = config['candidate']
    commit = capture_bytes([*git, 'rev-parse', '--verify', candidate + '^{commit}'],
                           output / 'commit.txt', 128, commands).decode().strip()
    if commit != candidate:
        raise ValueError('candidate must be an exact commit, not a tag/ref')
    tree = capture_bytes([*git, 'rev-parse', '--verify', candidate + '^{tree}'],
                         output / 'tree.txt', 128, commands).decode().strip()
    artifacts.hex_digest(tree, 40)
    archive = output / 'source.tar'
    capture([*git, 'archive', '--format=tar', candidate], archive, artifacts.SOURCE_LIMIT, commands)
    source = output / 'source'
    extract_archive(archive, source, max_bytes=artifacts.SOURCE_LIMIT, max_entries=10_000)
    archive.chmod(0o400)
    return source, tree


def source_inputs(source):
    """Use the original lock, and refuse an installed recipe from another tree."""
    recipe = hashlib.sha256()
    candidates = sorted((source / 'tools').rglob('*.py'))
    if not candidates:
        raise ValueError('candidate has no preparation recipe')
    if ({p.relative_to(source) for p in candidates}
            != {p.relative_to(ROOT) for p in (ROOT / 'tools').rglob('*.py')}):
        raise ValueError('running preparation recipe file set differs from candidate')
    for path in candidates:
        name = path.relative_to(source).as_posix()
        observed = artifacts.fingerprint(path, 4 * 1024**2, allow_empty=True)
        if artifacts.fingerprint(ROOT / name, 4 * 1024**2, allow_empty=True) != observed:
            raise ValueError('running preparation recipe differs from candidate')
        recipe.update(name.encode() + b'\0' + observed['sha256'].encode() + b'\n')
    # Do not permit an old tree omitting the two current entry modules.
    if not {'prepare_guest.py', 'guest_artifacts.py'} <= {p.name for p in candidates}:
        raise ValueError('candidate lacks the guest preparation recipe')
    input_info, raw = artifacts.fingerprint(source / 'references/v1-baseline.json', 1024**2, content=True)
    baseline = json.loads(raw, object_pairs_hook=unique_keys)['code_baseline']
    artifacts.versions(baseline['rust_guest'])
    lock = artifacts.fingerprint(source / 'Cargo.lock', 4 * 1024**2)
    if lock['sha256'] != baseline['cargo_lock_sha256']:
        raise ValueError('Cargo lock differs from source input lock')
    rust = tomllib.loads(artifacts.fingerprint(source / 'rust-toolchain.toml', 4096, content=True)[1].decode())
    if rust['toolchain']['channel'] != baseline['rust_toolchain']:
        raise ValueError('Rust toolchain differs from source input lock')
    return baseline, dict(cargo_lock_sha256=lock['sha256'], input_lock_sha256=input_info['sha256'],
                         recipe_sha256=recipe.hexdigest())


def build_environment(paths, target):
    toolchain = paths['toolchain_bin']
    return dict(PATH=str(toolchain) + ':/usr/bin:/bin:/usr/sbin:/sbin', LC_ALL='C',
                CARGO_HOME=str(paths['cargo_home']), CARGO_TARGET_DIR=str(target),
                CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='2', CARGO_INCREMENTAL='0',
                CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS='false',
                RUSTC=str(toolchain / 'rustc'), GIT_TERMINAL_PROMPT='0',
                GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL='/dev/null')


def build(source, output, paths, baseline, commands):
    target = output / 'target'
    target.mkdir(mode=0o700)
    env = build_environment(paths, target)
    compiler = {name: paths['toolchain_bin'] / name for name in ('rustc', 'cargo')}
    facts = {name: artifacts.fingerprint(path, artifacts.BINARY_LIMIT, executable=True)
             for name, path in compiler.items()}
    if facts['rustc']['sha256'] == facts['cargo']['sha256']:
        raise ValueError('independent native Rust/Cargo binaries required, not rustup shims')
    for name, flag in (('rustc', '-vV'), ('cargo', '--version')):
        facts[name + '_version'] = capture_bytes([str(compiler[name]), flag], output / (name + '.txt'),
                                                 4096, commands).decode().strip()
    version = baseline['rust_toolchain']
    if ('release: ' + version not in facts['rustc_version'].splitlines()
            or 'host: ' + artifacts.TARGET not in facts['rustc_version'].splitlines()
            or not facts['cargo_version'].startswith('cargo ' + version + ' ')):
        raise ValueError('fixed Linux Rust/Cargo toolchain required')
    commands.run([str(compiler['cargo']), 'build', '--release', '--locked', '--offline',
                  '--target', artifacts.TARGET, '-p', 'sandproof-guest-service',
                  '-p', 'sandproof-guest-runtime', '--bins'], env=env, cwd=source)
    reports, binaries = {}, {}
    for role, name in artifacts.BINARIES.items():
        binary = target / artifacts.TARGET / 'release' / name
        # Cargo may link its top-level output to the same fresh target's deps
        # artifact. Only this private build input may have aliases; the copied
        # bundle is independently checked as a single-link file by load().
        before = artifacts.fingerprint(binary, artifacts.BINARY_LIMIT, executable=True, single_link=False)
        remaining = commands.deadline - time.monotonic()
        reports[role] = bounded_json_command([str(binary), '--version'], env,
                                             timeout=min(5, remaining), max_bytes=4096)
        if artifacts.fingerprint(binary, artifacts.BINARY_LIMIT, executable=True, single_link=False) != before:
            raise ValueError('built guest binary changed during version query')
        verified_copy(binary, output / name, before)
        (output / name).chmod(0o500)
        with (output / name).open('rb') as saved:
            os.fsync(saved.fileno())
        binaries[role] = before
    artifacts.check_reports(baseline['rust_guest'], reports['service'], reports['worker'])
    for name, path in compiler.items():
        if artifacts.fingerprint(path, artifacts.BINARY_LIMIT, executable=True) != facts[name]:
            raise ValueError('compiler changed during guest build')
    return dict(target=artifacts.TARGET, **facts), binaries


def prepare(config):
    paths, output = configuration(config)
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise ValueError('guest release construction requires Linux x86_64')
    output.mkdir(mode=0o700)  # exclusive, never reuse a failed/partial run
    with (output / 'build.log').open('xb') as log:
        commands = Commands(time.monotonic() + config['timeout_seconds'], log, config['max_log_bytes'])
        source, tree = archive_source(config, paths, output, commands)
        baseline, source_facts = source_inputs(source)
        if time.monotonic() >= commands.deadline:
            raise TimeoutError('guest preparation deadline')
        build_facts, binaries = build(source, output, paths, baseline, commands)
        # Archive/read work is bounded by byte/entry limits. Already entered
        # filesystem calls cannot be interrupted by this deadline.
        if time.monotonic() >= commands.deadline:
            raise TimeoutError('guest preparation deadline')
        manifest = artifacts.validate(dict(schema_version=1, kind='rust-guest-v1',
            source=dict(commit=config['candidate'], tree=tree,
                        archive=artifacts.fingerprint(output / 'source.tar', artifacts.SOURCE_LIMIT), **source_facts),
            versions=baseline['rust_guest'], build=build_facts, artifacts=binaries))
        with (output / 'source.tar').open('rb') as archive:
            os.fsync(archive.fileno())
        if time.monotonic() >= commands.deadline:
            raise TimeoutError('guest preparation deadline')
        write_once(output / 'manifest.json', manifest)
    pin = artifacts.fingerprint(output / 'manifest.json', artifacts.MANIFEST_LIMIT)['sha256']
    artifacts.load(output, pin)
    if time.monotonic() >= commands.deadline:
        raise TimeoutError('guest preparation deadline; inspect possibly published manifest')
    return {'manifest_sha256': pin, 'worker': artifacts.worker_configuration(manifest)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    arguments = parser.parse_args()
    print(json.dumps(prepare(json_file(private_path(arguments.config))), sort_keys=True))
