#!/usr/bin/env python3
"""Restricted, approved dependency inputs for preparing the ordinary CI image.

The host packages reviewed Git blobs, never a checkout or existing cache. The
guest runs the fixed recipe as runner, retaining only public dependency caches.
This tool's READY line is diagnostic; the root bootstrap owns image approval.
"""
import argparse
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import pwd
import re
import stat
import subprocess
import sys
import tempfile
import tomllib
from urllib.parse import parse_qs, urlsplit, urlunsplit
try:
    from . import ci_redis
except ImportError:
    import ci_redis


MEMBERS = ('controller-core', 'sandbox-protocol', 'controller-service',
           'sandbox-runtime', 'sandbox-node', 'sandbox-api', 'sandbox-server', 'sandbox-data-transport',
           'sandbox-internal', 'sandbox-center-store', 'sandbox-center-core', 'sandbox-host-service',
           'sandbox-center-service', 'sandbox-center', 'sandbox-guest-core', 'sandbox-guest-runtime',
           'sandbox-guest-service', 'sandbox-stream-codec')
PROJECTS = ('tests/compat', 'benchmarks')
INPUT_PATHS = frozenset(('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
    'tools/ci_git.py', 'tools/ci_redis.py', *(member + '/Cargo.toml' for member in MEMBERS),
    *(project + '/' + name for project in PROJECTS for name in ('pyproject.toml', 'uv.lock'))))
PLATFORM = {'system': 'Linux', 'machine': 'x86_64', 'python': '3.12',
            'rust': '1.97.0', 'uv': '0.11.31', 'target': 'x86_64-unknown-linux-gnu'}
UV_ARGUMENTS = ['sync', '--locked', '--python', '/usr/bin/python3',
                '--no-python-downloads', '--no-managed-python', '--no-config']
CARGO_ARGUMENTS = ['fetch', '--locked', '--target', PLATFORM['target']]
RECIPE = {'version': 2, 'projects': list(PROJECTS), 'uv': UV_ARGUMENTS,
          'cargo': CARGO_ARGUMENTS, 'offline_rebuild': True,
          'cargo_sources': 'generated empty metadata targets; never compiled',
          'provenance': 'version, original Git URL, commit and subdirectory',
          'redis': ci_redis.PIN}
PUBLIC_GIT = {'e2b': ('https://github.com/e2b-dev/E2B.git', 'packages/python-sdk'),
              'harbor': ('https://github.com/laude-institute/harbor.git', None)}
MAX_FILE_BYTES = 4 * 1024 ** 2
MAX_PAYLOAD_BYTES = 16 * 1024 ** 2
MAX_INPUT_BYTES = 24 * 1024 ** 2


def canonical_json(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def recipe_sha256():
    return sha256(Path(__file__).read_bytes())


def read_regular(path, limit):
    # O_NOFOLLOW also rejects a last-component link changed after lstat.
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError('dependency input must be a regular file')
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), 'rb') as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size > limit:
            raise ValueError('dependency input must be a bounded regular file')
        data = source.read(limit + 1)
    if len(data) > limit:
        raise ValueError('dependency input exceeds size limit')
    return data


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate dependency input field')
        result[key] = value
    return result


def exact_fields(value, fields):
    if not isinstance(value, dict) or set(value) != set(fields):
        raise ValueError('unexpected or missing dependency input fields')


def python_sources(project, lock):
    uv = project.get('tool', {}).get('uv', {})
    expected_names = {'e2b', 'harbor'} if project['project']['name'] == 'sandproof-harbor-benchmarks' else {'e2b'}
    exact_fields(uv, ('package', 'sources'))
    if uv['package'] is not False or set(uv['sources']) != expected_names or 'build-system' in project:
        raise ValueError('only the non-package public CI Python projects are accepted')
    if any(not isinstance(item, str) or '@' in item for item in project['project']['dependencies']):
        raise ValueError('direct Python dependency URLs are not accepted')
    sources = {}
    for name, source in uv['sources'].items():
        url, subdirectory = PUBLIC_GIT[name]
        fields = ('git', 'rev', 'subdirectory') if subdirectory else ('git', 'rev')
        exact_fields(source, fields)
        if (source['git'] != url or source.get('subdirectory') != subdirectory
                or not isinstance(source['rev'], str) or not re.fullmatch(r'[0-9a-f]{40}', source['rev'])):
            raise ValueError('Python source must retain an exact official Git revision')
        sources[name] = {'url': url, 'commit_id': source['rev'], 'subdirectory': subdirectory}
    found = set()
    virtual = 0
    for package in lock['package']:
        source = package['source']
        if source == {'registry': 'https://pypi.org/simple'}:
            for artifact in ([package['sdist']] if 'sdist' in package else []) + package.get('wheels', []):
                url = urlsplit(artifact['url'])
                if (url.scheme != 'https' or url.netloc != 'files.pythonhosted.org'
                        or url.query or url.fragment or not re.fullmatch(r'sha256:[0-9a-f]{64}', artifact['hash'])):
                    raise ValueError('Python artifact is not a hashed public PyPI download')
        elif source == {'virtual': '.'} and package['name'] == project['project']['name']:
            virtual += 1
        elif set(source) == {'git'} and package['name'] in sources:
            name = package['name']
            wanted = sources[name]
            url = urlsplit(source['git'])
            query = {'rev': [wanted['commit_id']]}
            if wanted['subdirectory']:
                query['subdirectory'] = [wanted['subdirectory']]
            if (name in found or urlunsplit((url.scheme, url.netloc, url.path, '', '')) != wanted['url']
                    or url.fragment != wanted['commit_id'] or parse_qs(url.query) != query):
                raise ValueError('Python Git lock differs from the original project source')
            wanted['version'] = package['version']
            found.add(name)
        else:
            raise ValueError('unsupported Python dependency source')
    if found != set(sources) or virtual != 1:
        raise ValueError('missing Python project or Git source in lock')
    return sources


def validate_sources(files):
    try:
        parsed = {path: tomllib.loads(data.decode()) for path, data in files.items() if path.endswith(('.toml', '.lock'))}
        if parsed['rust-toolchain.toml'] != {'toolchain': {'channel': PLATFORM['rust'],
                'profile': 'minimal', 'components': ['clippy', 'rustfmt']}}:
            raise ValueError('unexpected Rust toolchain')
        root = parsed['Cargo.toml']
        if (set(root['workspace']['members']) != set(MEMBERS)
                or root['workspace'].get('exclude') or root['workspace']['package']['rust-version'] != '1.97'):
            raise ValueError('unsupported Cargo workspace')
        # No alternate registries, Git transport, or paths outside this workspace.
        def check_cargo(value):
            if isinstance(value, dict):
                for key, item in value.items():
                    if key in ('git', 'registry', 'patch', 'replace'):
                        raise ValueError('unsupported Cargo dependency source')
                    if key == 'path' and item not in {'../' + member for member in MEMBERS}:
                        if not isinstance(item, str) or not re.fullmatch(r'src/[A-Za-z0-9_/-]+\.rs', item) or '..' in PurePosixPath(item).parts:
                            raise ValueError('Cargo path escapes restricted metadata workspace')
                    check_cargo(item)
            elif isinstance(value, list):
                for item in value:
                    check_cargo(item)
        for path, value in parsed.items():
            if path.endswith('Cargo.toml'):
                check_cargo(value)
                for target in value.get('test', []):
                    if (not isinstance(target, dict) or not isinstance(target.get('name'), str)
                            or not re.fullmatch(r'[A-Za-z0-9_-]+', target['name'])):
                        raise ValueError('unsupported Cargo test target name')
        for package in parsed['Cargo.lock']['package']:
            if 'source' in package and (package['source'] != 'registry+https://github.com/rust-lang/crates.io-index'
                    or not re.fullmatch(r'[0-9a-f]{64}', package.get('checksum', ''))):
                raise ValueError('Cargo lock requires public crates.io checksums')
        provenance = {}
        for project, name in zip(PROJECTS, ('sandproof-sdk-contracts', 'sandproof-harbor-benchmarks')):
            pyproject = parsed[project + '/pyproject.toml']
            if pyproject['project']['name'] != name:
                raise ValueError('unexpected CI Python project identity')
            provenance[project] = python_sources(pyproject, parsed[project + '/uv.lock'])
        return provenance
    except (KeyError, TypeError, AttributeError, UnicodeError) as error:
        raise ValueError('malformed dependency manifest or lock') from error


def validate_input(bundle):
    exact_fields(bundle, ('input_sha256', 'manifest', 'files', 'redis_archive'))
    manifest = bundle['manifest']
    exact_fields(manifest, ('schema', 'source_commit', 'platform', 'recipe', 'recipe_sha256', 'files'))
    if (type(manifest['schema']) is not int or manifest['schema'] != 1
            or not isinstance(manifest['source_commit'], str) or not re.fullmatch(r'[0-9a-f]{40}', manifest['source_commit'])
            or manifest['platform'] != PLATFORM or manifest['recipe'] != RECIPE
            or manifest['recipe_sha256'] != recipe_sha256()
            or bundle['input_sha256'] != sha256(canonical_json(manifest))):
        raise ValueError('dependency input identity or fixed recipe mismatch')
    exact_fields(bundle['files'], INPUT_PATHS)
    exact_fields(manifest['files'], INPUT_PATHS)
    files = {}
    total = 0
    for path in sorted(INPUT_PATHS):
        entry = manifest['files'][path]
        exact_fields(entry, ('sha256', 'size'))
        if type(entry['size']) is not int or not 0 < entry['size'] <= MAX_FILE_BYTES:
            raise ValueError('dependency file size is invalid')
        total += entry['size']
        encoded = bundle['files'][path]
        if total > MAX_PAYLOAD_BYTES or not isinstance(encoded, str) or len(encoded) > 4 * ((MAX_FILE_BYTES + 2) // 3):
            raise ValueError('dependency input exceeds payload limits')
        try:
            data = base64.b64decode(encoded, validate=True)
        except (binascii.Error, ValueError) as error:
            raise ValueError('invalid dependency file encoding') from error
        if len(data) != entry['size'] or sha256(data) != entry['sha256']:
            raise ValueError('dependency file content differs from manifest')
        files[path] = data
    validate_sources(files)
    for name in ('ci_git.py', 'ci_redis.py'):
        if files['tools/' + name] != Path(__file__).with_name(name).read_bytes():
            raise ValueError('adapter differs from the controlled CI tool')
    encoded = bundle['redis_archive']
    if not isinstance(encoded, str) or len(encoded) > 4 * ((ci_redis.ARCHIVE_LIMIT + 2) // 3):
        raise ValueError('Redis archive exceeds fixed input budget')
    try:
        archive = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError('invalid Redis archive encoding') from error
    if len(archive) > ci_redis.ARCHIVE_LIMIT or ci_redis.digest(archive) != ci_redis.PIN['archive_sha256']:
        raise ValueError('Redis archive differs from fixed official input')
    return files


def load_input(path):
    bundle = json.loads(read_regular(path, MAX_INPUT_BYTES), object_pairs_hook=unique_object)
    validate_input(bundle)
    return bundle


def prepare_input(checkout, approved_commit, output, redis_archive):
    if not re.fullmatch(r'[0-9a-f]{40}', approved_commit):
        raise ValueError('an approved complete Git commit is required')
    checkout = checkout.absolute()
    if checkout.is_symlink() or not checkout.is_dir():
        raise ValueError('reviewed checkout must be a real directory')
    def git(*arguments):
        return subprocess.run(['/usr/bin/git', '-C', str(checkout), *arguments], check=True, capture_output=True).stdout
    if git('rev-parse', 'HEAD').decode().strip() != approved_commit:
        raise ValueError('checkout HEAD differs from approved commit')
    files = {}
    for path in sorted(INPUT_PATHS):
        current = checkout
        for component in PurePosixPath(path).parts:
            current /= component
            if current.is_symlink():
                raise ValueError('linked dependency input is not accepted')
        data = read_regular(current, MAX_FILE_BYTES)
        # Git mode also excludes a symlink or non-blob in the approved tree.
        tree = git('ls-tree', approved_commit, '--', path).decode()
        if not tree.startswith(('100644 blob ', '100755 blob ')) or data != git('show', approved_commit + ':' + path):
            raise ValueError('dependency input is dirty or absent from approved tree')
        files[path] = data
    manifest = {'schema': 1, 'source_commit': approved_commit, 'platform': PLATFORM,
                'recipe': RECIPE, 'recipe_sha256': recipe_sha256(),
                'files': {path: {'size': len(data), 'sha256': sha256(data)} for path, data in files.items()}}
    archive = ci_redis.checked_archive(redis_archive)
    bundle = {'manifest': manifest, 'input_sha256': sha256(canonical_json(manifest)),
              'redis_archive': base64.b64encode(archive).decode(),
              'files': {path: base64.b64encode(data).decode() for path, data in files.items()}}
    validate_input(bundle)
    encoded = canonical_json(bundle) + b'\n'
    if len(encoded) > MAX_INPUT_BYTES:
        raise ValueError('dependency input exceeds encoded limit')
    # Exclusive creation never replaces an approved bundle or follows a link.
    with open(output, 'xb', opener=lambda p, f: os.open(p, f | os.O_NOFOLLOW, 0o600)) as destination:
        destination.write(encoded)
    return bundle


def write_workspace(directory, files):
    for path, data in files.items():
        destination = directory / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    for member in MEMBERS:
        # Cargo fetch parses targets, but never reads/compiles repository code.
        paths = {'src/lib.rs'}
        manifest = tomllib.loads(files[member + '/Cargo.toml'].decode())
        for target in manifest.get('bin', []):
            paths.add(target.get('path', 'src/main.rs'))
        for target in manifest.get('test', []):
            paths.add(target.get('path', 'tests/' + target['name'] + '.rs'))
        for path in paths:
            destination = directory / member / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text('// Dependency fetch metadata only.\n')
    adapter = directory / 'git-bin'
    adapter.mkdir()
    # Execute the root-owned installation, never the runner-writable copy.
    (adapter / 'git').symlink_to(Path(__file__).with_name('ci_git.py').absolute())
    return adapter


PROVENANCE_CHECK = '''import importlib.metadata as metadata, json, sys
for name, expected in json.loads(sys.argv[1]).items():
    installed = metadata.distribution(name)
    direct = json.loads(installed.read_text("direct_url.json") or "null")
    if (not isinstance(direct, dict) or installed.version != expected["version"]
            or direct.get("url") != expected["url"]
            or direct.get("subdirectory") != expected["subdirectory"]
            or direct.get("vcs_info", {}).get("vcs") != "git"
            or direct.get("vcs_info", {}).get("commit_id") != expected["commit_id"]):
        raise ValueError("Git installation provenance mismatch: " + name)
'''


def guest_environment():
    if (os.geteuid() == 0 or pwd.getpwuid(os.geteuid()).pw_name != 'runner'
            or platform.system() != PLATFORM['system'] or platform.machine() != PLATFORM['machine']
            or sys.version_info[:2] != (3, 12)):
        raise ValueError('dependency warmup requires the unprivileged Linux x86_64/Python 3.12 runner')
    env = {'HOME': '/home/runner', 'USER': 'runner', 'LOGNAME': 'runner',
           'PATH': '/home/runner/.cargo/bin:/usr/local/bin:/usr/bin:/bin',
           'UV_CACHE_DIR': '/home/runner/.cache/uv', 'CARGO_HOME': '/home/runner/.cargo',
           'RUSTUP_TOOLCHAIN': PLATFORM['rust'], 'GIT_TERMINAL_PROMPT': '0',
           'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': '/dev/null'}
    for key in ('http_proxy', 'https_proxy', 'HTTP_PROXY', 'HTTPS_PROXY', 'no_proxy', 'NO_PROXY'):
        if key in os.environ:
            env[key] = os.environ[key]
    return env


def warmup(path):
    bundle = load_input(path)
    files = validate_input(bundle)
    provenance = validate_sources(files)
    env = guest_environment()
    for program, wanted in (('/usr/local/bin/uv', 'uv ' + PLATFORM['uv']),
                            ('/home/runner/.cargo/bin/cargo', 'cargo ' + PLATFORM['rust']),
                            ('/home/runner/.cargo/bin/rustc', 'rustc ' + PLATFORM['rust'])):
        version = subprocess.run([program, '--version'], check=True, capture_output=True, text=True, env=env).stdout
        if not re.fullmatch(re.escape(wanted) + r'( \([^()\r\n]+\))?\n?', version):
            raise ValueError('installed dependency tool version differs from recipe')
    # Each pass has new project and venv paths. Offline installation proves the
    # retained caches suffice, rather than reusing the online pass's venv.
    for offline in (False, True):
        with tempfile.TemporaryDirectory(prefix='sandproof-dependencies-') as temporary:
            directory = Path(temporary)
            adapter = write_workspace(directory, files)
            commands_env = dict(env, PATH=str(adapter) + ':' + env['PATH'])
            for project in PROJECTS:
                commands_env['CI_GIT_LOCK'] = str(directory / project / 'uv.lock')
                arguments = UV_ARGUMENTS + ['--project', str(directory / project)] + (['--offline'] if offline else [])
                subprocess.run(['/usr/local/bin/uv', *arguments], check=True, env=commands_env, cwd=directory)
                subprocess.run([str(directory / project / '.venv/bin/python'), '-I', '-c', PROVENANCE_CHECK,
                                canonical_json(provenance[project]).decode()], check=True, env=commands_env, cwd=directory)
            subprocess.run(['/home/runner/.cargo/bin/cargo', *CARGO_ARGUMENTS, *(['--offline'] if offline else [])],
                           check=True, env=commands_env, cwd=directory)
    print('SANDPROOF_CI_DEPENDENCIES_READY ' + bundle['input_sha256'])
    return bundle['input_sha256']


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    prepare = commands.add_parser('prepare-input')
    prepare.add_argument('--checkout', type=Path, required=True)
    prepare.add_argument('--approved-commit', required=True)
    prepare.add_argument('--output', type=Path, required=True)
    prepare.add_argument('--redis-archive', type=Path, required=True)
    guest = commands.add_parser('warmup')
    guest.add_argument('--input', type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == 'prepare-input':
            bundle = prepare_input(args.checkout, args.approved_commit, args.output, args.redis_archive)
            print(json.dumps({'input_sha256': bundle['input_sha256'], 'source_commit': args.approved_commit}))
        else:
            warmup(args.input)
        return 0
    except (OSError, ValueError, TypeError, subprocess.CalledProcessError) as error:
        print('CI dependency preparation refused: ' + str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
